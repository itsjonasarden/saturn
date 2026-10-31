//! The Akyba plugin's hook script talking to a real bridge, as Claude Code would run it.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use akyba_bridge::Bridge;
use serde_json::json;

fn report_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugin/bin/report")
}

fn run_hook(socket: Option<&std::path::Path>, input: &str) -> String {
    let mut command = Command::new(report_script());
    command.env("AKYBA_PANE_ID", "7").stdin(Stdio::piped()).stdout(Stdio::piped());
    match socket {
        Some(path) => command.env("AKYBA_SOCKET", path),
        None => command.env_remove("AKYBA_SOCKET"),
    };
    let mut child = command.spawn().expect("the hook script runs");
    child.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("write input");
    let output = child.wait_with_output().expect("the hook script ends");
    assert!(output.status.success(), "the hook script always exits 0");
    String::from_utf8(output.stdout).expect("utf-8")
}

#[test]
fn delivers_the_event_and_prints_the_answer() {
    let socket = std::env::temp_dir().join(format!("akyba-test-{}.sock", std::process::id()));
    let (bridge, calls) = Bridge::start(&socket).expect("the bridge starts");
    let answering = std::thread::spawn(move || {
        let call = calls.recv_blocking().expect("a hook call arrives");
        assert_eq!(call.pane, "7");
        assert_eq!(call.event(), "UserPromptSubmit");
        assert_eq!(call.field("prompt"), Some("/start abc-224"));
        call.answer(&json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "sessionTitle": "ABC-224"}}));
    });
    let printed = run_hook(Some(bridge.path()), r#"{"hook_event_name":"UserPromptSubmit","prompt":"/start abc-224"}"#);
    answering.join().expect("the answer was given");
    assert_eq!(printed, r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","sessionTitle":"ABC-224"}}"#);
}

#[test]
fn does_nothing_outside_akyba() {
    assert_eq!(run_hook(None, r#"{"hook_event_name":"Stop"}"#), "");
}
