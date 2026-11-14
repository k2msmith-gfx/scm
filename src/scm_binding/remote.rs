//! **A socket backend** (docs/editor-crate-plan.md, E5b): SLIME's model,
//! over scm's own REPL protocol (`~/devel/scm/docs/repl.md`). `scm
//! --serve` is the SWANK half; this is the client. A send evaluates in
//! ANOTHER process's image and comes back as a value, what the form printed,
//! or an error with its backtrace; completion and describe ask that image
//! too. The editor starts `scm --serve` itself on a free port, as SLIME
//! starts SBCL with SWANK, or attaches to a server already running — `ray
//! --tcp` on 4007, or a second editor's image.
//!
//! The wire is `scm::Request` / `scm::Response`, formatted and parsed by
//! scm's own code (so this client cannot come to disagree with the server),
//! framed the way `scm --connect` frames it: read until what we hold is one
//! complete datum.
//!
//! **A send does not wait** (async sends, 2026-11-13). Each connection has
//! a READER THREAD that parses answers as they arrive and matches each to
//! its request by `:id`: a send (`%remote-eval-async`) returns its request's
//! id at once, its answer is queued, the host is woken (`crate::wake`), and
//! the shell delivers it with `(remote-poll)` — SLIME's model, so the
//! window stays live while a form runs elsewhere. The quick questions —
//! complete, describe, and a synchronous `%remote-eval` for the REPL
//! commands that need a value now — still wait, for their own answer only,
//! with a timeout: past it the client sends `:interrupt` on a SECOND
//! connection (the server is single-threaded, so an interrupt queued behind
//! the evaluation would never arrive; `scm-mode.el` keeps the same pair).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use scm::{Op, Outcome, Request, Response, Val};

use super::{val_num, val_text, val_textv};

/// What the reader thread and the UI thread share for one connection.
#[derive(Default)]
struct Inbox {
    /// Requests a caller is blocked on, by id: their answer goes straight
    /// to the waiter.
    waiting: Mutex<HashMap<u64, mpsc::Sender<Response>>>,
    /// Async sends not yet answered, and the answers that have come.
    pending: Mutex<HashSet<u64>>,
    done: Mutex<VecDeque<Response>>,
    /// The server closed the connection (or the read failed): what it said.
    closed: Mutex<Option<String>>,
}

struct Conn {
    addr: String,
    writer: TcpStream,
    next_id: u64,
    inbox: Arc<Inbox>,
    /// The server, when this editor started it.
    child: Option<std::process::Child>,
}

thread_local! {
    static CONNS: RefCell<HashMap<u32, Conn>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

/// The reader: framing is the reader (one complete datum per answer, as
/// `scm --connect` reads it), and each answer goes to its waiter, to the
/// async queue, or nowhere (a streamed chunk; an answer nobody asked for).
/// The byte length of the first complete datum in `text` (leading space
/// included), or `None` while it is still arriving. Answers can come back
/// to back in one read now that sends do not wait — `parse_status` says
/// whether the text holds a complete datum, not where the first one ends,
/// and handing it two lost the second. The wire is machine-written: lists,
/// strings with `\` escapes, `#\` characters, atoms.
fn first_datum_len(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'#' if b.get(i + 1) == Some(&b'\\') => i += 2,
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            c if depth == 0 && !c.is_ascii_whitespace() => {
                // A bare atom at top level: up to the next delimiter.
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'(' && b[i] != b')' {
                    i += 1;
                }
                return (i < b.len()).then_some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn read_loop(mut stream: TcpStream, inbox: Arc<Inbox>, addr: String) {
    let mut pending = String::new();
    let mut chunk = [0u8; 8192];
    loop {
        while !pending.trim().is_empty() {
            match first_datum_len(&pending) {
                Some(n) => {
                    let text: String = pending.drain(..n).collect();
                    let Ok(answer) = Response::parse(&text) else { continue };
                    if matches!(answer.outcome, Outcome::Streamed(_)) {
                        continue;
                    }
                    let waiter = inbox.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&answer.id);
                    if let Some(w) = waiter {
                        let _ = w.send(answer);
                    } else if inbox.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&answer.id) {
                        inbox.done.lock().unwrap_or_else(|e| e.into_inner()).push_back(answer);
                        crate::wake::wake();
                    }
                }
                None => break,
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => pending.push_str(&String::from_utf8_lossy(&chunk[..n])),
        }
    }
    // Closed: every send still running fails, saying so, and every waiter
    // wakes with nothing (its channel drops).
    let why = format!("{addr} closed the connection");
    *inbox.closed.lock().unwrap_or_else(|e| e.into_inner()) = Some(why.clone());
    inbox.waiting.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let ids: Vec<u64> = inbox.pending.lock().unwrap_or_else(|e| e.into_inner()).drain().collect();
    let mut done = inbox.done.lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        done.push_back(Response {
            id,
            outcome: Outcome::Failed { message: why.clone(), output: String::new(), host_stopped: true, backtrace: Vec::new() },
        });
    }
    drop(done);
    crate::wake::wake();
}

fn connect(addr: &str, child: Option<std::process::Child>) -> Result<u32, String> {
    let stream = TcpStream::connect(addr).map_err(|e| format!("{addr}: {e}"))?;
    let _ = stream.set_nodelay(true);
    let reader = stream.try_clone().map_err(|e| format!("{addr}: {e}"))?;
    let inbox = Arc::new(Inbox::default());
    let (thread_inbox, thread_addr) = (inbox.clone(), addr.to_string());
    std::thread::spawn(move || read_loop(reader, thread_inbox, thread_addr));
    let id = NEXT.with(|n| n.replace(n.get() + 1));
    CONNS.with(|c| {
        c.borrow_mut()
            .insert(id, Conn { addr: addr.to_string(), writer: stream, next_id: 1, inbox, child })
    });
    Ok(id)
}

/// Start `program ARGS…` serving on 127.0.0.1:PORT (a free port when
/// `port` is 0) and connect to it, retrying while it boots. Each `{addr}` in
/// ARGS becomes the address; with no ARGS it is `--serve {addr}`, scm's.
fn start(program: &str, args: &[String], port: u16) -> Result<u32, String> {
    let port = if port == 0 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port()
    } else {
        port
    };
    let addr = format!("127.0.0.1:{port}");
    let args: Vec<String> = if args.is_empty() {
        vec!["--serve".into(), addr.clone()]
    } else {
        args.iter().map(|a| a.replace("{addr}", &addr)).collect()
    };
    let mut child = std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let t0 = Instant::now();
    loop {
        if TcpStream::connect(&addr).is_ok() {
            return connect(&addr, Some(child));
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("{program} (serving {addr}) exited: {status}"));
        }
        if t0.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            return Err(format!("{program} (serving {addr}) did not listen within 10 s"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Write a request, answering its id.
fn send(conn: &mut Conn, op: Op) -> Result<u64, String> {
    if let Some(why) = conn.inbox.closed.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Err(why);
    }
    let rid = conn.next_id;
    conn.next_id += 1;
    let text = format!("{}\n", Request { id: rid, op }.to_wire());
    conn.writer
        .write_all(text.as_bytes())
        .and_then(|()| conn.writer.flush())
        .map_err(|e| format!("{}: {e}", conn.addr))?;
    Ok(rid)
}

/// Ask, and wait for this request's own answer. Past `timeout` the request
/// is interrupted from a second connection, and the wait goes on for its
/// (host-stopped) answer.
fn ask(id: u32, op: Op, timeout: Duration) -> Result<Response, String> {
    let (rx, addr) = CONNS.with(|c| -> Result<_, String> {
        let mut conns = c.borrow_mut();
        let conn = conns.get_mut(&id).ok_or("no such connection")?;
        let (tx, rx) = mpsc::channel();
        // Registered under the id the request is about to get.
        conn.inbox.waiting.lock().unwrap_or_else(|e| e.into_inner()).insert(conn.next_id, tx);
        send(conn, op)?;
        Ok((rx, conn.addr.clone()))
    })?;
    match rx.recv_timeout(timeout) {
        Ok(answer) => Ok(answer),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(format!("{addr} closed the connection")),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            interrupt(&addr);
            rx.recv_timeout(Duration::from_secs(10))
                .map_err(|_| format!("{addr} did not answer, even interrupted"))
        }
    }
}

/// Send an evaluation and return its request id at once; the answer arrives
/// through [`drain`].
fn eval_async(id: u32, code: String, source: Option<String>) -> Result<u64, String> {
    CONNS.with(|c| {
        let mut conns = c.borrow_mut();
        let conn = conns.get_mut(&id).ok_or("no such connection")?;
        conn.inbox.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(conn.next_id);
        send(conn, Op::Eval { code, source })
    })
}

/// How many async sends to connection `id` are still running.
fn running(id: u32) -> usize {
    CONNS.with(|c| c.borrow().get(&id).map_or(0, |c| c.inbox.pending.lock().unwrap_or_else(|e| e.into_inner()).len()))
}

/// Every async answer that has come, on every connection: (connection,
/// request id, answer).
fn drain() -> Vec<(u32, Response)> {
    CONNS.with(|c| {
        let conns = c.borrow();
        let mut out = Vec::new();
        for (&id, conn) in conns.iter() {
            out.extend(conn.inbox.done.lock().unwrap_or_else(|e| e.into_inner()).drain(..).map(|r| (id, r)));
        }
        out
    })
}

/// `:interrupt`, down a connection of its own.
fn interrupt(addr: &str) {
    if let Ok(mut s) = TcpStream::connect(addr) {
        let text = format!("{}\n", Request { id: 0, op: Op::Interrupt }.to_wire());
        let _ = s.write_all(text.as_bytes());
    }
}

fn close(id: u32) {
    if let Some(mut conn) = CONNS.with(|c| c.borrow_mut().remove(&id)) {
        if let Some(mut child) = conn.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Stop every server this editor started — at exit, so a Scheme the editor
/// ran does not outlive it.
pub fn close_all() {
    let ids: Vec<u32> = CONNS.with(|c| c.borrow().keys().copied().collect());
    for id in ids {
        close(id);
    }
}

fn entry(k: &str, v: Val) -> Val {
    Val::Pair(Box::new(Val::Keyword(k.into())), Box::new(v))
}

/// An eval's answer as a backend RESULT: ((:status . ok) (:value .
/// "printed") (:output . "…")) or ((:status . error) (:message . "…")
/// (:output . "…") (:backtrace . ("form  at loc" …)) (:host-stopped . #t/#f)).
fn eval_result(outcome: Outcome) -> Result<Val, String> {
    Ok(match outcome {
        Outcome::Value { value, output } => Val::List(vec![
            entry("status", Val::Sym("ok".into())),
            entry("value", Val::Str(value)),
            entry("output", Val::Str(output)),
        ]),
        Outcome::Failed { message, output, host_stopped, backtrace } => Val::List(vec![
            entry("status", Val::Sym("error".into())),
            entry("message", Val::Str(message)),
            entry("output", Val::Str(output)),
            entry("backtrace", Val::List(backtrace.into_iter().map(Val::Str).collect())),
            entry("host-stopped", Val::Bool(host_stopped)),
        ]),
        Outcome::Malformed(why) => Val::List(vec![
            entry("status", Val::Sym("error".into())),
            entry("message", Val::Str(format!("the server could not read the request: {why}"))),
            entry("output", Val::Str(String::new())),
        ]),
        other => return Err(format!("unexpected answer to eval: {other:?}")),
    })
}

fn conn_id(args: &[Val]) -> u32 {
    val_num(args, 0) as u32
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%remote-start program port [args])` — run PROGRAM serving on PORT
    // (0: a free one) and connect: the connection's id. ARGS, a list in
    // which "{addr}" stands for the address, default to scm's `--serve
    // {addr}`; Guile's are its serve.scm (E6). Raises when it cannot.
    interp.define_fn("%remote-start", |args| {
        let program = val_text(args, 0).unwrap_or_else(|| "scm".into());
        let argv = val_textv(args, 2);
        Ok(Val::Int(start(&program, &argv, val_num(args, 1).max(0.0) as u16)? as i64))
    })?;
    // `(%remote-connect "host:port")` — attach to a server already running.
    interp.define_fn("%remote-connect", |args| {
        let addr = val_text(args, 0).ok_or("expected \"host:port\"")?;
        Ok(Val::Int(connect(&addr, None)? as i64))
    })?;
    // `(%remote-address id)` — what the connection is to, or #f.
    interp.define_fn("%remote-address", |args| {
        let id = conn_id(args);
        Ok(CONNS.with(|c| c.borrow().get(&id).map_or(Val::Bool(false), |c| Val::Str(c.addr.clone()))))
    })?;
    // `(%remote-close id)` — drop it, stopping the server if we started it.
    interp.define_fn("%remote-close", |args| {
        close(conn_id(args));
        Ok(Val::Bool(false))
    })?;
    // `(%remote-eval id code source timeout)` — a backend RESULT: ((:status
    // . ok) (:value . "printed") (:output . "…")) or ((:status . error)
    // (:message . "…") (:output . "…") (:backtrace . ("form  at loc" …))
    // (:host-stopped . #t/#f)).
    interp.define_fn("%remote-eval", |args| {
        let code = val_text(args, 1).unwrap_or_default();
        let source = val_text(args, 2);
        let timeout = Duration::from_secs_f64(match args.get(3) {
            Some(Val::Int(n)) => *n as f64,
            Some(Val::Float(x)) => *x,
            _ => 30.0,
        });
        let answer = ask(conn_id(args), Op::Eval { code, source }, timeout)?;
        eval_result(answer.outcome)
    })?;
    // `(%remote-eval-async id code source)` — send an evaluation and answer
    // its request id at once; the result arrives through %remote-poll.
    interp.define_fn("%remote-eval-async", |args| {
        let code = val_text(args, 1).unwrap_or_default();
        Ok(Val::Int(eval_async(conn_id(args), code, val_text(args, 2))? as i64))
    })?;
    // `(%remote-running id)` — how many async sends to it are unanswered.
    interp.define_fn("%remote-running", |args| Ok(Val::Int(running(conn_id(args)) as i64)))?;
    // `(%remote-poll)` — every async answer that has come, on any
    // connection: ((connection request-id result) …), oldest first.
    interp.define_fn("%remote-poll", |_| {
        let mut out = Vec::new();
        for (conn, answer) in drain() {
            out.push(Val::List(vec![Val::Int(conn as i64), Val::Int(answer.id as i64), eval_result(answer.outcome)?]));
        }
        Ok(Val::List(out))
    })?;
    // `(%remote-complete id prefix)` — the server's global names with PREFIX.
    interp.define_fn("%remote-complete", |args| {
        let prefix = val_text(args, 1).unwrap_or_default();
        match ask(conn_id(args), Op::Complete { prefix }, Duration::from_secs(5))?.outcome {
            Outcome::Candidates(c) => Ok(Val::List(c.into_iter().map(Val::Str).collect())),
            other => Err(format!("unexpected answer to complete: {other:?}")),
        }
    })?;
    // `(%remote-describe id name)` — ((:kind . "…") (:signature . "…"/#f)
    // (:doc . "…"/#f) (:value . "…")), or #f when NAME is unbound there.
    interp.define_fn("%remote-describe", |args| {
        let name = val_text(args, 1).unwrap_or_default();
        match ask(conn_id(args), Op::Describe { name }, Duration::from_secs(5))?.outcome {
            Outcome::Described(None) => Ok(Val::Bool(false)),
            Outcome::Described(Some(d)) => {
                let opt = |s: Option<String>| s.map_or(Val::Bool(false), Val::Str);
                Ok(Val::List(vec![
                    entry("kind", Val::Str(d.kind.to_string())),
                    entry("signature", opt(d.signature)),
                    entry("doc", opt(d.doc)),
                    entry("value", Val::Str(d.value)),
                ]))
            }
            other => Err(format!("unexpected answer to describe: {other:?}")),
        }
    })?;
    // `(%remote-interrupt id)` — stop what the server is evaluating.
    interp.define_fn("%remote-interrupt", |args| {
        let id = conn_id(args);
        if let Some(addr) = CONNS.with(|c| c.borrow().get(&id).map(|c| c.addr.clone())) {
            interrupt(&addr);
        }
        Ok(Val::Bool(false))
    })?;
    Ok(vec![
        "%remote-start",
        "%remote-connect",
        "%remote-address",
        "%remote-close",
        "%remote-eval",
        "%remote-complete",
        "%remote-describe",
        "%remote-interrupt",
        "%remote-eval-async",
        "%remote-running",
        "%remote-poll",
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-connection REPL server over scm's own `Session` — the protocol
    /// `scm --serve` speaks, without needing the binary.
    fn serve() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let mut session = scm::Session::new().unwrap();
            let (mut s, _) = listener.accept().unwrap();
            let mut pending = String::new();
            let mut buf = [0u8; 4096];
            loop {
                if !pending.trim().is_empty() && scm::parse_status(&pending) == scm::ParseStatus::Complete {
                    let req = Request::parse(&std::mem::take(&mut pending)).unwrap();
                    let out = format!("{}\n", session.handle(req).to_wire());
                    s.write_all(out.as_bytes()).unwrap();
                    continue;
                }
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => pending.push_str(&String::from_utf8_lossy(&buf[..n])),
                }
            }
        });
        addr
    }

    #[test]
    fn answers_back_to_back_split_one_datum_at_a_time() {
        let two = "(:id 1 :status ok :value \"a (\\\" ;\" :output \"\")\n(:id 2 :status ok :value \"#\\\\)\" :output \"\")\n";
        let n = first_datum_len(two).unwrap();
        assert_eq!(Response::parse(&two[..n]).unwrap().id, 1);
        let rest = &two[n..];
        let m = first_datum_len(rest).unwrap();
        assert_eq!(Response::parse(&rest[..m]).unwrap().id, 2);
        assert_eq!(first_datum_len("(:id 3 :status"), None, "still arriving");
    }

    /// A buffer attached to another process's image: a send evaluates
    /// THERE and answers as an in-process one does — the value in the
    /// transcript, an error with its calls — and describe asks it too.
    #[test]
    fn a_buffer_sends_to_a_remote_image() {
        let addr = serve();
        crate::editorpane::set_vi(false);
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        s.call(&format!("(remote-connect \"{addr}\")"));
        s.call(r#"(%editor-set-text 1 "(define (area r) \"Circle area.\" (* 3 r r))\n(area 2)")"#);
        s.call("(%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(10.0), "the answers arrive");
        let transcript: Vec<String> =
            crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(transcript.iter().any(|l| l == "=> 12"), "{transcript:?}");
        // Not this image's: `area` is defined over there only.
        assert_eq!(s.eval_to_string("(%env-ref 'area #f)").unwrap(), "#f");
        assert_eq!(
            s.eval_to_string("((backend-op (backend-of (%editor-active-buffer 1)) :doc) \"area\")").unwrap(),
            "\"Circle area.\""
        );
        // An error comes back with the calls it was in.
        s.call(r#"(%editor-set-text 1 "(vector-ref (vector 1 2) 9)") (%editor-goto-line 1 0) (eval-defun)"#);
        assert!(s.settle(10.0));
        assert_eq!(s.eval_to_string("(list-ref *last-error* 2)").unwrap(), "\"vector-ref: index 9 out of range\"");
        assert!(s.eval_to_string("(list-ref *last-error* 3)").unwrap().contains("(vector-ref (vector 1 2) 9)"));
        s.call("(remote-stop)");
    }

    /// The point of async sends: a slow form does not hold the editor. The
    /// send returns at once, describe answers nothing rather than queue
    /// behind it, and the answer arrives later through remote-poll.
    #[test]
    fn a_slow_send_does_not_wait() {
        let addr = serve();
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        s.call(&format!("(remote-connect \"{addr}\")"));
        s.call(r#"(%editor-set-text 1 "(let loop ((i 0)) (if (< i 600000) (loop (+ i 1)) i))")"#);
        let t0 = std::time::Instant::now();
        s.call("(%editor-goto-line 1 0) (eval-defun)");
        let send = t0.elapsed();
        let running = s.eval_to_string("(%remote-running *remote*)").unwrap();
        let t1 = std::time::Instant::now();
        let doc = s.eval_to_string("((backend-op (backend-of (%editor-active-buffer 1)) :doc) \"car\")").unwrap();
        let ask = t1.elapsed();
        assert!(s.settle(30.0), "the answer arrives");
        let total = t0.elapsed();
        let t: Vec<String> = crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(t.iter().any(|l| l == "=> 600000"), "{t:?}");
        eprintln!("send {send:?}, describe while running {ask:?} ({running} running, doc {doc}), answer after {total:?}");
        assert!(send < std::time::Duration::from_millis(100), "the send returned at once: {send:?}");
        if running == "1" {
            assert_eq!(doc, "#f", "describe does not queue behind a running send");
            assert!(ask < std::time::Duration::from_millis(100), "{ask:?}");
        }
        s.call("(remote-stop)");
    }
}

/// E6: GNU Guile through the same backend — a Guile the editor starts, running
/// edit/guile/serve.scm, which speaks scm's protocol.
#[cfg(test)]
mod guile_tests {
    fn guile_on_path() -> bool {
        std::process::Command::new("guile").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    fn transcript() -> Vec<String> {
        crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect()
    }

    /// Always: a file that says it is Guile opens as Guile, and a Guile
    /// send never falls through to scm's image when Guile cannot start.
    #[test]
    fn a_guile_file_is_guiles_and_never_runs_as_scm() {
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-guile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hello.scm");
        std::fs::write(&file, "(define-module (hello))\n(define (sq x) (* x x))\n").unwrap();
        let file = file.to_string_lossy().replace('\\', "/");
        s.call(&format!("(open-path \"{file}\")"));
        assert_eq!(s.eval_to_string("(%buffer-language 1 (%editor-active-buffer 1))").unwrap(), "\"guile\"");
        s.call("(set! *guile-program* \"no-such-guile-here\")");
        s.call("(%editor-goto-line 1 1) (eval-defun)");
        assert_eq!(s.eval_to_string("(%env-ref 'sq #f)").unwrap(), "#f", "nothing ran in scm's image");
        assert!(transcript().iter().any(|l| l.starts_with("error: guile:")), "{:?}", transcript());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With Guile on the PATH: a Guile buffer's sends evaluate in Guile —
    /// its own syntax included — and describe asks it.
    #[test]
    fn a_guile_buffer_sends_to_guile() {
        if !guile_on_path() {
            eprintln!("guile is not on the PATH (nix develop has it): skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        s.call("(let ((b (%editor-buffer-new 1))) (register-buffer b #f) (visit-buffer b)) (guile-mode)");
        s.call(r#"(%editor-set-text 1 "(define* (sq x #:key (by 1)) \"Squares X.\" (* x x by))\n(sq 7 #:by 2)")"#);
        s.call("(%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(30.0), "Guile answers");
        assert!(transcript().iter().any(|l| l == "=> 98"), "{:?}", transcript());
        assert_eq!(s.eval_to_string("(%env-ref 'sq #f)").unwrap(), "#f", "in Guile, not here");
        let b = "(backend-of (%editor-active-buffer 1))";
        assert_eq!(s.eval_to_string(&format!("((backend-op {b} :doc) \"sq\")")).unwrap(), "\"Squares X.\"");
        assert_eq!(s.eval_to_string(&format!("((backend-op {b} :signature) \"sq\")")).unwrap(), "\"(sq x #:key by)\"");
        // The REPL follows the buffer: Guile's prompt, Guile's image.
        s.call("(repl)");
        for c in "(sq 3)".chars() {
            s.text(&c.to_string());
        }
        s.key("return");
        // Guile's own REPL commands work at it, as at Guile's prompt.
        for line in [",help", ",describe sq"] {
            for c in line.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
        }
        assert!(s.settle(10.0), "the REPL's lines are answered");
        let t = transcript();
        assert!(t.iter().any(|l| l == "scheme@(guile-user)> (sq 3)"), "{t:?}");
        assert!(t.iter().any(|l| l == "=> 9"), "{t:?}");
        assert!(t.iter().any(|l| l.contains(",apropos REGEXP")), ",help listed the commands: {t:?}");
        assert!(t.iter().any(|l| l.contains("Squares X.")), ",describe showed the docstring: {t:?}");
        s.call("(focus-scene-editor)");
        // remote-start asks which Scheme; a scratch buffer started on Guile
        // becomes a Guile buffer, its sends Guile's.
        s.call("(let ((b (%editor-buffer-new 1))) (register-buffer b #f) (visit-buffer b))");
        assert_eq!(s.eval_to_string("(buffer-scheme)").unwrap(), "\"scm\"", "a scratch is scm's");
        s.call("(remote-start \"guile\")");
        assert_eq!(s.eval_to_string("(%buffer-language 1 (%editor-active-buffer 1))").unwrap(), "\"guile\"");
        s.call(r#"(%editor-set-text 1 "(define* (k #:key (v 3)) v)\n(k #:v 4)") (%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)"#);
        assert!(s.settle(10.0));
        assert!(transcript().iter().any(|l| l == "=> 4"), "{:?}", transcript());
        // The prompt offers the buffer's own Scheme first.
        s.call("(remote-start)");
        assert_eq!(s.eval_to_string("(%edit :text)").unwrap_or_default(), "\"guile\"");
        s.key("C-g");
        s.call("(remote-stop)");
        // An error comes back with Guile's message.
        s.call(r#"(%editor-set-text 1 "(car 5)") (%editor-goto-line 1 0) (eval-defun)"#);
        assert!(s.settle(10.0));
        assert!(transcript().iter().any(|l| l.starts_with("error: car:")), "{:?}", transcript());
        s.call("(guile-stop)");
    }
}
