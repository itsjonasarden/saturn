//! The pull request a pane works on, as GitHub's `gh` reports it, so a pane can show its number and
//! whether it's open, a draft, merged or closed. A pane's pull request is the one last linked in its
//! terminal (an agent printing the one it opened, or a link you pasted), else its branch's, else one
//! that names the pane's ticket. The `gh` calls live in the app.

use std::fmt::Write as _;

use serde::Deserialize;

/// The fields Akyba asks `gh pr view` for.
pub const GH_FIELDS: &str = "number,state,isDraft,url,title,statusCheckRollup,headRefOid";

/// The fields Akyba asks `gh pr list` for, to find a ticket's pull request.
pub const GH_LIST_FIELDS: &str = "number,state,isDraft,url,title,headRefName,statusCheckRollup,headRefOid";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PrState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Draft => "Draft",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    /// Its checks (CI), as of its last commit.
    pub checks: Checks,
    /// The commit its branch is at.
    pub head: Option<String>,
    /// A preview its checks link to, if any.
    pub preview: Option<Preview>,
}

/// A pull request's checks, by how they went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks {
    pub passed: usize,
    pub running: Vec<String>,
    pub failed: Vec<Check>,
}

/// A check that failed: its name, and where to read why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub url: Option<String>,
}

/// How a pull request's checks stand, all together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksState {
    Passed,
    Running,
    Failed,
}

impl Checks {
    /// Failed if any did, else running if any are, else passed; `None` with no checks at all.
    pub fn state(&self) -> Option<ChecksState> {
        if !self.failed.is_empty() {
            Some(ChecksState::Failed)
        } else if !self.running.is_empty() {
            Some(ChecksState::Running)
        } else if self.passed > 0 {
            Some(ChecksState::Passed)
        } else {
            None
        }
    }

    /// Reads `statusCheckRollup`: check runs (GitHub Actions and apps) and commit statuses.
    fn from_rollup(rollup: &[serde_json::Value]) -> Self {
        let mut checks = Self::default();
        for check in rollup {
            let text = |key: &str| check.get(key).and_then(serde_json::Value::as_str).unwrap_or_default();
            let (name, url) = if text("__typename") == "StatusContext" {
                (text("context"), text("targetUrl"))
            } else {
                (text("name"), text("detailsUrl"))
            };
            let outcome = if text("__typename") == "StatusContext" {
                match text("state") {
                    "SUCCESS" => Some(true),
                    "FAILURE" | "ERROR" => Some(false),
                    _ => None,
                }
            } else if text("status") == "COMPLETED" {
                Some(!matches!(
                    text("conclusion"),
                    "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE"
                ))
            } else {
                None
            };
            match outcome {
                Some(true) => checks.passed += 1,
                Some(false) => {
                    checks.failed.push(Check { name: name.to_owned(), url: (!url.is_empty()).then(|| url.to_owned()) });
                }
                None => checks.running.push(name.to_owned()),
            }
        }
        checks
    }
}

/// What to tell an agent when a pull request's checks failed: which, where to read why, and how to
/// get the logs from the terminal.
pub fn failed_checks_message(pr: &PullRequest) -> String {
    let mut out = format!("The checks of pull request #{} ({}) failed:\n", pr.number, pr.title);
    for check in &pr.checks.failed {
        let _ = match &check.url {
            Some(url) => writeln!(out, "- {}: {url}", check.name),
            None => writeln!(out, "- {}", check.name),
        };
    }
    let _ = writeln!(
        out,
        "\nSee why with `gh pr checks {}`, and for a GitHub Actions run `gh run view <run id> --log-failed` (the run id is in its link). Fix the cause, run the same checks here, and push.",
        pr.number
    );
    out
}

/// Where a pull request's work runs for people to try: a deployment of its commit (Vercel, Netlify,
/// a workflow), or a check that links to one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The site; `None` while it's still deploying.
    pub url: Option<String>,
    /// The environment's name (`preview`, `pr-12`), or the check's.
    pub name: String,
    /// It deployed; `false` while it's on its way.
    pub ready: bool,
}

/// The newest deployment in a `GET /repos/{owner}/{repo}/deployments?sha=…` answer: its id and
/// environment.
pub fn parse_deployment(json: &str) -> Option<(u64, String)> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let newest = list.first()?;
    let environment = newest.get("environment").and_then(serde_json::Value::as_str).unwrap_or("preview").to_owned();
    Some((newest.get("id")?.as_u64()?, environment))
}

/// A deployment's newest status (`GET …/deployments/{id}/statuses`) as a preview: only a deployed
/// site counts, not a link to the job that deployed it.
pub fn parse_deployment_status(json: &str, environment: &str) -> Option<Preview> {
    let list: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let newest = list.first()?;
    let state = newest.get("state").and_then(serde_json::Value::as_str).unwrap_or_default();
    let url = newest.get("environment_url").and_then(serde_json::Value::as_str).filter(|url| !url.is_empty());
    match state {
        "success" => url.map(|url| Preview { url: Some(url.to_owned()), name: environment.to_owned(), ready: true }),
        "queued" | "pending" | "in_progress" => Some(Preview { url: None, name: environment.to_owned(), ready: false }),
        _ => None,
    }
}

/// A preview a check links to, for setups that only post a status: one named like a preview or a
/// deploy whose link isn't GitHub's own.
pub fn preview_in_checks(rollup: &[serde_json::Value]) -> Option<Preview> {
    rollup.iter().find_map(|check| {
        let text = |key: &str| check.get(key).and_then(serde_json::Value::as_str).unwrap_or_default();
        let name = if text("__typename") == "StatusContext" { text("context") } else { text("name") };
        let url = if text("__typename") == "StatusContext" { text("targetUrl") } else { text("detailsUrl") };
        let lower = name.to_lowercase();
        let named = lower.contains("preview") || lower.contains("deploy");
        let success = text("state") == "SUCCESS" || text("conclusion") == "SUCCESS";
        (named && success && url.starts_with("http") && !url.contains("github.com/")).then(|| Preview {
            url: Some(url.to_owned()),
            name: name.to_owned(),
            ready: true,
        })
    })
}

/// What reviewers asked of a pull request that isn't settled: open review threads on lines, and
/// reviews that requested changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewNotes {
    pub threads: Vec<Thread>,
    /// Reviews that requested changes, with what they said: (author, text).
    pub changes_requested: Vec<(String, String)>,
}

/// An open review thread: the file and line it's on, and its comments, (author, text) in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub path: String,
    pub line: Option<u64>,
    pub comments: Vec<(String, String)>,
}

impl ReviewNotes {
    /// How many things there are to address.
    pub fn count(&self) -> usize {
        self.threads.len() + self.changes_requested.len()
    }
}

/// GitHub's GraphQL query for a pull request's reviews and review threads, for `gh api graphql`
/// with `owner`, `name` and `number`.
pub const REVIEW_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviews(last:30){nodes{state body author{login}}} reviewThreads(first:100){nodes{isResolved isOutdated path line comments(first:30){nodes{body author{login}}}}}}}}";

/// The owner and name of the repository a pull request's address is in.
pub fn owner_and_name(url: &str) -> Option<(String, String)> {
    let mut parts = url.split("github.com/").nth(1)?.split('/');
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

/// Reads the answer to `REVIEW_QUERY`: open threads that still apply to the code, and the latest
/// review of each reviewer when it requested changes.
pub fn parse_review_notes(json: &str) -> Option<ReviewNotes> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let pr = value.get("data")?.get("repository")?.get("pullRequest")?;
    let nodes = |key: &str| {
        pr.get(key)
            .and_then(|list| list.get("nodes"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let said = |node: &serde_json::Value| {
        let author = node.get("author").and_then(|author| author.get("login")).and_then(serde_json::Value::as_str);
        let body = node.get("body").and_then(serde_json::Value::as_str).unwrap_or_default().trim().to_owned();
        (author.unwrap_or("someone").to_owned(), body)
    };
    let threads = nodes("reviewThreads")
        .iter()
        .filter(|thread| {
            let flag = |key: &str| thread.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);
            !flag("isResolved") && !flag("isOutdated")
        })
        .map(|thread| Thread {
            path: thread.get("path").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
            line: thread.get("line").and_then(serde_json::Value::as_u64),
            comments: thread
                .get("comments")
                .and_then(|comments| comments.get("nodes"))
                .and_then(serde_json::Value::as_array)
                .map(|comments| comments.iter().map(said).filter(|(_, body)| !body.is_empty()).collect())
                .unwrap_or_default(),
        })
        .collect();
    // Each reviewer's latest review decides; an approval after a request for changes settles it.
    let mut latest: Vec<(String, String, String)> = Vec::new();
    for review in nodes("reviews") {
        let state = review.get("state").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
        if state == "COMMENTED" || state == "PENDING" {
            continue;
        }
        let (author, body) = said(&review);
        latest.retain(|(who, _, _)| *who != author);
        latest.push((author, state, body));
    }
    let changes_requested = latest
        .into_iter()
        .filter(|(_, state, _)| state == "CHANGES_REQUESTED")
        .map(|(author, _, body)| (author, body))
        .collect();
    Some(ReviewNotes { threads, changes_requested })
}

/// What to tell an agent about a pull request's open review: each thread with its file, line and
/// conversation, then the reviews that requested changes.
pub fn review_notes_message(pr: &PullRequest, notes: &ReviewNotes) -> String {
    let mut out =
        format!("Reviewers left these on pull request #{} ({}). Please address each one:\n", pr.number, pr.title);
    for (index, thread) in notes.threads.iter().enumerate() {
        let place = thread.line.map_or_else(|| thread.path.clone(), |line| format!("{}, line {line}", thread.path));
        let _ = writeln!(out, "\n{}. {place}:", index + 1);
        for (author, body) in &thread.comments {
            let _ = writeln!(out, "   {author}: {body}");
        }
    }
    for (author, body) in &notes.changes_requested {
        let _ = writeln!(
            out,
            "\n{author} requested changes{}",
            if body.is_empty() { ".".to_owned() } else { format!(": {body}") }
        );
    }
    out.push_str("\nWhen a thread is handled, say so in your reply, so I can resolve it.\n");
    out
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    number: u64,
    state: String,
    #[serde(default)]
    is_draft: bool,
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    status_check_rollup: Vec<serde_json::Value>,
    #[serde(default)]
    head_ref_oid: Option<String>,
}

impl Raw {
    /// A draft is open on GitHub, but it reads differently, so it gets its own state.
    fn into_pull_request(self) -> Option<PullRequest> {
        let state = match self.state.as_str() {
            "OPEN" if self.is_draft => PrState::Draft,
            "OPEN" => PrState::Open,
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ => return None,
        };
        Some(PullRequest {
            number: self.number,
            title: self.title,
            url: self.url,
            state,
            checks: Checks::from_rollup(&self.status_check_rollup),
            preview: preview_in_checks(&self.status_check_rollup),
            head: self.head_ref_oid,
        })
    }
}

/// Reads `gh pr view --json number,state,isDraft,url,title`.
pub fn parse(json: &str) -> Option<PullRequest> {
    serde_json::from_str::<Raw>(json).ok()?.into_pull_request()
}

/// The pull request of each branch from `gh pr list --state all --json …`: an open one before a
/// merged or closed one, the newest first.
pub fn by_branch(json: &str) -> std::collections::HashMap<String, PullRequest> {
    let mut found: Vec<(String, PullRequest)> = serde_json::from_str::<Vec<Raw>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|raw| {
            let branch = raw.head_ref_name.clone();
            raw.into_pull_request().map(|pr| (branch, pr))
        })
        .collect();
    // Best last, so it's the one the map keeps.
    found.sort_by_key(|(_, pr)| (!matches!(pr.state, PrState::Merged | PrState::Closed), pr.number));
    found.into_iter().collect()
}

/// The pull request for a ticket from `gh pr list --search <ticket> --json …`: one whose title or
/// branch names it, open before merged or closed, the newest first. One whose description only
/// mentions it isn't the ticket's: a subtask's "part of ABC-12", another ticket's "see ABC-12", and a
/// merged one of those took the pane of work that had just begun. GitHub's search also matches the
/// ticket's parts on their own, so only whole mentions count.
pub fn for_ticket(json: &str, ticket: &str) -> Option<PullRequest> {
    let found: Vec<Raw> = serde_json::from_str(json).ok()?;
    found
        .into_iter()
        .filter(|raw| names_ticket(&raw.title, ticket) || names_ticket(&raw.head_ref_name, ticket))
        .filter_map(Raw::into_pull_request)
        .min_by_key(|pr| (matches!(pr.state, PrState::Merged | PrState::Closed), std::cmp::Reverse(pr.number)))
}

/// Whether `text` names `ticket` as a whole word, in any case: `ABC-12` is in "abc-12-fix" and
/// "fix: ABC-12", not in "ABC-123".
fn names_ticket(text: &str, ticket: &str) -> bool {
    let (text, ticket) = (text.to_ascii_lowercase(), ticket.to_ascii_lowercase());
    text.match_indices(&ticket).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + ticket.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// The ticket a pane's name is about: the first ticket-shaped word, `ABC-123`, upper-cased.
/// Claude's titles carry a mark before the name ("✳ ABC-123"), so every word is tried.
pub fn ticket_in(name: &str) -> Option<String> {
    name.split(|c: char| c.is_whitespace() || matches!(c, '/' | ':' | '(' | ')' | '[' | ']')).find_map(|word| {
        let (prefix, number) = word.split_once('-')?;
        let number = number.split('-').next().unwrap_or_default();
        let valid = prefix.len() >= 2
            && prefix.chars().all(|c| c.is_ascii_alphanumeric())
            && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit());
        valid.then(|| format!("{}-{number}", prefix.to_ascii_uppercase()))
    })
}

/// How much of a terminal's output is kept between reads, so a link split across two reads is
/// still found: longer than any pull request link (GitHub's owner and repository names are short).
const TAIL: usize = 256;

/// Finds GitHub pull request links in a terminal's output as it arrives, without reading the
/// screen again: an agent printing the pull request it opened, or a link you pasted.
#[derive(Debug, Default)]
pub struct LinkScanner {
    /// The end of the last read, which a link may continue from.
    tail: Vec<u8>,
}

impl LinkScanner {
    /// Reads more output and returns the last complete pull request link in it, as
    /// `https://github.com/owner/repo/pull/123`. A link cut off at the end of the read waits for
    /// the next one.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<String> {
        const HOST: &[u8] = b"github.com/";
        let mut text = std::mem::take(&mut self.tail);
        text.extend_from_slice(bytes);
        let mut last = None;
        // Everything before `read` has been read; a link cut off at the end starts there.
        let mut read = 0;
        while let Some(at) = find(&text[read..], HOST).map(|at| read + at) {
            match link_at(&text, at + HOST.len()) {
                Link::Found(link, end) => {
                    last = Some(link);
                    read = end;
                }
                Link::Unfinished => {
                    read = at;
                    break;
                }
                Link::None => read = at + 1,
            }
        }
        // The unread end, which may hold the start of a link (even of `github.com/` itself).
        let keep = read.max(text.len().saturating_sub(TAIL));
        self.tail = text[keep..].to_vec();
        last
    }
}

enum Link {
    Found(String, usize),
    /// The output ends before the link does.
    Unfinished,
    None,
}

/// Reads `owner/repo/pull/123` from `at` on. While nothing after `at` has ended the link (a space,
/// a bracket, an escape), more of it may still come.
fn link_at(text: &[u8], at: usize) -> Link {
    let link_byte = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/');
    let end = at + text[at..].iter().take_while(|b| link_byte(b)).count();
    if end == text.len() {
        return Link::Unfinished;
    }
    // A sentence may end right after the link: its full stop isn't part of the link.
    let end = end - text[at..end].iter().rev().take_while(|&&b| b == b'.').count();
    let mut parts = std::str::from_utf8(&text[at..end]).unwrap_or_default().split('/');
    let (Some(owner), Some(repo), Some("pull"), Some(number)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Link::None;
    };
    let valid = !owner.is_empty()
        && !repo.is_empty()
        && !number.is_empty()
        && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && number.bytes().all(|b| b.is_ascii_digit());
    if !valid {
        return Link::None;
    }
    Link::Found(format!("https://github.com/{owner}/{repo}/pull/{number}"), end)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_where_a_pull_request_runs() {
        assert_eq!(
            super::parse_deployment(r#"[{"id": 7, "environment": "pr-12"}, {"id": 6}]"#),
            Some((7, "pr-12".to_owned()))
        );
        let live = super::parse_deployment_status(
            r#"[{"state": "success", "environment_url": "https://pr-12.acme.dev", "target_url": "https://github.com/acme/x/actions/runs/1"}]"#,
            "pr-12",
        );
        assert_eq!(
            live,
            Some(super::Preview { url: Some("https://pr-12.acme.dev".into()), name: "pr-12".into(), ready: true })
        );
        assert_eq!(
            super::parse_deployment_status(r#"[{"state": "in_progress"}]"#, "pr-12").map(|p| p.ready),
            Some(false)
        );
        assert_eq!(
            super::parse_deployment_status(
                r#"[{"state": "success", "environment_url": "", "target_url": "https://github.com/acme/x/actions/runs/1"}]"#,
                "dev"
            ),
            None,
            "a job log isn't a preview"
        );
        let rollup: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"__typename": "CheckRun", "name": "test", "conclusion": "SUCCESS", "detailsUrl": "https://github.com/acme/x/runs/1"},
            {"__typename": "StatusContext", "context": "Vercel – Preview", "state": "SUCCESS", "targetUrl": "https://acme-git-cart.vercel.app"}
        ]"#).expect("json");
        assert_eq!(
            super::preview_in_checks(&rollup).and_then(|p| p.url).as_deref(),
            Some("https://acme-git-cart.vercel.app")
        );
    }

    #[test]
    fn reads_what_reviewers_left_open() {
        let json = r#"{"data":{"repository":{"pullRequest":{
            "reviews":{"nodes":[
                {"state":"CHANGES_REQUESTED","body":"Needs a test.","author":{"login":"alice"}},
                {"state":"COMMENTED","body":"Nice","author":{"login":"carol"}},
                {"state":"CHANGES_REQUESTED","body":"Rename it.","author":{"login":"bob"}},
                {"state":"APPROVED","body":"","author":{"login":"bob"}}
            ]},
            "reviewThreads":{"nodes":[
                {"isResolved":false,"isOutdated":false,"path":"src/cart.ts","line":12,"comments":{"nodes":[
                    {"body":"Round to cents here.","author":{"login":"alice"}},{"body":"Agreed","author":{"login":"bob"}}]}},
                {"isResolved":true,"isOutdated":false,"path":"src/a.ts","line":1,"comments":{"nodes":[{"body":"done","author":{"login":"alice"}}]}},
                {"isResolved":false,"isOutdated":true,"path":"src/b.ts","line":2,"comments":{"nodes":[{"body":"old","author":{"login":"alice"}}]}}
            ]}}}}}"#;
        let notes = super::parse_review_notes(json).expect("notes");
        assert_eq!(notes.threads.len(), 1);
        assert_eq!(notes.threads[0].comments[1], ("bob".to_owned(), "Agreed".to_owned()));
        assert_eq!(notes.changes_requested, [("alice".to_owned(), "Needs a test.".to_owned())], "bob approved after");
        assert_eq!(notes.count(), 2);
        let pr = super::parse(
            r#"{"number": 160, "state": "OPEN", "url": "https://github.com/acme/acme-web/pull/160", "title": "Cart"}"#,
        )
        .expect("pr");
        assert_eq!(super::owner_and_name(&pr.url), Some(("acme".to_owned(), "acme-web".to_owned())));
        let message = super::review_notes_message(&pr, &notes);
        assert!(message.contains("1. src/cart.ts, line 12:\n   alice: Round to cents here.\n   bob: Agreed\n"));
        assert!(message.contains("alice requested changes: Needs a test."));
    }

    #[test]
    fn reads_how_the_checks_went() {
        let json = r#"{"number": 160, "state": "OPEN", "url": "u", "title": "Faster cube loads", "headRefOid": "abc",
            "statusCheckRollup": [
                {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "SUCCESS", "detailsUrl": "https://ci/1"},
                {"__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "https://ci/2"},
                {"__typename": "CheckRun", "name": "e2e", "status": "IN_PROGRESS", "conclusion": ""},
                {"__typename": "StatusContext", "context": "deploy/preview", "state": "PENDING", "targetUrl": "https://preview"},
                {"__typename": "StatusContext", "context": "coverage", "state": "SUCCESS"}
            ]}"#;
        let pr = super::parse(json).expect("a pull request");
        assert_eq!(pr.checks.passed, 2);
        assert_eq!(pr.checks.running, ["e2e", "deploy/preview"]);
        assert_eq!(pr.checks.failed, [super::Check { name: "test".into(), url: Some("https://ci/2".into()) }]);
        assert_eq!(pr.checks.state(), Some(super::ChecksState::Failed));
        assert_eq!(pr.head.as_deref(), Some("abc"));
        let message = super::failed_checks_message(&pr);
        assert!(
            message.starts_with("The checks of pull request #160 (Faster cube loads) failed:\n- test: https://ci/2\n")
        );
        assert!(message.contains("gh pr checks 160"));
        let none = super::parse(r#"{"number": 1, "state": "OPEN", "url": "u"}"#).expect("a pull request");
        assert_eq!(none.checks.state(), None);
    }

    #[test]
    fn finds_each_branch_its_best_pull_request() {
        let json = r#"[
            {"number": 3, "state": "CLOSED", "url": "u3", "headRefName": "abc-1"},
            {"number": 7, "state": "OPEN", "isDraft": true, "url": "u7", "headRefName": "abc-1"},
            {"number": 5, "state": "MERGED", "url": "u5", "headRefName": "abc-2"}
        ]"#;
        let found = super::by_branch(json);
        assert_eq!(found["abc-1"].number, 7);
        assert_eq!(found["abc-2"].state, super::PrState::Merged);
    }

    use super::*;

    fn view(state: &str, draft: bool) -> String {
        format!(
            r#"{{"isDraft":{draft},"number":42,"state":"{state}","title":"Add dark mode","url":"https://github.com/acme/web/pull/42"}}"#
        )
    }

    #[test]
    fn reads_each_state() {
        assert_eq!(parse(&view("OPEN", false)).map(|pr| pr.state), Some(PrState::Open));
        assert_eq!(parse(&view("OPEN", true)).map(|pr| pr.state), Some(PrState::Draft));
        assert_eq!(parse(&view("MERGED", false)).map(|pr| pr.state), Some(PrState::Merged));
        assert_eq!(parse(&view("CLOSED", false)).map(|pr| pr.state), Some(PrState::Closed));
    }

    #[test]
    fn keeps_the_number_title_and_link() {
        let pr = parse(&view("OPEN", false)).expect("parses");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Add dark mode");
        assert_eq!(pr.url, "https://github.com/acme/web/pull/42");
    }

    #[test]
    fn finds_the_last_link_in_output() {
        let mut scanner = LinkScanner::default();
        let output = b"PR #158\r\n(https://github.com/acme/acme-web/pull/157), then \x1b[34mhttps://github.com/acme/acme-web/pull/158/files\x1b[0m.";
        assert_eq!(scanner.feed(output).as_deref(), Some("https://github.com/acme/acme-web/pull/158"));
        assert_eq!(scanner.feed(b"nothing new"), None, "a link is reported once");
        assert_eq!(
            scanner.feed(b"See https://github.com/acme/web.site/pull/7.\r\n").as_deref(),
            Some("https://github.com/acme/web.site/pull/7")
        );
    }

    #[test]
    fn finds_a_link_split_between_reads() {
        let mut scanner = LinkScanner::default();
        assert_eq!(scanner.feed(b"opened https://github.com/acme/acme-web/pu"), None);
        assert_eq!(scanner.feed(b"ll/42"), None, "the number may go on");
        assert_eq!(scanner.feed(b"1 for review").as_deref(), Some("https://github.com/acme/acme-web/pull/421"));
    }

    #[test]
    fn ignores_links_that_are_not_pull_requests() {
        let mut scanner = LinkScanner::default();
        let output =
            b"https://github.com/acme/acme-web/pulls https://github.com/acme/acme-web/issues/3 github.com/acme";
        assert_eq!(scanner.feed(output), None);
        assert_eq!(scanner.feed(b"/acme-web/pull/9 "), Some("https://github.com/acme/acme-web/pull/9".into()));
    }

    #[test]
    fn reads_a_ticket_from_a_pane_name() {
        assert_eq!(ticket_in("ABC-123").as_deref(), Some("ABC-123"));
        assert_eq!(ticket_in("✳ abc-123 review").as_deref(), Some("ABC-123"));
        assert_eq!(ticket_in("feat/abc-12-dark-mode").as_deref(), Some("ABC-12"));
        assert_eq!(ticket_in("zsh"), None);
        assert_eq!(ticket_in("fix/vat-rounding"), None);
    }

    #[test]
    fn picks_a_tickets_open_pull_request_first() {
        let list = r#"[
            {"number":9,"state":"MERGED","isDraft":false,"url":"u9","title":"ABC-12: first try","headRefName":"x","body":""},
            {"number":11,"state":"OPEN","isDraft":false,"url":"u11","title":"Dark mode","headRefName":"abc-12-dark","body":""},
            {"number":12,"state":"OPEN","isDraft":false,"url":"u12","title":"ABC-123 is another ticket","headRefName":"y","body":""}
        ]"#;
        assert_eq!(for_ticket(list, "ABC-12").map(|pr| pr.number), Some(11));
        assert_eq!(for_ticket(list, "ABC-99"), None);
        assert_eq!(for_ticket("[]", "ABC-12"), None);
    }

    #[test]
    fn a_subtask_mentioning_the_ticket_is_not_its_pull_request() {
        let list = r#"[
            {"number":163,"state":"OPEN","isDraft":true,"url":"u163","title":"Rewrite comments","headRefName":"feat/ABC-276-comments","body":"Part of ABC-274."},
            {"number":162,"state":"OPEN","isDraft":true,"url":"u162","title":"ABC-274: writing standard","headRefName":"feat/ABC-274-standard","body":""}
        ]"#;
        assert_eq!(for_ticket(list, "ABC-274").map(|pr| pr.number), Some(162));
        // Another ticket's merged pull request that mentions this one isn't this one's.
        let only_mentioned = r#"[{"number":131,"state":"MERGED","isDraft":false,"url":"u","title":"Close the gaps","headRefName":"fix/ABC-275-gaps","body":"Follows ABC-274."}]"#;
        assert_eq!(for_ticket(only_mentioned, "ABC-274"), None);
    }

    #[test]
    fn anything_else_is_no_pull_request() {
        assert_eq!(parse("no pull requests found for branch \"main\""), None);
        assert_eq!(parse(&view("UNKNOWN", false)), None);
    }
}
