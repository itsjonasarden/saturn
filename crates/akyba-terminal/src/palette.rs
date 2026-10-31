//! Terminal colours from the design system: the ANSI palette is the `term-*` tokens, so programs'
//! output matches Akyba's chrome.

use akyba_design::color;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[expect(clippy::cast_possible_truncation, reason = "taking the bytes of 0xRRGGBB")]
fn rgb(token: akyba_design::Token) -> Rgb {
    let rgb = token.get().rgb;
    Rgb { r: (rgb >> 16) as u8, g: (rgb >> 8) as u8, b: rgb as u8 }
}

// Each is read when a frame is drawn, so a change of appearance recolours every terminal.

pub fn foreground() -> Rgb {
    rgb(color::TERM_WHITE)
}

pub fn background() -> Rgb {
    rgb(color::SURFACE)
}

pub fn cursor() -> Rgb {
    rgb(color::INK)
}

pub fn selection() -> Rgb {
    rgb(color::BLUE_VIVID)
}

/// How strongly selected text is tinted: less on a light background, where blue is louder.
pub fn selection_opacity() -> f32 {
    match akyba_design::appearance() {
        akyba_design::Appearance::Dark => 0.45,
        akyba_design::Appearance::Light => 0.25,
    }
}

/// ANSI 0–15: the eight `term-*` colours, then their bright versions.
fn ansi(index: u8) -> Rgb {
    rgb(match index {
        0 => color::TERM_BLACK,
        1 | 9 => color::TERM_RED,
        2 | 10 => color::TERM_GREEN,
        3 | 11 => color::TERM_YELLOW,
        4 | 12 => color::TERM_BLUE,
        5 | 13 => color::TERM_MAGENTA,
        6 | 14 => color::TERM_CYAN,
        7 => color::TERM_WHITE,
        8 => color::INK_FAINT,
        _ => color::INK,
    })
}

/// The xterm 256-colour palette: ANSI, a 6×6×6 cube, then 24 greys.
fn indexed(index: u8) -> Rgb {
    match index {
        0..=15 => ansi(index),
        16..=231 => {
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            let cube = index - 16;
            Rgb { r: level(cube / 36), g: level((cube / 6) % 6), b: level(cube % 6) }
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            Rgb { r: grey, g: grey, b: grey }
        }
    }
}

fn dim(color: Rgb) -> Rgb {
    let scale = |channel: u8| u8::try_from(u16::from(channel) * 2 / 3).unwrap_or(channel);
    Rgb { r: scale(color.r), g: scale(color.g), b: scale(color.b) }
}

/// Resolves a cell colour. Colours a program set itself (OSC 4/10/11) win over the palette.
pub fn resolve(color: Color, overrides: &Colors) -> Rgb {
    match color {
        Color::Spec(rgb) => rgb,
        Color::Indexed(index) => overrides[usize::from(index)].unwrap_or_else(|| indexed(index)),
        Color::Named(name) => overrides[name].unwrap_or_else(|| named(name)),
    }
}

/// The colour a program asks for by number (OSC 4, 10, 11, 12): 0–255 the palette, 256 the text,
/// 257 the background, 258 the cursor. Programs ask for the background to tell a light terminal from
/// a dark one, and choose their colours by it.
pub fn requested(index: usize, overrides: &Colors) -> Rgb {
    match u8::try_from(index) {
        Ok(index) => resolve(Color::Indexed(index), overrides),
        Err(_) => resolve(
            Color::Named(match index {
                256 => NamedColor::Foreground,
                257 => NamedColor::Background,
                _ => NamedColor::Cursor,
            }),
            overrides,
        ),
    }
}

/// Whether the terminal is dark now, for programs that ask (`COLORFGBG`, colour scheme reports).
pub fn is_dark() -> bool {
    akyba_design::appearance() == akyba_design::Appearance::Dark
}

fn named(name: NamedColor) -> Rgb {
    match name {
        NamedColor::Foreground | NamedColor::BrightForeground => foreground(),
        NamedColor::Background => background(),
        NamedColor::Cursor => cursor(),
        NamedColor::DimForeground => dim(foreground()),
        NamedColor::DimBlack => dim(ansi(0)),
        NamedColor::DimRed => dim(ansi(1)),
        NamedColor::DimGreen => dim(ansi(2)),
        NamedColor::DimYellow => dim(ansi(3)),
        NamedColor::DimBlue => dim(ansi(4)),
        NamedColor::DimMagenta => dim(ansi(5)),
        NamedColor::DimCyan => dim(ansi(6)),
        NamedColor::DimWhite => dim(ansi(7)),
        #[expect(clippy::cast_possible_truncation, reason = "the named colours below 16 are the ANSI ones")]
        other => ansi((other as usize).min(15) as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_for_the_background_the_text_and_the_palette() {
        let colors = Colors::default();
        assert_eq!(requested(257, &colors), background());
        assert_eq!(requested(256, &colors), foreground());
        assert_eq!(requested(1, &colors), ansi(1));
        assert_eq!(requested(196, &colors), indexed(196));
    }

    #[test]
    fn ansi_colours_are_the_design_tokens() {
        assert_eq!(indexed(1), rgb(color::TERM_RED));
        assert_eq!(indexed(15), rgb(color::INK));
    }

    #[test]
    fn the_cube_and_greys_match_xterm() {
        assert_eq!(indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(indexed(255), Rgb { r: 238, g: 238, b: 238 });
    }
}
