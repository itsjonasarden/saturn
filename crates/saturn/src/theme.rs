//! The design system's tokens as GPUI values, in the current appearance.

use gpui_kit::{BoxShadow, Hsla, Pixels, Rgba, point, px};
use saturn_design::{Appearance, Color, Shadow, Shadows, Token};

use crate::settings::{AppearanceSetting, Settings};

pub fn color(token: Token) -> Hsla {
    rgba(token.get())
}

fn rgba(color: Color) -> Hsla {
    #[expect(clippy::cast_precision_loss, reason = "8-bit channels")]
    let channel = |shift: u32| ((color.rgb >> shift) & 0xff) as f32 / 255.0;
    Rgba { r: channel(16), g: channel(8), b: channel(0), a: color.alpha }.into()
}

pub fn shadows(token: Shadows) -> Vec<BoxShadow> {
    layers(token.get())
}

pub fn layers(layers: &[Shadow]) -> Vec<BoxShadow> {
    layers
        .iter()
        .map(|layer| BoxShadow {
            color: rgba(layer.color),
            offset: point(px(layer.x), px(layer.y)),
            blur_radius: px(layer.blur),
            spread_radius: px(layer.spread),
            inset: layer.inset,
        })
        .collect()
}

// Layering, after how macOS builds its windows: the chrome (title bar, sidebar, the gaps between
// panes) is clear, so the window's own sidebar material shows: blur, saturation, a fill and a
// brightness clamp, drawn by macOS. Content (terminals, the editor) sits on opaque cards. Strokes
// and control fills on either are relative, black in light mode and white in dark at a low alpha,
// so they read the same over any background, as AppKit's separator and fill colours do.

/// Black in light mode, white in dark, at `alpha`: a fill or stroke relative to what's under it.
/// A relative fill or stroke at a named strength (design/tokens.json's tones).
pub fn tone(tone: saturn_design::Tone) -> Hsla {
    overlay(tone.get())
}

pub fn overlay(alpha: f32) -> Hsla {
    let base = if saturn_design::appearance() == saturn_design::Appearance::Dark { 1.0 } else { 0.0 };
    gpui_kit::hsla(0., 0., base, alpha)
}

/// A hairline between things: AppKit's separator colour (10%).
pub fn separator() -> Hsla {
    tone(saturn_design::tone::SEPARATOR)
}

/// Fills for controls on glass or cards, from faint to strong (AppKit's system fills).
pub fn fill_hover() -> Hsla {
    tone(saturn_design::tone::HOVER)
}

pub fn fill_control() -> Hsla {
    tone(saturn_design::tone::CONTROL)
}

pub fn fill_pressed() -> Hsla {
    overlay(0.11)
}

/// gpui-component's own controls (switches, dialogs, inputs, scrollbars) in Saturn's colours,
/// fonts and radii instead of the library's grey defaults, for one appearance.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "radii are small whole pixels")]
fn component_theme(appearance: Appearance) -> gpui_kit::component::ThemeConfig {
    use saturn_design::{color as c, font, radius};

    let hex = |token: Token| hex(token, appearance);
    // Relative colours as `#rrggbbaa`: black in light mode, white in dark (see `overlay`).
    let relative = |alpha: f32| {
        let base = if appearance == Appearance::Dark { "ffffff" } else { "000000" };
        format!("#{base}{:02x}", (alpha * 255.).round() as u8)
    };
    let colors = serde_json::json!({
        // The root view paints this under the whole window; clear, so the glass (the desktop's
        // blur through the ground and the panes) shows. Dialogs, which also default to it, set
        // their own surface (chrome::dialog_surface).
        "background": "#00000000",
        "foreground": hex(c::INK),
        "border": relative(0.1),
        "input.border": relative(0.16),
        // Cyberdeck is monochrome: focus is ink, as its selections are.
        "ring": hex(saturn_design::look().focus),
        "muted.background": hex(c::CONTROL),
        "muted.foreground": hex(c::INK_MUTED),
        "accent.background": hex(c::CONTROL_HOVER),
        "accent.foreground": hex(c::INK),
        "primary.background": hex(c::BLUE_VIVID),
        "primary.hover.background": hex(c::BLUE_VIVID),
        "primary.active.background": hex(c::BLUE_DEEP),
        "primary.foreground": hex(c::INK_ON_ACCENT),
        "secondary.background": relative(0.08),
        "secondary.hover.background": relative(0.11),
        "secondary.active.background": relative(0.14),
        "secondary.foreground": hex(c::INK),
        "popover.background": hex(c::SURFACE_RAISED),
        "popover.foreground": hex(c::INK),
        // A light dim under dialogs: the window stays sharp behind them.
        "overlay": if appearance == Appearance::Dark { "#00000059" } else { "#1a10301f" },
        "selection.background": format!("{}{}", hex(c::BLUE_VIVID), if appearance == Appearance::Dark { "73" } else { "40" }),
        "caret": hex(c::INK),
        "switch.background": hex(c::CONTROL_PRESSED),
        "switch.thumb.background": hex(c::INK_ON_ACCENT),
        "scrollbar.background": "#00000000",
        "scrollbar.thumb.background": relative(0.22),
        "scrollbar.thumb.hover.background": hex(c::INK_FAINT),
        // The diff view's added and removed lines.
        "success.background": hex(c::TERM_GREEN),
        "success.foreground": hex(c::INK_ON_SIGNAL),
        "danger.background": hex(c::RED),
        // Dark mode's red is light: dark ink on it, as on green.
        "danger.foreground": hex(if appearance == Appearance::Dark { c::INK_ON_SIGNAL } else { c::INK_ON_ACCENT }),
    });
    let number_only = saturn_design::look().current_line == saturn_design::CurrentLine::Number;
    let highlight = serde_json::json!({
        // The editor sits in a well, which draws the glass; its own background stays clear.
        "editor.background": "#00000000",
        "editor.foreground": hex(c::INK_BODY),
        // The kit paints the current line's fill across the line numbers too; a theme that marks
        // the line by its number alone has no fill, and the number in full ink.
        "editor.active_line.background": if number_only { "#00000000".to_owned() } else { hex(c::SURFACE_RAISED) },
        "editor.line_number": hex(c::INK_FAINT),
        "editor.active_line_number": hex(if number_only { c::INK } else { c::INK_MUTED }),
        "editor.invisible": hex(c::LINE_STRONG),
        "editor.gutter.background": "#00000000",
        // Language server underlines, in the palette: errors red, warnings the warm text colour
        // (orange means an agent needs you), notes blue, hints (unused, deprecated) faint.
        "error": hex(c::RED),
        "warning": hex(c::AMBER_TEXT),
        "info": hex(c::BLUE),
        "hint": hex(c::INK_FAINT),
        "syntax": syntax(appearance),
    });
    serde_json::from_value(serde_json::json!({
        "name": "Saturn",
        "mode": if appearance == Appearance::Dark { "dark" } else { "light" },
        "font.family": saturn_design::look().sans,
        "font.size": 13,
        "mono_font.family": font::MONO,
        "mono_font.size": 13,
        "radius": saturn_design::corner(radius::RADIUS_SM) as usize,
        "radius.lg": saturn_design::corner(radius::RADIUS_LG) as usize,
        "shadow": false,
        "colors": colors,
        "highlight": highlight,
    }))
    .expect("Saturn's theme config is valid")
}

/// `#rrggbb` for `appearance` in the current theme, as the theme config takes colours as strings.
fn hex(token: Token, appearance: Appearance) -> String {
    format!("#{:06x}", token.at(appearance).rgb)
}

/// The editor's colours: the terminal's palette, used the way most editors use it (One Dark's
/// roles), so code reads the same here as in the panes next to it. Keys and headings get colour
/// too, since in YAML, JSON and Markdown that is most of the file. Bold only where the text
/// itself is a heading or strong; JetBrains Mono ships no italic here.
fn syntax(appearance: Appearance) -> serde_json::Value {
    use saturn_design::color as c;

    let style = |token: Token| serde_json::json!({ "color": hex(token, appearance) });
    let bold = |token: Token| serde_json::json!({ "color": hex(token, appearance), "font_weight": 700 });
    serde_json::json!({
        "keyword": style(c::TERM_MAGENTA),
        "preproc": style(c::TERM_MAGENTA),
        "function": style(c::TERM_BLUE),
        "constructor": style(c::TERM_BLUE),
        "type": style(c::TERM_CYAN),
        "enum": style(c::TERM_CYAN),
        "variant": style(c::TERM_CYAN),
        "label": style(c::TERM_CYAN),
        "property": style(c::TERM_RED),
        "tag": style(c::TERM_RED),
        "variable": style(c::INK_BODY),
        "variable.special": style(c::TERM_RED),
        "attribute": style(c::AMBER_TEXT),
        "string": style(c::TERM_GREEN),
        "string.special": style(c::TERM_RED),
        "string.special.symbol": style(c::TERM_CYAN),
        "string.escape": style(c::TERM_CYAN),
        "string.regex": style(c::TERM_CYAN),
        "number": style(c::AMBER_TEXT),
        "boolean": style(c::AMBER_TEXT),
        "constant": style(c::AMBER_TEXT),
        "comment": style(c::INK_FAINT),
        "comment_doc": style(c::INK_FAINT),
        "operator": style(c::TERM_CYAN),
        "punctuation": style(c::INK_MUTED),
        "punctuation.bracket": style(c::INK_MUTED),
        "punctuation.delimiter": style(c::INK_MUTED),
        "punctuation.special": style(c::INK_FAINT),
        "punctuation.list_marker": style(c::AMBER_TEXT),
        "embedded": style(c::INK_BODY),
        // Markdown.
        "title": bold(c::TERM_BLUE),
        "emphasis": style(c::TERM_MAGENTA),
        "emphasis.strong": bold(c::INK),
        "link_text": style(c::TERM_BLUE),
        "link_uri": style(c::TERM_CYAN),
        "text.literal": style(c::TERM_GREEN),
        "text.code.span": style(c::TERM_GREEN),
    })
}

/// Draws the whole app as `settings` say, in their theme and appearance: Saturn's tokens, the
/// library's controls, and the window frame macOS draws (whose edge is pale in light mode and dark
/// in dark mode).
pub fn apply(settings: &Settings, cx: &mut gpui_kit::App) {
    use gpui_kit::component::{Theme, ThemeMode};

    let setting = settings.appearance;
    saturn_design::set_theme(&settings.theme);
    // Following the system means no override, so macOS reports its own appearance again.
    cx.set_window_appearance(match setting {
        AppearanceSetting::System => None,
        AppearanceSetting::Light => Some(gpui_kit::WindowAppearance::Light),
        AppearanceSetting::Dark => Some(gpui_kit::WindowAppearance::Dark),
    });
    let appearance = setting.resolve(system_appearance(cx.window_appearance()));
    saturn_design::set_appearance(appearance);
    Theme::update(cx, |theme| {
        theme.dark_theme = std::rc::Rc::new(component_theme(Appearance::Dark));
        theme.light_theme = std::rc::Rc::new(component_theme(Appearance::Light));
    });
    Theme::change(if appearance == Appearance::Dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    cx.refresh_windows();
}

/// The text size of terminals and files (⌘+, ⌘−, ⌘0): the terminals' cells, and the editor and
/// diff views' monospace text. The rest of the interface keeps its sizes.
pub fn set_text_size(points: u16, cx: &mut gpui_kit::App) {
    let size = f32::from(points);
    saturn_terminal::element::set_font_size(size);
    gpui_kit::component::Theme::update(cx, |theme| theme.mono_font_size = px(size));
    cx.refresh_windows();
}

/// The appearance macOS is in.
fn system_appearance(appearance: gpui_kit::WindowAppearance) -> Appearance {
    match appearance {
        gpui_kit::WindowAppearance::Light | gpui_kit::WindowAppearance::VibrantLight => Appearance::Light,
        gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

pub fn len(value: f32) -> Pixels {
    px(value)
}

/// The type scale on any element: `.text_scale(text::LABEL)` takes its size from the scale in
/// design/tokens.json, so no view writes a size of its own.
pub trait TypeScale: gpui_kit::Styled + Sized {
    fn text_scale(self, style: saturn_design::TextStyle) -> Self {
        self.text_size(px(style.size))
    }
}

impl<T: gpui_kit::Styled> TypeScale for T {}

/// A corner's radius in the current theme: as given in Saturn's, square in Cyberdeck's.
pub fn corner(radius: f32) -> Pixels {
    px(saturn_design::corner(radius))
}
