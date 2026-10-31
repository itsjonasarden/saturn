//! The editor panel: open files as tabs in its strip, the active one in its well, with syntax
//! highlighting from gpui-component's tree-sitter editor. A Changes tab, when open, reviews the
//! repository's diff with gpui-component's diff view.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::diff::{Diff, DiffFile, DiffMode, DiffState};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, px,
};
use saturn_core::format;
use saturn_core::git::{self, Scope};
use saturn_design::{color, radius, space};

use gpui_kit::component::tooltip::Tooltip;

use crate::chrome;
use crate::history_view::{HistoryEvent, HistoryView};
use crate::impact_view::ImpactView;
use crate::lineage_view::LineageView;
use crate::lsp::{self, Provider, Servers, Status};
use crate::preview_view::PreviewView;
use crate::problems_view::{self, ProblemsView};
use crate::references_view::ReferencesView;
use crate::review_comments::{ReviewComments, SendToAgent, Touched};
use crate::schema_view::SchemaView;
use crate::theme::TypeScale as _;
use crate::theme::{self, len};
use saturn_design::text;

/// Files larger than this open read-only elsewhere; the editor keeps the whole text in memory.
const MAX_SIZE: u64 = 4 * 1024 * 1024;

struct OpenFile {
    path: PathBuf,
    state: Entity<EditorState>,
    /// The text as it is on disk, to tell whether there are unsaved changes.
    saved: String,
    /// The file changed on disk while it had unsaved changes here; the user picks which to keep.
    changed_on_disk: bool,
    /// How its language server answers for it, when Saturn knows one for its language.
    language: Option<Status>,
    /// Asking its server for its problems, a moment after the last change.
    pull: Option<Task<()>>,
    _changes: Subscription,
}

impl OpenFile {
    fn document(&self) -> Option<&Arc<saturn_core::lsp::Document>> {
        match &self.language {
            Some(Status::Running(document)) => Some(document),
            _ => None,
        }
    }
}

impl OpenFile {
    fn name(&self) -> String {
        self.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    }

    fn dirty(&self, cx: &gpui_kit::App) -> bool {
        self.state.read(cx).value().as_ref() != self.saved
    }
}

/// A review of changes in the Changes tab: a scope's (kept current as agents work), or one
/// commit's (`scope` is `None`).
struct Review {
    state: Entity<DiffState>,
    scope: Option<Scope>,
    label: String,
    /// Comments for the agent, on a review of changes still being made (not on an old commit).
    comments: Option<(Entity<ReviewComments>, [Subscription; 2])>,
}

/// What the editor panel tells the status bar.
#[derive(Default)]
pub struct EditorStatus {
    pub errors: usize,
    pub warnings: usize,
    /// The cursor in the file in front, 1-based: line and column.
    pub cursor: Option<(u32, u32)>,
    /// The file in front's language, as people call it.
    pub language: Option<String>,
    /// Its language server: the server's name and how it's doing.
    pub server: Option<(String, ServerState)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Starting,
    Ready,
    /// Not installed, being installed, or stopped: the strip says which.
    Absent,
}

/// Renaming a symbol: the field in the strip, and where the symbol is.
struct Rename {
    input: Entity<InputState>,
    path: PathBuf,
    position: lsp_types::Position,
    _events: Subscription,
}

/// Which of the panel's tabs is in front.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Front {
    #[default]
    File,
    Review,
    History,
    Schema,
    Lineage,
    Impact,
    References,
    Problems,
    Preview,
}

#[derive(Default)]
pub struct EditorView {
    files: Vec<OpenFile>,
    /// The editor settings the open files were last given; the files follow a change on the next frame.
    applied: Option<crate::settings::EditorSettings>,
    active: usize,
    review: Option<Review>,
    front: Front,
    /// Every branch's commits as a graph, once opened.
    history: Option<Entity<HistoryView>>,
    /// Keeps History's clicks reaching this panel while it's open.
    history_events: Option<Subscription>,
    /// The database diagram, once opened.
    schema: Option<Entity<SchemaView>>,
    /// The running app's page, once opened.
    preview: Option<Entity<PreviewView>>,
    /// The dbt project's lineage graph, once opened.
    lineage: Option<Entity<LineageView>>,
    /// What the changes reach, once opened.
    impact: Option<Entity<ImpactView>>,
    /// Where a symbol is used, once asked.
    references: Option<Entity<ReferencesView>>,
    /// The rename in progress, if any.
    rename: Option<Rename>,
    /// Every problem the language servers report, by file.
    problems: BTreeMap<PathBuf, Vec<lsp_types::Diagnostic>>,
    /// The list of them, once opened.
    problems_view: Option<Entity<ProblemsView>>,
    /// The language servers, started with the first file that has one.
    servers: Option<Servers>,
    /// Applies what the servers report while the panel lives.
    server_events: Option<Task<()>>,
    /// Whether the panel takes the grid's room too, for a diagram or the history.
    maximized: bool,
    /// Why the last file could not be opened or saved, shown in the strip until the next action.
    problem: Option<String>,
}

impl EditorView {
    /// Opens `path` in a tab, or brings its tab forward if it is already open.
    pub fn open(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.problem = None;
        self.front = Front::File;
        if let Some(index) = self.files.iter().position(|file| file.path == path) {
            self.active = index;
            cx.notify();
            return;
        }
        let text = match std::fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_SIZE => Err(format!("{} is too large to edit here", display_name(path))),
            Ok(_) => std::fs::read_to_string(path).map_err(|_| format!("{} is not a text file", display_name(path))),
            Err(error) => Err(format!("{}: {error}", display_name(path))),
        };
        let text = match text {
            Ok(text) => text,
            Err(problem) => {
                self.problem = Some(problem);
                cx.notify();
                return;
            }
        };
        let language = language_for(path);
        let state = cx.new(|cx| {
            let mut state = EditorState::new(window, cx).language(language).searchable(true);
            let settings = cx.try_global::<crate::settings::EditorSettings>().cloned().unwrap_or_default();
            apply_settings(&mut state, &settings, window, cx);
            state.set_value(text.clone(), window, cx);
            state
        });
        let changes = cx.subscribe(&state, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(index) = this.files.iter().position(|file| file.state == state)
                    && let Some(document) = this.files[index].document()
                {
                    document.sync(state.read(cx).value().as_ref());
                    this.pull_diagnostics(index, Duration::from_millis(400), cx);
                }
                cx.notify();
            }
        });
        let language = self.start_language(path, &text, &state, cx);
        self.files.push(OpenFile {
            path: path.to_owned(),
            state,
            saved: text,
            changed_on_disk: false,
            language,
            pull: None,
            _changes: changes,
        });
        self.active = self.files.len() - 1;
        self.pull_diagnostics(self.active, Duration::ZERO, cx);
        cx.notify();
    }

    /// Asks a server that reports problems only when asked (TypeScript 7) for the file's, after
    /// `delay`; a newer change cancels the question.
    fn pull_diagnostics(&mut self, index: usize, delay: Duration, cx: &mut Context<Self>) {
        let Some(file) = self.files.get_mut(index) else { return };
        let Some(document) = file
            .document()
            .filter(|document| !document.server.is_ready() || document.server.can("diagnosticProvider"))
            .cloned()
        else {
            return;
        };
        file.pull = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let params = serde_json::json!({ "textDocument": { "uri": document.uri } });
            let Ok(answer) = document.server.request("textDocument/diagnostic", params).await else { return };
            // "unchanged" keeps what's shown.
            if answer.get("kind").and_then(serde_json::Value::as_str) != Some("full") {
                return;
            }
            let items = answer.get("items").cloned().unwrap_or_default();
            let diagnostics = serde_json::from_value(items).unwrap_or_default();
            let uri = document.uri.clone();
            let _ = this.update(cx, |this, cx| this.server_event(lsp::Event::Diagnostics { uri, diagnostics }, cx));
        }));
    }

    /// The text of the file in front, when a file is.
    pub fn front_state(&self) -> Option<Entity<EditorState>> {
        self.files.get(self.active).filter(|_| self.front == Front::File).map(|file| file.state.clone())
    }

    /// What the status bar shows about the panel.
    pub fn status(&self, cx: &gpui_kit::App) -> EditorStatus {
        let (errors, warnings) = problems_view::counts(&self.problems);
        let mut status = EditorStatus { errors, warnings, ..EditorStatus::default() };
        let Some(file) = self.files.get(self.active).filter(|_| self.front == Front::File) else { return status };
        let position = file.state.read(cx).cursor_position();
        status.cursor = Some((position.line + 1, position.character + 1));
        status.language = Some(language_name(language_for(&file.path)));
        status.server = file.language.as_ref().map(|language| match language {
            Status::Running(document) => {
                let state = if document.server.is_ready() { ServerState::Ready } else { ServerState::Starting };
                (document.server.name.to_owned(), state)
            }
            Status::Missing(spec) | Status::Installing(spec) => (spec.name.to_owned(), ServerState::Absent),
            Status::Failed(_) => (String::new(), ServerState::Absent),
        });
        status
    }

    /// Opens `path` with the cursor at `position` (go to definition, a reference).
    pub fn open_at(&mut self, path: &Path, position: lsp_types::Position, window: &mut Window, cx: &mut Context<Self>) {
        self.open(path, window, cx);
        if let Some(file) = self.files.iter().find(|file| file.path == path) {
            let state = file.state.clone();
            state.update(cx, |state, cx| {
                state.set_cursor_position(position, window, cx);
                state.focus(window, cx);
            });
        }
    }

    /// Asks the active file's server everything at `line`:`character` and prints the answers, then
    /// shows its references (spikes: hover, completion and definition need a person otherwise).
    pub fn probe_language(&mut self, line: u32, character: u32, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_kit::component::input::RopeExt as _;
        let Some(file) = self.files.get(self.active) else { return };
        let state = file.state.clone();
        let (editor, path) = (cx.weak_entity(), file.path.clone());
        let (text, providers) = {
            let state = state.read(cx);
            let lsp = state.lsp();
            let providers = (
                lsp.hover_provider.clone(),
                lsp.definition_provider.clone(),
                lsp.completion_provider.clone(),
                lsp.code_action_providers.first().cloned(),
            );
            (state.text().clone(), providers)
        };
        let offset = text.position_to_offset(&lsp_types::Position::new(line, character));
        let (hover, definition, completion, actions) = providers;
        let hover = hover.map(|provider| provider.hover(&text, offset, window, cx));
        let definition = definition.map(|provider| provider.definitions(&text, offset, window, cx));
        let context = lsp_types::CompletionContext {
            trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
            trigger_character: None,
        };
        let completion = completion.map(|provider| provider.completions(&text, offset, context, window, cx));
        let actions_task =
            actions.as_ref().map(|provider| provider.code_actions(state.clone(), offset..offset, window, cx));
        cx.spawn_in(window, async move |_, cx| {
            if let Some(Ok(Some(hover))) = match hover {
                Some(task) => Some(task.await),
                None => None,
            } {
                eprintln!(
                    "probe hover: {}",
                    serde_json::to_string(&hover.contents).unwrap_or_default().chars().take(160).collect::<String>()
                );
            }
            if let Some(Ok(links)) = match definition {
                Some(task) => Some(task.await),
                None => None,
            } {
                for link in links {
                    eprintln!(
                        "probe definition: {} line {}",
                        link.target_uri.as_str(),
                        link.target_range.start.line + 1
                    );
                }
            }
            if let Some(Ok(answer)) = match completion {
                Some(task) => Some(task.await),
                None => None,
            } {
                let items = match answer {
                    lsp_types::CompletionResponse::Array(items) => items,
                    lsp_types::CompletionResponse::List(list) => list.items,
                };
                let labels: Vec<String> = items.iter().take(6).map(|item| item.label.clone()).collect();
                eprintln!("probe completion: {} items, first {labels:?}", items.len());
            }
            if let Ok(name) = std::env::var("SATURN_LSP_RENAME") {
                let _ = cx.update(|window, cx| {
                    let _ = editor.update(cx, |editor, cx| {
                        let at = lsp_types::Position::new(line, character);
                        // "?" shows the rename field instead, for screenshots.
                        if name == "?" {
                            editor.start_rename(&path, at, "format", window, cx);
                        } else {
                            editor.rename_symbol(&path, at, &name, window, cx);
                        }
                    });
                });
            }
            if let (Some(provider), Some(Ok(actions))) = (
                actions,
                match actions_task {
                    Some(task) => Some(task.await),
                    None => None,
                },
            ) {
                eprintln!("probe actions: {:?}", actions.iter().map(|action| action.title.clone()).collect::<Vec<_>>());
                if let Some(references) = actions.into_iter().find(|action| action.title == "Find references") {
                    let task =
                        cx.update(|window, cx| provider.perform_code_action(state, references, false, window, cx));
                    if let Ok(task) = task {
                        eprintln!("probe references: {:?}", task.await.map(|()| "shown"));
                    }
                }
            }
        })
        .detach();
    }

    /// Opens `path` in its language's server and gives its editor the server's features.
    fn start_language(
        &mut self,
        path: &Path,
        text: &str,
        state: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> Option<Status> {
        if self.servers.is_none() {
            let (servers, events) = Servers::new();
            self.servers = Some(servers);
            self.server_events = Some(cx.spawn(async move |this, cx| {
                while let Ok(event) = events.recv().await {
                    if this.update(cx, |this, cx| this.server_event(event, cx)).is_err() {
                        break;
                    }
                }
            }));
        }
        let status = self.servers.as_mut()?.open(path, text)?;
        if std::env::var_os("SATURN_LSP_DEBUG").is_some() {
            let state = match &status {
                Status::Running(_) => "running".to_owned(),
                Status::Missing(spec) => format!("missing {}", spec.name),
                Status::Installing(spec) => format!("installing {}", spec.name),
                Status::Failed(problem) => problem.clone(),
            };
            eprintln!("lsp: {} → {state}", path.display());
        }
        if let Status::Running(document) = &status {
            Provider::attach(document.clone(), path, cx.weak_entity(), state, cx);
        }
        Some(status)
    }

    fn server_event(&mut self, event: lsp::Event, cx: &mut Context<Self>) {
        if std::env::var_os("SATURN_LSP_DEBUG").is_some() {
            match &event {
                lsp::Event::Diagnostics { uri, diagnostics } => {
                    eprintln!("lsp: {} problems in {uri}", diagnostics.len());
                }
                lsp::Event::Stopped { name, reason, .. } => {
                    eprintln!("lsp: {name} stopped: {reason:?}");
                }
            }
        }
        match event {
            lsp::Event::Diagnostics { uri, diagnostics } => {
                if let Some(path) = saturn_core::lsp::path(&uri) {
                    if diagnostics.is_empty() {
                        self.problems.remove(&path);
                    } else {
                        self.problems.insert(path, diagnostics.clone());
                    }
                    if let Some(view) = &self.problems_view {
                        let problems = self.problems.clone();
                        view.update(cx, |view, cx| view.set(problems, cx));
                    }
                    cx.notify();
                }
                // Matched ignoring case: on a case-insensitive disk a server may lowercase paths.
                let Some(file) =
                    self.files.iter().find(|file| file.document().is_some_and(|d| d.uri.eq_ignore_ascii_case(&uri)))
                else {
                    return;
                };
                // Least severe first, so where an error and a hint cover the same words, the
                // error's underline is the one drawn on top.
                let mut diagnostics = diagnostics;
                diagnostics.sort_by_key(|diagnostic| {
                    std::cmp::Reverse(diagnostic.severity.unwrap_or(lsp_types::DiagnosticSeverity::ERROR))
                });
                file.state.update(cx, |state, cx| {
                    let text = state.text().clone();
                    if let Some(set) = state.diagnostics_mut() {
                        set.reset(&text);
                        set.extend(diagnostics);
                    }
                    cx.notify();
                });
            }
            lsp::Event::Stopped { name, root, reason } => {
                if let Some(servers) = self.servers.as_mut() {
                    servers.forget(name, &root);
                }
                for file in &mut self.files {
                    if file
                        .document()
                        .is_some_and(|document| document.server.name == name && document.server.root == root)
                    {
                        let problem = reason.as_deref().map_or_else(|| format!("{name} stopped"), first_sentence);
                        file.language = Some(Status::Failed(problem));
                    }
                }
                cx.notify();
            }
        }
    }

    /// Installs the language server the active file is missing, then starts it for every open
    /// file of that language.
    fn install_language(&mut self, cx: &mut Context<Self>) {
        let Some(Status::Missing(spec)) = self.files.get(self.active).and_then(|file| file.language.clone()) else {
            return;
        };
        let missing = |file: &OpenFile| matches!(file.language, Some(Status::Missing(s)) if s.name == spec.name);
        for file in self.files.iter_mut().filter(|file| missing(file)) {
            file.language = Some(Status::Installing(spec));
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = cx.background_spawn(async move { lsp::install(spec) }).await;
            let _ = this.update(cx, |this, cx| {
                let installing =
                    |file: &OpenFile| matches!(file.language, Some(Status::Installing(s)) if s.name == spec.name);
                for index in 0..this.files.len() {
                    if !installing(&this.files[index]) {
                        continue;
                    }
                    let (path, state) = (this.files[index].path.clone(), this.files[index].state.clone());
                    let language = match &outcome {
                        Ok(()) => this.start_language(&path, &state.read(cx).value(), &state, cx),
                        Err(problem) => Some(Status::Failed(format!("Couldn't install {}: {problem}", spec.name))),
                    };
                    this.files[index].language = language;
                    this.pull_diagnostics(index, Duration::ZERO, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks for a symbol's new name, in a field in the strip.
    pub fn start_rename(
        &mut self,
        path: &Path,
        position: lsp_types::Position,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name.to_owned()));
        let events = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.finish_rename(window, cx),
            InputEvent::Blur => {
                this.rename = None;
                cx.notify();
            }
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.rename = Some(Rename { input, path: path.to_owned(), position, _events: events });
        cx.notify();
    }

    /// Renames the symbol everywhere the server finds it.
    fn finish_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else { return };
        cx.notify();
        let name = rename.input.read(cx).value().trim().to_owned();
        self.rename_symbol(&rename.path, rename.position, &name, window, cx);
    }

    /// Asks the server to rename the symbol at `position` in `path` to `name`, and makes its edit.
    pub fn rename_symbol(
        &mut self,
        path: &Path,
        position: lsp_types::Position,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.files.iter().find(|file| file.path == path).and_then(OpenFile::document).cloned();
        let Some(document) = document.filter(|_| !name.is_empty()) else { return };
        let mut params = document.at(position.line, position.character);
        params["newName"] = serde_json::json!(name);
        cx.spawn_in(window, async move |this, cx| {
            let answer = document.server.request("textDocument/rename", params).await;
            let _ = this.update_in(cx, |this, window, cx| match answer {
                Ok(answer) => {
                    if let Ok(Some(edit)) = serde_json::from_value::<Option<lsp_types::WorkspaceEdit>>(answer) {
                        this.apply_workspace_edit(&edit, window, cx);
                    }
                }
                Err(problem) => {
                    this.problem = Some(format!("Couldn't rename: {problem}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Makes a language server's edit: open files change in place (and are saved if they had no
    /// unsaved changes), the rest change on disk.
    pub fn apply_workspace_edit(
        &mut self,
        edit: &lsp_types::WorkspaceEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (path, edits) in lsp::all_edits(edit) {
            if let Some(index) = self.files.iter().position(|file| file.path == path) {
                let clean = !self.files[index].dirty(cx);
                let state = self.files[index].state.clone();
                state.update(cx, |state, cx| state.apply_lsp_edits(&edits, window, cx));
                if clean {
                    let text = state.read(cx).value().to_string();
                    if std::fs::write(&path, &text).is_ok() {
                        if let Some(document) = self.files[index].document() {
                            document.sync(&text);
                            document.saved();
                        }
                        self.files[index].saved = text;
                    }
                }
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                let _ = std::fs::write(&path, lsp::apply_to_text(&text, &edits));
            }
        }
        cx.notify();
    }

    /// The rename field, while renaming.
    fn render_rename(&self) -> Option<AnyElement> {
        let rename = self.rename.as_ref()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_2))
                .child(div().text_scale(text::META).text_color(theme::color(color::INK_MUTED)).child("Rename to"))
                .child(div().w(px(200.)).child(crate::chrome::input(&rename.input)))
                .into_any_element(),
        )
    }

    /// Shows every error and warning the servers report.
    pub fn show_problems(&mut self, cx: &mut Context<Self>) {
        let problems = self.problems.clone();
        if let Some(view) = &self.problems_view {
            view.update(cx, |view, cx| view.set(problems, cx));
        } else {
            let editor = cx.weak_entity();
            self.problems_view = Some(cx.new(|_| ProblemsView::new(problems, editor)));
        }
        self.front = Front::Problems;
        cx.notify();
    }

    /// The count of errors and warnings, in the strip; clicking lists them.
    fn render_problems_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (errors, warnings) = problems_view::counts(&self.problems);
        if errors + warnings == 0 {
            return None;
        }
        let count = |severity, count: usize| {
            let (icon, colour) = problems_view::mark(severity);
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_1))
                .child(chrome::icon(icon, 13., theme::color(colour)))
                .child(count.to_string())
        };
        Some(
            chrome::track_item("problems-button", false)
                .px(len(space::SPACE_2))
                .text_scale(text::META)
                .children((errors > 0).then(|| count(lsp_types::DiagnosticSeverity::ERROR, errors)))
                .children((warnings > 0).then(|| count(lsp_types::DiagnosticSeverity::WARNING, warnings)))
                .tooltip(chrome::tip("Errors and warnings the language servers found"))
                .on_click(cx.listener(|this, _, _, cx| this.show_problems(cx)))
                .into_any_element(),
        )
    }

    fn render_problems_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.problems_view.as_ref()?;
        let in_front = self.front == Front::Problems;
        Some(
            chrome::tab("problems-tab", in_front)
                .group("problems-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("circle-x", 13., theme::color(color::INK_MUTED)))
                .child("Problems")
                .child(
                    div()
                        .id("problems-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("problems-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.problems_view = None;
                            this.front = Front::File;
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::Problems;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// Shows where a symbol is used.
    pub fn show_references(&mut self, from: &Path, locations: Vec<lsp_types::Location>, cx: &mut Context<Self>) {
        let editor = cx.weak_entity();
        let from = from.to_owned();
        match &self.references {
            Some(references) => references.update(cx, |view, cx| view.set(&from, locations, cx)),
            None => self.references = Some(cx.new(|cx| ReferencesView::new(&from, locations, editor, cx))),
        }
        self.front = Front::References;
        cx.notify();
    }

    /// The active file's language server, when it needs a word: missing, installing, or stopped.
    fn render_language_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let file = self.files.get(self.active).filter(|_| self.front == Front::File)?;
        let note = |text: String, colour| {
            div().px(len(space::SPACE_2)).text_scale(text::META).text_color(theme::color(colour)).child(text)
        };
        let notice = match file.language.as_ref()? {
            Status::Running(_) => return None,
            Status::Missing(spec) if !spec.npm.is_empty() => div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_1))
                .child(note(format!("No {} language server", spec.name), color::INK_FAINT))
                .child(
                    chrome::button("install-language", "Install", None, false)
                        .tooltip(|window, cx| {
                            Tooltip::new("Installs it with npm into Saturn's own folder").build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.install_language(cx))),
                ),
            Status::Missing(spec) => {
                note(format!("Install {} for completions and errors", spec.servers[0].0), color::INK_FAINT)
            }
            Status::Installing(spec) => {
                note(format!("Installing the {} language server…", spec.name), color::INK_FAINT)
            }
            Status::Failed(problem) => note(problem.clone(), color::INK_MUTED),
        };
        Some(notice.into_any_element())
    }

    /// Files changed on disk: an open copy without unsaved changes reloads; one with them asks.
    pub fn files_changed(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        for file in &mut self.files {
            if !paths.contains(&file.path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file.path) else { continue };
            if text == file.saved {
                continue;
            }
            if file.state.read(cx).value().as_ref() == file.saved {
                file.state.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
                file.saved = text;
            } else {
                file.changed_on_disk = true;
            }
        }
        cx.notify();
    }

    /// Takes the disk's version of the active file, dropping the unsaved changes.
    fn reload_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.files.get_mut(self.active) else { return };
        if let Ok(text) = std::fs::read_to_string(&file.path) {
            file.state.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
            file.saved = text;
        }
        file.changed_on_disk = false;
        cx.notify();
    }

    /// Keeps the unsaved changes; saving will replace what is on disk.
    fn keep_active(&mut self, cx: &mut Context<Self>) {
        if let Some(file) = self.files.get_mut(self.active) {
            file.changed_on_disk = false;
        }
        cx.notify();
    }

    /// The question when the active file changed on disk under unsaved changes.
    fn render_disk_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let file = self.files.get(self.active).filter(|file| file.changed_on_disk && self.front == Front::File)?;
        Some(
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_1))
                .child(
                    div()
                        .px(len(space::SPACE_2))
                        .text_scale(text::META)
                        .text_color(theme::color(color::AMBER_TEXT))
                        .child(format!("{} changed on disk", file.name())),
                )
                .child(
                    chrome::button("disk-reload", "Reload", None, false)
                        .on_click(cx.listener(|this, _, window, cx| this.reload_active(window, cx))),
                )
                .child(
                    chrome::button("disk-keep", "Keep mine", None, false)
                        .on_click(cx.listener(|this, _, _, cx| this.keep_active(cx))),
                )
                .into_any_element(),
        )
    }

    /// Shows the changes in `repo` as a review, scrolled to `path`, which takes comments for the agent
    /// working there.
    pub fn review(&mut self, files: Vec<DiffFile>, scope: &Scope, path: &str, repo: &Path, cx: &mut Context<Self>) {
        let label = match scope {
            Scope::Uncommitted => "Uncommitted changes".to_owned(),
            Scope::Branch => "Branch changes".to_owned(),
            Scope::Against(target) => format!("Changes since {target}"),
        };
        self.show_review(files, Some(scope.clone()), label, path, cx);
        let Some(review) = &mut self.review else { return };
        let same = review.comments.as_ref().is_some_and(|(comments, _)| comments.read(cx).is_for(repo, scope));
        if !same {
            let comments = cx.new(|_| ReviewComments::new(repo.to_owned(), scope.clone()));
            let sent = cx.subscribe(&comments, |_, _, event: &SendToAgent, cx| {
                cx.emit(SendToAgent { repo: event.repo.clone(), message: event.message.clone() });
            });
            let touched = cx.subscribe(&comments, |this, _, event: &Touched, cx| {
                this.reread_review(&event.repo, cx);
                cx.emit(Touched { repo: event.repo.clone() });
            });
            review.comments = Some((comments, [sent, touched]));
        }
        if let Some((comments, _)) = &review.comments {
            comments.update(cx, ReviewComments::read_prompts);
        }
    }

    fn show_review(
        &mut self,
        files: Vec<DiffFile>,
        scope: Option<Scope>,
        label: String,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        let state = if let Some(review) = &mut self.review {
            review.scope = scope;
            review.label = label;
            review.state.update(cx, |state, cx| state.set_files(files, cx));
            review.state.clone()
        } else {
            let state = cx.new(|cx| DiffState::new(files, cx));
            self.review = Some(Review { state: state.clone(), scope, label, comments: None });
            state
        };
        state.update(cx, |state, cx| state.scroll_to_file(path, cx));
        self.front = Front::Review;
        cx.notify();
    }

    pub fn maximized(&self) -> bool {
        self.maximized
    }

    /// Makes the panel take the grid's room (for spikes and screenshots).
    pub fn maximize(&mut self, cx: &mut Context<Self>) {
        self.maximized = true;
        cx.notify();
    }

    /// Opens the Database tab for `repo`, reading its schema afresh.
    /// Opens the Preview tab on a pane's servers, or points the open one at them.
    pub fn show_preview(
        &mut self,
        servers: Vec<saturn_core::servers::Server>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &self.preview {
            Some(preview) => preview.update(cx, |preview, cx| preview.set_servers(servers, cx)),
            None => self.preview = Some(cx.new(|cx| PreviewView::new(servers, window, cx))),
        }
        self.front = Front::Preview;
        cx.notify();
    }

    fn render_preview_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.preview.as_ref()?;
        let in_front = self.front == Front::Preview;
        Some(
            chrome::tab("preview-tab", in_front)
                .group("preview-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("globe", 13., theme::color(color::INK_MUTED)))
                .child("Preview")
                .child(
                    div()
                        .id("preview-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("preview-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            // Closing it stops it drawing the page again.
                            this.preview = None;
                            this.front = if this.review.is_some() { Front::Review } else { Front::File };
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::Preview;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    pub fn show_schema(&mut self, repo: &Path, cx: &mut Context<Self>) {
        match &self.schema {
            Some(schema) if schema.read(cx).repo() == repo => schema.update(cx, SchemaView::reload),
            _ => self.schema = Some(cx.new(|cx| SchemaView::new(repo, cx))),
        }
        self.front = Front::Schema;
        cx.notify();
    }

    /// Opens the Lineage tab for the dbt project in `project`, reading it afresh, focused on the
    /// model in front in the editor if there is one.
    pub fn show_lineage(&mut self, project: &Path, cx: &mut Context<Self>) {
        match &self.lineage {
            Some(lineage) if lineage.read(cx).project() == project => lineage.update(cx, LineageView::reload),
            _ => {
                let editor = cx.weak_entity();
                self.lineage = Some(cx.new(|cx| LineageView::new(project, editor, cx)));
            }
        }
        if let (Some(lineage), Some(file)) = (&self.lineage, self.files.get(self.active).map(|file| file.path.clone()))
        {
            lineage.update(cx, |lineage, cx| lineage.focus_file(&file, cx));
        }
        self.front = Front::Lineage;
        cx.notify();
    }

    /// Opens the Impact tab for `repo`, reading the imports afresh, focused on `file` if given.
    pub fn show_impact(&mut self, repo: &Path, file: Option<PathBuf>, cx: &mut Context<Self>) {
        match &self.impact {
            Some(impact) if impact.read(cx).repo() == repo => impact.update(cx, ImpactView::reload),
            _ => {
                let editor = cx.weak_entity();
                self.impact = Some(cx.new(|cx| ImpactView::new(repo, editor, cx)));
            }
        }
        if let (Some(impact), Some(file)) = (&self.impact, file) {
            impact.update(cx, |impact, cx| impact.focus(file, cx));
        }
        self.front = Front::Impact;
        cx.notify();
    }

    /// Opens the History tab for `repo`, reading it afresh.
    pub fn show_history(&mut self, repo: &Path, cx: &mut Context<Self>) {
        match &self.history {
            Some(history) if history.read(cx).repo() == repo => history.update(cx, HistoryView::reload),
            _ => {
                let history = cx.new(|cx| HistoryView::new(repo, cx));
                self.history_events = Some(cx.subscribe(&history, |_, history, event: &HistoryEvent, cx| {
                    let HistoryEvent::Open { hash } = event;
                    let (repo, hash) = (history.read(cx).repo().to_owned(), hash.clone());
                    cx.spawn(async move |this, cx| {
                        let files = cx
                            .background_spawn({
                                let hash = hash.clone();
                                async move { DiffFile::parse(&git::commit_patch(&repo, &hash)).unwrap_or_default() }
                            })
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            let path = files.first().map(|file| file.path().to_string()).unwrap_or_default();
                            let label = format!("Commit {}", &hash[..hash.len().min(7)]);
                            this.show_review(files, None, label, &path, cx);
                        });
                    })
                    .detach();
                }));
                self.history = Some(history);
            }
        }
        self.front = Front::History;
        cx.notify();
    }

    /// Keeps an open review current while agents keep working.
    /// Reads the review's changes again, after Saturn itself changed files under it.
    fn reread_review(&mut self, repo: &Path, cx: &mut Context<Self>) {
        let Some(scope) = self.review.as_ref().and_then(|review| review.scope.clone()) else { return };
        let repo = repo.to_owned();
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn({
                    let scope = scope.clone();
                    async move { DiffFile::parse(&git::patch(&repo, scope)).unwrap_or_default() }
                })
                .await;
            let _ = this.update(cx, |this, cx| this.refresh_review(files, scope, cx));
        })
        .detach();
    }

    pub fn refresh_review(&mut self, files: Vec<DiffFile>, scope: Scope, cx: &mut Context<Self>) {
        if let Some(review) = self.review.as_ref().filter(|review| review.scope == Some(scope)) {
            review.state.update(cx, |state, cx| state.set_files(files, cx));
            // An agent at work: what it was asked may have grown too.
            if let Some((comments, _)) = &review.comments {
                comments.update(cx, ReviewComments::read_prompts);
            }
        }
    }

    /// Writes the active file to disk and, if `format` is on, runs the project's formatter on it.
    /// The formatted text comes back through the file watcher, like any change on disk.
    pub fn save(&mut self, root: &Path, format: bool, cx: &mut Context<Self>) {
        if self.front != Front::File {
            return;
        }
        let Some(file) = self.files.get_mut(self.active) else { return };
        let text = file.state.read(cx).value().to_string();
        let mut server = None;
        match std::fs::write(&file.path, &text) {
            Ok(()) => {
                server = file.document().map(|document| {
                    document.sync(&text);
                    document.saved();
                    document.server.clone()
                });
                file.saved = text;
                file.changed_on_disk = false;
                self.problem = None;
                if let Some(formatter) = format.then(|| format::formatter_for(&file.path, root)).flatten() {
                    Self::run_formatter(formatter, cx);
                }
            }
            Err(error) => self.problem = Some(format!("Could not save {}: {error}", file.name())),
        }
        // A save can fix or break the files that use this one.
        let ours = |file: &OpenFile| {
            file.document().is_some_and(|document| server.as_ref().is_some_and(|s| Arc::ptr_eq(s, &document.server)))
        };
        let indices: Vec<usize> = (0..self.files.len()).filter(|&index| ours(&self.files[index])).collect();
        for index in indices {
            self.pull_diagnostics(index, Duration::from_millis(200), cx);
        }
        cx.notify();
    }

    /// Runs a formatter off the main thread; a failure shows in the strip with its first line.
    fn run_formatter(formatter: format::Formatter, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let name = formatter.name;
            let outcome = cx
                .background_spawn(async move {
                    let mut command = std::process::Command::new(&formatter.program);
                    command.args(&formatter.args).current_dir(&formatter.dir);
                    // Started from Finder, Saturn's own PATH lacks node and rustfmt; the login
                    // shell's has them.
                    if let Some(path) = crate::LOGIN_PATH.get() {
                        command.env("PATH", path);
                    }
                    command.output()
                })
                .await;
            let problem = match outcome {
                Ok(output) if output.status.success() => None,
                Ok(output) => {
                    let errors = String::from_utf8_lossy(&output.stderr);
                    let first =
                        errors.lines().find(|line| !line.trim().is_empty()).unwrap_or("it exited with an error");
                    Some(format!("{name}: {}", first.trim()))
                }
                Err(error) => Some(format!("{name} could not run: {error}")),
            };
            if problem.is_some() {
                let _ = this.update(cx, |this, cx| {
                    this.problem = problem;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.files.len() {
            self.files.remove(index);
            if self.active >= self.files.len() {
                self.active = self.files.len().saturating_sub(1);
            }
            cx.notify();
        }
    }

    fn render_review_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let review = self.review.as_ref()?;
        let label = review.label.clone();
        let in_front = self.front == Front::Review;
        Some(
            chrome::tab("review-tab", in_front)
                .group("review-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("git-compare", 13., theme::color(color::INK_MUTED)))
                .child(label)
                .child(
                    div()
                        .id("review-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("review-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review = None;
                            this.front = Front::File;
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::Review;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn render_schema_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.schema.as_ref()?;
        let in_front = self.front == Front::Schema;
        Some(
            chrome::tab("schema-tab", in_front)
                .group("schema-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("database", 13., theme::color(color::INK_MUTED)))
                .child("Database")
                .child(
                    div()
                        .id("schema-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("schema-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.schema = None;
                            this.front = if this.review.is_some() { Front::Review } else { Front::File };
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::Schema;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn render_lineage_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.lineage.as_ref()?;
        let in_front = self.front == Front::Lineage;
        Some(
            chrome::tab("lineage-tab", in_front)
                .group("lineage-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("workflow", 13., theme::color(color::INK_MUTED)))
                .child("Lineage")
                .child(
                    div()
                        .id("lineage-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("lineage-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.lineage = None;
                            this.front = if this.review.is_some() { Front::Review } else { Front::File };
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::Lineage;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn render_impact_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.impact.as_ref()?;
        let in_front = self.front == Front::Impact;
        Some(
            chrome::tab("impact-tab", in_front)
                .group("impact-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("waypoints", 13., theme::color(color::INK_MUTED)))
                .child("Impact")
                .child(
                    div()
                        .id("impact-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("impact-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.impact = None;
                            this.front = if this.review.is_some() { Front::Review } else { Front::File };
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::Impact;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// For a file whose imports Saturn follows (JavaScript, TypeScript, Rust, Python): a button
    /// that shows what it uses and what uses it.
    fn render_impact_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let file = self.files.get(self.active).filter(|_| self.front == Front::File)?;
        if !saturn_core::imports::follows(&file.path) {
            return None;
        }
        let repo = file.path.ancestors().skip(1).find(|folder| folder.join(".git").exists())?.to_owned();
        let relative = file.path.strip_prefix(&repo).ok()?.to_owned();
        Some(
            chrome::icon_button("file-impact", "waypoints", true)
                .tooltip(chrome::tip("Dependencies: what this file uses, and what uses it"))
                .on_click(cx.listener(move |this, _, _, cx| this.show_impact(&repo, Some(relative.clone()), cx)))
                .into_any_element(),
        )
    }

    fn render_references_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.references.as_ref()?;
        let in_front = self.front == Front::References;
        Some(
            chrome::tab("references-tab", in_front)
                .group("references-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("search", 13., theme::color(color::INK_MUTED)))
                .child("References")
                .child(
                    div()
                        .id("references-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| {
                            slot.invisible().group_hover("references-tab", gpui_kit::Styled::visible)
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.references = None;
                            this.front = Front::File;
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::References;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn render_history_tab(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.history.as_ref()?;
        let in_front = self.front == Front::History;
        Some(
            chrome::tab("history-tab", in_front)
                .group("history-tab")
                .pl(len(space::SPACE_3))
                .pr(len(space::SPACE_1))
                .child(chrome::icon("git-branch", 13., theme::color(color::INK_MUTED)))
                .child("History")
                .child(
                    div()
                        .id("history-close")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(theme::corner(radius::RADIUS_XS))
                        .hover(|s| s.bg(theme::fill_control()))
                        .child(chrome::icon("x", 11., theme::color(color::INK_MUTED)))
                        .tooltip(chrome::tip("Close"))
                        .when(!in_front, |slot| slot.invisible().group_hover("history-tab", gpui_kit::Styled::visible))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.history = None;
                            this.history_events = None;
                            this.front = if this.review.is_some() { Front::Review } else { Front::File };
                            cx.notify();
                        })),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.front = Front::History;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// Unified or split, and stepping through the changes: the review's own controls.
    fn render_review_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let review = self.review.as_ref().filter(|_| self.front == Front::Review)?;
        let mode = review.state.read(cx).mode();
        let state = review.state.clone();
        let pick = |label: &'static str, target: DiffMode| {
            let state = state.clone();
            chrome::track_item(label, mode == target).px(len(space::SPACE_3)).child(label).on_click(move |_, _, cx| {
                state.update(cx, |state, cx| state.set_mode(target, cx));
            })
        };
        let step = |id: &'static str, icon: &str, next: bool| {
            let state = state.clone();
            chrome::icon_button(id, icon, true).on_click(move |_, _, cx| {
                state.update(cx, |state, cx| if next { state.next_change(cx) } else { state.previous_change(cx) });
            })
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(len(space::SPACE_1))
                .child(step("previous-change", "chevron-up", false))
                .child(step("next-change", "chevron-down", true))
                .child(
                    chrome::track()
                        .p(len(space::SPACE_0))
                        .child(pick("Unified", DiffMode::Unified))
                        .child(pick("Split", DiffMode::Split)),
                )
                .into_any_element(),
        )
    }

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let file = &self.files[index];
        let selected = index == self.active && self.front == Front::File;
        let dirty = file.dirty(cx);
        chrome::tab(("editor-tab", index), selected)
            .group("editor-tab")
            .pl(len(space::SPACE_3))
            .pr(len(space::SPACE_1))
            .child(chrome::icon("file", 13., theme::color(color::INK_MUTED)))
            .child(file.name())
            .child(
                div()
                    .id(("editor-close", index))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .rounded(theme::corner(radius::RADIUS_XS))
                    .hover(|s| s.bg(theme::fill_control()))
                    .map(|slot| {
                        // An unsaved file shows a dot where its close button is, until hovered.
                        if dirty {
                            slot.child(
                                div()
                                    .size(px(7.))
                                    .rounded_full()
                                    .bg(theme::color(color::INK))
                                    .group_hover("editor-tab", gpui_kit::Styled::invisible),
                            )
                        } else {
                            slot.child(chrome::icon("x", 11., theme::color(color::INK_MUTED))).when(!selected, |slot| {
                                slot.invisible().group_hover("editor-tab", gpui_kit::Styled::visible)
                            })
                        }
                    })
                    .tooltip(chrome::tip("Close"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.close(index, cx))),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.active = index;
                this.front = Front::File;
                cx.notify();
            }))
            .into_any_element()
    }
}

/// Gives a file's editor the user's editor settings.
fn apply_settings(
    state: &mut EditorState,
    settings: &crate::settings::EditorSettings,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    state.set_soft_wrap(settings.soft_wrap, window, cx);
    state.set_line_number(settings.line_numbers, window, cx);
    state.set_indent_guides(settings.indent_guides, window, cx);
    state.set_show_whitespaces(settings.show_whitespace, window, cx);
    state.set_tab_size(
        gpui_kit::component::input::TabSize { tab_size: usize::from(settings.tab_size), hard_tabs: false },
        cx,
    );
}

impl EventEmitter<SendToAgent> for EditorView {}
impl EventEmitter<Touched> for EditorView {}

impl EditorView {
    /// Clicks line `line` of the review's first file, or presses one of the picked change's buttons:
    /// for spikes (`SATURN_REVIEW_PICK`, `SATURN_REVIEW_ACT`).
    pub fn pick_in_review(&mut self, line: Option<usize>, press: Option<&str>, cx: &mut Context<Self>) {
        let Some(review) = &self.review else { return };
        let Some((comments, _)) = &review.comments else { return };
        let Some(path) = review.state.read(cx).files().first().map(|file| file.path().clone()) else { return };
        comments.update(cx, |comments, cx| {
            if let Some(line) = line {
                comments.pick(
                    &gpui_kit::component::diff::DiffLinePosition::new(
                        path,
                        gpui_kit::component::diff::DiffSide::Modified,
                        line,
                    ),
                    cx,
                );
            }
            if let Some(name) = press {
                comments.press(name, cx);
            }
        });
    }

    /// Comments on line `line` of the review's first file, then sends the comments when `send`: for
    /// spikes (`SATURN_REVIEW_COMMENT`, `SATURN_REVIEW_SEND`).
    pub fn comment_on_review(&mut self, line: usize, text: &str, send: bool, cx: &mut Context<Self>) {
        let Some(review) = &self.review else { return };
        let Some((comments, _)) = &review.comments else { return };
        let Some(path) = review.state.read(cx).files().first().map(|file| file.path().clone()) else { return };
        let range = gpui_kit::component::diff::DiffLineRange::new(
            path,
            gpui_kit::component::diff::DiffSide::Modified,
            line,
            line,
        );
        comments.update(cx, |comments, cx| {
            comments.comment(range, text, cx);
            if send {
                comments.send(cx);
            }
        });
    }
}

/// The review, with the comments for the agent when it takes them.
fn render_review(review: &Review, cx: &mut Context<EditorView>) -> AnyElement {
    let diff = Diff::new(&review.state).size_full();
    let Some((comments, _)) = &review.comments else { return diff.into_any_element() };
    let bar = comments.update(cx, |comments, cx| comments.render_bar(cx));
    let annotations = comments.read(cx).annotations();
    let (on_add, on_render, on_click, on_header) =
        (comments.clone(), comments.clone(), comments.clone(), comments.clone());
    let diff = diff
        .on_add_annotation(move |range, window, cx| {
            on_add.update(cx, |comments, cx| comments.start(range.clone(), window, cx));
        })
        .on_line_click(move |position, _, _, cx| {
            on_click.update(cx, |comments, cx| comments.pick(position, cx));
        })
        .render_header_title_suffix(move |file, _, cx| {
            on_header.read(cx).render_asked(file.path()).unwrap_or_else(|| div().into_any_element())
        })
        .annotations(annotations)
        .render_annotation(move |annotation, _, cx| {
            on_render.update(cx, |comments, cx| comments.render_annotation(annotation, cx))
        });
    div().flex().flex_col().size_full().children(bar).child(div().flex_1().min_h_0().child(diff)).into_any_element()
}

/// A view of the panel, when it's open.
fn shown<V: Render>(view: Option<&Entity<V>>) -> Option<AnyElement> {
    view.cloned().map(gpui_kit::IntoElement::into_any_element)
}

impl EditorView {
    /// What the tab in front shows: a file, a review, a diagram, the history, the preview…
    fn render_front(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match self.front {
            Front::Review => self.review.as_ref().map(|review| render_review(review, cx)),
            Front::History => shown(self.history.as_ref()),
            Front::Schema => shown(self.schema.as_ref()),
            Front::Preview => shown(self.preview.as_ref()),
            Front::Lineage => shown(self.lineage.as_ref()),
            Front::Impact => shown(self.impact.as_ref()),
            Front::References => shown(self.references.as_ref()),
            Front::Problems => shown(self.problems_view.as_ref()),
            Front::File => self
                .files
                .get(self.active)
                .map(|file| Editor::new(&file.state).appearance(false).bordered(false).size_full().into_any_element()),
        }
    }
}

impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wanted = cx.try_global::<crate::settings::EditorSettings>().cloned().unwrap_or_default();
        if self.applied.as_ref() != Some(&wanted) {
            for file in &self.files {
                file.state.update(cx, |state, cx| apply_settings(state, &wanted, window, cx));
            }
            self.applied = Some(wanted);
        }
        let body = self.render_front(cx);
        let Some(body) = body else {
            return chrome::card()
                .child(
                    div().relative().flex_1().min_h_0().flex().items_center().justify_center().child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(len(space::SPACE_2))
                            .child(
                                div()
                                    .text_scale(text::SUBTITLE)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::color(color::INK))
                                    .child("No file open"),
                            )
                            .child(
                                div().text_scale(text::BODY).text_color(theme::color(color::INK_MUTED)).child(
                                    self.problem.clone().unwrap_or_else(|| {
                                        "Pick a file in the sidebar. ⌘J hides this panel.".to_owned()
                                    }),
                                ),
                            ),
                    ),
                )
                .into_any_element();
        };
        chrome::panel()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(len(space::SPACE_0))
                    .h(len(chrome::CONTROL_HEIGHT))
                    .children(self.render_preview_tab(cx))
                    .children(self.render_schema_tab(cx))
                    .children(self.render_lineage_tab(cx))
                    .children(self.render_history_tab(cx))
                    .children(self.render_impact_tab(cx))
                    .children(self.render_references_tab(cx))
                    .children(self.render_problems_tab(cx))
                    .children(self.render_review_tab(cx))
                    .children((0..self.files.len()).map(|index| self.render_tab(index, cx)))
                    .child(div().flex_1())
                    .children(self.render_disk_notice(cx))
                    .children(self.render_rename())
                    .children(self.render_language_notice(cx))
                    .children(self.render_problems_button(cx))
                    .children(self.render_impact_button(cx))
                    .children(self.render_review_controls(cx))
                    .child(
                        chrome::corner_button(
                            "maximize-panel",
                            if self.maximized { "minimize-2" } else { "maximize-2" },
                            false,
                        )
                        .tooltip(chrome::tip("Make the editor bigger, or back"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.maximized = !this.maximized;
                            cx.notify();
                        })),
                    )
                    .children(self.problem.clone().map(|problem| {
                        div()
                            .px(len(space::SPACE_3))
                            .text_scale(text::META)
                            .text_color(theme::color(color::RED))
                            .child(problem)
                    })),
            )
            .child(chrome::well().child(body))
            .into_any_element()
    }
}

/// A server's reason, short enough for the strip: its first sentence, without the server's own
/// "Request initialize failed with message:" preamble.
fn first_sentence(reason: &str) -> String {
    // rustup's stand-in for a component that isn't installed.
    if reason.contains("Unknown binary 'rust-analyzer'") {
        return "rust-analyzer isn't installed: rustup component add rust-analyzer".to_owned();
    }
    let reason = reason.rsplit("message: ").next().unwrap_or(reason);
    let end = reason.find(". ").map_or(reason.len(), |end| end + 1);
    reason[..end].to_owned()
}

/// A grammar's name as people say it: `typescript` is TypeScript, `tsx` TypeScript JSX.
fn language_name(grammar: &str) -> String {
    let known = match grammar {
        "typescript" => "TypeScript",
        "tsx" => "TypeScript JSX",
        "javascript" => "JavaScript",
        "json" => "JSON",
        "yaml" => "YAML",
        "toml" => "TOML",
        "html" => "HTML",
        "css" => "CSS",
        "scss" => "SCSS",
        "sql" => "SQL",
        "bash" => "Shell",
        "cpp" => "C++",
        "csharp" => "C#",
        "objc" => "Objective-C",
        "graphql" => "GraphQL",
        "xml" => "XML",
        "text" | "" => "Plain text",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let mut chars = grammar.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The tree-sitter grammar for a file: by its whole name first (`Dockerfile`, `.gitignore`), then
/// by its extension. Files without a grammar of their own get the closest one that colours their
/// comments and strings: shell for `.env`, ignore files and Dockerfiles.
fn language_for(path: &Path) -> &'static str {
    let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.starts_with(".env") {
        return "bash";
    }
    match name.as_str() {
        "dockerfile" | "containerfile" | ".gitignore" | ".gitattributes" | ".dockerignore" | ".prettierignore"
        | ".npmignore" | ".zshrc" | ".bashrc" | ".zprofile" | ".bash_profile" | ".profile" => return "bash",
        "makefile" | "gnumakefile" => return "make",
        "gemfile" | "rakefile" | "podfile" | "brewfile" | "fastfile" | "appfile" | "matchfile" | "pluginfile"
        | "dangerfile" => return "ruby",
        ".editorconfig" | ".npmrc" | ".gitconfig" | ".gitmodules" => return "ini",
        "cargo.lock" | "poetry.lock" | "uv.lock" => return "toml",
        "cmakelists.txt" => return "cmake",
        "build.gradle" | "settings.gradle" | "jenkinsfile" => return "java",
        ".vimrc" | "_vimrc" => return "vim",
        "config.fish" => return "fish",
        _ if name.starts_with("dockerfile.") => return "bash",
        _ => {}
    }
    let extension = path.extension().map(|ext| ext.to_string_lossy().to_lowercase()).unwrap_or_default();
    match extension.as_str() {
        // Web
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" | "json5" | "jsonl" | "webmanifest" => "json",
        "css" | "less" => "css",
        "scss" => "scss",
        "html" | "htm" | "vue" => "html",
        "svelte" => "svelte",
        "astro" => "astro",
        "ejs" => "ejs",
        "erb" => "erb",
        "graphql" | "gql" => "graphql",
        "prisma" => "prisma",
        // Data and config
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "ini" | "cfg" | "conf" | "properties" => "ini",
        "xml" | "svg" | "plist" | "storyboard" | "xib" | "entitlements" | "xcprivacy" | "csproj" | "xaml" => "xml",
        "sql" => "sql",
        "md" | "mdx" | "markdown" => "markdown",
        "diff" | "patch" => "diff",
        "proto" => "proto",
        // Shells and build files
        "sh" | "bash" | "zsh" | "env" | "dockerfile" => "bash",
        "fish" => "fish",
        "ps1" | "psm1" | "psd1" => "powershell",
        "mk" => "make",
        "cmake" => "cmake",
        "nix" => "nix",
        "vim" => "vim",
        // Apple
        "swift" => "swift",
        "m" | "mm" => "objc",
        // Other languages
        "rs" => "rust",
        "py" | "pyi" => "python",
        "go" => "go",
        "rb" | "rake" | "gemspec" => "ruby",
        "php" => "php",
        "java" | "gradle" | "groovy" => "java",
        "kt" | "kts" => "kotlin",
        "scala" | "sc" => "scala",
        "dart" => "dart",
        "c" | "h" | "strings" | "xcconfig" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "cs" => "csharp",
        "ex" | "exs" | "heex" => "elixir",
        "erl" | "hrl" => "erlang",
        "gleam" => "gleam",
        "hs" | "lhs" => "haskell",
        "elm" => "elm",
        "r" => "r",
        "lua" => "lua",
        "zig" => "zig",
        "glsl" | "vert" | "frag" | "geom" | "comp" => "glsl",
        _ => "text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_follow_the_file_name() {
        assert_eq!(language_for(Path::new("/p/.env")), "bash");
        assert_eq!(language_for(Path::new("/p/.env.local")), "bash");
        assert_eq!(language_for(Path::new("/p/apps/api/orders.ts")), "typescript");
        assert_eq!(language_for(Path::new("/p/App.tsx")), "tsx");
        assert_eq!(language_for(Path::new("/p/Cargo.toml")), "toml");
        assert_eq!(language_for(Path::new("/p/README.MD")), "markdown");
        assert_eq!(language_for(Path::new("/p/.github/workflows/ci.yml")), "yaml");
        assert_eq!(language_for(Path::new("/p/Dockerfile")), "bash");
        assert_eq!(language_for(Path::new("/p/Dockerfile.dev")), "bash");
        assert_eq!(language_for(Path::new("/p/.gitignore")), "bash");
        assert_eq!(language_for(Path::new("/p/Makefile")), "make");
        assert_eq!(language_for(Path::new("/p/Cargo.lock")), "toml");
        assert_eq!(language_for(Path::new("/p/.editorconfig")), "ini");
        assert_eq!(language_for(Path::new("/p/ios/App/AppDelegate.swift")), "swift");
        assert_eq!(language_for(Path::new("/p/ios/App/Info.plist")), "xml");
        assert_eq!(language_for(Path::new("/p/ios/App/Bridge.m")), "objc");
        assert_eq!(language_for(Path::new("/p/app/lib/main.dart")), "dart");
        assert_eq!(language_for(Path::new("/p/android/build.gradle")), "java");
        assert_eq!(language_for(Path::new("/p/db/schema.prisma")), "prisma");
        assert_eq!(language_for(Path::new("/p/db/seed.sql")), "sql");
        assert_eq!(language_for(Path::new("/p/ios/fastlane/Fastfile")), "ruby");
        assert_eq!(language_for(Path::new("/p/LICENSE")), "text");
    }
}
