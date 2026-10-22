//! The server over its real transport: frames on stdin, frames on stdout.
//!
//! The unit tests in `text.rs` cover what can be decided about a buffer.
//! What they cannot reach is everything an editor actually touches -- the
//! `Content-Length` framing, the request/notification split, whether a
//! reply carries the id it must, and whether the answers are shaped the way
//! a client reads them. A server can pass every unit test and still be a
//! blank screen in the editor.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    id: u64,
}

impl Lsp {
    /// A server with no scm on the port, so it falls back to a session of
    /// its own -- which is the configuration a user gets before starting
    /// one, and therefore the one that must work.
    fn start() -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_scm-lsp"))
            // Port 1 on loopback: nothing listens and nothing may bind it.
            .args(["--addr", "127.0.0.1:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn scm-lsp");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut lsp = Lsp { child, stdin, out, id: 0 };
        let init = lsp.request("initialize", serde_json::json!({"capabilities": {}}));
        assert!(
            init["result"]["capabilities"]["completionProvider"].is_object(),
            "no completion capability: {init}"
        );
        lsp.notify("initialized", serde_json::json!({}));
        lsp
    }

    fn write(&mut self, body: serde_json::Value) {
        let text = body.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", text.len(), text).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) {
        self.write(serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Send a request and return the reply with *its* id, skipping any
    /// notification that arrives first -- diagnostics are published
    /// unasked, so a reader that took the next message would read one.
    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.id += 1;
        let id = self.id;
        self.write(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }));
        loop {
            let msg = self.read();
            if msg.get("id").and_then(|i| i.as_u64()) == Some(id) {
                return msg;
            }
        }
    }

    /// The next message of any kind.
    fn read(&mut self) -> serde_json::Value {
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "server closed");
            let line = line.trim_end().to_string();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length:") {
                len = n.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; len];
        self.out.read_exact(&mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            serde_json::json!({"textDocument": {"uri": uri, "languageId": "scm",
                                                "version": 1, "text": text}}),
        );
    }

    /// The diagnostics published for a document, which arrive as a
    /// notification rather than as an answer.
    fn diagnostics(&mut self, uri: &str) -> Vec<serde_json::Value> {
        loop {
            let msg = self.read();
            if msg["method"] == "textDocument/publishDiagnostics"
                && msg["params"]["uri"] == uri
            {
                return msg["params"]["diagnostics"].as_array().cloned().unwrap_or_default();
            }
        }
    }

    fn at(&mut self, method: &str, uri: &str, line: u64, ch: u64) -> serde_json::Value {
        self.request(
            method,
            serde_json::json!({"textDocument": {"uri": uri},
                               "position": {"line": line, "character": ch}}),
        )["result"]
            .clone()
    }
}

impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn completion_offers_the_sessions_names_and_the_files_own_first() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/a.scm";
    lsp.open(uri, "(define (vector-of-mine n) n)\n(vector-\n");
    let _ = lsp.diagnostics(uri);
    // The cursor after `(vector-` on line 1.
    let got = lsp.at("textDocument/completion", uri, 1, 8);
    let items = got["items"].as_array().expect("items");
    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    // From the running session (here, the fallback's own prelude).
    assert!(labels.contains(&"vector-ref"), "no prelude names: {labels:?}");
    // And from the file, which the session has never seen -- the procedure
    // written a minute ago and not yet evaluated is the one being called.
    assert!(labels.contains(&"vector-of-mine"), "no local names: {labels:?}");
    let local = items.iter().find(|i| i["label"] == "vector-of-mine").unwrap();
    assert_eq!(local["detail"], "in this file");
    // Sorted ahead of the globals.
    assert!(
        local["sortText"].as_str().unwrap() < items.iter()
            .find(|i| i["label"] == "vector-ref").unwrap()["sortText"].as_str().unwrap(),
        "the file's own names must sort first"
    );
}

#[test]
fn completion_is_silent_inside_a_string() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/b.scm";
    lsp.open(uri, "(display \"vector-\")\n");
    let _ = lsp.diagnostics(uri);
    let got = lsp.at("textDocument/completion", uri, 0, 17);
    assert_eq!(got["items"].as_array().unwrap().len(), 0, "{got}");
}

#[test]
fn hover_is_the_sessions_own_introspection() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/c.scm";
    lsp.open(uri, "(vector-ref v 0)\n");
    let _ = lsp.diagnostics(uri);
    let got = lsp.at("textDocument/hover", uri, 0, 3);
    let md = got["contents"]["value"].as_str().unwrap_or_default();
    assert!(md.contains("vector-ref"), "{got}");
    // The kind comes from `describe`, not from a guess about the text.
    assert!(md.contains("primitive") || md.contains("procedure"), "{md}");
}

#[test]
fn hover_on_a_docstring_shows_it() {
    // The docstring convention is a value in scm, not a comment, so a
    // hover shows what `(doc …)` would -- but only for something the
    // session knows, which a file's unevaluated text is not. `car` is.
    let mut lsp = Lsp::start();
    let uri = "file:///t/d.scm";
    lsp.open(uri, "(car xs)\n");
    let _ = lsp.diagnostics(uri);
    let md = lsp.at("textDocument/hover", uri, 0, 2)["contents"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(md.contains("```scheme"), "not markdown: {md}");
    assert!(md.contains("car"), "{md}");
}

#[test]
fn signature_help_points_at_the_argument_the_cursor_is_in() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/e.scm";
    lsp.open(uri, "(vector-ref v \n");
    let _ = lsp.diagnostics(uri);
    let got = lsp.at("textDocument/signatureHelp", uri, 0, 14);
    let sig = got["signatures"][0]["label"].as_str().unwrap_or_default();
    assert!(sig.contains("vector-ref"), "{got}");
    // `v` is argument 0, so the cursor is in argument 1.
    assert_eq!(got["activeParameter"], 1, "{got}");
}

#[test]
fn the_outline_is_the_files_definitions() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/f.scm";
    lsp.open(
        uri,
        "(define x 1)\n(define (area r) (* 3 r r))\n(define-record-type point (make-point) point?)\n",
    );
    let _ = lsp.diagnostics(uri);
    let got = lsp.request(
        "textDocument/documentSymbol",
        serde_json::json!({"textDocument": {"uri": uri}}),
    )["result"]
        .clone();
    let syms = got.as_array().expect("symbols");
    let names: Vec<&str> = syms.iter().filter_map(|s| s["name"].as_str()).collect();
    assert_eq!(names, vec!["x", "area", "point"]);
    // A procedure shows its call shape; a record type is a Struct (23).
    assert_eq!(syms[1]["detail"], "(area r)");
    assert_eq!(syms[2]["kind"], 23);
    // The selection range is the name, on its own line.
    assert_eq!(syms[1]["selectionRange"]["start"]["line"], 1);

    // The record's constructor and predicate are findable, but **nested**
    // under the type rather than flattened beside it: they have to exist
    // for go-to-definition, and six flat entries for one declaration is
    // not the shape of the file.
    let kids: Vec<&str> = syms[2]["children"]
        .as_array()
        .expect("the record type has children")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert_eq!(kids, vec!["make-point", "point?"]);
}

#[test]
fn go_to_definition_stays_inside_the_file() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/g.scm";
    lsp.open(uri, "(define (area r) (* 3 r r))\n(area 2)\n(car xs)\n");
    let _ = lsp.diagnostics(uri);
    // The call on line 1 jumps to the name on line 0.
    let got = lsp.at("textDocument/definition", uri, 1, 2);
    assert_eq!(got["uri"], uri, "{got}");
    assert_eq!(got["range"]["start"]["line"], 0, "{got}");
    // A prelude name has no line to jump to, and a wrong answer would be
    // worse than none.
    assert!(lsp.at("textDocument/definition", uri, 2, 2).is_null());
}

#[test]
fn invalid_is_an_error_and_incomplete_is_not() {
    // S1's distinction, carried all the way to the editor's gutter. An
    // unclosed form is every form being typed; marking it an error would
    // put a red squiggle under the cursor permanently.
    let mut lsp = Lsp::start();

    let bad = "file:///t/bad.scm";
    lsp.open(bad, "(define x 1))\n");
    let d = lsp.diagnostics(bad);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["severity"], 1, "invalid must be an error: {d:?}");
    assert_eq!(d[0]["source"], "scm");

    let half = "file:///t/half.scm";
    lsp.open(half, "(define (f a)\n");
    let d = lsp.diagnostics(half);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["severity"], 3, "incomplete must not be an error: {d:?}");

    let good = "file:///t/good.scm";
    lsp.open(good, "(define x 1)\n");
    assert!(lsp.diagnostics(good).is_empty());
}

#[test]
fn a_change_republishes_and_a_close_clears() {
    let mut lsp = Lsp::start();
    let uri = "file:///t/h.scm";
    lsp.open(uri, "(define x 1))\n");
    assert_eq!(lsp.diagnostics(uri).len(), 1);

    lsp.notify(
        "textDocument/didChange",
        serde_json::json!({"textDocument": {"uri": uri, "version": 2},
                           "contentChanges": [{"text": "(define x 1)\n"}]}),
    );
    assert!(lsp.diagnostics(uri).is_empty(), "the fix was not republished");

    // A closed file's errors must not stay in the problems list.
    lsp.notify("textDocument/didClose",
               serde_json::json!({"textDocument": {"uri": uri}}));
    assert!(lsp.diagnostics(uri).is_empty());
}

#[test]
fn an_unknown_request_is_answered_rather_than_dropped() {
    // A client that asked for something unimplemented and got nothing back
    // would wait for ever, and the editor would look hung rather than
    // merely limited.
    let mut lsp = Lsp::start();
    let got = lsp.request("textDocument/codeLens", serde_json::json!({}));
    assert!(got["result"].is_null(), "{got}");
    assert!(got["id"].is_u64());
}

#[test]
fn shutdown_is_answered_before_the_server_goes() {
    let mut lsp = Lsp::start();
    let got = lsp.request("shutdown", serde_json::json!({}));
    assert!(got["result"].is_null(), "{got}");
}

// --- the live session, which is the point of the stage --------------------

/// The `scm` binary, beside this test's own. `CARGO_BIN_EXE_scm` is not
/// available here because `scm` belongs to another package, so it is found
/// by path -- and asserted rather than skipped, since a test that quietly
/// does nothing is worse than one that fails.
fn scm_binary() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    // target/<profile>/deps/protocol-<hash> -> target/<profile>/scm
    let path = exe.parent().and_then(|p| p.parent()).expect("target dir").join("scm");
    assert!(
        path.exists(),
        "no scm binary at {path:?}; run `cargo build -p scm-cli` (a plain \
         `cargo test` at the workspace root builds it)"
    );
    path
}

#[test]
fn completion_knows_what_was_defined_at_the_prompt() {
    // The design claim of Z3/S22 in one test: the editor's completion comes
    // from the session the user is *working in*, not from a parse of the
    // file. A procedure defined at the prompt and written nowhere must be
    // completable, and no amount of reading the buffer could produce it.
    let mut server = Command::new(scm_binary())
        .args(["--serve", "127.0.0.1:0"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn scm --serve");
    let mut line = String::new();
    BufReader::new(server.stdout.take().unwrap())
        .read_line(&mut line)
        .expect("the server announces its address");
    let addr = line.trim().rsplit(' ').next().unwrap().to_string();
    assert!(addr.contains(':'), "no address in {line:?}");

    // Define it the way a user would: at a prompt, into that session.
    let mut prompt = Command::new(scm_binary())
        .args(["--connect", &addr])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn scm --connect");
    prompt
        .stdin
        .take()
        .unwrap()
        .write_all(b"(define (defined-at-the-prompt x) x)\n")
        .unwrap();
    assert!(prompt.wait().unwrap().success());

    // Now an LSP pointed at the same session.
    let mut child = Command::new(env!("CARGO_BIN_EXE_scm-lsp"))
        .args(["--addr", &addr])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn scm-lsp");
    let stdin = child.stdin.take().unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    let mut lsp = Lsp { child, stdin, out, id: 0 };
    let init = lsp.request("initialize", serde_json::json!({"capabilities": {}}));
    // The handshake says which session it found, so a user with no
    // completion can tell whether the server was reached.
    let origin = init["result"]["serverInfo"]["scmSession"].as_str().unwrap_or("");
    assert!(origin.contains(&addr), "not connected: {origin:?}");

    let uri = "file:///t/live.scm";
    lsp.open(uri, "(defined-at\n");
    let _ = lsp.diagnostics(uri);
    let got = lsp.at("textDocument/completion", uri, 0, 11);
    let labels: Vec<String> = got["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["label"].as_str().map(str::to_string))
        .collect();
    assert!(
        labels.iter().any(|l| l == "defined-at-the-prompt"),
        "the editor cannot see what the prompt defined: {labels:?}"
    );

    // And hover reads its signature off the live value, which no parse of
    // this file could know either.
    lsp.open("file:///t/live2.scm", "(defined-at-the-prompt 1)\n");
    let _ = lsp.diagnostics("file:///t/live2.scm");
    let md = lsp.at("textDocument/hover", "file:///t/live2.scm", 0, 3)["contents"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(md.contains("(defined-at-the-prompt x)"), "no live signature: {md}");

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn a_server_that_goes_away_falls_back_instead_of_dying() {
    // An editing session outlives a `--serve` that gets restarted. If the
    // LSP kept failing silently afterwards, completion would stop working
    // for the rest of the day with nothing to see.
    let mut server = Command::new(scm_binary())
        .args(["--serve", "127.0.0.1:0"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn scm --serve");
    let mut line = String::new();
    BufReader::new(server.stdout.take().unwrap())
        .read_line(&mut line)
        .expect("address");
    let addr = line.trim().rsplit(' ').next().unwrap().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_scm-lsp"))
        .args(["--addr", &addr])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn scm-lsp");
    let stdin = child.stdin.take().unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    let mut lsp = Lsp { child, stdin, out, id: 0 };
    lsp.request("initialize", serde_json::json!({"capabilities": {}}));
    let uri = "file:///t/gone.scm";
    lsp.open(uri, "(vector-\n");
    let _ = lsp.diagnostics(uri);
    assert!(!lsp.at("textDocument/completion", uri, 0, 8)["items"]
        .as_array()
        .unwrap()
        .is_empty());

    let _ = server.kill();
    let _ = server.wait();

    // The first request after the loss is allowed to come back empty; the
    // next must work, from the fallback session.
    let _ = lsp.at("textDocument/completion", uri, 0, 8);
    let labels: Vec<String> = lsp.at("textDocument/completion", uri, 0, 8)["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|i| i["label"].as_str().map(str::to_string))
        .collect();
    assert!(
        labels.iter().any(|l| l == "vector-ref"),
        "no fallback after the server went: {labels:?}"
    );
}

// --- go-to-definition across the workspace --------------------------------

/// A little project on disk, since the index reads files rather than being
/// told about them.
fn project(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("scm-lsp-ws-{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    for (path, text) in files {
        let full = root.join(path);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(full, text).unwrap();
    }
    root
}

impl Lsp {
    /// A server that has indexed a workspace root.
    fn start_in(root: &std::path::Path) -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_scm-lsp"))
            .args(["--addr", "127.0.0.1:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn scm-lsp");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut lsp = Lsp { child, stdin, out, id: 0 };
        let uri = format!("file://{}", root.display());
        let init = lsp.request(
            "initialize",
            serde_json::json!({"capabilities": {},
                               "workspaceFolders": [{"uri": uri, "name": "ws"}]}),
        );
        assert_eq!(init["result"]["capabilities"]["workspaceSymbolProvider"], true);
        lsp
    }
}

#[test]
fn a_jump_leaves_the_file() {
    // The case that matters most and the one that did not work: the
    // definition is somewhere you are *not* looking.
    let root = project(
        "jump",
        &[
            ("src/lib.scm", "; a library\n(define (helper x)\n  (* x 2))\n"),
            ("src/main.scm", "(define (go) (helper 21))\n"),
            // Under `target`, which must not be walked: an entry from here
            // would send a jump into a build directory.
            ("target/stale.scm", "(define (helper x) 'wrong)\n"),
        ],
    );
    let mut lsp = Lsp::start_in(&root);
    let main = format!("file://{}/src/main.scm", root.display());
    lsp.open(&main, "(define (go) (helper 21))\n");
    let _ = lsp.diagnostics(&main);

    let got = lsp.at("textDocument/definition", &main, 0, 14);
    let sites = got.as_array().unwrap_or_else(|| panic!("no jump: {got}"));
    assert_eq!(sites.len(), 1, "{got}");
    assert!(
        sites[0]["uri"].as_str().unwrap().ends_with("src/lib.scm"),
        "jumped to the wrong file: {got}"
    );
    // The name is on line 1 of lib.scm, not line 0 -- so the position is
    // real and not a jump to the top of the file.
    assert_eq!(sites[0]["range"]["start"]["line"], 1, "{got}");
}

#[test]
fn the_local_definition_wins_over_one_elsewhere() {
    // A name defined where you are reading is the one you meant.
    let root = project(
        "shadow",
        &[
            ("src/other.scm", "(define (shared) 'far)\n"),
            ("src/here.scm", "(define (shared) 'near)\n(shared)\n"),
        ],
    );
    let mut lsp = Lsp::start_in(&root);
    let here = format!("file://{}/src/here.scm", root.display());
    lsp.open(&here, "(define (shared) 'near)\n(shared)\n");
    let _ = lsp.diagnostics(&here);
    let got = lsp.at("textDocument/definition", &here, 1, 2);
    // One location, in this file, at line 0.
    assert_eq!(got["uri"], here.as_str(), "{got}");
    assert_eq!(got["range"]["start"]["line"], 0, "{got}");
}

#[test]
fn a_record_accessor_is_reachable_from_another_file() {
    // The gap that started this: `point-x` is bound by a form whose *name*
    // is `point`, so nothing used to find it -- not even in the file that
    // defines it.
    let root = project(
        "record",
        &[
            ("src/types.scm", "(define-record-type point\n  (make-point x y)\n  point?\n  (x point-x))\n"),
            ("src/use.scm", "(define (f p) (point-x p))\n"),
        ],
    );
    let mut lsp = Lsp::start_in(&root);
    let use_uri = format!("file://{}/src/use.scm", root.display());
    lsp.open(&use_uri, "(define (f p) (point-x p))\n");
    let _ = lsp.diagnostics(&use_uri);
    for (name, col, line_in_types) in [("point-x", 15, 3), ("make-point", 0, 1)] {
        let text = if name == "point-x" {
            "(define (f p) (point-x p))\n".to_string()
        } else {
            "(make-point 1 2)\n".to_string()
        };
        lsp.notify(
            "textDocument/didChange",
            serde_json::json!({"textDocument": {"uri": use_uri, "version": 2},
                               "contentChanges": [{"text": text}]}),
        );
        let _ = lsp.diagnostics(&use_uri);
        let got = lsp.at("textDocument/definition", &use_uri, 0, col + 1);
        let sites = got.as_array().unwrap_or_else(|| panic!("{name} not found: {got}"));
        assert!(sites[0]["uri"].as_str().unwrap().ends_with("src/types.scm"), "{name}: {got}");
        assert_eq!(sites[0]["range"]["start"]["line"], line_in_types, "{name}: {got}");
    }
}

#[test]
fn workspace_symbol_finds_a_half_remembered_name() {
    let root = project(
        "symbols",
        &[("src/a.scm", "(define (make-widget) 1)\n(define *widget-count* 0)\n")],
    );
    let mut lsp = Lsp::start_in(&root);
    let got = lsp.request("workspace/symbol", serde_json::json!({"query": "widget"}));
    let found = got["result"].as_array().expect("symbols");
    let names: Vec<&str> = found.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"make-widget"), "{got}");
    assert!(names.contains(&"*widget-count*"), "{got}");
    // With a location that is usable, not a stub.
    let w = found.iter().find(|s| s["name"] == "make-widget").unwrap();
    assert!(w["location"]["uri"].as_str().unwrap().ends_with("src/a.scm"));
}

#[test]
fn an_unsaved_edit_is_what_a_jump_uses() {
    // The buffer is the truth about itself: a definition just typed and not
    // saved must still be a jump target from elsewhere.
    let root = project(
        "unsaved",
        &[("src/a.scm", "(define (old-name) 1)\n"), ("src/b.scm", "(define (caller) 1)\n")],
    );
    let mut lsp = Lsp::start_in(&root);
    let a = format!("file://{}/src/a.scm", root.display());
    let b = format!("file://{}/src/b.scm", root.display());
    lsp.open(&a, "(define (old-name) 1)\n");
    let _ = lsp.diagnostics(&a);
    lsp.notify(
        "textDocument/didChange",
        serde_json::json!({"textDocument": {"uri": a, "version": 2},
                           "contentChanges": [{"text": "(define (brand-new) 1)\n"}]}),
    );
    let _ = lsp.diagnostics(&a);

    lsp.open(&b, "(brand-new)\n");
    let _ = lsp.diagnostics(&b);
    let got = lsp.at("textDocument/definition", &b, 0, 3);
    let sites = got.as_array().unwrap_or_else(|| panic!("unsaved edit not indexed: {got}"));
    assert!(sites[0]["uri"].as_str().unwrap().ends_with("src/a.scm"), "{got}");
}
