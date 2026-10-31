//! One change (a hunk) out of a diff, to keep or undo on its own: staged into the next commit, or
//! reverted in the working tree, leaving the file's other changes as they are. An agent's edit is
//! often right in one place and wrong in another; this takes the wrong one back without the rest.

use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

/// One `@@` hunk of a file's diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The file's header lines (`diff --git`, `---`, `+++`, modes), which a patch of it needs.
    pub header: String,
    /// The hunk itself, from its `@@` line.
    pub body: String,
    /// The file, as the diff names it (its new path).
    pub path: String,
    /// The lines it covers before and after (one-based start, count).
    pub old: (usize, usize),
    pub new: (usize, usize),
}

impl Hunk {
    /// A patch with only this hunk, for `git apply`.
    pub fn patch(&self) -> String {
        format!("{}{}", self.header, self.body)
    }

    /// The lines it changes, without the unchanged ones around them: on the new side, or the old
    /// one when it only removes. One-based, inclusive.
    pub fn changed_lines(&self) -> (usize, usize) {
        let (mut old, mut new) = (self.old.0, self.new.0);
        let (mut added, mut removed) = (Vec::new(), Vec::new());
        for line in self.body.lines().skip(1) {
            match line.chars().next() {
                Some('+') => {
                    added.push(new);
                    new += 1;
                }
                Some('-') => {
                    removed.push(old);
                    old += 1;
                }
                Some('\\') => {}
                _ => {
                    old += 1;
                    new += 1;
                }
            }
        }
        let lines = if added.is_empty() { removed } else { added };
        (lines.first().copied().unwrap_or(self.new.0), lines.last().copied().unwrap_or(self.new.0))
    }

    /// Whether it covers `line` on the new side (`removed` false) or the old one.
    pub fn covers(&self, line: usize, removed: bool) -> bool {
        let (start, count) = if removed { self.old } else { self.new };
        // A hunk that only adds or only removes has no lines on one side; it sits just after `start`.
        let end = start + count.max(1);
        line >= start.max(1) && line < end
    }
}

/// Every hunk of a `git diff` patch, in order.
pub fn parse(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut header = String::new();
    let mut file = String::new();
    let mut current: Option<Hunk> = None;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            hunks.extend(current.take());
            line.clone_into(&mut header);
            line.trim_end().rsplit(" b/").next().unwrap_or_default().clone_into(&mut file);
        } else if let Some(range) = line.strip_prefix("@@ ") {
            hunks.extend(current.take());
            let (old, new) = ranges(range);
            current = Some(Hunk { header: header.clone(), body: line.to_owned(), path: file.clone(), old, new });
        } else if let Some(hunk) = current.as_mut() {
            hunk.body.push_str(line);
        } else {
            if let Some(name) = line.strip_prefix("+++ b/") {
                name.trim_end().clone_into(&mut file);
            }
            header.push_str(line);
        }
    }
    hunks.extend(current);
    hunks
}

/// `-12,3 +12,4 @@ …` as ((12, 3), (12, 4)); a missing count is 1.
fn ranges(text: &str) -> ((usize, usize), (usize, usize)) {
    let mut parts = text.split_whitespace();
    let side = |part: Option<&str>, sign: char| {
        let part = part.and_then(|part| part.strip_prefix(sign)).unwrap_or("0");
        let (start, count) = part.split_once(',').unwrap_or((part, "1"));
        (start.parse().unwrap_or(0), count.parse().unwrap_or(1))
    };
    (side(parts.next(), '-'), side(parts.next(), '+'))
}

/// The hunk of `file` that covers `line` in `diff`.
pub fn at(diff: &str, file: &str, line: usize, removed: bool) -> Option<Hunk> {
    parse(diff).into_iter().find(|hunk| hunk.path == file && hunk.covers(line, removed))
}

/// Undoes `hunk` in the working tree of `repo`.
pub fn revert(repo: &Path, hunk: &Hunk) -> Result<(), String> {
    apply(repo, hunk, &["apply", "--reverse"])
}

/// Puts a reverted `hunk` back in the working tree of `repo`.
pub fn reapply(repo: &Path, hunk: &Hunk) -> Result<(), String> {
    apply(repo, hunk, &["apply"])
}

/// Stages `hunk` for the next commit, leaving the working tree as it is.
pub fn stage(repo: &Path, hunk: &Hunk) -> Result<(), String> {
    apply(repo, hunk, &["apply", "--cached"])
}

fn apply(repo: &Path, hunk: &Hunk, args: &[&str]) -> Result<(), String> {
    let mut child = crate::git::command(repo)
        .args(args)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("git could not run: {error}"))?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(hunk.patch().as_bytes()).map_err(|error| error.to_string())?;
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        Err(said.lines().next().unwrap_or("git couldn't apply the change").trim_start_matches("error: ").to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const PATCH: &str = "diff --git a/src/cart.ts b/src/cart.ts\nindex 1111..2222 100644\n--- a/src/cart.ts\n+++ b/src/cart.ts\n@@ -1,3 +1,3 @@\n-let a = 1;\n+let a = 2;\n let b = 0;\n let c = 0;\n@@ -8,2 +8,3 @@ function total() {\n let x = 1;\n+let y = 2;\n let z = 3;\ndiff --git a/new.ts b/new.ts\nnew file mode 100644\n--- /dev/null\n+++ b/new.ts\n@@ -0,0 +1 @@\n+export {};\n";

    #[test]
    fn splits_a_patch_into_its_hunks() {
        let hunks = parse(PATCH);
        assert_eq!(hunks.len(), 3);
        assert_eq!((hunks[0].path.as_str(), hunks[0].old, hunks[0].new), ("src/cart.ts", (1, 3), (1, 3)));
        assert_eq!(hunks[1].new, (8, 3));
        assert_eq!(hunks[1].changed_lines(), (9, 9));
        assert_eq!(hunks[0].changed_lines(), (1, 1));
        assert!(hunks[1].patch().starts_with("diff --git a/src/cart.ts b/src/cart.ts\nindex"));
        assert!(hunks[1].patch().contains("@@ -8,2 +8,3 @@ function total() {\n let x = 1;\n+let y = 2;\n"));
        assert!(!hunks[1].patch().contains("let a = 2"), "only its own hunk");
        assert_eq!((hunks[2].path.as_str(), hunks[2].old, hunks[2].new), ("new.ts", (0, 0), (1, 1)));
    }

    #[test]
    fn finds_the_hunk_under_a_line() {
        assert_eq!(at(PATCH, "src/cart.ts", 9, false).map(|hunk| hunk.new), Some((8, 3)));
        assert_eq!(at(PATCH, "src/cart.ts", 1, true).map(|hunk| hunk.old), Some((1, 3)));
        assert_eq!(at(PATCH, "src/cart.ts", 20, false), None);
        assert!(at(PATCH, "new.ts", 1, false).is_some());
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
    fn reverts_or_stages_one_change_and_leaves_the_other() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        fs::write(repo.join("a.txt"), lines.join("\n") + "\n").expect("write");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "start"]);
        let mut edited = lines.clone();
        edited[1] = "line 2 changed".to_owned();
        edited[17] = "line 18 changed".to_owned();
        fs::write(repo.join("a.txt"), edited.join("\n") + "\n").expect("write");

        let patch = crate::git::patch(repo, crate::git::Scope::Uncommitted);
        let last = at(&patch, "a.txt", 18, false).expect("the second change");
        revert(repo, &last).expect("reverted");
        let now = fs::read_to_string(repo.join("a.txt")).expect("read");
        assert!(now.contains("line 2 changed") && now.contains("line 18\n") && !now.contains("line 18 changed"));
        reapply(repo, &last).expect("put back");
        assert!(fs::read_to_string(repo.join("a.txt")).expect("read").contains("line 18 changed"));
        revert(repo, &last).expect("reverted again");

        let patch = crate::git::patch(repo, crate::git::Scope::Uncommitted);
        let first = at(&patch, "a.txt", 2, false).expect("the first change");
        stage(repo, &first).expect("staged");
        let staged = crate::git::command(repo).args(["diff", "--cached"]).output().expect("git runs");
        assert!(String::from_utf8_lossy(&staged.stdout).contains("+line 2 changed"));
    }
}
