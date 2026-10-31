//! Where the agents' hooks (Claude Code's, Codex's) reach Akyba. The Akyba plugin's hooks `curl` each event's JSON to a
//! Unix socket that only this Akyba instance listens on (`AKYBA_SOCKET`, set in every pane), with
//! the pane they run in (`AKYBA_PANE_ID`). Akyba may answer, e.g. with a `sessionTitle` for
//! `UserPromptSubmit`; hooks that get no answer in time carry on without one.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

/// How long a hook waits for Akyba's answer before it carries on without one.
const ANSWER_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_BODY: usize = 4 * 1024 * 1024;

/// One hook event from an agent's session in a Akyba pane.
pub struct HookCall {
    /// `AKYBA_PANE_ID` of the pane the session runs in.
    pub pane: String,
    /// The agent whose hook it is (`claude`, `codex`), as its hook script says; Claude Code's when
    /// it doesn't.
    pub agent: String,
    /// The hook's input, as Claude Code sends it.
    pub input: Value,
    answer: mpsc::Sender<String>,
}

impl HookCall {
    /// The hook event's name, e.g. `UserPromptSubmit`.
    pub fn event(&self) -> &str {
        self.input.get("hook_event_name").and_then(Value::as_str).unwrap_or_default()
    }

    /// A string field of the hook input.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.input.get(name).and_then(Value::as_str)
    }

    /// Answers the hook with JSON output for Claude Code (see the hooks reference).
    pub fn answer(self, output: &Value) {
        let _ = self.answer.send(output.to_string());
    }

    /// Lets the hook carry on with no output.
    pub fn done(self) {
        let _ = self.answer.send(String::new());
    }
}

/// The running bridge; the socket goes away with it.
pub struct Bridge {
    path: PathBuf,
}

impl Bridge {
    /// Listens on `path`. The returned receiver delivers every hook call.
    pub fn start(path: &Path) -> std::io::Result<(Self, async_channel::Receiver<HookCall>)> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        let (calls, receiver) = async_channel::unbounded();
        thread::Builder::new().name("akyba-bridge".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let calls = calls.clone();
                let _ = thread::Builder::new().name("akyba-hook".into()).spawn(move || serve(stream, &calls));
            }
        })?;
        Ok((Self { path: path.to_owned() }, receiver))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// One `name=value` of a request target's query.
fn query_value(target: &str, name: &str) -> Option<String> {
    let (_, query) = target.split_once('?')?;
    query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('=')).map(str::to_owned)
}

/// Reads one `POST /hook?pane=<id>&agent=<name>` request and answers it.
fn serve(stream: UnixStream, calls: &async_channel::Sender<HookCall>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let target = request_line.split_whitespace().nth(1).unwrap_or_default();
    let pane = query_value(target, "pane").unwrap_or_default();
    let agent = query_value(target, "agent").unwrap_or_else(|| "claude".to_owned());

    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length.min(MAX_BODY)];
    if reader.read_exact(&mut body).is_err() {
        return;
    }

    let answer = serde_json::from_slice(&body).ok().map_or_else(String::new, |input| {
        let (answer, answered) = mpsc::channel();
        if calls.send_blocking(HookCall { pane, agent, input, answer }).is_err() {
            return String::new();
        }
        answered.recv_timeout(ANSWER_TIMEOUT).unwrap_or_default()
    });
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
        answer.len()
    );
    let mut stream = stream;
    let _ = stream.write_all(response.as_bytes());
}
