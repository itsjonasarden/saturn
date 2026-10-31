//! Worktrees of one repository that change the same files: parallel agents that will collide when
//! their work is merged. Found early, while it's cheap to split the work differently or to merge one
//! first; found at merge time, it's a conflict to untangle.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The files a checkout changes: since its branch left the default branch, plus anything
/// uncommitted and new. Relative to the checkout.
pub fn changed_files(checkout: &Path) -> BTreeSet<String> {
    let run = |args: &[&str]| {
        crate::git::command(checkout)
            .arg("--no-optional-locks")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    };
    let base = crate::git::merge_base(checkout).unwrap_or_else(|| "HEAD".to_owned());
    let mut files: BTreeSet<String> =
        run(&["diff", "--name-only", "-z", &base]).split('\0').map(str::to_owned).collect();
    files.extend(run(&["ls-files", "--others", "--exclude-standard", "-z"]).split('\0').map(str::to_owned));
    files.remove("");
    files
}

/// For each checkout, the others that change some of the same files, and which files. Checkouts are
/// only compared with others of their own repository (`repo_of`).
pub fn overlaps(
    changed: &BTreeMap<PathBuf, BTreeSet<String>>,
    repo_of: impl Fn(&Path) -> Option<PathBuf>,
) -> BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>)>> {
    let repos: BTreeMap<&PathBuf, Option<PathBuf>> =
        changed.keys().map(|checkout| (checkout, repo_of(checkout))).collect();
    let mut found: BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>)>> = BTreeMap::new();
    for (checkout, files) in changed {
        for (other, theirs) in changed {
            if other == checkout || repos[checkout].is_none() || repos[checkout] != repos[other] {
                continue;
            }
            let shared: Vec<String> = files.intersection(theirs).cloned().collect();
            if !shared.is_empty() {
                found.entry(checkout.clone()).or_default().push((other.clone(), shared));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_checkouts_of_one_repo_changing_the_same_files() {
        let set = |files: &[&str]| files.iter().map(|file| (*file).to_owned()).collect::<BTreeSet<_>>();
        let changed = BTreeMap::from([
            (PathBuf::from("/code/acme-web-a"), set(&["src/cart.ts", "src/a.ts"])),
            (PathBuf::from("/code/acme-web-b"), set(&["src/cart.ts", "src/b.ts"])),
            (PathBuf::from("/code/acme-web-c"), set(&["src/c.ts"])),
            (PathBuf::from("/code/other-app"), set(&["src/cart.ts"])),
        ]);
        let repo_of = |path: &Path| {
            Some(PathBuf::from(if path.starts_with("/code/other-app") { "/code/other-app" } else { "/code/acme-web" }))
        };
        let found = overlaps(&changed, repo_of);
        assert_eq!(
            found[Path::new("/code/acme-web-a")],
            [(PathBuf::from("/code/acme-web-b"), vec!["src/cart.ts".to_owned()])]
        );
        assert_eq!(found[Path::new("/code/acme-web-b")].len(), 1);
        assert!(!found.contains_key(Path::new("/code/acme-web-c")));
        assert!(!found.contains_key(Path::new("/code/other-app")), "another repository's files are its own");
    }

    #[test]
    fn a_checkout_changes_its_branch_and_uncommitted_files() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let repo = dir.path();
        let git = |args: &[&str]| {
            let status = crate::git::command(repo)
                .args(["-c", "user.email=test@example.com", "-c", "user.name=Test"])
                .args(args)
                .output()
                .expect("git runs")
                .status;
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        fs::write(repo.join("a.txt"), "a\n").expect("write");
        fs::write(repo.join("b.txt"), "b\n").expect("write");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "start"]);
        git(&["switch", "-q", "-c", "work"]);
        fs::write(repo.join("a.txt"), "changed\n").expect("write");
        git(&["commit", "-q", "-am", "work"]);
        fs::write(repo.join("b.txt"), "uncommitted\n").expect("write");
        fs::write(repo.join("new.txt"), "new\n").expect("write");
        assert_eq!(changed_files(repo), ["a.txt", "b.txt", "new.txt"].map(str::to_owned).into_iter().collect());
    }
}
