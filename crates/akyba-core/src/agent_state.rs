//! What a terminal tab shows about the program in it, and how its name is read from the terminal
//! title. Matches `AgentStatus` in the design system.

use crate::program_status::ProgramStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentState {
    /// A plain shell; no agent reported anything.
    #[default]
    Shell,
    Idle,
    Working,
    NeedsYou,
    YourTurn,
    Error,
}

impl AgentState {
    /// States that ask for the user, and so light a pane up until it has been looked at.
    pub fn asks_for_you(self) -> bool {
        matches!(self, Self::NeedsYou | Self::YourTurn | Self::Error)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::NeedsYou => "Needs you",
            Self::YourTurn => "Your turn",
            Self::Error => "Error",
        }
    }

    /// Maps Claude Code's OSC 7501 report to a tab state. Unknown future states read as working.
    pub fn from_program_status(status: &ProgramStatus) -> Self {
        match status.state.as_str() {
            "clear" => Self::Shell,
            "idle" => Self::Idle,
            "blocked" => Self::NeedsYou,
            "done" => Self::YourTurn,
            "error" => Self::Error,
            _ => Self::Working,
        }
    }
}

/// What Codex's terminal title (its default one) says it's doing: `⠋ thread | project` with a
/// turning spinner while it works, `[ ! ] Action Required | …` (blinking with `[ . ]`) while it
/// waits for an approval, the same without the spinner when it's idle. Codex reports no state of its
/// own, so a turn that ends is the spinner going away after it was working or waiting: your turn.
pub fn from_codex_title(title: &str, previous: AgentState) -> AgentState {
    // A dot in front while it listens to the microphone.
    let title = title.trim_start_matches('●').trim_start();
    if CODEX_ACTION_REQUIRED.iter().any(|prefix| title.starts_with(prefix)) {
        AgentState::NeedsYou
    } else if title.chars().next().is_some_and(is_braille) {
        AgentState::Working
    } else {
        match previous {
            AgentState::Working | AgentState::NeedsYou | AgentState::YourTurn => AgentState::YourTurn,
            AgentState::Error => AgentState::Error,
            AgentState::Shell | AgentState::Idle => AgentState::Idle,
        }
    }
}

/// The two phases of Codex's blinking "needs you" title.
const CODEX_ACTION_REQUIRED: [&str; 2] = ["[ ! ] Action Required", "[ . ] Action Required"];

fn is_braille(c: char) -> bool {
    ('\u{2801}'..='\u{28ff}').contains(&c)
}

/// Spinner glyphs Claude Code may put in front of its terminal title.
fn is_spinner(c: char) -> bool {
    matches!(c, '·' | '✢' | '✳' | '✶' | '✻' | '✽' | '*') || is_braille(c)
}

/// A terminal title as a short tab name: Claude's session name, Codex's thread (or project) name,
/// or the shell's folder for titles like oh-my-zsh's `user@host: ~/some/path`. Only a `codex`
/// title is cut at its first ` | `: a session name of Claude's may hold one of its own.
pub fn tab_title(title: &str, codex: bool) -> Option<String> {
    let mut trimmed = title.trim();
    // Codex: `[ ! ] Action Required | thread | project`, `⠋ thread | project`.
    if codex {
        if let Some(prefix) = CODEX_ACTION_REQUIRED.iter().find(|prefix| trimmed.starts_with(*prefix)) {
            trimmed = trimmed[prefix.len()..].trim_start_matches([' ', '|']);
        }
        trimmed = trimmed.split(" | ").next().unwrap_or(trimmed);
    }
    if let Some(first) = trimmed.chars().next()
        && is_spinner(first)
    {
        trimmed = trimmed[first.len_utf8()..].trim_start();
    }
    if trimmed.is_empty() {
        return None;
    }
    if let Some((user_host, path)) = trimmed.split_once(':')
        && let Some((user, host)) = user_host.split_once('@')
        && !user.is_empty()
        && !host.is_empty()
        && !user_host.contains(char::is_whitespace)
    {
        let path = path.trim().trim_end_matches('/');
        let folder = path.rsplit('/').next().filter(|f| !f.is_empty()).unwrap_or(path);
        return (!folder.is_empty()).then(|| folder.to_owned());
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> AgentState {
        AgentState::from_program_status(&ProgramStatus { state: name.into(), ..Default::default() })
    }

    #[test]
    fn maps_program_status_to_tab_states() {
        assert_eq!(state("idle"), AgentState::Idle);
        assert_eq!(state("working"), AgentState::Working);
        assert_eq!(state("blocked"), AgentState::NeedsYou);
        assert_eq!(state("done"), AgentState::YourTurn);
        assert_eq!(state("clear"), AgentState::Shell);
        assert_eq!(state("error"), AgentState::Error);
        assert_eq!(state("something-new"), AgentState::Working);
    }

    #[test]
    fn keeps_a_claude_session_name() {
        assert_eq!(tab_title("ABC-236", false).as_deref(), Some("ABC-236"));
    }

    #[test]
    fn drops_a_leading_spinner_glyph() {
        assert_eq!(tab_title("✳ ABC-236", false).as_deref(), Some("ABC-236"));
    }

    #[test]
    fn turns_a_shell_prompt_title_into_its_folder() {
        assert_eq!(tab_title("dev@Mac: ~/Code/acme-app", false).as_deref(), Some("acme-app"));
        assert_eq!(tab_title("jonasarden@Mac:~", false).as_deref(), Some("~"));
    }

    #[test]
    fn reads_what_codex_does_from_its_title() {
        use AgentState::{Idle, NeedsYou, Working, YourTurn};
        assert_eq!(from_codex_title("acme-web", AgentState::Shell), Idle, "just started");
        assert_eq!(from_codex_title("⠋ fix-login | acme-web", Idle), Working);
        assert_eq!(from_codex_title("[ ! ] Action Required | fix-login | acme-web", Working), NeedsYou);
        assert_eq!(from_codex_title("[ . ] Action Required | fix-login | acme-web", NeedsYou), NeedsYou);
        assert_eq!(from_codex_title("⠹ fix-login | acme-web", NeedsYou), Working, "approved");
        assert_eq!(from_codex_title("fix-login | acme-web", Working), YourTurn, "the turn ended");
        assert_eq!(from_codex_title("fix-login | acme-web", YourTurn), YourTurn);
        assert_eq!(from_codex_title("● ⠋ fix-login", Idle), Working, "listening");
    }

    #[test]
    fn names_a_codex_tab_after_its_thread() {
        assert_eq!(tab_title("⠋ fix-login | acme-web", true).as_deref(), Some("fix-login"));
        assert_eq!(tab_title("[ ! ] Action Required | fix-login | acme-web", true).as_deref(), Some("fix-login"));
        assert_eq!(tab_title("acme-web", true).as_deref(), Some("acme-web"));
    }

    #[test]
    fn keeps_a_claude_session_name_whole() {
        let name = "ABC | Import the bank statements";
        assert_eq!(tab_title(&format!("✳ {name}"), false).as_deref(), Some(name));
    }

    #[test]
    fn gives_nothing_for_an_empty_title() {
        assert_eq!(tab_title("   ", false), None);
    }
}
