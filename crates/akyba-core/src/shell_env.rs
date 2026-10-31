//! The environment a pane's shell starts with.

/// Variables a running Claude Code session sets for its children. Akyba may itself be started from
/// a terminal where Claude runs; passed on, they make every Claude in a Akyba pane think it is a
/// child session, which turns its transcript off: no `/resume`, no saved name (Phase 0).
fn is_claude_session_marker(key: &str) -> bool {
    matches!(
        key,
        "CLAUDECODE"
            | "CLAUDE_PID"
            | "CLAUDE_EFFORT"
            | "CLAUDE_CODE_ENTRYPOINT"
            | "CLAUDE_CODE_EXECPATH"
            | "CLAUDE_CODE_CHILD_SESSION"
    ) || key.starts_with("CLAUDE_CODE_SESSION_")
        || key.starts_with("CLAUDE_CODE_MESSAGING_")
}

/// Marks where the login shell's `PATH` starts in its output, past anything its startup files print.
const PATH_MARKER: &str = "__AKYBA_PATH__";

/// The `PATH` the user's login shell sets up. An app started from Finder gets only the system's
/// short `PATH`, without Homebrew, nvm or `~/.cargo/bin`; tools Akyba runs (formatters) need the
/// real one. `None` when the shell cannot be asked.
pub fn login_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
    let output = std::process::Command::new(shell)
        .args(["-ilc", &format!("printf '\\n{PATH_MARKER}%s\\n' \"$PATH\"")])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    parse_login_path(&String::from_utf8_lossy(&output.stdout))
}

fn parse_login_path(output: &str) -> Option<String> {
    output.lines().find_map(|line| line.strip_prefix(PATH_MARKER)).map(str::to_owned).filter(|path| !path.is_empty())
}

/// The app's own environment minus Claude's session markers, with `extra` added last.
pub fn shell_env<I, K, V>(base: I, extra: &[(&str, &str)]) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let mut env: Vec<(String, String)> = base
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .filter(|(key, _)| !is_claude_session_marker(key) && !extra.iter().any(|(e, _)| e == key))
        .collect();
    env.extend(extra.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())));
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(env: &[(String, String)]) -> Vec<&str> {
        env.iter().map(|(key, _)| key.as_str()).collect()
    }

    #[test]
    fn drops_the_markers_a_parent_claude_session_sets() {
        let base = [
            ("PATH", "/usr/bin"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_PID", "42"),
            ("CLAUDE_EFFORT", "high"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CLAUDE_CODE_EXECPATH", "/x"),
            ("CLAUDE_CODE_SESSION_ID", "abc"),
            ("CLAUDE_CODE_SESSION_ATTENDED", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/s"),
            ("CLAUDE_CODE_MESSAGING_TOKEN", "t"),
        ];
        assert_eq!(keys(&shell_env(base, &[])), ["PATH"]);
    }

    #[test]
    fn keeps_claude_settings_a_user_sets_on_purpose() {
        let base = [("CLAUDE_CONFIG_DIR", "/c"), ("CLAUDE_CODE_DISABLE_MOUSE", "1")];
        assert_eq!(keys(&shell_env(base, &[])), ["CLAUDE_CONFIG_DIR", "CLAUDE_CODE_DISABLE_MOUSE"]);
    }

    #[test]
    fn extras_replace_inherited_values() {
        let env = shell_env([("TERM", "dumb")], &[("TERM", "xterm-256color")]);
        assert_eq!(env, [("TERM".to_owned(), "xterm-256color".to_owned())]);
    }

    #[test]
    fn the_login_path_is_read_past_whatever_the_shell_prints() {
        let output = "Welcome back!\n\n__AKYBA_PATH__/opt/homebrew/bin:/usr/bin\n";
        assert_eq!(parse_login_path(output).as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
        assert_eq!(parse_login_path("no marker here"), None);
    }
}
