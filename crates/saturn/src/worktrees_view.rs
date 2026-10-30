//! Worktrees (from the palette): every worktree of the repo, with its branch, when it was last
//! committed to, whether its work is merged (squash merges too), its pull request, and what would stop
//! it going: uncommitted work, an open pull request, a pane or a program still in it. The ones with
//! nothing in the way go with one click, their recipe's `cleanup` first; their ports are free again.
//! One that isn't finished but isn't needed now can be archived: its branch and its uncommitted work
//! are kept, the folder goes, and Restore brings it back where it was, set up again.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use saturn_core::pull_request::{PrState, PullRequest};
use saturn_core::recipe;
use saturn_core::worktrees::{self, Archived, Archives, Merged, Worktree};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::{self, len};
use crate::workspace::Workspace;

/// A worktree and what's known about it.
#[derive(Clone)]
struct Row {
    worktree: Worktree,
    merged: Merged,
    uncommitted: usize,
    /// Programs whose working folder is in it.
    programs: Vec<String>,
    /// A Saturn pane works in it.
    in_pane: bool,
    pull_request: Option<PullRequest>,
    last_commit: Option<i64>,
}

impl Row {
    /// Why it can't be removed yet, in words; `None` when nothing is in the way.
    fn blocker(&self) -> Option<String> {
        if self.worktree.main {
            return Some("The main checkout".to_owned());
        }
        if self.worktree.missing {
            return None;
        }
        if self.uncommitted > 0 {
            let s = if self.uncommitted == 1 { "" } else { "s" };
            return Some(format!("{} uncommitted change{s}", self.uncommitted));
        }
        if let Some(pr) = self.pull_request.as_ref().filter(|pr| matches!(pr.state, PrState::Open | PrState::Draft)) {
            return Some(format!("Pull request #{} is open", pr.number));
        }
        if self.merged == Merged::No {
            return Some("Not merged".to_owned());
        }
        if self.in_pane {
            return Some("Open in a pane".to_owned());
        }
        if !self.programs.is_empty() {
            return Some(format!("{} running in it", self.programs.join(", ")));
        }
        None
    }
}

pub struct WorktreesView {
    workspace: WeakEntity<Workspace>,
    /// The main checkout.
    repo: PathBuf,
    /// Folders Saturn's panes work in.
    pane_folders: Vec<PathBuf>,
    /// `None` while they're read.
    rows: Option<Vec<Row>>,
    /// The repo's archived worktrees, newest first.
    archived: Vec<Archived>,
    removing: HashSet<PathBuf>,
    /// What the last removal said.
    said: Option<String>,
    /// The window it's in, where a restored worktree opens its pane.
    window: gpui_kit::AnyWindowHandle,
    /// A spike pressed its button already.
    spiked: bool,
}

impl WorktreesView {
    fn new(
        workspace: WeakEntity<Workspace>,
        repo: PathBuf,
        pane_folders: Vec<PathBuf>,
        window: gpui_kit::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            workspace,
            repo,
            pane_folders,
            rows: None,
            archived: Vec::new(),
            removing: HashSet::new(),
            said: None,
            window,
            spiked: false,
        };
        view.read(cx);
        view
    }

    /// Reads every worktree and its state, off the main thread: git, `lsof` and `gh` all take a while.
    fn read(&mut self, cx: &mut Context<Self>) {
        let (repo, panes) = (self.repo.clone(), self.pane_folders.clone());
        cx.spawn(async move |this, cx| {
            let (rows, archived) = cx
                .background_spawn(async move {
                    let mut archived =
                        archives_file().map(|file| Archives::load(&file).of(&repo).to_vec()).unwrap_or_default();
                    archived.sort_by_key(|archived| std::cmp::Reverse(archived.at));
                    (read_rows(&repo, &panes), archived)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.rows = Some(rows);
                this.archived = archived;
                this.act_from_env(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Spikes press Archive or Restore by the worktree's folder name (`SATURN_WORKTREE_ARCHIVE`,
    /// `SATURN_WORKTREE_RESTORE`), or Remove the finished ones (`SATURN_WORKTREE_REMOVE_FINISHED`),
    /// once, as a click would.
    fn act_from_env(&mut self, cx: &mut Context<Self>) {
        let named = |key: &str| std::env::var(key).ok().filter(|name| !name.is_empty());
        if self.spiked {
            return;
        }
        if let Some(name) = named("SATURN_WORKTREE_ARCHIVE") {
            let found = self.rows.iter().flatten().find(|row| folder_name(&row.worktree.path) == name).cloned();
            if let Some(row) = found {
                self.spiked = true;
                self.archive(row.worktree, cx);
            }
        }
        if named("SATURN_WORKTREE_REMOVE_FINISHED").is_some() && self.rows.is_some() {
            let finished: Vec<Worktree> = self
                .rows
                .iter()
                .flatten()
                .filter(|row| row.blocker().is_none())
                .map(|row| row.worktree.clone())
                .collect();
            self.spiked = true;
            eprintln!("removing {} finished worktrees", finished.len());
            self.remove(finished, cx);
        }
        if let Some(name) = named("SATURN_WORKTREE_RESTORE") {
            let found = self.archived.iter().find(|archived| folder_name(&archived.path) == name).cloned();
            if let Some(archived) = found {
                self.spiked = true;
                self.restore(archived, cx);
            }
        }
    }

    /// Puts a worktree away, keeping its branch and uncommitted work, then reads the list again.
    fn archive(&mut self, worktree: Worktree, cx: &mut Context<Self>) {
        self.removing.insert(worktree.path.clone());
        self.said = None;
        cx.notify();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let said = cx
                .background_spawn(async move {
                    let name = folder_name(&worktree.path);
                    match archive_now(&repo, &worktree) {
                        Ok(()) => format!("Archived {name}: Restore brings it back with its uncommitted work."),
                        Err(problem) => format!("{name} stays: {problem}"),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.removing.clear();
                this.said = Some(said);
                this.read(cx);
            });
        })
        .detach();
    }

    /// Brings an archived worktree back and opens a pane in it that sets it up again.
    fn restore(&mut self, archived: Archived, cx: &mut Context<Self>) {
        self.removing.insert(archived.path.clone());
        self.said = None;
        cx.notify();
        let (repo, window) = (self.repo.clone(), self.window);
        cx.spawn(async move |this, cx| {
            let path = archived.path.clone();
            let restored = cx
                .background_spawn(async move {
                    worktrees::restore(&repo, &archived)?;
                    if let Some(file) = archives_file() {
                        let mut list = Archives::load(&file);
                        list.forget(&repo, &archived.path);
                        let _ = list.save(&file);
                    }
                    Ok::<(), String>(())
                })
                .await;
            let workspace = this.update(cx, |this, cx| {
                this.removing.clear();
                if let Err(problem) = &restored {
                    this.said = Some(format!("{} stays archived: {problem}", folder_name(&path)));
                }
                cx.notify();
                this.workspace.clone()
            });
            if let (Ok(()), Ok(workspace)) = (restored, workspace) {
                let _ = window.update(cx, |_, window, cx| {
                    if let Some(workspace) = workspace.upgrade() {
                        workspace.update(cx, |workspace, cx| workspace.open_restored(&path, window, cx));
                    }
                    window.close_dialog(cx);
                });
            }
        })
        .detach();
    }

    fn render_archived(&self, archived: &Archived, now: i64, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let restoring = self.removing.contains(&archived.path);
        let picked = archived.clone();
        let action = if restoring {
            div().text_color(theme::color(color::INK_MUTED)).child("Restoring…").into_any_element()
        } else {
            chrome::button(
                gpui_kit::SharedString::from(format!("restore-{}", archived.path.display())),
                "Restore",
                None,
                false,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.restore(picked.clone(), cx)))
            .into_any_element()
        };
        let kept = if archived.snapshot == archived.head { "" } else { " · with uncommitted work" };
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .px(len(space::SPACE_3))
            .py(len(space::SPACE_2))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK_MUTED))
                            .truncate()
                            .child(folder_name(&archived.path)),
                    )
                    .child(
                        div().text_size(px(12.)).text_color(theme::color(color::INK_FAINT)).truncate().child(format!(
                            "{} · archived {}{kept}",
                            archived.branch,
                            ago(now - archived.at)
                        )),
                    ),
            )
            .child(div().flex_none().w(px(190.)).flex().justify_end().text_size(px(12.)).child(action))
    }

    /// Removes worktrees, one after another, then reads the list again.
    fn remove(&mut self, worktrees: Vec<Worktree>, cx: &mut Context<Self>) {
        if worktrees.is_empty() {
            return;
        }
        self.removing.extend(worktrees.iter().map(|worktree| worktree.path.clone()));
        self.said = None;
        cx.notify();
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let said = cx
                .background_spawn(async move {
                    // One line for what went well; a worktree is named only when something didn't.
                    let (mut removed, mut branches, mut problems) = (0, 0, Vec::new());
                    for worktree in &worktrees {
                        // Only a recipe someone wrote has a cleanup; a guess never does.
                        let cleanup = recipe::load(&worktree.path)
                            .and_then(Result::ok)
                            .and_then(|recipe| recipe.cleanup_command());
                        let name = folder_name(&worktree.path);
                        match worktrees::remove(&repo, worktree, cleanup.as_deref()) {
                            Ok(done) => {
                                removed += 1;
                                branches += usize::from(done.branch_deleted);
                                if done.cleanup_failed {
                                    problems.push(format!("{name}'s cleanup failed; it was removed anyway."));
                                }
                            }
                            Err(problem) => problems.push(format!("{name} stays: {problem}")),
                        }
                    }
                    let mut said = removed_summary(removed, branches);
                    for problem in problems {
                        said.push(' ');
                        said.push_str(&problem);
                    }
                    said.trim().to_owned()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.removing.clear();
                this.said = Some(said);
                this.read(cx);
                cx.notify();
            });
            let _ = this.update(cx, |this, cx| {
                if let Some(workspace) = this.workspace.upgrade() {
                    workspace.update(cx, |_, cx| cx.notify());
                }
            });
        })
        .detach();
    }

    fn render_row(&self, row: &Row, now: i64, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let worktree = row.worktree.clone();
        let removing = self.removing.contains(&worktree.path);
        let detail = [
            worktree.branch.clone().or_else(|| Some("detached".to_owned())),
            row.last_commit.map(|time| ago(now - time)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let merged = match (worktree.main, worktree.missing, row.merged) {
            (true, _, _) => None,
            (_, true, _) => Some("Folder is gone"),
            (_, _, Merged::Yes) => Some("Merged"),
            (_, _, Merged::ByContent) => Some("Squash-merged"),
            (_, _, Merged::No) => None,
        };
        let pull_request =
            row.pull_request.as_ref().map(|pr| format!("#{} {}", pr.number, pr.state.label().to_lowercase()));
        // Unfinished but idle: it can be put away, its work kept.
        let archivable =
            !worktree.main && !worktree.missing && worktree.branch.is_some() && !row.in_pane && row.programs.is_empty();
        let action = match (removing, row.blocker()) {
            (true, _) => div().text_color(theme::color(color::INK_MUTED)).child("Working…").into_any_element(),
            (false, Some(why)) if archivable => {
                let picked = worktree.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_3))
                    .child(div().text_color(theme::color(color::INK_FAINT)).truncate().child(why))
                    .child(
                        chrome::button(
                            gpui_kit::SharedString::from(format!("archive-{}", worktree.path.display())),
                            "Archive",
                            None,
                            false,
                        )
                        .tooltip(chrome::tip(
                            "Put it away, keeping its branch and uncommitted work. Restore brings it back.",
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| this.archive(picked.clone(), cx))),
                    )
                    .into_any_element()
            }
            (false, Some(why)) => div().text_color(theme::color(color::INK_FAINT)).child(why).into_any_element(),
            (false, None) => {
                let label = if worktree.missing { "Forget" } else { "Remove" };
                chrome::button(
                    gpui_kit::SharedString::from(format!("remove-{}", worktree.path.display())),
                    label,
                    None,
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.remove(vec![worktree.clone()], cx)))
                .into_any_element()
            }
        };
        div()
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .px(len(space::SPACE_3))
            .py(len(space::SPACE_2))
            .rounded(theme::corner(radius::RADIUS_SM))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK))
                            .truncate()
                            .child(folder_name(&row.worktree.path)),
                    )
                    .child(
                        div().text_size(px(12.)).text_color(theme::color(color::INK_FAINT)).truncate().child(detail),
                    ),
            )
            .children(merged.map(|merged| {
                div().flex_none().text_size(px(12.)).text_color(theme::color(color::INK_MUTED)).child(merged)
            }))
            .children(pull_request.map(|pr| {
                div()
                    .flex_none()
                    .font_family(saturn_design::font::MONO)
                    .text_size(px(12.))
                    .text_color(theme::color(color::INK_MUTED))
                    .child(pr)
            }))
            .child(div().flex_none().w(px(300.)).flex().justify_end().text_size(px(12.)).child(action))
    }
}

impl Render for WorktreesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = now();
        let Some(rows) = self.rows.clone() else {
            return div()
                .py(len(space::SPACE_4))
                .text_size(px(14.))
                .text_color(theme::color(color::INK_MUTED))
                .child("Reading the worktrees…")
                .into_any_element();
        };
        let finished: Vec<Worktree> = rows
            .iter()
            .filter(|row| row.blocker().is_none() && !self.removing.contains(&row.worktree.path))
            .map(|row| row.worktree.clone())
            .collect();
        let summary = match (rows.len().saturating_sub(1), finished.len()) {
            (0, _) => "No worktrees besides the main checkout.".to_owned(),
            (all, 0) => format!("{all} worktrees, none finished."),
            (all, done) => format!("{all} worktrees; {done} finished, with nothing in the way."),
        };
        let mut list = div().id("worktrees").flex().flex_col().max_h(px(480.)).overflow_y_scroll();
        for row in &rows {
            list = list.child(self.render_row(row, now, cx));
        }
        if !self.archived.is_empty() {
            list = list.child(
                div()
                    .pt(len(space::SPACE_4))
                    .pb(len(space::SPACE_1))
                    .px(len(space::SPACE_3))
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK_FAINT))
                    .child("ARCHIVED"),
            );
            for archived in self.archived.clone() {
                list = list.child(self.render_archived(&archived, now, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_3))
                    .child(div().flex_1().text_size(px(13.)).text_color(theme::color(color::INK_MUTED)).child(summary))
                    .children((!finished.is_empty()).then(|| {
                        chrome::button("remove-finished", "Remove the finished ones", None, true)
                            .on_click(cx.listener(move |this, _, _, cx| this.remove(finished.clone(), cx)))
                    })),
            )
            .children(
                self.said
                    .clone()
                    .map(|said| div().text_size(px(13.)).text_color(theme::color(color::INK_MUTED)).child(said)),
            )
            .child(list)
            .into_any_element()
    }
}

/// Everything about each worktree of `repo`.
fn read_rows(repo: &Path, pane_folders: &[PathBuf]) -> Vec<Row> {
    let list = worktrees::list(repo);
    let folders: Vec<PathBuf> = list
        .iter()
        .filter(|worktree| !worktree.main && !worktree.missing)
        .map(|worktree| worktree.path.clone())
        .collect();
    // Everything at once: `lsof`, `gh` and each worktree's git commands, which one after another take
    // seconds in a repo with twenty worktrees.
    let base = worktrees::default_branch(repo);
    let (programs, pull_requests, details) = std::thread::scope(|scope| {
        let programs = scope.spawn(|| worktrees::programs_in(&folders));
        let pull_requests = scope.spawn(|| crate::github::pull_requests_by_branch(repo));
        let mut details = Vec::with_capacity(list.len());
        for chunk in list.chunks(8) {
            let reading: Vec<_> =
                chunk.iter().map(|worktree| scope.spawn(|| details_of(repo, worktree, base.as_deref()))).collect();
            details.extend(reading.into_iter().map(|reading| reading.join().unwrap_or((Merged::No, 0, None))));
        }
        (programs.join().unwrap_or_default(), pull_requests.join().unwrap_or_default(), details)
    });
    list.into_iter()
        .zip(details)
        .map(|(worktree, (merged, uncommitted, last_commit))| Row {
            merged,
            uncommitted,
            programs: programs.get(&worktree.path).cloned().unwrap_or_default(),
            in_pane: pane_folders.iter().any(|folder| folder.starts_with(&worktree.path)),
            pull_request: worktree.branch.as_ref().and_then(|branch| pull_requests.get(branch).cloned()),
            last_commit,
            worktree,
        })
        .collect()
}

/// A worktree's own state: whether it's merged into `base`, its uncommitted files, its last commit.
fn details_of(repo: &Path, worktree: &Worktree, base: Option<&str>) -> (Merged, usize, Option<i64>) {
    if worktree.missing {
        return (Merged::No, 0, None);
    }
    let merged = match (&worktree.branch, base) {
        (Some(branch), Some(base)) if !worktree.main => worktrees::merged(repo, branch, base),
        _ => Merged::No,
    };
    (merged, worktrees::uncommitted(&worktree.path), worktrees::last_commit(&worktree.path))
}

/// "Removed 7 worktrees and their merged branches."
fn removed_summary(removed: usize, branches: usize) -> String {
    let worktrees = if removed == 1 { "1 worktree".to_owned() } else { format!("{removed} worktrees") };
    match (removed, branches) {
        (0, _) => String::new(),
        (_, 0) => format!("Removed {worktrees}."),
        (1, _) => format!("Removed {worktrees} and its merged branch."),
        (_, b) if b == removed => format!("Removed {worktrees} and their merged branches."),
        (_, 1) => format!("Removed {worktrees} and 1 merged branch."),
        (_, b) => format!("Removed {worktrees} and {b} merged branches."),
    }
}

/// Archives a worktree of the main checkout `repo` (its recipe's cleanup first) and lists it, so
/// Restore can bring it back. Slow: off the main thread.
pub fn archive_now(repo: &Path, worktree: &Worktree) -> Result<(), String> {
    let cleanup = recipe::load(&worktree.path).and_then(Result::ok).and_then(|recipe| recipe.cleanup_command());
    let archived = worktrees::archive(repo, worktree, cleanup.as_deref(), now())?;
    if let Some(file) = archives_file() {
        let mut list = Archives::load(&file);
        list.add(repo, archived);
        let _ = list.save(&file);
    }
    Ok(())
}

/// Where archived worktrees are listed, beside Saturn's settings.
fn archives_file() -> Option<PathBuf> {
    crate::settings::data_dir().map(|dir| dir.join("archives.json"))
}

/// Seconds since 1970.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
}

fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// How long ago, in words: "today", "3 days ago", "2 months ago".
pub fn ago(seconds: i64) -> String {
    let days = seconds / 86_400;
    match days {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        _ => format!("{} months ago", days / 30),
    }
}

/// Opens the list for the main checkout `repo`. `pane_folders` are where Saturn's panes work.
pub fn open(
    workspace: WeakEntity<Workspace>,
    repo: PathBuf,
    pane_folders: Vec<PathBuf>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let handle = window.window_handle();
    let view = cx.new(|cx| WorktreesView::new(workspace, repo, pane_folders, handle, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title("Worktrees")
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(760.))
            .child(view.clone())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_up_what_was_removed() {
        assert_eq!(super::removed_summary(7, 7), "Removed 7 worktrees and their merged branches.");
        assert_eq!(super::removed_summary(1, 1), "Removed 1 worktree and its merged branch.");
        assert_eq!(super::removed_summary(3, 1), "Removed 3 worktrees and 1 merged branch.");
        assert_eq!(super::removed_summary(2, 0), "Removed 2 worktrees.");
        assert_eq!(super::removed_summary(0, 0), "");
    }

    #[test]
    fn says_how_long_ago() {
        assert_eq!(ago(3_600), "today");
        assert_eq!(ago(86_400 * 3), "3 days ago");
        assert_eq!(ago(86_400 * 21), "3 weeks ago");
        assert_eq!(ago(86_400 * 95), "3 months ago");
    }
}
