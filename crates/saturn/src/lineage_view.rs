//! The dbt lineage graph, in the editor panel: every source, seed, snapshot and model of the
//! repository's dbt project as a box, left to right from raw data to the final models, with a line
//! for every `ref()` and `source()`. Scroll or drag to pan, pinch (or ⌘-scroll) to zoom. Clicking a
//! node lights up everything it's built from and everything built from it; double-clicking a model
//! opens its file. Searching highlights the nodes whose names match, and Enter zooms to the first.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, ElementId, Entity, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, PathBuilder, PinchEvent, Pixels, Point, Render,
    ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, canvas, div,
    point, prelude::FluentBuilder as _, px,
};
use saturn_core::dbt::{self, Kind, Lineage, NODE_HEIGHT, NODE_WIDTH, Placed};
use saturn_core::schema;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::editor_view::EditorView;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::icon;
use saturn_design::text;
use saturn_design::tone;

const MIN_ZOOM: f32 = 0.15;
const MAX_ZOOM: f32 = 2.;
/// Space around the graph when it's fitted to the view.
const MARGIN: f32 = 32.;

pub struct LineageView {
    /// The folder holding `dbt_project.yml`.
    project: PathBuf,
    editor: WeakEntity<EditorView>,
    /// The whole project, as read.
    whole: Lineage,
    /// What's drawn: the whole project when it's small, else the neighbourhood of `center`.
    lineage: Lineage,
    /// The model a big project's graph is drawn around, by name.
    center: Option<String>,
    /// How many steps up and down from `center` are drawn (`None`: all of them).
    depth: Option<usize>,
    placed: Vec<Placed>,
    loaded: bool,
    zoom: f32,
    /// Where the graph's origin sits in the view, in pixels.
    offset: Point<Pixels>,
    /// The view's bounds on screen, from the last frame: events come in window coordinates.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    fit_pending: bool,
    /// The view's size when the graph was last fitted, and whether it's been panned or zoomed since:
    /// until it has, a change of size (the panel opening wider) fits it again.
    fitted_size: Option<gpui_kit::Size<Pixels>>,
    moved: bool,
    dragging_from: Option<Point<Pixels>>,
    focused: Option<usize>,
    search: Option<Entity<InputState>>,
    query: String,
    search_events: Option<Subscription>,
}

/// How a node kind looks: its icon and the word under its name.
fn kind_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Source => "database",
        Kind::Seed => "sprout",
        Kind::Snapshot => "camera",
        Kind::Model => "box",
    }
}

impl LineageView {
    pub fn new(project: &Path, editor: WeakEntity<EditorView>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            project: project.to_owned(),
            editor,
            whole: Lineage::default(),
            lineage: Lineage::default(),
            center: None,
            depth: Some(2),
            placed: Vec::new(),
            loaded: false,
            zoom: 1.,
            offset: point(px(MARGIN), px(MARGIN)),
            bounds: Rc::new(Cell::new(None)),
            fit_pending: true,
            fitted_size: None,
            moved: false,
            dragging_from: None,
            focused: None,
            search: None,
            query: String::new(),
            search_events: None,
        };
        if std::env::var_os("SATURN_LINEAGE").is_some() {
            // Screenshots: the test window stops drawing before a background read would land.
            view.whole = dbt::load(project);
            view.loaded = true;
            let focus = std::env::var("SATURN_LINEAGE_FOCUS").ok().filter(|name| !name.is_empty());
            if view.big() {
                view.center = focus;
                view.show_scope();
            } else {
                view.show_scope();
                view.focused = focus.and_then(|name| view.lineage.nodes.iter().position(|node| node.name == name));
            }
        } else {
            view.reload(cx);
        }
        view
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Reads the project again, off the main thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let project = self.project.clone();
        cx.spawn(async move |this, cx| {
            let whole = cx.background_spawn(async move { dbt::load(&project) }).await;
            let _ = this.update(cx, |this, cx| {
                this.whole = whole;
                this.loaded = true;
                this.show_scope();
                cx.notify();
            });
        })
        .detach();
    }

    /// Too many nodes to draw at once: the graph shows one model's neighbourhood instead.
    fn big(&self) -> bool {
        self.whole.nodes.len() > dbt::WHOLE_GRAPH_LIMIT
    }

    /// Picks what's drawn, and lays it out: the whole project when it's small, else the
    /// neighbourhood of the model in the centre (nothing until one is picked).
    fn show_scope(&mut self) {
        let center = self.center.as_ref().and_then(|name| self.whole.nodes.iter().position(|node| &node.name == name));
        self.lineage = match center {
            Some(center) if self.big() => {
                dbt::subgraph(&self.whole, &dbt::neighbourhood(&self.whole, center, self.depth))
            }
            _ if self.big() => Lineage { name: self.whole.name.clone(), ..Lineage::default() },
            _ => self.whole.clone(),
        };
        self.placed = dbt::layout(&self.lineage);
        // The neighbourhood lists its centre first.
        self.focused = (center.is_some() && self.big()).then_some(0);
        self.fit_pending = true;
    }

    /// Draws the graph around `name` (a big project), or focuses it (a small one).
    fn center_on(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.big() {
            self.center = Some(name.to_owned());
            self.show_scope();
            cx.notify();
        } else if let Some(index) = self.lineage.nodes.iter().position(|node| node.name == name) {
            self.zoom_to(index, cx);
        }
    }

    /// Centres on the node a file defines (a model's SQL open in the editor), if the project has it.
    pub fn focus_file(&mut self, file: &Path, cx: &mut Context<Self>) {
        let relative = file.strip_prefix(&self.project).unwrap_or(file);
        if let Some(node) = self.whole.nodes.iter().find(|node| node.path.as_deref() == Some(relative)) {
            let name = node.name.clone();
            self.center_on(&name, cx);
        }
    }

    fn extent(&self) -> (f32, f32) {
        self.placed.iter().fold((0., 0.), |(w, h): (f32, f32), p| (w.max(p.x + NODE_WIDTH), h.max(p.y + NODE_HEIGHT)))
    }

    /// Zooms so the whole graph fits, at most 100%.
    fn fit(&mut self) {
        let Some(bounds) = self.bounds.get() else { return };
        let (width, height) = self.extent();
        let (zoom, x, y) =
            schema::fit(width, height, f32::from(bounds.size.width), f32::from(bounds.size.height), MARGIN, MIN_ZOOM);
        self.zoom = zoom;
        self.offset = point(px(x), px(y));
        self.fit_pending = false;
        self.fitted_size = Some(bounds.size);
        self.moved = false;
    }

    /// Zooms by `factor` keeping the point under `anchor` (window coordinates) in place.
    fn zoom_at(&mut self, factor: f32, anchor: Point<Pixels>, cx: &mut Context<Self>) {
        let origin = self.bounds.get().map_or(point(px(0.), px(0.)), |bounds| bounds.origin);
        let local = anchor - origin;
        self.moved = true;
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let scale = new_zoom / self.zoom;
        self.offset = point(local.x - (local.x - self.offset.x) * scale, local.y - (local.y - self.offset.y) * scale);
        self.zoom = new_zoom;
        cx.notify();
    }

    fn zoom_centre(&mut self, factor: f32, cx: &mut Context<Self>) {
        let centre = self.bounds.get().map_or(point(px(0.), px(0.)), |bounds| bounds.center());
        self.zoom_at(factor, centre, cx);
    }

    /// Centres `node` (at 100% at most) and focuses it.
    fn zoom_to(&mut self, node: usize, cx: &mut Context<Self>) {
        let (Some(place), Some(bounds)) = (self.placed.get(node).copied(), self.bounds.get()) else {
            self.focused = Some(node);
            return;
        };
        let (view_w, view_h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        self.zoom = self.zoom.clamp(0.8, 1.);
        self.offset = point(
            px(view_w / 2. - (place.x + NODE_WIDTH / 2.) * self.zoom),
            px(view_h / 2. - (place.y + NODE_HEIGHT / 2.) * self.zoom),
        );
        self.focused = Some(node);
        cx.notify();
    }

    /// Nodes whose names contain the search, in order.
    fn matches(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        (0..self.lineage.nodes.len()).filter(|&i| self.lineage.nodes[i].name.to_lowercase().contains(&query)).collect()
    }

    /// The nodes to keep bright: the search's matches while searching, else the focused node with
    /// everything upstream and downstream of it. `None` keeps them all bright.
    fn lit(&self) -> Option<Vec<bool>> {
        if !self.query.trim().is_empty() {
            let mut lit = vec![false; self.lineage.nodes.len()];
            for node in self.matches() {
                lit[node] = true;
            }
            return Some(lit);
        }
        self.focused.map(|node| dbt::related(&self.lineage, node))
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(search) = &self.search {
            return search.clone();
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search models"));
        self.search_events = Some(cx.subscribe(&search, |this, search, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                this.query = search.read(cx).value().to_string();
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                // A big project's search looks through all of it, not just what's drawn.
                let query = this.query.trim().to_lowercase();
                let first = this.whole.nodes.iter().find(|node| node.name.to_lowercase().contains(&query));
                if let Some(name) = first.map(|node| node.name.clone()) {
                    this.center_on(&name, cx);
                }
            }
            _ => {}
        }));
        self.search = Some(search.clone());
        search
    }

    /// Opens the file that defines `node`, when it has one.
    fn open(&self, node: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.lineage.nodes[node].path.as_ref().map(|path| self.project.join(path)) else { return };
        let _ = self.editor.update(cx, |editor, cx| editor.open(&path, window, cx));
    }

    fn render_node(&self, index: usize, dimmed: bool, cx: &mut Context<Self>) -> AnyElement {
        let node = &self.lineage.nodes[index];
        let place = self.placed[index];
        let z = self.zoom;
        let focused = self.focused == Some(index);
        // Sources are where data comes in: the one kind tinted, so the graph's left edge reads.
        let tint = if node.kind == Kind::Source { color::BLUE } else { color::INK_MUTED };
        let detail = if node.folder.is_empty() {
            node.kind.name().to_owned()
        } else {
            format!("{} · {}", node.kind.name(), node.folder)
        };
        let tip = match node.kind {
            Kind::Source => "A source: data dbt reads from the warehouse. Click to light up what's built from it",
            Kind::Model => "A model. Click to light up its lineage, double-click to open it",
            Kind::Seed => "A seed: a CSV file dbt loads. Click to light up its lineage, double-click to open it",
            Kind::Snapshot => "A snapshot of a table's history. Click to light up its lineage, double-click to open it",
        };
        div()
            .id(("lineage-node", index))
            .absolute()
            .left(self.offset.x + px(place.x * z))
            .top(self.offset.y + px(place.y * z))
            .w(px(NODE_WIDTH * z))
            .h(px(NODE_HEIGHT * z))
            .flex()
            .items_center()
            .gap(px(8. * z))
            .px(px(10. * z))
            .rounded(px(saturn_design::corner(radius::RADIUS_SM) * z.max(0.5)))
            .bg(theme::color(color::SURFACE))
            .border_1()
            .border_color(if focused { theme::color(color::BLUE) } else { theme::tone(tone::PRESSED) })
            .when(dimmed, |node| node.opacity(0.3))
            .overflow_hidden()
            .cursor_pointer()
            .child(chrome::icon(kind_icon(node.kind), icon::MD * z, theme::color(tint)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .font_family(saturn_design::font::MONO)
                            .text_size(px(text::META.size * z))
                            .line_height(px(16. * z))
                            .text_color(theme::color(color::INK))
                            .child(node.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(10.5 * z))
                            .line_height(px(14. * z))
                            .text_color(theme::color(color::INK_FAINT))
                            .child(detail),
                    ),
            )
            .tooltip(chrome::tip(tip))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.open(index, window, cx);
                } else if this.big() {
                    // A big project's graph moves to the node clicked: its own neighbourhood.
                    let name = this.lineage.nodes[index].name.clone();
                    this.center_on(&name, cx);
                } else {
                    this.focused = if this.focused == Some(index) { None } else { Some(index) };
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().absolute().top(len(space::SPACE_2)).right(len(space::SPACE_2)).child(
            chrome::floating(chrome::track())
                .when(self.big() && self.center.is_some(), |track| {
                    let depths = [("1", Some(1)), ("2", Some(2)), ("3", Some(3)), ("All", None)];
                    track
                        .children(depths.map(|(label, depth)| {
                            chrome::track_item(
                                ElementId::Name(format!("lineage-depth-{label}").into()),
                                self.depth == depth,
                            )
                            .px(len(space::SPACE_2))
                            .child(label)
                            .tooltip(chrome::tip(match depth {
                                Some(1) => "Only what it reads and what reads it",
                                Some(_) => "That many steps up and down",
                                None => "Everything upstream and downstream",
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.depth = depth;
                                this.show_scope();
                                cx.notify();
                            }))
                        }))
                        .child(chrome::track_separator())
                })
                .child(
                    chrome::icon_button("lineage-zoom-out", "zoom-out", true)
                        .tooltip(chrome::tip("Zoom out (or pinch, or ⌘-scroll)"))
                        .on_click(cx.listener(|this, _, _, cx| this.zoom_centre(1. / 1.25, cx))),
                )
                .child(
                    div()
                        .w(px(44.))
                        .text_center()
                        .text_scale(text::META)
                        .text_color(theme::color(color::INK_MUTED))
                        .child(format!("{}%", (self.zoom * 100.).round())),
                )
                .child(
                    chrome::icon_button("lineage-zoom-in", "zoom-in", true)
                        .tooltip(chrome::tip("Zoom in (or pinch, or ⌘-scroll)"))
                        .on_click(cx.listener(|this, _, _, cx| this.zoom_centre(1.25, cx))),
                )
                .child(chrome::track_separator())
                .child(
                    chrome::icon_button("lineage-fit", "scan", true)
                        .tooltip(chrome::tip("Fit the whole graph"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.fit();
                            cx.notify();
                        })),
                )
                .child(
                    chrome::icon_button("lineage-reload", "refresh-cw", true)
                        .tooltip(chrome::tip("Read the project again"))
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                ),
        )
    }

    /// A big project, and no model picked yet: what to do.
    fn render_prompt(&self) -> impl IntoElement {
        div()
            .absolute()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(len(space::SPACE_2))
            .text_scale(text::BODY)
            .text_color(theme::color(color::INK_MUTED))
            .child(format!(
                "{} has {} models: too many to draw at once.",
                self.whole.name,
                self.whole.nodes.iter().filter(|node| node.kind == Kind::Model).count()
            ))
            .child("Search for one and press Enter, or open its SQL in the editor, to see its lineage.")
    }

    fn render_caption(&self) -> impl IntoElement {
        let count = |kind: Kind| {
            let n = self.whole.nodes.iter().filter(|node| node.kind == kind).count();
            format!("{n} {}{}", kind.name(), if n == 1 { "" } else { "s" })
        };
        let how = match &self.center {
            Some(center) if self.big() => format!(
                "{} of them around {center} · click a node to move there, double-click to open it",
                self.lineage.nodes.len()
            ),
            _ => "click a node for its lineage, double-click to open it".to_owned(),
        };
        div()
            .absolute()
            .left(len(space::SPACE_3))
            .bottom(len(space::SPACE_2))
            .text_scale(text::MICRO)
            .text_color(theme::color(color::INK_FAINT))
            .child(format!(
                "{} · {} · {} · {} · {} · {how}",
                self.whole.name,
                count(Kind::Model),
                count(Kind::Source),
                count(Kind::Seed),
                count(Kind::Snapshot)
            ))
    }
}

/// Each `ref()` and `source()` as a curve from the right edge of what's read to the left edge of
/// what reads it.
fn edges(view: &LineageView, lit: Option<&[bool]>) -> impl IntoElement {
    let z = view.zoom;
    let offset = view.offset;
    let bounds_cell = view.bounds.clone();
    let curves: Vec<(Point<f32>, Point<f32>, bool, bool)> = view
        .lineage
        .edges
        .iter()
        .map(|&(from, to)| {
            let (a, b) = (view.placed[from], view.placed[to]);
            let both_lit = lit.is_some_and(|lit| lit[from] && lit[to]);
            let highlighted = view.focused.is_some() && both_lit;
            let dim = lit.is_some() && !both_lit;
            (
                Point { x: a.x + NODE_WIDTH, y: a.y + NODE_HEIGHT / 2. },
                Point { x: b.x, y: b.y + NODE_HEIGHT / 2. },
                highlighted,
                dim,
            )
        })
        .collect();
    canvas(
        move |bounds, window, _| {
            // A new size (the panel opening wider) gets another frame, which fits the graph to it.
            if bounds_cell.get().map(|old| old.size) != Some(bounds.size) {
                window.refresh();
            }
            bounds_cell.set(Some(bounds));
        },
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            let at = |p: Point<f32>| {
                point(bounds.origin.x + offset.x + px(p.x * z), bounds.origin.y + offset.y + px(p.y * z))
            };
            for (from, to, highlighted, dim) in &curves {
                let (start, end) = (at(*from), at(*to));
                let reach = (end.x - start.x).max(px(32. * z)) / 2.;
                let mut path = PathBuilder::stroke(px(if *highlighted { 2. } else { 1.2 }));
                path.move_to(start);
                path.cubic_bezier_to(end, point(start.x + reach, start.y), point(end.x - reach, end.y));
                let colour: Hsla = if *highlighted {
                    theme::color(color::BLUE)
                } else if *dim {
                    theme::tone(tone::HOVER)
                } else {
                    theme::tone(tone::STRONG)
                };
                if let Ok(path) = path.build() {
                    window.paint_path(path, colour);
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Render for LineageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let resized = self.bounds.get().is_some_and(|bounds| Some(bounds.size) != self.fitted_size);
        if resized && !self.moved && self.loaded {
            self.fit_pending = true;
        }
        if self.fit_pending && self.bounds.get().is_some() && self.loaded {
            self.fit();
        } else if self.fit_pending && self.loaded {
            // The view's size arrives with the first frame; fit on the next one.
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        if self.loaded && self.whole.nodes.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_scale(text::BODY)
                .text_color(theme::color(color::INK_MUTED))
                .child("This dbt project has no models yet.")
                .into_any_element();
        }
        let lit = self.lit();
        let mut root = div()
            .id("lineage")
            .relative()
            .size_full()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.dragging_from = Some(event.position);
                    this.focused = None;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let (Some(from), Some(MouseButton::Left)) = (this.dragging_from, event.pressed_button) {
                    this.offset += event.position - from;
                    this.moved = true;
                    this.dragging_from = Some(event.position);
                    cx.notify();
                } else {
                    this.dragging_from = None;
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.dragging_from = None))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.));
                if event.modifiers.platform || event.modifiers.control {
                    let factor = (1. + f32::from(delta.y) / 200.).clamp(0.5, 2.);
                    this.zoom_at(factor, event.position, cx);
                } else {
                    this.offset += delta;
                    this.moved = true;
                    cx.notify();
                }
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                this.zoom_at(1. + event.delta, event.position, cx);
            }));
        if self.lineage.nodes.is_empty() {
            root = root.child(self.render_prompt());
        }
        root = root.child(edges(self, lit.as_deref()));
        for index in 0..self.lineage.nodes.len() {
            let dimmed = lit.as_ref().is_some_and(|lit| !lit[index]);
            root = root.child(self.render_node(index, dimmed, cx));
        }
        let search = self.search_field(window, cx);
        let found = self.matches().len();
        root.child(
            chrome::floating(div())
                .absolute()
                .top(len(space::SPACE_2))
                .left(len(space::SPACE_2))
                .w(px(280.))
                .h(px(36.))
                .px(len(space::SPACE_3))
                .rounded(theme::corner(radius::RADIUS_MD))
                .flex()
                .items_center()
                .gap(len(space::SPACE_2))
                .child(chrome::icon("search", icon::MD, theme::color(color::INK_FAINT)))
                .child(div().flex_1().child(Input::new(&search).appearance(false)))
                .when(!self.query.is_empty(), |row| row.child(chrome::clear_button(&search)))
                .when(!self.query.trim().is_empty(), |row| {
                    row.child(
                        div().flex_none().text_scale(text::MICRO).text_color(theme::color(color::INK_FAINT)).child(
                            match found {
                                0 => "none".to_owned(),
                                1 => "1 node".to_owned(),
                                n => format!("{n} nodes"),
                            },
                        ),
                    )
                }),
        )
        .child(self.render_controls(cx))
        .child(self.render_caption())
        .into_any_element()
    }
}
