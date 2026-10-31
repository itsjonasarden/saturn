//! What an agent is waiting for, in words, from Claude Code's hooks: a permission (and for what:
//! the command, the file), a question, the end of its turn, or a failure. One list of these says
//! where you're needed without visiting every pane.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    /// Asking to use a tool: its name and what it would do.
    Permission { tool: String, detail: String },
    /// Asking something.
    Question(String),
    /// Its turn is over: the start of what it said last.
    Done(Option<String>),
    /// The turn failed, and why.
    Failed(String),
}

/// What a hook says about waiting: `Some(Some(…))` it now waits for this, `Some(None)` it's working
/// again, `None` it says nothing about it.
pub fn from_hook(event: &str, input: &Value) -> Option<Option<Waiting>> {
    let text = |name: &str| input.get(name).and_then(Value::as_str).map(str::to_owned);
    match event {
        "PermissionRequest" => {
            let tool = text("tool_name").unwrap_or_default();
            let detail = describe(&tool, input.get("tool_input").unwrap_or(&Value::Null));
            Some(Some(Waiting::Permission { tool, detail }))
        }
        "Notification" => match text("notification_type").as_deref() {
            Some("elicitation_dialog" | "agent_needs_input") => {
                text("message").map(|message| Some(Waiting::Question(message)))
            }
            _ => None,
        },
        "Stop" => Some(Some(Waiting::Done(text("last_assistant_message").as_deref().and_then(first_line)))),
        "StopFailure" => Some(Some(Waiting::Failed(text("error").unwrap_or_else(|| "The turn failed.".to_owned())))),
        "UserPromptSubmit" | "PostToolUse" | "SessionEnd" => Some(None),
        _ => None,
    }
}

/// What a Codex notification (OSC 9) says it waits for: an approval for a command or an edit, a plan
/// to approve, else the end of its turn with the start of its answer ("Agent turn complete" when
/// it has none to show).
#[must_use]
pub fn from_codex_notification(text: &str) -> Waiting {
    let text = text.trim();
    if let Some(command) = text.strip_prefix("Approval requested: ") {
        Waiting::Permission { tool: "Bash".to_owned(), detail: command.to_owned() }
    } else if let Some(files) = text.strip_prefix("Codex wants to edit ") {
        Waiting::Permission { tool: "Edit".to_owned(), detail: files.to_owned() }
    } else if let Some(server) = text.strip_prefix("Approval requested by ") {
        Waiting::Permission { tool: server.to_owned(), detail: String::new() }
    } else if let Some(plan) = text.strip_prefix("Plan mode prompt: ") {
        Waiting::Question(plan.to_owned())
    } else if text.is_empty() || text == "Agent turn complete" {
        Waiting::Done(None)
    } else {
        Waiting::Done(first_line(text))
    }
}

/// What a tool would do, in a line: the command a shell runs, the file an edit writes, the address a
/// fetch reads; for anything else its first piece of text.
pub fn describe(tool: &str, input: &Value) -> String {
    let field = |name: &str| input.get(name).and_then(Value::as_str);
    let found = match tool {
        "Bash" => field("command"),
        "Edit" | "Write" | "MultiEdit" | "Read" | "NotebookEdit" => {
            field("file_path").or_else(|| field("notebook_path"))
        }
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        _ => None,
    };
    let found = found
        .map(str::to_owned)
        .or_else(|| input.as_object().and_then(|fields| fields.values().find_map(Value::as_str)).map(str::to_owned));
    let line = found.as_deref().and_then(first_line).unwrap_or_default();
    match tool.strip_prefix("mcp__").and_then(|rest| rest.split_once("__")) {
        Some((server, name)) if line.is_empty() => format!("{server}: {name}"),
        Some((server, name)) => format!("{server}: {name} · {line}"),
        None => line,
    }
}

/// The first line that says something, at most 200 characters.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(if line.chars().count() > 200 {
        format!("{}…", line.chars().take(200).collect::<String>())
    } else {
        line.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn says_what_a_permission_is_for() {
        let asked = from_hook(
            "PermissionRequest",
            &json!({"tool_name": "Bash", "tool_input": {"command": "rm -rf dist\n# then build", "description": "Clean"}}),
        );
        assert_eq!(asked, Some(Some(Waiting::Permission { tool: "Bash".into(), detail: "rm -rf dist".into() })));
        assert_eq!(
            describe("Edit", &json!({"file_path": "/code/acme-web/src/cart.ts", "old_string": "a"})),
            "/code/acme-web/src/cart.ts"
        );
        assert_eq!(
            describe("mcp__github__create_issue", &json!({"title": "Flaky test"})),
            "github: create_issue · Flaky test"
        );
        assert_eq!(describe("mcp__db__list", &json!({})), "db: list");
    }

    #[test]
    fn follows_a_turn() {
        assert_eq!(from_hook("UserPromptSubmit", &json!({})), Some(None));
        assert_eq!(
            from_hook("Stop", &json!({"last_assistant_message": "\n\nDone: the cart rounds to cents now.\nDetails…"})),
            Some(Some(Waiting::Done(Some("Done: the cart rounds to cents now.".into()))))
        );
        assert_eq!(
            from_hook("StopFailure", &json!({"error": "rate_limit"})),
            Some(Some(Waiting::Failed("rate_limit".into())))
        );
        assert_eq!(
            from_hook(
                "Notification",
                &json!({"notification_type": "elicitation_dialog", "message": "Which database?"})
            ),
            Some(Some(Waiting::Question("Which database?".into())))
        );
        assert_eq!(from_hook("Notification", &json!({"notification_type": "idle_prompt"})), None);
        assert_eq!(from_hook("PreToolUse", &json!({})), None);
    }

    #[test]
    fn reads_what_codex_notifies() {
        assert_eq!(
            from_codex_notification("Approval requested: npm test"),
            Waiting::Permission { tool: "Bash".to_owned(), detail: "npm test".to_owned() }
        );
        assert_eq!(
            from_codex_notification("Codex wants to edit src/app.ts"),
            Waiting::Permission { tool: "Edit".to_owned(), detail: "src/app.ts".to_owned() }
        );
        assert_eq!(from_codex_notification("Plan mode prompt: Ship it"), Waiting::Question("Ship it".to_owned()));
        assert_eq!(from_codex_notification("Agent turn complete"), Waiting::Done(None));
        assert!(matches!(from_codex_notification("All 40 tests pass"), Waiting::Done(Some(_))));
    }
}
