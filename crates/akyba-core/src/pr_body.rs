//! A pull request's description against its repository's template: the template GitHub would start
//! it from, and which of the template's sections the description leaves empty or leaves out. A
//! description is what a reviewer reads first; an agent's is often thin where it matters (testing,
//! risks), and nobody notices until review.

use std::path::Path;

/// The places GitHub looks for a pull request template, in its order.
const TEMPLATES: [&str; 5] = [
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    "docs/pull_request_template.md",
    "pull_request_template.md",
    "PULL_REQUEST_TEMPLATE.md",
];

/// The repository's pull request template, and where it is, if it has one. A folder of templates
/// (`.github/PULL_REQUEST_TEMPLATE/`) gives its first, by name.
pub fn template(checkout: &Path) -> Option<(String, String)> {
    for path in TEMPLATES {
        if let Ok(text) = std::fs::read_to_string(checkout.join(path)) {
            return Some((path.to_owned(), text));
        }
    }
    let folder = checkout.join(".github/PULL_REQUEST_TEMPLATE");
    let mut files: Vec<_> = std::fs::read_dir(&folder).ok()?.flatten().map(|entry| entry.path()).collect();
    files.sort();
    let first = files.into_iter().find(|path| path.extension().is_some_and(|extension| extension == "md"))?;
    let text = std::fs::read_to_string(&first).ok()?;
    let name = first.strip_prefix(checkout).unwrap_or(&first).display().to_string();
    Some((name, text))
}

/// A Markdown text's sections: each heading's text, and what's under it until the next heading.
fn sections(text: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ') {
            found.push((trimmed.trim_start_matches('#').trim().to_owned(), String::new()));
        } else if let Some((_, body)) = found.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    found
}

/// Whether a section says anything: not only blank lines, HTML comments (a template's hints) or
/// unticked boxes.
fn says_something(body: &str) -> bool {
    let mut text = body.to_owned();
    while let Some(start) = text.find("<!--") {
        let end = text[start..].find("-->").map_or(text.len(), |end| start + end + 3);
        text.replace_range(start..end, "");
    }
    text.lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && line != "-" && line != "- [ ]" && !line.starts_with("- [ ] "))
}

/// The template's sections the description leaves out, and those it has but leaves empty.
pub fn gaps(template: &str, description: &str) -> (Vec<String>, Vec<String>) {
    let described = sections(description);
    let normalise = |heading: &str| heading.to_lowercase();
    let missing = sections(template)
        .into_iter()
        .map(|(heading, _)| heading)
        .filter(|heading| !described.iter().any(|(have, _)| normalise(have) == normalise(heading)))
        .collect();
    let empty = described.into_iter().filter(|(_, body)| !says_something(body)).map(|(heading, _)| heading).collect();
    (missing, empty)
}

/// What to tell an agent to write the description: the pull request, and the template to follow.
pub fn ask_message(number: u64, template: Option<&(String, String)>) -> String {
    let follow = match template {
        Some((path, _)) => format!("Follow the repository's template, {path}, and fill every section."),
        None => "Say what changed and why, how it was tested, and what a reviewer should look at.".to_owned(),
    };
    format!(
        "Write the description of pull request #{number}. {follow} Base it on the branch's changes against the base branch, not on this conversation alone. Then save it with `gh pr edit {number} --body-file <file>`.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "## What\n<!-- What changed -->\n\n## Why\n\n## Testing\n- [ ] Tests\n";

    #[test]
    fn finds_what_a_description_leaves_out_or_empty() {
        let description = "## What\nRounds VAT half up.\n\n## Testing\n<!-- How -->\n- [ ] Tests\n";
        let (missing, empty) = gaps(TEMPLATE, description);
        assert_eq!(missing, ["Why"]);
        assert_eq!(empty, ["Testing"]);
        let full = "## What\nRounds VAT.\n## Why\nIt was off by a cent.\n## Testing\n- [x] Tests\n";
        assert_eq!(gaps(TEMPLATE, full), (Vec::new(), Vec::new()));
    }

    #[test]
    fn finds_the_template_where_github_does() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        assert_eq!(template(dir.path()), None);
        std::fs::create_dir_all(dir.path().join(".github/PULL_REQUEST_TEMPLATE")).expect("make folders");
        std::fs::write(dir.path().join(".github/PULL_REQUEST_TEMPLATE/feature.md"), "## Feature\n").expect("write");
        assert_eq!(
            template(dir.path()).map(|(path, _)| path),
            Some(".github/PULL_REQUEST_TEMPLATE/feature.md".to_owned())
        );
        std::fs::write(dir.path().join(".github/pull_request_template.md"), TEMPLATE).expect("write");
        assert_eq!(template(dir.path()), Some((".github/pull_request_template.md".to_owned(), TEMPLATE.to_owned())));
        let message = ask_message(12, template(dir.path()).as_ref());
        assert!(
            message.contains("#12")
                && message.contains(".github/pull_request_template.md")
                && message.contains("gh pr edit 12")
        );
    }
}
