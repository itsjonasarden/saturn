//! Language servers in the editor: completions, hover, go to definition, quick fixes, references
//! and diagnostics, from the server each language uses (see `akyba_core::lsp`). One server runs
//! per language and repository, started when the first of its files opens.
//!
//! The editor component draws the menus and popovers; this module answers them from the server.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use akyba_core::lsp::{self as core, Document, Server, Spec};
use gpui_kit::component::input::{
    CodeActionProvider, CompletionProvider, DefinitionProvider, EditorState, HoverProvider, Rope, RopeExt as _,
};
use gpui_kit::{App, AppContext as _, Entity, SharedString, Task, WeakEntity, Window};
use lsp_types::{
    CodeAction, CodeActionOrCommand, Command, CompletionContext, CompletionResponse, CompletionTriggerKind,
    GotoDefinitionResponse, Hover, LocationLink, Position, ShowDocumentParams, TextEdit, WorkspaceEdit,
};
use serde_json::{Value, json};

use crate::editor_view::EditorView;

/// What the servers report, for the editor to show.
pub enum Event {
    /// The problems the server found in a file, replacing those it reported before.
    Diagnostics { uri: String, diagnostics: Vec<lsp_types::Diagnostic> },
    /// A server stopped (or never really started), with why when it said.
    Stopped { name: &'static str, root: PathBuf, reason: Option<String> },
}

/// How a file's language is served.
#[derive(Clone)]
pub enum Status {
    /// A server answers for it.
    Running(Arc<Document>),
    /// Akyba knows a server for it, but none is installed.
    Missing(&'static Spec),
    /// Akyba is installing its server.
    Installing(&'static Spec),
    /// Its server stopped, or wouldn't start.
    Failed(String),
}

/// The running servers, by language and repository.
pub struct Servers {
    running: HashMap<(&'static str, PathBuf), Arc<Server>>,
    events: async_channel::Sender<Event>,
}

impl Servers {
    /// The servers, and where their events arrive.
    pub fn new() -> (Self, async_channel::Receiver<Event>) {
        let (events, receiver) = async_channel::unbounded();
        (Self { running: HashMap::new(), events }, receiver)
    }

    /// Opens `path` (with its `text`) in its language's server, starting one if needed. `None`
    /// when Akyba knows no server for the language.
    pub fn open(&mut self, path: &Path, text: &str) -> Option<Status> {
        let spec = core::spec_for(path)?;
        let root = repository(path);
        if let Some(server) = self.running.get(&(spec.name, root.clone())).filter(|server| server.is_alive()) {
            return Some(Status::Running(Arc::new(Document::open(server.clone(), path, text))));
        }
        let path_env = login_path();
        let Some((program, args)) = core::find_server(spec, path, &root, &installed_bin(), &path_env) else {
            return Some(Status::Missing(spec));
        };
        let diagnostics = self.events.clone();
        let stopped = self.events.clone();
        let (name, stopped_root) = (spec.name, root.clone());
        let started = Server::start(
            spec.name,
            &program,
            args,
            &root,
            &path_env,
            move |method, params| {
                if method == "textDocument/publishDiagnostics" {
                    let uri = params.get("uri").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let diagnostics_list = params.get("diagnostics").cloned().unwrap_or_default();
                    let parsed = serde_json::from_value(diagnostics_list).unwrap_or_default();
                    let _ = diagnostics.try_send(Event::Diagnostics { uri, diagnostics: parsed });
                }
            },
            move |reason| {
                let _ = stopped.try_send(Event::Stopped { name, root: stopped_root, reason });
            },
            core::initialization_options(&program, path, &root),
            core::settings(spec),
        );
        match started {
            Ok(server) => {
                self.running.insert((spec.name, root), server.clone());
                Some(Status::Running(Arc::new(Document::open(server, path, text))))
            }
            Err(error) => Some(Status::Failed(format!("{} didn't start: {error}", spec.name))),
        }
    }

    /// Forgets a server that stopped, so the next file starts a fresh one.
    pub fn forget(&mut self, name: &str, root: &Path) {
        self.running.retain(|(running, at), _| !(*running == name && at == root));
    }
}

/// The repository a file belongs to (the folder with `.git`), or its own folder.
fn repository(path: &Path) -> PathBuf {
    path.ancestors()
        .skip(1)
        .find(|folder| folder.join(".git").exists())
        .or_else(|| path.parent())
        .unwrap_or(Path::new("/"))
        .to_owned()
}

/// The login shell's PATH: started from Finder, Akyba's own lacks node, cargo and Homebrew.
pub fn login_path() -> String {
    crate::LOGIN_PATH.get().cloned().unwrap_or_else(|| "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_owned())
}

/// Where Akyba installs servers that come from npm.
fn installed_root() -> PathBuf {
    crate::settings::data_dir().unwrap_or_else(std::env::temp_dir).join("servers")
}

fn installed_bin() -> PathBuf {
    installed_root().join("node_modules/.bin")
}

/// Installs `spec`'s server from npm into Akyba's own folder, off the main thread. The error is
/// npm's last line.
pub fn install(spec: &'static Spec) -> Result<(), String> {
    let root = installed_root();
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let output = std::process::Command::new("npm")
        .args(["install", "--no-fund", "--no-audit", "--prefix"])
        .arg(&root)
        .args(spec.npm)
        .env("PATH", login_path())
        .output()
        .map_err(|_| "npm isn't installed".to_owned())?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.lines().rfind(|line| !line.trim().is_empty()).unwrap_or("npm failed").to_owned())
    }
}

/// Answers the editor's menus and popovers for one open file from its server.
pub struct Provider {
    document: Arc<Document>,
    path: PathBuf,
    editor: WeakEntity<EditorView>,
}

impl Provider {
    /// Gives `state` its language features, answered by `document`'s server.
    pub fn attach(
        document: Arc<Document>,
        path: &Path,
        editor: WeakEntity<EditorView>,
        state: &Entity<EditorState>,
        cx: &mut App,
    ) {
        let provider = Rc::new(Self { document, path: path.to_owned(), editor: editor.clone() });
        let own = path.to_owned();
        state.update(cx, |state, cx| {
            let lsp = state.lsp_mut();
            lsp.completion_provider = Some(provider.clone());
            lsp.hover_provider = Some(provider.clone());
            lsp.definition_provider = Some(provider.clone());
            lsp.code_action_providers = vec![provider];
            // Go to definition in another file opens it.
            lsp.show_document = Some(Rc::new(move |params: &ShowDocumentParams, window: &mut Window, cx: &mut App| {
                let Some(target) = core::path(params.uri.as_str()) else { return false };
                if target == own {
                    return false;
                }
                let start = params.selection.map(|range| range.start).unwrap_or_default();
                editor.update(cx, |editor, cx| editor.open_at(&target, start, window, cx)).is_ok()
            }));
            state.refresh(cx);
        });
    }

    /// The server's request for where `offset` is in `text`, after telling it the text.
    fn at(&self, text: &Rope, offset: usize) -> Value {
        self.document.sync(&text.to_string());
        let position = text.offset_to_position(offset);
        self.document.at(position.line, position.character)
    }

    fn ask<T: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        method: &'static str,
        params: Value,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<T>>> {
        let server = self.document.server.clone();
        cx.background_spawn(async move {
            let answer = server.request(method, params).await.map_err(anyhow::Error::msg)?;
            Ok(serde_json::from_value(answer)?)
        })
    }
}

impl CompletionProvider for Provider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let mut params = self.at(text, offset);
        // The editor names the typed word as the trigger; a server only knows its own characters.
        let character = trigger.trigger_character.filter(|c| self.document.server.trigger_characters().contains(c));
        params["context"] = match character {
            Some(character) => {
                json!({ "triggerKind": CompletionTriggerKind::TRIGGER_CHARACTER, "triggerCharacter": character })
            }
            None => json!({ "triggerKind": CompletionTriggerKind::INVOKED }),
        };
        // The editor shows what it's given; servers send everything in scope and leave the
        // filtering to the client.
        let position = text.offset_to_position(offset);
        let line = text.slice_line(position.line as usize).to_string();
        let column = text.offset_to_point(offset).column;
        let typed = core::word_before(&line, column).to_owned();
        let answer = self.ask::<CompletionResponse>("textDocument/completion", params, cx);
        cx.background_spawn(async move {
            let items = match answer.await? {
                Some(CompletionResponse::Array(items)) => items,
                Some(CompletionResponse::List(list)) => list.items,
                None => Vec::new(),
            };
            let keys: Vec<(&str, &str)> = items
                .iter()
                .map(|item| {
                    let text = item.filter_text.as_deref().unwrap_or(&item.label);
                    (text, item.sort_text.as_deref().unwrap_or(&item.label))
                })
                .collect();
            let order = core::rank(&typed, &keys, 100);
            let mut items: Vec<Option<lsp_types::CompletionItem>> = items.into_iter().map(Some).collect();
            Ok(CompletionResponse::Array(order.into_iter().filter_map(|index| items[index].take()).collect()))
        })
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        let word = !new_text.is_empty() && new_text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$');
        word || self.document.server.trigger_characters().iter().any(|c| c == new_text)
    }
}

impl HoverProvider for Provider {
    fn hover(&self, text: &Rope, offset: usize, _: &mut Window, cx: &mut App) -> Task<anyhow::Result<Option<Hover>>> {
        let params = self.at(text, offset);
        self.ask("textDocument/hover", params, cx)
    }
}

impl DefinitionProvider for Provider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<LocationLink>>> {
        let params = self.at(text, offset);
        let answer = self.ask::<GotoDefinitionResponse>("textDocument/definition", params, cx);
        cx.background_spawn(async move {
            let links = match answer.await? {
                None => Vec::new(),
                Some(GotoDefinitionResponse::Link(links)) => links,
                Some(GotoDefinitionResponse::Scalar(location)) => vec![link(location)],
                Some(GotoDefinitionResponse::Array(locations)) => locations.into_iter().map(link).collect(),
            };
            Ok(links)
        })
    }
}

fn link(location: lsp_types::Location) -> LocationLink {
    LocationLink {
        origin_selection_range: None,
        target_uri: location.uri,
        target_range: location.range,
        target_selection_range: location.range,
    }
}

/// The commands behind the Find references and Rename entries: Akyba's own, never sent to the
/// server.
const REFERENCES: &str = "akyba.references";
const RENAME: &str = "akyba.rename";

impl CodeActionProvider for Provider {
    fn id(&self) -> SharedString {
        "lsp".into()
    }

    /// Find references, Rename, and the server's quick fixes (edits, not server commands).
    fn code_actions(
        &self,
        state: Entity<EditorState>,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        let text = state.read(cx).text().clone();
        self.document.sync(&text.to_string());
        let (start, end) = (text.offset_to_position(range.start), text.offset_to_position(range.end));
        let diagnostics: Vec<lsp_types::Diagnostic> = state
            .read(cx)
            .diagnostics()
            .map(|set| {
                set.range(range.clone())
                    .map(|entry| {
                        // What the server sent, back: its quick fixes match on the code and data.
                        let diagnostic = &entry.diagnostic;
                        lsp_types::Diagnostic {
                            range: lsp_types::Range { start: diagnostic.range.start, end: diagnostic.range.end },
                            message: diagnostic.message.to_string(),
                            code: diagnostic.code.as_ref().map(|code| match code.parse::<i32>() {
                                Ok(number) => lsp_types::NumberOrString::Number(number),
                                Err(_) => lsp_types::NumberOrString::String(code.to_string()),
                            }),
                            source: diagnostic.source.as_ref().map(ToString::to_string),
                            data: diagnostic.data.clone(),
                            ..lsp_types::Diagnostic::default()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let params = json!({
            "textDocument": { "uri": self.document.uri },
            "range": { "start": start, "end": end },
            "context": { "diagnostics": diagnostics, "triggerKind": 1 },
        });
        let wants_fixes = self.document.server.can("codeActionProvider");
        let answer = wants_fixes.then(|| self.ask::<Vec<CodeActionOrCommand>>("textDocument/codeAction", params, cx));
        let at = text.offset_to_position(range.start);
        let name = text.word_at(range.start);
        let ours = |title: &str, command: &str, arguments: Vec<Value>| CodeAction {
            title: title.to_owned(),
            command: Some(Command { title: title.to_owned(), command: command.to_owned(), arguments: Some(arguments) }),
            ..CodeAction::default()
        };
        let mut actions = vec![ours("Find references", REFERENCES, vec![json!(at)])];
        if !name.is_empty() && self.document.server.can("renameProvider") {
            actions.push(ours("Rename…", RENAME, vec![json!(at), json!(name)]));
        }
        cx.background_spawn(async move {
            if let Some(answer) = answer {
                actions.extend(answer.await.ok().flatten().unwrap_or_default().into_iter().filter_map(
                    |item| match item {
                        CodeActionOrCommand::CodeAction(action)
                            if action.command.is_none() && action.edit.is_some() =>
                        {
                            Some(action)
                        }
                        _ => None,
                    },
                ));
            }
            Ok(actions)
        })
    }

    fn perform_code_action(
        &self,
        _: Entity<EditorState>,
        action: CodeAction,
        _: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<()>> {
        if let Some(command) = action.command.as_ref().filter(|command| command.command == REFERENCES) {
            let at: Position = command
                .arguments
                .as_ref()
                .and_then(|arguments| arguments.first())
                .and_then(|argument| serde_json::from_value(argument.clone()).ok())
                .unwrap_or_default();
            let mut params = self.document.at(at.line, at.character);
            params["context"] = json!({ "includeDeclaration": false });
            let answer = self.ask::<Vec<lsp_types::Location>>("textDocument/references", params, cx);
            let (editor, path) = (self.editor.clone(), self.path.clone());
            return window.spawn(cx, async move |cx| {
                let locations = answer.await?.unwrap_or_default();
                editor.update(cx, |editor, cx| editor.show_references(&path, locations, cx))?;
                Ok(())
            });
        }
        let editor = self.editor.clone();
        if let Some(command) = action.command.as_ref().filter(|command| command.command == RENAME) {
            let arguments = command.arguments.clone().unwrap_or_default();
            let at: Position =
                arguments.first().and_then(|at| serde_json::from_value(at.clone()).ok()).unwrap_or_default();
            let name = arguments.get(1).and_then(Value::as_str).unwrap_or_default().to_owned();
            let path = self.path.clone();
            // After the menu has closed, so the rename field keeps the focus.
            window.defer(cx, move |window, cx| {
                let _ = editor.update(cx, |editor, cx| editor.start_rename(&path, at, &name, window, cx));
            });
            return Task::ready(Ok(()));
        }
        if let Some(edit) = action.edit {
            // Outside this editor's update: the edit may change this file too.
            window.defer(cx, move |window, cx| {
                let _ = editor.update(cx, |editor, cx| editor.apply_workspace_edit(&edit, window, cx));
            });
        }
        Task::ready(Ok(()))
    }
}

/// A workspace edit's edits, by file, each file's last first so earlier ones keep their
/// positions. Creating, renaming and deleting files are left out.
pub fn all_edits(edit: &WorkspaceEdit) -> Vec<(PathBuf, Vec<TextEdit>)> {
    let mut by_file: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
    for (uri, edits) in edit.changes.iter().flatten() {
        if let Some(path) = core::path(uri.as_str()) {
            by_file.entry(path).or_default().extend(edits.iter().cloned());
        }
    }
    let documents = match &edit.document_changes {
        Some(lsp_types::DocumentChanges::Edits(documents)) => documents.clone(),
        Some(lsp_types::DocumentChanges::Operations(operations)) => operations
            .iter()
            .filter_map(|operation| match operation {
                lsp_types::DocumentChangeOperation::Edit(document) => Some(document.clone()),
                lsp_types::DocumentChangeOperation::Op(_) => None,
            })
            .collect(),
        None => Vec::new(),
    };
    for document in documents {
        if let Some(path) = core::path(document.text_document.uri.as_str()) {
            by_file.entry(path).or_default().extend(document.edits.iter().map(|edit| match edit {
                lsp_types::OneOf::Left(edit) => edit.clone(),
                lsp_types::OneOf::Right(annotated) => annotated.text_edit.clone(),
            }));
        }
    }
    let mut files: Vec<(PathBuf, Vec<TextEdit>)> = by_file.into_iter().collect();
    for (_, edits) in &mut files {
        edits.sort_by(|a, b| {
            (b.range.start.line, b.range.start.character).cmp(&(a.range.start.line, a.range.start.character))
        });
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// `text` with `edits` made, last first (as `all_edits` orders them).
pub fn apply_to_text(text: &str, edits: &[TextEdit]) -> String {
    let mut rope = Rope::from_str(text);
    for edit in edits {
        let start = rope.position_to_offset(&edit.range.start);
        let end = rope.position_to_offset(&edit.range.end);
        rope.replace(start..end, &edit.new_text);
    }
    rope.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_edits_last_first() {
        let edit = |line, from, to, text: &str| TextEdit {
            range: lsp_types::Range { start: Position::new(line, from), end: Position::new(line, to) },
            new_text: text.to_owned(),
        };
        let text = "const total = 1;\nuse(total, total);\n";
        let edits = [edit(1, 11, 16, "sum"), edit(1, 4, 9, "sum"), edit(0, 6, 11, "sum")];
        assert_eq!(apply_to_text(text, &edits), "const sum = 1;\nuse(sum, sum);\n");
    }
}
