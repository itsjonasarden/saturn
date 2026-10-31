//! Review comments for an agent: what you wrote about a line of its changes, gathered into one
//! message it can act on, each with the file, the lines and the code it's about, so the agent
//! doesn't have to guess which "this" you mean.

use std::fmt::Write as _;

/// One comment on a line or lines of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// The file, relative to the checkout.
    pub path: String,
    /// One-based, inclusive.
    pub lines: (usize, usize),
    /// Lines that were removed, not ones that are there now.
    pub removed: bool,
    /// The code commented on, as far as it's known.
    pub code: Vec<String>,
    pub text: String,
}

/// Lines of code quoted with a comment, at most; the agent can read the rest.
const QUOTED: usize = 8;

/// The comments as one message to the agent.
pub fn message(comments: &[Comment]) -> String {
    let mut out = String::from(if comments.len() == 1 {
        "A review comment on your changes. Please address it:\n"
    } else {
        "Review comments on your changes. Please address each one:\n"
    });
    for (index, comment) in comments.iter().enumerate() {
        let (start, end) = comment.lines;
        let lines = if start == end { format!("line {start}") } else { format!("lines {start}–{end}") };
        let removed = if comment.removed { " (removed)" } else { "" };
        let _ = write!(out, "\n{}. {}, {lines}{removed}:\n", index + 1, comment.path);
        if !comment.code.is_empty() {
            out.push_str("```\n");
            for line in comment.code.iter().take(QUOTED) {
                out.push_str(line);
                out.push('\n');
            }
            if comment.code.len() > QUOTED {
                out.push_str("…\n");
            }
            out.push_str("```\n");
        }
        out.push_str(comment.text.trim());
        out.push('\n');
    }
    out
}

/// Lines `start` to `end` (one-based, inclusive) of `text`.
pub fn lines_of(text: &str, (start, end): (usize, usize)) -> Vec<String> {
    text.lines().skip(start.saturating_sub(1)).take(end + 1 - start.max(1)).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(path: &str, lines: (usize, usize), code: &[&str], text: &str) -> Comment {
        Comment {
            path: path.to_owned(),
            lines,
            removed: false,
            code: code.iter().map(|line| (*line).to_owned()).collect(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn one_comment_quotes_its_code() {
        let text = message(&[comment("src/cart.ts", (12, 12), &["  return total;"], " Round to cents here. ")]);
        assert_eq!(
            text,
            "A review comment on your changes. Please address it:\n\n1. src/cart.ts, line 12:\n```\n  return total;\n```\nRound to cents here.\n"
        );
    }

    #[test]
    fn several_comments_are_numbered_and_long_code_is_cut() {
        let long: Vec<String> = (1..=12).map(|n| format!("line {n}")).collect();
        let long: Vec<&str> = long.iter().map(String::as_str).collect();
        let mut removed = comment("src/old.ts", (3, 4), &[], "Why was this removed?");
        removed.removed = true;
        let text = message(&[comment("src/a.ts", (1, 12), &long, "Split this up."), removed]);
        assert!(text.starts_with("Review comments on your changes. Please address each one:\n"));
        assert!(text.contains("1. src/a.ts, lines 1–12:\n```\nline 1\n"));
        assert!(text.contains("line 8\n…\n```\nSplit this up.\n"));
        assert!(!text.contains("line 9\n"));
        assert!(text.contains("2. src/old.ts, lines 3–4 (removed):\nWhy was this removed?\n"));
    }

    #[test]
    fn takes_the_lines_asked_for() {
        let text = "one\ntwo\nthree\nfour\n";
        assert_eq!(lines_of(text, (2, 3)), ["two", "three"]);
        assert_eq!(lines_of(text, (4, 9)), ["four"]);
    }
}
