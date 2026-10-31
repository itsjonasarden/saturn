//! Reads the signals programs send to their terminal that alacritty doesn't keep: Claude Code's
//! OSC 7501 status, the working folder (OSC 7), shell prompt marks (OSC 133) and notifications
//! (OSC 9, which Codex sends: an approval it waits for, the end of a turn). It sees the same
//! bytes just before alacritty does.

use std::path::PathBuf;

use akyba_core::program_status::{self, PROGRAM_STATUS_OSC, ProgramStatus};
use alacritty_terminal::vte::{Parser, Perform};

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// Claude asks whether this terminal understands OSC 7501.
    StatusProbe,
    Status(ProgramStatus),
    /// The shell reported its working folder.
    Cwd(PathBuf),
    /// A new shell prompt starts (OSC 133;A): whatever ran before has finished.
    PromptStart,
    /// A notification for the person at the terminal (OSC 9), its text.
    Notification(String),
}

#[derive(Default)]
struct Collector {
    signals: Vec<Signal>,
}

/// Decodes `%XX` escapes in an OSC 7 path.
fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = path.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

impl Perform for Collector {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let Some((code, rest)) = params.split_first() else { return };
        let data = rest.iter().map(|part| String::from_utf8_lossy(part)).collect::<Vec<_>>().join(";");
        match std::str::from_utf8(code).ok().and_then(|code| code.parse::<u16>().ok()) {
            Some(PROGRAM_STATUS_OSC) if program_status::is_probe(&data) => self.signals.push(Signal::StatusProbe),
            Some(PROGRAM_STATUS_OSC) => {
                if let Some(status) = program_status::parse(&data) {
                    self.signals.push(Signal::Status(status));
                }
            }
            Some(7) => {
                // file://host/path
                if let Some(path) = data.strip_prefix("file://").and_then(|rest| rest.find('/').map(|at| &rest[at..])) {
                    self.signals.push(Signal::Cwd(PathBuf::from(percent_decode(path))));
                }
            }
            Some(133) if data.starts_with('A') => self.signals.push(Signal::PromptStart),
            // `9;4;…` is a progress report (ConEmu's), not a notification.
            Some(9) if !data.is_empty() && !data.starts_with("4;") => self.signals.push(Signal::Notification(data)),
            _ => {}
        }
    }
}

#[derive(Default)]
pub struct SignalTap {
    parser: Parser,
    collector: Collector,
}

impl SignalTap {
    /// Reads a chunk of terminal output; sequences split across chunks are kept until complete.
    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Signal> {
        self.parser.advance(&mut self.collector, bytes);
        std::mem::take(&mut self.collector.signals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_notifications_but_not_progress() {
        let mut tap = SignalTap::default();
        let signals = tap.advance(b"\x1b]9;Approval requested: npm test\x07\x1b]9;4;1;50\x07");
        assert_eq!(signals, [Signal::Notification("Approval requested: npm test".to_owned())]);
    }

    #[test]
    fn reads_the_probe_and_a_status_report() {
        let mut tap = SignalTap::default();
        let signals = tap.advance(b"hello\x1b]7501;?\x07world\x1b]7501;state=working:app=claude-code\x1b\\");
        assert_eq!(signals.len(), 2);
        assert_eq!(signals[0], Signal::StatusProbe);
        assert!(matches!(&signals[1], Signal::Status(status) if status.state == "working"));
    }

    #[test]
    fn keeps_a_sequence_split_across_chunks() {
        let mut tap = SignalTap::default();
        assert_eq!(tap.advance(b"\x1b]7501;state=do"), []);
        let signals = tap.advance(b"ne:app=claude-code\x07");
        assert!(matches!(&signals[..], [Signal::Status(status)] if status.state == "done"));
    }

    #[test]
    fn reads_the_working_folder_and_prompt_marks() {
        let mut tap = SignalTap::default();
        let signals = tap.advance(b"\x1b]7;file://Mac.local/Users/jonas/My%20Code\x07\x1b]133;A\x07");
        assert_eq!(signals, [Signal::Cwd(PathBuf::from("/Users/jonas/My Code")), Signal::PromptStart]);
    }

    #[test]
    fn ignores_other_sequences() {
        let mut tap = SignalTap::default();
        assert_eq!(tap.advance(b"\x1b]0;title\x07\x1b[31mred\x1b[0m"), []);
    }
}
