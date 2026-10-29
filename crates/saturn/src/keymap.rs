//! Saturn's shortcuts: each one's default keys, and the keys the user chose instead in Settings
//! (kept in `settings.json` by the shortcut's id, an empty string for none). Menus, tooltips and the
//! palettes read the keymap, so they follow a change at once.

use std::collections::BTreeMap;

use gpui_kit::{Action, App, KeyBinding, Keystroke, SharedString, Unbind};

use crate::files_view::{NewFile, NewFolder};
use crate::workspace::{
    CloseTab, CommandPalette, EvenOutPanes, FocusDown, FocusLeft, FocusPane1, FocusPane2, FocusPane3, FocusPane4,
    FocusPane5, FocusPane6, FocusPane7, FocusPane8, FocusPane9, FocusRight, FocusUp, GoToFile, NewTab, NextAttention,
    NextTab, OpenProject, OpenSettings, OpenTicketPicker, PreviousTab, SaveFile, SplitDown, SplitRight, TextBigger,
    TextReset, TextSmaller, ToggleEditor, ToggleMaximize, ToggleSidebar, ToggleTickets,
};

/// One shortcut a user can change.
pub struct Shortcut {
    /// Its key in `settings.json`.
    pub id: &'static str,
    /// What it does, as Settings lists it.
    pub name: &'static str,
    pub group: &'static str,
    /// The keys it has until the user picks others; the first is the one shown.
    pub keys: &'static [&'static str],
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! shortcut {
    ($id:literal, $name:literal, $group:literal, [$($key:literal),*], $action:expr) => {
        Shortcut { id: $id, name: $name, group: $group, keys: &[$($key),*], action: || Box::new($action) }
    };
}

/// Every shortcut, in the order Settings lists them. Copy, paste and quit stay macOS's own.
pub const SHORTCUTS: &[Shortcut] = &[
    // ⌘T as terminals open a tab; ⌘N makes a file, as editors do.
    shortcut!("new-tab", "New terminal", "Terminals", ["cmd-t"], NewTab),
    shortcut!("new-agent", "New agent, in a worktree of its own", "Terminals", ["cmd-shift-n"], OpenTicketPicker),
    shortcut!("split-right", "Split right", "Terminals", ["cmd-d"], SplitRight),
    shortcut!("split-down", "Split down", "Terminals", ["cmd-shift-d"], SplitDown),
    shortcut!("close-tab", "Close the terminal", "Terminals", ["cmd-w"], CloseTab),
    shortcut!("next-tab", "Next pane", "Terminals", ["cmd-shift-]"], NextTab),
    shortcut!("previous-tab", "Previous pane", "Terminals", ["cmd-shift-["], PreviousTab),
    shortcut!("maximize", "Maximise the pane", "Terminals", ["cmd-shift-enter"], ToggleMaximize),
    shortcut!("even-out", "Even out the panes", "Terminals", ["cmd-ctrl-="], EvenOutPanes),
    shortcut!("next-attention", "Next agent that needs you", "Terminals", ["cmd-shift-a"], NextAttention),
    shortcut!("focus-left", "Pane to the left", "Moving around", ["cmd-alt-left"], FocusLeft),
    shortcut!("focus-right", "Pane to the right", "Moving around", ["cmd-alt-right"], FocusRight),
    shortcut!("focus-up", "Pane above", "Moving around", ["cmd-alt-up"], FocusUp),
    shortcut!("focus-down", "Pane below", "Moving around", ["cmd-alt-down"], FocusDown),
    shortcut!("focus-1", "Pane 1", "Moving around", ["cmd-1"], FocusPane1),
    shortcut!("focus-2", "Pane 2", "Moving around", ["cmd-2"], FocusPane2),
    shortcut!("focus-3", "Pane 3", "Moving around", ["cmd-3"], FocusPane3),
    shortcut!("focus-4", "Pane 4", "Moving around", ["cmd-4"], FocusPane4),
    shortcut!("focus-5", "Pane 5", "Moving around", ["cmd-5"], FocusPane5),
    shortcut!("focus-6", "Pane 6", "Moving around", ["cmd-6"], FocusPane6),
    shortcut!("focus-7", "Pane 7", "Moving around", ["cmd-7"], FocusPane7),
    shortcut!("focus-8", "Pane 8", "Moving around", ["cmd-8"], FocusPane8),
    shortcut!("focus-9", "Pane 9", "Moving around", ["cmd-9"], FocusPane9),
    shortcut!("go-to-file", "Go to file", "Files and views", ["cmd-p"], GoToFile),
    shortcut!("command-palette", "Command palette", "Files and views", ["cmd-shift-p"], CommandPalette),
    shortcut!("save", "Save the file", "Files and views", ["cmd-s"], SaveFile),
    shortcut!("new-file", "New file", "Files and views", ["cmd-n"], NewFile),
    shortcut!("new-folder", "New folder", "Files and views", ["cmd-alt-n"], NewFolder),
    shortcut!("open-project", "Add a repo", "Files and views", ["cmd-o"], OpenProject),
    shortcut!("toggle-sidebar", "Show or hide the sidebar", "Files and views", ["cmd-b"], ToggleSidebar),
    shortcut!("toggle-editor", "Show or hide the editor", "Files and views", ["cmd-j"], ToggleEditor),
    shortcut!("toggle-tickets", "Show or hide tickets", "Files and views", ["cmd-alt-b"], ToggleTickets),
    shortcut!("settings", "Settings", "Files and views", ["cmd-,"], OpenSettings),
    // ⌘+ is ⌘= without shift on most keyboards; both make text bigger.
    shortcut!("text-bigger", "Bigger text", "Text size", ["cmd-=", "cmd-+"], TextBigger),
    shortcut!("text-smaller", "Smaller text", "Text size", ["cmd--"], TextSmaller),
    shortcut!("text-reset", "Actual size", "Text size", ["cmd-0"], TextReset),
];

/// The keys `shortcut` has now: the user's choice (none, when they cleared it), else its defaults.
pub fn keys_of(shortcut: &Shortcut, chosen: &BTreeMap<String, String>) -> Vec<String> {
    match chosen.get(shortcut.id) {
        Some(keys) if keys.is_empty() => Vec::new(),
        Some(keys) => vec![keys.clone()],
        None => shortcut.keys.iter().map(|&keys| keys.to_owned()).collect(),
    }
}

/// Binds every shortcut, as the user chose them. Once, as Saturn starts.
pub fn bind(chosen: &BTreeMap<String, String>, cx: &mut App) {
    let bindings: Vec<KeyBinding> = SHORTCUTS
        .iter()
        .flat_map(|shortcut| {
            keys_of(shortcut, chosen).into_iter().filter_map(|keys| binding(&keys, (shortcut.action)()))
        })
        .collect();
    cx.bind_keys(bindings);
}

/// Moves `shortcut` from the keys it had (`before`) to the ones now chosen. GPUI has no way to take
/// one binding out, so the old keys get an `Unbind` for this action, which outranks them.
pub fn rebind(shortcut: &Shortcut, before: &[String], chosen: &BTreeMap<String, String>, cx: &mut App) {
    let name = SharedString::from((shortcut.action)().name());
    let mut bindings: Vec<KeyBinding> =
        before.iter().filter_map(|keys| binding(keys, Box::new(Unbind(name.clone())))).collect();
    bindings.extend(keys_of(shortcut, chosen).iter().filter_map(|keys| binding(keys, (shortcut.action)())));
    cx.bind_keys(bindings);
}

/// A binding for `keys`, when they parse.
fn binding(keys: &str, action: Box<dyn Action>) -> Option<KeyBinding> {
    KeyBinding::load(keys, action, None, false, None, &gpui_kit::DummyKeyboardMapper).ok()
}

/// The keys of a keystroke as Saturn stores them (`cmd-shift-n`), or `None` for a lone modifier,
/// which can't be a shortcut.
pub fn keys_for(keystroke: &Keystroke) -> Option<String> {
    let lone_modifier = matches!(keystroke.key.as_str(), "shift" | "control" | "alt" | "platform" | "function" | "");
    let modified = keystroke.modifiers.platform || keystroke.modifiers.control || keystroke.modifiers.alt;
    // A plain letter would stop it being typed anywhere; F-keys and the like can stand alone.
    let plain_typing = !modified && keystroke.key.chars().count() == 1;
    (!lone_modifier && !plain_typing).then(|| keystroke.unparse())
}

/// The other shortcut already on `keys`, if one is.
pub fn taken_by(keys: &str, except: &str, chosen: &BTreeMap<String, String>) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|shortcut| shortcut.id != except && keys_of(shortcut, chosen).iter().any(|k| k == keys))
}

/// `cmd-shift-n` as macOS writes it: ⌘⇧N.
pub fn symbols(keys: &str) -> String {
    let mut text = String::new();
    let mut parts: Vec<&str> = keys.split('-').collect();
    // `cmd--` splits into an empty last part: the key is the minus itself.
    if keys.ends_with("--") {
        parts.truncate(parts.len() - 2);
        parts.push("-");
    }
    let Some((key, modifiers)) = parts.split_last() else { return text };
    for modifier in modifiers {
        text.push_str(match *modifier {
            "ctrl" | "control" => "⌃",
            "alt" | "option" => "⌥",
            "shift" => "⇧",
            "cmd" | "platform" => "⌘",
            "fn" | "function" => "fn ",
            other => other,
        });
    }
    let key = match *key {
        "enter" => "↩".to_owned(),
        "left" => "←".to_owned(),
        "right" => "→".to_owned(),
        "up" => "↑".to_owned(),
        "down" => "↓".to_owned(),
        "space" => "Space".to_owned(),
        "tab" => "⇥".to_owned(),
        "escape" => "⎋".to_owned(),
        "backspace" => "⌫".to_owned(),
        other => other.to_uppercase(),
    };
    text.push_str(&key);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_keys_as_macos_does() {
        assert_eq!(symbols("cmd-shift-n"), "⌘⇧N");
        assert_eq!(symbols("cmd--"), "⌘-");
        assert_eq!(symbols("cmd-alt-left"), "⌘⌥←");
        assert_eq!(symbols("cmd-shift-enter"), "⌘⇧↩");
    }

    #[test]
    fn a_choice_replaces_the_defaults_and_an_empty_one_clears_them() {
        let bigger = SHORTCUTS.iter().find(|shortcut| shortcut.id == "text-bigger").expect("listed");
        let mut chosen = BTreeMap::new();
        assert_eq!(keys_of(bigger, &chosen), ["cmd-=", "cmd-+"]);
        chosen.insert("text-bigger".to_owned(), "cmd-shift-=".to_owned());
        assert_eq!(keys_of(bigger, &chosen), ["cmd-shift-="]);
        chosen.insert("text-bigger".to_owned(), String::new());
        assert_eq!(keys_of(bigger, &chosen), Vec::<String>::new());
        assert_eq!(taken_by("cmd-n", "x", &chosen).map(|shortcut| shortcut.id), Some("new-file"));
    }

    #[test]
    fn every_default_parses_and_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for shortcut in SHORTCUTS {
            assert!(ids.insert(shortcut.id), "{} twice", shortcut.id);
            for keys in shortcut.keys {
                assert!(Keystroke::parse(keys).is_ok(), "{keys}");
            }
        }
    }
}
