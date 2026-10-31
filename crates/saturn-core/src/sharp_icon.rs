//! A sharp version of a Lucide icon, for a theme with hard corners: the same drawing with square
//! line ends and mitred joins, its small rounded corners made square (an arc a quarter turn round
//! of radius 3 or less becomes the corner it rounded), its rounded rectangles square, and its small
//! dots little squares. Larger circles and curves stay as they are: a clock is still round.

/// Arcs this small or smaller are a rounded corner, not a curve the icon is drawn with.
const CORNER: f32 = 3.0;
/// Circles this small or smaller are a dot (a git node), drawn as a square.
const DOT: f32 = 3.0;

/// The sharp version of `svg`.
pub fn sharpen(svg: &str) -> String {
    let svg = svg
        .replace("stroke-linecap=\"round\"", "stroke-linecap=\"square\"")
        .replace("stroke-linejoin=\"round\"", "stroke-linejoin=\"miter\"");
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg.as_str();
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('>').map_or(rest.len(), |end| start + end + 1);
        out.push_str(&element(&rest[start..end]));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// One tag, sharpened.
fn element(tag: &str) -> String {
    if tag.starts_with("<path") {
        if let Some(d) = attribute(tag, "d") {
            return tag.replacen(&format!("d=\"{d}\""), &format!("d=\"{}\"", sharpen_path(&d)), 1);
        }
    } else if tag.starts_with("<rect") {
        return without(&without(tag, "rx"), "ry");
    } else if tag.starts_with("<circle") {
        let number = |name| attribute(tag, name).and_then(|value| value.parse::<f32>().ok());
        if let (Some(x), Some(y), Some(r)) = (number("cx"), number("cy"), number("r"))
            && r <= DOT
        {
            let close = if tag.trim_end().ends_with("/>") { "/>" } else { ">" };
            return format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {close}",
                x - r,
                y - r,
                r * 2.0,
                r * 2.0
            );
        }
    }
    tag.to_owned()
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

/// `tag` without the attribute `name`.
fn without(tag: &str, name: &str) -> String {
    match attribute(tag, name) {
        Some(value) => tag.replacen(&format!(" {name}=\"{value}\""), "", 1),
        None => tag.to_owned(),
    }
}

/// A path's commands, read one by one: arc flags are single digits, which Lucide writes packed
/// together (`0 012 2`).
struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip_separators(&mut self) {
        while self.at < self.text.len() && matches!(self.text[self.at], b' ' | b',' | b'\n' | b'\t' | b'\r') {
            self.at += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let c = *self.text.get(self.at)?;
        c.is_ascii_alphabetic().then(|| {
            self.at += 1;
            c
        })
    }

    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.at;
        if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        let mut dot = false;
        while let Some(&c) = self.text.get(self.at) {
            if c.is_ascii_digit() || (c == b'.' && !dot) {
                dot |= c == b'.';
                self.at += 1;
            } else if c == b'e' || c == b'E' {
                self.at += 1;
                if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.text[start..self.at]).ok()?.parse().ok()
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let c = *self.text.get(self.at)?;
        self.at += 1;
        matches!(c, b'0' | b'1').then_some(c == b'1')
    }

    fn numbers(&mut self, count: usize) -> Option<Vec<f32>> {
        (0..count).map(|_| self.number()).collect()
    }
}

/// How many numbers each command takes.
fn arity(command: u8) -> usize {
    match command.to_ascii_lowercase() {
        b'h' | b'v' => 1,
        b'm' | b'l' | b't' => 2,
        b's' | b'q' => 4,
        b'c' => 6,
        b'a' => 7,
        _ => 0,
    }
}

fn number(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".to_owned() } else { text.to_owned() }
}

/// The path with its small quarter arcs, the rounded corners, made square.
fn sharpen_path(d: &str) -> String {
    let mut reader = Reader { text: d.as_bytes(), at: 0 };
    let mut out: Vec<String> = Vec::new();
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    let (mut start_x, mut start_y) = (0.0_f32, 0.0_f32);
    let mut current = None;
    loop {
        reader.skip_separators();
        if reader.at >= reader.text.len() {
            break;
        }
        let command = match reader.command() {
            Some(command) => command,
            // More numbers carry on the last command (after a move, as lines).
            None => match current {
                Some(b'm') => b'l',
                Some(b'M') => b'L',
                Some(command) => command,
                None => return d.to_owned(),
            },
        };
        current = Some(command);
        let relative = command.is_ascii_lowercase();
        if command.eq_ignore_ascii_case(&b'z') {
            out.push((command as char).to_string());
            (x, y) = (start_x, start_y);
            continue;
        }
        let values = if command.eq_ignore_ascii_case(&b'a') {
            let Some(mut arc) = reader.numbers(3) else { return d.to_owned() };
            let (Some(large), Some(sweep)) = (reader.flag(), reader.flag()) else { return d.to_owned() };
            let Some(end) = reader.numbers(2) else { return d.to_owned() };
            arc.extend([f32::from(u8::from(large)), f32::from(u8::from(sweep))]);
            arc.extend(end);
            arc
        } else {
            match reader.numbers(arity(command)) {
                Some(values) => values,
                None => return d.to_owned(),
            }
        };
        let (to_x, to_y) = match command.to_ascii_lowercase() {
            b'h' => (if relative { x + values[0] } else { values[0] }, y),
            b'v' => (x, if relative { y + values[0] } else { values[0] }),
            _ => {
                let (last_x, last_y) = (values[values.len() - 2], values[values.len() - 1]);
                if relative { (x + last_x, y + last_y) } else { (last_x, last_y) }
            }
        };
        if command.eq_ignore_ascii_case(&b'a') {
            let (dx, dy) = (to_x - x, to_y - y);
            let quarter = (values[0] - values[1]).abs() < 0.01
                && (dx.abs() - values[0]).abs() < 0.01
                && (dy.abs() - values[0]).abs() < 0.01;
            if values[0] <= CORNER && quarter {
                // The corner the arc rounded: clockwise, a run along x then y when the two move
                // the same way; anticlockwise, the other way round.
                let along_x_first = (dx * dy > 0.0) == (values[4] > 0.5);
                let (corner_x, corner_y) = if along_x_first { (to_x, y) } else { (x, to_y) };
                out.push(format!("L{} {}L{} {}", number(corner_x), number(corner_y), number(to_x), number(to_y)));
                (x, y) = (to_x, to_y);
                continue;
            }
        }
        out.push(format!(
            "{}{}",
            command as char,
            values.iter().map(|value| number(*value)).collect::<Vec<_>>().join(" ")
        ));
        if command.eq_ignore_ascii_case(&b'm') {
            (start_x, start_y) = (to_x, to_y);
        }
        (x, y) = (to_x, to_y);
    }
    out.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ends_are_square_and_joins_mitred() {
        let svg = r#"<svg stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h16" /></svg>"#;
        let sharp = sharpen(svg);
        assert!(sharp.contains(r#"stroke-linecap="square""#));
        assert!(sharp.contains(r#"stroke-linejoin="miter""#));
        assert!(sharp.contains(r#"d="M4 12h16""#));
    }

    #[test]
    fn rounded_corners_become_the_corners_they_rounded() {
        // A box from (4,4) to (20,20) with corners of radius 2, as Lucide draws one.
        let sharp = sharpen_path("M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z");
        assert!(!sharp.contains('a'), "no arcs left: {sharp}");
        for corner in ["L20 4", "L20 20", "L4 20", "L4 4"] {
            assert!(sharp.contains(corner), "{corner} in {sharp}");
        }
        // Anticlockwise, with the flags packed together.
        let sharp = sharpen_path("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2");
        assert!(sharp.contains("L22 20") && sharp.contains("L22 6"), "{sharp}");
        assert_eq!(sharpen_path("M2 18a2 2 0 002 2"), "M2 18L2 20L4 20");
    }

    #[test]
    fn curves_and_circles_stay_round_but_dots_go_square() {
        // A half circle of radius 9 is part of the drawing.
        let arc = "M3 12a9 9 0 0 1 18 0";
        assert!(sharpen_path(arc).contains('a'));
        let sharp = sharpen(r#"<circle cx="12" cy="12" r="10" /><circle cx="6" cy="6" r="2" />"#);
        assert!(sharp.contains(r#"<circle cx="12" cy="12" r="10" />"#));
        assert!(sharp.contains(r#"<rect x="4" y="4" width="4" height="4" />"#));
        let rect = sharpen(r#"<rect width="18" height="18" x="3" y="3" rx="2" />"#);
        assert_eq!(rect, r#"<rect width="18" height="18" x="3" y="3" />"#);
    }

    #[test]
    fn every_icon_saturn_ships_still_reads() {
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../saturn/assets/icons");
        for entry in std::fs::read_dir(icons).expect("the icons") {
            let path = entry.expect("an icon").path();
            if path.extension().is_some_and(|extension| extension == "svg") {
                let svg = std::fs::read_to_string(&path).expect("an icon's text");
                let sharp = sharpen(&svg);
                assert!(sharp.contains("<svg") && sharp.contains("</svg>"), "{}", path.display());
                assert_eq!(sharp.matches("<path").count(), svg.matches("<path").count(), "{}", path.display());
            }
        }
    }
}
