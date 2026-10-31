//! Turns a key press into the bytes a terminal program expects (xterm conventions).
//!
//! Option types characters, as everywhere else on a Mac, rather than acting as Meta: on many
//! keyboard layouts (Czech among them) `@`, `{`, `}`, `[` and `]` need it. Settings can make it Meta
//! instead (`KeyPress::meta`), for shells and editors that bind Alt. The Mac terminal habits
//! still work: Option+←/→ jumps a word, Option+Backspace deletes one, ⌘←/→ go to line start/end.

/// A key press, independent of the UI framework.
#[expect(clippy::struct_excessive_bools, reason = "one flag per modifier key, as the keyboard has them")]
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyPress<'a> {
    /// The key's name: `a`, `enter`, `left`, `f5`…
    pub key: &'a str,
    /// The text the key types, if any (`ß` for Option+s on a US layout).
    pub text: Option<&'a str>,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub command: bool,
    /// Option is Meta: Option+key sends Escape and the key, not the character the layout puts on it.
    pub meta: bool,
    /// Caps Lock is on. The text the framework gives with a key follows Shift and Option only, so
    /// letters are made capitals here, as everywhere else on a Mac.
    pub caps_lock: bool,
}

/// xterm's modifier parameter: 1 + shift + 2·alt + 4·control.
fn modifier_param(key: &KeyPress) -> u8 {
    1 + u8::from(key.shift) + 2 * u8::from(key.alt) + 4 * u8::from(key.control)
}

/// Cursor and navigation keys: `ESC [ final` (or `ESC O final` in application cursor mode), with
/// modifiers as `ESC [ 1 ; m final`.
fn cursor_key(key: &KeyPress, final_byte: char, app_cursor: bool) -> Vec<u8> {
    let modifiers = modifier_param(key);
    if modifiers > 1 {
        format!("\x1b[1;{modifiers}{final_byte}").into_bytes()
    } else if app_cursor {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

/// `ESC [ code ~` keys, with modifiers as `ESC [ code ; m ~`.
fn tilde_key(key: &KeyPress, code: u8) -> Vec<u8> {
    let modifiers = modifier_param(key);
    if modifiers > 1 { format!("\x1b[{code};{modifiers}~").into_bytes() } else { format!("\x1b[{code}~").into_bytes() }
}

/// The bytes for a key press, or `None` when the terminal should not see it (⌘ shortcuts belong to
/// the app).
pub fn encode(key: &KeyPress, app_cursor: bool) -> Option<Vec<u8>> {
    if key.command {
        return match key.key {
            "left" => Some(b"\x01".to_vec()),
            "right" => Some(b"\x05".to_vec()),
            "backspace" => Some(b"\x15".to_vec()),
            _ => None,
        };
    }

    let bytes = match key.key {
        "enter" if key.shift || key.alt => b"\x1b\r".to_vec(),
        "enter" => b"\r".to_vec(),
        "backspace" if key.alt => b"\x1b\x7f".to_vec(),
        "backspace" if key.control => b"\x08".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "tab" if key.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "left" if key.alt && !key.shift && !key.control => b"\x1bb".to_vec(),
        "right" if key.alt && !key.shift && !key.control => b"\x1bf".to_vec(),
        "up" => cursor_key(key, 'A', app_cursor),
        "down" => cursor_key(key, 'B', app_cursor),
        "right" => cursor_key(key, 'C', app_cursor),
        "left" => cursor_key(key, 'D', app_cursor),
        "home" => cursor_key(key, 'H', app_cursor),
        "end" => cursor_key(key, 'F', app_cursor),
        "insert" => tilde_key(key, 2),
        "delete" => tilde_key(key, 3),
        "pageup" => tilde_key(key, 5),
        "pagedown" => tilde_key(key, 6),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde_key(key, 15),
        "f6" => tilde_key(key, 17),
        "f7" => tilde_key(key, 18),
        "f8" => tilde_key(key, 19),
        "f9" => tilde_key(key, 20),
        "f10" => tilde_key(key, 21),
        "f11" => tilde_key(key, 23),
        "f12" => tilde_key(key, 24),
        "space" if key.control => vec![0],
        name if key.control => {
            // Ctrl+letter is the letter's control code; a few punctuation keys have their own.
            let byte = match name.as_bytes() {
                [letter] if letter.is_ascii_alphabetic() => letter.to_ascii_lowercase() - b'a' + 1,
                b"[" => 0x1b,
                b"\\" => 0x1c,
                b"]" => 0x1d,
                b"6" => 0x1e,
                b"-" => 0x1f,
                b"@" | b"2" => 0,
                _ => return key.text.map(|text| text.as_bytes().to_vec()),
            };
            vec![byte]
        }
        name if key.alt && key.meta && name.chars().count() == 1 => {
            let name = if key.shift { name.to_uppercase() } else { name.to_owned() };
            format!("\x1b{name}").into_bytes()
        }
        _ => {
            let text = key.text.filter(|text| !text.is_empty())?;
            let text = if key.caps_lock { text.to_uppercase() } else { text.to_owned() };
            return Some(text.into_bytes());
        }
    };
    Some(bytes)
}

/// Files dropped on a terminal, as Terminal.app types them: each path with the shell's special
/// characters escaped, a space between, and one after, ready to keep typing. Claude Code reads a
/// dropped image's path as an attachment.
/// What pasting the clipboard types: copied files as their paths (a copied file's clipboard also
/// carries its bare name as text, which an agent can't find), an image (a screenshot copied to the
/// clipboard) saved into `folder` and typed as its path, else the text. Paths are what Claude Code
/// attaches, as with a dropped file.
pub fn paste_text(item: &gpui_kit::ClipboardItem, folder: &std::path::Path) -> Option<String> {
    for entry in &item.entries {
        if let gpui_kit::ClipboardEntry::ExternalPaths(paths) = entry {
            return Some(dropped_paths(paths.paths()));
        }
    }
    for entry in &item.entries {
        if let gpui_kit::ClipboardEntry::Image(image) = entry {
            let kind = image.format.mime_type().rsplit('/').next().unwrap_or("png");
            let extension = kind.split('+').next().unwrap_or("png").replace("jpeg", "jpg");
            let path = folder.join(format!("pasted-{}.{extension}", image.id));
            let saved = std::fs::create_dir_all(folder).and_then(|()| std::fs::write(&path, &image.bytes));
            return saved.ok().map(|()| dropped_paths(&[path]));
        }
    }
    item.text()
}

pub fn dropped_paths(paths: &[std::path::PathBuf]) -> String {
    let mut text = String::new();
    for path in paths {
        for c in path.to_string_lossy().chars() {
            if !(c.is_alphanumeric() || "/._-+,:@%=~".contains(c)) {
                text.push('\\');
            }
            text.push(c);
        }
        text.push(' ');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: &str) -> KeyPress<'_> {
        KeyPress { key, text: Some(key).filter(|k| k.chars().count() == 1), ..KeyPress::default() }
    }

    fn bytes(key: &KeyPress) -> Vec<u8> {
        encode(key, false).expect("the terminal gets this key")
    }

    #[test]
    fn pastes_files_as_paths_and_images_as_saved_files() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let files = gpui_kit::ClipboardItem {
            entries: vec![
                gpui_kit::ClipboardEntry::ExternalPaths(gpui_kit::ExternalPaths(
                    vec![std::path::PathBuf::from("/Users/mia/data.json")].into(),
                )),
                gpui_kit::ClipboardEntry::String(gpui_kit::ClipboardString::new("data.json".to_owned())),
            ],
        };
        assert_eq!(paste_text(&files, dir.path()).as_deref(), Some("/Users/mia/data.json "));
        let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, b"\x89PNG".to_vec());
        let typed = paste_text(&gpui_kit::ClipboardItem::new_image(&image), dir.path()).expect("a path");
        let saved = std::path::PathBuf::from(typed.trim_end());
        assert!(
            saved.starts_with(dir.path()) && saved.extension().is_some_and(|extension| extension == "png"),
            "{typed}"
        );
        assert_eq!(std::fs::read(&saved).expect("saved"), b"\x89PNG");
        assert_eq!(
            paste_text(&gpui_kit::ClipboardItem::new_string("hi".to_owned()), dir.path()).as_deref(),
            Some("hi")
        );
    }

    #[test]
    fn dropped_paths_are_escaped_for_the_shell() {
        let paths = [
            std::path::PathBuf::from("/Users/mia/Desktop/CleanShot 2026-10-19 at 13.27.05@2x.png"),
            std::path::PathBuf::from("/tmp/it's (final).txt"),
        ];
        assert_eq!(
            dropped_paths(&paths),
            "/Users/mia/Desktop/CleanShot\\ 2026-10-19\\ at\\ 13.27.05@2x.png /tmp/it\\'s\\ \\(final\\).txt "
        );
    }

    #[test]
    fn caps_lock_types_capitals() {
        let press = |key: &'static str, text: &'static str| KeyPress {
            key,
            text: Some(text),
            caps_lock: true,
            ..KeyPress::default()
        };
        assert_eq!(encode(&press("a", "a"), false), Some(b"A".to_vec()));
        assert_eq!(encode(&press("2", "č"), false), Some("Č".as_bytes().to_vec()), "a Czech č becomes Č");
        assert_eq!(encode(&press("1", "1"), false), Some(b"1".to_vec()), "digits stay");
        let control = KeyPress { key: "c", text: Some("c"), control: true, caps_lock: true, ..KeyPress::default() };
        assert_eq!(encode(&control, false), Some(vec![3]), "Ctrl+C is still Ctrl+C");
    }

    #[test]
    fn types_text() {
        assert_eq!(bytes(&press("a")), b"a");
        assert_eq!(bytes(&KeyPress { key: "e", text: Some("é"), ..KeyPress::default() }), "é".as_bytes());
    }

    #[test]
    fn option_types_the_characters_a_layout_puts_on_it() {
        let at = KeyPress { key: "v", text: Some("@"), alt: true, ..KeyPress::default() };
        assert_eq!(bytes(&at), b"@");
    }

    #[test]
    fn option_as_meta_sends_escape_and_the_key() {
        let meta = KeyPress { key: "b", text: Some("∫"), alt: true, meta: true, ..KeyPress::default() };
        assert_eq!(bytes(&meta), b"\x1bb");
        assert_eq!(bytes(&KeyPress { shift: true, ..meta }), b"\x1bB");
    }

    #[test]
    fn shift_enter_is_a_newline_for_claude() {
        assert_eq!(bytes(&KeyPress { shift: true, ..press("enter") }), b"\x1b\r");
        assert_eq!(bytes(&press("enter")), b"\r");
    }

    #[test]
    fn control_letters_are_control_codes() {
        assert_eq!(bytes(&KeyPress { control: true, ..press("c") }), [3]);
        assert_eq!(bytes(&KeyPress { control: true, ..press("a") }), [1]);
        assert_eq!(bytes(&KeyPress { control: true, ..press("[") }), [0x1b]);
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        assert_eq!(bytes(&press("up")), b"\x1b[A");
        assert_eq!(encode(&press("up"), true).expect("arrow"), b"\x1bOA");
        assert_eq!(bytes(&KeyPress { shift: true, ..press("right") }), b"\x1b[1;2C");
    }

    #[test]
    fn mac_word_and_line_jumps() {
        assert_eq!(bytes(&KeyPress { alt: true, ..press("left") }), b"\x1bb");
        assert_eq!(bytes(&KeyPress { alt: true, ..press("backspace") }), b"\x1b\x7f");
        assert_eq!(bytes(&KeyPress { command: true, ..press("left") }), b"\x01");
        assert_eq!(bytes(&KeyPress { command: true, ..press("backspace") }), b"\x15");
    }

    #[test]
    fn command_shortcuts_stay_with_the_app() {
        assert_eq!(encode(&KeyPress { command: true, ..press("t") }, false), None);
    }

    #[test]
    fn navigation_and_function_keys() {
        assert_eq!(bytes(&press("pageup")), b"\x1b[5~");
        assert_eq!(bytes(&press("delete")), b"\x1b[3~");
        assert_eq!(bytes(&press("f1")), b"\x1bOP");
        assert_eq!(bytes(&press("f5")), b"\x1b[15~");
        assert_eq!(bytes(&KeyPress { shift: true, ..press("tab") }), b"\x1b[Z");
    }
}
