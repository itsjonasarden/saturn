//! The design system's rules, checked over the app's code: sizes, spacing and corners come from
//! the tokens, and no view asks which theme is on. `chrome.rs` (the components) and `theme.rs`
//! (where tokens meet GPUI) draw the shapes themselves, so they may write a pixel where a shape
//! needs one. Anywhere else, a value off the scale says why on its line, after `// off-scale:`.

use std::path::Path;

/// Every line of the app's code, with its file and line number.
fn lines() -> Vec<(String, usize, String)> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut lines = Vec::new();
    for entry in std::fs::read_dir(&source).expect("the app's source") {
        let path = entry.expect("a source file").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let name = path.file_name().expect("a file name").to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in text.lines().enumerate() {
            lines.push((name.clone(), index + 1, line.to_owned()));
        }
    }
    lines
}

/// Whether `line` writes a number straight into `call`, as in `.text_size(px(13.))`.
fn writes_a_number(line: &str, call: &str) -> bool {
    writes_a_number_then(line, call, "))")
}

/// Whether `line` writes a number into `call` that `end` follows, as in `overlay(0.3)`.
fn writes_a_number_then(line: &str, call: &str, end: &str) -> bool {
    line.match_indices(call).any(|(at, _)| {
        let rest = &line[at + call.len()..];
        let rest = rest.strip_prefix('-').unwrap_or(rest);
        let number: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        !number.is_empty() && rest[number.len()..].starts_with(end)
    })
}

fn breaks(rule: impl Fn(&str, &str) -> bool) -> Vec<String> {
    lines()
        .into_iter()
        .filter(|(file, _, line)| {
            let code = line.split("//").next().unwrap_or_default();
            !line.contains("// off-scale:") && rule(file, code)
        })
        .map(|(file, number, line)| format!("{file}:{number}: {}", line.trim()))
        .collect()
}

fn draws_shapes(file: &str) -> bool {
    file == "chrome.rs" || file == "theme.rs"
}

#[test]
fn text_sizes_come_from_the_type_scale() {
    let found = breaks(|_, line| writes_a_number(line, ".text_size(px("));
    assert!(found.is_empty(), "write `.text_scale(text::…)` instead:\n{}", found.join("\n"));
}

#[test]
fn spacing_comes_from_the_spacing_scale() {
    let calls = ["p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "ml", "mr", "gap"];
    let found = breaks(|file, line| {
        !draws_shapes(file) && calls.iter().any(|call| writes_a_number(line, &format!(".{call}(px(")))
    });
    assert!(found.is_empty(), "write `len(space::…)`, or say why on the line:\n{}", found.join("\n"));
}

#[test]
fn corners_come_from_the_theme() {
    let found = breaks(|file, line| {
        !draws_shapes(file) && (writes_a_number(line, ".rounded(px(") || line.contains(".rounded_full()"))
    });
    assert!(found.is_empty(), "write `theme::corner(radius::…)`, or `chrome::pill` for a dot:\n{}", found.join("\n"));
}

#[test]
fn line_heights_come_from_the_type_scale() {
    let found = breaks(|file, line| !draws_shapes(file) && writes_a_number(line, ".line_height(px("));
    assert!(found.is_empty(), "write `px(text::….line_height)`:\n{}", found.join("\n"));
}

#[test]
fn icons_come_in_the_icon_sizes() {
    // `icon(name, 13., …)`: a number straight after the name.
    let found = breaks(|_, line| {
        line.match_indices("icon(").any(|(at, _)| {
            let rest = &line[at..];
            rest.split_once(", ").is_some_and(|(_, size)| size.starts_with(|c: char| c.is_ascii_digit()))
        })
    });
    assert!(found.is_empty(), "write `icon::SM` (or XS, MD, LG):\n{}", found.join("\n"));
}

#[test]
fn fills_and_strokes_come_in_the_tones() {
    let found = breaks(|file, line| file != "theme.rs" && writes_a_number_then(line, "overlay(", ")"));
    assert!(found.is_empty(), "write `theme::tone(tone::…)`:\n{}", found.join("\n"));
}

#[test]
fn colours_come_from_the_tokens() {
    // White, black or a colour written out: a view says which token it means instead.
    let raw = ["gpui_kit::white()", "gpui_kit::black()", "gpui_kit::hsla(", "gpui_kit::rgb(", "rgba(0x"];
    let found = breaks(|file, line| !draws_shapes(file) && raw.iter().any(|colour| line.contains(colour)));
    assert!(found.is_empty(), "write `theme::color(color::…)` or `theme::tone(tone::…)`:\n{}", found.join("\n"));
}

#[test]
fn no_view_asks_which_theme_is_on() {
    // A component reads the theme's description (`chrome::look()`); it never compares names.
    let found =
        breaks(|_, line| line.contains("look().id") || line.contains("\"cyberdeck\"") || line.contains("cyberdeck()"));
    assert!(found.is_empty(), "ask `chrome::look()` for the version to draw:\n{}", found.join("\n"));
}
