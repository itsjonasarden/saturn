//! What an agent is up to beyond its state, for its tile in the Agents tab: how much context its
//! session holds (from the transcript Claude Code writes) and how far through its task list it is
//! (from the `TodoWrite` calls the hooks report).

use serde_json::Value;

/// How far through its task list an agent is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steps {
    pub done: usize,
    pub total: usize,
    /// The step in progress, as Claude words it ("Running the tests").
    pub current: Option<String>,
}

/// The steps in a `TodoWrite` call's input, if it is one.
pub fn steps(tool_name: &str, tool_input: &Value) -> Option<Steps> {
    if tool_name != "TodoWrite" {
        return None;
    }
    let todos = tool_input.get("todos")?.as_array()?;
    let status = |todo: &Value| todo.get("status").and_then(Value::as_str).unwrap_or_default().to_owned();
    let done = todos.iter().filter(|todo| status(todo) == "completed").count();
    let current = todos.iter().find(|todo| status(todo) == "in_progress").and_then(|todo| {
        todo.get("activeForm").or_else(|| todo.get("content")).and_then(Value::as_str).map(str::to_owned)
    });
    Some(Steps { done, total: todos.len(), current })
}

/// The tokens in the session's context after its latest reply: the newest assistant message's
/// usage in the transcript (input, cache reads and writes, and its own output). `tail` is the end
/// of the transcript file; a partial first line is skipped.
pub fn context_tokens(tail: &str) -> Option<u64> {
    tail.lines().rev().find_map(|line| {
        let entry: Value = serde_json::from_str(line).ok()?;
        let usage = entry.get("message")?.get("usage")?;
        let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        let total = count("input_tokens")
            + count("cache_read_input_tokens")
            + count("cache_creation_input_tokens")
            + count("output_tokens");
        (total > 0).then_some(total)
    })
}

/// `412k`, `1.2M`: a token count as short as a tile wants it.
pub fn short_tokens(tokens: u64) -> String {
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..1_000_000 => format!("{}k", tokens / 1_000),
        _ => format!("{}.{}M", tokens / 1_000_000, tokens % 1_000_000 / 100_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_todo_steps_and_the_current_one() {
        let input = serde_json::json!({ "todos": [
            { "content": "Read the code", "status": "completed", "activeForm": "Reading the code" },
            { "content": "Write the fix", "status": "in_progress", "activeForm": "Writing the fix" },
            { "content": "Run the tests", "status": "pending", "activeForm": "Running the tests" },
        ]});
        assert_eq!(
            steps("TodoWrite", &input),
            Some(Steps { done: 1, total: 3, current: Some("Writing the fix".to_owned()) })
        );
        assert_eq!(steps("Bash", &input), None);
    }

    #[test]
    fn reads_the_newest_usage_and_skips_a_cut_line() {
        let tail = concat!(
            "ens\":5}}}\n",
            r#"{"type":"assistant","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":1000,"output_tokens":5}}}"#,
            "\n",
            r#"{"type":"user","message":{"content":"thanks"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"usage":{"input_tokens":3,"cache_read_input_tokens":41000,"cache_creation_input_tokens":200,"output_tokens":40}}}"#,
        );
        assert_eq!(context_tokens(tail), Some(41_243));
        assert_eq!(context_tokens("not json"), None);
    }

    #[test]
    fn shortens_token_counts() {
        assert_eq!(short_tokens(950), "950");
        assert_eq!(short_tokens(412_345), "412k");
        assert_eq!(short_tokens(1_234_567), "1.2M");
    }
}
