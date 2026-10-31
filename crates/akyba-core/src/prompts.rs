//! What was asked that changed a file: the prompts you gave Claude Code whose turn edited it, read
//! from its session transcripts, so a review shows the intent beside the diff. Edits made through
//! the shell can't be told apart, so only Claude's own file tools count. The transcripts' format is
//! Claude Code's own and may change; anything not understood is passed over.

use std::collections::HashMap;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// A prompt, and when it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub prompt: String,
    /// As the transcript has it (ISO 8601), for ordering and display.
    pub at: String,
}

/// The tools that write files, and so make changes a review shows.
const WRITERS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Where Claude Code keeps the transcripts of sessions started in `checkout`:
/// `~/.claude/projects/<the path with every other character a dash>`.
pub fn transcripts_folder(claude_home: &Path, checkout: &Path) -> PathBuf {
    let name: String =
        checkout.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    claude_home.join("projects").join(name)
}

/// Claude Code's own folder: `CLAUDE_CONFIG_DIR`, else `~/.claude`.
pub fn claude_home() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
}

/// The text of a prompt you typed, or `None` for anything else in a user entry: tool results,
/// commands and notes Claude Code adds itself (`<command-name>…`, `<system-reminder>…`), a subagent's
/// instructions.
pub(crate) fn typed_prompt(entry: &Value) -> Option<String> {
    if entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || entry.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let content = entry.get("message")?.get("content")?;
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            if parts.iter().any(|part| part.get("type").and_then(Value::as_str) == Some("tool_result")) {
                return None;
            }
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_owned())
}

/// Every prompt you typed in a transcript, in order.
pub fn typed_prompts(lines: impl Iterator<Item = String>) -> Vec<Asked> {
    lines
        .filter(|line| line.contains("\"user\""))
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("user"))
        .filter_map(|entry| {
            let prompt = typed_prompt(&entry)?;
            let at = entry.get("timestamp").and_then(Value::as_str).unwrap_or_default().to_owned();
            Some(Asked { prompt, at })
        })
        .collect()
}

/// Whether a prompt corrects the agent: it says no, never, always, instead, stop… These are the
/// lessons worth keeping for the next session.
pub fn is_correction(prompt: &str) -> bool {
    let text = format!(" {} ", prompt.to_lowercase().replace(['.', ',', '!', '?', ':', ';'], " "));
    let starts = ["no ", "nope ", "stop ", "wait ", "wrong "];
    let says = [
        " don't ",
        " dont ",
        " do not ",
        " never ",
        " always ",
        " instead ",
        " not like ",
        " shouldn't ",
        " should not ",
        " why did you ",
        " wrong ",
        " stop ",
        " please don't ",
        " i said ",
        " i told you ",
    ];
    starts.iter().any(|start| text.trim_start().starts_with(start)) || says.iter().any(|phrase| text.contains(phrase))
}

/// The files an assistant entry writes with Claude's file tools.
fn written_files(entry: &Value) -> Vec<String> {
    let Some(Value::Array(parts)) = entry.get("message").and_then(|message| message.get("content")) else {
        return Vec::new();
    };
    parts
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter(|part| part.get("name").and_then(Value::as_str).is_some_and(|name| WRITERS.contains(&name)))
        .filter_map(|part| {
            let input = part.get("input")?;
            input.get("file_path").or_else(|| input.get("notebook_path")).and_then(Value::as_str).map(str::to_owned)
        })
        .collect()
}

/// Reads one transcript into `found`: for each file under `checkout` that a turn wrote, the prompt
/// that started the turn, relative to `checkout`.
pub fn read_transcript<S: std::hash::BuildHasher>(
    lines: impl Iterator<Item = String>,
    checkout: &Path,
    found: &mut HashMap<String, Vec<Asked>, S>,
) {
    let mut asked: Option<Asked> = None;
    for line in lines {
        // Most lines are neither; skip them without parsing.
        if !line.contains("\"user\"") && !line.contains("tool_use") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue };
        match entry.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(prompt) = typed_prompt(&entry) {
                    let at = entry.get("timestamp").and_then(Value::as_str).unwrap_or_default().to_owned();
                    asked = Some(Asked { prompt, at });
                }
            }
            Some("assistant") => {
                let Some(asked) = &asked else { continue };
                for file in written_files(&entry) {
                    let Ok(relative) = Path::new(&file).strip_prefix(checkout) else { continue };
                    let prompts = found.entry(relative.to_string_lossy().into_owned()).or_default();
                    if !prompts.contains(asked) {
                        prompts.push(asked.clone());
                    }
                }
            }
            _ => {}
        }
    }
}

/// The prompts behind each changed file of `checkout`, oldest first, from the transcripts of the last
/// fortnight. Reads files from disk: off the main thread.
pub fn for_checkout(checkout: &Path) -> HashMap<String, Vec<Asked>> {
    let mut found = HashMap::new();
    let Some(home) = claude_home() else { return found };
    let folder = transcripts_folder(&home, checkout);
    let Ok(entries) = std::fs::read_dir(&folder) else { return found };
    let fortnight = std::time::Duration::from_hours(14 * 24);
    let mut transcripts: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "jsonl"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .filter(|(modified, _)| modified.elapsed().is_ok_and(|age| age < fortnight))
        .collect();
    transcripts.sort();
    for (_, path) in transcripts.iter().rev().take(30).rev() {
        let Ok(file) = std::fs::File::open(path) else { continue };
        read_transcript(std::io::BufReader::new(file).lines().map_while(Result::ok), checkout, &mut found);
    }
    for prompts in found.values_mut() {
        prompts.sort_by(|a, b| a.at.cmp(&b.at));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> impl Iterator<Item = String> + '_ {
        text.lines().map(str::to_owned)
    }

    #[test]
    fn ties_each_written_file_to_the_prompt_of_its_turn() {
        let transcript = r#"{"type":"user","timestamp":"2026-10-20T10:00:00Z","message":{"role":"user","content":"Round prices to cents"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/code/acme-web/src/cart.ts"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/code/acme-web/src/cart.ts"}}]}}
{"type":"user","isMeta":true,"message":{"content":"<command-name>/clear</command-name>"}}
{"type":"user","timestamp":"2026-10-20T11:00:00Z","message":{"content":[{"type":"text","text":"Add a test for it"}]}}
{"type":"user","isSidechain":true,"message":{"content":"You are a subagent"}}
{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/code/acme-web/src/cart.test.ts"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/code/acme-web/src/cart.ts"}},{"type":"tool_use","name":"Write","input":{"file_path":"/elsewhere/notes.md"}}]}}
not json at all"#;
        let mut found = HashMap::new();
        read_transcript(lines(transcript), Path::new("/code/acme-web"), &mut found);
        let prompts = |file: &str| found[file].iter().map(|asked| asked.prompt.as_str()).collect::<Vec<_>>();
        assert_eq!(prompts("src/cart.ts"), ["Round prices to cents", "Add a test for it"]);
        assert_eq!(
            prompts("src/cart.test.ts"),
            ["Add a test for it"],
            "a subagent's edit counts for the prompt it served"
        );
        assert_eq!(found.len(), 2, "files outside the checkout and only read ones don't count");
    }

    #[test]
    fn lists_typed_prompts_and_spots_corrections() {
        let transcript = r#"{"type":"user","timestamp":"t1","message":{"content":"Add a cart page"}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}
{"type":"user","timestamp":"t2","message":{"content":"No, don't use reduce here, keep the loop."}}"#;
        let prompts: Vec<String> = typed_prompts(lines(transcript)).into_iter().map(|asked| asked.prompt).collect();
        assert_eq!(prompts, ["Add a cart page", "No, don't use reduce here, keep the loop."]);
        assert!(is_correction(&prompts[1]));
        assert!(is_correction("Always run the tests before you push"));
        assert!(is_correction("use the tokens instead of hex"));
        assert!(!is_correction(&prompts[0]));
        assert!(!is_correction("Nothing else for now, thanks"));
    }

    #[test]
    fn finds_the_transcripts_of_a_folder() {
        assert_eq!(
            transcripts_folder(Path::new("/home/me/.claude"), Path::new("/code/acme-web.v2")),
            PathBuf::from("/home/me/.claude/projects/-code-acme-web-v2")
        );
    }
}
