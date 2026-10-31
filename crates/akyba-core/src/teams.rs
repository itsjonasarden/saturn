//! Claude Code's agent teams: a lead session and teammates that share a task list and message each
//! other. Teammates usually run inside the lead's terminal, so a pane holds the whole team; this
//! reads what it's doing from where Claude Code keeps it: `~/.claude/teams/<team>/config.json` (the
//! members), `…/inboxes/<member>.json` (messages to each) and `~/.claude/tasks/<team>/` (the tasks).
//! A team is named after its lead's session (`session-` and the id's first eight characters). Only
//! some of this format is documented, so every field is read loosely and anything else passed over.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// One member: the lead or a teammate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// `team-lead` for the lead, else the kind of agent.
    pub kind: Option<String>,
}

/// One message, to a member's inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub text: String,
    /// As the inbox has it (ISO 8601), for order and display.
    pub at: String,
    pub read: bool,
}

/// One task on the team's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub subject: String,
    /// `pending`, `in_progress` or `completed`.
    pub status: String,
    pub owner: Option<String>,
}

/// A team, as read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Team {
    pub name: String,
    pub members: Vec<Member>,
    /// Oldest first.
    pub messages: Vec<Message>,
    pub tasks: Vec<Task>,
}

impl Team {
    /// Messages not yet read by the member they're for.
    #[must_use]
    pub fn unread(&self) -> usize {
        self.messages.iter().filter(|message| !message.read).count()
    }

    /// Tasks done, and all of them.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        (self.tasks.iter().filter(|task| task.status == "completed").count(), self.tasks.len())
    }
}

/// The team a session leads, if it has one: by its name, else any team whose config mentions it.
#[must_use]
pub fn for_session(claude_home: &Path, session: &str) -> Option<Team> {
    let teams = claude_home.join("teams");
    let short: String = session.chars().take(8).collect();
    let named = teams.join(format!("session-{short}"));
    let folder = if named.join("config.json").is_file() {
        named
    } else {
        std::fs::read_dir(&teams).ok()?.flatten().map(|entry| entry.path()).find(|folder| {
            std::fs::read_to_string(folder.join("config.json")).is_ok_and(|config| config.contains(session))
        })?
    };
    let name = folder.file_name()?.to_string_lossy().into_owned();
    Some(read(&folder, &claude_home.join("tasks").join(&name)))
}

/// Reads a team from its folder and its task folder.
#[must_use]
pub fn read(folder: &Path, tasks: &Path) -> Team {
    let name = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let config = read_json(&folder.join("config.json"));
    let members = config
        .as_ref()
        .and_then(|config| config.get("members"))
        .and_then(Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(|member| {
                    Some(Member {
                        name: text(member, &["name", "agentName"])?,
                        kind: text(member, &["agentType", "type"]),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut messages = Vec::new();
    for inbox in json_files(&folder.join("inboxes")) {
        let to = inbox.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let entries = read_json(&inbox);
        let entries = entries.as_ref().and_then(|value| value.as_array().or_else(|| value.get("messages")?.as_array()));
        for entry in entries.into_iter().flatten() {
            let Some(body) = text(entry, &["text", "message", "content"]) else { continue };
            messages.push(Message {
                from: text(entry, &["from", "sender"]).unwrap_or_else(|| "?".to_owned()),
                to: to.clone(),
                text: readable(&body),
                at: text(entry, &["timestamp", "time", "at"]).unwrap_or_default(),
                read: entry.get("read").and_then(Value::as_bool).unwrap_or(false),
            });
        }
    }
    messages.sort_by(|a, b| a.at.cmp(&b.at));
    let mut tasks: Vec<Task> = json_files(tasks)
        .iter()
        .filter_map(|file| {
            let task = read_json(file)?;
            Some(Task {
                id: text(&task, &["id"])
                    .or_else(|| file.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
                    .unwrap_or_default(),
                subject: text(&task, &["subject", "title"])?,
                status: text(&task, &["status"]).unwrap_or_else(|| "pending".to_owned()),
                owner: text(&task, &["owner"]).filter(|owner| !owner.is_empty()),
            })
        })
        .collect();
    // Numbered ids in their order; others after, by name.
    tasks.sort_by_key(|task| (task.id.parse::<u64>().unwrap_or(u64::MAX), task.id.clone()));
    Team { name, members, messages, tasks }
}

/// A message as people read it: a structured one (an idle note, a shutdown request) is JSON, said
/// by its type and summary.
fn readable(body: &str) -> String {
    let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(body) else { return body.to_owned() };
    let kind = fields.get("type").and_then(Value::as_str).unwrap_or("message").replace('_', " ");
    let detail = ["summary", "reason", "content", "message"]
        .iter()
        .find_map(|key| fields.get(*key).and_then(Value::as_str))
        .map(|detail| format!(": {detail}"))
        .unwrap_or_default();
    format!("{kind}{detail}")
}

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match value.get(*key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn read_json(file: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

fn json_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "json"))
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_team_its_messages_and_tasks() {
        let home = tempfile::tempdir().expect("temp dir");
        let session = "0b6f7c2e-1a2b-4c3d-9e8f-001122334455";
        let team = home.path().join("teams").join("session-0b6f7c2e");
        let tasks = home.path().join("tasks").join("session-0b6f7c2e");
        std::fs::create_dir_all(team.join("inboxes")).expect("folders");
        std::fs::create_dir_all(&tasks).expect("folders");
        std::fs::write(
            team.join("config.json"),
            r#"{"members":[{"name":"team-lead","agentId":"a1","agentType":"team-lead"},{"name":"tester","agentId":"a2","agentType":"general-purpose"}]}"#,
        )
        .expect("write");
        std::fs::write(
            team.join("inboxes").join("team-lead.json"),
            r#"[{"from":"tester","text":"All 40 tests pass","timestamp":"2026-10-20T10:02:00Z","read":true},
                {"from":"tester","text":"{\"type\":\"idle_notification\",\"summary\":\"waiting for work\"}","timestamp":"2026-10-20T10:03:00Z","read":false}]"#,
        )
        .expect("write");
        std::fs::write(
            team.join("inboxes").join("tester.json"),
            r#"[{"from":"team-lead","text":"Run the checkout tests","timestamp":"2026-10-20T10:00:00Z","read":true}]"#,
        )
        .expect("write");
        std::fs::write(tasks.join("2.json"), r#"{"id":"2","subject":"Fix flaky test","status":"pending"}"#)
            .expect("write");
        std::fs::write(
            tasks.join("1.json"),
            r#"{"id":"1","subject":"Run tests","status":"completed","owner":"tester"}"#,
        )
        .expect("write");

        let team = for_session(home.path(), session).expect("a team");
        assert_eq!(team.name, "session-0b6f7c2e");
        assert_eq!(team.members.iter().map(|member| member.name.as_str()).collect::<Vec<_>>(), ["team-lead", "tester"]);
        let said: Vec<(&str, &str, &str)> = team
            .messages
            .iter()
            .map(|message| (message.from.as_str(), message.to.as_str(), message.text.as_str()))
            .collect();
        assert_eq!(
            said,
            [
                ("team-lead", "tester", "Run the checkout tests"),
                ("tester", "team-lead", "All 40 tests pass"),
                ("tester", "team-lead", "idle notification: waiting for work"),
            ]
        );
        assert_eq!(team.unread(), 1);
        assert_eq!(
            team.tasks.iter().map(|task| task.subject.as_str()).collect::<Vec<_>>(),
            ["Run tests", "Fix flaky test"]
        );
        assert_eq!(team.progress(), (1, 2));
        assert_eq!(for_session(home.path(), "ffffffff-0000-0000-0000-000000000000"), None);
    }
}
