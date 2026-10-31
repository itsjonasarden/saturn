//! The file tree in the sidebar's well: a virtualized list of the open folders' entries, with git
//! badges, kept current by a file watcher. A right click on an entry makes a file or folder beside
//! it, renames it or moves it to the Trash; dragging one onto a folder moves it there.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    Action, AnyElement, App, AppContext as _, Context, EventEmitter, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Point, PromptLevel, Render, StatefulInteractiveElement as _,
    Styled as _, Window, actions, div, prelude::FluentBuilder as _, px, uniform_list,
};
use notify::Watcher as _;
use saturn_core::file_icon::{FileIcon, file_icon};
use saturn_core::files::{self, FileTree, Ignored, Row};
use saturn_core::git::{self, FileState};
use saturn_design::{color, radius, space};

use crate::chrome;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::icon;
use saturn_design::text;

const ROW_HEIGHT: f32 = saturn_design::size::ROW;
/// How far each level is indented.
const INDENT: f32 = 14.0;
/// File changes are gathered this long before the tree reads them, so a `git checkout` touching a
/// thousand files refreshes once.
const SETTLE: Duration = Duration::from_millis(150);

actions!(files, [NewFile, NewFolder, RenameEntry, TrashEntry]);

pub enum FilesEvent {
    Open(PathBuf),
    /// Git reported a new state: the Changes list and any review should read it again.
    GitChanged,
    /// Files changed on disk (not ignored ones); an open copy may need reloading.
    Changed(Vec<PathBuf>),
}

pub struct FilesView {
    tree: FileTree,
    rows: Vec<Row>,
    changes: HashMap<PathBuf, FileState>,
    selected: Option<PathBuf>,
    ignored: Ignored,
    /// The menu a right click opened: on which entry (`None`: the folder's empty space), and where.
    menu: Option<(Option<Row>, Point<Pixels>)>,
    /// Kept alive for as long as the view watches its root.
    _watcher: Option<notify::RecommendedWatcher>,
}

impl EventEmitter<FilesEvent> for FilesView {}

impl FilesView {
    pub fn new(root: &Path, cx: &mut Context<Self>) -> Self {
        let mut tree = FileTree::new(root.to_owned());
        let rows = tree.rows();
        let (sender, events) = async_channel::unbounded::<PathBuf>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    let _ = sender.send_blocking(path);
                }
            }
        })
        .and_then(|mut watcher| watcher.watch(root, notify::RecursiveMode::Recursive).map(|()| watcher))
        .ok();
        cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                cx.background_executor().timer(SETTLE).await;
                let mut batch = vec![first];
                while let Ok(path) = events.try_recv() {
                    batch.push(path);
                }
                if this.update(cx, |this, cx| this.files_changed(&batch, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let ignored = Ignored::new(root);
        let mut view =
            Self { tree, rows, changes: HashMap::new(), selected: None, ignored, menu: None, _watcher: watcher };
        view.refresh_git(cx);
        view
    }

    /// Re-reads the folders whose entries changed, and git's view of everything.
    fn files_changed(&mut self, changed: &[PathBuf], cx: &mut Context<Self>) {
        // Builds write thousands of ignored files (target/, node_modules/); they change nothing
        // the tree shows or git reports.
        let changed: Vec<&PathBuf> = changed.iter().filter(|path| !self.ignored.is_ignored(path)).collect();
        if changed.is_empty() {
            return;
        }
        cx.emit(FilesEvent::Changed(changed.iter().map(|path| (*path).clone()).collect()));
        let folders: HashSet<PathBuf> = changed
            .iter()
            .filter(|path| !path.components().any(|part| part.as_os_str() == ".git"))
            .filter_map(|path| path.parent().map(Path::to_owned))
            .collect();
        if !folders.is_empty() {
            self.tree.invalidate(Some(&folders.into_iter().collect::<Vec<_>>()));
            self.rows = self.tree.rows();
        }
        self.refresh_git(cx);
        cx.notify();
    }

    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let root = self.tree.root().to_owned();
        cx.spawn(async move |this, cx| {
            let changes = cx.background_spawn(async move { git::status(&root) }).await;
            let _ = this.update(cx, |this, cx| {
                if this.changes != changes {
                    this.changes = changes;
                    cx.emit(FilesEvent::GitChanged);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn click(&mut self, row: &Row, cx: &mut Context<Self>) {
        self.selected = Some(row.path.clone());
        if row.is_dir {
            self.tree.toggle(&row.path);
            self.rows = self.tree.rows();
        } else {
            cx.emit(FilesEvent::Open(row.path.clone()));
        }
        cx.notify();
    }

    /// The folder something new goes in: the entry's own, when it's a folder, else the one it's in;
    /// with no entry, the selected one's, else the root.
    fn folder_for(&self, entry: Option<&Row>) -> PathBuf {
        let root = self.tree.root().to_owned();
        let at = entry.map(|row| (row.path.clone(), row.is_dir)).or_else(|| {
            self.selected.clone().map(|path| {
                let is_dir = path.is_dir();
                (path, is_dir)
            })
        });
        match at {
            Some((path, true)) => path,
            Some((path, false)) => path.parent().map_or(root, Path::to_owned),
            None => root,
        }
    }

    /// Asks for the name of a new file (or folder) in `folder`, makes it, and opens a new file.
    pub fn ask_new(&mut self, folder: Option<PathBuf>, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        let folder = folder.unwrap_or_else(|| self.folder_for(None));
        let shown = folder.strip_prefix(self.tree.root()).unwrap_or(&folder).display().to_string();
        let place = if shown.is_empty() { "the repo's top".to_owned() } else { shown };
        let (title, button) = if is_dir { ("New folder", "Create folder") } else { ("New file", "Create file") };
        let files = cx.weak_entity();
        ask_name(title, format!("In {place}"), String::new(), button, window, cx, move |name, cx| {
            let path = files::new_entry(&folder, &name)?;
            let files = files.clone();
            cx.spawn(async move |cx| {
                let made = path.clone();
                let done = cx
                    .background_spawn(async move {
                        if is_dir {
                            std::fs::create_dir_all(&made)
                        } else {
                            made.parent()
                                .map_or(Ok(()), std::fs::create_dir_all)
                                .and_then(|()| std::fs::write(&made, ""))
                        }
                    })
                    .await;
                if done.is_ok() && !is_dir {
                    let _ = files.update(cx, |_, cx| cx.emit(FilesEvent::Open(path)));
                }
            })
            .detach();
            Ok(())
        });
    }

    fn ask_rename(from: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = from.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        ask_name("Rename", name.clone(), name, "Rename", window, cx, move |name, cx| {
            let to = files::renamed(&from, &name)?;
            let from = from.clone();
            cx.background_spawn(async move {
                let _ = std::fs::rename(from, to);
            })
            .detach();
            Ok(())
        });
    }

    /// Moves `path` to the Trash once asked, so it can come back from there.
    fn ask_trash(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let message = format!("Move “{name}” to the Trash?");
        let detail =
            if path.is_dir() { "The folder and everything in it go." } else { "You can put it back from the Trash." };
        let answer = window.prompt(PromptLevel::Warning, &message, Some(detail), &["Move to Trash", "Cancel"], cx);
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                cx.background_spawn(async move { trash(&path) }).await;
            }
        })
        .detach();
    }

    /// Moves `from` into `folder`, when it may go there.
    fn move_into(from: &Path, folder: &Path, cx: &mut Context<Self>) {
        let Ok(to) = files::moved_into(from, folder) else { return };
        let from = from.to_owned();
        cx.background_spawn(async move {
            let _ = std::fs::rename(from, to);
        })
        .detach();
    }

    fn open_menu(&mut self, entry: Option<Row>, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if let Some(row) = &entry {
            self.selected = Some(row.path.clone());
        }
        self.menu = Some((entry, event.position));
        cx.notify();
    }

    fn pick(&mut self, action: &dyn Action, window: &mut Window, cx: &mut Context<Self>) {
        let Some((entry, _)) = self.menu.take() else { return };
        let folder = Some(self.folder_for(entry.as_ref()));
        let any = action.as_any();
        if any.is::<NewFile>() {
            self.ask_new(folder, false, window, cx);
        } else if any.is::<NewFolder>() {
            self.ask_new(folder, true, window, cx);
        } else if let Some(row) = entry {
            if any.is::<RenameEntry>() {
                Self::ask_rename(row.path, window, cx);
            } else if any.is::<TrashEntry>() {
                Self::ask_trash(row.path, window, cx);
            }
        }
        cx.notify();
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (entry, position) = self.menu.as_ref()?;
        let mut items: Vec<crate::menu::Item> = vec![
            Some(("file-plus", "New file", Box::new(NewFile))),
            Some(("folder-plus", "New folder", Box::new(NewFolder))),
        ];
        if entry.is_some() {
            items.extend([
                None,
                Some(("pencil", "Rename", Box::new(RenameEntry) as Box<dyn Action>)),
                Some(("trash", "Move to Trash", Box::new(TrashEntry) as Box<dyn Action>)),
            ]);
        }
        let files = cx.weak_entity();
        let pick = move |action: Box<dyn Action>, window: &mut Window, cx: &mut App| {
            let _ = files.update(cx, |this, cx| this.pick(action.as_ref(), window, cx));
        };
        let files = cx.weak_entity();
        let dismiss = move |_: &mut Window, cx: &mut App| {
            let _ = files.update(cx, |this, cx| {
                this.menu = None;
                cx.notify();
            });
        };
        Some(crate::menu::render_at("file-menu", items, *position, pick, dismiss, cx))
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = self.rows[index].clone();
        let selected = self.selected.as_deref() == Some(row.path.as_path());
        let state = self.changes.get(&row.path).copied();
        // A closed folder with changes beneath it gets a quiet dot, so nothing hides in it.
        let changed_inside = row.is_dir && !row.expanded && git::has_changes_under(&self.changes, &row.path);
        let name_color = match state {
            _ if selected => color::INK,
            Some(FileState::Deleted) => color::INK_FAINT,
            _ => color::INK_MUTED,
        };
        let icon = match (row.is_dir, row.expanded) {
            (true, true) => FileIcon::Icon("folder-open"),
            (true, false) => FileIcon::Icon("folder"),
            (false, _) => file_icon(&row.name),
        };
        div()
            .id(("file", index))
            .w_full()
            .flex()
            .items_center()
            .gap(len(space::SPACE_2))
            .h(px(ROW_HEIGHT))
            .pl(px(f32::from(u16::try_from(row.depth).unwrap_or(u16::MAX)) * INDENT + 6.))
            .pr(len(space::SPACE_3))
            .rounded(theme::corner(radius::RADIUS_SM))
            .cursor_pointer()
            .text_scale(text::LABEL)
            .text_color(theme::color(name_color))
            // What git ignores (build output, node_modules, local env files) is listed, but the
            // whole row steps back, icon and all, so it reads as scenery.
            .when(row.ignored && !selected, |row| row.opacity(0.45))
            .when(selected, |row| row.bg(theme::fill_pressed()))
            .when(!selected, |row| row.hover(|s| s.bg(theme::fill_hover())))
            .child(div().w(px(12.)).flex_none().when(row.is_dir, |slot| {
                slot.child(chrome::icon(
                    if row.expanded { "chevron-down" } else { "chevron-right" },
                    icon::SM,
                    theme::color(color::INK_FAINT),
                ))
            }))
            .child(render_icon(icon))
            .child(div().flex_1().min_w_0().truncate().child(row.name.clone()))
            .children(state.map(|state| {
                div()
                    .flex_none()
                    .font_family(saturn_design::font::MONO)
                    .text_scale(text::MICRO)
                    .text_color(theme::color(badge_color(state)))
                    .child(state.badge())
            }))
            .when(changed_inside, |row| {
                row.child(
                    div()
                        .flex_none()
                        .size(px(saturn_design::size::DOT))
                        .map(crate::chrome::pill)
                        .bg(theme::color(color::AMBER_TEXT)),
                )
            })
            .on_mouse_down(MouseButton::Right, {
                let row = row.clone();
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(Some(row.clone()), event, cx);
                })
            })
            .on_drag(DraggedEntry { path: row.path.clone(), name: row.name.clone() }, |dragged, _, _, cx| {
                cx.new(|_| dragged.clone())
            })
            // A folder takes what's dropped on it; a file, the folder it's in.
            .drag_over::<DraggedEntry>(|style, _, _, _| style.bg(theme::fill_pressed()))
            .on_drop({
                let folder = if row.is_dir {
                    row.path.clone()
                } else {
                    row.path.parent().map(Path::to_owned).unwrap_or_default()
                };
                cx.listener(move |_, dragged: &DraggedEntry, _, cx| Self::move_into(&dragged.path, &folder, cx))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.click(&row, cx)))
            .into_any_element()
    }
}

/// A file's mark: a Lucide icon, or a language's letters in a small square (filled for code,
/// outlined for configuration formats), the icon's size and colour either way.
fn render_icon(icon: FileIcon) -> AnyElement {
    let tint = theme::color(color::INK_MUTED);
    let letters = |text: &'static str, filled: bool| {
        div()
            .flex_none()
            .size(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::corner(3.))
            .font_family(saturn_design::font::MONO)
            .font_weight(gpui_kit::FontWeight::BOLD)
            .text_size(px(if text.len() > 1 { 7. } else { 9. }))
            .line_height(px(14.)) // off-scale: a letter inside a 14px icon
            .map(|square| {
                if filled {
                    square.bg(tint).text_color(theme::color(color::SURFACE))
                } else {
                    square.border_1().border_color(tint).text_color(tint)
                }
            })
            .child(text)
            .into_any_element()
    };
    match icon {
        FileIcon::Icon(name) => chrome::icon(name, icon::MD, tint).into_any_element(),
        FileIcon::Language(text) => letters(text, true),
        FileIcon::Format(text) => letters(text, false),
    }
}

fn badge_color(state: FileState) -> saturn_design::Token {
    match state {
        FileState::Modified | FileState::Renamed => color::AMBER_TEXT,
        FileState::Added | FileState::Untracked => color::TERM_GREEN,
        FileState::Deleted | FileState::Conflicted => color::RED,
    }
}

/// A file or folder being dragged to another folder.
#[derive(Clone)]
struct DraggedEntry {
    path: PathBuf,
    name: String,
}

impl Render for DraggedEntry {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        chrome::floating(div())
            .px(len(space::SPACE_3))
            .py(len(space::SPACE_1))
            .rounded(theme::corner(radius::RADIUS_SM))
            .text_scale(text::LABEL)
            .text_color(theme::color(color::INK))
            .child(self.name.clone())
    }
}

/// Asks for a name in a dialog: `done` gets it, and the dialog stays open with what's wrong when
/// it says no.
fn ask_name(
    title: &'static str,
    note: String,
    initial: String,
    button: &'static str,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(String, &mut App) -> Result<(), String> + 'static,
) {
    let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    input.update(cx, |input, cx| input.focus(window, cx));
    let problem: std::rc::Rc<std::cell::RefCell<Option<String>>> = std::rc::Rc::default();
    let done = std::rc::Rc::new(done);
    window.open_dialog(cx, move |dialog, _, _| {
        let submit = {
            let (input, problem, done) = (input.clone(), problem.clone(), done.clone());
            move |window: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().to_string();
                match done(name, cx) {
                    Ok(()) => window.close_dialog(cx),
                    Err(why) => {
                        *problem.borrow_mut() = Some(why);
                        window.refresh();
                    }
                }
            }
        };
        let on_ok = submit.clone();
        let wrong = problem.borrow().clone();
        dialog
            .title(title)
            .entrance(gpui_kit::component::dialog::DialogEntrance::None)
            .bg(chrome::dialog_surface())
            .w(px(saturn_design::size::DIALOG_SMALL))
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                false
            })
            .child(
                div()
                    .pb(len(space::SPACE_2))
                    .text_scale(text::LABEL)
                    .text_color(theme::color(color::INK_MUTED))
                    .child(note.clone()),
            )
            .child(crate::chrome::input(&input))
            .children(wrong.map(|why| {
                div().pt(len(space::SPACE_2)).text_scale(text::LABEL).text_color(theme::color(color::RED)).child(why)
            }))
            .child(
                div().flex().justify_end().pt(len(space::SPACE_4)).child(
                    chrome::button(("ask-name", 0usize), button, None, true)
                        .on_click(move |_, window, cx| submit(window, cx)),
                ),
            )
    });
}

/// Moves `path` to the Trash, as Finder does.
fn trash(path: &Path) {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let _ = NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None);
}

impl Render for FilesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = self.tree.root().to_owned();
        div()
            .id("file-tree-area")
            .size_full()
            // A right click below the entries makes something at the top, and a drop there moves
            // what's dragged to the top.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.open_menu(None, event, cx)),
            )
            .on_drop(cx.listener(move |_, dragged: &DraggedEntry, _, cx| Self::move_into(&dragged.path, &root, cx)))
            .child(
                uniform_list(
                    "file-tree",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|index| this.render_row(index, cx)).collect::<Vec<_>>()
                    }),
                )
                // Edge to edge in the card, which pads it already: its rows line up with the tabs.
                .size_full(),
            )
            .children(self.render_menu(cx))
    }
}
