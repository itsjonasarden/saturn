//! Best of N: one task given to several agents at once, each in a worktree of its own, so their
//! answers can be laid side by side and the best kept. The attempts are told apart by their
//! branches alone (`fix-login-try-1`, `fix-login-try-2`), so nothing else needs remembering: any
//! repository's worktrees say which attempts there are, even after Akyba restarts.

use std::path::{Path, PathBuf};

use crate::worktrees::Worktree;

/// The most attempts at once; more are hard to compare side by side.
pub const MOST: usize = 4;

/// What marks an attempt's branch: `<task>-try-<n>`.
const TRY: &str = "-try-";

/// The work names of `count` attempts at `task`, from its first few words: `Fix the login page`
/// becomes `fix-the-login-page-try-1` and so on (as `git::branch_for` makes them branches).
#[must_use]
pub fn work_names(task: &str, count: usize) -> Vec<String> {
    let words: Vec<&str> = task.split_whitespace().take(5).collect();
    let mut stem = crate::git::branch_for(&words.join(" "));
    if stem.is_empty() {
        "attempt".clone_into(&mut stem);
    }
    stem.truncate(40);
    let stem = stem.trim_end_matches('-');
    (1..=count).map(|number| format!("{stem}{TRY}{number}")).collect()
}

/// The task's stem and the attempt's number, for an attempt's branch.
#[must_use]
pub fn attempt_of(branch: &str) -> Option<(&str, usize)> {
    let (stem, number) = branch.rsplit_once(TRY)?;
    let number = number.parse().ok()?;
    (!stem.is_empty()).then_some((stem, number))
}

/// One group of attempts: the task's stem, and its worktrees in number order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub stem: String,
    pub attempts: Vec<(usize, Worktree)>,
}

/// The repository's attempts, grouped by task, the group with the newest folder first.
#[must_use]
pub fn groups(worktrees: &[Worktree]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for worktree in worktrees.iter().filter(|worktree| !worktree.main && !worktree.missing) {
        let Some((stem, number)) = worktree.branch.as_deref().and_then(attempt_of) else { continue };
        match groups.iter_mut().find(|group| group.stem == stem) {
            Some(group) => group.attempts.push((number, worktree.clone())),
            None => groups.push(Group { stem: stem.to_owned(), attempts: vec![(number, worktree.clone())] }),
        }
    }
    for group in &mut groups {
        group.attempts.sort_by_key(|(number, _)| *number);
    }
    let made = |group: &Group| {
        group
            .attempts
            .iter()
            .filter_map(|(_, worktree)| std::fs::metadata(&worktree.path).and_then(|meta| meta.created()).ok())
            .max()
    };
    groups.sort_by_key(|group| std::cmp::Reverse(made(group)));
    groups
}

/// One file an attempt changed, with its lines added and removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

/// What an attempt did since the attempts began: its files, committed or not, and its commits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub files: Vec<FileChange>,
    pub commits: usize,
}

impl Outcome {
    #[must_use]
    pub fn added(&self) -> usize {
        self.files.iter().map(|file| file.added).sum()
    }

    #[must_use]
    pub fn removed(&self) -> usize {
        self.files.iter().map(|file| file.removed).sum()
    }
}

/// Where the attempts began: the commit all their branches come from. A lone attempt has nothing
/// to meet, so it's measured from where it left the default branch.
#[must_use]
pub fn start(main: &Path, group: &Group) -> Option<String> {
    let branches: Vec<&str> = group.attempts.iter().filter_map(|(_, worktree)| worktree.branch.as_deref()).collect();
    let mut args = vec!["merge-base", "--octopus"];
    if branches.len() < 2 {
        args = vec!["merge-base"];
        args.extend(branches.iter().copied());
        let base = crate::worktrees::default_branch(main)?;
        return run(main, &[&args[..], &[base.as_str()]].concat());
    }
    args.extend(branches.iter().copied());
    run(main, &args)
}

/// What the attempt in `worktree` changed since `start`: what's committed, what isn't, and new
/// files not yet added.
#[must_use]
pub fn outcome(worktree: &Path, start: &str) -> Outcome {
    let mut files: Vec<FileChange> = run(worktree, &["diff", "--numstat", "--no-renames", start])
        .unwrap_or_default()
        .lines()
        .filter_map(parse_numstat)
        .collect();
    let untracked = run(worktree, &["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for path in untracked.lines().filter(|line| !line.is_empty()) {
        let lines = std::fs::read_to_string(worktree.join(path)).map_or(0, |text| text.lines().count());
        files.push(FileChange { path: path.to_owned(), added: lines, removed: 0 });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let commits = run(worktree, &["rev-list", "--count", &format!("{start}..HEAD")])
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0);
    Outcome { files, commits }
}

/// A line of `git diff --numstat`: added, removed, path (`-` for a binary file's counts).
fn parse_numstat(line: &str) -> Option<FileChange> {
    let mut parts = line.splitn(3, '\t');
    let added = parts.next()?.parse().unwrap_or(0);
    let removed = parts.next()?.parse().unwrap_or(0);
    let path = parts.next()?.to_owned();
    Some(FileChange { path, added, removed })
}

/// The folder of each attempt, in number order, for opening panes in them.
#[must_use]
pub fn folders(group: &Group) -> Vec<PathBuf> {
    group.attempts.iter().map(|(_, worktree)| worktree.path.clone()).collect()
}

fn run(dir: &Path, args: &[&str]) -> Option<String> {
    let output = crate::git::command(dir).args(args).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = crate::git::command(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("git runs");
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    }

    #[test]
    fn names_attempts_and_reads_them_back() {
        let names = work_names("Fix the login page so it remembers you", 3);
        assert_eq!(
            names,
            ["fix-the-login-page-so-try-1", "fix-the-login-page-so-try-2", "fix-the-login-page-so-try-3"]
        );
        assert_eq!(attempt_of("fix-the-login-page-so-try-2"), Some(("fix-the-login-page-so", 2)));
        assert_eq!(work_names("!!!", 1), ["attempt-try-1"]);
        assert_eq!(attempt_of("abc-123"), None);
        assert_eq!(attempt_of("try-again-try-x"), None);
        assert_eq!(attempt_of("-try-1"), None);
    }

    #[test]
    fn groups_attempts_by_task_in_number_order() {
        let worktree = |path: &str, branch: &str| Worktree {
            path: PathBuf::from(path),
            branch: Some(branch.to_owned()),
            main: false,
            missing: false,
        };
        let main = Worktree { main: true, ..worktree("/acme-web", "main") };
        let list = [
            main,
            worktree("/acme-web-fix-try-2", "fix-try-2"),
            worktree("/acme-web-abc-123", "abc-123"),
            worktree("/acme-web-fix-try-1", "fix-try-1"),
        ];
        let groups = groups(&list);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].stem, "fix");
        assert_eq!(groups[0].attempts.iter().map(|(number, _)| *number).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn reads_what_each_attempt_changed_since_they_began() {
        let root = tempfile::tempdir().expect("temp dir");
        let main = root.path().join("acme-web");
        std::fs::create_dir(&main).expect("repo folder");
        git(&main, &["init", "-q", "-b", "main"]);
        std::fs::write(main.join("app.js"), "one\ntwo\n").expect("write");
        git(&main, &["add", "."]);
        git(&main, &["commit", "-q", "-m", "start"]);
        let mut attempts = Vec::new();
        for name in work_names("Fix it", 2) {
            let (dir, made) = crate::git::work_worktree(&main, &name).expect("worktree");
            assert!(made);
            attempts.push(dir);
        }
        // The first commits a change; the second leaves one uncommitted and adds a file.
        std::fs::write(attempts[0].join("app.js"), "one\n2\nthree\n").expect("write");
        git(&attempts[0], &["commit", "-q", "-am", "fix"]);
        std::fs::write(attempts[1].join("app.js"), "one\ntwo\nthree\n").expect("write");
        std::fs::write(attempts[1].join("new.js"), "a\nb\n").expect("write");

        let groups = groups(&crate::worktrees::list(&main));
        assert_eq!(groups.len(), 1);
        let start = start(&main, &groups[0]).expect("a start");
        let first = outcome(&attempts[0], &start);
        assert_eq!(first.commits, 1);
        assert_eq!((first.added(), first.removed()), (2, 1));
        let second = outcome(&attempts[1], &start);
        assert_eq!(second.commits, 0);
        assert_eq!(second.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), ["app.js", "new.js"]);
        assert_eq!((second.added(), second.removed()), (3, 0));
    }
}
