//! Web addresses in terminal text, so Command-click can open them. Programs that mark their links
//! (OSC 8) don't need this; everything else prints addresses as plain text.

use std::ops::Range;

const SCHEMES: [&str; 2] = ["https://", "http://"];

/// Whether a link may be opened. Only web pages and mail: a marked link can hide its target behind
/// any text, and a `file://` or custom-scheme target could start an app.
pub fn can_open(target: &str) -> bool {
    ["https://", "http://", "mailto:"].iter().any(|scheme| starts_with(&target.chars().collect::<Vec<_>>(), scheme))
}

/// The address that covers `column` in `line`: where it is, in characters, and the address itself.
pub fn url_at(line: &[char], column: usize) -> Option<(Range<usize>, String)> {
    let mut start = 0;
    while start < line.len() {
        let Some(scheme) = SCHEMES.iter().find(|scheme| starts_with(&line[start..], scheme)) else {
            start += 1;
            continue;
        };
        let mut end = start + line[start..].iter().take_while(|&&c| in_url(c)).count();
        end = trim_end(line, start + scheme.len(), end);
        if end > start + scheme.len() && (start..end).contains(&column) {
            return Some((start..end, line[start..end].iter().collect()));
        }
        start = end.max(start + 1);
    }
    None
}

fn starts_with(text: &[char], prefix: &str) -> bool {
    text.len() >= prefix.len() && prefix.chars().zip(text).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Characters an address can hold. Letters beyond ASCII count; the lines and boxes programs draw
/// around text don't.
fn in_url(c: char) -> bool {
    (c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '|' | '\\' | '^'))
        || (!c.is_ascii() && c.is_alphanumeric())
}

/// Leaves out what ends the sentence rather than the address: a full stop, a comma, or the closing
/// bracket of a parenthesis the address sits in.
fn trim_end(line: &[char], body: usize, mut end: usize) -> usize {
    while end > body {
        let last = line[end - 1];
        let unbalanced = |open: char| {
            let text = &line[body..end];
            text.iter().filter(|&&c| c == last).count() > text.iter().filter(|&&c| c == open).count()
        };
        let trailing = match last {
            '.' | ',' | ';' | ':' | '!' | '?' => true,
            ')' => unbalanced('('),
            ']' => unbalanced('['),
            _ => false,
        };
        if !trailing {
            break;
        }
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(text: &str, column: usize) -> Option<String> {
        url_at(&text.chars().collect::<Vec<_>>(), column).map(|(_, url)| url)
    }

    #[test]
    fn finds_the_address_under_a_column() {
        let text = "see https://acme.dev/docs?q=1#top for more";
        assert_eq!(find(text, 4).as_deref(), Some("https://acme.dev/docs?q=1#top"));
        assert_eq!(find(text, 20).as_deref(), Some("https://acme.dev/docs?q=1#top"));
        assert_eq!(find(text, 3), None);
        assert_eq!(find(text, 34), None);
    }

    #[test]
    fn reports_where_the_address_is() {
        let line: Vec<char> = "go http://localhost:3000/ now".chars().collect();
        assert_eq!(url_at(&line, 10).map(|(range, _)| range), Some(3..25));
    }

    #[test]
    fn leaves_out_sentence_punctuation_and_brackets_around_it() {
        assert_eq!(find("Open https://acme.dev.", 6).as_deref(), Some("https://acme.dev"));
        assert_eq!(find("(at https://acme.dev/a)", 6).as_deref(), Some("https://acme.dev/a"));
        assert_eq!(find("[docs](https://acme.dev/a)", 10).as_deref(), Some("https://acme.dev/a"));
        assert_eq!(
            find("https://en.wikipedia.org/wiki/Rust_(language)", 0).as_deref(),
            Some("https://en.wikipedia.org/wiki/Rust_(language)")
        );
    }

    #[test]
    fn stops_at_the_border_of_a_box() {
        assert_eq!(find("│ https://acme.dev/a│", 3).as_deref(), Some("https://acme.dev/a"));
    }

    #[test]
    fn finds_the_second_of_two_addresses_and_ignores_a_bare_scheme() {
        let text = "https://a.dev and https://b.dev";
        assert_eq!(find(text, 20).as_deref(), Some("https://b.dev"));
        assert_eq!(find("https:// alone", 2), None);
        assert_eq!(find("file:///tmp/notes.md", 9), None);
    }

    #[test]
    fn opens_only_web_pages_and_mail() {
        assert!(can_open("https://github.com/acme/acme-web/pull/123"));
        assert!(can_open("HTTP://acme.dev"));
        assert!(can_open("mailto:team@acme.dev"));
        assert!(!can_open("file:///Applications/Calculator.app"));
        assert!(!can_open("x-custom://run"));
    }
}
