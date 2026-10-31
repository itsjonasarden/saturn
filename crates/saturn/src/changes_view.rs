//! The Changes tab: what isn't committed yet, as one `git diff` parsed into files, grouped into
//! tracked and untracked files with what's staged for the next commit. The same files feed the
//! list here and the review in the editor panel, so the two never disagree.
//!
//! At the bottom, the branch's whole diff since it left the default branch (what a pull request
//! would show) opens as a review of its own.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gpui_kit::component::diff::{DiffFile, DiffFileStatus};
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, ElementId, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use saturn_core::git::{self, Change, Scope, Staged};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::{self, len};

const ROW_HEIGHT: f32 = 26.0;

pub enum ChangesEvent {
    /// Review the changes, starting at `path`.
    Open { files: Vec<DiffFile>, scope: Scope, path: String },
    /// The changes were read again; an open review should show the new ones.
    Updated { files: Vec<DiffFile>, scope: Scope },
    /// Show the repository's history.
    History { repo: PathBuf },
    /// Show what the changes reach.
    Impact { repo: PathBuf },
    /// Pick a branch, tag or commit to compare with.
    Compare { repo: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Group {
    Tracked,
    Untracked,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Self::Tracked => "Tracked",
            Self::Untracked => "Untracked",
        }
    }
}

/// A line of the list: a group's header, or a file (an index into `files`).
#[derive(Clone, Copy)]
enum Item {
    Header(Group),
    File(usize),
}

pub struct ChangesView {
    repo: PathBuf,
    files: Vec<DiffFile>,
    /// What git says about each changed file, by path from the repository's top.
    changes: HashMap<String, Change>,
    branch: Option<String>,
    collapsed: HashSet<Group>,
    items: Vec<Item>,
    selected: Option<String>,
    loaded: bool,
    /// Whether the branch's review is open, so it's read again along with the list.
    reviewing_branch: bool,
    /// The last staging that failed, until the next one works.
    problem: Option<String>,
}

impl EventEmitter<ChangesEvent> for ChangesView {}

impl ChangesView {
    pub fn repo(&self) -> &Path {
        &self.repo
    }

    pub fn new(repo: &Path, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            repo: repo.to_owned(),
            files: Vec::new(),
            changes: HashMap::new(),
            branch: None,
            collapsed: HashSet::new(),
            items: Vec::new(),
            selected: None,
            loaded: false,
            reviewing_branch: false,
            problem: None,
        };
        view.reload(cx);
        view
    }

    /// Reads the changes again, off the main thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let branch_too = self.reviewing_branch;
        cx.spawn(async move |this, cx| {
            let (files, changes, branch, branch_files) = cx
                .background_spawn(async move {
                    let files = DiffFile::parse(&git::patch(&repo, Scope::Uncommitted)).unwrap_or_default();
                    let changes: HashMap<String, Change> =
                        git::changes(&repo).into_iter().map(|change| (change.path.clone(), change)).collect();
                    let branch_files =
                        branch_too.then(|| DiffFile::parse(&git::patch(&repo, Scope::Branch)).unwrap_or_default());
                    (files, changes, git::branch(&repo), branch_files)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.files = files;
                this.changes = changes;
                this.branch = branch;
                this.loaded = true;
                this.rebuild_items();
                cx.emit(ChangesEvent::Updated { files: this.files.clone(), scope: Scope::Uncommitted });
                if let Some(files) = branch_files {
                    cx.emit(ChangesEvent::Updated { files, scope: Scope::Branch });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn group_of(&self, file: &DiffFile) -> Group {
        let untracked = self.changes.get(file.path().as_ref() as &str).is_some_and(|change| change.untracked);
        if untracked { Group::Untracked } else { Group::Tracked }
    }

    fn files_in(&self, group: Group) -> Vec<usize> {
        (0..self.files.len()).filter(|index| self.group_of(&self.files[*index]) == group).collect()
    }

    fn rebuild_items(&mut self) {
        let mut items = Vec::new();
        for group in [Group::Tracked, Group::Untracked] {
            let files = self.files_in(group);
            if files.is_empty() {
                continue;
            }
            items.push(Item::Header(group));
            if !self.collapsed.contains(&group) {
                items.extend(files.into_iter().map(Item::File));
            }
        }
        self.items = items;
    }

    fn staged(&self, file: &DiffFile) -> Staged {
        self.changes.get(file.path().as_ref() as &str).map_or(Staged::No, |change| change.staged)
    }

    /// How much of `group` is staged: all of it, none, or some.
    fn group_staged(&self, group: Group) -> Staged {
        let states: Vec<Staged> = self.files_in(group).iter().map(|index| self.staged(&self.files[*index])).collect();
        if states.iter().all(|staged| *staged == Staged::Yes) {
            Staged::Yes
        } else if states.iter().all(|staged| *staged == Staged::No) {
            Staged::No
        } else {
            Staged::Partly
        }
    }

    /// Stages `paths`, or unstages them when they're all staged already.
    fn toggle_staged(&mut self, paths: Vec<String>, all_staged: bool, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    if all_staged { git::unstage(&repo, &paths) } else { git::stage(&repo, &paths) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.problem = outcome.err();
                this.reload(cx);
            });
        })
        .detach();
    }

    pub fn open(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(file) = self.files.get(index) else { return };
        let path = file.path().to_string();
        self.selected = Some(path.clone());
        cx.emit(ChangesEvent::Open { files: self.files.clone(), scope: Scope::Uncommitted, path });
        cx.notify();
    }

    /// Opens the branch's whole diff as a review.
    fn review_branch(&mut self, cx: &mut Context<Self>) {
        self.reviewing_branch = true;
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async move { DiffFile::parse(&git::patch(&repo, Scope::Branch)).unwrap_or_default() })
                .await;
            let _ = this.update(cx, |_, cx| {
                let path = files.first().map(|file| file.path().to_string()).unwrap_or_default();
                cx.emit(ChangesEvent::Open { files, scope: Scope::Branch, path });
            });
        })
        .detach();
    }

    fn render_item(&self, item: Item, cx: &mut Context<Self>) -> AnyElement {
        match item {
            Item::Header(group) => self.render_header(group, cx),
            Item::File(index) => self.render_row(index, cx),
        }
    }

    fn render_header(&self, group: Group, cx: &mut Context<Self>) -> AnyElement {
        let files = self.files_in(group);
        let paths: Vec<String> = files.iter().map(|index| self.files[*index].path().to_string()).collect();
        let staged = self.group_staged(group);
        let collapsed = self.collapsed.contains(&group);
        div()
            .id(("group", group as usize))
            .w_full()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(px(ROW_HEIGHT))
            .pl(len(space::SPACE_2))
            .pr(len(space::SPACE_3))
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::color(color::INK_MUTED))
            .child(chrome::icon(
                if collapsed { "chevron-right" } else { "chevron-down" },
                14.,
                theme::color(color::INK_FAINT),
            ))
            .child(div().flex_1().child(group.label()))
            .child(
                div()
                    .px(len(space::SPACE_2))
                    .rounded(theme::corner(radius::RADIUS_XS))
                    .bg(theme::fill_hover())
                    .font_family(saturn_design::font::MONO)
                    .text_size(px(11.))
                    .child(files.len().to_string()),
            )
            .child(checkbox(("stage-group", group as usize), staged).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.toggle_staged(paths.clone(), staged == Staged::Yes, cx);
                },
            )))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.collapsed.remove(&group) {
                    this.collapsed.insert(group);
                }
                this.rebuild_items();
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let file = &self.files[index];
        let path = file.path().to_string();
        let (folder, name) = match path.rsplit_once('/') {
            Some((folder, name)) => (folder.to_owned(), name.to_owned()),
            None => (String::new(), path.clone()),
        };
        let selected = self.selected.as_deref() == Some(path.as_str());
        let (icon, icon_color) = match file.status() {
            DiffFileStatus::Added => ("square-plus", color::TERM_GREEN),
            DiffFileStatus::Deleted => ("square-minus", color::RED),
            DiffFileStatus::Renamed | DiffFileStatus::Copied => ("square-arrow-right", color::AMBER_TEXT),
            DiffFileStatus::Conflicted => ("square-x", color::RED),
            _ => ("square-dot", color::AMBER_TEXT),
        };
        let staged = self.staged(file);
        div()
            .id(("change", index))
            .w_full()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(px(ROW_HEIGHT))
            .pl(len(space::SPACE_2))
            .pr(len(space::SPACE_3))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .text_size(px(13.))
            .when(selected, |row| row.bg(theme::fill_pressed()))
            .when(!selected, |row| row.hover(|s| s.bg(theme::fill_hover())))
            .child(chrome::icon(icon, 14., theme::color(icon_color)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .gap(len(space::SPACE_2))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme::color(if selected { color::INK } else { color::INK_BODY }))
                            .child(name),
                    )
                    .child(div().min_w_0().truncate().text_color(theme::color(color::INK_FAINT)).child(folder)),
            )
            .child(stats(file.additions(), file.deletions()))
            .child(checkbox(("stage", index), staged).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_staged(vec![path.clone()], staged == Staged::Yes, cx);
            })))
            .on_click(cx.listener(move |this, _, _, cx| this.open(index, cx)))
            .into_any_element()
    }

    /// The branch, and its whole diff as a review.
    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(len(space::SPACE_2))
            .p(len(space::SPACE_1))
            .pl(len(space::SPACE_3))
            .border_t_1()
            .border_color(theme::separator())
            .child(chrome::icon("git-branch", 14., theme::color(color::INK_FAINT)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(theme::color(color::INK_MUTED))
                    .child(self.branch.clone().unwrap_or_else(|| "No branch".to_owned())),
            )
            .child(
                chrome::icon_button("impact", "waypoints", true)
                    .tooltip(chrome::tip_for("Impact of the changes", crate::workspace::ShowImpact))
                    .on_click(cx.listener(|this, _, _, cx| cx.emit(ChangesEvent::Impact { repo: this.repo.clone() }))),
            )
            .child(
                chrome::icon_button("history", "git-commit-horizontal", true)
                    .tooltip(chrome::tip_for("History", crate::workspace::ShowHistory))
                    .on_click(cx.listener(|this, _, _, cx| cx.emit(ChangesEvent::History { repo: this.repo.clone() }))),
            )
            .child(
                chrome::icon_button("branch-diff", "git-pull-request", true)
                    .tooltip(chrome::tip("Review the whole branch's diff"))
                    .on_click(cx.listener(|this, _, _, cx| this.review_branch(cx))),
            )
            .child(
                chrome::icon_button("compare", "git-compare", true)
                    .tooltip(chrome::tip("Compare with a branch, tag or commit"))
                    .on_click(cx.listener(|this, _, _, cx| cx.emit(ChangesEvent::Compare { repo: this.repo.clone() }))),
            )
    }
}

/// A stage checkbox in the macOS manner: 14px, a quiet relative outline when off; when on, the
/// accent with a soft highlight along its top edge and a thin white tick (a dash when partly).
pub fn checkbox(id: impl Into<ElementId>, staged: Staged) -> gpui_kit::Stateful<gpui_kit::Div> {
    let on = staged != Staged::No;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.))
        .rounded(theme::corner(4.))
        .cursor_pointer()
        .when(on, |box_| {
            box_.bg(theme::color(color::BLUE_VIVID)).shadow(vec![gpui_kit::BoxShadow {
                color: gpui_kit::hsla(0., 0., 1., 0.22),
                offset: gpui_kit::point(px(0.), px(1.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
                inset: true,
            }])
        })
        .when(!on, |box_| {
            box_.bg(theme::overlay(0.04))
                .border_1()
                .border_color(theme::overlay(0.24))
                .hover(|s| s.border_color(theme::overlay(0.4)))
        })
        .children(match staged {
            Staged::Yes => Some(chrome::icon("check", 10., gpui_kit::white())),
            Staged::Partly => Some(chrome::icon("minus", 10., gpui_kit::white())),
            Staged::No => None,
        })
}

/// `+12 −3`, in the diff's own colours.
pub fn stats(added: usize, removed: usize) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .gap(len(space::SPACE_1))
        .font_family(saturn_design::font::MONO)
        .text_size(px(11.))
        .when(added > 0, |stats| {
            stats.child(div().text_color(theme::color(color::TERM_GREEN)).child(format!("+{added}")))
        })
        .when(removed > 0, |stats| stats.child(div().text_color(theme::color(color::RED)).child(format!("−{removed}"))))
}

impl Render for ChangesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = if self.loaded && self.files.is_empty() {
            div()
                .flex_1()
                .p(len(space::SPACE_4))
                .text_size(px(14.))
                .text_color(theme::color(color::INK_MUTED))
                .child("Nothing uncommitted.")
                .into_any_element()
        } else {
            uniform_list(
                "changes",
                self.items.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range.map(|index| this.render_item(this.items[index], cx)).collect::<Vec<_>>()
                }),
            )
            // Edge to edge in the card, which pads it already: its rows line up with the tabs.
            .flex_1()
            .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(list)
            .children(self.problem.clone().map(|problem| {
                div()
                    .flex_none()
                    .px(len(space::SPACE_3))
                    .py(len(space::SPACE_2))
                    .text_size(px(12.))
                    .text_color(theme::color(color::RED))
                    .child(problem)
            }))
            .child(self.render_footer(cx))
    }
}
