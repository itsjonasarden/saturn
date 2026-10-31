//! How much of the Claude plan's limits is used, as Claude Code's `/usage` shows it: the current
//! session, the week across all models, and the week for a model with its own limit.
//!
//! Claude Code answers a `get_usage` request on its stream-json control channel, so Akyba asks a
//! short-lived `claude -p` rather than reading the login token itself: Claude Code keeps its
//! sign-in, refreshes it, and knows which endpoint to ask.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

const REQUEST_ID: &str = "akyba-usage";
/// Starting Claude Code and asking takes two or three seconds; past this something is wrong.
const TIMEOUT: Duration = Duration::from_secs(20);

/// One limit's window.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    /// "Session", "Week", or the model with its own weekly limit ("Fable").
    pub label: String,
    /// 0–100.
    pub percent: f64,
    /// When the window starts again, in Unix seconds.
    pub resets_at: Option<i64>,
}

/// Asks Claude Code for the plan's usage. `None` when `claude` isn't there, isn't signed in to a
/// plan with limits (an API key has none), or doesn't answer.
pub fn read(path_env: &str) -> Option<Vec<Limit>> {
    let request = serde_json::json!({
        "type": "control_request",
        "request_id": REQUEST_ID,
        "request": { "subtype": "get_usage" },
    });
    // No settings, MCP servers or saved session: this asks one question, and the user's hooks
    // shouldn't fire every few minutes for it.
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--setting-sources",
            "",
        ])
        .env("PATH", path_env)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    writeln!(stdin, "{request}").ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let answer = BufReader::new(stdout).lines().map_while(Result::ok).find_map(|line| answer(&line));
        let _ = sender.send(answer);
    });
    let limits = receiver.recv_timeout(TIMEOUT).ok().flatten();
    // Closing stdin ends the session; kill it in case it lingers.
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    limits.flatten()
}

/// The limits in one line of Claude Code's output, if it's the answer to Akyba's request:
/// `Some(None)` when the answer says there are no plan limits.
pub fn answer(line: &str) -> Option<Option<Vec<Limit>>> {
    let message: Value = serde_json::from_str(line).ok()?;
    let response = message.get("response")?;
    if message.get("type")?.as_str()? != "control_response" || response.get("request_id")?.as_str()? != REQUEST_ID {
        return None;
    }
    let usage = response.get("response")?;
    if usage.get("rate_limits_available").and_then(Value::as_bool) == Some(false) {
        return Some(None);
    }
    Some(usage.get("rate_limits").map(limits).filter(|limits| !limits.is_empty()))
}

/// The rows of `/usage`, from the server's `limits` list when it has one (it names the model
/// with its own limit), else from the fixed session and week windows.
fn limits(rate_limits: &Value) -> Vec<Limit> {
    let listed: Vec<Limit> = rate_limits
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let model = row.pointer("/scope/model/display_name").and_then(Value::as_str);
            let label = match (row.get("kind")?.as_str()?, model) {
                (_, Some(model)) => model.to_owned(),
                ("session", None) => "Session".to_owned(),
                ("weekly_all", None) => "Week".to_owned(),
                _ => return None,
            };
            Some(Limit {
                label,
                percent: row.get("percent")?.as_f64()?,
                resets_at: row.get("resets_at").and_then(Value::as_str).and_then(unix_seconds),
            })
        })
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    [("five_hour", "Session"), ("seven_day", "Week")]
        .into_iter()
        .filter_map(|(key, label)| {
            let window = rate_limits.get(key)?;
            Some(Limit {
                label: label.to_owned(),
                percent: window.get("utilization")?.as_f64()?,
                resets_at: window.get("resets_at").and_then(Value::as_str).and_then(unix_seconds),
            })
        })
        .collect()
}

/// An ISO 8601 time with an offset (`2026-10-19T11:40:00.49+00:00`) as Unix seconds.
fn unix_seconds(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    // The offset follows the seconds and their fraction: `Z`, `+hh:mm` or `-hh:mm`.
    let rest = text.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest.chars().next() {
        Some('Z') | None => 0,
        Some(sign) => {
            let hours = rest.get(1..3)?.parse::<i64>().ok()?;
            let minutes = rest.get(4..6).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
            let seconds = (hours * 60 + minutes) * 60;
            if sign == '-' { -seconds } else { seconds }
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 for a date in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// When a window starts again, in words: "in 40 min", "in 2 h 10 min", "in 6 days".
pub fn resets_in(resets_at: i64, now: i64) -> String {
    let minutes = u64::try_from(resets_at - now).unwrap_or(0).div_ceil(60);
    match minutes {
        0..=59 => format!("in {minutes} min"),
        60..=1439 if minutes % 60 == 0 => format!("in {} h", minutes / 60),
        60..=1439 => format!("in {} h {} min", minutes / 60, minutes % 60),
        _ => match minutes.div_ceil(1440) {
            1 => "in 1 day".to_owned(),
            days => format!("in {days} days"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"akyba-usage","response":{"rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":8,"resets_at":"2026-10-19T11:40:00.492029+00:00"},"seven_day":{"utilization":11,"resets_at":"2026-10-25T13:00:00+00:00"},"limits":[{"kind":"session","percent":8,"resets_at":"2026-10-19T11:40:00.492029+00:00","scope":null},{"kind":"weekly_all","percent":11,"resets_at":"2026-10-25T13:00:00.492055+00:00","scope":null},{"kind":"weekly_scoped","percent":0,"resets_at":"2026-10-25T13:00:00+00:00","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null}}]}}}}"#;

    #[test]
    fn reads_the_session_the_week_and_a_models_week() {
        let limits = answer(ANSWER).expect("our answer").expect("plan limits");
        let labels: Vec<&str> = limits.iter().map(|limit| limit.label.as_str()).collect();
        assert_eq!(labels, ["Session", "Week", "Fable"]);
        assert!((limits[1].percent - 11.).abs() < f64::EPSILON);
        assert_eq!(limits[0].resets_at, Some(1_792_410_000));
    }

    #[test]
    fn falls_back_to_the_fixed_windows_and_knows_when_there_are_none() {
        let fixed = ANSWER.replace(r#""limits":["#, r#""ignored":["#);
        let limits = answer(&fixed).expect("our answer").expect("plan limits");
        assert_eq!(limits.iter().map(|limit| limit.label.as_str()).collect::<Vec<_>>(), ["Session", "Week"]);
        let api_key = r#"{"type":"control_response","response":{"request_id":"akyba-usage","response":{"rate_limits_available":false,"rate_limits":null}}}"#;
        assert_eq!(answer(api_key), Some(None));
        assert_eq!(answer(r#"{"type":"system","subtype":"init"}"#), None);
    }

    #[test]
    fn reads_iso_times_with_any_offset() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_seconds("2026-10-19T13:40:00+02:00"), unix_seconds("2026-10-19T11:40:00.5+00:00"));
        assert_eq!(unix_seconds("2000-03-01T00:00:00-01:30"), Some(951_868_800 + 5400));
    }

    #[test]
    fn says_when_a_window_resets() {
        assert_eq!(resets_in(2400, 0), "in 40 min");
        assert_eq!(resets_in(3 * 3600, 0), "in 3 h");
        assert_eq!(resets_in(2 * 3600 + 600, 0), "in 2 h 10 min");
        assert_eq!(resets_in(6 * 86_400 - 100, 0), "in 6 days");
        assert_eq!(resets_in(0, 50), "in 0 min");
    }
}
