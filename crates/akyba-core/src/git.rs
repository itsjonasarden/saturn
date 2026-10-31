//! What git says about the files in a repository, for the tree's badges and the Changes list.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl FileState {
    /// The one-letter badge shown after the file's name.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Untracked => "?",
            Self::Conflicted => "!",
        }
    }
}

/// Changed files by absolute path, from `git status` in `repo`. Empty when `repo` is not a git
/// repository or git is missing.
pub fn status(repo: &Path) -> HashMap<PathBuf, FileState> {
    let Ok(output) = command(repo)
        // A read must not write: without this, `git status` refreshes .git/index, which the file
        // watcher sees, which runs `git status` again.
        .args(["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
    else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }
    let Some(top) = toplevel(repo) else { return HashMap::new() };
    parse_porcelain(&output.stdout).into_iter().map(|(path, state)| (top.join(path), state)).collect()
}

/// The repository's top folder, which `git status` paths are relative to.
pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    let output = command(dir).args(["rev-parse", "--show-toplevel"]).output().ok()?;
    output.status.success().then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

/// Parses `git status --porcelain=v1 -z`: entries of `XY path`, NUL-separated; a rename or copy
/// is followed by its old path as one more entry.
pub fn parse_porcelain(raw: &[u8]) -> Vec<(PathBuf, FileState)> {
    let mut entries = raw.split(|byte| *byte == 0).filter(|entry| entry.len() > 3);
    let mut files = Vec::new();
    while let Some(entry) = entries.next() {
        let (x, y) = (entry[0], entry[1]);
        let path = PathBuf::from(String::from_utf8_lossy(&entry[3..]).into_owned());
        let state = match (x, y) {
            (b'?', b'?') => FileState::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => FileState::Conflicted,
            (b'R' | b'C', _) => {
                entries.next(); // the old path
                FileState::Renamed
            }
            (b'A', _) => FileState::Added,
            (b'D', _) | (_, b'D') => FileState::Deleted,
            _ => FileState::Modified,
        };
        files.push((path, state));
    }
    files
}

/// How much of a changed file is staged for the next commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staged {
    No,
    /// Some of its changes are staged, some aren't.
    Partly,
    Yes,
}

/// A changed file as the Changes list shows it: its path from the repository's top, whether git
/// tracks it yet, and how much of it is staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub untracked: bool,
    pub staged: Staged,
}

/// Reads `git status --porcelain=v1 -z` for staging: the first column is the index (staged), the
/// second the working tree (not staged yet).
pub fn parse_changes(raw: &[u8]) -> Vec<Change> {
    let mut entries = raw.split(|byte| *byte == 0).filter(|entry| entry.len() > 3);
    let mut changes = Vec::new();
    while let Some(entry) = entries.next() {
        let (x, y) = (entry[0], entry[1]);
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        if matches!(x, b'R' | b'C') {
            entries.next(); // the old path
        }
        let staged = match (x, y) {
            (b'?' | b' ', _) => Staged::No,
            (_, b' ') => Staged::Yes,
            _ => Staged::Partly,
        };
        changes.push(Change { path, untracked: x == b'?', staged });
    }
    changes
}

/// The changed files in `repo`, for staging.
pub fn changes(repo: &Path) -> Vec<Change> {
    let Some(top) = toplevel(repo) else { return Vec::new() };
    let Ok(output) = git(&top).args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]).output() else {
        return Vec::new();
    };
    if output.status.success() { parse_changes(&output.stdout) } else { Vec::new() }
}

/// Stages `paths` (relative to the repository's top) for the next commit.
pub fn stage(repo: &Path, paths: &[String]) -> Result<(), String> {
    change_index(repo, &["add", "--"], paths)
}

/// Takes `paths` out of the next commit again, keeping their changes in the working tree.
pub fn unstage(repo: &Path, paths: &[String]) -> Result<(), String> {
    change_index(repo, &["restore", "--staged", "--"], paths)
}

fn change_index(repo: &Path, command: &[&str], paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let top = toplevel(repo).ok_or_else(|| format!("{} is not a git repository", repo.display()))?;
    // Writing the index needs git's lock, so not `git()`, which reads without it.
    let output = self::command(&top)
        .args(command)
        .args(paths)
        .output()
        .map_err(|error| format!("git could not run: {error}"))?;
    if output.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()) }
}

/// What a review covers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scope {
    /// Everything not committed yet: staged, unstaged and new files, against `HEAD`.
    #[default]
    Uncommitted,
    /// Everything the branch changed since it left the default branch, committed or not: what a
    /// pull request would show, plus work in progress.
    Branch,
    /// Everything that differs from a branch, tag or commit, uncommitted work too.
    Against(String),
}

/// The unified diff for `scope` in `repo`, new files included. Empty outside a repository.
pub fn patch(repo: &Path, scope: Scope) -> String {
    let base = match scope {
        Scope::Uncommitted => "HEAD".to_owned(),
        Scope::Branch => merge_base(repo).unwrap_or_else(|| "HEAD".to_owned()),
        Scope::Against(target) => target,
    };
    let mut patch = run(repo, &["diff", "--no-color", "--no-ext-diff", "-M", &base]).unwrap_or_default();
    // `git diff` leaves out files git does not track yet; show them as added in full.
    let untracked = run(repo, &["ls-files", "--others", "--exclude-standard", "-z"]).unwrap_or_default();
    for file in untracked.split('\0').filter(|file| !file.is_empty()) {
        // `--no-index` exits with 1 when the files differ, which they always do here.
        if let Ok(output) = git(repo).args(["diff", "--no-color", "--no-index", "--", "/dev/null", file]).output() {
            patch.push_str(&String::from_utf8_lossy(&output.stdout));
        }
    }
    patch
}

/// Something to compare with: a branch, a tag or a recent commit, with how old it is and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// What git calls it: `main`, `origin/main`, `v1.2.0`, or a short commit hash.
    pub name: String,
    pub detail: String,
    pub kind: TargetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Branch,
    Tag,
    Commit,
}

/// What `repo` can be compared with: its branches (local and remote) and tags, newest first, then the
/// last 30 commits.
pub fn compare_targets(repo: &Path) -> Vec<Target> {
    let refs = run(
        repo,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%09%(refname:short)%09%(committerdate:relative)%09%(subject)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )
    .unwrap_or_default();
    let mut targets: Vec<Target> = refs
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let (full, name, when, subject) =
                (fields.next()?, fields.next()?, fields.next()?, fields.next().unwrap_or(""));
            // `origin/HEAD` only points at another branch.
            if full.ends_with("/HEAD") {
                return None;
            }
            let kind = if full.starts_with("refs/tags/") { TargetKind::Tag } else { TargetKind::Branch };
            Some(Target { name: name.to_owned(), detail: format!("{when} · {subject}"), kind })
        })
        .collect();
    let commits = run(repo, &["log", "-30", "--format=%h%x09%cr%x09%s"]).unwrap_or_default();
    targets.extend(commits.lines().filter_map(|line| {
        let mut fields = line.splitn(3, '\t');
        let (hash, when, subject) = (fields.next()?, fields.next()?, fields.next().unwrap_or(""));
        Some(Target { name: hash.to_owned(), detail: format!("{when} · {subject}"), kind: TargetKind::Commit })
    }));
    targets
}

/// What one commit changed: against its first parent (so a merge shows what it brought in), or
/// everything, for the first commit.
pub fn commit_patch(repo: &Path, hash: &str) -> String {
    run(repo, &["show", "--no-color", "--no-ext-diff", "-M", "--first-parent", "--format=", hash]).unwrap_or_default()
}

/// The checked-out branch, or `None` when detached or outside a repository.
pub fn branch(repo: &Path) -> Option<String> {
    run(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).map(|name| name.trim().to_owned())
}

/// How many commits the current branch has that its upstream doesn't, and the other way round:
/// what a push would send and a pull would bring. `None` without an upstream.
pub fn ahead_behind(repo: &Path) -> Option<(usize, usize)> {
    let counts = run(repo, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])?;
    parse_counts(&counts)
}

fn parse_counts(counts: &str) -> Option<(usize, usize)> {
    let mut parts = counts.split_whitespace().map(str::parse::<usize>);
    Some((parts.next()?.ok()?, parts.next()?.ok()?))
}

/// Where the current branch left the default branch (`origin/HEAD`, else `main` or `master`).
pub fn merge_base(repo: &Path) -> Option<String> {
    let default_branch = crate::worktrees::default_branch(repo)?;
    run(repo, &["merge-base", "HEAD", &default_branch]).map(|sha| sha.trim().to_owned())
}

/// The main checkout of the repository `dir` is in: itself for a normal checkout, the original
/// for a linked worktree (`git worktree add`), so a ticket's worktree belongs to its repository.
pub fn main_worktree(dir: &Path) -> Option<PathBuf> {
    let common = run(dir, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let common = PathBuf::from(common.trim());
    common.parent().map(Path::to_path_buf)
}

/// A branch name for a piece of work, a ticket (`ABC-236`) or words (`Fix login`): `abc-236`,
/// `fix-login`. Empty when nothing usable is left.
pub fn branch_for(work: &str) -> String {
    let words: String = work
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    words.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-")
}

/// A worktree for a piece of work next to `repo` (`../<repo>-<branch>`) on a branch named after it
/// (`branch_for`), made from the current `HEAD`, or the one already there. Returns its folder, and
/// whether it was made just now (and so still needs setting up).
pub fn work_worktree(repo: &Path, work: &str) -> Result<(PathBuf, bool), String> {
    let branch = branch_for(work);
    if branch.is_empty() {
        return Err(format!("\"{work}\" can't name a branch"));
    }
    let top = toplevel(repo).ok_or_else(|| format!("{} is not a git repository", repo.display()))?;
    let name = top.file_name().map_or_else(|| "repo".into(), |name| name.to_string_lossy().into_owned());
    let dir = top.parent().unwrap_or(&top).join(format!("{name}-{branch}"));
    if dir.join(".git").exists() {
        return Ok((dir, false));
    }
    let dir_arg = dir.to_string_lossy().into_owned();
    let branch_exists = run(&top, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_some();
    let args: Vec<&str> = if branch_exists {
        vec!["worktree", "add", &dir_arg, &branch]
    } else {
        vec!["worktree", "add", "-b", &branch, &dir_arg]
    };
    let output = git(&top).args(&args).output().map_err(|error| format!("git could not run: {error}"))?;
    if output.status.success() {
        Ok((dir, true))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or("git worktree add failed").to_owned())
    }
}

fn git(repo: &Path) -> Command {
    let mut command = command(repo);
    command.arg("--no-optional-locks");
    command
}

/// Git's variables that say which repository to work on. A hook sets them for its own repository
/// (`GIT_DIR` in a worktree, `GIT_INDEX_FILE` before a commit), and a command that inherited them
/// would work on that repository whatever folder it ran in: tests run by a hook once committed
/// into the repository being pushed. The list is `git rev-parse --local-env-vars`.
const LOCATION_VARIABLES: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// `git` working on the repository in `dir`, and only that one.
pub fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dir);
    for name in LOCATION_VARIABLES {
        command.env_remove(name);
    }
    command
}

/// Runs git and returns its output when it succeeds.
fn run(repo: &Path, args: &[&str]) -> Option<String> {
    let output = git(repo).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether anything under `dir` has changed, for the quiet dot on a closed folder.
pub fn has_changes_under<S: std::hash::BuildHasher>(changes: &HashMap<PathBuf, FileState, S>, dir: &Path) -> bool {
    changes.keys().any(|path| path.starts_with(dir) && path != dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ahead_and_behind() {
        assert_eq!(parse_counts("2\t5\n"), Some((2, 5)));
        assert_eq!(parse_counts(""), None);
    }

    #[test]
    fn reads_each_kind_of_change() {
        let raw =
            b" M apps/api/orders.ts\0A  apps/api/filter.ts\0 D old.ts\0?? .env.local\0R  new.ts\0was.ts\0UU merge.ts\0";
        let files = parse_porcelain(raw);
        assert_eq!(
            files,
            [
                (PathBuf::from("apps/api/orders.ts"), FileState::Modified),
                (PathBuf::from("apps/api/filter.ts"), FileState::Added),
                (PathBuf::from("old.ts"), FileState::Deleted),
                (PathBuf::from(".env.local"), FileState::Untracked),
                (PathBuf::from("new.ts"), FileState::Renamed),
                (PathBuf::from("merge.ts"), FileState::Conflicted),
            ]
        );
    }

    #[test]
    fn a_folder_has_changes_when_a_file_beneath_it_does() {
        let changes = HashMap::from([(PathBuf::from("/repo/apps/api/orders.ts"), FileState::Modified)]);
        assert!(has_changes_under(&changes, Path::new("/repo/apps")));
        assert!(!has_changes_under(&changes, Path::new("/repo/packages")));
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        let sh = |args: &[&str]| {
            let status = command(dir.path()).args(args).output().expect("git runs").status;
            assert!(status.success(), "git {args:?}");
        };
        sh(&["init", "--quiet", "--initial-branch=main"]);
        sh(&["config", "user.email", "test@akyba.local"]);
        sh(&["config", "user.name", "Akyba tests"]);
        std::fs::write(dir.path().join("orders.ts"), "export const a = 1;\n").expect("write");
        sh(&["add", "."]);
        sh(&["commit", "--quiet", "-m", "first"]);
        sh(&["switch", "--quiet", "-c", "abc-236"]);
        std::fs::write(dir.path().join("filter.ts"), "export const f = 2;\n").expect("write");
        sh(&["add", "."]);
        sh(&["commit", "--quiet", "-m", "on the branch"]);
        std::fs::write(dir.path().join("orders.ts"), "export const a = 10;\n").expect("write");
        std::fs::write(dir.path().join(".env"), "PORT=4000\n").expect("write");
        dir
    }

    #[test]
    fn uncommitted_covers_edits_and_new_files() {
        let dir = repo();
        let patch = patch(dir.path(), Scope::Uncommitted);
        assert!(patch.contains("+export const a = 10;"), "{patch}");
        assert!(patch.contains("+PORT=4000"), "{patch}");
        assert!(!patch.contains("filter.ts"), "the branch's commit is not uncommitted: {patch}");
    }

    #[test]
    fn branch_covers_its_commits_and_work_in_progress() {
        let dir = repo();
        let patch = patch(dir.path(), Scope::Branch);
        assert!(patch.contains("+export const f = 2;"), "{patch}");
        assert!(patch.contains("+export const a = 10;"), "{patch}");
        assert!(patch.contains("+PORT=4000"), "{patch}");
    }

    #[test]
    fn the_branch_is_the_checked_out_one() {
        let dir = repo();
        assert_eq!(branch(dir.path()).as_deref(), Some("abc-236"));
        assert_eq!(
            toplevel(&dir.path().join(".")).map(|top| top.ends_with(dir.path().file_name().expect("a name"))),
            Some(true)
        );
    }

    #[test]
    fn names_a_branch_after_a_ticket_or_words() {
        assert_eq!(branch_for("ABC-236"), "abc-236");
        assert_eq!(branch_for("  Fix the login page! "), "fix-the-login-page");
        assert_eq!(branch_for("feat/new  thing"), "feat-new-thing");
        assert_eq!(branch_for("???"), "");
    }

    #[test]
    fn lists_branches_tags_and_commits_to_compare_with() {
        let dir = repo();
        let sh = |args: &[&str]| assert!(command(dir.path()).args(args).status().is_ok_and(|status| status.success()));
        sh(&["tag", "v1.0.0", "main"]);
        let targets = compare_targets(dir.path());
        let named = |name: &str| targets.iter().find(|target| target.name == name).map(|target| target.kind);
        assert_eq!(named("main"), Some(TargetKind::Branch));
        assert_eq!(named("abc-236"), Some(TargetKind::Branch));
        assert_eq!(named("v1.0.0"), Some(TargetKind::Tag));
        assert_eq!(targets.iter().filter(|target| target.kind == TargetKind::Commit).count(), 2);
        assert!(targets.iter().any(|target| target.detail.ends_with("· on the branch")));
        let against = patch(dir.path(), Scope::Against("main".to_owned()));
        assert!(against.contains("+export const f = 2;") && against.contains("+PORT=4000"), "{against}");
    }

    #[test]
    fn a_ticket_gets_a_sibling_worktree_once() {
        let dir = repo();
        let (first, made) = work_worktree(dir.path(), "ABC-240").expect("created");
        assert!(made, "made just now, so it gets set up");
        assert!(first.ends_with(format!("{}-abc-240", dir.path().file_name().expect("a name").to_string_lossy())));
        assert_eq!(branch(&first).as_deref(), Some("abc-240"));
        assert_eq!(work_worktree(dir.path(), "ABC-240").expect("reused"), (first.clone(), false));
        let _ = std::fs::remove_dir_all(&first);
    }

    #[test]
    fn a_worktree_belongs_to_its_main_checkout() {
        let dir = repo();
        let main = toplevel(dir.path()).expect("a repository");
        let (worktree, _) = work_worktree(dir.path(), "ABC-241").expect("created");
        assert_eq!(main_worktree(&worktree), Some(main.clone()));
        assert_eq!(main_worktree(&main), Some(main));
        let _ = std::fs::remove_dir_all(&worktree);
    }

    #[test]
    fn reads_what_is_staged() {
        let raw = b"M  staged.ts\0 M unstaged.ts\0MM both.ts\0?? new.ts\0A  added.ts\0R  to.ts\0from.ts\0";
        let changes = parse_changes(raw);
        let staged: Vec<(&str, Staged, bool)> =
            changes.iter().map(|change| (change.path.as_str(), change.staged, change.untracked)).collect();
        assert_eq!(
            staged,
            [
                ("staged.ts", Staged::Yes, false),
                ("unstaged.ts", Staged::No, false),
                ("both.ts", Staged::Partly, false),
                ("new.ts", Staged::No, true),
                ("added.ts", Staged::Yes, false),
                ("to.ts", Staged::Yes, false),
            ]
        );
    }

    #[test]
    fn stages_and_unstages() {
        let dir = repo();
        std::fs::write(dir.path().join("new.txt"), "hi").expect("written");
        let staged_of = |dir: &Path| changes(dir).into_iter().find(|change| change.path == "new.txt").map(|c| c.staged);
        assert_eq!(staged_of(dir.path()), Some(Staged::No));
        stage(dir.path(), &["new.txt".to_owned()]).expect("staged");
        assert_eq!(staged_of(dir.path()), Some(Staged::Yes));
        unstage(dir.path(), &["new.txt".to_owned()]).expect("unstaged");
        assert_eq!(staged_of(dir.path()), Some(Staged::No));
    }

    #[test]
    fn a_folder_outside_git_has_no_status() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert!(status(dir.path()).is_empty());
    }
}
