//! The palettes: Go to file (⌘P) and the command palette (⌘⇧P). One search field over a list,
//! fuzzy-matched as you type; ↑↓ move, Enter picks, Esc closes. With nothing typed, a palette
//! shows what was picked in it lately, so it opens at once and at a steady size.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::{
    Action, AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use saturn_core::fuzzy;
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use crate::workspace::{self, Workspace};
use saturn_design::text;

/// How many matches a palette lists: enough to scroll, few enough to draw at once.
const SHOWN: usize = 60;

/// How many recent picks a palette remembers and shows before you type.
const RECENT: usize = 8;

thread_local! {
    /// Each palette's recent picks (their search texts), newest first, for this run of Saturn.
    static RECENTS: RefCell<HashMap<&'static str, Vec<String>>> = RefCell::new(HashMap::new());
}

fn recents(kind: &'static str) -> Vec<String> {
    RECENTS.with_borrow(|recents| recents.get(kind).cloned().unwrap_or_default())
}

fn remember(kind: &'static str, text: &str) {
    RECENTS.with_borrow_mut(|recents| {
        let list = recents.entry(kind).or_default();
        list.retain(|seen| seen != text);
        list.insert(0, text.to_owned());
        list.truncate(RECENT);
    });
}

/// A line in a palette.
struct Entry {
    /// What the search matches: a file's path, a command's name.
    text: String,
    /// What the line says, and in faint type after it.
    label: String,
    detail: Option<String>,
    icon: &'static str,
    /// The action whose shortcut the line shows.
    action: Option<Box<dyn Action>>,
}

type Pick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// What a palette says when it has nothing to list.
struct Words {
    /// With nothing typed and nothing picked lately.
    hint: &'static str,
    /// While the entries are still being read.
    loading: &'static str,
    /// When nothing matches what's typed.
    none: &'static str,
}

pub struct Palette {
    /// Which palette: its recent picks are kept under this.
    kind: &'static str,
    search: Entity<InputState>,
    entries: Vec<Entry>,
    loaded: bool,
    /// The entries listed: the recent picks, or what matches the search, best first.
    shown: Vec<usize>,
    selected: usize,
    words: Words,
    pick: Pick,
    _search_events: Subscription,
}

impl Palette {
    fn new(
        kind: &'static str,
        placeholder: &'static str,
        words: Words,
        entries: Option<Vec<Entry>>,
        pick: Pick,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Screenshots type their query up front.
        let typed = std::env::var("SATURN_PALETTE_QUERY").unwrap_or_default();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(typed));
        let search_events = cx.subscribe_in(&search, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.filter(cx),
            InputEvent::PressEnter { .. } => this.choose(window, cx),
            _ => {}
        });
        let loaded = entries.is_some();
        let mut palette = Self {
            kind,
            search,
            entries: entries.unwrap_or_default(),
            loaded,
            shown: Vec::new(),
            selected: 0,
            words,
            pick,
            _search_events: search_events,
        };
        palette.filter(cx);
        palette
    }

    fn set_entries(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        self.entries = entries;
        self.loaded = true;
        self.filter(cx);
    }

    fn query(&self, cx: &Context<Self>) -> String {
        self.search.read(cx).value().trim().to_owned()
    }

    /// Lists the recent picks while nothing is typed, else what matches.
    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.shown = if query.is_empty() && (self.kind == "compare" || self.kind == "checks") {
            // Branches and commits are few and in an order that matters: all of them, newest first.
            (0..self.entries.len().min(SHOWN)).collect()
        } else if query.is_empty() {
            recents(self.kind)
                .iter()
                .filter_map(|text| self.entries.iter().position(|entry| entry.text == *text))
                .collect()
        } else {
            fuzzy::rank(&query, self.entries.iter().map(|entry| entry.text.as_str()), SHOWN)
        };
        self.selected = 0;
        cx.notify();
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(&index) = self.shown.get(self.selected) else { return };
        remember(self.kind, &self.entries[index].text);
        let pick = self.pick.clone();
        window.close_dialog(cx);
        // After the dialog has handed the focus back, so a command reaches the pane it's for.
        window.defer(cx, move |window, cx| pick(index, window, cx));
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() - 1;
        self.selected = self.selected.saturating_add_signed(by).min(last);
        cx.notify();
    }

    fn render_entry(&self, row: usize, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.entries[index];
        let selected = row == self.selected;
        div()
            .id(("entry", row))
            .flex()
            .items_center()
            .gap(len(space::SPACE_3))
            .h(px(32.))
            .px(len(space::SPACE_3))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .text_scale(text::BODY)
            .when(selected, |line| line.bg(theme::fill_pressed()))
            .when(!selected, |line| line.hover(|s| s.bg(theme::fill_hover())))
            .child(chrome::icon(entry.icon, 14., theme::color(color::INK_MUTED)))
            .child(div().flex_none().text_color(theme::color(color::INK)).child(entry.label.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_scale(text::META)
                    .text_color(theme::color(color::INK_FAINT))
                    .children(entry.detail.clone()),
            )
            .children(entry.action.as_deref().and_then(|action| Kbd::binding_for_action(action, None, window)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = row;
                this.choose(window, cx);
            }))
            .into_any_element()
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .shown
            .clone()
            .into_iter()
            .enumerate()
            .map(|(row, index)| self.render_entry(row, index, window, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(len(space::SPACE_2))
            // Before the search field sees them: it would move its caret instead.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "up" => {
                    this.step(-1, cx);
                    cx.stop_propagation();
                }
                "down" => {
                    this.step(1, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }))
            // A quiet field: a fill, no focus ring; the caret says where typing goes.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(len(space::SPACE_2))
                    .h(px(36.))
                    .px(len(space::SPACE_3))
                    .rounded(theme::corner(radius::RADIUS_SM))
                    .bg(theme::fill_control())
                    .child(chrome::icon("search", 14., theme::color(color::INK_FAINT)))
                    .child(div().flex_1().child(Input::new(&self.search).appearance(false))),
            )
            .child(if rows.is_empty() {
                let words = if self.query(cx).is_empty() {
                    self.words.hint
                } else if self.loaded {
                    self.words.none
                } else {
                    self.words.loading
                };
                div()
                    .py(len(space::SPACE_3))
                    .px(len(space::SPACE_3))
                    .text_scale(text::LABEL)
                    .text_color(theme::color(color::INK_FAINT))
                    .child(words)
                    .into_any_element()
            } else {
                div()
                    .id("palette-list")
                    .flex()
                    .flex_col()
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .children(rows)
                    .into_any_element()
            })
    }
}

impl Palette {
    /// Whether typing goes into the search (spikes check it).
    pub fn search_has_focus(&self, window: &Window, cx: &App) -> bool {
        use gpui_kit::Focusable as _;
        self.search.focus_handle(cx).is_focused(window)
    }
}

thread_local! {
    /// The palette open last (spikes check its focus).
    pub static LAST: RefCell<Option<gpui_kit::WeakEntity<Palette>>> = const { RefCell::new(None) };
}

fn open(title: &'static str, palette: Entity<Palette>, window: &mut Window, cx: &mut App) {
    // Once the dialog is up: it takes the focus as it opens, and typing belongs in the search.
    let search = palette.read(cx).search.clone();
    LAST.with_borrow_mut(|last| *last = Some(palette.downgrade()));
    window.defer(cx, move |window, cx| search.update(cx, |search, cx| search.focus(window, cx)));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(title)
            .entrance(DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(640.))
            .child(palette.clone())
    });
}

/// Go to file: every file in the folder the sidebar shows (what git ignores left out).
pub fn go_to_file(workspace: WeakEntity<Workspace>, root: PathBuf, window: &mut Window, cx: &mut App) {
    let files: Rc<std::cell::RefCell<Vec<PathBuf>>> = Rc::default();
    let pick: Pick = {
        let (files, root) = (files.clone(), root.clone());
        Rc::new(move |index, window, cx| {
            let Some(file) = files.borrow().get(index).cloned() else { return };
            let _ = workspace.update(cx, |workspace, cx| workspace.open_file(&root.join(file), window, cx));
        })
    };
    let words =
        Words {
            hint: "Type part of a file's name or path.", loading: "Reading the files…", none: "No file matches."
        };
    let palette = cx.new(|cx| Palette::new("files", "Search files", words, None, pick, window, cx));
    let weak = palette.downgrade();
    cx.spawn(async move |cx| {
        let found = cx.background_spawn(async move { list_files(&root) }).await;
        let entries = found.iter().map(|file| file_entry(file)).collect();
        *files.borrow_mut() = found;
        let _ = weak.update(cx, |palette, cx| palette.set_entries(entries, cx));
    })
    .detach();
    open("Go to file", palette, window, cx);
}

/// What to compare `repo`'s work with: its branches, tags and recent commits. Picking one reviews
/// everything that differs from it.
pub fn compare(workspace: WeakEntity<Workspace>, repo: PathBuf, window: &mut Window, cx: &mut App) {
    let targets: Rc<std::cell::RefCell<Vec<saturn_core::git::Target>>> = Rc::default();
    let pick: Pick = {
        let (targets, repo) = (targets.clone(), repo.clone());
        Rc::new(move |index, _, cx| {
            let Some(target) = targets.borrow().get(index).cloned() else { return };
            let _ = workspace.update(cx, |workspace, cx| workspace.review_against(&repo, target.name, cx));
        })
    };
    let words = Words {
        hint: "Type part of a branch, tag or commit.",
        loading: "Reading the branches…",
        none: "Nothing matches.",
    };
    let palette =
        cx.new(|cx| Palette::new("compare", "Search branches, tags and commits", words, None, pick, window, cx));
    let weak = palette.downgrade();
    cx.spawn(async move |cx| {
        let found = cx.background_spawn(async move { saturn_core::git::compare_targets(&repo) }).await;
        let entries = found
            .iter()
            .map(|target| Entry {
                text: format!("{} {}", target.name, target.detail),
                label: target.name.clone(),
                detail: Some(target.detail.clone()),
                icon: match target.kind {
                    saturn_core::git::TargetKind::Branch => "git-branch",
                    saturn_core::git::TargetKind::Tag => "tag",
                    saturn_core::git::TargetKind::Commit => "git-commit-horizontal",
                },
                action: None,
            })
            .collect();
        *targets.borrow_mut() = found;
        let _ = weak.update(cx, |palette, cx| palette.set_entries(entries, cx));
    })
    .detach();
    open("Compare with", palette, window, cx);
}

/// The checks of the focused pane's checkout (its recipe's, or guessed), to run one, or all.
pub fn checks(workspace: WeakEntity<Workspace>, checkout: PathBuf, window: &mut Window, cx: &mut App) {
    let found = match saturn_core::recipe::for_checkout(&checkout) {
        Ok((recipe, _)) => recipe.checks,
        Err(_) => Vec::new(),
    };
    let mut entries: Vec<Entry> = found
        .iter()
        .map(|(name, command)| Entry {
            text: format!("{name} {command}"),
            label: name.clone(),
            detail: Some(command.clone()),
            icon: "play",
            action: None,
        })
        .collect();
    if found.len() > 1 {
        entries.push(Entry {
            text: "All of them".to_owned(),
            label: "All of them".to_owned(),
            detail: None,
            icon: "play",
            action: None,
        });
    }
    let pick: Pick = Rc::new(move |index, _, cx| {
        let picked: Vec<(String, String)> = match found.get(index) {
            Some(check) => vec![check.clone()],
            None => found.clone(),
        };
        let _ = workspace.update(cx, |workspace, cx| {
            for (name, command) in picked {
                workspace.run_check(&checkout, name, command, cx);
            }
        });
    });
    let words = Words {
        hint: "Pick a check to run.",
        loading: "",
        none: "No checks here. Add \"checks\" to .saturn/worktree.json, or a lint or test script.",
    };
    let palette = cx.new(|cx| Palette::new("checks", "Run a check", words, Some(entries), pick, window, cx));
    open("Run a check", palette, window, cx);
}

fn file_entry(file: &Path) -> Entry {
    let label = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let detail = file.parent().map(|folder| folder.display().to_string()).filter(|folder| !folder.is_empty());
    Entry { text: file.display().to_string(), label, detail, icon: "file-code", action: None }
}

/// The files under `root`, relative to it, without what git ignores; at most 100,000.
fn list_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = ignore::WalkBuilder::new(root)
        .require_git(false)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git" && entry.file_name() != "node_modules")
        .build()
        .flatten()
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .take(100_000)
        .collect();
    files.sort();
    files
}

/// The command palette: everything Saturn does, by name, with its shortcut.
pub fn commands(window: &mut Window, cx: &mut App) {
    let commands = workspace::commands();
    let entries = commands
        .iter()
        .map(|(name, icon, action)| Entry {
            text: (*name).to_owned(),
            label: (*name).to_owned(),
            detail: None,
            icon,
            action: Some(action.boxed_clone()),
        })
        .collect();
    let pick: Pick = Rc::new(move |index, window, cx| {
        if let Some((_, _, action)) = commands.get(index) {
            window.dispatch_action(action.boxed_clone(), cx);
        }
    });
    let words = Words { hint: "Type what you want to do.", loading: "", none: "No command matches." };
    let palette = cx.new(|cx| Palette::new("commands", "Search commands", words, Some(entries), pick, window, cx));
    open("Commands", palette, window, cx);
}
