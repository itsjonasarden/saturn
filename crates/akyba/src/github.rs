//! Pull requests from GitHub through the `gh` CLI, already signed in on most machines that use
//! GitHub. Without `gh`, or outside a GitHub repository, there is simply no pull request.

use std::path::Path;
use std::process::{Command, Output};

use akyba_core::pull_request::{self, GH_FIELDS, GH_LIST_FIELDS, PullRequest};

/// A pull request by its branch, number or link, as `gh pr view` takes it, run in the repository
/// at `root` (a link works from any repository).
pub fn pull_request(root: &Path, which: &str) -> Option<PullRequest> {
    let output = gh(root, &["pr", "view", which, "--json", GH_FIELDS])?;
    pull_request::parse(&String::from_utf8_lossy(&output.stdout))
}

/// Where a pull request's commit is deployed for people to try, from GitHub's deployments.
pub fn deployment_preview(root: &Path, pr: &PullRequest) -> Option<pull_request::Preview> {
    let (owner, name) = pull_request::owner_and_name(&pr.url)?;
    let head = pr.head.as_deref()?;
    let output = gh(root, &["api", &format!("repos/{owner}/{name}/deployments?sha={head}&per_page=1")])?;
    let (id, environment) = pull_request::parse_deployment(&String::from_utf8_lossy(&output.stdout))?;
    let output = gh(root, &["api", &format!("repos/{owner}/{name}/deployments/{id}/statuses?per_page=1")])?;
    pull_request::parse_deployment_status(&String::from_utf8_lossy(&output.stdout), &environment)
}

/// A pull request's description as it is on GitHub.
pub fn pr_body(root: &Path, number: u64) -> Option<String> {
    let output = gh(root, &["pr", "view", &number.to_string(), "--json", "body", "--jq", ".body"])?;
    Some(String::from_utf8_lossy(&output.stdout).trim_end().to_owned())
}

/// Replaces a pull request's description on GitHub, saying why when it can't.
pub fn set_pr_body(root: &Path, number: u64, body: &str) -> Result<(), String> {
    let file = std::env::temp_dir().join(format!("akyba-pr-{number}-{}.md", std::process::id()));
    std::fs::write(&file, body).map_err(|error| error.to_string())?;
    let done = gh(root, &["pr", "edit", &number.to_string(), "--body-file", &file.to_string_lossy()]);
    let _ = std::fs::remove_file(&file);
    done.map(|_| ()).ok_or_else(|| "gh couldn't save it (signed in, and allowed to edit the pull request?)".to_owned())
}

/// What reviewers left open on a pull request: its unresolved threads and requests for changes.
pub fn review_notes(root: &Path, pr: &PullRequest) -> Option<pull_request::ReviewNotes> {
    let (owner, name) = pull_request::owner_and_name(&pr.url)?;
    let output = gh(
        root,
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={}", pull_request::REVIEW_QUERY),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={}", pr.number),
        ],
    )?;
    pull_request::parse_review_notes(&String::from_utf8_lossy(&output.stdout))
}

/// Every branch's pull request in the repository at `root` (the newest 200), in one call.
pub fn pull_requests_by_branch(root: &Path) -> std::collections::HashMap<String, PullRequest> {
    gh(root, &["pr", "list", "--state", "all", "--limit", "200", "--json", GH_LIST_FIELDS])
        .map(|output| pull_request::by_branch(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

/// The pull request about a ticket in the repository at `root`: one that names it, open first.
pub fn pull_request_for_ticket(root: &Path, ticket: &str) -> Option<PullRequest> {
    let output =
        gh(root, &["pr", "list", "--state", "all", "--limit", "20", "--search", ticket, "--json", GH_LIST_FIELDS])?;
    pull_request::for_ticket(&String::from_utf8_lossy(&output.stdout), ticket)
}

fn gh(root: &Path, args: &[&str]) -> Option<Output> {
    let mut command = Command::new("gh");
    command.args(args).current_dir(root);
    // Started from Finder, Akyba's own PATH lacks Homebrew's bin, where `gh` usually lives.
    let path =
        crate::LOGIN_PATH.get().cloned().unwrap_or_else(|| "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_owned());
    command.env("PATH", path).env("GH_PROMPT_DISABLED", "1");
    let output = command.output().ok()?;
    output.status.success().then_some(output)
}
