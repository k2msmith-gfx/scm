//! **A language server, when one is on the PATH** (docs/editor-crate-plan.md,
//! E7; Kevin, 2026-11-14: "lets do next phase"). The client half of the
//! Language Server Protocol over a child's stdin/stdout, written for
//! rust-analyzer and general enough for the next server.
//!
//! **Opt-in, with no configuration.** A language registers the program that
//! serves it and the file that marks a project's root (`Cargo.toml`). The
//! first buffer of that language with a path starts the server for its
//! project, and every buffer is synced to it as its text changes. When the
//! program is not on the PATH nothing starts, nothing is said until asked,
//! and the editor behaves as it did before: the buffer's own words
//! complete, nothing is underlined.
//!
//! **The UI never waits on startup.** `initialize` is sent at once and what
//! follows is queued until its answer lands on the reader thread, which
//! then sends `initialized` and the queue in order. The quick questions
//! (completion, definition, hover) wait for their own answer only, with a
//! timeout, and are cancelled past it. Diagnostics are pushed by the server
//! when it likes: the reader stores them and wakes the host
//! ([`crate::wake`]), and the widget draws them with its own
//! ([`crate::editorpane::Watcher`]). The argument hints are asked without
//! waiting, and the shell takes the answer when the host is woken.
//!
//! **Positions are bytes where the server agrees** (`positionEncodings`,
//! LSP 3.17: rust-analyzer does), as the widget counts them; a server that
//! speaks only UTF-16 is converted line by line.
//!
//! This module never names the VM (the layering rule): its prims are
//! `scm_binding/lsp.rs`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::language::{Diagnostic, Severity};

/// How long a quick question waits for its answer.
const WAIT: Duration = Duration::from_millis(1500);

// ── The wire ──────────────────────────────────────────────────────────────

/// Write one message, framed with its `Content-Length`.
pub fn write_message(w: &mut dyn Write, v: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(v)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

/// Read one message; None at the end of the stream.
pub fn read_message(r: &mut dyn BufRead) -> io::Result<Option<Value>> {
    let mut len: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            len = value.trim().parse().ok();
        }
    }
    let mut body = vec![0; len.unwrap_or(0)];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(io::Error::other)
}

// ── Positions and paths ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
}

/// A position's `character` on `line` as a byte offset, clamped to the line
/// and to a character boundary.
pub fn to_byte(line: &str, character: u64, enc: Encoding) -> usize {
    let character = character as usize;
    match enc {
        Encoding::Utf8 => {
            let mut b = character.min(line.len());
            while !line.is_char_boundary(b) {
                b -= 1;
            }
            b
        }
        Encoding::Utf16 => {
            let mut units = 0;
            for (b, c) in line.char_indices() {
                if units >= character {
                    return b;
                }
                units += c.len_utf16();
            }
            line.len()
        }
    }
}

/// A byte offset on `line` as a position's `character`.
pub fn from_byte(line: &str, byte: usize, enc: Encoding) -> u64 {
    let byte = byte.min(line.len());
    match enc {
        Encoding::Utf8 => byte as u64,
        Encoding::Utf16 => line[..byte].chars().map(|c| c.len_utf16() as u64).sum(),
    }
}

/// `file://` and the path, its bytes percent-encoded outside the unreserved
/// set (and `/`).
pub fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &b in path.to_string_lossy().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path a `file://` URI names; None for any other scheme.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(h) = rest.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

/// The project a file belongs to: the nearest directory above it holding a
/// marker, then further up while a directory's marker declares a workspace
/// (`[workspace]` in a `Cargo.toml`) — so every crate of ray shares one
/// server. The file's own directory when nothing marks one.
pub fn project_root(file: &Path, markers: &[String]) -> PathBuf {
    let dir = file.parent().unwrap_or(file);
    let marked = |d: &Path| markers.iter().find(|m| d.join(m).is_file()).map(|m| d.join(m));
    let Some(nearest) = dir.ancestors().find(|d| marked(d).is_some()) else {
        return dir.to_path_buf();
    };
    let mut root = nearest.to_path_buf();
    for d in nearest.ancestors().skip(1) {
        if let Some(m) = marked(d)
            && std::fs::read_to_string(&m).is_ok_and(|s| s.contains("[workspace]"))
        {
            root = d.to_path_buf();
        }
    }
    root
}

// ── One server ────────────────────────────────────────────────────────────

/// A diagnostic as the server sent it, in its own units.
#[derive(Clone, Debug)]
struct RawDiagnostic {
    start: (u64, u64),
    end: (u64, u64),
    severity: u64,
    message: String,
}

/// What the reader thread and the UI thread share.
#[derive(Default)]
struct State {
    /// `initialize` has been answered and `initialized` sent.
    ready: bool,
    encoding: Encoding,
    /// What was sent before `ready`, to go in order once it is.
    queued: Vec<Value>,
    /// Answers waited for, by request id.
    answers: HashMap<u64, Value>,
    /// Requests whose waiter gave up: their answers are dropped.
    abandoned: HashSet<u64>,
    /// The argument-hint request in flight, and the hint that came back.
    hint_id: Option<u64>,
    hint: Option<String>,
    diagnostics: HashMap<PathBuf, Vec<RawDiagnostic>>,
    /// Work the server reports (`$/progress`), by token.
    progress: HashMap<String, String>,
    /// rust-analyzer's own "done loading" (`experimental/serverStatus`).
    quiescent: Option<bool>,
    /// Why the server is gone, once it is.
    closed: Option<String>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Messages to the server go through the WRITER THREAD, so the UI never
/// blocks on a pipe the server is slow to drain (a whole-file change of a
/// large file fills one).
type Writer = mpsc::Sender<Value>;

pub struct Server {
    pub program: String,
    pub root: PathBuf,
    writer: Writer,
    shared: Arc<Shared>,
    next_id: Cell<u64>,
    child: Option<Child>,
}

impl Server {
    /// Start PROGRAM in ROOT and introduce ourselves.
    pub fn spawn(program: &str, args: &[String], root: &Path) -> io::Result<Server> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("no pipes"));
        };
        let mut server = Server::over(program, root, Box::new(stdout), Box::new(stdin));
        server.child = Some(child);
        Ok(server)
    }

    /// A server on any pair of streams — a child's pipes, or a test's.
    pub fn over(program: &str, root: &Path, from: Box<dyn Read + Send>, to: Box<dyn Write + Send>) -> Server {
        let (writer, outgoing) = mpsc::channel::<Value>();
        std::thread::spawn(move || {
            let mut to = to;
            for v in outgoing {
                if write_message(&mut *to, &v).is_err() {
                    return;
                }
            }
        });
        let shared = Arc::new(Shared::default());
        {
            let (writer, shared) = (writer.clone(), shared.clone());
            std::thread::spawn(move || read_loop(from, &writer, &shared));
        }
        let server = Server { program: program.to_string(), root: root.to_path_buf(), writer, shared, next_id: Cell::new(1), child: None };
        let uri = path_to_uri(root);
        let name = root.file_name().map_or_else(|| "root".to_string(), |n| n.to_string_lossy().into_owned());
        let init = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": uri,
                "workspaceFolders": [{"uri": uri, "name": name}],
                "clientInfo": {"name": "edit"},
                "capabilities": {
                    "general": {"positionEncodings": ["utf-8", "utf-16"]},
                    "window": {"workDoneProgress": true},
                    "textDocument": {
                        "synchronization": {"didSave": true},
                        "publishDiagnostics": {},
                        "completion": {"completionItem": {"snippetSupport": false}},
                        "hover": {"contentFormat": ["markdown", "plaintext"]},
                        "signatureHelp": {"signatureInformation": {}},
                        "definition": {}
                    },
                    "experimental": {"serverStatusNotification": true}
                }
            }
        });
        server.write(&init);
        server
    }

    fn write(&self, v: &Value) {
        // A server that has gone is answered by the reader's `closed`.
        let _ = self.writer.send(v.clone());
    }

    /// Send now, or queue until `initialize` is answered. Under the state's
    /// lock, so nothing overtakes `initialized` and the queue (the reader
    /// sends those under it too).
    fn send(&self, v: Value) {
        let mut st = self.shared.lock();
        if !st.ready {
            st.queued.push(v);
            return;
        }
        let _ = self.writer.send(v);
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub fn request(&self, method: &str, params: Value) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// Request and wait at most `timeout` for the result; None for an error,
    /// a gone server, or no answer in time (the request is then cancelled).
    pub fn ask(&self, method: &str, params: Value, timeout: Duration) -> Option<Value> {
        let id = self.request(method, params);
        let until = Instant::now() + timeout;
        let mut st = self.shared.lock();
        loop {
            if let Some(answer) = st.answers.remove(&id) {
                return answer.get("result").filter(|r| !r.is_null()).cloned();
            }
            if st.closed.is_some() {
                return None;
            }
            let now = Instant::now();
            if now >= until {
                st.abandoned.insert(id);
                drop(st);
                self.notify("$/cancelRequest", json!({"id": id}));
                return None;
            }
            st = self.shared.changed.wait_timeout(st, until - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    pub fn encoding(&self) -> Encoding {
        self.shared.lock().encoding
    }

    /// Ask for the argument hint at a position without waiting; the answer
    /// is [`Server::take_hint`]'s.
    fn ask_hint(&self, params: Value) {
        let id = self.request("textDocument/signatureHelp", params);
        self.shared.lock().hint_id = Some(id);
    }

    fn take_hint(&self) -> Option<String> {
        self.shared.lock().hint.take()
    }

    /// One line on how the server is: starting, busy with what, ready, gone.
    pub fn status(&self) -> String {
        let st = self.shared.lock();
        if let Some(why) = &st.closed {
            return format!("{} has stopped: {why}", self.program);
        }
        if !st.ready {
            return format!("{} is starting", self.program);
        }
        if !st.progress.is_empty() {
            let mut work: Vec<&String> = st.progress.values().collect();
            work.sort();
            return format!("{}: {}", self.program, work.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("; "));
        }
        if st.quiescent == Some(false) {
            return format!("{} is loading the project", self.program);
        }
        format!("{} is ready ({})", self.program, self.root.display())
    }

    /// How the server is in a word or two, for a modeline: its name alone
    /// once it is ready.
    pub fn mode(&self) -> String {
        let st = self.shared.lock();
        let program = &self.program;
        if st.closed.is_some() {
            return format!("{program} stopped");
        }
        if !st.ready {
            return format!("{program} starting");
        }
        let mut work: Vec<&String> = st.progress.values().collect();
        work.sort();
        if let Some(w) = work.first() {
            // "Indexing — 12/40 (core) (30%)" is "indexing (30%)" here.
            let title = w.split(" — ").next().unwrap_or("").split(" (").next().unwrap_or("").to_lowercase();
            let pct = w.rsplit_once(" (").map(|(_, p)| p).filter(|p| p.ends_with("%)"));
            return match pct {
                Some(p) => format!("{program} {title} ({p}"),
                None => format!("{program} {title}"),
            };
        }
        if st.quiescent == Some(false) {
            return format!("{program} loading");
        }
        program.clone()
    }

    pub fn closed(&self) -> bool {
        self.shared.lock().closed.is_some()
    }

    fn diagnostics_of(&self, path: &Path) -> Vec<RawDiagnostic> {
        self.shared.lock().diagnostics.get(path).cloned().unwrap_or_default()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The reader thread: every message the server sends, until it stops.
fn read_loop(from: Box<dyn Read + Send>, writer: &Writer, shared: &Shared) {
    let mut from = BufReader::new(from);
    loop {
        match read_message(&mut from) {
            Ok(Some(msg)) => handle(&msg, writer, shared),
            Ok(None) | Err(_) => {
                shared.lock().closed.get_or_insert_with(|| "it exited".to_string());
                shared.changed.notify_all();
                crate::wake::wake();
                return;
            }
        }
    }
}

fn reply(writer: &Writer, v: &Value) {
    let _ = writer.send(v.clone());
}

fn handle(msg: &Value, writer: &Writer, shared: &Shared) {
    let id = msg.get("id");
    match (id, msg.get("method").and_then(Value::as_str)) {
        // A request from the server: answer the few a client must, refuse
        // the rest.
        (Some(id), Some(method)) => {
            let params = msg.get("params");
            let answer = match method {
                "workspace/configuration" => {
                    let n = params.and_then(|p| p.get("items")).and_then(Value::as_array).map_or(0, Vec::len);
                    json!({"jsonrpc": "2.0", "id": id, "result": vec![Value::Null; n]})
                }
                "window/workDoneProgress/create" | "client/registerCapability" | "client/unregisterCapability" => {
                    json!({"jsonrpc": "2.0", "id": id, "result": null})
                }
                _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not supported by edit"}}),
            };
            reply(writer, &answer);
        }
        // An answer.
        (Some(id), None) => {
            let Some(id) = id.as_u64() else { return };
            if id == 0 {
                // `initialize`: `ready` flips and `initialized` and the queue
                // go under the state's lock, so no send overtakes them.
                let mut st = shared.lock();
                let enc = msg.pointer("/result/capabilities/positionEncoding").and_then(Value::as_str);
                st.encoding = if enc == Some("utf-8") { Encoding::Utf8 } else { Encoding::Utf16 };
                st.ready = true;
                let _ = writer.send(json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
                for v in std::mem::take(&mut st.queued) {
                    let _ = writer.send(v);
                }
            } else {
                let mut st = shared.lock();
                if st.hint_id == Some(id) {
                    st.hint_id = None;
                    st.hint = msg.get("result").and_then(hint_text);
                    drop(st);
                    crate::wake::wake();
                } else if !st.abandoned.remove(&id) {
                    st.answers.insert(id, msg.clone());
                }
            }
            shared.changed.notify_all();
        }
        // A notification.
        (None, Some(method)) => {
            let Some(params) = msg.get("params") else { return };
            match method {
                "textDocument/publishDiagnostics" => {
                    let Some(path) = params.get("uri").and_then(Value::as_str).and_then(uri_to_path) else { return };
                    let diags = params.get("diagnostics").and_then(Value::as_array).map_or_else(Vec::new, |ds| {
                        ds.iter().filter_map(raw_diagnostic).collect()
                    });
                    shared.lock().diagnostics.insert(path, diags);
                }
                "$/progress" => {
                    let token = match params.get("token") {
                        Some(Value::String(s)) => s.clone(),
                        Some(v) => v.to_string(),
                        None => return,
                    };
                    let Some(value) = params.get("value") else { return };
                    let mut st = shared.lock();
                    if value.get("kind").and_then(Value::as_str) == Some("end") {
                        st.progress.remove(&token);
                    } else {
                        let prev = st.progress.get(&token).cloned().unwrap_or_default();
                        st.progress.insert(token, progress_text(value, &prev));
                    }
                }
                "experimental/serverStatus" => {
                    shared.lock().quiescent = params.get("quiescent").and_then(Value::as_bool);
                }
                _ => return,
            }
            crate::wake::wake();
        }
        _ => {}
    }
}

fn raw_diagnostic(d: &Value) -> Option<RawDiagnostic> {
    let pos = |p: &str| -> Option<(u64, u64)> {
        let at = d.pointer(p)?;
        Some((at.get("line")?.as_u64()?, at.get("character")?.as_u64()?))
    };
    let message = d.get("message")?.as_str()?;
    Some(RawDiagnostic {
        start: pos("/range/start")?,
        end: pos("/range/end")?,
        severity: d.get("severity").and_then(Value::as_u64).unwrap_or(1),
        // One line, for the echo area: a long message's detail joins on.
        message: message.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" — "),
    })
}

/// A `$/progress` value as "Indexing 12/40 (30%)", a report keeping its
/// begin's title.
fn progress_text(value: &Value, prev: &str) -> String {
    let field = |k: &str| value.get(k).and_then(Value::as_str).unwrap_or("");
    let title = match field("title") {
        "" => prev.split(" — ").next().unwrap_or("").split(" (").next().unwrap_or("").to_string(),
        t => t.to_string(),
    };
    let mut out = title;
    if !field("message").is_empty() {
        out = format!("{out} — {}", field("message"));
    }
    if let Some(p) = value.get("percentage").and_then(Value::as_u64) {
        out = format!("{out} ({p}%)");
    }
    out
}

/// The active signature's label from a `signatureHelp` result.
fn hint_text(result: &Value) -> Option<String> {
    let sigs = result.get("signatures")?.as_array()?;
    let active = result.get("activeSignature").and_then(Value::as_u64).unwrap_or(0) as usize;
    let sig = sigs.get(active).or(sigs.first())?;
    sig.get("label")?.as_str().map(str::to_string)
}

/// The words a completion result offers: each item's text as it would be
/// inserted, without a call's `(…)`, in the server's order, once each.
pub fn completion_words(result: &Value) -> Vec<String> {
    let items = match result {
        Value::Array(items) => items.as_slice(),
        v => v.get("items").and_then(Value::as_array).map_or(&[][..], Vec::as_slice),
    };
    let mut ranked: Vec<(&str, String)> = items
        .iter()
        .filter_map(|item| {
            let text = item
                .pointer("/textEdit/newText")
                .or_else(|| item.get("insertText"))
                .or_else(|| item.get("label"))
                .and_then(Value::as_str)?;
            let word = text.split(['(', '<', ' ', '!']).next().unwrap_or(text).trim_end_matches("::");
            let word = if text.contains('!') && !word.is_empty() { format!("{word}!") } else { word.to_string() };
            let sort = item.get("sortText").and_then(Value::as_str).unwrap_or("");
            (!word.is_empty()).then_some((sort, word))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(b.0));
    let mut seen = HashSet::new();
    ranked.into_iter().map(|(_, w)| w).filter(|w| seen.insert(w.clone())).collect()
}

/// A hover's text as (signature, everything): its code blocks unfenced, and
/// the signature the last code block before the first rule (rust-analyzer
/// puts the item's module first, then its signature, then `---` and the
/// docs).
pub fn hover_parts(result: &Value) -> Option<(String, String)> {
    let contents = result.get("contents")?;
    let text = match contents {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string).or_else(|| p.get("value")?.as_str().map(str::to_string)))
            .collect::<Vec<_>>()
            .join("\n\n"),
        v => v.get("value")?.as_str()?.to_string(),
    };
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut doc: Vec<&str> = Vec::new();
    let (mut in_code, mut before_rule) = (false, true);
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code && before_rule {
                blocks.push(Vec::new());
            }
            continue;
        }
        if !in_code && line.trim() == "---" {
            before_rule = false;
            doc.push("");
            continue;
        }
        if in_code && before_rule
            && let Some(b) = blocks.last_mut()
        {
            b.push(line);
        }
        doc.push(line);
    }
    let signature = blocks.last().map(|b| b.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" "))?;
    let mut doc = doc.join("\n").trim().to_string();
    while doc.contains("\n\n\n") {
        doc = doc.replace("\n\n\n", "\n\n");
    }
    (!signature.is_empty()).then_some((signature, doc))
}

// ── The servers this editor runs ──────────────────────────────────────────

/// What serves a language: the program, its arguments, and what marks a
/// project's root.
struct Spec {
    program: String,
    args: Vec<String>,
    markers: Vec<String>,
}

/// A buffer the servers know: which server, the version and text last sent.
struct Doc {
    server: usize,
    version: i64,
    text: String,
}

#[derive(Default)]
struct Registry {
    specs: HashMap<String, Spec>,
    servers: Vec<Server>,
    by_root: HashMap<(String, PathBuf), usize>,
    /// Programs that would not start, with why — asked again only after a
    /// restart.
    failed: HashMap<String, String>,
    docs: HashMap<PathBuf, Doc>,
    canonical: HashMap<String, PathBuf>,
    /// The last hover asked: (path, version, line, byte) and its answer.
    hover: Option<((PathBuf, i64, usize, usize), Option<(String, String)>)>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

impl Registry {
    fn canonical(&mut self, path: &str) -> PathBuf {
        if let Some(p) = self.canonical.get(path) {
            return p.clone();
        }
        let p = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        self.canonical.insert(path.to_string(), p.clone());
        p
    }

    /// The server for `file` in `lang`, started if need be; None when the
    /// language has no server or its program will not start.
    fn server_for(&mut self, lang: &str, file: &Path) -> Option<usize> {
        let spec = self.specs.get(lang)?;
        if spec.program.is_empty() || self.failed.contains_key(&spec.program) {
            return None;
        }
        let root = project_root(file, &spec.markers);
        let key = (spec.program.clone(), root.clone());
        if let Some(&i) = self.by_root.get(&key)
            && !self.servers[i].closed()
        {
            return Some(i);
        }
        match Server::spawn(&spec.program, &spec.args, &root) {
            Ok(server) => {
                self.servers.push(server);
                let i = self.servers.len() - 1;
                self.by_root.insert(key, i);
                // Every document of a server that stopped is opened afresh.
                self.docs.retain(|_, d| !self.servers[d.server].closed());
                Some(i)
            }
            Err(e) => {
                let why = if e.kind() == io::ErrorKind::NotFound { "not on the PATH".to_string() } else { e.to_string() };
                self.failed.insert(spec.program.clone(), why);
                None
            }
        }
    }

    /// Tell the server what `path` holds now: open it the first time, a
    /// whole-text change after. The server's index, or None.
    fn sync(&mut self, path: &str, lang: &str, text: &str) -> Option<(PathBuf, usize)> {
        if !self.specs.contains_key(lang) {
            return None;
        }
        let file = self.canonical(path);
        if let Some(doc) = self.docs.get_mut(&file)
            && !self.servers[doc.server].closed()
        {
            let server = &self.servers[doc.server];
            if doc.text != text {
                doc.version += 1;
                doc.text = text.to_string();
                server.notify(
                    "textDocument/didChange",
                    json!({"textDocument": {"uri": path_to_uri(&file), "version": doc.version}, "contentChanges": [{"text": text}]}),
                );
            }
            return Some((file, doc.server));
        }
        let i = self.server_for(lang, &file)?;
        self.servers[i].notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": path_to_uri(&file), "languageId": lang, "version": 1, "text": text}}),
        );
        self.docs.insert(file.clone(), Doc { server: i, version: 1, text: text.to_string() });
        Some((file, i))
    }

    /// The `textDocument` + `position` params for (`line`, `byte`) in `file`.
    fn position(&self, file: &Path, line: usize, byte: usize) -> Value {
        let doc = &self.docs[file];
        let enc = self.servers[doc.server].encoding();
        let text = doc.text.split('\n').nth(line).unwrap_or("");
        json!({
            "textDocument": {"uri": path_to_uri(file)},
            "position": {"line": line, "character": from_byte(text, byte, enc)}
        })
    }
}

/// Serve LANG with PROGRAM (and ARGS) in projects marked by MARKERS. An
/// empty PROGRAM turns the language's server off; registering again starts
/// over — stopping what runs and forgetting a program that would not start.
pub fn register(lang: &str, program: &str, args: &[String], markers: &[String]) {
    stop(Some(lang));
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.failed.remove(program);
        r.specs.insert(
            lang.to_string(),
            Spec { program: program.to_string(), args: args.to_vec(), markers: markers.to_vec() },
        );
    });
    crate::editorpane::set_watcher(&WATCHER);
}

/// Stop LANG's servers (every server, for None). The buffers are opened
/// afresh by the next server.
pub fn stop(lang: Option<&str>) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let program = lang.and_then(|l| r.specs.get(l)).map(|s| s.program.clone());
        let stopping: Vec<usize> = (0..r.servers.len())
            .filter(|&i| lang.is_none() || Some(&r.servers[i].program) == program.as_ref())
            .collect();
        for &i in &stopping {
            let server = &r.servers[i];
            if !server.closed() {
                server.notify("exit", Value::Null);
            }
        }
        // Indices must stay stable for the ones kept: replace, don't remove.
        let docs: Vec<PathBuf> = r.docs.iter().filter(|(_, d)| stopping.contains(&d.server)).map(|(p, _)| p.clone()).collect();
        for p in docs {
            r.docs.remove(&p);
        }
        r.by_root.retain(|_, i| !stopping.contains(i));
        for i in stopping {
            let dead = Server::over("", Path::new(""), Box::new(io::empty()), Box::new(io::sink()));
            dead.shared.lock().closed = Some("stopped".into());
            r.servers[i] = dead;
        }
        r.hover = None;
    });
}

/// The buffer at PATH holds TEXT now (the widget's watcher, and before each
/// question).
pub fn sync(path: &str, lang: &str, text: &str) {
    REGISTRY.with(|r| {
        r.borrow_mut().sync(path, lang, text);
    });
}

/// Is any server registered for LANG?
pub fn serves(lang: &str) -> bool {
    REGISTRY.with(|r| r.borrow().specs.get(lang).is_some_and(|s| !s.program.is_empty()))
}

/// PATH was written to disk: rust-analyzer runs `cargo check` then, and its
/// errors come back as diagnostics.
pub fn saved(path: &str) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let file = r.canonical(path);
        if let Some(doc) = r.docs.get(&file) {
            r.servers[doc.server].notify("textDocument/didSave", json!({"textDocument": {"uri": path_to_uri(&file)}}));
        }
    });
}

/// What is wrong with PATH as its server last said, in the widget's units
/// against the text last sent. Hints (severity 4) are left out; an error is
/// an error, everything else is drawn quietly.
pub fn diagnostics(path: &str) -> Vec<Diagnostic> {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let file = r.canonical(path);
        let Some(doc) = r.docs.get(&file) else { return Vec::new() };
        let server = &r.servers[doc.server];
        let enc = server.encoding();
        let lines: Vec<&str> = doc.text.split('\n').collect();
        let at = |(l, c): (u64, u64)| {
            let l = (l as usize).min(lines.len().saturating_sub(1));
            (l, to_byte(lines.get(l).copied().unwrap_or(""), c, enc))
        };
        server
            .diagnostics_of(&file)
            .into_iter()
            .filter(|d| d.severity < 4)
            .map(|d| Diagnostic {
                start: at(d.start),
                end: at(d.end),
                message: d.message,
                severity: if d.severity == 1 { Severity::Error } else { Severity::Warning },
            })
            .collect()
    })
}

/// Ask PATH's server a question about (`line`, `byte`), after syncing TEXT.
fn ask_at(path: &str, lang: &str, text: &str, line: usize, byte: usize, method: &str) -> Option<Value> {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let (file, i) = r.sync(path, lang, text)?;
        let params = r.position(&file, line, byte);
        r.servers[i].ask(method, params, WAIT)
    })
}

/// What completes at the point; None when there is no server to ask or it
/// did not answer in time.
pub fn complete(path: &str, lang: &str, text: &str, line: usize, byte: usize) -> Option<Vec<String>> {
    let words = completion_words(&ask_at(path, lang, text, line, byte, "textDocument/completion")?);
    (!words.is_empty()).then_some(words)
}

/// Where the name at the point is defined: (path, line, column in
/// CHARACTERS), as the shell's jump counts.
pub fn definition(path: &str, lang: &str, text: &str, line: usize, byte: usize) -> Option<Vec<(String, usize, usize)>> {
    let result = ask_at(path, lang, text, line, byte, "textDocument/definition")?;
    let enc = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let file = r.canonical(path);
        r.docs.get(&file).map(|d| r.servers[d.server].encoding())
    })?;
    let locations: Vec<&Value> = match &result {
        Value::Array(v) => v.iter().collect(),
        v => vec![v],
    };
    let mut out = Vec::new();
    for loc in locations {
        let uri = loc.get("targetUri").or_else(|| loc.get("uri")).and_then(Value::as_str);
        let range = loc.get("targetSelectionRange").or_else(|| loc.get("range"));
        let (Some(target), Some(range)) = (uri.and_then(uri_to_path), range) else { continue };
        let l = range.pointer("/start/line").and_then(Value::as_u64).unwrap_or(0) as usize;
        let c = range.pointer("/start/character").and_then(Value::as_u64).unwrap_or(0);
        let held = REGISTRY.with(|r| r.borrow().docs.get(&target).map(|d| d.text.clone()));
        let text = held.or_else(|| std::fs::read_to_string(&target).ok()).unwrap_or_default();
        let line_text = text.lines().nth(l).unwrap_or("");
        let col = line_text[..to_byte(line_text, c, enc)].chars().count();
        out.push((target.to_string_lossy().into_owned(), l, col));
    }
    Some(out)
}

/// The hover at the point as (signature, text); the last answer is kept, so
/// describe's two questions (signature, then doc) ask once.
pub fn hover(path: &str, lang: &str, text: &str, line: usize, byte: usize) -> Option<(String, String)> {
    let key = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let (file, _) = r.sync(path, lang, text)?;
        let version = r.docs.get(&file)?.version;
        Some((file, version, line, byte))
    })?;
    if let Some(answer) = REGISTRY.with(|r| r.borrow().hover.as_ref().filter(|(k, _)| *k == key).map(|(_, a)| a.clone())) {
        return answer;
    }
    let answer = ask_at(path, lang, text, line, byte, "textDocument/hover").as_ref().and_then(hover_parts);
    // Only an answer is kept: a server still loading answers nothing, and
    // asking again must ask it.
    if answer.is_some() {
        REGISTRY.with(|r| r.borrow_mut().hover = Some((key, answer.clone())));
    }
    answer
}

/// Ask for the argument hint at the point without waiting; true when asked.
pub fn ask_hint(path: &str, lang: &str, text: &str, line: usize, byte: usize) -> bool {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let Some((file, i)) = r.sync(path, lang, text) else { return false };
        let params = r.position(&file, line, byte);
        r.servers[i].ask_hint(params);
        true
    })
}

/// The argument hint that has come back since it was asked, once.
pub fn take_hint() -> Option<String> {
    REGISTRY.with(|r| r.borrow().servers.iter().find_map(Server::take_hint))
}

/// How LANG's server for PATH is, in a line.
pub fn status(path: &str, lang: &str) -> String {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let Some(spec) = r.specs.get(lang) else { return format!("no language server for {lang}") };
        if spec.program.is_empty() {
            return format!("the {lang} language server is off");
        }
        if let Some(why) = r.failed.get(&spec.program) {
            return format!("{}: {why}", spec.program);
        }
        let program = spec.program.clone();
        let file = r.canonical(path);
        match r.docs.get(&file) {
            Some(doc) => r.servers[doc.server].status(),
            None => format!("{program} has not seen this buffer yet"),
        }
    })
}

/// No buffer visits PATH now: its server is told, and forgets it.
pub fn closed(path: &str) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let file = r.canonical(path);
        let Some(doc) = r.docs.remove(&file) else { return };
        let server = &r.servers[doc.server];
        if !server.closed() {
            server.notify("textDocument/didClose", json!({"textDocument": {"uri": path_to_uri(&file)}}));
        }
        if r.hover.as_ref().is_some_and(|((p, ..), _)| *p == file) {
            r.hover = None;
        }
    })
}

/// LANG's server for PATH in a word or two, for the modeline; None when
/// none serves it — no server for the language, or its program is not on
/// the PATH (nothing to say, every time the buffer is drawn).
pub fn mode(path: &str, lang: &str) -> Option<String> {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let spec = r.specs.get(lang)?;
        if spec.program.is_empty() || r.failed.contains_key(&spec.program) {
            return None;
        }
        let file = r.canonical(path);
        let doc = r.docs.get(&file)?;
        Some(r.servers[doc.server].mode())
    })
}

/// The widget's view of the servers: each buffer's text as it changes, and
/// the diagnostics to draw.
struct LspWatcher;

static WATCHER: LspWatcher = LspWatcher;

impl crate::editorpane::Watcher for LspWatcher {
    fn text_changed(&self, path: &str, lang: &str, lines: &[&str]) {
        if serves(lang) {
            sync(path, lang, &lines.join("\n"));
        }
    }

    fn diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        diagnostics(path)
    }

    fn closed(&self, path: &str) {
        closed(path);
    }

    fn mode(&self, path: &str, lang: &str) -> Option<String> {
        mode(path, lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_frames_and_reads_back() {
        let mut wire = Vec::new();
        write_message(&mut wire, &json!({"id": 1, "result": "é"})).unwrap();
        assert!(wire.starts_with(b"Content-Length: "));
        let mut r = io::Cursor::new(wire);
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({"id": 1, "result": "é"})));
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn positions_convert_between_bytes_and_utf16() {
        let line = "let é = '🦀'; x";
        let x = line.find('x').unwrap();
        // é is one UTF-16 unit and two bytes; the crab two units, four bytes.
        assert_eq!(from_byte(line, x, Encoding::Utf16), 14);
        assert_eq!(to_byte(line, 14, Encoding::Utf16), x);
        assert_eq!(from_byte(line, x, Encoding::Utf8), x as u64);
        assert_eq!(to_byte(line, 99, Encoding::Utf8), line.len(), "clamped to the line");
        assert_eq!(to_byte(line, 5, Encoding::Utf8), 4, "back to a character's start");
    }

    #[test]
    fn a_path_round_trips_through_a_uri() {
        let p = Path::new("/tmp/a dir/main#1.rs");
        let uri = path_to_uri(p);
        assert_eq!(uri, "file:///tmp/a%20dir/main%231.rs");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(p));
        assert_eq!(uri_to_path("https://x"), None);
    }

    #[test]
    fn a_crate_in_a_workspace_roots_at_the_workspace() {
        let top = std::env::temp_dir().join(format!("edit-lsp-root-{}", std::process::id()));
        let member = top.join("member/src");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(top.join("Cargo.toml"), "[workspace]\nmembers = [\"member\"]\n").unwrap();
        std::fs::write(top.join("member/Cargo.toml"), "[package]\nname = \"m\"\n").unwrap();
        let markers = vec!["Cargo.toml".to_string()];
        assert_eq!(project_root(&member.join("lib.rs"), &markers), top);
        std::fs::write(top.join("Cargo.toml"), "[package]\nname = \"other\"\n").unwrap();
        assert_eq!(project_root(&member.join("lib.rs"), &markers), top.join("member"), "not a workspace: the nearest");
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn completion_words_drop_the_call_and_keep_the_order() {
        let result = json!({"items": [
            {"label": "foo(…)", "insertText": "foo", "sortText": "b"},
            {"label": "bar", "sortText": "a"},
            {"label": "println!(…)", "sortText": "c"},
            {"label": "foo", "sortText": "d"},
        ]});
        assert_eq!(completion_words(&result), vec!["bar", "foo", "println!"]);
    }

    #[test]
    fn a_hover_splits_into_its_signature_and_text() {
        let hover = json!({"contents": {"kind": "markdown", "value":
            "```rust\nmy_crate\n```\n\n```rust\npub fn foo(x: u32)\n-> bool\n```\n\n---\n\nDoes foo."}});
        let (sig, doc) = hover_parts(&hover).unwrap();
        assert_eq!(sig, "pub fn foo(x: u32) -> bool");
        assert!(doc.contains("pub fn foo") && doc.ends_with("Does foo.") && !doc.contains("```"), "{doc}");
    }

    /// A server on a pair of pipes, played by a thread: the client queues
    /// until `initialize` is answered, then sends `initialized` and what it
    /// queued in order; diagnostics pushed land; a question is answered;
    /// a server's own request is replied to.
    #[test]
    fn the_client_talks_to_a_server() {
        let (from_server, to_client) = io::pipe().unwrap();
        let (from_client, to_server) = io::pipe().unwrap();
        let server = Server::over("fake", Path::new("/proj"), Box::new(from_server), Box::new(to_server));
        server.notify("textDocument/didOpen", json!({"textDocument": {"uri": "file:///proj/a.rs", "text": "x"}}));
        let id = server.request("textDocument/completion", json!({}));
        assert_eq!(server.mode(), "fake starting");
        let fake = std::thread::spawn(move || {
            let mut from_client = BufReader::new(from_client);
            let mut to_client = to_client;
            let mut seen = Vec::new();
            let next = |r: &mut BufReader<io::PipeReader>| read_message(r).unwrap().unwrap();
            let init = next(&mut from_client);
            seen.push(init["method"].as_str().unwrap().to_string());
            assert_eq!(init["params"]["capabilities"]["general"]["positionEncodings"][0], "utf-8");
            write_message(&mut to_client, &json!({"id": 0, "result": {"capabilities": {"positionEncoding": "utf-8"}}})).unwrap();
            for _ in 0..3 {
                let m = next(&mut from_client);
                seen.push(m["method"].as_str().unwrap().to_string());
                if m["method"] == "textDocument/completion" {
                    write_message(&mut to_client, &json!({"id": m["id"], "result": [{"label": "frob"}]})).unwrap();
                }
            }
            write_message(&mut to_client, &json!({"method": "textDocument/publishDiagnostics", "params": {
                "uri": "file:///proj/a.rs",
                "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                                 "severity": 1, "message": "mismatched types\nexpected u32"}]}})).unwrap();
            for value in [json!({"kind": "begin", "title": "Indexing", "percentage": 0}),
                          json!({"kind": "report", "message": "12/40 (core)", "percentage": 30})] {
                write_message(&mut to_client, &json!({"method": "$/progress", "params": {"token": "idx", "value": value}})).unwrap();
            }
            write_message(&mut to_client, &json!({"id": 7, "method": "workspace/configuration", "params": {"items": [{}, {}]}})).unwrap();
            let answer = next(&mut from_client);
            // The pipe stays open: a closed one is a server that stopped.
            (seen, answer, to_client)
        });
        let result = server.ask_existing(id);
        assert_eq!(result.as_ref().map(completion_words), Some(vec!["frob".to_string()]));
        let (seen, answer, _open) = fake.join().unwrap();
        assert_eq!(seen, vec!["initialize", "initialized", "textDocument/didOpen", "textDocument/completion"]);
        assert_eq!(answer, json!({"jsonrpc": "2.0", "id": 7, "result": [null, null]}));
        assert_eq!(server.encoding(), Encoding::Utf8);
        let diags = server.diagnostics_of(Path::new("/proj/a.rs"));
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "mismatched types — expected u32");
        assert_eq!(server.status(), "fake: Indexing — 12/40 (core) (30%)");
        assert_eq!(server.mode(), "fake indexing (30%)", "the modeline's word");
    }

    impl Server {
        /// Wait for the answer to a request already sent.
        fn ask_existing(&self, id: u64) -> Option<Value> {
            let until = Instant::now() + Duration::from_secs(5);
            let mut st = self.shared.lock();
            loop {
                if let Some(a) = st.answers.remove(&id) {
                    return a.get("result").cloned();
                }
                if Instant::now() >= until {
                    return None;
                }
                st = self.shared.changed.wait_timeout(st, Duration::from_millis(50)).unwrap().0;
            }
        }
    }

    #[test]
    fn a_program_not_on_the_path_serves_nothing_and_says_so() {
        let dir = std::env::temp_dir().join(format!("edit-lsp-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn main() {}").unwrap();
        let path = file.to_string_lossy().into_owned();
        register("rust", "edit-no-such-language-server", &[], &["Cargo.toml".into()]);
        assert_eq!(complete(&path, "rust", "fn main() {}", 0, 3), None);
        assert!(diagnostics(&path).is_empty());
        assert_eq!(status(&path, "rust"), "edit-no-such-language-server: not on the PATH");
        assert_eq!(mode(&path, "rust"), None, "the modeline says nothing of it");
        register("rust", "", &[], &[]);
        assert_eq!(status(&path, "rust"), "the rust language server is off");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real thing: rust-analyzer on a small crate.
    /// `cargo test -p edit --lib real_rust_analyzer -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_rust_analyzer() {
        let dir = std::env::temp_dir().join(format!("edit-lsp-ra-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        let src = "/// Frobs it.\nfn frobnicate(x: u32) -> u32 { x }\n\nfn main() {\n    let y: u32 = \"no\";\n    frobnicate(1);\n    frob\n}\n";
        let main = dir.join("src/main.rs");
        std::fs::write(&main, src).unwrap();
        let path = main.to_string_lossy().into_owned();
        register("rust", "rust-analyzer", &[], &["Cargo.toml".into()]);
        sync(&path, "rust", src);
        saved(&path);
        let t0 = Instant::now();
        let mut words = None;
        while t0.elapsed() < Duration::from_secs(120) && words.is_none() {
            words = complete(&path, "rust", src, 6, 8).filter(|w| w.iter().any(|w| w == "frobnicate"));
            if words.is_none() {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        eprintln!("completion after {:?}: {:?}", t0.elapsed(), words.as_ref().map(|w| &w[..w.len().min(5)]));
        assert!(words.is_some(), "{}", status(&path, "rust"));
        let def = definition(&path, "rust", src, 5, 6).unwrap();
        assert_eq!(def, vec![(std::fs::canonicalize(&main).unwrap().to_string_lossy().into_owned(), 1, 3)]);
        // Busy loading (two of these tests at once), a first hover can come
        // back empty; it is not cached, so ask until it answers.
        let th = Instant::now();
        let mut h = None;
        while th.elapsed() < Duration::from_secs(60) && h.is_none() {
            h = hover(&path, "rust", src, 1, 5);
            if h.is_none() {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let (sig, doc) = h.unwrap();
        assert_eq!(sig, "fn frobnicate(x: u32) -> u32");
        assert!(doc.contains("Frobs it."), "{doc}");
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_secs(60) && !diagnostics(&path).iter().any(|d| d.is_error()) {
            std::thread::sleep(Duration::from_millis(200));
        }
        let diags = diagnostics(&path);
        eprintln!("diagnostics: {diags:?}\n{}", status(&path, "rust"));
        assert!(diags.iter().any(|d| d.is_error() && d.start.0 == 4), "{diags:?}");
        stop(None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
