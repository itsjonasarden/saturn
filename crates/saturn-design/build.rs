//! Compiles design/tokens.json (the one source of every design value, the same file as the Saturn
//! Design System artifact) into Rust constants, so a token changes in one place.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::{env, fs};

use serde_json::Value;

/// The keys a colour or shadow has for each theme in design/tokens.json, in the order `Token`
/// holds them: each theme's dark, then its light. The first theme's are plain `dark` and `light`.
fn variants(themes: &[Value]) -> Vec<String> {
    themes
        .iter()
        .enumerate()
        .flat_map(|(index, theme)| {
            let id = theme["id"].as_str().expect("a theme id");
            ["dark", "light"]
                .map(|appearance| if index == 0 { appearance.to_owned() } else { format!("{id}-{appearance}") })
        })
        .collect()
}

/// `segments` as the enum variant `Segments`.
fn variant(value: &Value) -> String {
    let word = value.as_str().expect("a theme option");
    word[..1].to_uppercase() + &word[1..]
}

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

/// A token's value in `theme` (`dark`, `light`, `cyberdeck-dark`…): a plain string is every
/// theme's. A theme's missing value borrows the first theme's in the same appearance, and a missing
/// `light` borrows `dark`.
fn themed<'a>(value: &'a Value, theme: &str) -> &'a str {
    let appearance = if theme.ends_with("light") { "light" } else { "dark" };
    match value {
        Value::String(text) => text,
        Value::Object(themes) => themes
            .get(theme)
            .or_else(|| themes.get(appearance))
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

    let themes = tokens["themes"]["themes"].as_array().expect("themes");
    let variants = variants(themes);
    let mut out = String::from("// Generated from design/tokens.json by crates/saturn-design/build.rs.\n\n");
    writeln!(out, "/// How many themes there are.\npub const THEME_COUNT: usize = {};\n", themes.len()).ok();
    out.push_str("pub mod color {\n    use super::{Color, Token};\n");
    for token in tokens["color"]["tokens"].as_array().expect("colour tokens") {
        let values: Vec<String> = variants
            .iter()
            .map(|theme| {
                let (rgb, alpha) = resolve_color(&colors, themed(&token["value"], theme), theme);
                format!("Color {{ rgb: 0x{rgb:06x}, alpha: {alpha:?} }}")
            })
            .collect();
        let usage = token["usage"].as_str().unwrap_or_default().replace('`', "");
        writeln!(
            out,
            "    /// {usage}\n    pub const {}: Token = Token {{ values: [{}] }};",
            const_name(token["name"].as_str().expect("name")),
            values.join(", ")
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
        let values: Vec<String> = variants
            .iter()
            .map(|theme| format!("&[{}]", shadow_layers(themed(&token["value"], theme)).join(", ")))
            .collect();
        let usage = token["usage"].as_str().unwrap_or_default().replace('`', "");
        writeln!(
            out,
            "    /// {usage}\n    pub const {}: Shadows = Shadows {{ values: [{}] }};",
            const_name(token["name"].as_str().expect("name")),
            values.join(", ")
        )
        .ok();
    }
    out.push_str("}\n\n");

    out.push_str("pub mod font {\n");
    for (key, stack) in tokens["type"]["families"].as_object().expect("families") {
        let first = stack.as_str().expect("stack").split(',').next().expect("a family").trim().trim_matches('"');
        writeln!(out, "    pub const {}: &str = {first:?};", const_name(key)).ok();
    }
    out.push_str("}\n\n");

    out.push_str(&text_styles(&tokens));
    out.push_str(&looks(&tokens, themes));

    fs::write(PathBuf::from(env::var("OUT_DIR").expect("out dir")).join("tokens.rs"), out).expect("write tokens.rs");
}

/// The type scale: every style of every group, by its name.
fn text_styles(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("pub mod text {\n    use super::TextStyle;\n");
    for group in tokens["type"]["groups"].as_array().expect("type groups") {
        for style in group["styles"].as_array().expect("styles") {
            let usage = style["usage"].as_str().unwrap_or_default().replace('`', "");
            writeln!(
                out,
                "    /// {usage}\n    pub const {}: TextStyle = TextStyle {{ size: {:?}, line_height: {:?}, weight: {} }};",
                const_name(style["name"].as_str().expect("name")),
                px(style["fontSize"].as_str().expect("size")),
                px(style["lineHeight"].as_str().expect("line height")),
                style["fontWeight"].as_u64().expect("weight")
            )
            .ok();
        }
    }
    out.push_str("}\n\n");

    out
}

/// The themes, each a description its components read.
fn looks(tokens: &Value, themes: &[Value]) -> String {
    let mut out = String::new();
    let spacing: HashMap<&str, f32> = tokens["spacing"]["tokens"]
        .as_array()
        .expect("spacing")
        .iter()
        .map(|token| (token["name"].as_str().expect("name"), px(token["value"].as_str().expect("value"))))
        .collect();
    let mut looks = Vec::new();
    for theme in themes {
        let text = |key: &str| theme[key].as_str().unwrap_or_else(|| panic!("a theme's {key}"));
        let number = |key: &str| theme[key].as_f64().unwrap_or_else(|| panic!("a theme's {key}"));
        let options: Vec<String> = [
            ("corners", "Corners"),
            ("surfaces", "Surfaces"),
            ("titleBar", "TitleBar"),
            ("tabs", "Tabs"),
            ("selection", "Selection"),
            ("choices", "Choices"),
            ("switch", "Switch"),
            ("buttons", "Buttons"),
            ("menus", "Menus"),
            ("settingRows", "SettingRows"),
            ("meters", "Meters"),
            ("agentCards", "AgentCards"),
            ("currentLine", "CurrentLine"),
        ]
        .iter()
        .map(|(key, kind)| {
            let field = key.chars().fold(String::new(), |mut field, c| {
                if c.is_uppercase() {
                    field.push('_');
                }
                field.push(c.to_ascii_lowercase());
                field
            });
            format!("{field}: {kind}::{}", variant(&theme[*key]))
        })
        .collect();
        looks.push(format!(
            "Look {{ id: {:?}, name: {:?}, description: {:?}, sans: font::{}, panel_cut: {:?}, control_cut: {:?}, card_gap: {:?}, focus: color::{}, {} }}",
            text("id"),
            text("name"),
            text("description"),
            const_name(text("font")),
            number("panelCut"),
            number("controlCut"),
            spacing[text("cardGap")],
            const_name(text("focus")),
            options.join(", ")
        ));
    }
    writeln!(
        out,
        "/// Every theme, the default first.\npub const THEMES: [Look; THEME_COUNT] = [{}];",
        looks.join(", ")
    )
    .ok();

    out
}
