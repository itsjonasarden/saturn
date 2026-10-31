//! The History view, in the editor panel: every branch's commits as a graph, newest first, with
//! their branches and tags, message, date, author and hash. Clicking a commit reviews its diff.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, EventEmitter, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathBuilder, Pixels, Render, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, fill, point, prelude::FluentBuilder as _, px, size, uniform_list,
};
use saturn_core::git_graph::{self, Commit, Row, Span};
use saturn_design::{color, radius, space};

use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::text;

const ROW_HEIGHT: f32 = saturn_design::size::ROW;
/// The distance between lanes.
const LANE: f32 = 14.;
/// How many commits the graph reads: enough history to orient, quick to draw.
const LIMIT: usize = 500;

pub enum HistoryEvent {
    /// Review what `hash` changed.
    Open { hash: String },
}

pub struct HistoryView {
    repo: PathBuf,
    commits: Vec<Commit>,
    rows: Vec<Row>,
    /// Lanes the graph column makes room for: the widest row's, within reason.
    lanes: usize,
    selected: Option<String>,
    loaded: bool,
}

impl EventEmitter<HistoryEvent> for HistoryView {}

impl HistoryView {
    pub fn new(repo: &Path, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            repo: repo.to_owned(),
            commits: Vec::new(),
            rows: Vec::new(),
            lanes: 1,
            selected: None,
            loaded: false,
        };
        if std::env::var_os("SATURN_HISTORY").is_some() {
            // Screenshots: the test window stops drawing before a background read would land.
            let commits = git_graph::log(repo, LIMIT);
            let rows = git_graph::layout(&commits);
            view.lanes = rows.iter().map(|row| row.width).max().unwrap_or(1).clamp(1, 12);
            (view.commits, view.rows, view.loaded) = (commits, rows, true);
        } else {
            view.reload(cx);
        }
        view
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    /// Reads the history again, off the main thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let (commits, rows) = cx
                .background_spawn(async move {
                    let commits = git_graph::log(&repo, LIMIT);
                    let rows = git_graph::layout(&commits);
                    (commits, rows)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.lanes = rows.iter().map(|row| row.width).max().unwrap_or(1).clamp(1, 12);
                this.commits = commits;
                this.rows = rows;
                this.loaded = true;
                cx.notify();
            });
        })
        .detach();
    }

    fn render_row(&self, index: usize, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let commit = &self.commits[index];
        let row = self.rows[index].clone();
        let selected = self.selected.as_deref() == Some(commit.hash.as_str());
        let hash = commit.hash.clone();
        let graph_width = lanes_width(self.lanes) + LANE;
        div()
            .id(("commit", index))
            .w_full()
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .h(px(ROW_HEIGHT))
            .pr(len(space::SPACE_3))
            .cursor_pointer()
            .text_scale(text::LABEL)
            .when(selected, |line| line.bg(theme::fill_pressed()))
            .when(!selected, |line| line.hover(|s| s.bg(theme::fill_hover())))
            .child(graph(row, graph_width))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .children(commit.refs.iter().map(|name| ref_chip(name)))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme::color(if selected { color::INK } else { color::INK_BODY }))
                            .child(commit.subject.clone()),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(96.))
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(git_graph::when(commit.time, now)),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(130.))
                    .truncate()
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_MUTED))
                    .child(commit.author.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(60.))
                    .font_family(saturn_design::font::MONO)
                    .text_scale(text::MICRO)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(commit.short_hash().to_owned()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(hash.clone());
                cx.emit(HistoryEvent::Open { hash: hash.clone() });
                cx.notify();
            }))
            .into_any_element()
    }
}

/// A branch or tag on a commit: a small mono chip, the checked-out branch in blue.
fn ref_chip(name: &str) -> impl IntoElement {
    let head = name.starts_with("HEAD");
    let label = name.trim_start_matches("HEAD -> ").trim_start_matches("tag: ").to_owned();
    div()
        .flex_none()
        .px(len(space::SPACE_2))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(theme::corner(radius::RADIUS_XS))
        .border_1()
        .border_color(if head { theme::color(color::BLUE).opacity(0.5) } else { theme::separator() })
        .bg(if head { theme::color(color::BLUE_VIVID).opacity(0.14) } else { theme::fill_hover() })
        .font_family(saturn_design::font::MONO)
        .text_scale(text::MICRO)
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::color(if head { color::BLUE } else { color::INK_MUTED }))
        .max_w(px(220.))
        .truncate()
        .child(label)
}

/// The width of `lanes` lanes.
fn lanes_width(lanes: usize) -> f32 {
    LANE * f32::from(u16::try_from(lanes).unwrap_or(u16::MAX))
}

/// A lane's colour: the terminal palette in turn, so a branch keeps its colour down the graph.
fn lane_color(lane: usize) -> Hsla {
    const LANES: [saturn_design::Token; 6] =
        [color::BLUE, color::TERM_MAGENTA, color::TERM_GREEN, color::AMBER_TEXT, color::TERM_CYAN, color::TERM_RED];
    theme::color(LANES[lane % LANES.len()])
}

/// One row of the graph: its lines between lanes, then the commit's dot.
fn graph(row: Row, width: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            let x = |lane: usize| bounds.origin.x + px(lanes_width(lane) + LANE * 0.75);
            let (top, middle, bottom) = (bounds.origin.y, bounds.center().y, bounds.bottom());
            for line in &row.lines {
                let (from, to, start_y, end_y, colour_lane) = match line.span {
                    Span::TopToDot => (line.from, line.to, top, middle, line.from),
                    Span::TopToBottom => (line.from, line.to, top, bottom, line.from),
                    Span::DotToBottom => (line.from, line.to, middle, bottom, line.to),
                };
                draw_line(window, x(from), start_y, x(to), end_y, lane_color(colour_lane));
            }
            let dot = px(4.);
            let centre = point(x(row.lane), middle);
            window.paint_quad(
                fill(
                    Bounds::new(point(centre.x - dot, centre.y - dot), size(dot * 2., dot * 2.)),
                    lane_color(row.lane),
                )
                .corner_radii(dot),
            );
        },
    )
    .flex_none()
    .w(px(width))
    .h_full()
}

/// A line between two lanes: straight down when it stays in its lane, else an S-curve.
fn draw_line(window: &mut Window, x1: Pixels, y1: Pixels, x2: Pixels, y2: Pixels, colour: Hsla) {
    let mut path = PathBuilder::stroke(px(1.5));
    path.move_to(point(x1, y1));
    if x1 == x2 {
        path.line_to(point(x2, y2));
    } else {
        let mid = (y1 + y2) / 2.;
        path.cubic_bezier_to(point(x2, y2), point(x1, mid), point(x2, mid));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, colour);
    }
}

impl Render for HistoryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loaded && self.commits.is_empty() {
            return div()
                .size_full()
                .p(len(space::SPACE_4))
                .text_scale(text::BODY)
                .text_color(theme::color(color::INK_MUTED))
                .child("No commits here yet.")
                .into_any_element();
        }
        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0));
        uniform_list(
            "history",
            self.commits.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range.map(|index| this.render_row(index, now, cx)).collect::<Vec<_>>()
            }),
        )
        .size_full()
        .into_any_element()
    }
}
