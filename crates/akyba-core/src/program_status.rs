//! Claude Code's OSC 7501 "Program Status Protocol" (Claude Code 2.1.295 and later).
//!
//! Claude asks `ESC ] 7501 ; ? ST` once at startup. A terminal that answers with any OSC 7501 whose
//! data starts with `?` then receives `state=<state>:app=claude-code[:id=..][:kind=..][:progress=N]
//! [:title=<base64>][:msg=<base64>]` as the session changes, and `state=clear` when it exits.
//! Verified against a real Claude Code session.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub const PROGRAM_STATUS_OSC: u16 = 7501;

/// The reply that tells Claude this terminal understands the protocol.
pub const PROBE_REPLY: &[u8] = b"\x1b]7501;?\x1b\\";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProgramStatus {
    pub state: String,
    pub app: Option<String>,
    pub id: Option<String>,
    pub kind: Option<String>,
    pub progress: Option<u8>,
    pub title: Option<String>,
    pub msg: Option<String>,
}

/// Whether the OSC data is Claude's question "do you understand OSC 7501?".
pub fn is_probe(data: &str) -> bool {
    data.starts_with('?')
}

fn decode(value: &str) -> Option<String> {
    let bytes = STANDARD.decode(value).ok()?;
    String::from_utf8(bytes).ok()
}

/// Reads one status report. Fields that can't be read are left out rather than failing the report.
pub fn parse(data: &str) -> Option<ProgramStatus> {
    let mut status = ProgramStatus::default();
    for part in data.split(':') {
        let Some((key, value)) = part.split_once('=') else { continue };
        match key {
            "state" => value.clone_into(&mut status.state),
            "app" => status.app = Some(value.to_owned()),
            "id" => status.id = Some(value.to_owned()),
            "kind" => status.kind = Some(value.to_owned()),
            "progress" => status.progress = value.parse().ok(),
            "title" => status.title = decode(value),
            "msg" => status.msg = decode(value),
            _ => {}
        }
    }
    (!status.state.is_empty()).then_some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(text: &str) -> String {
        STANDARD.encode(text)
    }

    #[test]
    fn reads_state_kind_and_base64_fields() {
        let data = format!(
            "state=blocked:app=claude-code:id=7:kind=permission:title={}:msg={}",
            b64("ABC-236 ✳"),
            b64("Run pnpm test?")
        );
        assert_eq!(
            parse(&data),
            Some(ProgramStatus {
                state: "blocked".into(),
                app: Some("claude-code".into()),
                id: Some("7".into()),
                kind: Some("permission".into()),
                progress: None,
                title: Some("ABC-236 ✳".into()),
                msg: Some("Run pnpm test?".into()),
            })
        );
    }

    #[test]
    fn reads_progress_only_when_present() {
        assert_eq!(parse("state=working:progress=40").and_then(|s| s.progress), Some(40));
        assert_eq!(parse("state=working").and_then(|s| s.progress), None);
    }

    #[test]
    fn reads_the_clear_message_sent_on_exit() {
        assert_eq!(parse("state=clear").map(|s| s.state), Some("clear".into()));
    }

    #[test]
    fn ignores_data_without_a_state() {
        assert_eq!(parse("app=claude-code"), None);
    }

    #[test]
    fn drops_a_title_that_is_not_base64_instead_of_failing() {
        assert_eq!(parse("state=working:title=%%%").map(|s| s.title), Some(None));
    }

    #[test]
    fn recognises_the_probe() {
        assert!(is_probe("?"));
        assert!(!is_probe("state=working"));
    }
}
