//! A checkout's files as they are, saved in git without touching its branch, its index or any hook:
//! a commit on top of `HEAD` that only Akyba's refs point at. Taken before and after an agent's
//! turn, two of them say exactly which files the turn changed, however it changed them (an edit, a
//! formatter, a shell command), and can put those files back either way.

use std::path::Path;

/// Files as they were at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The commit holding them.
    pub commit: String,
    /// Its tree: two snapshots with the same tree hold the same files.
    pub tree: String,
}

/// Saves `checkout`'s files (tracked, changed and new; not what git ignores) and keeps them under
/// `name`, a ref of Akyba's (`refs/akyba/<name>`), so git doesn't drop them.
pub fn take(checkout: &Path, name: &str) -> Result<Snapshot, String> {
    let head = git(checkout, &["rev-parse", "HEAD"]).ok_or("git can't read the commit")?;
    // Through an index of its own, so the checkout's staging is left alone.
    let index = std::env::temp_dir().join(format!("akyba-snapshot-{}-{}", std::process::id(), name.replace('/', "-")));
    let with_index = |args: &[&str]| -> Option<String> {
        let output = crate::git::command(checkout).env("GIT_INDEX_FILE", &index).args(args).output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let tree = with_index(&["read-tree", "HEAD"])
        .and_then(|_| with_index(&["add", "-A"]))
        .and_then(|_| with_index(&["write-tree"]));
    let _ = std::fs::remove_file(&index);
    let tree = tree.ok_or("git can't save the files")?;
    let commit = git(checkout, &["commit-tree", &tree, "-p", &head, "-m", "Files as they were, saved by Akyba"])
        .ok_or("git can't keep the saved files")?;
    git(checkout, &["update-ref", &format!("refs/akyba/{name}"), &commit]).ok_or("git can't keep the saved files")?;
    Ok(Snapshot { commit, tree })
}

/// The files that differ between two snapshots.
pub fn changed_between(checkout: &Path, from: &Snapshot, to: &Snapshot) -> Vec<String> {
    git(checkout, &["diff", "--name-only", "--no-renames", "-z", &from.tree, &to.tree])
        .map(|out| out.split('\0').filter(|file| !file.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Makes the files that differ between `from` and `to` as they are in `to`, leaving every other file
/// alone: a file `to` doesn't have is deleted. Returns how many files it put back.
pub fn put_back(checkout: &Path, from: &Snapshot, to: &Snapshot) -> Result<usize, String> {
    let files = changed_between(checkout, from, to);
    let in_to: std::collections::HashSet<String> = git(checkout, &["ls-tree", "-r", "--name-only", "-z", &to.tree])
        .map(|out| out.split('\0').filter(|file| !file.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let (restore, delete): (Vec<&String>, Vec<&String>) = files.iter().partition(|file| in_to.contains(*file));
    if !restore.is_empty() {
        let output = crate::git::command(checkout)
            .args(["restore", "--source", &to.commit, "--worktree", "--"])
            .args(restore.iter().map(|file| file.as_str()))
            .output()
            .map_err(|error| format!("git could not run: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
    }
    for file in delete {
        let _ = std::fs::remove_file(checkout.join(file));
    }
    Ok(files.len())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = crate::git::command(dir).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn undoes_and_redoes_one_turn_and_leaves_other_work() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let repo = dir.path();
        let sh = |args: &[&str]| {
            let status = crate::git::command(repo)
                .args(["-c", "user.email=test@example.com", "-c", "user.name=Test"])
                .args(args)
                .output()
                .expect("git runs")
                .status;
            assert!(status.success(), "git {args:?}");
        };
        sh(&["init", "-q", "-b", "main"]);
        fs::write(repo.join("cart.ts"), "v1\n").expect("write");
        fs::write(repo.join("old.ts"), "old\n").expect("write");
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "start"]);
        fs::write(repo.join("mine.ts"), "my own work\n").expect("write");

        let before = take(repo, "turns/1/before").expect("saved");
        // The agent's turn: an edit, a new file, a deletion (as a shell command would).
        fs::write(repo.join("cart.ts"), "v2\n").expect("write");
        fs::write(repo.join("added.ts"), "new\n").expect("write");
        fs::remove_file(repo.join("old.ts")).expect("remove");
        let after = take(repo, "turns/1/after").expect("saved");
        assert_eq!(changed_between(repo, &before, &after), ["added.ts", "cart.ts", "old.ts"]);

        assert_eq!(put_back(repo, &after, &before).expect("undone"), 3);
        assert_eq!(fs::read_to_string(repo.join("cart.ts")).expect("read"), "v1\n");
        assert!(!repo.join("added.ts").exists());
        assert_eq!(fs::read_to_string(repo.join("old.ts")).expect("read"), "old\n");
        assert_eq!(fs::read_to_string(repo.join("mine.ts")).expect("read"), "my own work\n", "not the turn's");

        put_back(repo, &before, &after).expect("redone");
        assert_eq!(fs::read_to_string(repo.join("cart.ts")).expect("read"), "v2\n");
        assert!(repo.join("added.ts").exists() && !repo.join("old.ts").exists());
        let staged = crate::git::command(repo).args(["diff", "--cached", "--name-only"]).output().expect("git runs");
        assert!(staged.stdout.is_empty(), "the index is left alone");
    }
}
