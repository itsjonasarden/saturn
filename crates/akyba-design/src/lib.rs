//! The Akyba design system's tokens as Rust constants, compiled from `design/tokens.json`.
//!
//! `color`, `space`, `radius`, `shadow`, `font` and `text` mirror the token families of the design
//! system (<https://claude.ai/artifact/T1AStNjEW6sEdXwrDB2XRm>). Change a value in
//! `design/tokens.json`, never here.
//!
//! A theme ([`Look`], one of [`THEMES`]) is a description: its font, its corners, how far apart its
//! cards sit and which version of each control it draws. Colours and shadows have a dark and a
//! light value in every theme. [`set_theme`] and [`set_appearance`] pick which one the whole app
//! draws with; components read [`look`] and never ask which theme is on.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// An sRGB colour as `0xRRGGBB` plus opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub rgb: u32,
    pub alpha: f32,
}

/// One layer of a glow or drop shadow, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    /// Drawn inside the element.
    pub inset: bool,
    pub color: Color,
}

/// How strong a relative fill or stroke is, in dark mode and in light: white or black at this
/// opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub dark: f32,
    pub light: f32,
}

impl Tone {
    /// The opacity in the current appearance.
    pub fn get(self) -> f32 {
        match appearance() {
            Appearance::Dark => self.dark,
            Appearance::Light => self.light,
        }
    }
}

/// A text style from the type scale: its size and line height in pixels, and its weight (400 is
/// regular, 500 medium).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

/// How a theme draws its corners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corners {
    /// Rounded by the radius tokens, nested from the window inward.
    Round,
    /// Square, with a corner cut off panels, selections and buttons.
    Cut,
}

/// What shows between the cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surfaces {
    /// The window's glass, between translucent cards.
    Glass,
    /// Opaque cards on an opaque ground.
    Opaque,
}

/// How the title bar holds its controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleBar {
    /// In tracks on the cards' surface.
    Tracks,
    /// Bare, filling the bar but for the gap between cards.
    Bare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tabs {
    /// Track items, the chosen one raised.
    Track,
    /// Words over a bar, lit under the chosen one.
    Underline,
}

/// How a chosen track item shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Raised: white in light mode, lighter in dark.
    Raised,
    /// An ink outline round a faint fill, its corner cut.
    Outline,
}

/// How a choice of a few values is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choices {
    /// A track of segments.
    Segments,
    /// The value between two arrows, over a dash for each value.
    Selector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// macOS's switch.
    Toggle,
    /// The word, then a box whose block sits right and lit, or left and dim.
    Box,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Buttons {
    /// Filled; the primary one in the accent gradient with a bloom.
    Filled,
    /// An outline with its corner cut; the primary one blue, its words blue too.
    Outline,
}

/// How a menu marks the line under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menus {
    Fill,
    Outline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRows {
    /// Rows between hairlines.
    Lines,
    /// Each setting its own slab, notched along its top.
    Slabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meters {
    Bars,
    /// Rows of low slanted segments, like a run of slashes.
    Slants,
}

/// What an empty area (the grid with no terminal) shows behind its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyAreas {
    /// A faint dot grid, brightest in the middle.
    Dots,
    /// Faint crosshairs on a wider grid, like a technical sheet; Akyba's mark above the words.
    Crosshairs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCards {
    /// A soft gradient.
    Gradient,
    /// A flat grey slab.
    Slab,
}

/// How a theme's icons are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icons {
    /// Lucide as drawn: round line ends and joins, rounded corners.
    Round,
    /// The same icons with square ends, mitred joins and square corners.
    Sharp,
}

/// How the editor marks the line the cursor is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentLine {
    /// The line is filled.
    Fill,
    /// Only its number is lit.
    Number,
}

/// A theme, as `design/tokens.json` describes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// How settings.json names it.
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// The interface's font family; code stays in [`font::MONO`].
    pub sans: &'static str,
    pub corners: Corners,
    /// How far along each edge a panel's corner is cut, and a selection's or a button's.
    pub panel_cut: f32,
    pub control_cut: f32,
    /// The gap between cards, and round the window's edge.
    pub card_gap: f32,
    /// The width a setting's control is given, right-aligned, so the words beside every control
    /// wrap at one place; the selector and the boxed switch fill it.
    pub control_column: f32,
    /// The colour of focus.
    pub focus: Token,
    pub surfaces: Surfaces,
    pub title_bar: TitleBar,
    pub tabs: Tabs,
    pub selection: Selection,
    pub choices: Choices,
    pub switch: Switch,
    pub buttons: Buttons,
    pub menus: Menus,
    pub setting_rows: SettingRows,
    pub meters: Meters,
    pub empty_areas: EmptyAreas,
    pub agent_cards: AgentCards,
    pub current_line: CurrentLine,
    pub icons: Icons,
    /// Icons drawn as another: a Lucide icon in place of one too round for the theme.
    pub icon_swaps: &'static [(&'static str, &'static str)],
}

static LIGHT: AtomicBool = AtomicBool::new(false);
static THEME: AtomicUsize = AtomicUsize::new(0);

/// The theme everything is drawn in.
pub fn look() -> &'static Look {
    &THEMES[THEME.load(Ordering::Relaxed)]
}

/// Draws everything in the theme `id` names; an unknown one is the default.
pub fn set_theme(id: &str) {
    let index = |id: &str| THEMES.iter().position(|theme| theme.id == id);
    THEME.store(index(id).or_else(|| index(DEFAULT_THEME)).unwrap_or(0), Ordering::Relaxed);
}

/// A corner's radius in the current theme: as the radius tokens give it where corners are round,
/// square where they are cut (the app draws the cuts itself).
pub fn corner(radius: f32) -> f32 {
    match look().corners {
        Corners::Round => radius,
        Corners::Cut => 0.0,
    }
}

/// Which version of every token the app draws with.
pub fn appearance() -> Appearance {
    if LIGHT.load(Ordering::Relaxed) { Appearance::Light } else { Appearance::Dark }
}

pub fn set_appearance(appearance: Appearance) {
    LIGHT.store(appearance == Appearance::Light, Ordering::Relaxed);
}

/// Where a theme's value sits among a token's: each theme's dark, then its light.
fn slot(theme: usize, appearance: Appearance) -> usize {
    theme * 2 + usize::from(appearance == Appearance::Light)
}

/// A colour token: its value in each theme and appearance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Token {
    values: [Color; THEME_COUNT * 2],
}

impl Token {
    /// The value in the current theme and appearance.
    pub fn get(self) -> Color {
        self.at(appearance())
    }

    /// The value in the current theme, in `appearance`.
    pub fn at(self, appearance: Appearance) -> Color {
        self.values[slot(THEME.load(Ordering::Relaxed), appearance)]
    }

    /// The value in the theme `id` names, in `appearance`.
    pub fn of(self, id: &str, appearance: Appearance) -> Color {
        self.values[slot(THEMES.iter().position(|theme| theme.id == id).expect("a theme"), appearance)]
    }
}

/// A shadow token: its layers in each theme and appearance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadows {
    values: [&'static [Shadow]; THEME_COUNT * 2],
}

impl Shadows {
    /// The layers in the current theme and appearance.
    pub fn get(self) -> &'static [Shadow] {
        self.values[slot(THEME.load(Ordering::Relaxed), appearance())]
    }

    /// The layers in the theme `id` names, in `appearance`.
    pub fn of(self, id: &str, appearance: Appearance) -> &'static [Shadow] {
        self.values[slot(THEMES.iter().position(|theme| theme.id == id).expect("a theme"), appearance)]
    }
}

// Generated colour values are hex as written in tokens.json, and the docs are the tokens' usage
// notes, which name components (IconTile) as the design system writes them.
#[allow(clippy::unreadable_literal, clippy::doc_markdown)]
mod generated {
    use super::{
        AgentCards, Buttons, Choices, Color, Corners, CurrentLine, EmptyAreas, Icons, Look, Menus, Meters, Selection,
        SettingRows, Shadow, Shadows, Surfaces, Switch, Tabs, TextStyle, TitleBar, Token, Tone,
    };
    include!(concat!(env!("OUT_DIR"), "/tokens.rs"));
}
pub use generated::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_colours_come_from_the_tokens() {
        assert_eq!(color::BLUE_VIVID.of("saturn", Appearance::Dark), Color { rgb: 0x001c_69e3, alpha: 1.0 });
        assert_eq!(color::BLUE_VIVID.of("saturn", Appearance::Light), color::BLUE_VIVID.of("saturn", Appearance::Dark));
        assert_eq!(color::STATUS_WORKING.of("saturn", Appearance::Dark), color::BLUE.of("saturn", Appearance::Dark));
    }

    #[test]
    fn light_mode_has_its_own_neutrals_and_signals() {
        assert_ne!(color::SURFACE.of("saturn", Appearance::Light), color::SURFACE.of("saturn", Appearance::Dark));
        // Amber text darkens in light mode; the amber fill stays, always with dark text on it.
        assert_ne!(color::AMBER_TEXT.of("saturn", Appearance::Light), color::AMBER.of("saturn", Appearance::Light));
        assert_eq!(color::AMBER.of("saturn", Appearance::Light), color::AMBER.of("saturn", Appearance::Dark));
        // Needs you is orange, darkened in light mode; your turn is the terminal's green.
        assert_eq!(
            color::STATUS_NEEDS_YOU.of("saturn", Appearance::Light),
            color::ORANGE.of("saturn", Appearance::Light)
        );
        assert_ne!(color::ORANGE.of("saturn", Appearance::Light), color::ORANGE.of("saturn", Appearance::Dark));
        assert_eq!(
            color::STATUS_YOUR_TURN.of("saturn", Appearance::Dark),
            color::TERM_GREEN.of("saturn", Appearance::Dark)
        );
        assert_eq!(
            color::INK_ON_ACCENT.of("saturn", Appearance::Light),
            color::INK_ON_ACCENT.of("saturn", Appearance::Dark)
        );
    }

    /// Every row of the nesting table in the design system's README: inner = outer − padding.
    #[test]
    fn nested_radii_follow_the_padding_rule() {
        // The macOS window corner (about 16) and the 6px gap give panels their radius.
        let window = 16.0;
        let nestings = [
            (window, space::SPACE_2, radius::RADIUS_MD),
            (radius::RADIUS_MD, space::SPACE_1, radius::RADIUS_SM),
            (radius::RADIUS_LG, space::SPACE_3, radius::RADIUS_SM),
        ];
        for (outer, padding, inner) in nestings {
            assert!((outer - padding - inner).abs() < f32::EPSILON, "{outer} − {padding} ≠ {inner}");
        }
    }

    #[test]
    fn glows_have_a_ring_and_a_soft_halo() {
        for glow in [
            shadow::GLOW_NEEDS_YOU.of("saturn", Appearance::Dark),
            shadow::GLOW_NEEDS_YOU.of("saturn", Appearance::Light),
        ] {
            assert_eq!(glow.len(), 2);
            assert!((glow[0].spread - 1.0).abs() < f32::EPSILON);
            assert!(!glow[0].inset);
        }
        assert!((shadow::GLOW_NEEDS_YOU.of("saturn", Appearance::Dark)[1].blur - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fonts_are_the_first_family_of_each_stack() {
        assert_eq!(font::SANS, "Urbanist");
        assert_eq!(font::MONO, "JetBrains Mono");
        assert_eq!(font::SANS_CYBERDECK, "Rajdhani");
    }

    #[test]
    fn themes_are_described_in_full_and_the_first_is_saturns() {
        assert_eq!(THEMES[0].id, "saturn");
        set_theme("no such theme");
        assert_eq!(look().id, DEFAULT_THEME);
        assert_eq!(DEFAULT_THEME, "cyberdeck");
        let cyberdeck = THEMES.iter().find(|theme| theme.id == "cyberdeck").expect("cyberdeck");
        assert_eq!(cyberdeck.sans, "Rajdhani");
        assert_eq!(cyberdeck.corners, Corners::Cut);
        assert!(cyberdeck.card_gap < THEMES[0].card_gap);
        let ids: std::collections::HashSet<_> = THEMES.iter().map(|theme| theme.id).collect();
        assert_eq!(ids.len(), THEMES.len(), "theme ids are unique");
    }

    #[test]
    fn the_type_scale_comes_from_the_tokens() {
        assert!((text::LABEL.size - 13.0).abs() < f32::EPSILON);
        assert_eq!(text::LABEL.weight, 500);
        const { assert!(text::META.size < text::BODY.size) };
    }

    #[test]
    fn cyberdeck_is_monochrome_but_its_signals_are_saturated() {
        // Its own neutrals, with no blue in them.
        assert_ne!(color::SURFACE.of("cyberdeck", Appearance::Dark), color::SURFACE.of("saturn", Appearance::Dark));
        let rgb = color::SURFACE.of("cyberdeck", Appearance::Dark).rgb;
        assert!((rgb >> 16) & 0xff <= (rgb & 0xff) + 1 && (rgb & 0xff) <= ((rgb >> 16) & 0xff) + 1);
        // Its signals are more saturated than Saturn's soft ones on the dark ground.
        let saturation = |rgb: u32| {
            let channels = [(rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff];
            let (most, least) =
                (*channels.iter().max().expect("a channel"), *channels.iter().min().expect("a channel"));
            f64::from(most - least) / f64::from(most.max(1))
        };
        for signal in [color::BLUE, color::ORANGE, color::RED, color::TERM_GREEN] {
            let neon = saturation(signal.of("cyberdeck", Appearance::Dark).rgb);
            assert!(neon > saturation(signal.of("saturn", Appearance::Dark).rgb), "{signal:?}");
            assert!(neon > 0.75, "{signal:?}");
        }
    }
}
