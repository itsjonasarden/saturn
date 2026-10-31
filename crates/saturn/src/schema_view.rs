//! The database diagram, in the editor panel: every table as a box with its columns, and a line for
//! every foreign key. Scroll or drag to pan, pinch (or ⌘-scroll) to zoom. Clicking a table focuses
//! it: its relations light up and unrelated tables step back; double-clicking also zooms to it.
//! Searching highlights the tables whose names match, and Enter zooms to the first.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, PathBuilder, PinchEvent, Pixels, Point, Render,
    ScrollWheelEvent, StatefulInteractiveElement as _, Styled as _, Subscription, Window, canvas, div, point,
    prelude::FluentBuilder as _, px,
};
use saturn_core::schema::{self, FRAME_TITLE, Frame, HEADER_HEIGHT, Key, Placed, ROW_HEIGHT, Schema};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::icon;
use saturn_design::text;
use saturn_design::tone;

const MIN_ZOOM: f32 = 0.15;
const MAX_ZOOM: f32 = 2.;
/// Space around the diagram when it's fitted to the view.
const MARGIN: f32 = 32.;

pub struct SchemaView {
    repo: PathBuf,
    schema: Schema,
    placed: Vec<Placed>,
    /// The groups' labelled areas, when the schema has groups.
    groups: Vec<Frame>,
    loaded: bool,
    zoom: f32,
    /// Where the diagram's origin sits in the view, in pixels.
    offset: Point<Pixels>,
    /// The view's bounds on screen, from the last frame: events come in window coordinates.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Fit the diagram to the view once its size is known.
    fit_pending: bool,
    /// Where a drag started, to pan by.
    dragging_from: Option<Point<Pixels>>,
    focused: Option<usize>,
    /// The search field, made on the first frame (it needs the window).
    search: Option<Entity<InputState>>,
    query: String,
    search_events: Option<Subscription>,
}

impl SchemaView {
    pub fn new(repo: &Path, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            repo: repo.to_owned(),
            schema: Schema::default(),
            placed: Vec::new(),
            groups: Vec::new(),
            loaded: false,
            zoom: 1.,
            offset: point(px(MARGIN), px(MARGIN)),
            bounds: Rc::new(Cell::new(None)),
            fit_pending: true,
            dragging_from: None,
            focused: None,
            search: None,
            query: String::new(),
            search_events: None,
        };
        if std::env::var_os("SATURN_SCHEMA").is_some() {
            // Screenshots: the test window stops drawing before a background read would land.
            view.schema = schema::load(repo);
            let layout = schema::layout(&view.schema);
            (view.placed, view.groups) = (layout.tables, layout.groups);
            view.loaded = true;
        } else {
            view.reload(cx);
        }
        view
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    /// Reads the schema files again, off the main thread.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let (schema, layout) = cx
                .background_spawn(async move {
                    let schema = schema::load(&repo);
                    let layout = schema::layout(&schema);
                    (schema, layout)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.schema = schema;
                (this.placed, this.groups) = (layout.tables, layout.groups);
                this.loaded = true;
                this.fit_pending = true;
                cx.notify();
            });
        })
        .detach();
    }

    /// The whole diagram in diagram units.
    fn extent(&self) -> (f32, f32) {
        let tables =
            self.placed.iter().fold((0., 0.), |(w, h): (f32, f32), p| (w.max(p.x + p.width), h.max(p.y + p.height)));
        self.groups.iter().fold(tables, |(w, h), g| (w.max(g.x + g.width), h.max(g.y + g.height)))
    }

    /// Zooms so the whole diagram fits, at most 100%.
    fn fit(&mut self) {
        let Some(bounds) = self.bounds.get() else { return };
        let (width, height) = self.extent();
        let (zoom, x, y) =
            schema::fit(width, height, f32::from(bounds.size.width), f32::from(bounds.size.height), MARGIN, MIN_ZOOM);
        self.zoom = zoom;
        self.offset = point(px(x), px(y));
        self.fit_pending = false;
    }

    /// Zooms by `factor` keeping the diagram point under `anchor` (window coordinates) in place.
    fn zoom_at(&mut self, factor: f32, anchor: Point<Pixels>, cx: &mut Context<Self>) {
        let origin = self.bounds.get().map_or(point(px(0.), px(0.)), |bounds| bounds.origin);
        let local = anchor - origin;
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let scale = new_zoom / self.zoom;
        self.offset = point(local.x - (local.x - self.offset.x) * scale, local.y - (local.y - self.offset.y) * scale);
        self.zoom = new_zoom;
        cx.notify();
    }

    /// Zooms to `table` (up to 100%) with it in the middle, and focuses it.
    fn zoom_to(&mut self, table: usize, cx: &mut Context<Self>) {
        let Some(place) = self.placed.get(table).copied() else { return };
        self.zoom_to_area(place.x, place.y, place.width, place.height, cx);
        self.focused = Some(table);
        cx.notify();
    }

    /// Zooms so the area fills the view (up to 100%), centred.
    fn zoom_to_area(&mut self, x: f32, y: f32, width: f32, height: f32, cx: &mut Context<Self>) {
        let Some(bounds) = self.bounds.get() else { return };
        let (view_w, view_h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        self.zoom = ((view_w - MARGIN * 2.) / width).min((view_h - MARGIN * 2.) / height).clamp(MIN_ZOOM, 1.);
        self.offset =
            point(px(view_w / 2. - (x + width / 2.) * self.zoom), px(view_h / 2. - (y + height / 2.) * self.zoom));
        cx.notify();
    }

    /// A group's area: a soft panel with its name and size on top. Double-click it to zoom to it.
    fn render_group(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let frame = &self.groups[index];
        let z = self.zoom;
        let (x, y, width, height) = (frame.x, frame.y, frame.width, frame.height);
        div()
            .id(("group", index))
            .absolute()
            .left(self.offset.x + px(x * z))
            .top(self.offset.y + px(y * z))
            .w(px(width * z))
            .h(px(height * z))
            .rounded(px(saturn_design::corner(radius::RADIUS_LG) * z.max(0.5)))
            .bg(theme::overlay(0.025))
            .border_1()
            .border_color(theme::overlay(0.07))
            .child(
                div()
                    .h(px(FRAME_TITLE * z))
                    .px(px(24. * z))
                    .flex()
                    .items_center()
                    .gap(px(8. * z))
                    .text_size(px(text::SUBTITLE.size * z))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::color(color::INK_MUTED))
                    .child(frame.name.clone())
                    .child(
                        div()
                            .text_size(px(text::META.size * z))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::color(color::INK_FAINT))
                            .child(match frame.tables {
                                1 => "1 table".to_owned(),
                                n => format!("{n} tables"),
                            }),
                    ),
            )
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.zoom_to_area(x, y, width, height, cx);
                }
            }))
            .into_any_element()
    }

    /// Tables whose names contain the search, in order.
    fn matches(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        (0..self.schema.tables.len()).filter(|&i| self.schema.tables[i].name.to_lowercase().contains(&query)).collect()
    }

    /// The search field, top left; made once, when the window is at hand.
    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(search) = &self.search {
            return search.clone();
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search tables"));
        self.search_events = Some(cx.subscribe(&search, |this, search, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                this.query = search.read(cx).value().to_string();
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                if let Some(&first) = this.matches().first() {
                    this.zoom_to(first, cx);
                }
            }
            _ => {}
        }));
        self.search = Some(search.clone());
        search
    }

    fn zoom_centre(&mut self, factor: f32, cx: &mut Context<Self>) {
        let centre = self.bounds.get().map_or(point(px(0.), px(0.)), |bounds| bounds.center());
        self.zoom_at(factor, centre, cx);
    }

    /// The tables to keep bright: the search's matches while searching, else the focused table and
    /// the tables related to it. `None` keeps them all bright.
    fn related(&self) -> Option<Vec<bool>> {
        if !self.query.trim().is_empty() {
            let mut lit = vec![false; self.schema.tables.len()];
            for table in self.matches() {
                lit[table] = true;
            }
            return Some(lit);
        }
        let focused = self.focused?;
        let name = &self.schema.tables[focused].name;
        let mut related = vec![false; self.schema.tables.len()];
        related[focused] = true;
        for relation in &self.schema.relations {
            let other = if &relation.from_table == name {
                &relation.to_table
            } else if &relation.to_table == name {
                &relation.from_table
            } else {
                continue;
            };
            if let Some(index) = self.schema.tables.iter().position(|table| &table.name == other) {
                related[index] = true;
            }
        }
        Some(related)
    }

    fn render_table(&self, index: usize, dimmed: bool, cx: &mut Context<Self>) -> AnyElement {
        let table = &self.schema.tables[index];
        let place = self.placed[index];
        let z = self.zoom;
        let focused = self.focused == Some(index);
        div()
            .id(("table", index))
            .absolute()
            .left(self.offset.x + px(place.x * z))
            .top(self.offset.y + px(place.y * z))
            .w(px(place.width * z))
            .h(px(place.height * z))
            .flex()
            .flex_col()
            .rounded(px(saturn_design::corner(radius::RADIUS_SM) * z.max(0.5)))
            .bg(theme::color(color::SURFACE))
            .border_1()
            .border_color(if focused { theme::color(color::BLUE) } else { theme::tone(tone::PRESSED) })
            .when(dimmed, |table| table.opacity(0.3))
            .overflow_hidden()
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6. * z))
                    .h(px(HEADER_HEIGHT * z))
                    .px(px(10. * z))
                    .bg(theme::fill_hover())
                    .border_b_1()
                    .border_color(theme::separator())
                    .child(chrome::icon("database", 12. * z, theme::color(color::BLUE)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(text::LABEL.size * z))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::color(color::INK))
                            .child(table.name.clone()),
                    ),
            )
            .child(div().flex().flex_col().pt(px(4. * z)).children(table.columns.iter().map(|column| {
                let (icon, tint) = match column.key {
                    Some(Key::Primary) => (Some("key-round"), color::AMBER_TEXT),
                    Some(Key::Foreign) => (Some("link-2"), color::BLUE),
                    Some(Key::Unique) => (Some("key-round"), color::INK_FAINT),
                    None => (None, color::INK_FAINT),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6. * z))
                    .h(px(ROW_HEIGHT * z))
                    .px(px(10. * z))
                    .child(
                        div()
                            .flex_none()
                            .w(px(12. * z))
                            .children(icon.map(|icon| chrome::icon(icon, 11. * z, theme::color(tint)))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(text::META.size * z))
                            .text_color(theme::color(color::INK_BODY))
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(100. * z))
                            .truncate()
                            .font_family(saturn_design::font::MONO)
                            .text_size(px(text::MICRO.size * z))
                            .text_color(theme::color(color::INK_FAINT))
                            .child(format!("{}{}", column.ty, if column.nullable { "?" } else { "" })),
                    )
            })))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.zoom_to(index, cx);
                } else {
                    this.focused = if this.focused == Some(index) { None } else { Some(index) };
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    /// Zoom out, the zoom level, zoom in, and fit: floating in the corner.
    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().absolute().top(len(space::SPACE_2)).right(len(space::SPACE_2)).child(
            chrome::floating(chrome::track())
                .child(
                    chrome::icon_button("zoom-out", "zoom-out", true)
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
                    chrome::icon_button("zoom-in", "zoom-in", true)
                        .tooltip(chrome::tip("Zoom in (or pinch, or ⌘-scroll)"))
                        .on_click(cx.listener(|this, _, _, cx| this.zoom_centre(1.25, cx))),
                )
                .child(chrome::track_separator())
                .child(
                    chrome::icon_button("fit", "scan", true).tooltip(chrome::tip("Fit the whole diagram")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.fit();
                            cx.notify();
                        }),
                    ),
                ),
        )
    }

    /// What the diagram was read from, bottom left.
    fn render_caption(&self) -> impl IntoElement {
        let sources = match self.schema.sources.as_slice() {
            [one] => one.display().to_string(),
            many => format!("{} schema files", many.len()),
        };
        div()
            .absolute()
            .left(len(space::SPACE_3))
            .bottom(len(space::SPACE_2))
            .text_scale(text::MICRO)
            .text_color(theme::color(color::INK_FAINT))
            .child(format!(
                "{sources} · {} tables · {} relations · click a table to focus it, double-click to zoom to it",
                self.schema.tables.len(),
                self.schema.relations.len()
            ))
    }
}

/// The foreign keys, as curves from a column's row to the referenced column's row.
fn relations(view: &SchemaView, related: Option<&[bool]>) -> impl IntoElement {
    let z = view.zoom;
    let offset = view.offset;
    let bounds_cell = view.bounds.clone();
    let focused = view.focused.map(|index| view.schema.tables[index].name.clone());
    // (from point, to point, highlighted)
    let mut curves: Vec<(Point<f32>, Point<f32>, bool, bool)> = Vec::new();
    let find = |name: &str| view.schema.tables.iter().position(|table| table.name == name);
    for relation in &view.schema.relations {
        let (Some(from), Some(to)) = (find(&relation.from_table), find(&relation.to_table)) else { continue };
        let row = |table: usize, column: &str| {
            let position = view.schema.tables[table].columns.iter().position(|c| c.name == column).unwrap_or(0);
            HEADER_HEIGHT + 4. + ROW_HEIGHT * (f32::from(u16::try_from(position).unwrap_or(0)) + 0.5)
        };
        let (a, b) = (view.placed[from], view.placed[to]);
        let (ay, by) = (a.y + row(from, &relation.from_column), b.y + row(to, &relation.to_column));
        let rightward = a.x + a.width <= b.x;
        let (ax, bx) = if rightward {
            (a.x + a.width, b.x)
        } else if b.x + b.width <= a.x {
            (a.x, b.x + b.width)
        } else {
            (a.x + a.width, b.x + b.width)
        };
        let lit = focused.as_ref().is_some_and(|name| *name == relation.from_table || *name == relation.to_table);
        let dim = related.is_some() && !lit;
        curves.push((Point { x: ax, y: ay }, Point { x: bx, y: by }, lit, dim));
    }
    canvas(
        move |bounds, _, _| bounds_cell.set(Some(bounds)),
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            let at = |p: Point<f32>| {
                point(bounds.origin.x + offset.x + px(p.x * z), bounds.origin.y + offset.y + px(p.y * z))
            };
            for (from, to, lit, dim) in &curves {
                let (start, end) = (at(*from), at(*to));
                let reach = px((40. * z).max(16.)) * if end.x >= start.x { 1. } else { -1. };
                let same_side = (from.x - to.x).abs() < 1.;
                let (c1, c2) = if same_side {
                    (point(start.x + px(40. * z), start.y), point(end.x + px(40. * z), end.y))
                } else {
                    (point(start.x + reach, start.y), point(end.x - reach, end.y))
                };
                let mut path = PathBuilder::stroke(px(if *lit { 2. } else { 1.2 }));
                path.move_to(start);
                path.cubic_bezier_to(end, c1, c2);
                let colour: Hsla = if *lit {
                    theme::color(color::BLUE)
                } else if *dim {
                    theme::tone(tone::HOVER)
                } else {
                    theme::overlay(0.28)
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

impl Render for SchemaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.fit_pending && self.bounds.get().is_some() && self.loaded {
            self.fit();
        } else if self.fit_pending && self.loaded {
            // The view's size arrives with the first frame; fit on the next one.
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        if self.loaded && self.schema.tables.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_scale(text::BODY)
                .text_color(theme::color(color::INK_MUTED))
                .child("No database schema found: Saturn looks for Prisma schemas and SQL migrations.")
                .into_any_element();
        }
        let related = self.related();
        let mut root = div()
            .id("schema")
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
                    cx.notify();
                }
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                this.zoom_at(1. + event.delta, event.position, cx);
            }));
        // Groups at the back, then the relations' lines, then the tables on top.
        for index in 0..self.groups.len() {
            root = root.child(self.render_group(index, cx));
        }
        root = root.child(relations(self, related.as_deref()));
        for index in 0..self.schema.tables.len() {
            let dimmed = related.as_ref().is_some_and(|related| !related[index]);
            root = root.child(self.render_table(index, dimmed, cx));
        }
        let search = self.search_field(window, cx);
        let found = self.matches().len();
        root.child(
            // An island over the diagram, as the zoom controls are: tables never show through.
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
                .child(div().flex_1().child(Input::new(&search).appearance(false).cleanable(true)))
                .when(!self.query.trim().is_empty(), |row| {
                    row.child(
                        div().flex_none().text_scale(text::MICRO).text_color(theme::color(color::INK_FAINT)).child(
                            match found {
                                0 => "none".to_owned(),
                                1 => "1 table".to_owned(),
                                n => format!("{n} tables"),
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
