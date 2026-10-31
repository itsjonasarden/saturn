//! How much of the Codex plan's limits is used, read live the way Codex's own `/status` reads it.
//!
//! Codex's app server answers `account/rateLimits/read` from the plan's usage endpoint, so Akyba
//! asks a short-lived `codex app-server` rather than reading the login itself: Codex keeps its
//! sign-in and knows where to ask. It starts no session and spends nothing.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::usage::Limit;

const REQUEST_ID: i64 = 2;
/// Starting the app server and asking takes a second or two; past this something is wrong.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Asks Codex for the plan's limits. `None` when `codex` isn't there, isn't signed in to a plan
/// with limits, or doesn't answer.
pub fn read(path_env: &str) -> Option<Vec<Limit>> {
    let requests = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "clientInfo": { "name": "akyba", "version": env!("CARGO_PKG_VERSION") } } }),
        json!({ "jsonrpc": "2.0", "method": "initialized" }),
        // A background poll: the details of reset credits are a second lookup Akyba doesn't show.
        json!({ "jsonrpc": "2.0", "id": REQUEST_ID, "method": "account/rateLimits/read",
                "params": { "excludeResetCreditDetails": true } }),
    ];
    let mut child = Command::new("codex")
        .arg("app-server")
        .env("PATH", path_env)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    for request in requests {
        writeln!(stdin, "{request}").ok()?;
    }
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let answer = BufReader::new(stdout).lines().map_while(Result::ok).find_map(|line| answer(&line));
        let _ = sender.send(answer);
    });
    let limits = receiver.recv_timeout(TIMEOUT).ok().flatten();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    limits.flatten()
}

/// The limits in one line of the app server's output, if it's the answer to Akyba's request:
/// `Some(None)` when it failed or holds no windows.
pub fn answer(line: &str) -> Option<Option<Vec<Limit>>> {
    let message: Value = serde_json::from_str(line).ok()?;
    if message.get("id")?.as_i64()? != REQUEST_ID {
        return None;
    }
    let Some(result) = message.get("result") else { return Some(None) };
    // Every metered bucket, the plan's own (`codex`) first; older servers send only the one.
    let mut buckets: Vec<(&str, &Value)> = result
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .map(|by_id| by_id.iter().map(|(id, snapshot)| (id.as_str(), snapshot)).collect())
        .unwrap_or_default();
    if buckets.is_empty() {
        buckets.extend(result.get("rateLimits").map(|snapshot| ("codex", snapshot)));
    }
    buckets.sort_by_key(|(id, _)| (*id != "codex", *id));
    let limits: Vec<Limit> = buckets
        .into_iter()
        .flat_map(|(id, snapshot)| {
            // A model's own bucket is named after it, as Claude's per-model week is.
            let name =
                (id != "codex").then(|| snapshot.get("limitName").and_then(Value::as_str).unwrap_or(id).to_owned());
            ["primary", "secondary"].into_iter().filter_map(move |slot| {
                let window = snapshot.get(slot).filter(|window| window.is_object())?;
                let length = window_label(window.get("windowDurationMins")?.as_i64()?);
                Some(Limit {
                    label: name.as_ref().map_or_else(|| length.clone(), |name| format!("{name} · {length}")),
                    percent: window.get("usedPercent")?.as_f64()?,
                    resets_at: window.get("resetsAt").and_then(Value::as_i64),
                })
            })
        })
        .collect();
    Some((!limits.is_empty()).then_some(limits))
}

/// A window's name by its length, as Claude's are named.
fn window_label(minutes: i64) -> String {
    match minutes {
        300 => "Session".to_owned(),
        10_080 => "Week".to_owned(),
        minutes if minutes % 1_440 == 0 => format!("{} days", minutes / 1_440),
        minutes => format!("{} h", minutes / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_window_of_every_bucket_the_plans_first() {
        let line = r#"{"id":2,"result":{"rateLimits":{"limitId":"codex","primary":{"usedPercent":11,"windowDurationMins":10080,"resetsAt":1793092024},"secondary":null},"rateLimitsByLimitId":{"codex_mini":{"limitId":"codex_mini","limitName":"Mini","primary":{"usedPercent":3,"windowDurationMins":10080,"resetsAt":1793092024}},"codex":{"limitId":"codex","primary":{"usedPercent":40,"windowDurationMins":300,"resetsAt":1792500000},"secondary":{"usedPercent":11,"windowDurationMins":10080,"resetsAt":1793092024}}}}}"#;
        let limits = answer(line).expect("our answer").expect("limits");
        let labels: Vec<&str> = limits.iter().map(|limit| limit.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Week", "Mini · Week"]);
        assert_eq!((limits[1].percent, limits[1].resets_at), (11.0, Some(1_793_092_024)));
    }

    #[test]
    fn reads_the_single_bucket_and_knows_a_failure() {
        let line = r#"{"id":2,"result":{"rateLimits":{"limitId":"codex","primary":{"usedPercent":11,"windowDurationMins":10080,"resetsAt":1}}}}"#;
        assert_eq!(answer(line).expect("our answer").expect("limits")[0].label, "Week");
        assert_eq!(answer(r#"{"id":2,"error":{"code":-32600,"message":"not signed in"}}"#), Some(None));
        assert_eq!(answer(r#"{"id":1,"result":{}}"#), None);
        assert_eq!(answer(r#"{"method":"account/updated"}"#), None);
    }
}
