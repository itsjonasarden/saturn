//! A sharp version of a Lucide icon, for a theme with hard corners: the same drawing with square
//! line ends and mitred joins, its small rounded corners made square (an arc of radius 3 or less
//! becomes the corner it rounded, whatever its angle), its rounded rectangles square, and its small
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

/// The corner a small arc from `from` to `to` rounds off: where the straight edges at its two
/// ends would meet. `arc` is the arc's numbers (radii, rotation, large-arc and sweep flags, end).
/// `None` for an arc that is part of the drawing: a large one, one of half a turn or more, or one
/// whose corner would stick out further than twice its radius.
fn corner(from: (f32, f32), to: (f32, f32), arc: &[f32]) -> Option<(f32, f32)> {
    let radius = arc[0];
    if radius > CORNER || (arc[0] - arc[1]).abs() > 0.01 || arc[3] > 0.5 {
        return None;
    }
    // The arc's centre, as SVG finds it (no rotation, round): half the chord, then out along the
    // perpendicular, to the side the flags say.
    let (half_x, half_y) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let half_chord = half_x.mul_add(half_x, half_y * half_y);
    if half_chord < 1e-6 {
        return None;
    }
    let reach = ((radius * radius - half_chord).max(0.0) / half_chord).sqrt();
    let side = if (arc[3] > 0.5) == (arc[4] > 0.5) { -1.0 } else { 1.0 };
    let centre = (
        (side * reach).mul_add(half_y, f32::midpoint(from.0, to.0)),
        (-side * reach).mul_add(half_x, f32::midpoint(from.1, to.1)),
    );
    // The two radii's directions; the corner is out along their bisector, where the tangents meet.
    let (a, b) = (
        ((from.0 - centre.0) / radius, (from.1 - centre.1) / radius),
        ((to.0 - centre.0) / radius, (to.1 - centre.1) / radius),
    );
    let (sum_x, sum_y) = (a.0 + b.0, a.1 + b.1);
    let length = sum_x.mul_add(sum_x, sum_y * sum_y);
    if length < 0.01 {
        return None;
    }
    let scale = 2.0 * radius / length;
    let corner = (sum_x.mul_add(scale, centre.0), sum_y.mul_add(scale, centre.1));
    // An arc that turns back far (a gear's tooth) would make a spike; it is drawing, not a corner.
    let reach = ((corner.0 - from.0).powi(2) + (corner.1 - from.1).powi(2)).sqrt();
    (reach <= 2.0 * radius).then_some(corner)
}

/// The path with its small arcs, the rounded corners, made square.
fn sharpen_path(d: &str) -> String {
    let mut reader = Reader { text: d.as_bytes(), at: 0 };
    let mut out: Vec<String> = Vec::new();
    let (mut x, mut y) = (0.0_f32, 0.0_f32);
    let (mut start_x, mut start_y) = (0.0_f32, 0.0_f32);
    let mut current = None;
    // Whether the last segment drawn was straight: only an arc after a straight edge rounds a
    // corner. Arcs joined to arcs (a gear's teeth) are the drawing itself.
    let mut after_straight = false;
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
        if command.eq_ignore_ascii_case(&b'a')
            && after_straight
            && let Some((corner_x, corner_y)) = corner((x, y), (to_x, to_y), &values)
        {
            out.push(format!("L{} {}L{} {}", number(corner_x), number(corner_y), number(to_x), number(to_y)));
            (x, y) = (to_x, to_y);
            after_straight = true;
            continue;
        }
        out.push(format!(
            "{}{}",
            command as char,
            values.iter().map(|value| number(*value)).collect::<Vec<_>>().join(" ")
        ));
        if command.eq_ignore_ascii_case(&b'm') {
            (start_x, start_y) = (to_x, to_y);
        }
        after_straight = matches!(command.to_ascii_lowercase(), b'l' | b'h' | b'v' | b'm');
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
    fn a_triangles_rounded_corners_become_points() {
        let sharp = sharpen_path("m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3");
        assert!(!sharp.contains('a') && !sharp.contains('A'), "no arcs left: {sharp}");
        // The top corner sits on the middle line, above where the arc was.
        assert!(sharp.contains("L11.99 0.93"), "{sharp}");
    }

    #[test]
    fn a_gears_teeth_stay_teeth() {
        // Lucide's gear is arcs joined to arcs, with no straight edge to make a corner with.
        let gear = "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033";
        assert_eq!(sharpen_path(gear).matches('a').count(), 3, "{}", sharpen_path(gear));
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
    fn every_icon_akyba_ships_still_reads() {
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../akyba/assets/icons");
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
