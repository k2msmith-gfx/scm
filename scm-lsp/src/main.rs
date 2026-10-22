//! `scm-lsp` -- a Language Server for scm.
//!
//! Zed is the named consumer (`zed-scm`, Z3) and the reason this exists at
//! all: a Zed extension cannot answer a completion request itself, only
//! name a program to launch. But nothing here is Zed-specific -- it is
//! stdio LSP, and the editor on the other end is not asked.
//!
//! The division of labour is the thing to understand before changing it:
//!
//!   * **names come from a live session** (`session.rs`), so completion and
//!     hover know what the user defined at the prompt, not merely what the
//!     file says;
//!   * **structure comes from a token scan** (`text.rs`), because a buffer
//!     being edited is unparseable most of the time;
//!   * **diagnostics come from scm's own reader**, because "is this text a
//!     program" is its question and nothing else should answer it.
//!
//! A separate binary from `scm`, deliberately: the `scm` binary has a 10 ms
//! boot budget with a test enforcing it, and an editor server that lives
//! for hours should not make a `--batch` sweep pay for a JSON parser.

mod index;
mod session;
mod text;
use text as scm_text;

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{Value as J, json};

use index::Index;
use session::Backend;
use text::Doc;

/// Where the session is, unless `SCM_ADDR` or an initialization option says
/// otherwise. The same default `scm-eval` uses, so one server serves the
/// editor, the prompt and this.
const DEFAULT_ADDR: &str = "127.0.0.1:7000";

struct Server {
    docs: HashMap<String, Doc>,
    /// Every definition in the workspace, so a jump can leave the file.
    index: Index,
    backend: Backend,
    /// What the backend is, reported in `initialize` and in a hover's
    /// footer: a user who gets no completion needs to know whether the
    /// server was found, not just that nothing happened.
    origin: String,
    shutdown: bool,
}

fn main() {
    let addr = std::env::args()
        .skip_while(|a| a != "--addr")
        .nth(1)
        .or_else(|| std::env::var("SCM_ADDR").ok())
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());
    let (backend, origin) = Backend::new(&addr);
    eprintln!("scm-lsp: {origin}");
    let mut server = Server {
        docs: HashMap::new(),
        index: Index::default(),
        backend,
        origin,
        shutdown: false,
    };

    let stdin = io::stdin();
    let mut input = stdin.lock();
    while let Some(msg) = read_message(&mut input) {
        let Some(out) = server.handle(&msg) else { continue };
        write_message(&out);
        if server.shutdown {
            break;
        }
    }
}

/// LSP framing: `Content-Length: N`, a blank line, then N **bytes** of
/// JSON. Bytes here and nowhere else in this crate -- a header counts the
/// encoded length, while every offset inside a document is a character.
fn read_message(input: &mut impl BufRead) -> Option<String> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            len = n.trim().parse().ok();
        }
    }
    let mut buf = vec![0u8; len?];
    input.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn write_message(body: &J) {
    let text = body.to_string();
    let out = io::stdout();
    let mut out = out.lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", text.len(), text);
    let _ = out.flush();
}

/// A notification the server sends unasked, for diagnostics.
fn notify(method: &str, params: J) {
    write_message(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
}

impl Server {
    /// `None` for a notification, which has no reply.
    fn handle(&mut self, raw: &str) -> Option<J> {
        let msg: J = serde_json::from_str(raw).ok()?;
        let method = msg.get("method")?.as_str()?.to_string();
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(J::Null);

        let result = match method.as_str() {
            "initialize" => {
                // An explicit option wins over the environment, so a
                // project can pin its own port.
                if let Some(a) = params
                    .pointer("/initializationOptions/address")
                    .and_then(J::as_str)
                {
                    let (b, o) = Backend::new(a);
                    self.backend = b;
                    self.origin = o;
                    eprintln!("scm-lsp: {}", self.origin);
                }
                // The workspace, scanned once. `workspaceFolders` is the
                // current spelling and `rootUri` the old one; clients still
                // send either, so both are read.
                let mut roots: Vec<String> = params
                    .pointer("/workspaceFolders")
                    .and_then(J::as_array)
                    .map(|fs| {
                        fs.iter()
                            .filter_map(|f| f.get("uri").and_then(J::as_str).map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                if roots.is_empty()
                    && let Some(u) = params.pointer("/rootUri").and_then(J::as_str)
                {
                    roots.push(u.to_string());
                }
                for uri in roots {
                    if let Some(path) = index::path_of(&uri) {
                        let n = self.index.scan_root(&path);
                        eprintln!("scm-lsp: indexed {n} file(s) under {}", path.display());
                    }
                }
                eprintln!(
                    "scm-lsp: {} definitions in {} file(s)",
                    self.index.names(),
                    self.index.files()
                );
                Some(self.capabilities())
            }
            "initialized" => None,
            "shutdown" => {
                self.shutdown = true;
                Some(J::Null)
            }
            "exit" => {
                self.shutdown = true;
                None
            }
            "textDocument/didOpen" => {
                let uri = uri_of(&params, "/textDocument/uri")?;
                let text = params
                    .pointer("/textDocument/text")
                    .and_then(J::as_str)
                    .unwrap_or_default();
                self.docs.insert(uri.clone(), Doc::new(text));
                // An open buffer is the truth about itself, so its entries
                // come from the text rather than from what is on disk.
                self.index.update(&uri, text);
                self.publish(&uri);
                None
            }
            "textDocument/didChange" => {
                let uri = uri_of(&params, "/textDocument/uri")?;
                // Full sync: the capabilities ask for whole documents, so
                // the last change carries the whole text.
                if let Some(text) = params
                    .pointer("/contentChanges")
                    .and_then(J::as_array)
                    .and_then(|c| c.last())
                    .and_then(|c| c.get("text"))
                    .and_then(J::as_str)
                {
                    self.docs.insert(uri.clone(), Doc::new(text));
                    self.index.update(&uri, text);
                    self.publish(&uri);
                }
                None
            }
            "textDocument/didSave" => {
                // The file on disk may have been formatted or reverted by
                // something other than the editor.
                let uri = uri_of(&params, "/textDocument/uri")?;
                if let Some(path) = index::path_of(&uri) {
                    self.index.scan_file(&path);
                }
                None
            }
            "textDocument/didClose" => {
                let uri = uri_of(&params, "/textDocument/uri")?;
                self.docs.remove(&uri);
                // Not forgotten outright: the file still exists and its
                // definitions are still jump targets. Re-read it from disk,
                // so unsaved edits stop being visible -- and if it cannot
                // be read it was deleted or renamed while open, so its
                // entries have to go rather than become jumps to nowhere.
                match index::path_of(&uri) {
                    Some(path) if self.index.scan_file(&path) => {}
                    _ => self.index.forget(&uri),
                }
                // Clear its diagnostics: a closed file's errors must not
                // stay in the problems list for ever.
                notify(
                    "textDocument/publishDiagnostics",
                    json!({"uri": uri, "diagnostics": []}),
                );
                None
            }
            "textDocument/completion" => Some(self.completion(&params)),
            "textDocument/hover" => Some(self.hover(&params)),
            "textDocument/signatureHelp" => Some(self.signature_help(&params)),
            "textDocument/documentSymbol" => Some(self.document_symbol(&params)),
            "textDocument/definition" => Some(self.definition(&params)),
            "workspace/symbol" => Some(self.workspace_symbol(&params)),
            // An unknown request still needs an answer, or a client waits
            // for ever; an unknown notification is correctly ignored.
            _ => Some(J::Null),
        };

        match (id, result) {
            (Some(id), Some(result)) => {
                Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
            }
            _ => None,
        }
    }

    fn capabilities(&self) -> J {
        json!({
            "capabilities": {
                // 1 = full text on every change. A Lisp file is small and
                // the token scan is one pass; incremental sync would be
                // state to keep correct for no measurable gain.
                "textDocumentSync": 1,
                "completionProvider": { "triggerCharacters": ["(", "-", ":", "#", "?", "!"] },
                "hoverProvider": true,
                "signatureHelpProvider": { "triggerCharacters": [" ", "("] },
                "documentSymbolProvider": true,
                "definitionProvider": true,
                "workspaceSymbolProvider": true
            },
            "serverInfo": {
                "name": "scm-lsp",
                "version": env!("CARGO_PKG_VERSION"),
                // Not part of LSP; harmless, and it puts the one fact a
                // user needs when completion is empty into the handshake.
                "scmSession": self.origin
            }
        })
    }

    /// Diagnostics, from scm's reader rather than from the token scan.
    ///
    /// The two statuses are kept apart because S1 made them different
    /// answers: **invalid** is wrong and is an error, **incomplete** is
    /// "keep typing" and is not. Reporting an unclosed form as an error
    /// would put a red mark under the cursor of every form being written,
    /// so it is reported as information, at the end, where the missing
    /// bracket is.
    fn publish(&mut self, uri: &str) {
        let Some(doc) = self.docs.get(uri) else { return };
        let text = doc.text();
        let mut diagnostics = Vec::new();
        let mut engine = scm::Engine::new();
        if let Err(e) = scm::Reader::read_all(&text, &mut engine) {
            let (start, end, message, severity) = match e {
                scm::ReadError::Invalid { span, msg } => (span.start, span.end, msg, 1),
                scm::ReadError::Incomplete => (
                    doc.len(),
                    doc.len(),
                    "incomplete datum: something is still open".to_string(),
                    3,
                ),
            };
            let (sl, sc) = doc.position(start);
            let (el, ec) = doc.position(end.max(start));
            diagnostics.push(json!({
                "range": {"start": {"line": sl, "character": sc},
                          "end": {"line": el, "character": ec}},
                "severity": severity,
                "source": "scm",
                "message": message
            }));
        }
        notify(
            "textDocument/publishDiagnostics",
            json!({"uri": uri, "diagnostics": diagnostics}),
        );
    }

    fn at(&self, params: &J) -> Option<(&Doc, usize)> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?;
        let doc = self.docs.get(uri)?;
        let line = params.pointer("/position/line")?.as_u64()? as u32;
        let ch = params.pointer("/position/character")?.as_u64()? as u32;
        Some((doc, doc.offset(line, ch)))
    }

    fn completion(&mut self, params: &J) -> J {
        let Some((doc, offset)) = self.at(params) else { return J::Null };
        if doc.in_text(offset) {
            // Inside a string or a comment there is nothing to complete,
            // and offering the global environment there is noise.
            return json!({"isIncomplete": false, "items": []});
        }
        let prefix = doc.prefix_at(offset);
        // The file's own definitions first: a procedure written a minute
        // ago and not yet evaluated is not in the session, and it is
        // exactly what the user is about to call.
        let local: Vec<String> = doc
            .definitions()
            .into_iter()
            .map(|d| d.name)
            .filter(|n| n.starts_with(&prefix))
            .collect();
        let mut items: Vec<J> = local
            .iter()
            .map(|n| {
                json!({"label": n, "kind": 3, "detail": "in this file",
                       "sortText": format!("0{n}")})
            })
            .collect();
        for name in self.backend.complete(&prefix) {
            if local.contains(&name) {
                continue;
            }
            // 3 = Function. Everything in a Scheme global environment is
            // callable or close enough, and asking `describe` per candidate
            // would be one round trip per name on every keystroke.
            items.push(json!({"label": name, "kind": 3, "sortText": format!("1{name}")}));
        }
        json!({"isIncomplete": false, "items": items})
    }

    fn hover(&mut self, params: &J) -> J {
        let Some((doc, offset)) = self.at(params) else { return J::Null };
        let Some((name, start, end)) = doc.symbol_at(offset) else { return J::Null };
        let (sl, sc) = doc.position(start);
        let (el, ec) = doc.position(end);
        let Some(d) = self.backend.describe(&name) else {
            return J::Null;
        };
        // Markdown, because a signature wants a code block and a docstring
        // wants prose, and a client that renders neither still shows both.
        let mut md = String::new();
        md.push_str("```scheme\n");
        md.push_str(d.signature.as_deref().unwrap_or(&name));
        md.push_str("\n```\n\n");
        if let Some(doc_text) = &d.doc {
            md.push_str(doc_text);
            md.push_str("\n\n");
        }
        md.push_str(&format!("*{}* — `{}`", d.kind, d.value));
        json!({
            "contents": {"kind": "markdown", "value": md},
            "range": {"start": {"line": sl, "character": sc},
                      "end": {"line": el, "character": ec}}
        })
    }

    fn signature_help(&mut self, params: &J) -> J {
        let Some((doc, offset)) = self.at(params) else { return J::Null };
        if doc.in_text(offset) {
            return J::Null;
        }
        let Some((head, _, arg)) = doc.enclosing_call(offset) else { return J::Null };
        let Some(d) = self.backend.describe(&head) else { return J::Null };
        let Some(signature) = d.signature else { return J::Null };
        // The parameters, from the signature's own text: it is what
        // `(signature …)` prints, so splitting it is reading scm's answer
        // rather than inventing a second one.
        let params_in: Vec<J> = signature
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split_whitespace()
            .skip(1)
            .map(|p| json!({"label": p}))
            .collect();
        json!({
            "signatures": [{
                "label": signature,
                "documentation": d.doc,
                "parameters": params_in
            }],
            "activeSignature": 0,
            "activeParameter": arg
        })
    }

    fn document_symbol(&mut self, params: &J) -> J {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(J::as_str) else {
            return J::Null;
        };
        let Some(doc) = self.docs.get(uri) else { return J::Null };
        let one = |d: &scm_text::Definition| -> J {
            let (sl, sc) = doc.position(d.start);
            let (el, ec) = doc.position(d.end);
            let (nl, nc) = doc.position(d.name_at);
            let (nel, nec) = doc.position(d.name_at + d.name.chars().count());
            json!({
                "name": d.name,
                "detail": d.detail.clone().unwrap_or_else(|| d.form.clone()),
                "kind": kind_of(&d.form),
                "range": {"start": {"line": sl, "character": sc},
                          "end": {"line": el, "character": ec}},
                "selectionRange": {"start": {"line": nl, "character": nc},
                                   "end": {"line": nel, "character": nec}}
            })
        };
        // A record type's constructor, predicate and accessors **nest under
        // it**. They have to be findable -- go-to-definition needs them --
        // but six flat entries for one declaration is not the file's shape,
        // which is what an outline is for. They arrive in order, so the
        // type opens a parent and the next non-type entry closes it.
        let mut symbols: Vec<J> = Vec::new();
        let mut children: Vec<J> = Vec::new();
        let mut open_record: Option<J> = None;
        for d in doc.definitions() {
            let is_member =
                d.form == "define-record-type" && d.detail.as_deref() != Some("type");
            if is_member && open_record.is_some() {
                children.push(one(&d));
                continue;
            }
            if let Some(mut parent) = open_record.take() {
                parent["children"] = J::Array(std::mem::take(&mut children));
                symbols.push(parent);
            }
            if d.form == "define-record-type" && d.detail.as_deref() == Some("type") {
                open_record = Some(one(&d));
            } else {
                symbols.push(one(&d));
            }
        }
        if let Some(mut parent) = open_record.take() {
            parent["children"] = J::Array(children);
            symbols.push(parent);
        }
        J::Array(symbols)
    }

    /// This file first, then the rest of the workspace.
    ///
    /// The local definition wins when there is one, because a name defined
    /// where you are reading is the one you meant; otherwise every site in
    /// the workspace is offered, since a name defined in two libraries is
    /// normal and the editor can present the choice.
    ///
    /// A prelude name still has no answer: it is a value, not a line. Hover
    /// is what tells you about those, and a wrong jump would be worse than
    /// none. The same is true of anything defined at the REPL -- see
    /// `index.rs` for why that needs the session rather than the text.
    fn definition(&mut self, params: &J) -> J {
        let Some((doc, offset)) = self.at(params) else { return J::Null };
        let Some((name, _, _)) = doc.symbol_at(offset) else { return J::Null };
        let Some(uri) = params
            .pointer("/textDocument/uri")
            .and_then(J::as_str)
            .map(str::to_string)
        else {
            return J::Null;
        };
        for d in doc.definitions() {
            if d.name == name {
                let (l, c) = doc.position(d.name_at);
                let (el, ec) = doc.position(d.name_at + name.chars().count());
                return json!({
                    "uri": uri,
                    "range": {"start": {"line": l, "character": c},
                              "end": {"line": el, "character": ec}}
                });
            }
        }
        let sites: Vec<J> = self
            .index
            .sites(&name)
            .into_iter()
            // The open buffer was searched above and found nothing, so an
            // entry for it here would be stale.
            .filter(|s| s.uri != uri)
            .map(|s| {
                json!({
                    "uri": s.uri,
                    "range": {"start": {"line": s.line, "character": s.character},
                              "end": {"line": s.line, "character": s.end_character}}
                })
            })
            .collect();
        if sites.is_empty() { J::Null } else { J::Array(sites) }
    }

    /// `workspace/symbol`: the index, filtered. Nearly free once the index
    /// exists, and it is how a reader finds a name they half-remember.
    fn workspace_symbol(&mut self, params: &J) -> J {
        let query = params.pointer("/query").and_then(J::as_str).unwrap_or("");
        let found: Vec<J> = self
            .index
            .matching(query, 512)
            .into_iter()
            .map(|(name, s)| {
                json!({
                    "name": name,
                    "kind": kind_of(&s.form),
                    "containerName": s.detail,
                    "location": {
                        "uri": s.uri,
                        "range": {"start": {"line": s.line, "character": s.character},
                                  "end": {"line": s.line, "character": s.end_character}}
                    }
                })
            })
            .collect();
        J::Array(found)
    }
}

/// LSP `SymbolKind`. A record type is a Struct, a macro an Operator, a
/// library a Module: the outline's icons then say what the form was.
fn kind_of(form: &str) -> u64 {
    match form {
        "define-record-type" | "define-structure" => 23,
        "define-syntax" => 25,
        "define-library" => 2,
        _ => 12,
    }
}

fn uri_of(params: &J, path: &str) -> Option<String> {
    Some(params.pointer(path)?.as_str()?.to_string())
}
