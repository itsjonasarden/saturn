//! What an agent in a folder is told, found the way it finds it. Claude Code: instruction files from
//! the folder up to the repository (`CLAUDE.md`, `CLAUDE.local.md`, and `AGENTS.md` where there's no
//! `CLAUDE.md` at all) and the personal one, rules (`.claude/rules/`, some only for certain files),
//! skills, commands, subagents, hooks and MCP servers. Codex: `AGENTS.override.md` or `AGENTS.md` in
//! each folder from the repository down, the personal one, and skills in `.agents/skills`. And what's said twice: the same instruction in several files drifts apart the first time
//! one is edited, and a model reading both can't tell which one wins.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Instructions,
    Rule,
    Skill,
    Command,
    Subagent,
    Hook,
    McpServer,
}

impl Kind {
    pub fn heading(self) -> &'static str {
        match self {
            Self::Instructions => "Instructions",
            Self::Rule => "Rules",
            Self::Skill => "Skills",
            Self::Command => "Commands",
            Self::Subagent => "Subagents",
            Self::Hook => "Hooks",
            Self::McpServer => "MCP servers",
        }
    }
}

/// One thing the agent is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub kind: Kind,
    /// What it's called: the file's place, a skill's name, a hook's event.
    pub name: String,
    /// What it says it's for, or which files it's for.
    pub detail: Option<String>,
    /// Where it is, to open.
    pub path: PathBuf,
    /// Its length in lines, for files.
    pub lines: usize,
}

/// Instruction files longer than this get a word: models follow short ones better (Anthropic's
/// advice: "ruthlessly prune").
pub const LONG: usize = 200;

/// `name:`, `description:` and `paths:` from a Markdown file's front matter (between `---` lines).
fn front_matter(text: &str) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return found;
    }
    let mut key: Option<String> = None;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(item) = line.trim().strip_prefix("- ").filter(|_| line.starts_with(' ') || line.starts_with('-')) {
            if let Some(key) = &key {
                let entry: &mut String = found.entry(key.clone()).or_default();
                if !entry.is_empty() {
                    entry.push_str(", ");
                }
                entry.push_str(item.trim().trim_matches(['"', '\'']));
            }
        } else if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_owned();
            let value = value.trim().trim_matches(['"', '\'']).to_owned();
            key = Some(name.clone());
            if !value.is_empty() {
                found.insert(name, value);
            }
        }
    }
    found
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Markdown files in a folder, sorted, and those one level down named `file` (skills' `SKILL.md`).
fn markdown_in(folder: &Path, nested: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| match nested {
            Some(file) => Some(path.join(file)).filter(|inner| inner.is_file()),
            None => (path.extension().is_some_and(|extension| extension == "md")).then_some(path),
        })
        .collect();
    found.sort();
    found
}

/// Everything an agent working in `folder` (inside `checkout`) is given; `home` is Claude Code's own
/// folder (`~/.claude`), for the personal instructions, skills and commands.
pub fn gather(checkout: &Path, folder: &Path, home: Option<&Path>) -> Vec<Source> {
    let mut found = Vec::new();
    let place = |path: &Path| {
        path.strip_prefix(checkout)
            .map_or_else(|_| path.display().to_string(), |relative| relative.display().to_string())
    };
    let mut add_file = |kind: Kind, path: PathBuf, name: String, detail: Option<String>| {
        if let Some(text) = read(&path) {
            found.push(Source { kind, name, detail, path, lines: text.lines().count() });
        }
    };
    // Instruction files: the personal one, then from the repository's top down to the folder.
    if let Some(home) = home {
        add_file(
            Kind::Instructions,
            home.join("CLAUDE.md"),
            "Your own CLAUDE.md".to_owned(),
            Some("for every project".to_owned()),
        );
    }
    let mut folders: Vec<PathBuf> =
        folder.ancestors().take_while(|path| path.starts_with(checkout)).map(Path::to_path_buf).collect();
    folders.reverse();
    // Claude Code reads AGENTS.md only where no CLAUDE.md (or CLAUDE.local.md) is in the folder or
    // above it.
    let claude_files = ["CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"];
    let has_claude_md = folder.ancestors().any(|dir| claude_files.iter().any(|file| dir.join(file).is_file()));
    let files: &[&str] = if has_claude_md { &claude_files } else { &["AGENTS.md", "CLAUDE.local.md"] };
    for dir in &folders {
        for file in files {
            let path = dir.join(file);
            let name = place(&path);
            add_file(Kind::Instructions, path, name, None);
        }
    }
    let claude = checkout.join(".claude");
    for path in markdown_in(&claude.join("rules"), None) {
        let text = read(&path).unwrap_or_default();
        let paths = front_matter(&text).remove("paths").map(|paths| format!("for {paths}"));
        add_file(Kind::Rule, path.clone(), place(&path), paths.or_else(|| Some("for every file".to_owned())));
    }
    let mut named = |kind: Kind, folder: &Path, nested: Option<&str>, own: bool| {
        for path in markdown_in(folder, nested) {
            let text = read(&path).unwrap_or_default();
            let mut matter = front_matter(&text);
            let fallback = if nested.is_some() { path.parent() } else { Some(path.as_path()) }
                .and_then(Path::file_stem)
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut name = matter.remove("name").unwrap_or(fallback);
            if kind == Kind::Command {
                name = format!("/{name}");
            }
            if own {
                name.push_str(" (yours)");
            }
            add_file(kind, path, name, matter.remove("description"));
        }
    };
    named(Kind::Skill, &claude.join("skills"), Some("SKILL.md"), false);
    named(Kind::Command, &claude.join("commands"), None, false);
    named(Kind::Subagent, &claude.join("agents"), None, false);
    if let Some(home) = home {
        named(Kind::Skill, &home.join("skills"), Some("SKILL.md"), true);
        named(Kind::Command, &home.join("commands"), None, true);
        named(Kind::Subagent, &home.join("agents"), None, true);
    }
    found.extend(hooks(&claude.join("settings.json")));
    found.extend(hooks(&claude.join("settings.local.json")));
    found.extend(mcp_servers(&checkout.join(".mcp.json")));
    found
}

/// Everything Codex working in `folder` (inside `checkout`) is given; `home` is Codex's own folder
/// (`~/.codex`) and `user_home` the person's, for their own instructions and skills.
pub fn gather_codex(checkout: &Path, folder: &Path, home: Option<&Path>, user_home: Option<&Path>) -> Vec<Source> {
    let mut found = Vec::new();
    let place = |path: &Path| {
        path.strip_prefix(checkout)
            .map_or_else(|_| path.display().to_string(), |relative| relative.display().to_string())
    };
    // In each folder the first of these that says something.
    let first_of = |dir: &Path| {
        ["AGENTS.override.md", "AGENTS.md"]
            .iter()
            .map(|file| dir.join(file))
            .find(|path| read(path).is_some_and(|text| !text.trim().is_empty()))
    };
    let mut add = |kind: Kind, path: PathBuf, name: String, detail: Option<String>| {
        if let Some(text) = read(&path) {
            found.push(Source { kind, name, detail, path, lines: text.lines().count() });
        }
    };
    if let Some(path) = home.and_then(first_of) {
        let name = format!("Your own {}", path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default());
        add(Kind::Instructions, path, name, Some("for every project".to_owned()));
    }
    let mut folders: Vec<PathBuf> =
        folder.ancestors().take_while(|path| path.starts_with(checkout)).map(Path::to_path_buf).collect();
    folders.reverse();
    for dir in &folders {
        if let Some(path) = first_of(dir) {
            let name = place(&path);
            add(Kind::Instructions, path, name, None);
        }
    }
    let mut skills = |folder: &Path, own: bool| {
        for path in markdown_in(folder, Some("SKILL.md")) {
            let text = read(&path).unwrap_or_default();
            let mut matter = front_matter(&text);
            let fallback = path.parent().and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned());
            let mut name = matter.remove("name").or(fallback).unwrap_or_default();
            if own {
                name.push_str(" (yours)");
            }
            add(Kind::Skill, path, name, matter.remove("description"));
        }
    };
    for dir in folders.iter().rev() {
        skills(&dir.join(".agents/skills"), false);
    }
    if let Some(user_home) = user_home {
        skills(&user_home.join(".agents/skills"), true);
    }
    found
}

/// The hooks a settings file sets: each event, and what runs.
fn hooks(settings: &Path) -> Vec<Source> {
    let Some(json) = read(settings).and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()) else {
        return Vec::new();
    };
    let Some(events) = json.get("hooks").and_then(serde_json::Value::as_object) else { return Vec::new() };
    events
        .iter()
        .map(|(event, groups)| {
            let commands: Vec<String> = groups
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|group| group.get("hooks").and_then(serde_json::Value::as_array).cloned().unwrap_or_default())
                .filter_map(|hook| hook.get("command").and_then(serde_json::Value::as_str).map(str::to_owned))
                .collect();
            Source {
                kind: Kind::Hook,
                name: event.clone(),
                detail: Some(commands.join(" · ")),
                path: settings.to_owned(),
                lines: 0,
            }
        })
        .collect()
}

/// The MCP servers a project's `.mcp.json` names.
fn mcp_servers(file: &Path) -> Vec<Source> {
    let Some(json) = read(file).and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()) else {
        return Vec::new();
    };
    let Some(servers) = json.get("mcpServers").and_then(serde_json::Value::as_object) else { return Vec::new() };
    servers
        .iter()
        .map(|(name, server)| {
            let what = server
                .get("command")
                .or_else(|| server.get("url"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            Source { kind: Kind::McpServer, name: name.clone(), detail: what, path: file.to_owned(), lines: 0 }
        })
        .collect()
}

/// The heading lessons go under in an instruction file.
pub const LESSONS: &str = "## Lessons";

/// Adds a lesson as a line under the file's `## Lessons` heading (made at the end if it isn't
/// there), making the file if it doesn't exist. Returns the line it's on.
pub fn add_lesson(file: &Path, lesson: &str) -> Result<usize, String> {
    let lesson = lesson.trim().trim_start_matches(['-', '*', ' ']);
    if lesson.is_empty() {
        return Err("The lesson is empty.".to_owned());
    }
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let at = if let Some(heading) = lines.iter().position(|line| line.trim() == LESSONS) {
        // After the section's last line that says something, before the next heading.
        let end = lines[heading + 1..]
            .iter()
            .position(|line| line.starts_with('#'))
            .map_or(lines.len(), |next| heading + 1 + next);
        let last = (heading + 1..end).rev().find(|index| !lines[*index].trim().is_empty()).unwrap_or(heading);
        last + 1
    } else {
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(LESSONS.to_owned());
        lines.len()
    };
    lines.insert(at, format!("- {lesson}"));
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(file, out).map_err(|error| error.to_string())?;
    Ok(at + 1)
}

/// An instruction said in more than one place: the words, and where (file, line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    pub text: String,
    pub places: Vec<(PathBuf, usize)>,
}

/// A line as it's compared: words only, lower-case, so `- **Never** push to main.` and
/// `Never push to main` are the same.
fn words_of(line: &str) -> String {
    line.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Instructions said in more than one of `sources`' files (instructions, rules and skills): lines of
/// six words or more that read the same once punctuation and case are set aside. A whole file that
/// copies another shows as one entry, line 1 in each.
pub fn repeats(sources: &[Source]) -> Vec<Repeat> {
    let files: Vec<&Source> =
        sources.iter().filter(|source| matches!(source.kind, Kind::Instructions | Kind::Rule | Kind::Skill)).collect();
    let texts: Vec<(PathBuf, String)> =
        files.iter().filter_map(|source| Some((source.path.clone(), read(&source.path)?))).collect();
    let mut found: Vec<Repeat> = Vec::new();
    // Whole copies first, so their lines aren't listed one by one.
    let mut copied: Vec<PathBuf> = Vec::new();
    for (index, (path, text)) in texts.iter().enumerate() {
        for (other, other_text) in &texts[index + 1..] {
            if !text.trim().is_empty() && words_of(text) == words_of(other_text) {
                found.push(Repeat {
                    text: "The whole file".to_owned(),
                    places: vec![(path.clone(), 1), (other.clone(), 1)],
                });
                copied.push(other.clone());
            }
        }
    }
    let mut lines: BTreeMap<String, (String, Vec<(PathBuf, usize)>)> = BTreeMap::new();
    for (path, text) in texts.iter().filter(|(path, _)| !copied.contains(path)) {
        let mut in_code = false;
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
                continue;
            }
            let key = words_of(line);
            if in_code || key.split(' ').count() < 6 || line.trim_start().starts_with('#') {
                continue;
            }
            let entry = lines
                .entry(key)
                .or_insert_with(|| (line.trim().trim_start_matches(['-', '*', ' ']).to_owned(), Vec::new()));
            if !entry.1.iter().any(|(seen, _)| seen == path) {
                entry.1.push((path.clone(), number + 1));
            }
        }
    }
    found.extend(
        lines.into_values().filter(|(_, places)| places.len() > 1).map(|(text, places)| Repeat { text, places }),
    );
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("a folder")).expect("make folders");
        fs::write(path, text).expect("write");
    }

    #[test]
    fn gathers_what_an_agent_is_told() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (repo, home) = (dir.path().join("acme-web"), dir.path().join("home"));
        write(&repo, "CLAUDE.md", "# Acme\nRun the tests.\n");
        write(&repo, "apps/web/AGENTS.md", "Use the design tokens.\n");
        write(&repo, ".claude/rules/api.md", "---\npaths:\n  - \"apps/api/**\"\n---\nValidate input.\n");
        write(
            &repo,
            ".claude/skills/release/SKILL.md",
            "---\nname: release\ndescription: Ship a version\n---\nSteps.\n",
        );
        write(&repo, ".claude/commands/start.md", "Start a ticket.\n");
        write(
            &repo,
            ".claude/settings.json",
            r#"{"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"lint.sh"}]}]}}"#,
        );
        write(&repo, ".mcp.json", r#"{"mcpServers":{"db":{"command":"db-mcp"}}}"#);
        write(&home, "CLAUDE.md", "Be brief.\n");
        let found = gather(&repo, &repo.join("apps/web"), Some(&home));
        let named = |kind: Kind| {
            found.iter().filter(|source| source.kind == kind).map(|source| source.name.as_str()).collect::<Vec<_>>()
        };
        // With a CLAUDE.md, Claude Code leaves AGENTS.md alone.
        assert_eq!(named(Kind::Instructions), ["Your own CLAUDE.md", "CLAUDE.md"]);
        assert_eq!(
            found.iter().find(|source| source.kind == Kind::Rule).and_then(|rule| rule.detail.clone()).as_deref(),
            Some("for apps/api/**")
        );
        assert_eq!(named(Kind::Skill), ["release"]);
        assert_eq!(named(Kind::Command), ["/start"]);
        assert_eq!(
            found.iter().find(|source| source.kind == Kind::Hook).and_then(|hook| hook.detail.clone()).as_deref(),
            Some("lint.sh")
        );
        assert_eq!(named(Kind::McpServer), ["db"]);
    }

    #[test]
    fn claude_reads_agents_md_where_there_is_no_claude_md() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let repo = dir.path().join("acme-web");
        write(&repo, "AGENTS.md", "Use pnpm.\n");
        write(&repo, "apps/web/AGENTS.md", "Use the design tokens.\n");
        let found = gather(&repo, &repo.join("apps/web"), None);
        let names: Vec<&str> = found.iter().map(|source| source.name.as_str()).collect();
        assert_eq!(names, ["AGENTS.md", "apps/web/AGENTS.md"]);
    }

    #[test]
    fn gathers_what_codex_is_told() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (repo, codex, user) = (dir.path().join("acme-web"), dir.path().join("codex"), dir.path().join("user"));
        write(&repo, "AGENTS.md", "Use pnpm.\n");
        write(&repo, "apps/web/AGENTS.md", "Use the design tokens.\n");
        write(&repo, "apps/web/AGENTS.override.md", "Not today: use plain CSS.\n");
        write(&repo, "CLAUDE.md", "Only for Claude.\n");
        write(&repo, ".agents/skills/release/SKILL.md", "---\nname: release\ndescription: Ship a version\n---\n");
        write(&codex, "AGENTS.md", "Be brief.\n");
        write(&user, ".agents/skills/notes/SKILL.md", "---\nname: notes\n---\n");
        let found = gather_codex(&repo, &repo.join("apps/web"), Some(&codex), Some(&user));
        let named = |kind: Kind| {
            found.iter().filter(|source| source.kind == kind).map(|source| source.name.as_str()).collect::<Vec<_>>()
        };
        assert_eq!(named(Kind::Instructions), ["Your own AGENTS.md", "AGENTS.md", "apps/web/AGENTS.override.md"]);
        assert_eq!(named(Kind::Skill), ["release", "notes (yours)"]);
    }

    #[test]
    fn adds_lessons_under_their_heading() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let file = dir.path().join("AGENTS.md");
        fs::write(&file, "# Acme\nRun the tests.\n").expect("write");
        assert_eq!(add_lesson(&file, "- Keep loops readable; no reduce.").expect("added"), 5);
        assert_eq!(add_lesson(&file, "Use the design tokens.").expect("added"), 6);
        assert_eq!(
            fs::read_to_string(&file).expect("read"),
            "# Acme\nRun the tests.\n\n## Lessons\n- Keep loops readable; no reduce.\n- Use the design tokens.\n"
        );
        fs::write(&file, "## Lessons\n- One.\n\n## Later\nText.\n").expect("write");
        add_lesson(&file, "Two.").expect("added");
        assert_eq!(fs::read_to_string(&file).expect("read"), "## Lessons\n- One.\n- Two.\n\n## Later\nText.\n");
        let new = dir.path().join("CLAUDE.md");
        assert_eq!(add_lesson(&new, "First.").expect("made"), 2);
        assert!(add_lesson(&new, "  ").is_err());
    }

    #[test]
    fn finds_what_is_said_twice() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let repo = dir.path();
        write(
            repo,
            "CLAUDE.md",
            "# Rules\n- **Never** push to main without a review.\n- Short line here.\n```\nnever push to main without a review\n```\n",
        );
        write(repo, ".claude/rules/git.md", "Never push to main, without a review!\n");
        write(repo, ".claude/CLAUDE.md", "Same file.\nWith two lines of six words or more.\n");
        write(repo, "apps/CLAUDE.md", "Same file.\nWith two lines of six words or more.\n");
        let mut sources = gather(repo, &repo.join("apps"), None);
        sources.retain(|source| source.kind != Kind::Hook);
        let found = repeats(&sources);
        assert!(found.iter().any(|repeat| repeat.text == "The whole file" && repeat.places.len() == 2));
        let push = found.iter().find(|repeat| repeat.text.contains("push to main")).expect("the push rule");
        assert_eq!(push.places.len(), 2, "the code block doesn't count: {push:?}");
        assert!(!found.iter().any(|repeat| repeat.text.contains("Short line")));
    }
}
