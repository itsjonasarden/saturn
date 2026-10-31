//! Every Claude Code session on this Mac, to search and resume: read from the transcripts under
//! `~/.claude/projects`, one file per session. Each is summed up by its title (the one you gave it,
//! else Claude's), the folder and branch it ran in, its pull request, and the prompts you typed;
//! search looks at those, not at Claude's replies. Transcripts run to megabytes, so a file is read
//! again only when it changed. The format is Claude Code's own; anything not understood is passed over.

use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// How much of a session's prompts is kept to search, so a long session can't fill the memory.
const KEPT: usize = 16_000;

/// One session, summed up.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    /// Whose session: Claude Code's (these transcripts) or Codex's (`codex_sessions`).
    pub agent: crate::agents::Agent,
    pub id: String,
    /// Its title: the one you gave it, else Claude's, else its first prompt.
    pub title: String,
    /// The folder it ran in, where it can be resumed.
    pub cwd: Option<PathBuf>,
    pub branch: Option<String>,
    pub pull_request: Option<u32>,
    /// The prompts you typed, in order, cut at `KEPT` characters all told.
    pub prompts: Vec<String>,
    /// When it last did anything (ISO 8601), for display.
    pub at: String,
    /// The transcript changed last at this moment, for ordering.
    pub modified: Option<SystemTime>,
}

impl Session {
    /// Whether every word of `query` is in its title, folder, branch or prompts, ignoring case.
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return true;
        }
        let mut text = self.title.to_lowercase();
        for part in [self.cwd.as_ref().map(|cwd| cwd.to_string_lossy().into_owned()), self.branch.clone()]
            .into_iter()
            .flatten()
            .chain(self.prompts.iter().cloned())
        {
            text.push('\n');
            text.push_str(&part.to_lowercase());
        }
        words.iter().all(|word| text.contains(word.as_str()))
    }

    /// The prompt that best shows why it matched `query`: the first holding one of its words, else
    /// the last one typed.
    #[must_use]
    pub fn snippet(&self, query: &str) -> Option<&str> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.prompts
            .iter()
            .find(|prompt| {
                let prompt = prompt.to_lowercase();
                words.iter().any(|word| prompt.contains(word.as_str()))
            })
            .or_else(|| self.prompts.last())
            .map(String::as_str)
    }
}

/// Sums up one transcript; `None` when it holds no prompt of yours (a session that never began).
#[must_use]
pub fn summarize(id: &str, lines: impl Iterator<Item = String>) -> Option<Session> {
    let mut session = Session { id: id.to_owned(), ..Session::default() };
    let (mut custom, mut generated) = (None, None);
    let mut kept = 0;
    for line in lines {
        // Most lines are replies and tool output: skip them before parsing.
        let wanted = ["\"user\"", "-title\"", "\"pr-link\""].iter().any(|kind| line.contains(kind));
        if !wanted {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
        match entry.get("type").and_then(Value::as_str) {
            Some("custom-title") => custom = text("customTitle").or(custom),
            Some("ai-title") => generated = text("aiTitle").or(generated),
            Some("pr-link") => {
                session.pull_request =
                    entry.get("prNumber").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
            }
            Some("user") => {
                if session.cwd.is_none() {
                    session.cwd = text("cwd").map(PathBuf::from);
                }
                if let Some(branch) = text("gitBranch").filter(|branch| !branch.is_empty() && branch != "HEAD") {
                    session.branch = Some(branch);
                }
                if let Some(at) = text("timestamp") {
                    session.at = at;
                }
                if let Some(prompt) = crate::prompts::typed_prompt(&entry)
                    && kept < KEPT
                {
                    kept += prompt.len();
                    session.prompts.push(prompt);
                }
            }
            _ => {}
        }
    }
    let first = session.prompts.first()?.lines().next().unwrap_or_default().to_owned();
    session.title = custom.or(generated).unwrap_or(first);
    Some(session)
}

/// The sessions read so far, kept between searches so only changed transcripts are read again.
#[derive(Default)]
pub struct Index {
    read: HashMap<PathBuf, (SystemTime, Option<Session>)>,
}

impl Index {
    /// Reads what changed under `claude_home`'s projects, then gives every session, newest first.
    /// Slow the first time: off the main thread.
    pub fn refresh(&mut self, claude_home: &Path) -> Vec<Session> {
        let mut seen = Vec::new();
        let projects = std::fs::read_dir(claude_home.join("projects")).into_iter().flatten().flatten();
        for project in projects {
            // A session's subagents keep their transcripts in folders of their own; only the
            // project's top-level files are sessions.
            for file in std::fs::read_dir(project.path()).into_iter().flatten().flatten() {
                let path = file.path();
                if path.extension().is_none_or(|extension| extension != "jsonl") {
                    continue;
                }
                let Ok(modified) = file.metadata().and_then(|meta| meta.modified()) else { continue };
                seen.push(path.clone());
                if self.read.get(&path).is_some_and(|(when, _)| *when == modified) {
                    continue;
                }
                let id = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                let session = std::fs::File::open(&path).ok().and_then(|file| {
                    summarize(&id, std::io::BufReader::new(file).lines().map_while(Result::ok))
                        .map(|session| Session { modified: Some(modified), ..session })
                });
                self.read.insert(path, (modified, session));
            }
        }
        self.read.retain(|path, _| seen.contains(path));
        let mut sessions: Vec<Session> = self.read.values().filter_map(|(_, session)| session.clone()).collect();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
        sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> impl Iterator<Item = String> + '_ {
        text.lines().map(str::to_owned)
    }

    const TRANSCRIPT: &str = r#"{"type":"permission-mode","permissionMode":"default","sessionId":"s1"}
{"type":"user","cwd":"/code/acme-web","gitBranch":"abc-123","timestamp":"2026-01-02T10:00:00Z","message":{"role":"user","content":"Fix the checkout total when a coupon is applied"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"The user wants a coupon fix"}]}}
{"type":"user","cwd":"/code/acme-web","gitBranch":"abc-123","timestamp":"2026-01-02T10:05:00Z","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}
{"type":"user","cwd":"/code/acme-web","gitBranch":"abc-123","timestamp":"2026-01-02T10:09:00Z","message":{"role":"user","content":"Now add a test for it"}}
{"type":"ai-title","aiTitle":"Coupon totals at checkout","sessionId":"s1"}
{"type":"pr-link","prNumber":42,"prUrl":"https://example.com/pull/42","sessionId":"s1"}"#;

    #[test]
    fn sums_up_a_session() {
        let session = summarize("s1", lines(TRANSCRIPT)).expect("a session");
        assert_eq!(session.title, "Coupon totals at checkout");
        assert_eq!(session.cwd.as_deref(), Some(Path::new("/code/acme-web")));
        assert_eq!(session.branch.as_deref(), Some("abc-123"));
        assert_eq!(session.pull_request, Some(42));
        assert_eq!(session.prompts, ["Fix the checkout total when a coupon is applied", "Now add a test for it"]);
        assert_eq!(session.at, "2026-01-02T10:09:00Z");
        let named = format!("{TRANSCRIPT}\n{{\"type\":\"custom-title\",\"customTitle\":\"ABC-123\"}}");
        assert_eq!(summarize("s1", lines(&named)).expect("a session").title, "ABC-123");
        assert_eq!(summarize("s2", lines(r#"{"type":"mode","mode":"normal"}"#)), None);
    }

    #[test]
    fn searches_prompts_titles_and_places_but_not_replies() {
        let session = summarize("s1", lines(TRANSCRIPT)).expect("a session");
        assert!(session.matches("COUPON test"));
        assert!(session.matches("acme-web abc-123"));
        assert!(session.matches(""));
        assert!(!session.matches("user wants"));
        assert!(!session.matches("coupon refund"));
        assert_eq!(session.snippet("test"), Some("Now add a test for it"));
        assert_eq!(session.snippet("acme"), Some("Now add a test for it"));
    }

    #[test]
    fn reads_changed_transcripts_again_and_forgets_removed_ones() {
        let home = tempfile::tempdir().expect("temp dir");
        let project = home.path().join("projects").join("-code-acme-web");
        std::fs::create_dir_all(project.join("s1").join("subagents")).expect("folders");
        std::fs::write(project.join("s1.jsonl"), TRANSCRIPT).expect("write");
        std::fs::write(project.join("s1").join("subagents").join("agent.jsonl"), TRANSCRIPT).expect("write");
        let mut index = Index::default();
        let sessions = index.refresh(home.path());
        assert_eq!(sessions.iter().map(|session| session.id.as_str()).collect::<Vec<_>>(), ["s1"]);
        std::fs::remove_file(project.join("s1.jsonl")).expect("remove");
        assert_eq!(index.refresh(home.path()), Vec::new());
    }
}
