//! Comments on a review, for the agent: the + beside a line (or a selection of lines) opens a box
//! under it, comments gather under their lines as in a pull request review, and "Send to the agent"
//! hands them all over in one message, each with its file, lines and code (`saturn_core::review`).
//!
//! Clicking a line picks the change (the hunk) it's in, to stage on its own or revert, leaving the
//! file's other changes as they are; a revert can be undone until the next one.
//!
//! Each file's header says what was asked that changed it (`saturn_core::prompts`), so the intent
//! sits beside the diff.

use std::path::PathBuf;

use gpui_kit::component::diff::{DiffAnnotation, DiffLinePosition, DiffLineRange, DiffSide};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use saturn_core::git::{self, Scope};
use saturn_core::hunks::{self, Hunk};
use saturn_core::prompts::{self, Asked};
use saturn_core::review::{self, Comment};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::text;

/// The comments are ready for the agent working in `repo`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Stage,
    Revert,
    Undo,
}

/// Files in `repo` changed under the review (a change staged, reverted or put back), so the list of
/// changes should be read again.
pub struct Touched {
    pub repo: PathBuf,
}

pub struct SendToAgent {
    pub repo: PathBuf,
    pub message: String,
}

struct Written {
    id: u64,
    range: DiffLineRange,
    text: String,
}

/// The comment being written, under the line it's about.
struct Writing {
    range: DiffLineRange,
    input: Entity<InputState>,
    _events: Subscription,
}

pub struct ReviewComments {
    /// Where the reviewed files are.
    repo: PathBuf,
    /// Which changes are reviewed: staging only makes sense for uncommitted ones.
    scope: Scope,
    /// The change the last clicked line is in.
    picked: Option<Hunk>,
    /// The change last reverted, to put back.
    reverted: Option<Hunk>,
    /// What the last stage or revert said.
    said: Option<String>,
    /// The prompts that changed each file, relative to `repo`, oldest first.
    asked: std::collections::HashMap<String, Vec<Asked>>,
    /// When they were last read, so a busy agent doesn't have its transcripts read every second.
    asked_at: Option<std::time::Instant>,
    written: Vec<Written>,
    writing: Option<Writing>,
    next: u64,
}

impl EventEmitter<SendToAgent> for ReviewComments {}
impl EventEmitter<Touched> for ReviewComments {}

impl ReviewComments {
    pub fn new(repo: PathBuf, scope: Scope) -> Self {
        Self {
            repo,
            scope,
            picked: None,
            reverted: None,
            said: None,
            asked: std::collections::HashMap::new(),
            asked_at: None,
            written: Vec::new(),
            writing: None,
            next: 0,
        }
    }

    /// Reads what was asked that changed each file, at most every 20 seconds.
    pub fn read_prompts(&mut self, cx: &mut Context<Self>) {
        if self.asked_at.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(20)) {
            return;
        }
        self.asked_at = Some(std::time::Instant::now());
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let asked = cx.background_spawn(async move { prompts::for_checkout(&repo) }).await;
            let _ = this.update(cx, |this, cx| {
                this.asked = asked;
                cx.notify();
            });
        })
        .detach();
    }

    /// For a file's header: the last prompt that changed it, all of them on hover.
    pub fn render_asked(&self, path: &str) -> Option<AnyElement> {
        let asked = self.asked.get(path).filter(|asked| !asked.is_empty())?;
        let short = |text: &str, most: usize| {
            let line = text.lines().next().unwrap_or_default();
            if line.chars().count() > most || text.lines().nth(1).is_some() {
                format!("{}…", line.chars().take(most).collect::<String>().trim_end())
            } else {
                line.to_owned()
            }
        };
        let latest = asked.last().map(|asked| short(&asked.prompt, 70)).unwrap_or_default();
        let all =
            asked.iter().rev().take(5).map(|asked| format!("“{}”", short(&asked.prompt, 240))).collect::<Vec<_>>();
        let tip = format!("What was asked that changed this file, newest first:\n\n{}", all.join("\n\n"));
        Some(
            div()
                .id(gpui_kit::SharedString::from(format!("asked-{path}")))
                .flex_none()
                .max_w(px(420.))
                .truncate()
                .text_scale(text::META)
                .text_color(theme::color(color::INK_MUTED))
                .child(format!("Asked: “{latest}”"))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                .into_any_element(),
        )
    }

    /// Whether this holds the comments for `scope`'s changes in `repo`.
    pub fn is_for(&self, repo: &std::path::Path, scope: &Scope) -> bool {
        self.repo == repo && &self.scope == scope
    }

    /// Picks the change that `position`'s line is in, reading the diff again so it's current.
    pub fn pick(&mut self, position: &DiffLinePosition, cx: &mut Context<Self>) {
        let (repo, scope) = (self.repo.clone(), self.scope.clone());
        let (path, line, removed) =
            (position.path().to_string(), position.line(), position.side() == DiffSide::Original);
        cx.spawn(async move |this, cx| {
            let hunk =
                cx.background_spawn(async move { hunks::at(&git::patch(&repo, scope), &path, line, removed) }).await;
            let _ = this.update(cx, |this, cx| {
                this.picked = hunk;
                this.said = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Presses Stage, Revert or Undo by name: for spikes (`SATURN_REVIEW_ACT`).
    pub fn press(&mut self, name: &str, cx: &mut Context<Self>) {
        match name {
            "stage" => self.act(Action::Stage, cx),
            "revert" => self.act(Action::Revert, cx),
            "undo" => self.act(Action::Undo, cx),
            _ => {}
        }
    }

    /// Stages, reverts or puts back a change, off the main thread. The review follows the files.
    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        let hunk = match action {
            Action::Undo => self.reverted.take(),
            Action::Stage | Action::Revert => self.picked.take(),
        };
        let Some(hunk) = hunk else { return };
        let repo = self.repo.clone();
        let applied = hunk.clone();
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    match action {
                        Action::Stage => hunks::stage(&repo, &applied),
                        Action::Revert => hunks::revert(&repo, &applied),
                        Action::Undo => hunks::reapply(&repo, &applied),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.said = Some(match (&done, action) {
                    (Ok(()), Action::Stage) => format!("Staged the change in {}.", hunk.path),
                    (Ok(()), Action::Revert) => format!("Reverted the change in {}.", hunk.path),
                    (Ok(()), Action::Undo) => format!("Put the change in {} back.", hunk.path),
                    (Err(problem), _) => format!("Couldn't: {problem}"),
                });
                if done.is_ok() && action == Action::Revert {
                    this.reverted = Some(hunk);
                }
                if done.is_ok() {
                    cx.emit(Touched { repo: this.repo.clone() });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a box for a comment on `range`.
    pub fn start(&mut self, range: DiffLineRange, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("What should change here?"));
        input.update(cx, |input, cx| input.focus(window, cx));
        let events = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.add(cx);
            }
        });
        self.writing = Some(Writing { range, input, _events: events });
        cx.notify();
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let Some(writing) = self.writing.take() else { return };
        let text = writing.input.read(cx).value().trim().to_owned();
        if !text.is_empty() {
            self.next += 1;
            self.written.push(Written { id: self.next, range: writing.range, text });
        }
        cx.notify();
    }

    /// Adds a comment without the box: for spikes (`SATURN_REVIEW_COMMENT`).
    pub fn comment(&mut self, range: DiffLineRange, text: &str, cx: &mut Context<Self>) {
        self.next += 1;
        self.written.push(Written { id: self.next, range, text: text.to_owned() });
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.writing = None;
        cx.notify();
    }

    fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.written.retain(|written| written.id != id);
        cx.notify();
    }

    pub fn discard(&mut self, cx: &mut Context<Self>) {
        self.written.clear();
        self.writing = None;
        cx.notify();
    }

    /// Hands every comment to the agent, with the code each is about, and clears them.
    pub fn send(&mut self, cx: &mut Context<Self>) {
        // A comment still being written goes too.
        self.add(cx);
        if self.written.is_empty() {
            return;
        }
        let comments: Vec<Comment> = self
            .written
            .drain(..)
            .map(|written| {
                let path = written.range.path().to_string();
                let lines =
                    (written.range.start().min(written.range.end()), written.range.start().max(written.range.end()));
                let removed = written.range.side() == DiffSide::Original;
                // What's there now is on disk; removed lines are only in the diff, so they go unquoted.
                let code = if removed {
                    Vec::new()
                } else {
                    std::fs::read_to_string(self.repo.join(&path))
                        .map(|text| review::lines_of(&text, lines))
                        .unwrap_or_default()
                };
                Comment { path, lines, removed, code, text: written.text }
            })
            .collect();
        cx.emit(SendToAgent { repo: self.repo.clone(), message: review::message(&comments) });
        cx.notify();
    }

    /// Where each comment shows: under the last line it's about.
    pub fn annotations(&self) -> Vec<DiffAnnotation> {
        let under = |range: &DiffLineRange| DiffLinePosition::new(range.path().clone(), range.end_side(), range.end());
        let mut annotations: Vec<DiffAnnotation> = self
            .written
            .iter()
            .map(|written| DiffAnnotation::line(("review-comment", written.id), under(&written.range)))
            .collect();
        if let Some(writing) = &self.writing {
            annotations.push(DiffAnnotation::line("review-writing", under(&writing.range)));
        }
        annotations
    }

    /// A comment, or the box for the one being written.
    pub fn render_annotation(&self, annotation: &DiffAnnotation, cx: &mut Context<Self>) -> AnyElement {
        let card = || {
            div()
                .flex()
                .items_start()
                .gap(len(space::SPACE_2))
                .my(len(space::SPACE_1))
                .mx(len(space::SPACE_3))
                .p(len(space::SPACE_2))
                .rounded(theme::corner(radius::RADIUS_SM))
                .bg(theme::color(color::SURFACE))
                .border_1()
                .border_color(theme::separator())
                .text_scale(text::LABEL)
        };
        if annotation.id() == &gpui_kit::ElementId::from("review-writing") {
            let Some(writing) = &self.writing else { return div().into_any_element() };
            return card()
                .items_center()
                .child(div().flex_1().min_w_0().child(crate::chrome::input(&writing.input)))
                .child(
                    chrome::button("review-add", "Add comment", None, true)
                        .on_click(cx.listener(|this, _, _, cx| this.add(cx))),
                )
                .child(
                    chrome::button("review-cancel", "Cancel", None, false)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
                .into_any_element();
        }
        let Some(written) = self
            .written
            .iter()
            .find(|written| annotation.id() == &gpui_kit::ElementId::from(("review-comment", written.id)))
        else {
            return div().into_any_element();
        };
        let id = written.id;
        card()
            .child(div().flex_1().min_w_0().text_color(theme::color(color::INK)).child(written.text.clone()))
            .child(
                chrome::icon_button(("review-remove", id), "x", true)
                    .tooltip(chrome::tip("Remove this comment"))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(id, cx))),
            )
            .into_any_element()
    }

    /// Above the review: the picked change and what to do with it, and the comments waiting for the
    /// agent. Nothing when there's neither.
    pub fn render_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let strip = |id: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .gap(len(space::SPACE_2))
                .px(len(space::SPACE_3))
                .py(len(space::SPACE_1))
                .border_b_1()
                .border_color(theme::separator())
                .text_scale(text::LABEL)
        };
        let mut rows = Vec::new();
        if let Some(hunk) = &self.picked {
            let (start, end) = hunk.changed_lines();
            let lines = if start == end { format!("line {start}") } else { format!("lines {start}–{end}") };
            rows.push(
                strip("review-change-bar")
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::color(color::INK_MUTED))
                            .child(format!("The change at {}, {lines}", hunk.path)),
                    )
                    .children((self.scope == Scope::Uncommitted).then(|| {
                        chrome::button("review-stage", "Stage this change", None, false)
                            .on_click(cx.listener(|this, _, _, cx| this.act(Action::Stage, cx)))
                    }))
                    .child(
                        chrome::button("review-revert", "Revert this change", None, false)
                            .on_click(cx.listener(|this, _, _, cx| this.act(Action::Revert, cx))),
                    )
                    .child(chrome::icon_button("review-unpick", "x", true).tooltip(chrome::tip("Leave it")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.picked = None;
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
            );
        } else if let Some(said) = &self.said {
            rows.push(
                strip("review-said")
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::color(color::INK_MUTED))
                            .child(said.clone()),
                    )
                    .children(self.reverted.is_some().then(|| {
                        chrome::button("review-undo", "Undo", None, false)
                            .on_click(cx.listener(|this, _, _, cx| this.act(Action::Undo, cx)))
                    }))
                    .into_any_element(),
            );
        }
        let count = self.written.len();
        if count > 0 {
            let what = if count == 1 {
                "1 comment for the agent".to_owned()
            } else {
                format!("{count} comments for the agent")
            };
            rows.push(
                strip("review-comments-bar")
                    .child(div().flex_1().text_color(theme::color(color::INK_MUTED)).child(what))
                    .child(
                        chrome::button("review-discard", "Discard", None, false)
                            .on_click(cx.listener(|this, _, _, cx| this.discard(cx))),
                    )
                    .child(
                        chrome::button("review-send", "Send to the agent", None, true)
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    )
                    .into_any_element(),
            );
        }
        (!rows.is_empty()).then(|| div().flex().flex_col().flex_none().children(rows).into_any_element())
    }
}
