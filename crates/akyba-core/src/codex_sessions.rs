//! Codex's sessions, from the rollouts it writes under `$CODEX_HOME/sessions/YYYY/MM/DD/`
//! (`rollout-<time>-<id>.jsonl`, one JSON event a line): each session summed up as Search sessions
//! shows Claude Code's (title, folder, branch, the prompts you typed), and the session a folder
//! runs. The names you give threads (`/rename`) are in
//! `session_index.jsonl`. The format is Codex's own; anything not understood is passed over.

use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::agents::Agent;
use crate::past_sessions::Session;

/// Every rollout under `home`'s sessions, newest first by when it last changed.
#[must_use]
pub fn rollouts(home: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut found = Vec::new();
    let walk = ignore::WalkBuilder::new(home.join("sessions")).standard_filters(false).build();
    for entry in walk.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with("rollout-")
            && path.extension().is_some_and(|extension| extension == "jsonl")
            && let Some(modified) = entry.metadata().ok().and_then(|meta| meta.modified().ok())
        {
            found.push((path.to_owned(), modified));
        }
    }
    found.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    found
}

/// A prompt you typed, from a `user` message: not the context Codex adds itself (the project's
/// `AGENTS.md`, `<environment_context>` and the like).
fn typed(payload: &Value) -> Option<String> {
    if payload.get("type").and_then(Value::as_str) != Some("message")
        || payload.get("role").and_then(Value::as_str) != Some("user")
    {
        return None;
    }
    let text: Vec<&str> = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    let text = text.join("\n");
    let text = text.trim();
    let added = text.starts_with('<') || text.starts_with("# AGENTS.md instructions");
    (!text.is_empty() && !added).then(|| text.to_owned())
}

/// How much of a session's prompts is kept to search, as for Claude Code's.
const KEPT: usize = 16_000;

/// A rollout summed up as a session; `name` is the one you gave the thread, if any.
pub fn summarize(lines: impl Iterator<Item = String>, name: Option<&str>) -> Option<Session> {
    let mut session = Session { agent: Agent::Codex, ..Session::default() };
    let mut kept = 0;
    for line in lines {
        let wanted = ["\"session_meta\"", "\"user\""].iter().any(|kind| line.contains(kind));
        if !wanted {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        let payload = entry.get("payload").unwrap_or(&Value::Null);
        let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        match entry.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                session.id = text(payload, "id").unwrap_or_default();
                session.cwd = text(payload, "cwd").map(PathBuf::from);
                session.branch =
                    payload.get("git").and_then(|git| text(git, "branch")).filter(|branch| !branch.is_empty());
                session.at = text(payload, "timestamp").unwrap_or_default();
            }
            Some("response_item") => {
                if let Some(prompt) = typed(payload)
                    && kept < KEPT
                {
                    kept += prompt.len();
                    session.prompts.push(prompt);
                }
                if let Some(at) = text(&entry, "timestamp") {
                    session.at = at;
                }
            }
            _ => {}
        }
    }
    if session.id.is_empty() {
        return None;
    }
    let first = session.prompts.first().map(|prompt| prompt.lines().next().unwrap_or_default().to_owned());
    session.title = name.map(str::to_owned).or(first).unwrap_or_else(|| session.id.clone());
    Some(session)
}

/// The names given to threads, by id: `session_index.jsonl`, the last line for an id winning.
#[must_use]
pub fn names(home: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(home.join("session_index.jsonl")) else { return HashMap::new() };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?.to_owned();
            let name = entry.get("thread_name")?.as_str()?.trim().to_owned();
            (!name.is_empty()).then_some((id, name))
        })
        .collect()
}

/// The sessions read so far, kept between searches so only changed rollouts are read again.
#[derive(Default)]
pub struct Index {
    read: HashMap<PathBuf, (SystemTime, Option<Session>)>,
}

impl Index {
    /// Reads what changed under `home`'s sessions, then gives every session, newest first. Slow the
    /// first time: off the main thread.
    pub fn refresh(&mut self, home: &Path) -> Vec<Session> {
        let names = names(home);
        let rollouts = rollouts(home);
        for (path, modified) in &rollouts {
            if self.read.get(path).is_some_and(|(when, _)| when == modified) {
                continue;
            }
            let session = std::fs::File::open(path).ok().and_then(|file| {
                summarize(std::io::BufReader::new(file).lines().map_while(Result::ok), None)
                    .map(|session| Session { modified: Some(*modified), ..session })
            });
            self.read.insert(path.clone(), (*modified, session));
        }
        self.read.retain(|path, _| rollouts.iter().any(|(rollout, _)| rollout == path));
        let mut sessions: Vec<Session> = self
            .read
            .values()
            .filter_map(|(_, session)| session.clone())
            .map(|mut session| {
                // A name given later (`/rename`) is in the index, not the rollout.
                if let Some(name) = names.get(&session.id) {
                    session.title.clone_from(name);
                }
                session
            })
            .collect();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.modified));
        sessions
    }
}

/// The session that started in `folder` at or after `since` (Unix seconds), newest first: how a
/// pane's Codex session is found when Codex hasn't told Akyba about it yet.
#[must_use]
pub fn session_in(home: &Path, folder: &Path, since: i64) -> Option<String> {
    let since = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(u64::try_from(since).unwrap_or(0));
    let folder = folder.canonicalize().unwrap_or_else(|_| folder.to_owned());
    rollouts(home).into_iter().take_while(|(_, modified)| *modified >= since).find_map(|(path, _)| {
        let first = std::io::BufReader::new(std::fs::File::open(&path).ok()?).lines().next()?.ok()?;
        let entry: Value = serde_json::from_str(&first).ok()?;
        let meta = entry.get("payload")?;
        let cwd = PathBuf::from(meta.get("cwd")?.as_str()?);
        let cwd = cwd.canonicalize().unwrap_or(cwd);
        (cwd == folder).then(|| meta.get("id")?.as_str().map(str::to_owned)).flatten()
    })
}

/// The prompts typed in the session `id`, in order: what Teach the repo looks through.
#[must_use]
pub fn typed_prompts(home: &Path, id: &str) -> Vec<String> {
    let Some((path, _)) = rollouts(home).into_iter().find(|(path, _)| path.to_string_lossy().contains(id)) else {
        return Vec::new();
    };
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("response_item"))
        .filter_map(|entry| typed(entry.get("payload")?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLLOUT: &str = r##"{"timestamp":"2026-10-20T10:00:00Z","type":"session_meta","payload":{"id":"019a0000-1111-7222-8333-444455556666","timestamp":"2026-10-20T10:00:00Z","cwd":"/code/acme-web","git":{"branch":"fix-login"}}}
{"timestamp":"2026-10-20T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /code/acme-web\n\nUse pnpm."}]}}
{"timestamp":"2026-10-20T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/code/acme-web</cwd>\n</environment_context>"}]}}
{"timestamp":"2026-10-20T10:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the login redirect"}]}}
{"timestamp":"2026-10-20T10:01:00Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}
{"timestamp":"2026-10-20T10:02:00Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"codex","primary":{"used_percent":44.0,"window_minutes":10080,"resets_at":1761300000},"secondary":null}}}
{"timestamp":"2026-10-20T10:03:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"no, keep the old URL"}]}}"##;

    fn home_with_rollout() -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("temp dir");
        let day = home.path().join("sessions/2026/10/20");
        std::fs::create_dir_all(&day).expect("folders");
        std::fs::write(day.join("rollout-2026-10-20T10-00-00-019a0000-1111-7222-8333-444455556666.jsonl"), ROLLOUT)
            .expect("write");
        home
    }

    #[test]
    fn sums_up_a_codex_session_without_the_context_it_adds() {
        let session = summarize(ROLLOUT.lines().map(str::to_owned), None).expect("a session");
        assert_eq!(session.agent, Agent::Codex);
        assert_eq!(session.id, "019a0000-1111-7222-8333-444455556666");
        assert_eq!(session.cwd, Some(PathBuf::from("/code/acme-web")));
        assert_eq!(session.branch.as_deref(), Some("fix-login"));
        assert_eq!(session.prompts, ["Fix the login redirect", "no, keep the old URL"]);
        assert_eq!(session.title, "Fix the login redirect");
        let named = summarize(ROLLOUT.lines().map(str::to_owned), Some("Login redirect")).expect("a session");
        assert_eq!(named.title, "Login redirect");
    }

    #[test]
    fn indexes_sessions_with_their_given_names() {
        let home = home_with_rollout();
        std::fs::write(
            home.path().join("session_index.jsonl"),
            r#"{"id":"019a0000-1111-7222-8333-444455556666","thread_name":"Login redirect","updated_at":"x"}"#,
        )
        .expect("write");
        let sessions = Index::default().refresh(home.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Login redirect");
        assert!(sessions[0].modified.is_some());
    }

    #[test]
    fn finds_a_folders_session_and_its_prompts() {
        let home = home_with_rollout();
        let id = "019a0000-1111-7222-8333-444455556666";
        assert_eq!(session_in(home.path(), Path::new("/code/acme-web"), 0).as_deref(), Some(id));
        assert_eq!(session_in(home.path(), Path::new("/code/other"), 0), None);
        assert_eq!(typed_prompts(home.path(), id), ["Fix the login redirect", "no, keep the old URL"]);
    }
}
