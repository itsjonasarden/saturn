//! Draws a terminal's screen: cell backgrounds as merged rectangles, text as shaped lines, the
//! cursor on top.
//!
//! Same-style runs of plain ASCII are shaped as one line: in a monospace font each character is one
//! cell wide, so columns stay true. Any other character (wide, emoji, box drawing from a fallback
//! font) is shaped on its own and placed at its own column, so a fallback font's width can never
//! push the rest of the row off the grid. The same approach as zortax/gpui-terminal (MIT/Apache)
//! and yicheng47/runner (MIT).

use akyba_design::font as fonts;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::point_to_viewport;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use gpui_kit::{
    App, BorderStyle, Bounds, Element, Entity, Font, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, Rgba, ShapedLine, SharedString, Style, TextAlign, TextRun, UnderlineStyle,
    Window, fill, font, outline, point, px, relative, size,
};

use crate::palette;
use crate::terminal::GridSize;
use crate::view::{Geometry, TerminalView};

/// The text size terminals start at, and go back to with ⌘0.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;

/// The text size every terminal draws at now (⌘+ and ⌘−), as `f32` bits.
static FONT_SIZE_BITS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x4150_0000);

/// The text size terminals draw at.
pub fn font_size() -> f32 {
    f32::from_bits(FONT_SIZE_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Sets the text size every terminal draws at; each lays itself out again on its next frame.
pub fn set_font_size(size: f32) {
    FONT_SIZE_BITS.store(size.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Time spent laying out terminal frames, for the performance spike (`AKYBA_PERF=1`).
pub mod stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    static FRAMES: AtomicU64 = AtomicU64::new(0);
    static NANOS: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn record(nanos: u64) {
        FRAMES.fetch_add(1, Ordering::Relaxed);
        NANOS.fetch_add(nanos, Ordering::Relaxed);
    }

    /// Terminal frames laid out and the time they took since the last call.
    pub fn take() -> (u64, std::time::Duration) {
        (FRAMES.swap(0, Ordering::Relaxed), std::time::Duration::from_nanos(NANOS.swap(0, Ordering::Relaxed)))
    }
}
/// Line height as a multiple of the font size (the design system's `terminal` style).
const LINE_HEIGHT: f32 = 1.2;

pub fn hsla(color: Rgb, alpha: f32) -> Hsla {
    Rgba { r: f32::from(color.r) / 255.0, g: f32::from(color.g) / 255.0, b: f32::from(color.b) / 255.0, a: alpha }
        .into()
}

pub struct TerminalElement {
    view: Entity<TerminalView>,
    focused: bool,
    /// Where the grid was drawn, so the view can turn mouse positions into cells.
    geometry: std::rc::Rc<std::cell::Cell<Option<Geometry>>>,
}

impl TerminalElement {
    pub fn new(
        view: Entity<TerminalView>,
        focused: bool,
        geometry: std::rc::Rc<std::cell::Cell<Option<Geometry>>>,
    ) -> Self {
        Self { view, focused, geometry }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

enum CursorPaint {
    Block { bounds: Bounds<Pixels>, text: Option<Box<(Point<Pixels>, ShapedLine)>> },
    Hollow(Bounds<Pixels>),
    Bar(Bounds<Pixels>),
}

pub struct Frame {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<CursorPaint>,
    line_height: Pixels,
}

/// The look of one cell's text; a run continues while this stays the same.
#[derive(Clone, Copy, PartialEq)]
struct TextStyle {
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
}

struct Span {
    row: usize,
    col: usize,
    end: usize,
    text: String,
    style: TextStyle,
}

fn terminal_font(style: TextStyle) -> Font {
    let mut font = font(fonts::MONO);
    if style.bold {
        font.weight = FontWeight::BOLD;
    }
    if style.italic {
        font.style = FontStyle::Italic;
    }
    font
}

fn shape(text: String, style: TextStyle, font_size: Pixels, window: &Window) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: terminal_font(style),
        color: style.color,
        background_color: None,
        underline: style.underline.then_some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(style.color),
            wavy: false,
        }),
        strikethrough: None,
    };
    window.text_system().shape_line(SharedString::from(text), font_size, &[run], None)
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Frame;

    fn id(&self) -> Option<gpui_kit::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let started = std::time::Instant::now();
        let frame = self.layout(bounds, window, cx);
        stats::record(u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX));
        frame
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut (),
        frame: &mut Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        paint_frame(frame, window, cx);
    }
}

impl TerminalElement {
    #[expect(clippy::too_many_lines, reason = "one pass over the screen, kept together")]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "cell counts and pixel sizes are small"
    )]
    fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Frame {
        let points = font_size();
        let font_size = px(points);
        let font_id = window.text_system().resolve_font(&font(fonts::MONO));
        let cell_width = window.text_system().em_advance(font_id, font_size).unwrap_or(px(points * 0.6));
        let line_height = px((points * LINE_HEIGHT).round());

        let cols = (f32::from(bounds.size.width) / f32::from(cell_width)).floor().max(2.0) as u16;
        let rows = (f32::from(bounds.size.height) / f32::from(line_height)).floor().max(1.0) as u16;
        let wanted = GridSize { cols, rows };
        self.geometry.set(Some(Geometry { origin: bounds.origin, cell_width, line_height }));
        if self.view.read(cx).terminal().size() != wanted {
            self.view.update(cx, |view, _| view.resize(wanted));
        }

        let view = self.view.read(cx);
        let term = view.terminal().term();
        let content = term.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset;
        let origin = bounds.origin;
        let cell_bounds = |row: usize, col: usize, width: usize| {
            Bounds::new(
                point(origin.x + cell_width * col as f32, origin.y + line_height * row as f32),
                size(cell_width * width as f32, line_height),
            )
        };

        let link = view.hovered_link();
        let selection = content.selection;
        let selection_color = hsla(palette::selection(), palette::selection_opacity());
        let mut backgrounds: Vec<(usize, usize, usize, Hsla)> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        let mut singles: Vec<(usize, usize, String, TextStyle)> = Vec::new();

        for indexed in content.display_iter {
            let Some(at) = point_to_viewport(offset, indexed.point) else { continue };
            let (row, col) = (at.line, at.column.0);
            let cell = indexed.cell;
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            let (mut fg, mut bg) = (palette::resolve(cell.fg, colors), palette::resolve(cell.bg, colors));
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            let selected = selection.is_some_and(|range| range.contains(indexed.point));
            if selected || bg != palette::background() {
                let color = if selected { selection_color } else { hsla(bg, 1.0) };
                match backgrounds.last_mut() {
                    Some(last) if last.0 == row && last.2 == col && last.3 == color => last.2 = col + width,
                    _ => backgrounds.push((row, col, col + width, color)),
                }
            }

            if flags.contains(Flags::HIDDEN) || cell.c == ' ' || cell.c == '\0' {
                continue;
            }
            let style = TextStyle {
                color: hsla(fg, if flags.contains(Flags::DIM) { 0.66 } else { 1.0 }),
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES)
                    || link.is_some_and(|link| link.contains(indexed.point)),
            };
            let simple = cell.c.is_ascii() && width == 1 && cell.zerowidth().is_none();
            if simple {
                match spans.last_mut() {
                    // Spaces between same-style text stay in the run.
                    Some(span) if span.row == row && span.style == style && col >= span.end && col - span.end <= 3 => {
                        span.text.extend(std::iter::repeat_n(' ', col - span.end));
                        span.text.push(cell.c);
                        span.end = col + 1;
                    }
                    _ => spans.push(Span { row, col, end: col + 1, text: cell.c.to_string(), style }),
                }
            } else {
                let mut text = cell.c.to_string();
                text.extend(cell.zerowidth().into_iter().flatten());
                singles.push((row, col, text, style));
            }
        }

        let backgrounds = backgrounds
            .into_iter()
            .map(|(row, start, end, color)| (cell_bounds(row, start, end - start), color))
            .collect();
        let mut lines: Vec<(Point<Pixels>, ShapedLine)> = spans
            .into_iter()
            .map(|span| (cell_bounds(span.row, span.col, 1).origin, shape(span.text, span.style, font_size, window)))
            .collect();
        lines.extend(
            singles.into_iter().map(|(row, col, text, style)| {
                (cell_bounds(row, col, 1).origin, shape(text, style, font_size, window))
            }),
        );

        let cursor = (term.mode().contains(TermMode::SHOW_CURSOR) && content.cursor.shape != CursorShape::Hidden)
            .then(|| point_to_viewport(offset, content.cursor.point))
            .flatten()
            .filter(|at| at.line < term.screen_lines())
            .map(|at| {
                let cell = &term.grid()[content.cursor.point];
                let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
                let bounds = cell_bounds(at.line, at.column.0, width);
                match (self.focused, content.cursor.shape) {
                    (false, _) => CursorPaint::Hollow(bounds),
                    (true, CursorShape::Beam) => {
                        CursorPaint::Bar(Bounds::new(bounds.origin, size(px(2.0), line_height)))
                    }
                    (true, CursorShape::Underline) => CursorPaint::Bar(Bounds::new(
                        point(bounds.origin.x, bounds.origin.y + line_height - px(2.0)),
                        size(bounds.size.width, px(2.0)),
                    )),
                    (true, _) => {
                        let text = (cell.c != ' ' && cell.c != '\0').then(|| {
                            let style = TextStyle {
                                color: hsla(palette::background(), 1.0),
                                bold: cell.flags.contains(Flags::BOLD),
                                italic: cell.flags.contains(Flags::ITALIC),
                                underline: false,
                            };
                            Box::new((bounds.origin, shape(cell.c.to_string(), style, font_size, window)))
                        });
                        CursorPaint::Block { bounds, text }
                    }
                }
            });

        Frame { backgrounds, lines, cursor, line_height }
    }
}

fn paint_frame(frame: &Frame, window: &mut Window, cx: &mut App) {
    for (bounds, color) in &frame.backgrounds {
        window.paint_quad(fill(*bounds, *color));
    }
    for (origin, line) in &frame.lines {
        let _ = line.paint(*origin, frame.line_height, TextAlign::Left, None, window, cx);
    }
    let cursor_color = hsla(palette::cursor(), 1.0);
    match &frame.cursor {
        Some(CursorPaint::Block { bounds, text }) => {
            window.paint_quad(fill(*bounds, cursor_color));
            if let Some(text) = text {
                let (origin, line) = &**text;
                let _ = line.paint(*origin, frame.line_height, TextAlign::Left, None, window, cx);
            }
        }
        Some(CursorPaint::Hollow(bounds)) => window.paint_quad(outline(*bounds, cursor_color, BorderStyle::default())),
        Some(CursorPaint::Bar(bounds)) => window.paint_quad(fill(*bounds, cursor_color)),
        None => {}
    }
}
