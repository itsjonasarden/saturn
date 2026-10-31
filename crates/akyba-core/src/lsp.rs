//! A language server client: the server's process, JSON-RPC over its stdin and stdout, and which
//! server each language uses. It speaks plain JSON (`serde_json::Value`); the editor turns the
//! answers into its own types.
//!
//! Messages sent before the server has answered `initialize` wait, then go in order: callers can
//! open documents and ask questions as soon as they have a server.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

/// A language Akyba can start a server for, and the servers that speak it, in order of preference.
#[derive(Debug)]
pub struct Spec {
    /// What the editor calls it: "TypeScript", "Rust".
    pub name: &'static str,
    extensions: &'static [&'static str],
    /// The programs that serve it, each with its arguments, wherever they are installed.
    pub servers: &'static [(&'static str, &'static [&'static str])],
    /// The server Akyba installs itself, found only in its own folder (TypeScript 7's `tsc`
    /// serves; a project's own older `tsc` doesn't).
    pub installed: Option<(&'static str, &'static [&'static str])>,
    /// The npm packages Akyba installs it from, when it comes from npm.
    pub npm: &'static [&'static str],
}

pub const SPECS: [Spec; 4] = [
    Spec {
        name: "TypeScript",
        extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        servers: &[("typescript-language-server", &["--stdio"]), ("vtsls", &["--stdio"])],
        installed: Some(("tsc", &["--lsp", "--stdio"])),
        npm: &["typescript-language-server", "typescript@7"],
    },
    Spec { name: "Rust", extensions: &["rs"], servers: &[("rust-analyzer", &[])], installed: None, npm: &[] },
    Spec {
        name: "Python",
        extensions: &["py", "pyi"],
        servers: &[("basedpyright-langserver", &["--stdio"]), ("pyright-langserver", &["--stdio"]), ("pylsp", &[])],
        installed: None,
        npm: &["pyright"],
    },
    Spec { name: "Go", extensions: &["go"], servers: &[("gopls", &[])], installed: None, npm: &[] },
];

/// The language a file is in, if Akyba knows a server for it.
pub fn spec_for(path: &Path) -> Option<&'static Spec> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    SPECS.iter().find(|spec| spec.extensions.contains(&ext.as_str()))
}

/// The LSP language id of a file: what `textDocument/didOpen` says it is.
pub fn language_id(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or_default() {
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "rs" => "rust",
        "py" | "pyi" => "python",
        "go" => "go",
        _ => "plaintext",
    }
}

/// Where a server for `spec` is, looking in the project's own `node_modules/.bin` (from `file`'s
/// folder up to `root`), then `extra` (Akyba's own installs), then each folder of `path`; last,
/// the one Akyba installs, in `extra` alone.
pub fn find_server(
    spec: &Spec,
    file: &Path,
    root: &Path,
    extra: &Path,
    path: &str,
) -> Option<(PathBuf, &'static [&'static str])> {
    let mut folders: Vec<PathBuf> = file
        .ancestors()
        .skip(1)
        .take_while(|folder| folder.starts_with(root))
        .map(|folder| folder.join("node_modules/.bin"))
        .collect();
    folders.push(extra.to_owned());
    folders.extend(std::env::split_paths(path));
    // typescript-language-server runs the project's own TypeScript, so it needs one; without,
    // Akyba's TypeScript 7 serves.
    let usable = |program: &str| program != "typescript-language-server" || project_tsserver(file, root).is_some();
    let anywhere = spec.servers.iter().filter(|(program, _)| usable(program)).find_map(|(program, args)| {
        folders
            .iter()
            .map(|folder| folder.join(program))
            .find(|candidate| candidate.is_file())
            .map(|found| (found, *args))
    });
    anywhere.or_else(|| {
        let (program, args) = spec.installed?;
        let found = extra.join(program);
        found.is_file().then_some((found, args))
    })
}

/// The settings a server asks for (`workspace/configuration`), by section. For TypeScript:
/// renaming an imported name renames it everywhere, not just here as an alias.
pub fn settings(spec: &Spec) -> Value {
    if spec.name != "TypeScript" {
        return Value::Null;
    }
    let language = json!({ "preferences": { "useAliasesForRenames": false } });
    json!({ "typescript": language, "javascript": language, "js/ts": language })
}

/// The project's own TypeScript server: the nearest `node_modules/typescript` from `file` up to
/// `root` that has one (TypeScript 7 has none; its `tsc` serves instead).
pub fn project_tsserver(file: &Path, root: &Path) -> Option<PathBuf> {
    file.ancestors()
        .skip(1)
        .take_while(|folder| folder.starts_with(root))
        .map(|folder| folder.join("node_modules/typescript/lib/tsserver.js"))
        .find(|candidate| candidate.is_file())
}

/// What a server needs told when it starts: for typescript-language-server, the project's own
/// TypeScript.
pub fn initialization_options(program: &Path, file: &Path, root: &Path) -> Value {
    if program.file_name().is_some_and(|name| name == "typescript-language-server")
        && let Some(path) = project_tsserver(file, root)
    {
        return json!({ "tsserver": { "path": path } });
    }
    Value::Null
}

/// A `file://` URI for a path, its bytes outside the unreserved set percent-encoded.
pub fn uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// The path a `file://` URI names.
pub fn path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut chars = encoded.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let hex = [chars.next()?, chars.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().map(|path| true_case(Path::new(&path)))
}

/// `path` as the disk spells it. On a case-insensitive disk a server may lowercase the paths it
/// sends; each name is matched exactly if it can be, else ignoring case.
pub fn true_case(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        let name = component.as_os_str();
        let names: Vec<std::ffi::OsString> = match std::fs::read_dir(&out) {
            Ok(entries) if !out.as_os_str().is_empty() => entries.flatten().map(|entry| entry.file_name()).collect(),
            _ => Vec::new(),
        };
        let wanted = name.to_string_lossy().to_lowercase();
        let spelled = if names.iter().any(|entry| entry == name) {
            None
        } else {
            names.into_iter().find(|entry| entry.to_string_lossy().to_lowercase() == wanted)
        };
        match spelled {
            Some(spelled) => out.push(spelled),
            None => out.push(name),
        }
    }
    out
}

/// One message on the wire: a `Content-Length` header, a blank line, the JSON.
pub fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Reads the next message, or `None` when the stream has ended.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a message without a length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

type Reply = async_channel::Sender<Result<Value, String>>;

/// What the server is sent: written now, or held until it has answered `initialize`.
#[derive(Default)]
struct Outbox {
    ready: bool,
    held: Vec<Value>,
}

/// A running language server.
pub struct Server {
    /// What the editor calls its language.
    pub name: &'static str,
    pub root: PathBuf,
    stdin: Mutex<Option<ChildStdin>>,
    outbox: Mutex<Outbox>,
    pending: Mutex<HashMap<i64, Reply>>,
    next_id: AtomicI64,
    /// Cleared when the server's output ends: it has stopped, or never really started.
    alive: AtomicBool,
    /// Why the server refused to start, when it answered `initialize` with an error.
    failure: Mutex<Option<String>>,
    /// What `workspace/configuration` answers, by section.
    settings: Value,
    child: Mutex<Child>,
    /// The server's capabilities, once it has answered `initialize`.
    capabilities: Mutex<Value>,
}

impl Server {
    /// Starts `program` for the project at `root`, and initializes it. Notifications from the
    /// server (diagnostics, mostly) go to `on_notification` on the reading thread; when the server
    /// stops, `on_exit` is called there too, with why if it refused to start. `options` are its
    /// `initializationOptions`.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        name: &'static str,
        program: &Path,
        args: &[&str],
        root: &Path,
        path_env: &str,
        on_notification: impl Fn(&str, Value) + Send + 'static,
        on_exit: impl FnOnce(Option<String>) + Send + 'static,
        options: Value,
        settings: Value,
    ) -> io::Result<Arc<Self>> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .env("PATH", path_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("no stdout"))?;
        let stderr = child.stderr.take();
        let server = Arc::new(Self {
            name,
            root: root.to_owned(),
            stdin: Mutex::new(child.stdin.take()),
            outbox: Mutex::new(Outbox::default()),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(1),
            alive: AtomicBool::new(true),
            failure: Mutex::new(None),
            settings,
            child: Mutex::new(child),
            capabilities: Mutex::new(Value::Null),
        });
        // The last thing the server said on stderr: why it stopped, when it stops before starting.
        let last_words = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = stderr {
            let last_words = last_words.clone();
            std::thread::Builder::new().name(format!("lsp {name} stderr")).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        line.trim().clone_into(&mut lock(&last_words));
                    }
                }
            })?;
        }
        let reader = Arc::downgrade(&server);
        std::thread::Builder::new().name(format!("lsp {name}")).spawn(move || {
            let mut stdout = BufReader::new(stdout);
            let debug = std::env::var_os("AKYBA_LSP_DEBUG").is_some();
            while let Ok(Some(message)) = read_message(&mut stdout) {
                if debug {
                    eprintln!("lsp ← {message}");
                }
                let Some(server) = reader.upgrade() else { break };
                server.receive(&message, &on_notification);
            }
            if let Some(server) = reader.upgrade() {
                server.alive.store(false, Ordering::SeqCst);
                // Whoever is waiting for an answer gets one.
                for (_, reply) in lock(&server.pending).drain() {
                    let _ = reply.try_send(Err(format!("{name} stopped")));
                }
            }
            let failure = reader.upgrade().and_then(|server| {
                let refused = lock(&server.failure).take();
                // Stopped before it was ready: what it last said is why.
                let early = (!server.is_ready()).then(|| {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    lock(&last_words).clone()
                });
                refused.or(early.filter(|words| !words.is_empty()))
            });
            on_exit(failure);
        })?;
        server.write(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": initialize_params(root, options),
        }));
        Ok(server)
    }

    /// Asks the server something and waits for its answer.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = async_channel::bounded(1);
        lock(&self.pending).insert(id, reply);
        // Checked after the reply is filed, so a server that stops meanwhile still answers it.
        if !self.is_alive() {
            lock(&self.pending).remove(&id);
            return Err(format!("{} stopped", self.name));
        }
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        answer.recv().await.unwrap_or_else(|_| Err(format!("{} stopped", self.name)))
    }

    /// Whether the server has answered `initialize`, so its capabilities are known.
    pub fn is_ready(&self) -> bool {
        lock(&self.outbox).ready
    }

    /// Whether the server is still running.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Tells the server something, without an answer.
    pub fn notify(&self, method: &str, params: Value) {
        let mut message = json!({ "jsonrpc": "2.0", "method": method });
        message["params"] = params;
        self.send(message);
    }

    /// Whether the server says it can do `capability` (`"referencesProvider"`, …).
    pub fn can(&self, capability: &str) -> bool {
        lock(&self.capabilities).get(capability).is_some_and(|value| !value.is_null() && *value != json!(false))
    }

    /// The characters after which the server offers completions.
    pub fn trigger_characters(&self) -> Vec<String> {
        lock(&self.capabilities)
            .pointer("/completionProvider/triggerCharacters")
            .and_then(Value::as_array)
            .map(|chars| chars.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn send(&self, message: Value) {
        let mut outbox = lock(&self.outbox);
        if outbox.ready {
            drop(outbox);
            self.write(&message);
        } else {
            outbox.held.push(message);
        }
    }

    fn write(&self, message: &Value) {
        if let Some(stdin) = lock(&self.stdin).as_mut() {
            let _ = stdin.write_all(&frame(message)).and_then(|()| stdin.flush());
        }
    }

    fn receive(&self, message: &Value, on_notification: &impl Fn(&str, Value)) {
        let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = message.get("id").cloned();
        match (method, id) {
            // An answer to the client's `initialize`: the server is ready for everything held back.
            (None, Some(id)) if id == json!(0) => {
                if let Some(error) = message.get("error") {
                    let reason = error.get("message").and_then(Value::as_str).unwrap_or("it refused to start");
                    *lock(&self.failure) = Some(reason.to_owned());
                    self.stop();
                    return;
                }
                *lock(&self.capabilities) = message.pointer("/result/capabilities").cloned().unwrap_or_default();
                self.write(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
                let mut outbox = lock(&self.outbox);
                for held in std::mem::take(&mut outbox.held) {
                    self.write(&held);
                }
                outbox.ready = true;
            }
            (None, Some(id)) => {
                let reply = id.as_i64().and_then(|id| lock(&self.pending).remove(&id));
                if let Some(reply) = reply {
                    let answer = match message.get("error") {
                        Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("failed").to_owned()),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = reply.try_send(answer);
                }
            }
            // The server asking the client: configuration gets each section's settings (or none),
            // the rest (progress tokens, registrations) a plain yes.
            (Some(method), Some(id)) => {
                let result = if method == "workspace/configuration" {
                    let items = message.pointer("/params/items").and_then(Value::as_array).cloned().unwrap_or_default();
                    let section = |item: &Value| {
                        let name = item.get("section").and_then(Value::as_str).unwrap_or_default();
                        self.settings.get(name).cloned().unwrap_or(Value::Null)
                    };
                    Value::Array(items.iter().map(section).collect())
                } else {
                    Value::Null
                };
                self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            (Some(method), None) => on_notification(&method, message.get("params").cloned().unwrap_or_default()),
            (None, None) => {}
        }
    }

    /// Asks the server to stop, and makes sure it does.
    pub fn stop(&self) {
        self.write(&json!({ "jsonrpc": "2.0", "id": -1, "method": "shutdown" }));
        self.write(&json!({ "jsonrpc": "2.0", "method": "exit" }));
        lock(&self.stdin).take();
        let _ = lock(&self.child).kill();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What Akyba tells a server it can do: the editor's features, nothing it would ignore.
fn initialize_params(root: &Path, options: Value) -> Value {
    let name = root.file_name().map_or_else(|| "project".to_owned(), |name| name.to_string_lossy().into_owned());
    let mut params = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "Akyba" },
        "rootUri": uri(root),
        "workspaceFolders": [{ "uri": uri(root), "name": name }],
        "capabilities": {
            "general": { "positionEncodings": ["utf-16"] },
            "workspace": { "configuration": true, "workspaceFolders": true },
            "window": { "workDoneProgress": false },
            "textDocument": {
                "synchronization": { "didSave": true, "dynamicRegistration": false },
                "completion": {
                    "completionItem": {
                        "snippetSupport": false,
                        "documentationFormat": ["markdown", "plaintext"],
                        "labelDetailsSupport": true
                    },
                    "contextSupport": true
                },
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "definition": { "linkSupport": true },
                "references": {},
                "publishDiagnostics": { "relatedInformation": false, "versionSupport": false }
            }
        }
    });
    params["initializationOptions"] = options;
    params
}

/// A file open in a server: what it was last told the text is, so it's told only of changes.
pub struct Document {
    pub server: Arc<Server>,
    pub uri: String,
    synced: Mutex<(i64, String)>,
}

impl Document {
    pub fn open(server: Arc<Server>, path: &Path, text: &str) -> Self {
        let uri = uri(path);
        server.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": language_id(path), "version": 1, "text": text } }),
        );
        Self { server, uri, synced: Mutex::new((1, text.to_owned())) }
    }

    /// Tells the server the text, if it has changed since it was last told.
    pub fn sync(&self, text: &str) {
        let mut synced = lock(&self.synced);
        if synced.1 == text {
            return;
        }
        synced.0 += 1;
        text.clone_into(&mut synced.1);
        self.server.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": self.uri, "version": synced.0 },
                "contentChanges": [{ "text": text }],
            }),
        );
    }

    pub fn saved(&self) {
        self.server.notify("textDocument/didSave", json!({ "textDocument": { "uri": self.uri } }));
    }

    /// Where the cursor is, as a request's `textDocument` and `position`.
    pub fn at(&self, line: u32, character: u32) -> Value {
        json!({ "textDocument": { "uri": self.uri }, "position": { "line": line, "character": character } })
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        self.server.notify("textDocument/didClose", json!({ "textDocument": { "uri": self.uri } }));
    }
}

/// The typed word that ends at byte `end` of `line`: the letters, digits, `_` and `$` before it.
pub fn word_before(line: &str, end: usize) -> &str {
    let before = &line[..end.min(line.len())];
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '$')
        .last()
        .map_or(before.len(), |(at, _)| at);
    &before[start..]
}

/// Which completions match `typed`, best first, at most `limit`: each candidate is its text to
/// match and the server's own sort key. A prefix in the same case beats one in any case, which
/// beats the typed letters starting words (`gEBI`, `get_by`), which beats them merely in order.
/// Ties keep the server's order.
pub fn rank(typed: &str, candidates: &[(&str, &str)], limit: usize) -> Vec<usize> {
    let typed_lower = typed.to_lowercase();
    let mut scored: Vec<(u8, usize, &str, usize)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, (text, sort))| {
            let tier = if typed.is_empty() || text.starts_with(typed) {
                0
            } else if text.to_lowercase().starts_with(&typed_lower) {
                1
            } else if starts_words(text, &typed_lower) {
                2
            } else if in_order(&text.to_lowercase(), &typed_lower) {
                3
            } else {
                return None;
            };
            Some((tier, text.len(), *sort, index))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(b.2)).then(a.1.cmp(&b.1)).then(a.3.cmp(&b.3)));
    scored.into_iter().take(limit).map(|(.., index)| index).collect()
}

/// Whether each typed letter starts a word of `text`, in order (camelCase humps, `_` and `-`
/// parts), the first letter at the start.
fn starts_words(text: &str, typed: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let starts: Vec<char> = chars
        .iter()
        .enumerate()
        .filter(|(at, c)| {
            *at == 0 || (c.is_uppercase() && !chars[at - 1].is_uppercase()) || matches!(chars[at - 1], '_' | '-' | '$')
        })
        .map(|(_, c)| c.to_ascii_lowercase())
        .collect();
    let mut typed_chars = typed.chars();
    let Some(first) = typed_chars.next() else { return true };
    if starts.first() != Some(&first) {
        return false;
    }
    let mut rest = starts[1..].iter();
    typed_chars.all(|c| rest.any(|start| *start == c))
}

fn in_order(text: &str, typed: &str) -> bool {
    let mut chars = text.chars();
    typed.chars().all(|c| chars.any(|t| t == c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_that_stops_answers_with_an_error() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let server = Server::start(
            "Nothing",
            Path::new("/usr/bin/true"),
            &[],
            dir.path(),
            "/usr/bin",
            |_, _| {},
            |_| {},
            Value::Null,
            Value::Null,
        )
        .expect("started");
        let answer = block_on(async {
            for _ in 0..200 {
                if !server.is_alive() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            server.request("textDocument/hover", Value::Null).await
        });
        assert_eq!(answer, Err("Nothing stopped".to_owned()));
    }

    #[test]
    fn ranks_prefixes_then_word_starts_then_letters_in_order() {
        let candidates = [
            ("SVGAnimatedString", "15"),
            ("getElementById", "15"),
            ("format", "11"),
            ("Format", "15"),
            ("formatMoney", "11"),
            ("reformat", "15"),
        ];
        let names = |typed: &str| -> Vec<&str> {
            rank(typed, &candidates, 10).into_iter().map(|index| candidates[index].0).collect()
        };
        assert_eq!(names("form"), ["format", "formatMoney", "Format", "reformat"]);
        assert_eq!(names("gEBI"), ["getElementById"]);
        assert_eq!(names("fm"), ["formatMoney", "format", "Format", "reformat"]);
        assert_eq!(rank("", &candidates, 2).len(), 2);
        assert_eq!(word_before("  const x = form", 16), "form");
        assert_eq!(word_before("obj.", 4), "");
    }

    #[test]
    fn frames_and_reads_messages_back() {
        let first = json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": "é" } });
        let second = json!({ "jsonrpc": "2.0", "method": "window/logMessage", "params": {} });
        let mut wire = frame(&first);
        wire.extend(b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n");
        wire.extend(frame(&second));
        let mut reader = io::Cursor::new(wire);
        assert_eq!(read_message(&mut reader).expect("read"), Some(first));
        assert_eq!(read_message(&mut reader).expect("read"), Some(second));
        assert_eq!(read_message(&mut reader).expect("read"), None);
    }

    #[test]
    fn file_uris_round_trip() {
        let odd = Path::new("/Users/a b/čo/[id].tsx");
        assert_eq!(uri(odd), "file:///Users/a%20b/%C4%8Do/%5Bid%5D.tsx");
        assert_eq!(path(&uri(odd)).as_deref(), Some(odd));
        assert_eq!(path("https://example.com"), None);
    }

    #[test]
    fn finds_the_case_the_disk_uses() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let real = dir.path().join("Shop/Cart.ts");
        std::fs::create_dir_all(real.parent().expect("a folder")).expect("folders");
        std::fs::write(&real, "").expect("written");
        let lowered = PathBuf::from(dir.path().join("shop/cart.ts").to_string_lossy().into_owned());
        assert_eq!(true_case(&lowered), real);
        assert_eq!(true_case(&real), real);
    }

    #[test]
    fn knows_each_language_and_its_id() {
        assert_eq!(spec_for(Path::new("a/b.tsx")).map(|spec| spec.name), Some("TypeScript"));
        assert_eq!(spec_for(Path::new("main.RS")).map(|spec| spec.name), Some("Rust"));
        assert!(spec_for(Path::new("notes.md")).is_none());
        assert_eq!(language_id(Path::new("x.jsx")), "javascriptreact");
    }

    #[test]
    fn prefers_the_projects_own_server() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        let bin = root.join("web/node_modules/.bin");
        std::fs::create_dir_all(&bin).expect("folders");
        std::fs::write(bin.join("typescript-language-server"), "").expect("written");
        let typescript = root.join("web/node_modules/typescript/lib");
        std::fs::create_dir_all(&typescript).expect("folders");
        std::fs::write(typescript.join("tsserver.js"), "").expect("written");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("folders");
        std::fs::write(elsewhere.join("vtsls"), "").expect("written");
        let path = elsewhere.to_string_lossy().into_owned();
        let found = find_server(&SPECS[0], &root.join("web/src/a.ts"), root, &root.join("none"), &path);
        assert_eq!(found.map(|(program, _)| program), Some(bin.join("typescript-language-server")));
        let found = find_server(&SPECS[0], &root.join("api/a.ts"), root, &root.join("none"), &path);
        assert_eq!(found.map(|(program, _)| program), Some(elsewhere.join("vtsls")));
        // Akyba's own TypeScript 7, and never a project's older `tsc`.
        let akyba = root.join("akyba");
        std::fs::create_dir_all(&akyba).expect("folders");
        std::fs::write(akyba.join("tsc"), "").expect("written");
        std::fs::write(bin.join("tsc"), "").expect("written");
        std::fs::remove_file(bin.join("typescript-language-server")).expect("removed");
        let found = find_server(&SPECS[0], &root.join("web/a.ts"), root, &akyba, "/nowhere");
        assert_eq!(found, Some((akyba.join("tsc"), &["--lsp", "--stdio"][..])));
    }

    /// Talks to a real rust-analyzer: `AKYBA_RUST_ANALYZER=path cargo test -- --ignored`, or the
    /// first one on `PATH`.
    #[test]
    #[ignore = "needs rust-analyzer"]
    fn a_real_server_answers_hover() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
            .expect("written");
        std::fs::create_dir_all(root.join("src")).expect("folders");
        let text = "fn main() { let answer = 42; println!(\"{answer}\"); }\n";
        std::fs::write(root.join("src/main.rs"), text).expect("written");
        let path_env = std::env::var("PATH").unwrap_or_default();
        let (program, args) = match std::env::var_os("AKYBA_RUST_ANALYZER") {
            Some(program) => (PathBuf::from(program), &[][..]),
            None => find_server(&SPECS[1], &root.join("src/main.rs"), root, root, &path_env).expect("rust-analyzer"),
        };
        let server =
            Server::start("Rust", &program, args, root, &path_env, |_, _| {}, |_| {}, Value::Null, Value::Null)
                .expect("started");
        let document = Document::open(server.clone(), &root.join("src/main.rs"), text);
        let hover = block_on(async {
            for _ in 0..60 {
                let answer = server.request("textDocument/hover", document.at(0, 17)).await;
                if let Ok(answer) = answer
                    && !answer.is_null()
                {
                    return answer;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Value::Null
        });
        assert!(hover.to_string().contains("i32"), "{hover}");
    }

    /// Runs a future on this thread (tests only: the app has its own executor).
    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        use std::task::{Context, Poll, Waker};
        let mut future = std::pin::pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
