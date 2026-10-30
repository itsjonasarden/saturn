//! Compiles design/tokens.json (the one source of every design value, the same file as the Saturn
//! Design System artifact) into Rust constants, so a token changes in one place.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::{env, fs};

use serde_json::Value;

/// The themes in design/tokens.json, in the order `Token` holds them: Saturn's own look in dark
/// and light, then the Cyberdeck theme's.
const THEMES: [&str; 4] = ["dark", "light", "cyberdeck-dark", "cyberdeck-light"];

fn const_name(name: &str) -> String {
    name.replace(['-', '.'], "_").to_uppercase()
}

/// `#rrggbb`, `#rgb` or `rgba(r, g, b, a)` as (0xRRGGBB, alpha).
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "colour channels in tokens.json are 0-255")]
fn parse_color(value: &str) -> (u32, f32) {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let hex = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_owned() };
        return (u32::from_str_radix(&hex[..6], 16).expect("hex colour"), 1.0);
    }
    let inner = value.trim_start_matches("rgba(").trim_start_matches("rgb(").trim_end_matches(')');
    let parts: Vec<f32> = inner.split(',').map(|p| p.trim().parse().expect("colour channel")).collect();
    let rgb = ((parts[0] as u32) << 16) | ((parts[1] as u32) << 8) | parts[2] as u32;
    (rgb, parts.get(3).copied().unwrap_or(1.0))
}

/// A token's value in `theme`: a plain string is every theme's. A missing Cyberdeck value borrows
/// Saturn's in the same appearance, and a missing `light` borrows `dark`.
fn themed<'a>(value: &'a Value, theme: &str) -> &'a str {
    match value {
        Value::String(text) => text,
        Value::Object(themes) => themes
            .get(theme)
            .or_else(|| themes.get(theme.trim_start_matches("cyberdeck-")))
            .or_else(|| themes.get("dark"))
            .and_then(Value::as_str)
            .expect("a theme value"),
        _ => panic!("a token value is a string or a map of themes"),
    }
}

/// Follows `{alias}` references to a literal colour, in `theme`.
fn resolve_color(colors: &HashMap<String, Value>, value: &str, theme: &str) -> (u32, f32) {
    let mut value = value;
    while let Some(alias) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        value = themed(colors.get(alias).unwrap_or_else(|| panic!("unknown colour alias {alias}")), theme);
    }
    parse_color(value)
}

fn px(value: &str) -> f32 {
    value.trim().trim_end_matches("px").parse().expect("a px length")
}

/// One CSS box-shadow layer: `[inset] x y blur [spread] colour`.
fn parse_shadow_layer(layer: &str) -> String {
    let (inset, layer) = match layer.strip_prefix("inset ") {
        Some(rest) => (true, rest.trim_start()),
        None => (false, layer),
    };
    let colour_at = layer.find(['#', 'r']).expect("shadow colour");
    let lengths: Vec<f32> = layer[..colour_at].split_whitespace().map(px).collect();
    let (rgb, alpha) = parse_color(&layer[colour_at..]);
    let spread = lengths.get(3).copied().unwrap_or(0.0);
    format!(
        "Shadow {{ x: {:?}, y: {:?}, blur: {:?}, spread: {:?}, inset: {inset}, color: Color {{ rgb: 0x{rgb:06x}, alpha: {alpha:?} }} }}",
        lengths[0], lengths[1], lengths[2], spread
    )
}

/// Splits a box-shadow list on the commas between layers, not those inside `rgba(...)`.
fn shadow_layers(value: &str) -> Vec<String> {
    let mut layers = vec![String::new()];
    let mut depth = 0;
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                layers.push(String::new());
                continue;
            }
            _ => {}
        }
        layers.last_mut().expect("a layer").push(c);
    }
    layers.into_iter().map(|layer| parse_shadow_layer(layer.trim())).collect()
}

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../../design/tokens.json");
    println!("cargo::rerun-if-changed={}", root.display());
    let tokens: Value =
        serde_json::from_str(&fs::read_to_string(&root).expect("design/tokens.json")).expect("valid JSON");

    let colors: HashMap<String, Value> = tokens["color"]["tokens"]
        .as_array()
        .expect("colour tokens")
        .iter()
        .map(|t| (t["name"].as_str().expect("name").to_owned(), t["value"].clone()))
        .collect();

    let mut out = String::from("// Generated from design/tokens.json by crates/saturn-design/build.rs.\n\n");
    out.push_str("pub mod color {\n    use super::{Color, Token};\n");
    for token in tokens["color"]["tokens"].as_array().expect("colour tokens") {
        let [dark, light, cyberdeck_dark, cyberdeck_light] = THEMES.map(|theme| {
            let (rgb, alpha) = resolve_color(&colors, themed(&token["value"], theme), theme);
            format!("Color {{ rgb: 0x{rgb:06x}, alpha: {alpha:?} }}")
        });
        let usage = token["usage"].as_str().unwrap_or_default().replace('`', "");
        writeln!(
            out,
            "    /// {usage}\n    pub const {}: Token = Token {{ dark: {dark}, light: {light}, cyberdeck_dark: {cyberdeck_dark}, cyberdeck_light: {cyberdeck_light} }};",
            const_name(token["name"].as_str().expect("name"))
        )
        .ok();
    }
    out.push_str("}\n\n");

    for (family, module) in [("spacing", "space"), ("radius", "radius")] {
        writeln!(out, "pub mod {module} {{").ok();
        for token in tokens[family]["tokens"].as_array().expect("tokens") {
            let usage = token["usage"].as_str().unwrap_or_default().replace('`', "");
            writeln!(
                out,
                "    /// {usage}\n    pub const {}: f32 = {:?};",
                const_name(token["name"].as_str().expect("name")),
                px(token["value"].as_str().expect("value"))
            )
            .ok();
        }
        out.push_str("}\n\n");
    }

    out.push_str("pub mod shadow {\n    use super::{Color, Shadow, Shadows};\n");
    for token in tokens["shadow"]["tokens"].as_array().expect("shadow tokens") {
        let [dark, light, cyberdeck_dark, cyberdeck_light] =
            THEMES.map(|theme| shadow_layers(themed(&token["value"], theme)).join(", "));
        let usage = token["usage"].as_str().unwrap_or_default().replace('`', "");
        writeln!(
            out,
            "    /// {usage}\n    pub const {}: Shadows = Shadows {{ dark: &[{dark}], light: &[{light}], cyberdeck_dark: &[{cyberdeck_dark}], cyberdeck_light: &[{cyberdeck_light}] }};",
            const_name(token["name"].as_str().expect("name"))
        )
        .ok();
    }
    out.push_str("}\n\n");

    out.push_str("pub mod font {\n");
    for (key, stack) in tokens["type"]["families"].as_object().expect("families") {
        let first = stack.as_str().expect("stack").split(',').next().expect("a family").trim().trim_matches('"');
        writeln!(out, "    pub const {}: &str = {first:?};", const_name(key)).ok();
    }
    out.push_str("}\n");

    fs::write(PathBuf::from(env::var("OUT_DIR").expect("out dir")).join("tokens.rs"), out).expect("write tokens.rs");
}
