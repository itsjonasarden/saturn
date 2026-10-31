//! The coding agents Akyba runs in its panes, Claude Code and Codex, and what differs between them:
//! the program, how it starts with a first message, how a session resumes, where it keeps its
//! sessions. Which of them a Mac has is found, not set: on the login `PATH`, and logged in.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    /// What Akyba ran before it knew others, and what anything saved before then was.
    #[default]
    Claude,
    Codex,
}

impl Agent {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    /// Its name, as people know it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    /// The program that runs it, as typed and as the process is named.
    #[must_use]
    pub fn program(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The agent a process is, by its name.
    #[must_use]
    pub fn of_program(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.program() == name.trim_start_matches('-'))
    }

    /// The agent a command starts, by its first word.
    #[must_use]
    pub fn of_command(command: &str) -> Option<Self> {
        command.split_whitespace().next().and_then(Self::of_program)
    }

    /// The command that starts it in a pane, with `prompt` as its first message when there is one,
    /// quoted for the shell by `quote`.
    #[must_use]
    pub fn start(self, prompt: Option<&str>, quote: impl Fn(&str) -> String) -> String {
        match prompt {
            Some(prompt) => format!("{} {}", self.program(), quote(prompt)),
            None => self.program().to_owned(),
        }
    }

    /// The command that resumes `session` in the folder it ran in.
    #[must_use]
    pub fn resume(self, session: &str) -> String {
        match self {
            Self::Claude => format!("claude --resume {session}"),
            Self::Codex => format!("codex resume {session}"),
        }
    }

    /// The keys that answer its permission prompt, allowing or refusing: Claude Code's highlighted
    /// "Yes" and Escape; Codex's `y` and `d` (deny).
    #[must_use]
    pub fn answer_keys(self, allow: bool) -> &'static [u8] {
        match (self, allow) {
            (Self::Claude, true) => b"\r",
            (Self::Claude, false) => b"\x1b",
            (Self::Codex, true) => b"y",
            (Self::Codex, false) => b"d",
        }
    }

    /// What starting it on a ticket runs, before a project chooses its own: Claude Code names its
    /// session after the ticket (`-n`); Codex has no such flag.
    #[must_use]
    pub fn ticket_command(self) -> &'static str {
        match self {
            Self::Claude => "claude -n {ticket}",
            Self::Codex => "codex",
        }
    }

    /// Where it keeps its settings and sessions: `CLAUDE_CONFIG_DIR` or `CODEX_HOME` when set, else
    /// `~/.claude` or `~/.codex`.
    #[must_use]
    pub fn home(self) -> Option<PathBuf> {
        let (variable, folder) = match self {
            Self::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
            Self::Codex => ("CODEX_HOME", ".codex"),
        };
        std::env::var_os(variable)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(folder)))
    }

    /// The command that says whether it's logged in, without starting a session.
    fn login_check(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["auth", "status"],
            Self::Codex => &["login", "status"],
        }
    }

    /// Whether its login check says it's logged in: Claude Code answers JSON with `loggedIn`, Codex
    /// a sentence (`Logged in using ChatGPT`) and success.
    #[must_use]
    pub fn logged_in_from(self, output: &str, success: bool) -> bool {
        match self {
            // The JSON comes first; a warning on stderr may follow it.
            Self::Claude => serde_json::Deserializer::from_str(output)
                .into_iter::<serde_json::Value>()
                .next()
                .and_then(Result::ok)
                .and_then(|status| status.get("loggedIn")?.as_bool())
                .unwrap_or(false),
            Self::Codex => success && output.contains("Logged in"),
        }
    }
}

/// The Codex profile Akyba starts its Codex sessions with (`codex -p akyba`), layered on the
/// person's own `config.toml`: Codex sends its notifications to the terminal (OSC 9) whatever the
/// terminal, and, when `notify` is the program given (the person has none of their own), runs it as
/// each turn ends with the session's id and last answer.
pub const CODEX_PROFILE: &str = "akyba";

/// What Codex shows under its prompt with Akyba's status line on: its own items, as `/statusline`
/// would set them.
pub const CODEX_STATUS_LINE: [&str; 6] =
    ["model-with-reasoning", "current-dir", "git-branch", "context-used", "five-hour-limit", "weekly-limit"];

/// Akyba's settings set in the profile file `existing` (empty for a new one): `notify` when given,
/// Codex's notifications to the terminal, and its status line when `status_line`. Everything else
/// stays as it is: Codex keeps its own choices in the profile it runs with (the folders you trust
/// in a session Akyba started, for one), and those must outlast Akyba's next start.
#[must_use]
pub fn codex_profile(existing: &str, notify: Option<&str>, status_line: bool) -> String {
    use toml_edit::{Array, DocumentMut, Item, Table, value};
    let mut profile = existing.parse::<DocumentMut>().unwrap_or_default();
    if profile.is_empty() {
        profile.decor_mut().set_prefix(
            "# Akyba's profile for the Codex sessions it starts (codex -p akyba), over your config.toml.\n\
             # Akyba sets notify and [tui]'s notification_method, notification_condition and status_line\n\
             # as it starts; anything else here is kept.\n",
        );
    }
    match notify {
        Some(program) => profile["notify"] = value(Array::from_iter([program])),
        None => {
            profile.remove("notify");
        }
    }
    if !profile.contains_table("tui") {
        profile.insert("tui", Item::Table(Table::new()));
    }
    let tui = &mut profile["tui"];
    tui["notification_method"] = value("osc9");
    tui["notification_condition"] = value("always");
    if status_line {
        tui["status_line"] = value(Array::from_iter(CODEX_STATUS_LINE));
    } else if let Some(table) = tui.as_table_mut() {
        table.remove("status_line");
    }
    profile.to_string()
}

/// `command` with Akyba's Codex profile when it starts Codex without a profile of its own: after
/// `codex` (`codex -p akyba 'Fix the login'`, `codex -p akyba resume …`).
#[must_use]
pub fn with_codex_profile(command: &str) -> String {
    let Some(rest) = command.strip_prefix("codex") else { return command.to_owned() };
    let words: Vec<&str> = rest.split_whitespace().collect();
    let has_profile = words.iter().any(|word| *word == "-p" || word.starts_with("--profile"));
    if !(rest.is_empty() || rest.starts_with(' ')) || has_profile {
        return command.to_owned();
    }
    format!("codex -p {CODEX_PROFILE}{rest}")
}

/// `command`, which starts an agent as Akyba writes it (`claude …`, `codex -p akyba resume …`),
/// started with `own` instead: the person's own command for that agent, an alias such as `cdsk`.
/// Akyba keeps writing the agent's own name until the last moment, so it can still tell which
/// agent a command starts. Anything else, or an empty `own`, is left as it is.
#[must_use]
pub fn with_own_command(command: &str, own: &str) -> String {
    let own = own.trim();
    let Some(agent) = Agent::of_command(command) else { return command.to_owned() };
    match command.strip_prefix(agent.program()) {
        Some(rest) if !own.is_empty() && (rest.is_empty() || rest.starts_with(' ')) => format!("{own}{rest}"),
        _ => command.to_owned(),
    }
}

/// What Codex's `notify` program gets as a turn ends (`agent-turn-complete`), as the hook input
/// Akyba reads from Claude Code: a `Stop` with the session's id, its folder and its last answer.
#[must_use]
pub fn codex_turn_as_hook(notification: &serde_json::Value) -> Option<serde_json::Value> {
    if notification.get("type").and_then(serde_json::Value::as_str) != Some("agent-turn-complete") {
        return None;
    }
    Some(serde_json::json!({
        "hook_event_name": "Stop",
        "session_id": notification.get("thread-id")?,
        "cwd": notification.get("cwd"),
        "last_assistant_message": notification.get("last-assistant-message"),
    }))
}

/// What Akyba found of an agent on this Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub agent: Agent,
    /// Its program is on the login `PATH`.
    pub installed: bool,
    /// It says it's logged in, so it can run.
    pub logged_in: bool,
}

impl Found {
    /// Installed and logged in: an agent Akyba can start.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.installed && self.logged_in
    }
}

/// Finds each agent with `path` as the `PATH` (the login shell's: Akyba started from Finder has
/// little of its own). Runs their login checks, which take a moment: off the main thread.
#[must_use]
pub fn detect(path: &str) -> Vec<Found> {
    Agent::ALL
        .into_iter()
        .map(|agent| {
            let output = Command::new(agent.program()).args(agent.login_check()).env("PATH", path).output();
            match output {
                Ok(output) => {
                    // Codex says it on stderr, Claude Code on stdout.
                    let said = [output.stdout, output.stderr].concat();
                    Found {
                        agent,
                        installed: true,
                        logged_in: agent.logged_in_from(&String::from_utf8_lossy(&said), output.status.success()),
                    }
                }
                Err(_) => Found { agent, installed: false, logged_in: false },
            }
        })
        .collect()
}

/// The agent a new pane starts: the one chosen last when it's ready, else the first ready one, else
/// Claude Code (whose missing program the shell then reports).
#[must_use]
pub fn pick(found: &[Found], last: Option<Agent>) -> Agent {
    let ready = |agent: Agent| found.iter().any(|found| found.agent == agent && found.ready());
    last.filter(|agent| ready(*agent))
        .or_else(|| Agent::ALL.into_iter().find(|agent| ready(*agent)))
        .unwrap_or(Agent::Claude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(text: &str) -> String {
        format!("'{text}'")
    }

    #[test]
    fn starts_and_resumes_each_agent() {
        assert_eq!(Agent::Claude.start(Some("Fix the login"), quote), "claude 'Fix the login'");
        assert_eq!(Agent::Codex.start(Some("Fix the login"), quote), "codex 'Fix the login'");
        assert_eq!(Agent::Codex.start(None, quote), "codex");
        assert_eq!(Agent::Claude.resume("0b6f7c2e"), "claude --resume 0b6f7c2e");
        assert_eq!(Agent::Codex.resume("019d7c54"), "codex resume 019d7c54");
        assert_eq!(Agent::of_program("-codex"), Some(Agent::Codex));
        assert_eq!(Agent::of_command("claude -n ABC-123"), Some(Agent::Claude));
        assert_eq!(Agent::of_command("aider"), None);
        assert_eq!(Agent::Codex.answer_keys(true), b"y");
        assert_eq!(Agent::Claude.answer_keys(false), b"\x1b");
    }

    #[test]
    fn starts_an_agent_with_the_persons_own_command() {
        assert_eq!(with_own_command("claude --resume 0b6f7c2e", "cdsk"), "cdsk --resume 0b6f7c2e");
        assert_eq!(with_own_command("claude", " cdsk "), "cdsk");
        assert_eq!(with_own_command("codex -p akyba 'Fix it'", "cx --yolo"), "cx --yolo -p akyba 'Fix it'");
        assert_eq!(with_own_command("claude 'Fix it'", ""), "claude 'Fix it'");
        assert_eq!(with_own_command("aider 'Fix it'", "cdsk"), "aider 'Fix it'");
    }

    #[test]
    fn starts_codex_with_akybas_profile() {
        assert_eq!(with_codex_profile("codex 'Fix it'"), "codex -p akyba 'Fix it'");
        assert_eq!(with_codex_profile("codex"), "codex -p akyba");
        assert_eq!(with_codex_profile("codex resume 019d"), "codex -p akyba resume 019d");
        assert_eq!(with_codex_profile("codex -p work"), "codex -p work", "a profile of its own stays");
        assert_eq!(with_codex_profile("claude 'Fix it'"), "claude 'Fix it'");
        let profile =
            codex_profile("", Some("/Applications/Akyba.app/Contents/Resources/plugin/bin/codex-notify"), false);
        assert!(profile.contains("notify = [\"/Applications/Akyba.app/Contents/Resources/plugin/bin/codex-notify\"]"));
        assert!(profile.contains("notification_method = \"osc9\""));
        assert!(!codex_profile("", None, false).contains("notify ="));
        assert!(!profile.contains("status_line ="));
        assert!(codex_profile("", None, true).contains("status_line = [\"model-with-reasoning\", \"current-dir\""));
    }

    #[test]
    fn keeps_what_codex_writes_into_the_profile() {
        let codex_wrote = "notify = [\"/old/codex-notify\"]\nmodel = \"gpt-6\"\n\n[tui]\nnotification_method = \"osc9\"\nstatus_line = [\"model\"]\n\n[projects.\"/code/acme-web\"]\ntrust_level = \"trusted\"\n";
        let profile = codex_profile(codex_wrote, Some("/new/codex-notify"), false);
        assert!(profile.contains("[projects.\"/code/acme-web\"]\ntrust_level = \"trusted\""), "{profile}");
        assert!(profile.contains("model = \"gpt-6\""));
        assert!(profile.contains("notify = [\"/new/codex-notify\"]"));
        assert!(!profile.contains("status_line ="), "turned off");
        assert_eq!(codex_profile(&profile, Some("/new/codex-notify"), false), profile, "nothing to change");
    }

    #[test]
    fn reads_the_end_of_a_codex_turn_as_a_stop() {
        let notification = serde_json::json!({
            "type": "agent-turn-complete", "thread-id": "019d7c54", "turn-id": "1",
            "cwd": "/code/acme-web", "last-assistant-message": "Done: the tests pass.",
        });
        let hook = codex_turn_as_hook(&notification).expect("a stop");
        assert_eq!(hook["hook_event_name"], "Stop");
        assert_eq!(hook["session_id"], "019d7c54");
        assert_eq!(hook["last_assistant_message"], "Done: the tests pass.");
        assert_eq!(codex_turn_as_hook(&serde_json::json!({"type": "other"})), None);
    }

    #[test]
    fn reads_each_login_check() {
        assert!(Agent::Claude.logged_in_from(r#"{"loggedIn": true, "authMethod": "claude.ai"}"#, true));
        assert!(!Agent::Claude.logged_in_from(r#"{"loggedIn": false}"#, true));
        assert!(Agent::Claude.logged_in_from("{\"loggedIn\": true}\nwarning: something\n", true));
        assert!(Agent::Codex.logged_in_from("Logged in using ChatGPT\n", true));
        assert!(!Agent::Codex.logged_in_from("Not logged in\n", false));
    }

    #[test]
    fn picks_the_last_ready_agent_else_the_first() {
        let found = |claude: bool, codex: bool| {
            vec![
                Found { agent: Agent::Claude, installed: claude, logged_in: claude },
                Found { agent: Agent::Codex, installed: codex, logged_in: codex },
            ]
        };
        assert_eq!(pick(&found(true, true), Some(Agent::Codex)), Agent::Codex);
        assert_eq!(pick(&found(true, false), Some(Agent::Codex)), Agent::Claude, "Codex isn't there");
        assert_eq!(pick(&found(false, true), None), Agent::Codex, "the only one");
        assert_eq!(pick(&found(false, false), None), Agent::Claude);
    }
}
