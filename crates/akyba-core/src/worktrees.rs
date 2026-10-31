//! A repository's worktrees, and which of them are done with. Agents make worktrees faster than
//! anyone tidies them, and removing one by hand means checking it's merged (a squash merge leaves no
//! trace in the branch), has nothing uncommitted, and has nothing still running in it. This reads all
//! of that, and removes a worktree the way the repository asks (its recipe's `cleanup` first).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One worktree, as `git worktree list --porcelain` tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// `None` when it's on a detached `HEAD`.
    pub branch: Option<String>,
    /// The main checkout, which is never removed.
    pub main: bool,
    /// Its folder is gone; git still lists it until it's pruned.
    pub missing: bool,
}

/// Reads `git worktree list --porcelain`: blocks of lines, one per worktree, the main one first.
pub fn parse_list(porcelain: &str) -> Vec<Worktree> {
    porcelain
        .split("\n\n")
        .filter_map(|block| {
            let mut worktree = None::<Worktree>;
            for line in block.lines() {
                if let Some(path) = line.strip_prefix("worktree ") {
                    worktree = Some(Worktree { path: PathBuf::from(path), branch: None, main: false, missing: false });
                } else if let (Some(branch), Some(worktree)) = (line.strip_prefix("branch "), worktree.as_mut()) {
                    worktree.branch = Some(branch.trim_start_matches("refs/heads/").to_owned());
                } else if let (true, Some(worktree)) = (line.starts_with("prunable"), worktree.as_mut()) {
                    worktree.missing = true;
                }
            }
            worktree
        })
        .enumerate()
        .map(|(index, worktree)| Worktree { main: index == 0, ..worktree })
        .collect()
}

/// The worktrees of the repository `repo` is in, the main checkout first.
pub fn list(repo: &Path) -> Vec<Worktree> {
    run(repo, &["worktree", "list", "--porcelain"]).map(|out| parse_list(&out)).unwrap_or_default()
}

/// The branch work lands in: what `origin/HEAD` points at, else `main` or `master`.
pub fn default_branch(repo: &Path) -> Option<String> {
    run(repo, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"])
        .map(|name| name.trim().to_owned())
        .or_else(|| {
            ["main", "master"]
                .into_iter()
                .find(|name| run(repo, &["rev-parse", "--verify", "--quiet", name]).is_some())
                .map(str::to_owned)
        })
}

/// Whether a branch's work is in the default branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merged {
    No,
    /// Its commits are in it (merged, or none of its own).
    Yes,
    /// Its changes are, under other commits: squashed or rebased in.
    ByContent,
}

/// Whether `branch` is merged into `base`. A squash merge leaves the branch's commits out of `base`,
/// so the content counts too: if merging the branch would change nothing, its work is in.
pub fn merged(repo: &Path, branch: &str, base: &str) -> Merged {
    let ancestor = crate::git::command(repo)
        .args(["merge-base", "--is-ancestor", branch, base])
        .status()
        .is_ok_and(|status| status.success());
    if ancestor {
        return Merged::Yes;
    }
    let merged_tree =
        run(repo, &["merge-tree", "--write-tree", base, branch]).and_then(|out| out.lines().next().map(str::to_owned));
    let base_tree = run(repo, &["rev-parse", &format!("{base}^{{tree}}")]).map(|out| out.trim().to_owned());
    match (merged_tree, base_tree) {
        (Some(merged), Some(base)) if merged == base => Merged::ByContent,
        _ => Merged::No,
    }
}

/// Files changed or added and not committed, in a worktree. Ignored files (`node_modules`, shared env
/// links) don't count.
pub fn uncommitted(worktree: &Path) -> usize {
    run(worktree, &["--no-optional-locks", "status", "--porcelain"]).map_or(0, |out| out.lines().count())
}

/// When the worktree's last commit was made, in seconds since 1970.
pub fn last_commit(worktree: &Path) -> Option<i64> {
    run(worktree, &["log", "-1", "--format=%ct"]).and_then(|out| out.trim().parse().ok())
}

/// The programs whose working folder is somewhere in each of `folders` (a dev server, an agent, a
/// shell), by name: what would break if the folder went. Runs `lsof` over every process: off the main
/// thread.
pub fn programs_in(folders: &[PathBuf]) -> HashMap<PathBuf, Vec<String>> {
    let Ok(output) = Command::new("lsof").args(["-nP", "-d", "cwd", "-Fcn"]).output() else {
        return HashMap::new();
    };
    let mut found: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut program = String::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(name) = line.strip_prefix('c') {
            name.clone_into(&mut program);
        } else if let Some(cwd) = line.strip_prefix('n') {
            let cwd = Path::new(cwd);
            if let Some(folder) = folders.iter().find(|folder| cwd.starts_with(folder)) {
                let names = found.entry(folder.clone()).or_default();
                if !names.contains(&program) {
                    names.push(program.clone());
                }
            }
        }
    }
    found
}

/// What removing a worktree did, besides removing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Removed {
    /// Its branch was merged, so it went too.
    pub branch_deleted: bool,
    /// Its recipe's cleanup failed; the worktree went anyway.
    pub cleanup_failed: bool,
}

/// Removes a worktree: its recipe's `cleanup` first (when there is one), then the folder, then its
/// branch when git agrees it's merged (`branch -d`; a squashed one stays, for git to keep its
/// history). Refuses one with uncommitted work, as `git worktree remove` does.
///
/// Deleting a worktree's files (a `node_modules` is tens of thousands) takes seconds, so the folder
/// is moved into the repository's trash instead, which is instant on the same disk, git is told it's
/// gone, and the files are deleted in the background. Where the move can't be made, `git worktree
/// remove` does it all, slowly.
pub fn remove(main: &Path, worktree: &Worktree, cleanup: Option<&str>) -> Result<Removed, String> {
    if worktree.main {
        return Err("The main checkout stays.".to_owned());
    }
    if worktree.missing {
        return run(main, &["worktree", "prune"])
            .map(|_| Removed::default())
            .ok_or_else(|| "git couldn't forget the missing worktree.".to_owned());
    }
    let mut removed = Removed::default();
    if let Some(cleanup) = cleanup {
        let ran = Command::new("/bin/sh").args(["-c", cleanup]).current_dir(&worktree.path).output();
        removed.cleanup_failed = !ran.is_ok_and(|output| output.status.success());
    }
    if !move_to_trash(main, &worktree.path)? {
        let output = crate::git::command(main)
            .args(["worktree", "remove"])
            .arg(&worktree.path)
            .output()
            .map_err(|error| format!("git could not run: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
    }
    removed.branch_deleted =
        worktree.branch.as_ref().is_some_and(|branch| run(main, &["branch", "-d", branch]).is_some());
    Ok(removed)
}

/// Moves a worktree's folder into the trash and drops git's record of it, as `git worktree remove`
/// would, then empties the trash in the background. `Ok(false)` when it can't be done this way
/// (a locked worktree, a folder git's status can't read, another disk), so git should do it.
fn move_to_trash(main: &Path, folder: &Path) -> Result<bool, String> {
    let Some(status) = run(folder, &["--no-optional-locks", "status", "--porcelain"]) else { return Ok(false) };
    if !status.trim().is_empty() {
        return Err("It has uncommitted work.".to_owned());
    }
    // The folder's `.git` file names git's record of it: `gitdir: <repo>/.git/worktrees/<name>`.
    let Some(record) = std::fs::read_to_string(folder.join(".git"))
        .ok()
        .and_then(|text| text.strip_prefix("gitdir:").map(|path| PathBuf::from(path.trim())))
    else {
        return Ok(false);
    };
    if record.join("locked").exists() {
        return Ok(false);
    }
    let Some(trash) = run(main, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map(|dir| PathBuf::from(dir.trim()).join("akyba-trash"))
    else {
        return Ok(false);
    };
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
    let name = folder.file_name().map_or_else(|| "worktree".into(), |name| name.to_string_lossy().into_owned());
    if std::fs::create_dir_all(&trash).is_err()
        || std::fs::rename(folder, trash.join(format!("{name}-{nanos}"))).is_err()
    {
        return Ok(false);
    }
    let _ = std::fs::remove_dir_all(&record);
    // Everything in the trash, a removal an earlier run didn't finish too.
    std::thread::spawn(move || {
        for entry in std::fs::read_dir(&trash).into_iter().flatten().flatten() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    });
    Ok(true)
}

/// A worktree put away: its branch, and its work as it was, so it can come back where it was.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Archived {
    /// Where it was, and where it comes back.
    pub path: PathBuf,
    pub branch: String,
    /// The branch's commit when it was put away.
    pub head: String,
    /// Its files as they were, uncommitted work and new files too: a commit on top of `head` (or
    /// `head` itself when nothing was uncommitted), kept by a ref of Akyba's so git doesn't drop it.
    pub snapshot: String,
    /// When, in seconds since 1970.
    pub at: i64,
}

/// The ref that keeps an archived worktree's snapshot.
fn archive_ref(branch: &str) -> String {
    format!("refs/akyba/archive/{branch}")
}

/// Puts a worktree away: saves its files as they are (without touching its branch or running any
/// hook), runs its recipe's `cleanup`, and removes the folder. Ignored files (`node_modules`) go;
/// setup makes them again. Refuses the main checkout, a missing one and a detached one.
pub fn archive(main: &Path, worktree: &Worktree, cleanup: Option<&str>, now: i64) -> Result<Archived, String> {
    let branch = match (&worktree.branch, worktree.main || worktree.missing) {
        (Some(branch), false) => branch.clone(),
        _ => return Err("Only a worktree on a branch can be archived.".to_owned()),
    };
    let path = &worktree.path;
    let head = run(path, &["rev-parse", "HEAD"]).map(|out| out.trim().to_owned()).ok_or("git can't read its commit")?;
    let saved = crate::snapshot::take(path, &format!("archive/{branch}"))?;
    let head_tree = run(path, &["rev-parse", "HEAD^{tree}"]).map(|out| out.trim().to_owned());
    // Nothing uncommitted: the branch's own commit is all there is to bring back.
    let snapshot = if head_tree.as_deref() == Some(saved.tree.as_str()) { head.clone() } else { saved.commit };
    if let Some(cleanup) = cleanup {
        let _ = Command::new("/bin/sh").args(["-c", cleanup]).current_dir(path).output();
    }
    // Forced: what it held is saved above.
    let output = crate::git::command(main)
        .args(["worktree", "remove", "--force"])
        .arg(path)
        .output()
        .map_err(|error| format!("git could not run: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(Archived { path: path.clone(), branch, head, snapshot, at: now })
}

/// Brings an archived worktree back where it was: its branch (made again from its commit if it was
/// deleted meanwhile), then its saved files over it, uncommitted as they were.
pub fn restore(main: &Path, archived: &Archived) -> Result<(), String> {
    if archived.path.exists() {
        return Err(format!("{} is in the way.", archived.path.display()));
    }
    let path = archived.path.to_string_lossy().into_owned();
    let branch_exists =
        run(main, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{}", archived.branch)]).is_some();
    let args: Vec<&str> = if branch_exists {
        vec!["worktree", "add", &path, &archived.branch]
    } else {
        vec!["worktree", "add", "-b", &archived.branch, &path, &archived.head]
    };
    let output =
        crate::git::command(main).args(&args).output().map_err(|error| format!("git could not run: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    if archived.snapshot != archived.head {
        run(&archived.path, &["restore", "--source", &archived.snapshot, "--worktree", "--", "."])
            .ok_or("The worktree is back, but git couldn't put its uncommitted work back.")?;
    }
    let _ = run(main, &["update-ref", "-d", &archive_ref(&archived.branch)]);
    // Archived before Akyba, it was kept under Akyba's name.
    let _ = run(main, &["update-ref", "-d", &format!("refs/saturn/archive/{}", archived.branch)]);
    Ok(())
}

/// The archived worktrees of each repository, kept in a file beside Akyba's settings.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Archives {
    repos: std::collections::BTreeMap<PathBuf, Vec<Archived>>,
}

impl Archives {
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(file, serde_json::to_string_pretty(self).unwrap_or_default())
    }

    pub fn of(&self, repo: &Path) -> &[Archived] {
        self.repos.get(repo).map_or(&[], Vec::as_slice)
    }

    pub fn add(&mut self, repo: &Path, archived: Archived) {
        self.repos.entry(repo.to_owned()).or_default().push(archived);
    }

    pub fn forget(&mut self, repo: &Path, path: &Path) {
        if let Some(list) = self.repos.get_mut(repo) {
            list.retain(|archived| archived.path != path);
        }
    }
}

fn run(repo: &Path, args: &[&str]) -> Option<String> {
    let output = crate::git::command(repo).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_the_list() {
        let list = parse_list(
            "worktree /code/acme-web\nHEAD 1111\nbranch refs/heads/main\n\nworktree /code/acme-web-abc-1\nHEAD 2222\nbranch refs/heads/abc-1\n\nworktree /tmp/gone\nHEAD 3333\ndetached\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(list.len(), 3);
        assert!(list[0].main && list[0].branch.as_deref() == Some("main"));
        assert_eq!(list[1].branch.as_deref(), Some("abc-1"));
        assert!(!list[1].main && !list[1].missing);
        assert!(list[2].missing && list[2].branch.is_none());
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = crate::git::command(dir)
            .args(["-c", "user.email=test@example.com", "-c", "user.name=Test"])
            .args(args)
            .output()
            .expect("git runs")
            .status;
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn knows_merged_squashed_and_unfinished_work() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        fs::create_dir_all(&main).expect("make folders");
        git(&main, &["init", "-q", "-b", "main"]);
        fs::write(main.join("a.txt"), "a\n").expect("write");
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "start"]);
        for (branch, file) in [("merged", "m.txt"), ("squashed", "s.txt"), ("open", "o.txt")] {
            git(&main, &["switch", "-q", "-c", branch, "main"]);
            fs::write(main.join(file), "work\n").expect("write");
            git(&main, &["add", "."]);
            git(&main, &["commit", "-q", "-m", branch]);
        }
        git(&main, &["switch", "-q", "main"]);
        git(&main, &["merge", "-q", "--no-edit", "merged"]);
        git(&main, &["merge", "-q", "--squash", "squashed"]);
        git(&main, &["commit", "-q", "-m", "squashed in"]);
        assert_eq!(merged(&main, "merged", "main"), Merged::Yes);
        assert_eq!(merged(&main, "squashed", "main"), Merged::ByContent);
        assert_eq!(merged(&main, "open", "main"), Merged::No);
        assert_eq!(default_branch(&main).as_deref(), Some("main"));
    }

    #[test]
    fn removes_a_finished_worktree_and_its_merged_branch() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        fs::create_dir_all(&main).expect("make folders");
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "start"]);
        let path = dir.path().join("acme-web-abc-1");
        git(&main, &["worktree", "add", "-q", "-b", "abc-1", &path.to_string_lossy()]);
        let worktrees = list(&main);
        let worktree = worktrees.iter().find(|worktree| worktree.branch.as_deref() == Some("abc-1")).expect("listed");
        assert_eq!(uncommitted(&worktree.path), 0);

        let marker = dir.path().join("cleaned");
        let removed = remove(&main, worktree, Some(&format!("touch {}", marker.display()))).expect("removed");
        assert!(marker.exists(), "cleanup ran first");
        assert!(!path.exists());
        assert_eq!(removed, Removed { branch_deleted: true, cleanup_failed: false });
        assert_eq!(list(&main).len(), 1, "git forgot it");
        assert!(run(&main, &["rev-parse", "--verify", "--quiet", "refs/heads/abc-1"]).is_none());
        assert!(remove(&main, &list(&main)[0], None).is_err(), "the main checkout stays");
    }

    #[test]
    fn removes_a_worktree_with_ignored_files_by_moving_it_aside() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        fs::create_dir_all(&main).expect("make folders");
        git(&main, &["init", "-q", "-b", "main"]);
        fs::write(main.join(".gitignore"), "node_modules\n").expect("write");
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "start"]);
        let path = dir.path().join("acme-web-abc-4");
        git(&main, &["worktree", "add", "-q", "-b", "abc-4", &path.to_string_lossy()]);
        fs::create_dir_all(path.join("node_modules/left-pad")).expect("make folders");
        fs::write(path.join("node_modules/left-pad/index.js"), "module.exports = 1;\n").expect("write");
        let worktree = list(&main).into_iter().find(|worktree| !worktree.main).expect("listed");

        remove(&main, &worktree, None).expect("removed");
        assert!(!path.exists());
        assert_eq!(list(&main).len(), 1, "git forgot it");
        assert!(!main.join(".git/worktrees/acme-web-abc-4").exists(), "and its record");
        // The trash empties in the background.
        let trash = main.join(".git/akyba-trash");
        for _ in 0..100 {
            if fs::read_dir(&trash).map_or(true, |mut entries| entries.next().is_none()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the trash was never emptied");
    }

    #[test]
    fn archives_a_worktree_with_its_unsaved_work_and_brings_it_back() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        fs::create_dir_all(&main).expect("make folders");
        git(&main, &["init", "-q", "-b", "main"]);
        fs::write(main.join("a.txt"), "a\n").expect("write");
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "start"]);
        let path = dir.path().join("acme-web-abc-3");
        git(&main, &["worktree", "add", "-q", "-b", "abc-3", &path.to_string_lossy()]);
        fs::write(path.join("a.txt"), "edited\n").expect("write");
        fs::write(path.join("new.txt"), "brand new\n").expect("write");
        let worktree = list(&main).into_iter().find(|worktree| !worktree.main).expect("listed");

        let archived = archive(&main, &worktree, None, 1).expect("archived");
        assert!(!path.exists());
        assert_ne!(archived.snapshot, archived.head, "the unsaved work is in a snapshot");
        assert!(run(&main, &["rev-parse", "--verify", "refs/heads/abc-3"]).is_some(), "the branch stays");
        assert_eq!(run(&main, &["rev-parse", "abc-3"]).map(|out| out.trim().to_owned()), Some(archived.head.clone()));

        restore(&main, &archived).expect("restored");
        assert_eq!(fs::read_to_string(path.join("a.txt")).expect("read"), "edited\n");
        assert_eq!(fs::read_to_string(path.join("new.txt")).expect("read"), "brand new\n");
        assert!(uncommitted(&path) > 0, "still uncommitted, as it was");
        assert!(run(&main, &["rev-parse", "--verify", "--quiet", &archive_ref("abc-3")]).is_none());
    }

    #[test]
    fn remembers_archives_per_repo() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let file = dir.path().join("archives.json");
        let put_away = Archived {
            path: "/code/acme-web-abc-1".into(),
            branch: "abc-1".into(),
            head: "1".into(),
            snapshot: "1".into(),
            at: 5,
        };
        let mut written = Archives::default();
        written.add(Path::new("/code/acme-web"), put_away.clone());
        written.save(&file).expect("saved");
        let mut read = Archives::load(&file);
        assert_eq!(read.of(Path::new("/code/acme-web")), [put_away]);
        read.forget(Path::new("/code/acme-web"), Path::new("/code/acme-web-abc-1"));
        assert_eq!(read.of(Path::new("/code/acme-web")), []);
    }

    #[test]
    fn refuses_a_worktree_with_uncommitted_work() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        fs::create_dir_all(&main).expect("make folders");
        git(&main, &["init", "-q", "-b", "main"]);
        git(&main, &["commit", "-q", "--allow-empty", "-m", "start"]);
        let path = dir.path().join("acme-web-abc-2");
        git(&main, &["worktree", "add", "-q", "-b", "abc-2", &path.to_string_lossy()]);
        fs::write(path.join("draft.txt"), "unsaved\n").expect("write");
        let worktree = list(&main).into_iter().find(|worktree| !worktree.main).expect("listed");
        assert_eq!(uncommitted(&worktree.path), 1);
        assert!(remove(&main, &worktree, None).is_err());
        assert!(path.exists());
    }
}
