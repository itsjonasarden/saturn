//! `.worktreeinclude`: Claude Code's list of files a new worktree should get from the main checkout,
//! written like `.gitignore`. They're files git doesn't track (local config, certificates, a
//! downloaded model), so a worktree starts without them. Akyba copies them in when it makes a
//! worktree, as `claude --worktree` does, so the two make the same worktree.

use std::path::{Path, PathBuf};

/// Where the list lives, at the top of the main checkout.
pub const FILE: &str = ".worktreeinclude";

/// The files in `main` that the list names and git doesn't track, relative to it.
pub fn included(main: &Path) -> Vec<PathBuf> {
    let list = main.join(FILE);
    if !list.is_file() {
        return Vec::new();
    }
    let Ok(output) = crate::git::command(main)
        .args(["--no-optional-locks", "ls-files", "-z", "--others", "--ignored", "--exclude-from"])
        .arg(&list)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout).split('\0').filter(|path| !path.is_empty()).map(PathBuf::from).collect()
}

/// Copies the listed files from the main checkout into `worktree`, leaving any it already has (a link
/// to a shared env file, or its own version). Returns what was copied. Nothing happens in the main
/// checkout itself.
pub fn copy_into(worktree: &Path) -> Vec<PathBuf> {
    let Some(main) = crate::git::main_worktree(worktree) else { return Vec::new() };
    let (Ok(main), Ok(worktree)) = (main.canonicalize(), worktree.canonicalize()) else { return Vec::new() };
    if main == worktree {
        return Vec::new();
    }
    included(&main)
        .into_iter()
        .filter(|relative| {
            let target = worktree.join(relative);
            if target.symlink_metadata().is_ok() {
                return false;
            }
            let copied = target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::copy(main.join(relative), &target));
            copied.is_ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn copies_what_the_list_names_and_keeps_what_is_there() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let main = dir.path().join("acme-web");
        fs::create_dir_all(main.join("certs")).expect("make folders");
        let git = |args: &[&str]| {
            let status = crate::git::command(&main).args(args).output().expect("git runs").status;
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        fs::write(main.join(".gitignore"), "certs/\n.env\nlocal.json\n").expect("write");
        fs::write(main.join(FILE), "certs/\nlocal.json\n").expect("write");
        git(&["add", "."]);
        git(&["-c", "user.email=test@example.com", "-c", "user.name=Test", "commit", "-q", "-m", "start"]);
        fs::write(main.join("certs/dev.pem"), "key").expect("write");
        fs::write(main.join("local.json"), "{ \"main\": true }").expect("write");
        fs::write(main.join(".env"), "PORT=4000").expect("write");
        let worktree = dir.path().join("acme-web-abc-123");
        git(&["worktree", "add", "-q", "-b", "abc-123", &worktree.to_string_lossy()]);
        fs::write(worktree.join("local.json"), "{ \"own\": true }").expect("write");

        let copied = copy_into(&worktree);
        assert_eq!(copied, [PathBuf::from("certs/dev.pem")]);
        assert_eq!(fs::read_to_string(worktree.join("certs/dev.pem")).expect("copied"), "key");
        assert_eq!(fs::read_to_string(worktree.join("local.json")).expect("kept"), "{ \"own\": true }");
        assert!(!worktree.join(".env").exists(), "not on the list");
        assert_eq!(copy_into(&main), Vec::<PathBuf>::new(), "main is left alone");
    }

    #[test]
    fn no_list_copies_nothing() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        assert_eq!(included(dir.path()), Vec::<PathBuf>::new());
    }
}
