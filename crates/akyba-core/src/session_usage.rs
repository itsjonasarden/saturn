//! How much one Claude Code session has used, from its transcript: the tokens of its replies since
//! a moment (the start of the plan's 5-hour window). The plan's limits are the account's, not a
//! session's; comparing sessions says which agent is spending them.

use serde_json::Value;

/// Tokens a session used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    /// Written to the prompt cache: billed like input, a little more.
    pub cache_write: u64,
    /// Read back from the prompt cache: a tenth of the price, and the bulk of every long session.
    pub cache_read: u64,
}

impl Tokens {
    /// What counts against a limit, roughly: everything but cache reads, which cost little and
    /// would swamp the rest.
    pub fn counted(&self) -> u64 {
        self.input + self.output + self.cache_write
    }
}

/// Seconds since 1970 for an ISO 8601 time in UTC (`2026-10-20T10:00:00.123Z`), as transcripts
/// write them.
pub fn iso_to_unix(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    // Days from 1970-01-01 to the date, by Howard Hinnant's civil calendar formula.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_era_days = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_era_days - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// The tokens of the replies in a transcript since `since` (Unix seconds; `None` for all of them).
/// A reply is written once per part, each with the same usage, so each counts once.
pub fn read(lines: impl Iterator<Item = String>, since: Option<i64>) -> Tokens {
    let mut seen = std::collections::HashMap::<String, Tokens>::new();
    for line in lines {
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(since) = since {
            let at = entry.get("timestamp").and_then(Value::as_str).and_then(iso_to_unix);
            if at.is_none_or(|at| at < since) {
                continue;
            }
        }
        let Some(message) = entry.get("message") else { continue };
        let Some(usage) = message.get("usage") else { continue };
        let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        let tokens = Tokens {
            input: count("input_tokens"),
            output: count("output_tokens"),
            cache_write: count("cache_creation_input_tokens"),
            cache_read: count("cache_read_input_tokens"),
        };
        let id = message.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
        // The last part has the reply's final output count; keep the largest of each.
        let kept = seen.entry(id).or_default();
        kept.input = kept.input.max(tokens.input);
        kept.output = kept.output.max(tokens.output);
        kept.cache_write = kept.cache_write.max(tokens.cache_write);
        kept.cache_read = kept.cache_read.max(tokens.cache_read);
    }
    seen.values().fold(Tokens::default(), |sum, tokens| Tokens {
        input: sum.input + tokens.input,
        output: sum.output + tokens.output,
        cache_write: sum.cache_write + tokens.cache_write,
        cache_read: sum.cache_read + tokens.cache_read,
    })
}

/// A count of tokens in a few characters: `950`, `12k`, `1.2M`.
pub fn short(tokens: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "a count shown to one decimal")]
    let value = tokens as f64;
    if tokens >= 1_000_000 {
        format!("{:.1}M", value / 1_000_000.)
    } else if tokens >= 1_000 {
        format!("{:.0}k", value / 1_000.)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_iso_times() {
        assert_eq!(iso_to_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_to_unix("2026-10-20T10:00:00.123Z"), Some(1_792_490_400));
        assert_eq!(iso_to_unix("2024-02-29T12:30:15Z"), Some(1_709_209_815));
        assert_eq!(iso_to_unix("soon"), None);
    }

    #[test]
    fn counts_each_reply_once_since_a_moment() {
        let transcript = r#"{"type":"assistant","timestamp":"2026-10-20T08:00:00Z","message":{"id":"m0","usage":{"input_tokens":999,"output_tokens":999}}}
{"type":"user","timestamp":"2026-10-20T10:00:00Z","message":{"content":"go"}}
{"type":"assistant","timestamp":"2026-10-20T10:00:01Z","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":3,"cache_creation_input_tokens":100,"cache_read_input_tokens":5000}}}
{"type":"assistant","timestamp":"2026-10-20T10:00:02Z","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":40,"cache_creation_input_tokens":100,"cache_read_input_tokens":5000}}}
{"type":"assistant","timestamp":"2026-10-20T10:05:00Z","message":{"id":"m2","usage":{"input_tokens":5,"output_tokens":20,"cache_read_input_tokens":7000}}}"#;
        let since = iso_to_unix("2026-10-20T09:00:00Z");
        let tokens = read(transcript.lines().map(str::to_owned), since);
        assert_eq!(tokens, Tokens { input: 15, output: 60, cache_write: 100, cache_read: 12_000 });
        assert_eq!(tokens.counted(), 175);
        assert_eq!(read(transcript.lines().map(str::to_owned), None).input, 1014);
    }

    #[test]
    fn writes_counts_short() {
        assert_eq!(short(950), "950");
        assert_eq!(short(12_400), "12k");
        assert_eq!(short(1_240_000), "1.2M");
    }
}
