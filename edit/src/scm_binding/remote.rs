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
//!
//! **Events** (scm's S40): `remote-connect` asks the server for messages
//! nobody asked for -- `(:op ping :events #t)`, a plain ping to a server
//! that has none -- and the reader queues each `(:event …)` that comes,
//! apart from the answers, for `(remote-poll)` to show. lux sends `paused`
//! when a frame's error pauses it, after the send that caused it was
//! answered `ok`, and `resumed`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use scm::{Event, Op, Outcome, Request, Response, Val};

use super::{val_num, val_text, val_textv};

/// What the reader thread and the UI thread share for one connection.
#[derive(Default)]
struct Inbox {
    /// Requests a caller is blocked on, by id: their answer goes straight
    /// to the waiter.
    waiting: Mutex<HashMap<u64, mpsc::Sender<(Response, String)>>>,
    /// Async sends not yet answered, and the answers that have come.
    pending: Mutex<HashSet<u64>>,
    done: Mutex<VecDeque<(Response, String)>>,
    /// Events the server pushed, as they came (S40): wire text, read on
    /// the UI thread, since a `Val` cannot cross threads.
    events: Mutex<VecDeque<String>>,
    /// The server closed the connection (or the read failed): what it said.
    closed: Mutex<Option<String>>,
    /// ONE request in flight at a time; the rest wait here. A server that
    /// frames its input as "read until the buffer holds a complete datum" —
    /// `scm --serve` does — parses only the first of two requests that
    /// arrive in one read, and drops the second; async sends made two in a
    /// row possible (two quick C-M-x), and a test lost an answer to it in
    /// the gate. The server evaluates one at a time anyway, so queueing
    /// here costs nothing. The reader sends the next when an answer lands.
    outbox: Mutex<Outbox>,
}

#[derive(Default)]
struct Outbox {
    writer: Option<TcpStream>,
    in_flight: bool,
    queue: VecDeque<String>,
}

impl Inbox {
    /// Write TEXT now if nothing is in flight, else queue it.
    fn submit(&self, text: String) -> std::io::Result<()> {
        let mut out = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
        if out.in_flight {
            out.queue.push_back(text);
            return Ok(());
        }
        out.in_flight = true;
        let w = out.writer.as_mut().ok_or_else(|| std::io::Error::other("no writer"))?;
        w.write_all(text.as_bytes()).and_then(|()| w.flush())
    }

    /// An answer landed: send the next queued request, or note that none is
    /// in flight.
    fn next(&self) {
        let mut out = self.outbox.lock().unwrap_or_else(|e| e.into_inner());
        match out.queue.pop_front() {
            Some(text) => {
                if let Some(w) = out.writer.as_mut() {
                    let _ = w.write_all(text.as_bytes()).and_then(|()| w.flush());
                }
            }
            None => out.in_flight = false,
        }
    }
}

struct Conn {
    addr: String,
    next_id: u64,
    inbox: Arc<Inbox>,
    /// The server, when this editor started it.
    child: Option<std::process::Child>,
}

thread_local! {
    static CONNS: RefCell<HashMap<u32, Conn>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

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

/// The reader: framing is the reader (one complete datum per answer), and
/// each answer goes to its waiter, to the async queue, or nowhere (a
/// streamed chunk; an answer nobody asked for) — and lets the next queued
/// request go.
fn read_loop(mut stream: TcpStream, inbox: Arc<Inbox>, addr: String) {
    let mut pending = String::new();
    let mut chunk = [0u8; 8192];
    loop {
        while !pending.trim().is_empty() {
            match first_datum_len(&pending) {
                Some(n) => {
                    let text: String = pending.drain(..n).collect();
                    // An event answers nothing, so it lets no request go.
                    if text.trim_start().starts_with("(:event") && Event::parse(&text).is_ok() {
                        inbox.events.lock().unwrap_or_else(|e| e.into_inner()).push_back(text);
                        crate::wake::wake();
                        continue;
                    }
                    let Ok(answer) = Response::parse(&text) else { continue };
                    if matches!(answer.outcome, Outcome::Streamed(_)) {
                        continue;
                    }
                    inbox.next();
                    let waiter = inbox.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(&answer.id);
                    // The text goes too: keys scm's Response does not know
                    // (SBCL's break, below) are read from it.
                    if let Some(w) = waiter {
                        let _ = w.send((answer, text));
                    } else if inbox.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&answer.id) {
                        inbox.done.lock().unwrap_or_else(|e| e.into_inner()).push_back((answer, text));
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
        done.push_back((
            Response {
                id,
                outcome: Outcome::Failed { message: why.clone(), output: String::new(), host_stopped: true, backtrace: Vec::new(), locals: Vec::new() },
            },
            String::new(),
        ));
    }
    drop(done);
    crate::wake::wake();
}

fn connect(addr: &str, child: Option<std::process::Child>) -> Result<u32, String> {
    let stream = TcpStream::connect(addr).map_err(|e| format!("{addr}: {e}"))?;
    let _ = stream.set_nodelay(true);
    let reader = stream.try_clone().map_err(|e| format!("{addr}: {e}"))?;
    let inbox = Arc::new(Inbox::default());
    inbox.outbox.lock().unwrap_or_else(|e| e.into_inner()).writer = Some(stream);
    let (thread_inbox, thread_addr) = (inbox.clone(), addr.to_string());
    std::thread::spawn(move || read_loop(reader, thread_inbox, thread_addr));
    let id = NEXT.with(|n| n.replace(n.get() + 1));
    CONNS.with(|c| {
        c.borrow_mut()
            .insert(id, Conn { addr: addr.to_string(), next_id: 1, inbox, child })
    });
    Ok(id)
}

/// Start `program ARGS…` serving on 127.0.0.1:PORT (a free port when
/// `port` is 0) and connect to it, retrying while it boots. Each `{addr}` in
/// ARGS becomes the address; with no ARGS it is `--serve {addr}`, scm's.
fn start(program: &str, args: &[String], port: u16, dir: Option<&str>) -> Result<u32, String> {
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
    let mut command = std::process::Command::new(program);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command
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
    conn.inbox.submit(text).map_err(|e| format!("{}: {e}", conn.addr))?;
    Ok(rid)
}

/// Ask, and wait for this request's own answer. Past `timeout` the request
/// is interrupted from a second connection, and the wait goes on for its
/// (host-stopped) answer.
fn ask(id: u32, op: Op, timeout: Duration) -> Result<Response, String> {
    ask_text(id, op, timeout).map(|(answer, _)| answer)
}

/// [`ask`], with the answer's text as it came.
fn ask_text(id: u32, op: Op, timeout: Duration) -> Result<(Response, String), String> {
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
/// answer, its text).
fn drain() -> Vec<(u32, Response, String)> {
    CONNS.with(|c| {
        let conns = c.borrow();
        let mut out = Vec::new();
        for (&id, conn) in conns.iter() {
            out.extend(conn.inbox.done.lock().unwrap_or_else(|e| e.into_inner()).drain(..).map(|(r, text)| (id, r, text)));
        }
        out
    })
}

/// Ask connection `id`'s server for its events, not waiting: the answer is
/// a pong, and the reader lets the next request go when it lands.
fn subscribe(id: u32) -> Result<(), String> {
    CONNS.with(|c| {
        let mut conns = c.borrow_mut();
        let conn = conns.get_mut(&id).ok_or("no such connection")?;
        send(conn, Op::Events { on: true }).map(|_| ())
    })
}

/// Every event that has come, on every connection: (connection, event),
/// oldest first on each.
fn drain_events() -> Vec<(u32, Event)> {
    CONNS.with(|c| {
        let conns = c.borrow();
        let mut out = Vec::new();
        for (&id, conn) in conns.iter() {
            let texts: Vec<String> = conn.inbox.events.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
            out.extend(texts.iter().filter_map(|t| Event::parse(t).ok()).map(|ev| (id, ev)));
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
    if let Some(mut conn) = CONNS.with(|c| c.borrow_mut().remove(&id))
        && let Some(mut child) = conn.child.take() {
            let _ = child.kill();
            let _ = child.wait();
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
/// (:output . "…") (:backtrace . ("form  at loc" …)) (:host-stopped . #t/#f)),
/// and after either the keys of TEXT, the answer as it came, that only a
/// server of another language sends ([`EXTRA_KEYS`]).
fn eval_result(outcome: Outcome, text: &str) -> Result<Val, String> {
    let mut result = eval_outcome(outcome)?;
    if let Val::List(entries) = &mut result {
        entries.extend(extra_keys(text).into_iter().map(|(k, v)| entry(&k, v)));
    }
    Ok(result)
}

/// The keys an eval's answer may carry that scm's protocol does not have:
/// SBCL's (edit/sbcl/serve.lisp, docs/sbcl-plan.md L7) — the break level
/// the connection is in, its numbered restarts, each call's locals beside
/// `:backtrace`, the restart waiting for a value, and the package the next
/// line reads in; and the compiler's warnings with where each one's form
/// began in the text sent (L6).
const EXTRA_KEYS: [&str; 7] = ["level", "restarts", "locals", "asking", "package", "warnings", "warning-at"];

/// TEXT's [`EXTRA_KEYS`], read as the wire writes them: strings with
/// Scheme's escapes, integers, `#t`/`#f`, symbols and lists of them.
fn extra_keys(text: &str) -> Vec<(String, Val)> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let Some(Val::List(items)) = wire_datum(&chars, &mut i) else { return Vec::new() };
    items
        .chunks(2)
        .filter_map(|kv| match kv {
            [Val::Keyword(k), v] if EXTRA_KEYS.contains(&k.as_str()) => Some((k.clone(), v.clone())),
            _ => None,
        })
        .collect()
}

fn wire_datum(chars: &[char], i: &mut usize) -> Option<Val> {
    while chars.get(*i).is_some_and(|c| c.is_whitespace()) {
        *i += 1;
    }
    match *chars.get(*i)? {
        '(' => {
            *i += 1;
            let mut items = Vec::new();
            loop {
                while chars.get(*i).is_some_and(|c| c.is_whitespace()) {
                    *i += 1;
                }
                if *chars.get(*i)? == ')' {
                    *i += 1;
                    return Some(Val::List(items));
                }
                items.push(wire_datum(chars, i)?);
            }
        }
        '"' => {
            *i += 1;
            let mut s = String::new();
            loop {
                let c = *chars.get(*i)?;
                *i += 1;
                match c {
                    '"' => return Some(Val::Str(s)),
                    '\\' => {
                        let e = *chars.get(*i)?;
                        *i += 1;
                        s.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            e => e,
                        });
                    }
                    c => s.push(c),
                }
            }
        }
        _ => {
            let start = *i;
            while chars.get(*i).is_some_and(|c| !c.is_whitespace() && *c != '(' && *c != ')' && *c != '"') {
                *i += 1;
            }
            let atom: String = chars[start..*i].iter().collect();
            Some(match atom.as_str() {
                "#t" => Val::Bool(true),
                "#f" => Val::Bool(false),
                a if a.starts_with(':') => Val::Keyword(a[1..].into()),
                a => a.parse().map_or_else(|_| Val::Sym(a.into()), Val::Int),
            })
        }
    }
}

fn eval_outcome(outcome: Outcome) -> Result<Val, String> {
    Ok(match outcome {
        Outcome::Value { value, output } => Val::List(vec![
            entry("status", Val::Sym("ok".into())),
            entry("value", Val::Str(value)),
            entry("output", Val::Str(output)),
        ]),
        Outcome::Failed { message, output, host_stopped, backtrace, locals } => Val::List(vec![
            entry("status", Val::Sym("error".into())),
            entry("message", Val::Str(message)),
            entry("output", Val::Str(output)),
            entry("backtrace", Val::List(backtrace.into_iter().map(Val::Str).collect())),
            entry("host-stopped", Val::Bool(host_stopped)),
            // scm S43: each call's locals, ((name printed) …) a call --
            // the shape SBCL's server already sends, which 14-remote reads.
            entry(
                "locals",
                Val::List(
                    locals
                        .into_iter()
                        .map(|frame| {
                            Val::List(frame.into_iter().map(|(n, v)| Val::List(vec![Val::Str(n), Val::Str(v)])).collect())
                        })
                        .collect(),
                ),
            ),
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
    // {addr}`; Guile's are its serve.scm (E6). DIR, when given, is the
    // directory it runs in (ray's, its checkout). Raises when it cannot.
    interp.define_fn("%remote-start", |args| {
        let program = val_text(args, 0).unwrap_or_else(|| "scm".into());
        let argv = val_textv(args, 2);
        let dir = val_text(args, 3);
        Ok(Val::Int(start(&program, &argv, val_num(args, 1).max(0.0) as u16, dir.as_deref())? as i64))
    })?;
    // `(%remote-connect "host:port")` — attach to a server already running.
    interp.define_fn("%remote-connect", |args| {
        let addr = val_text(args, 0).ok_or("expected \"host:port\"")?;
        Ok(Val::Int(connect(&addr, None)? as i64))
    })?;
    // `(%remote-address id)` — what the connection is to, or #f: #f too once
    // the server has closed it (a lux window closed, a Scheme that exited),
    // so a backend that asks whether its server is up starts it again.
    interp.define_fn("%remote-address", |args| {
        let id = conn_id(args);
        Ok(CONNS.with(|c| {
            c.borrow()
                .get(&id)
                .filter(|c| c.inbox.closed.lock().unwrap_or_else(|e| e.into_inner()).is_none())
                .map_or(Val::Bool(false), |c| Val::Str(c.addr.clone()))
        }))
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
        let (answer, text) = ask_text(conn_id(args), Op::Eval { code, source }, timeout)?;
        eval_result(answer.outcome, &text)
    })?;
    // `(%remote-eval-in id code frame timeout)` — scm S43: evaluate in call
    // FRAME (innermost 0) of the last error the server kept, its locals in
    // scope; a backend RESULT, as %remote-eval's.
    interp.define_fn("%remote-eval-in", |args| {
        let code = val_text(args, 1).unwrap_or_default();
        let frame = val_num(args, 2).max(0.0) as usize;
        let timeout = Duration::from_secs_f64(match args.get(3) {
            Some(Val::Int(n)) => *n as f64,
            Some(Val::Float(x)) => *x,
            _ => 30.0,
        });
        let (answer, text) = ask_text(conn_id(args), Op::EvalIn { code, frame }, timeout)?;
        eval_result(answer.outcome, &text)
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
        for (conn, answer, text) in drain() {
            out.push(Val::List(vec![Val::Int(conn as i64), Val::Int(answer.id as i64), eval_result(answer.outcome, &text)?]));
        }
        Ok(Val::List(out))
    })?;
    // `(%remote-subscribe id)` — ask the server for its events (S40). A
    // server with none answers it as a ping.
    interp.define_fn("%remote-subscribe", |args| {
        subscribe(conn_id(args))?;
        Ok(Val::Bool(true))
    })?;
    // `(%remote-events)` — every event that has come, on any connection:
    // ((connection ((:event . paused) (:message . "…") (:backtrace . (…))))
    // …), oldest first.
    interp.define_fn("%remote-events", |_| {
        Ok(Val::List(
            drain_events()
                .into_iter()
                .map(|(conn, ev)| {
                    let mut alist = vec![entry("event", Val::Sym(ev.name))];
                    alist.extend(ev.fields.into_iter().map(|(k, v)| entry(&k, v)));
                    Val::List(vec![Val::Int(conn as i64), Val::List(alist)])
                })
                .collect(),
        ))
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
        "%remote-eval-in",
        "%remote-complete",
        "%remote-describe",
        "%remote-interrupt",
        "%remote-eval-async",
        "%remote-running",
        "%remote-poll",
        "%remote-subscribe",
        "%remote-events",
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-connection REPL server over scm's own `Session` — the protocol
    /// `scm --serve` speaks, without needing the binary.
    fn serve() -> String {
        serve_pausing(std::time::Duration::ZERO)
    }

    /// The same server, pausing after it accepts: whatever the client sends
    /// meanwhile arrives in ONE read, as it can under load — the case its
    /// "read until a complete datum" framing gets wrong.
    fn serve_pausing(pause: std::time::Duration) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let mut session = scm::Session::new().unwrap();
            let (mut s, _) = listener.accept().unwrap();
            std::thread::sleep(pause);
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
        assert!(transcript.iter().any(|l| l == "=> area"), "a remote definition answers its name: {transcript:?}");
        assert!(!transcript.iter().any(|l| l.contains("#<unspecified>")), "{transcript:?}");
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

    /// scm S43: a remote send that fails comes back with its calls' locals,
    /// for the error pane, and eval-in-frame evaluates in one of them, its
    /// locals in scope, in the program it failed in.
    #[test]
    fn a_remote_error_keeps_its_calls_locals_to_evaluate_in() {
        let addr = serve();
        crate::editorpane::set_vi(false);
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        s.call(&format!("(remote-connect \"{addr}\")"));
        s.call(r#"(%editor-set-text 1 "(define (ninth v) (let ((n (vector-length v))) (vector-ref v 9)))\n(ninth (vector 1 2))")"#);
        s.call("(%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(10.0));
        let frames = s.eval_to_string("(list-ref *last-error* 3)").unwrap();
        assert!(frames.contains(r#"("n" "2")"#) && frames.contains(r##"("v" "#(1 2)")"##), "{frames}");
        assert!(s.eval_to_string("(error-markdown *last-error*)").unwrap().contains("M-x eval-in-frame"));
        s.call(r#"(eval-in-frame 1 "(list n (vector-ref v 1))")"#);
        let transcript: Vec<String> =
            crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect();
        assert!(transcript.iter().any(|l| l == "(2 2)"), "{transcript:?}");
        s.call("(remote-stop)");
    }

    /// Sends fired back to back are all answered, in order: one request in
    /// flight per connection, the rest queued — a server that frames its
    /// input by "read until a complete datum" (scm --serve, and this test's)
    /// drops the second of two requests arriving in one read, and the gate
    /// lost an answer to it once async sends could send two in a row.
    #[test]
    fn sends_back_to_back_are_all_answered_in_order() {
        let addr = serve_pausing(std::time::Duration::from_millis(300));
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        s.call(&format!("(remote-connect \"{addr}\")"));
        s.call(r#"
            (define got '())
            (define b (backend-of (%editor-active-buffer 1)))
            (let loop ((i 0))
              (when (< i 20)
                ((backend-op b :eval-async) (number->string i)
                  (lambda (r) (set! got (cons (%alist-ref r :printed "") got))))
                (loop (+ i 1))))"#);
        assert!(s.settle(20.0), "every send is answered");
        let want = (0..20).map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(" ");
        assert_eq!(s.eval_to_string("(reverse got)").unwrap(), format!("({want})"));
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

    /// scm's S40, as lux uses it: remote-connect asks for events, and a
    /// frame's error -- which no send raised, so no answer carries it --
    /// shows in the transcript and the error pane with its calls; resumed
    /// follows. A real `scm::Server`, retaining `paused` as lux does while
    /// paused, so it arrives the moment the editor asks.
    #[test]
    fn a_servers_event_shows_as_a_failed_send() {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut session = scm::Session::new().unwrap();
            let server = scm::Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
            tx.send((server.local_addr().to_string(), server.event_sender())).unwrap();
            loop {
                server.serve_pending(&mut session, |_, _| {});
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let (addr, events) = rx.recv().unwrap();
        events.retain_event(Some(&Event::new(
            "paused",
            vec![
                ("message", Val::Str("view: car: not a pair: 0".into())),
                ("backtrace", Val::List(vec![Val::Str("(car n)  at lantern.scm:4:19".into()), Val::Str("(view m)".into())])),
            ],
        )));
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        s.call(&format!("(remote-connect \"{addr}\")"));
        let transcript = || -> Vec<String> { crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect() };
        let until = |s: &mut super::super::standalone::Session, line: &str| {
            let t0 = Instant::now();
            while !transcript().iter().any(|l| l == line) {
                assert!(t0.elapsed() < Duration::from_secs(10), "no {line:?} in {:?}", transcript());
                s.call("(remote-poll)");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        until(&mut s, "paused: view: car: not a pair: 0");
        assert_eq!(s.eval_to_string("(list-ref *last-error* 2)").unwrap(), "\"view: car: not a pair: 0\"");
        assert_eq!(
            s.eval_to_string("(list-ref *last-error* 3)").unwrap(),
            "((\"(car n)\" \"lantern.scm:4:19\" ()) (\"(view m)\" #f ()))"
        );
        let page = s.eval_to_string("(error-markdown *last-error*)").unwrap();
        assert!(page.contains(&format!("From `the program at {addr}`")), "{page}");
        // Sends still answer, between events.
        s.call(r#"(%editor-set-text 1 "(* 6 7)") (%editor-goto-line 1 0) (eval-defun)"#);
        assert!(s.settle(10.0));
        assert!(transcript().iter().any(|l| l == "=> 42"), "{:?}", transcript());
        events.retain_event(None);
        assert_eq!(events.push_event(&Event::new("resumed", vec![])), 1);
        until(&mut s, "resumed");
        s.call("(remote-stop)");
    }
}

/// One backend at a time (Kevin, 2026-11-23): a Scheme file sends to the
/// editor's image until M-x lux-start (or scm-start, guile-start,
/// remote-start) runs a Scheme in a process of its own; then every Scheme
/// file sends there, the scratch too but not the editor's own files, and
/// the modeline and the REPL say where; starting another stops it;
/// remote-stop goes back.
/// The lux here is a stand-in script that notes its arguments and serves
/// as `scm --serve`, so the test needs scm on PATH, not lux.
#[cfg(test)]
mod backend_tests {
    fn transcript() -> Vec<String> {
        crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect()
    }

    #[test]
    fn one_backend_at_a_time_and_every_scheme_file_sends_to_it() {
        if !std::process::Command::new("scm").arg("--version").output().is_ok_and(|o| o.status.success()) {
            eprintln!("skipped: no scm on PATH");
            return;
        }
        let dir = std::env::temp_dir().join(format!("edit-one-backend-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let (a, b) = (dir.join("a.scm"), dir.join("b.scm"));
        std::fs::write(&a, "(define (frame target time) #f)\n").unwrap();
        std::fs::write(&b, "").unwrap();
        let log = dir.join("log");
        let lux = dir.join("lux");
        std::fs::write(&lux, format!("#!/bin/sh\necho \"$@\" >> {}\nfor x; do addr=$x; done\nexec scm --serve \"$addr\"\n", log.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&lux, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = || -> Vec<String> { std::fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_string).collect() };

        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let open = |s: &mut super::super::standalone::Session, f: &std::path::Path| s.call(&format!("(open-path \"{}\")", f.display()));
        let name = |s: &mut super::super::standalone::Session| s.eval_to_string("(%alist-ref (backend-of (%editor-active-buffer 1)) :name #f)").unwrap();
        let runs = |s: &mut super::super::standalone::Session| s.eval_to_string("(buffer-runs-in (%editor-active-buffer 1))").unwrap();
        let send = |s: &mut super::super::standalone::Session, code: &str| {
            s.call(&format!("(%editor-set-text 1 {code:?}) (%editor-goto-line 1 0) (eval-defun)"));
            assert!(s.settle(10.0));
        };

        // Nothing starts by itself: a Scheme file is the editor's image's.
        open(&mut s, &a);
        assert_eq!(name(&mut s), "\"this image\"");
        send(&mut s, "(define here 1)");
        assert_eq!(s.eval_to_string("here").unwrap(), "1", "it ran here");
        assert!(started().is_empty());
        assert_eq!(runs(&mut s), "\"\"");
        s.call("(repl-focus) (focus-scene-editor)");
        assert_eq!(s.eval_to_string("*repl-prompt-shown*").unwrap(), "\"scheme λ > \"");

        // lux-start runs the file; every Scheme file sends there.
        s.call(&format!("(set! *lux-program* \"{}\") (lux-start)", lux.display()));
        let addr = s.eval_to_string("(%remote-address *remote*)").unwrap();
        let addr = addr.trim_matches('"');
        assert_eq!(started(), [format!("{} --serve {addr}", a.display())]);
        assert_eq!(runs(&mut s), format!("\"lux at {addr}\""), "the modeline says where");
        s.key("right");
        assert_eq!(s.eval_to_string("*repl-prompt-shown*").unwrap(), "\"lux λ > \"", "and the REPL, at the next key");
        send(&mut s, "(* 6 7)");
        assert!(transcript().iter().any(|l| l == "=> 42"), "{:?}", transcript());
        open(&mut s, &b);
        assert!(name(&mut s).starts_with("\"lux at "), "another file sends there too: {}", name(&mut s));
        send(&mut s, "(* 6 8)");
        assert!(transcript().iter().any(|l| l == "=> 48"));
        s.call("(let ((b (%editor-buffer-new 1))) (register-buffer b #f) (visit-buffer b))");
        assert!(name(&mut s).starts_with("\"lux at "), "the scratch too: {}", name(&mut s));
        let shell_file = super::super::shell_dir().join("05-buffers.scm");
        open(&mut s, &shell_file);
        assert_eq!(name(&mut s), "\"this image\"", "the editor's own files are the editor's whatever runs");

        // Starting another stops it: one backend.
        let first = s.eval_to_string("*remote*").unwrap();
        s.call("(scm-start)");
        assert_eq!(s.eval_to_string(&format!("(%remote-address {first})")).unwrap(), "#f", "lux was stopped");
        open(&mut s, &a);
        assert!(name(&mut s).starts_with("\"scm at "), "{}", name(&mut s));
        assert_eq!(s.eval_to_string("(repl-prompt)").unwrap(), "\"scm λ > \"");

        // remote-stop: back to the editor's image.
        s.call("(remote-stop)");
        assert_eq!(name(&mut s), "\"this image\"");
        assert_eq!(runs(&mut s), "\"\"");
        assert_eq!(s.eval_to_string("(repl-prompt)").unwrap(), "\"scheme λ > \"");

        // lux-start with no file: lux with nothing loaded.
        s.call("(let ((b (%editor-buffer-new 1))) (register-buffer b #f) (visit-buffer b)) (lux-start)");
        assert!(started().last().unwrap().starts_with("--serve "), "{:?}", started());
        s.call("(remote-stop)");
        let _ = std::fs::remove_dir_all(&dir);
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

    /// Always: a file that says it is Guile opens as Guile, and nothing
    /// starts by itself (Kevin, 2026-11-23): with no Guile running, its
    /// sends go where a Scheme file's do, the editor's image.
    #[test]
    fn a_guile_file_is_guiles_and_starts_nothing() {
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-guile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hello.scm");
        std::fs::write(&file, "(define-module (hello))\n(define (sq x) (* x x))\n").unwrap();
        let file = file.to_string_lossy().replace('\\', "/");
        s.call(&format!("(open-path \"{file}\")"));
        assert_eq!(s.eval_to_string("(%buffer-language 1 (%editor-active-buffer 1))").unwrap(), "\"guile\"");
        s.call("(%editor-goto-line 1 1) (eval-defun)");
        s.poll();
        assert_eq!(s.eval_to_string("(if (remote-running) 1 0)").unwrap(), "0", "no Guile was started");
        assert_eq!(s.eval_to_string("(sq 3)").unwrap(), "9", "it ran in the editor's image");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With Guile on the PATH: a Guile FILE is read by Guile's own reader
    /// (Kevin, 2026-11-17) — what it cannot read is marked where it
    /// stopped, with its own words; its keywords are not marked; the mark
    /// goes when the text is mended; and with no Guile running the
    /// brackets are still marked, by the editor.
    #[test]
    fn guile_reads_a_guile_buffer_and_marks_what_it_cannot() {
        if !guile_on_path() {
            eprintln!("guile is not on the PATH (nix develop has it): skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-guile-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mod.scm");
        std::fs::write(&file, "(define-module (m))\n(define* (f #:key (n 1)) n)\n(define c #\\notachar)\n(f #:n 2)\n").unwrap();
        s.call(&format!("(open-path \"{}\")", file.to_string_lossy().replace('\\', "/")));
        assert_eq!(s.eval_to_string("(%buffer-language 1 (%editor-active-buffer 1))").unwrap(), "\"guile\"");
        let marks = |s: &mut super::super::standalone::Session| s.eval_to_string("(map (lambda (d) (list (car d) (cadddr d) (caddr d))) (%editor-diagnostics 1))").unwrap();
        // Read until Guile has answered for the text as it stands.
        let read = |s: &mut super::super::standalone::Session| {
            let t0 = std::time::Instant::now();
            loop {
                let got = marks(s);
                s.poll();
                if s.settle(10.0) && s.eval_to_string("(if (or %guile-reading (%read-pending \"guile\")) 1 0)").as_deref() == Ok("0") {
                    return marks(s);
                }
                assert!(t0.elapsed() < std::time::Duration::from_secs(20), "Guile never answered: {got}");
            }
        };
        // No Guile yet: nothing is marked, and nothing was started to ask.
        assert_eq!(marks(&mut s), "()", "its keywords are not errors");
        s.poll();
        assert_eq!(s.eval_to_string("(if (guile-conn) 1 0)").unwrap(), "0", "none was started to ask");
        // guile-start runs it, and a send goes there; from then on Guile
        // reads the buffer.
        s.call("(guile-start) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(30.0), "Guile answers");
        let got = read(&mut s);
        assert!(got.starts_with("((2 1 "), "an error (1) on line 3: {got}");
        assert!(got.to_lowercase().contains("notachar") || got.to_lowercase().contains("character"), "in Guile's own words: {got}");
        // Mended: the mark goes.
        s.call(r##"(%editor-edit 1 "#\\notachar" "#\\a")"##);
        assert_eq!(read(&mut s), "()");
        // A string never closed: only the reader knows; marked quietly (0).
        s.call(r#"(%editor-edit 1 "(f #:n 2)" "(display \"open")"#);
        let got = read(&mut s);
        assert!(got.contains("never closed") && got.contains(" 0 "), "{got}");
        // A bracket is the editor's to mark, Guile or no Guile, and at once.
        s.call(r#"(%editor-edit 1 "(display \"open" "(f #:n 2))")"#);
        assert!(marks(&mut s).contains("closes nothing"), "{}", marks(&mut s));
        s.call("(guile-stop)");
        s.call(r#"(%editor-edit 1 "(f #:n 2))" "(f #:n 2")"#);
        let got = marks(&mut s);
        assert!(got.contains("never closed") && got.starts_with("((3 0 "), "the opener, with no Guile running: {got}");
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
        s.call("(let ((b (%editor-buffer-new 1))) (register-buffer b #f) (visit-buffer b)) (guile-mode) (guile-start)");
        s.call(r#"(%editor-set-text 1 "(define* (sq x #:key (by 1)) \"Squares X.\" (* x x by))\n(sq 7 #:by 2)")"#);
        s.call("(%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(30.0), "Guile answers");
        assert!(transcript().iter().any(|l| l == "=> 98"), "{:?}", transcript());
        assert!(transcript().iter().any(|l| l == "=> sq"), "Guile's define* answers its name: {:?}", transcript());
        assert_eq!(s.eval_to_string("(%env-ref 'sq #f)").unwrap(), "#f", "in Guile, not here");
        let b = "(backend-of (%editor-active-buffer 1))";
        assert_eq!(s.eval_to_string(&format!("((backend-op {b} :doc) \"sq\")")).unwrap(), "\"Squares X.\"");
        assert_eq!(s.eval_to_string(&format!("((backend-op {b} :signature) \"sq\")")).unwrap(), "\"(sq x #:key by)\"");
        // C-c C-d d itself, on the name and on an argument of a call of
        // it: Guile's docstring in the transcript, its signature echoed.
        let echo = || crate::textpane::lines(crate::app::MINIBUFFER).last().map(|(_, l)| l.clone()).unwrap_or_default();
        for (from, to, what) in [(1, 3, "on the name"), (4, 5, "on an argument")] {
            s.call(&format!("(%pane-clear 2) (%editor-select 1 1 {from} {to}) (editor-describe)"));
            let t = transcript();
            assert!(t.iter().any(|l| l.starts_with("── (doc sq)")) && t.iter().any(|l| l == "Squares X."), "{what}: {t:?}");
            assert_eq!(echo(), "(sq x #:key by)", "{what}");
        }
        // TAB in the buffer offers Guile's names: what was sent there, and
        // Guile's own (`define*` is Guile's; scm has none).
        let pool = s.eval_to_string("(editor-candidates)").unwrap();
        assert!(pool.contains("\"sq\"") && pool.contains("\"define*\""), "the buffer's pool is Guile's");
        // The REPL follows the buffer: Guile's prompt, Guile's image — and
        // TAB at it offers what was sent to Guile.
        s.call("(repl)");
        s.text("s");
        s.text("q");
        s.key("tab");
        assert_eq!(s.eval_to_string("(and (%edit :comp) (member \"sq\" (%comp :cands)) #t)").unwrap(), "#t", "sq completes at Guile's REPL");
        s.key("C-g");
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
        // An error comes back with Guile's message.
        s.call(r#"(%editor-set-text 1 "(car 5)") (%editor-goto-line 1 0) (eval-defun)"#);
        assert!(s.settle(10.0));
        assert!(transcript().iter().any(|l| l.starts_with("error: car:")), "{:?}", transcript());
        s.call("(guile-stop)");
        assert_eq!(s.eval_to_string("(if (remote-running) 1 0)").unwrap(), "0", "guile-stop stopped it");
    }
}

/// L1 of docs/sbcl-plan.md: SBCL through the same protocol — an SBCL
/// running edit/sbcl/serve.lisp, driven here as a client drives it.
#[cfg(test)]
mod sbcl_tests {
    use super::*;

    fn sbcl_on_path() -> bool {
        std::process::Command::new("sbcl").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    /// An SBCL serving on a free port, connected: the connection's id. No
    /// init file, so the test is the same on every machine.
    fn start_sbcl() -> u32 {
        let serve = concat!(env!("CARGO_MANIFEST_DIR"), "/sbcl/serve.lisp");
        let args: Vec<String> = ["--noinform", "--disable-debugger", "--no-userinit", "--load", serve, "--end-toplevel-options", "{addr}"]
            .iter()
            .map(|a| a.to_string())
            .collect();
        start("sbcl", &args, 0, None).expect("sbcl serves")
    }

    fn ask_ok(id: u32, op: Op) -> Outcome {
        ask(id, op, Duration::from_secs(20)).expect("an answer").outcome
    }

    fn eval(id: u32, code: &str) -> Outcome {
        ask_ok(id, Op::Eval { code: code.into(), source: None })
    }

    fn value(id: u32, code: &str) -> (String, String) {
        match eval(id, code) {
            Outcome::Value { value, output } => (value, output),
            other => panic!("{code}: {other:?}"),
        }
    }

    fn failure(id: u32, code: &str) -> (String, String, bool, Vec<String>) {
        match eval(id, code) {
            Outcome::Failed { message, output, host_stopped, backtrace, .. } => (message, output, host_stopped, backtrace),
            other => panic!("{code}: {other:?}"),
        }
    }

    /// The exit criterion of L1: a value, output kept with a failure, a
    /// multi-form send, completion of a name just defined, describe with
    /// docstring and lambda list — and what would open SBCL's debugger, or
    /// wait for a terminal, answers instead.
    #[test]
    fn sbcl_answers_the_protocol() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        let c = start_sbcl();
        assert!(matches!(ask_ok(c, Op::Ping), Outcome::Pong { version } if version.starts_with("sbcl ")));
        assert_eq!(value(c, "(+ 1 2)").0, "3");
        // Every form, in order; the last one's value; what they printed.
        let (v, out) = value(c, "(defun sq (x) \"Squares X.\" (* x x))\n(print :hi)\n(sq 7)");
        assert_eq!(v, "49");
        assert!(out.contains(":HI"), "{out:?}");
        assert_eq!(value(c, "(values 1 \"two\")").0, "1, \"two\"");
        assert_eq!(value(c, "(values)").0, "; No values");
        // An error: SBCL's message, what was printed before it, the calls.
        let (msg, out, stopped, frames) = failure(c, "(defun inner (x) (car x)) (defun outer (x) (1+ (inner (1+ x)))) (format t \"before~%\") (outer 4)");
        assert!(msg.contains("is not of type") && msg.contains("LIST"), "{msg}");
        assert_eq!(out, "before\n");
        assert!(!stopped, "the script failed; the host did not stop it");
        assert_eq!(frames.first().map(String::as_str), Some("(INNER 5)"), "the calls start where it broke: {frames:?}");
        assert!(frames.iter().any(|f| f.starts_with("(OUTER 4)")), "{frames:?}");
        assert!(!frames.iter().any(|f| f.contains("EDIT-SERVE")), "none of the server's own: {frames:?}");
        // The compiler's warnings come back with a send that compiled.
        let (v, out) = value(c, "(defun bad (x) (undefined-fn-zz x) (car 1 2))");
        assert_eq!(v, "BAD");
        assert!(out.contains("; warning: The function CAR is called with two arguments"), "{out:?}");
        assert!(out.contains("; style-warning: undefined function: COMMON-LISP-USER::UNDEFINED-FN-ZZ"), "{out:?}");
        // What would open the debugger, or read from a terminal, answers.
        assert_eq!(failure(c, "(break \"look here\")").0, "look here");
        assert!(failure(c, "(read-line)").0.starts_with("end of file"));
        let (msg, _, _, frames) = failure(c, "(sq 3");
        assert_eq!(msg, "end of file: a form is not closed");
        assert!(frames.is_empty(), "the text was wrong, not a call: {frames:?}");
        // The image's names: one just defined, and SBCL's own.
        let cands = |p: &str| match ask_ok(c, Op::Complete { prefix: p.into() }) {
            Outcome::Candidates(names) => names,
            other => panic!("{other:?}"),
        };
        assert_eq!(cands("sq"), ["sq", "sqrt"]);
        assert!(cands("sb-ext:*invoke").contains(&"sb-ext:*invoke-debugger-hook*".to_string()));
        let describe = |n: &str| match ask_ok(c, Op::Describe { name: n.into() }) {
            Outcome::Described(d) => d,
            other => panic!("{other:?}"),
        };
        let d = describe("sq").expect("sq is bound");
        assert_eq!((d.kind.as_str(), d.signature.as_deref(), d.doc.as_deref()), ("procedure", Some("(sq x)"), Some("Squares X.")));
        assert_eq!(describe("mapcar").unwrap().signature.as_deref(), Some("(mapcar function list &rest more-lists)"));
        assert_eq!(describe("*print-base*").unwrap().kind, "variable");
        assert!(describe("nothing-here-zz").is_none());
        // Is the text whole? A name in a package not made yet is.
        let framing = |code: &str| match ask_ok(c, Op::Parse { code: code.into() }) {
            Outcome::Framed(f) => f,
            other => panic!("{other:?}"),
        };
        assert_eq!(framing("(a (b"), scm::Framing::Incomplete);
        assert_eq!(framing("(nopkg-zz:bar #.(error \"no\"))"), scm::Framing::Complete);
        assert_eq!(framing(")"), scm::Framing::Invalid);
        // A REPL's (in-package …) lasts for its connection.
        assert_eq!(value(c, "(in-package :sb-ext) *package*").0, "#<PACKAGE \"SB-EXT\">");
        assert_eq!(value(c, "*package*").0, "#<PACKAGE \"SB-EXT\">");
        close(c);
    }

    fn transcript() -> Vec<String> {
        crate::textpane::lines(crate::app::TRANSCRIPT).into_iter().map(|(_, l)| l).collect()
    }

    /// Always: a .lisp file opens as Lisp, and its send never falls through
    /// to scm's image when SBCL cannot start.
    #[test]
    fn a_lisp_file_is_lisps_and_never_runs_as_scm() {
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-lisp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("hello.lisp");
        std::fs::write(&file, "(defun square-it (x) (* x x))\n").unwrap();
        s.call(&format!("(open-path \"{}\")", file.to_string_lossy().replace('\\', "/")));
        assert_eq!(s.eval_to_string("(%buffer-language 1 (%editor-active-buffer 1))").unwrap(), "\"lisp\"");
        s.call("(set! *sbcl-program* \"no-such-sbcl-here\") (%editor-goto-line 1 0) (eval-defun)");
        s.poll();
        assert!(transcript().iter().any(|l| l.starts_with("error: sbcl:")), "{:?}", transcript());
        // With no SBCL running, a Lisp buffer completes from its own words,
        // not scm's names.
        let pool = s.eval_to_string("(editor-candidates)").unwrap();
        assert!(pool.contains("\"square-it\"") && !pool.contains("\"string-map\""), "{pool}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L3's exit criterion, against a real SBCL: each send key; a defun
    /// answers its name; an error its message; describe on a name and on an
    /// argument; TAB in the buffer and at the REPL offering what was sent;
    /// and a file of two packages sending each form to its own.
    #[test]
    fn a_lisp_buffer_sends_to_sbcl() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-lisp-send-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shapes.lisp");
        std::fs::write(
            &file,
            "(defpackage :shapes (:use :cl))\n(in-package :shapes)\n(defun sq (x) \"Squares X.\" (* x x))\n(sq 7)\n(in-package :cl-user)\n(defun sq (x) (list :user x))\n(sq 7)\n(car 5)\n",
        )
        .unwrap();
        s.call(&format!("(open-path \"{}\")", file.to_string_lossy().replace('\\', "/")));
        // The same on every machine: no ~/.sbclrc.
        s.call("(set! *sbcl-init* #f)");
        let sent = |s: &mut super::super::standalone::Session, call: &str, want: &str| {
            s.call("(%pane-clear 2)");
            s.call(call);
            assert!(s.settle(30.0), "SBCL answers {call}");
            assert!(transcript().iter().any(|l| l.starts_with(want)), "{call}: want {want:?} in {:?}", transcript());
        };
        // C-c C-b: the whole file, from CL-USER, its in-packages taking effect.
        sent(&mut s, "(eval-buffer)", "error: The value");
        // C-M-x, each form in the package above it.
        sent(&mut s, "(%editor-goto-line 1 2) (eval-defun)", "=> SQ");
        sent(&mut s, "(%editor-goto-line 1 3) (eval-defun)", "=> 49");
        sent(&mut s, "(%editor-goto-line 1 6) (eval-defun)", "=> (:USER 7)");
        // C-x C-e: the expression before the point.
        sent(&mut s, "(%editor-select 1 3 0 6) (eval-last-sexp)", "=> 49");
        // An error: SBCL's message.
        s.call("(%pane-clear 2) (%editor-goto-line 1 7) (eval-defun)");
        assert!(s.settle(30.0));
        assert!(transcript().iter().any(|l| l == "error: The value 5 is not of type LIST when binding LIST"), "{:?}", transcript());
        // Describe, in the buffer's package: on the name and on an argument.
        let echo = || crate::textpane::lines(crate::app::MINIBUFFER).last().map(|(_, l)| l.clone()).unwrap_or_default();
        for (from, to, what) in [(1, 3, "on the name"), (4, 5, "on an argument")] {
            s.call(&format!("(%pane-clear 2) (%editor-select 1 3 {from} {to}) (editor-describe)"));
            let t = transcript();
            assert!(t.iter().any(|l| l == "Squares X."), "{what}: {t:?}");
            assert_eq!(echo(), "(sq x)", "{what}");
        }
        // TAB in the buffer offers SBCL's names, what was sent included.
        let pool = s.eval_to_string("(editor-candidates)").unwrap();
        assert!(pool.contains("\"sq\"") && pool.contains("\"mapcar\""), "the buffer's pool is SBCL's");
        // The REPL: SBCL's prompt in the REPL's own package, which an
        // in-package typed there changes — two breaks down, after the two
        // errors (L7), until :top leaves them.
        s.call("(repl)");
        assert_eq!(s.eval_to_string("(repl-prompt)").unwrap(), "\"CL-USER[2]> \"");
        let line = |s: &mut super::super::standalone::Session, text: &str| {
            for c in text.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
            assert!(s.settle(30.0));
        };
        line(&mut s, ":top");
        assert_eq!(s.eval_to_string("(repl-prompt)").unwrap(), "\"CL-USER> \"");
        line(&mut s, "(in-package :shapes)");
        let t0 = std::time::Instant::now();
        while s.eval_to_string("(repl-prompt)").unwrap() != "\"SHAPES> \"" {
            s.poll();
            assert!(t0.elapsed() < std::time::Duration::from_secs(10), "the prompt follows the package");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        line(&mut s, "(sq 2)");
        assert!(transcript().iter().any(|l| l == "SHAPES> (sq 2)"), "{:?}", transcript());
        assert!(transcript().iter().any(|l| l == "=> 4"), "{:?}", transcript());
        // A line left open waits for the rest.
        line(&mut s, "(+ 1");
        line(&mut s, "2)");
        assert!(transcript().iter().any(|l| l == "=> 3"), "{:?}", transcript());
        s.call("(sbcl-stop)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L7's exit criterion, through the editor: an error is a break, served
    /// at the REPL panel. USE-VALUE finishes the original send with its
    /// value; CONTINUE after a cerror; (break); an error inside a break and
    /// out of both; a call's locals in the error pane; and an interrupt of a
    /// runaway lands in a break, where it can be looked at and resumed.
    #[test]
    fn a_lisp_error_is_a_break() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-lisp-break-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("break.lisp");
        std::fs::write(
            &file,
            "(defun inner (x) (declare (optimize (debug 2))) (let ((y (* 2 x))) (error \"too big: ~a\" y)))\n(inner 4)\n(+ 1 nope)\n",
        )
        .unwrap();
        s.call(&format!("(open-path \"{}\")", file.to_string_lossy().replace('\\', "/")));
        s.call("(set! *sbcl-init* #f)");
        let prompt = |s: &mut super::super::standalone::Session| s.eval_to_string("(repl-prompt)").unwrap();
        let line = |s: &mut super::super::standalone::Session, text: &str| {
            s.call("(%pane-clear 2)");
            for c in text.chars() {
                s.text(&c.to_string());
            }
            s.key("return");
            assert!(s.settle(30.0), "SBCL answers {text}");
        };
        let has = |want: &str| transcript().iter().any(|l| l == want);
        // A call's locals reach the error pane.
        s.call("(%editor-goto-line 1 0) (eval-defun) (%editor-goto-line 1 1) (eval-defun)");
        assert!(s.settle(30.0));
        assert!(has("error: too big: 8"), "{:?}", transcript());
        assert!(has("  0: [ABORT] Give this send up."), "the restarts, numbered: {:?}", transcript());
        let page = s.eval_to_string("(error-markdown *last-error*)").unwrap();
        assert!(page.contains("(INNER 4)") && page.contains("`x` = `4`"), "{page}");
        s.call("(repl)");
        assert_eq!(prompt(&mut s), "\"CL-USER[1]> \"");
        // Inside the error, an expression evaluates there; :abort leaves.
        line(&mut s, "(+ 2 2)");
        assert!(has("=> 4"), "{:?}", transcript());
        line(&mut s, ":abort");
        assert_eq!(prompt(&mut s), "\"CL-USER> \"");
        // USE-VALUE: the restart asks for its value, and the send it broke
        // finishes with it — answering the value's own line.
        s.call("(focus-scene-editor) (%editor-goto-line 1 2) (eval-defun)");
        assert!(s.settle(30.0));
        assert!(has("error: The variable NOPE is unbound."), "{:?}", transcript());
        assert!(has("  1: [USE-VALUE] Use specified value."), "{:?}", transcript());
        s.call("(repl)");
        line(&mut s, "1");
        assert_eq!(prompt(&mut s), "\"USE-VALUE value> \"");
        line(&mut s, "(* 6 7)");
        assert!(has("=> 43"), "the original send's value: {:?}", transcript());
        assert_eq!(prompt(&mut s), "\"CL-USER> \"");
        // CONTINUE after a cerror; out of a (break).
        line(&mut s, "(progn (cerror \"go on\" \"bad\") :went-on)");
        assert!(has("  0: [CONTINUE] go on"), "{:?}", transcript());
        line(&mut s, "0");
        assert!(has("=> :WENT-ON"), "{:?}", transcript());
        line(&mut s, "(progn (break \"look\") 7)");
        assert!(has("error: look"), "{:?}", transcript());
        line(&mut s, "0");
        assert!(has("=> 7"), "{:?}", transcript());
        // An error inside a break is a second break; out of both.
        line(&mut s, "(car 5)");
        line(&mut s, "(car 6)");
        assert_eq!(prompt(&mut s), "\"CL-USER[2]> \"");
        line(&mut s, ":abort");
        assert_eq!(prompt(&mut s), "\"CL-USER[1]> \"");
        line(&mut s, ":abort");
        assert_eq!(prompt(&mut s), "\"CL-USER> \"");
        // An interrupted runaway is a break, not lost work: look, go on,
        // and give it up.
        line(&mut s, "(defvar *n* 0)");
        s.call("(%pane-clear 2)");
        for c in "(loop (incf *n*))".chars() {
            s.text(&c.to_string());
        }
        s.key("return");
        std::thread::sleep(Duration::from_millis(300));
        s.call("(remote-interrupt)");
        assert!(s.settle(30.0));
        assert!(has("error: interrupted  (stopped)"), "{:?}", transcript());
        assert!(has("  0: [CONTINUE] Go on with the evaluation."), "{:?}", transcript());
        line(&mut s, "(plusp *n*)");
        assert!(has("=> T"), "the work so far is there: {:?}", transcript());
        s.call("(%pane-clear 2)");
        s.text("0");
        s.key("return");
        std::thread::sleep(Duration::from_millis(300));
        s.call("(remote-interrupt)");
        assert!(s.settle(30.0));
        assert!(has("error: interrupted  (stopped)"), "it went on, and stopped again: {:?}", transcript());
        line(&mut s, ":top");
        assert_eq!(prompt(&mut s), "\"CL-USER> \"");
        s.call("(sbcl-stop)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L4's exit criterion: a Lisp FILE is read by SBCL's reader — what it
    /// cannot read is marked on its line in SBCL's words (`#\notachar`, a
    /// package that does not exist); `#.(…)` is not run, nor marked; mending
    /// clears the mark; and reading leaves no symbol behind in the user's
    /// package, `shapes::brand-new` included.
    #[test]
    fn sbcl_reads_a_lisp_buffer_and_marks_what_it_cannot() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-lisp-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shapes.lisp");
        std::fs::write(
            &file,
            "(defpackage :shapes (:use :cl))\n(in-package :shapes)\n(defun f () (list #\\notachar))\n(defun g () (shapes::brand-new #.(error \"no\")))\n",
        )
        .unwrap();
        s.call(&format!("(open-path \"{}\")", file.to_string_lossy().replace('\\', "/")));
        s.call("(set! *sbcl-init* #f)");
        let marks = |s: &mut super::super::standalone::Session| s.eval_to_string("(map (lambda (d) (list (car d) (cadddr d) (caddr d))) (%editor-diagnostics 1))").unwrap();
        let read = |s: &mut super::super::standalone::Session| {
            let t0 = std::time::Instant::now();
            loop {
                let got = marks(s);
                s.poll();
                if s.settle(10.0) && s.eval_to_string("(if (or %sbcl-reading (%read-pending \"lisp\")) 1 0)").as_deref() == Ok("0") {
                    return marks(s);
                }
                assert!(t0.elapsed() < std::time::Duration::from_secs(20), "SBCL never answered: {got}");
            }
        };
        // No SBCL yet: the brackets are fine, so nothing is marked, and
        // nothing was started to ask.
        assert_eq!(marks(&mut s), "()");
        s.poll();
        assert_eq!(s.eval_to_string("(if (sbcl-running?) 1 0)").unwrap(), "0", "none was started to ask");
        // The brackets are the editor's to mark, in a .lisp file too (L2),
        // SBCL or no SBCL.
        s.call(r#"(%editor-edit 1 "(in-package :shapes)" "(in-package :shapes))")"#);
        assert!(marks(&mut s).contains("closes nothing"), "{}", marks(&mut s));
        s.call(r#"(%editor-edit 1 "(in-package :shapes))" "(in-package :shapes)")"#);
        assert_eq!(marks(&mut s), "()");
        // A send starts it (the defpackage); from then on SBCL reads.
        s.call("(%editor-goto-line 1 0) (eval-defun)");
        assert!(s.settle(30.0), "SBCL answers");
        let got = read(&mut s);
        assert!(got.starts_with("((2 1 ") && got.contains("unrecognized character name: \\\"notachar\\\""), "line 3, in SBCL's words: {got}");
        // Mended: the mark goes — and #.(error …) was neither run nor marked.
        s.call(r##"(%editor-edit 1 "#\\notachar" "#\\a")"##);
        assert_eq!(read(&mut s), "()");
        let left = s.eval_to_string(r#"(lisp-ask *sbcl* "(list (find-symbol \"BRAND-NEW\" :shapes) (find-symbol \"F\" :shapes))")"#).unwrap();
        assert_eq!(left, "(NIL NIL)", "reading interned nothing in SHAPES");
        // A package that does not exist.
        s.call(r#"(%editor-edit 1 "(list #\\a)" "(list (foo:bar))")"#);
        let got = read(&mut s);
        assert!(got.starts_with("((2 1 ") && got.contains("Package FOO does not exist."), "{got}");
        // A string never closed: only the reader knows; marked quietly (0).
        s.call(r#"(%editor-edit 1 "(list (foo:bar))" "(list \"open))")"#);
        let got = read(&mut s);
        assert!(got.contains("never closed") && got.contains(" 0 "), "{got}");
        s.call("(sbcl-stop)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L5's exit criterion: M-. on a call jumps to its `defun` in another
    /// file — one SBCL never loaded, from the project's own files, with no
    /// SBCL running; one SBCL loaded from outside the project, from SBCL —
    /// and M-, comes back.
    #[test]
    fn m_dot_finds_a_lisp_definition_loaded_or_not() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let top = std::env::temp_dir().join(format!("edit-lisp-xref-{}", std::process::id()));
        let (proj, lib) = (top.join("proj"), top.join("lib"));
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(proj.join("geo.asd"), "(asdf:defsystem :geo :components ((:file \"src/geo\")))\n").unwrap();
        std::fs::write(proj.join("src/geo.lisp"), "(defpackage :geo (:use :cl))\n(in-package :geo)\n\n;; The area of a circle.\n(defun area (r)\n  (* pi r r))\n").unwrap();
        std::fs::write(proj.join("main.lisp"), "(in-package :geo)\n(print (area 2))\n(print (far-fn 1))\n").unwrap();
        std::fs::write(lib.join("far.lisp"), "(in-package :cl-user)\n\n(defun far-fn (x)\n  (1+ x))\n").unwrap();
        let path = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
        s.call(&format!("(open-path \"{}\")", path(&proj.join("main.lisp"))));
        s.call("(set! *sbcl-init* #f)");
        let at = |s: &mut super::super::standalone::Session| s.eval_to_string("(list (%text (buffer-basename (buffer-path (%editor-active-buffer 1)))) (car (%editor-cursor 1)))").unwrap();
        let on = |s: &mut super::super::standalone::Session, line: usize, col: usize| {
            s.call(&format!("(%editor-goto-line 1 {line})"));
            for _ in 0..col {
                s.call("(%editor-act 1 \"right\")");
            }
        };
        // Not loaded, no SBCL: the project's own files.
        on(&mut s, 1, 9);
        s.call("(find-definition-at-point)");
        assert_eq!(at(&mut s), "(\"geo.lisp\" 4)", "the defun in src/geo.lisp");
        assert_eq!(s.eval_to_string("(if (sbcl-running?) 1 0)").unwrap(), "0", "none was started to ask");
        s.call("(go-back-from-definition)");
        assert_eq!(at(&mut s), "(\"main.lisp\" 1)", "M-, comes back");
        // Loaded from outside the project: SBCL says where.
        s.call(&format!(
            "(sbcl-start) (lisp-ask *sbcl* \"(load (compile-file \\\"{}\\\" :output-file \\\"{}\\\"))\")",
            path(&lib.join("far.lisp")),
            path(&top.join("far.fasl"))
        ));
        on(&mut s, 2, 9);
        s.call("(find-definition-at-point)");
        assert_eq!(at(&mut s), "(\"far.lisp\" 2)", "the defun SBCL loaded");
        s.call("(go-back-from-definition)");
        assert_eq!(at(&mut s), "(\"main.lisp\" 2)");
        // A name defined nowhere says so.
        s.call("(jump-to-definition \"no-such-fn\")");
        let echo = crate::textpane::lines(crate::app::MINIBUFFER).last().map(|(_, l)| l.clone()).unwrap_or_default();
        assert_eq!(echo, "no definition of no-such-fn found");
        s.call("(sbcl-stop)");
        let _ = std::fs::remove_dir_all(&top);
    }

    /// L6's exit criterion: what SBCL's compiler says of a sent form — a
    /// function that does not exist, a wrong count of arguments, an unused
    /// variable — is shown under the send and marked at the form, the mark
    /// following it as the text moves, until it is sent again clean.
    #[test]
    fn sbcl_warnings_mark_the_form_until_it_is_sent_clean() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        crate::app::resize(800, 600, 1.0);
        let mut s = super::super::standalone::Session::boot(None, &[]).expect("boot");
        let dir = std::env::temp_dir().join(format!("edit-lisp-warn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("warn.lisp");
        std::fs::write(&file, "(defun ok () 1)\n\n(defun bad (x)\n  (nope-zz x))\n(defun worse (y) (car 1 2))\n").unwrap();
        s.call(&format!("(open-path \"{}\")", file.to_string_lossy().replace('\\', "/")));
        s.call("(set! *sbcl-init* #f) (set! *sbcl-read-check* #f)");
        // (line message) of each mark.
        let marks = |s: &mut super::super::standalone::Session| s.eval_to_string("(map (lambda (d) (list (car d) (caddr d))) (%editor-diagnostics 1))").unwrap();
        assert_eq!(marks(&mut s), "()");
        // The whole buffer: two forms warned of, each marked at its head.
        s.call("(eval-buffer)");
        assert!(s.settle(30.0));
        assert!(transcript().iter().any(|l| l == "; style-warning: undefined function: COMMON-LISP-USER::NOPE-ZZ"), "under the send: {:?}", transcript());
        let got = marks(&mut s);
        assert!(got.starts_with("((2 \"style-warning: undefined function: COMMON-LISP-USER::NOPE-ZZ\") (4 \"warning: The function CAR is called with two arguments"), "{got}");
        assert!(got.contains("· style-warning: The variable Y is defined but never used."), "both said of worse, on one mark: {got}");
        // The marks follow their forms as the text moves.
        s.call(r#"(%editor-edit 1 "(defun ok () 1)" ";; header\n(defun ok () 1)")"#);
        let got = marks(&mut s);
        assert!(got.starts_with("((3 ") && got.contains("(5 "), "{got}");
        // Mended but not sent: still marked. Sent clean: the mark goes, the
        // other stays.
        s.call(r#"(%editor-edit 1 "(nope-zz x)" "(1+ x)")"#);
        assert!(marks(&mut s).starts_with("((3 "), "{}", marks(&mut s));
        s.call("(%editor-goto-line 1 3) (eval-defun)");
        assert!(s.settle(30.0));
        let got = marks(&mut s);
        assert!(got.starts_with("((5 \"warning: The function CAR") && !got.contains("NOPE-ZZ"), "{got}");
        s.call("(sbcl-stop)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An interrupt from a second connection stops a runaway where it
    /// runs, says the host stopped it, and leaves the server up.
    #[test]
    fn an_interrupt_stops_sbcl_and_it_stays_up() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        let c = start_sbcl();
        // Past the timeout, ask interrupts down a second connection.
        let answer = ask(c, Op::Eval { code: "(loop)".into(), source: None }, Duration::from_millis(500)).expect("an answer");
        match answer.outcome {
            Outcome::Failed { message, host_stopped, backtrace, .. } => {
                assert_eq!(message, "interrupted");
                assert!(host_stopped);
                assert_eq!(backtrace, ["(LOOP)"], "where it was");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(value(c, "(+ 1 1)").0, "2", "the server is still up");
        close(c);
    }

    /// A send runs on its connection's thread; `with-main-thread` hands a
    /// body to the process's main thread — the one macOS lets open a
    /// window — and the send waits for it as for any call.
    #[test]
    fn a_send_reaches_the_main_thread() {
        if !sbcl_on_path() {
            eprintln!("sbcl is not on the PATH: skipped");
            return;
        }
        let c = start_sbcl();
        let on_main = "(eq sb-thread:*current-thread* (sb-thread:main-thread))";
        assert_eq!(value(c, on_main).0, "NIL", "a send is not on the main thread");
        assert_eq!(value(c, &format!("(edit-serve:with-main-thread () {on_main})")).0, "T");
        // Its values, its output with the send's, the send's package.
        let (v, out) = value(c, "(defpackage :mt (:use :cl)) (in-package :mt) (edit-serve:with-main-thread () (print :there) (values 1 (symbol-name (intern \"X\"))))");
        assert_eq!(v, "1, \"X\"");
        assert!(out.contains(":THERE"), "{out:?}");
        assert_eq!(value(c, "(symbol-package (find-symbol \"X\")) ").0, "#<PACKAGE \"MT\">");
        // Its error is the send's, a break like any other.
        let (msg, ..) = failure(c, "(edit-serve:with-main-thread () (error \"on main\"))");
        assert_eq!(msg, "on main");
        assert_eq!(value(c, ":abort").0, "; Evaluation aborted");
        // Not waited for: the send returns, the main thread goes on.
        value(c, "(defvar *ran* nil) (edit-serve:with-main-thread (:wait nil) (sleep 0.2) (setf *ran* t))");
        assert_eq!(value(c, "*ran*").0, "NIL");
        assert_eq!(value(c, "(sleep 0.5) *ran*").0, "T");
        // A runaway on the main thread: interrupted, then given up, gives
        // the job up too — the main thread takes the next one.
        let answer = ask(c, Op::Eval { code: "(edit-serve:with-main-thread () (loop))".into(), source: None }, Duration::from_millis(500)).expect("an answer");
        assert!(matches!(answer.outcome, Outcome::Failed { ref message, .. } if message == "interrupted"), "{:?}", answer.outcome);
        assert_eq!(value(c, ":abort").0, "; Evaluation aborted");
        assert_eq!(value(c, &format!("(edit-serve:with-main-thread () {on_main})")).0, "T", "the main thread is free again");
        close(c);
    }
}
