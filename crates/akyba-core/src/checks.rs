//! Running a repository's checks (`lint`, `test`…, from its recipe or guessed) away from any
//! terminal: the whole output kept in a log file, a few words for the pane, and a short summary for
//! an agent instead of thousands of lines in its context. A run that says no tests ran counts as a
//! failure, whatever its exit code: a test command that finds nothing has checked nothing.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

/// How a check went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The exit code; `None` when it couldn't start or was stopped by a signal.
    pub exit: Option<i32>,
    pub seconds: u64,
    /// Its output said no tests ran.
    pub no_tests: bool,
    /// The last lines of its output, for a summary.
    pub tail: Vec<String>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.exit == Some(0) && !self.no_tests
    }
}

/// Lines of output a summary carries; the rest is in the log.
const TAIL: usize = 40;

/// Runs `command` with the shell in `checkout`, its output (both streams) into `log`. Blocks: off the
/// main thread.
pub fn run(checkout: &Path, command: &str, log: &Path) -> Outcome {
    let started = Instant::now();
    if let Some(folder) = log.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    // `exec 2>&1` interleaves the two streams as a terminal would show them.
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("exec 2>&1; {command}"))
        .current_dir(checkout)
        .stdin(Stdio::null())
        .output();
    let seconds = started.elapsed().as_secs();
    let Ok(output) = output else {
        return Outcome { exit: None, seconds, no_tests: false, tail: vec![format!("Couldn't run `{command}`.")] };
    };
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if let Ok(mut file) = std::fs::File::create(log) {
        let _ = writeln!(file, "$ {command}\n");
        let _ = file.write_all(text.as_bytes());
        let _ = writeln!(file, "\n[exit {:?} after {seconds} s]", output.status.code());
    }
    let lines: Vec<String> = text.lines().map(strip_colours).collect();
    let tail = lines[lines.len().saturating_sub(TAIL)..].to_vec();
    Outcome { exit: output.status.code(), seconds, no_tests: no_tests_ran(&text), tail }
}

/// Whether test output says nothing was tested: a count of none, and no count of some. Several test
/// runners in one command (Cargo's doc tests, a monorepo's packages) each report, so one empty
/// report beside a full one isn't a failure.
pub fn no_tests_ran(output: &str) -> bool {
    let text = strip_colours(output).to_lowercase();
    let empty =
        ["no tests ran", "no tests found", "no test files found", "collected 0 items", "ran 0 tests", "0 tests ran"]
            .iter()
            .any(|phrase| text.contains(phrase))
            || counts(&text).any(|(count, _)| count == 0);
    empty && !counts(&text).any(|(count, _)| count > 0)
}

/// Numbers followed by a word that counts tests: `24 passed`, `3 passing`, `12 tests`.
fn counts(text: &str) -> impl Iterator<Item = (u64, &str)> {
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).collect();
    let pairs: Vec<(u64, &str)> = words
        .windows(2)
        .filter_map(|pair| {
            let count = pair[0].parse::<u64>().ok()?;
            ["passed", "passing", "tests", "test"].contains(&pair[1]).then_some((count, pair[1]))
        })
        .collect();
    pairs.into_iter()
}

/// Output without the colour codes test runners write to terminals.
fn strip_colours(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // `ESC [ … letter`
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What to tell an agent about a check that failed: what ran, how it ended, the last lines, and where
/// the whole log is.
pub fn failure_message(name: &str, command: &str, outcome: &Outcome, log: &Path) -> String {
    let how = if outcome.no_tests {
        "ran no tests".to_owned()
    } else {
        outcome.exit.map_or_else(|| "was stopped".to_owned(), |code| format!("failed with exit code {code}"))
    };
    format!(
        "The `{name}` check (`{command}`) {how} after {} s. Its last lines:\n```\n{}\n```\nThe whole output is in {}. Find the cause, fix it, and run the check again.\n",
        outcome.seconds,
        outcome.tail.join("\n"),
        log.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_when_no_tests_ran() {
        assert!(no_tests_ran("No test files found, exiting with code 0"));
        assert!(no_tests_ran("=== no tests ran in 0.01s ==="));
        assert!(no_tests_ran("Tests: 0 passed, 0 total"));
        assert!(!no_tests_ran(
            "running 0 tests\ntest result: ok. 0 passed\nrunning 24 tests\ntest result: ok. 24 passed"
        ));
        assert!(!no_tests_ran("\x1b[32m✓\x1b[0m 19 passed (19)"));
        assert!(!no_tests_ran("Lint: all good"));
    }

    #[test]
    fn runs_a_check_and_keeps_its_log() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let log = dir.path().join("logs/test.log");
        let passed = run(dir.path(), "echo '3 passed'; echo oops >&2", &log);
        assert!(passed.passed() && passed.tail == ["3 passed", "oops"], "{passed:?}");
        assert!(std::fs::read_to_string(&log).expect("a log").contains("$ echo '3 passed'"));

        let failed = run(dir.path(), "echo 'expected 2, got 3'; exit 1", &log);
        assert_eq!(failed.exit, Some(1));
        let message = failure_message("test", "pnpm test", &failed, &log);
        assert!(message.contains("failed with exit code 1") && message.contains("expected 2, got 3"), "{message}");

        let empty = run(dir.path(), "echo 'No test files found'", &log);
        assert!(!empty.passed() && empty.no_tests);
        assert!(failure_message("test", "pnpm test", &empty, &log).contains("ran no tests"));
    }
}
