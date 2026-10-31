//! Work that spans repositories: one ticket that changes an app and its API, say. Each repository
//! gets a worktree for it on the same branch (`git::work_worktree` names both alike), and one agent
//! works in the first with the others added to what it may read and change (`--add-dir`, which
//! Claude Code and Codex both take).
//! The worktrees are linked by their branch alone, so nothing else needs remembering.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The worktrees of `repos` on `branch`, leaving out `own` (the checkout asking): the same work in
/// the other repositories. Main checkouts never count: every repository's is on its `main`.
#[must_use]
pub fn linked(branch: &str, repos: &[PathBuf], own: &Path) -> Vec<PathBuf> {
    let own = own.canonicalize().unwrap_or_else(|_| own.to_owned());
    let mut found = Vec::new();
    for repo in repos {
        for worktree in crate::worktrees::list(repo) {
            let path = worktree.path.canonicalize().unwrap_or_else(|_| worktree.path.clone());
            if worktree.branch.as_deref() == Some(branch)
                && !worktree.main
                && !worktree.missing
                && path != own
                && !found.contains(&path)
            {
                found.push(path);
            }
        }
    }
    found
}

/// An agent command that also reaches `folders`: `claude …` or `codex …` with `--add-dir` for each
/// (both take it), given as shell words by `quote`. Any other command is left as it is.
#[must_use]
pub fn with_folders(command: &str, folders: &[PathBuf], quote: impl Fn(&str) -> String) -> String {
    let Some(agent) = crate::agents::Agent::of_command(command) else { return command.to_owned() };
    let Some(rest) = command.strip_prefix(agent.program()) else { return command.to_owned() };
    if folders.is_empty() {
        return command.to_owned();
    }
    let mut added = String::new();
    for folder in folders {
        let _ = write!(added, " --add-dir {}", quote(&folder.to_string_lossy()));
    }
    format!("{}{added}{rest}", agent.program())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(text: &str) -> String {
        format!("'{text}'")
    }

    #[test]
    fn adds_folders_to_an_agent_command_only() {
        let folders = [PathBuf::from("/code/acme-api-abc-123"), PathBuf::from("/code/acme-docs-abc-123")];
        assert_eq!(
            with_folders("claude '/start ABC-123'", &folders, quote),
            "claude --add-dir '/code/acme-api-abc-123' --add-dir '/code/acme-docs-abc-123' '/start ABC-123'"
        );
        assert_eq!(with_folders("claude", &folders[..1], quote), "claude --add-dir '/code/acme-api-abc-123'");
        assert_eq!(with_folders("claude-other x", &folders, quote), "claude-other x");
        assert_eq!(
            with_folders("codex 'ABC-123'", &folders[..1], quote),
            "codex --add-dir '/code/acme-api-abc-123' 'ABC-123'"
        );
        assert_eq!(with_folders("aider 'ABC-123'", &folders, quote), "aider 'ABC-123'");
        assert_eq!(with_folders("claude x", &[], quote), "claude x");
    }

    #[test]
    fn finds_the_same_branch_in_the_other_repos() {
        let root = tempfile::tempdir().expect("temp dir");
        let mut repos = Vec::new();
        for name in ["acme-web", "acme-api", "acme-docs"] {
            let repo = root.path().join(name);
            std::fs::create_dir(&repo).expect("folder");
            for args in [
                &["init", "-q", "-b", "main"][..],
                &[
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@example.com",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    "start",
                ],
            ] {
                let output = crate::git::command(&repo).args(args).output().expect("git runs");
                assert!(output.status.success());
            }
            repos.push(repo);
        }
        let (web, _) = crate::git::work_worktree(&repos[0], "ABC-123").expect("worktree");
        let (api, _) = crate::git::work_worktree(&repos[1], "ABC-123").expect("worktree");
        let found = linked("abc-123", &repos, &web);
        assert_eq!(found, [api.canonicalize().expect("there")]);
        assert_eq!(linked("abc-999", &repos, &web), Vec::<PathBuf>::new());
    }
}
