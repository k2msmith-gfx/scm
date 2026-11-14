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
//! **A send waits**, as an in-process send does — the editor's sends are
//! synchronous — but not forever: past the timeout the client sends
//! `:interrupt` on a SECOND connection (the server is single-threaded, so an
//! interrupt queued behind the evaluation would never arrive; `scm-mode.el`
//! keeps the same pair), and the answer comes back `host-stopped`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use scm::{Op, Outcome, Request, Response, Val};

use super::{val_num, val_text};

struct Conn {
    addr: String,
    stream: TcpStream,
    pending: String,
    next_id: u64,
    /// The server, when this editor started it.
    child: Option<std::process::Child>,
}

thread_local! {
    static CONNS: RefCell<HashMap<u32, Conn>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

fn connect(addr: &str, child: Option<std::process::Child>) -> Result<u32, String> {
    let stream = TcpStream::connect(addr).map_err(|e| format!("{addr}: {e}"))?;
    let _ = stream.set_nodelay(true);
    let id = NEXT.with(|n| n.replace(n.get() + 1));
    CONNS.with(|c| {
        c.borrow_mut()
            .insert(id, Conn { addr: addr.to_string(), stream, pending: String::new(), next_id: 1, child })
    });
    Ok(id)
}

/// Start `program --serve 127.0.0.1:PORT` (a free port when `port` is 0)
/// and connect to it, retrying while it boots.
fn start(program: &str, port: u16) -> Result<u32, String> {
    let port = if port == 0 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port()
    } else {
        port
    };
    let addr = format!("127.0.0.1:{port}");
    let mut child = std::process::Command::new(program)
        .args(["--serve", &addr])
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
            return Err(format!("{program} --serve {addr} exited: {status}"));
        }
        if t0.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            return Err(format!("{program} --serve {addr} did not listen within 10 s"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// One request, one response — no pipelining, so the first complete datum
/// that is not a streamed chunk is the answer. Past `timeout` the request is
/// interrupted from a second connection, and the wait goes on for its
/// (host-stopped) answer.
fn ask(id: u32, op: Op, timeout: Duration) -> Result<Response, String> {
    CONNS.with(|c| {
        let mut conns = c.borrow_mut();
        let conn = conns.get_mut(&id).ok_or("no such connection")?;
        let rid = conn.next_id;
        conn.next_id += 1;
        let text = format!("{}\n", Request { id: rid, op }.to_wire());
        conn.stream
            .write_all(text.as_bytes())
            .and_then(|()| conn.stream.flush())
            .map_err(|e| format!("{}: {e}", conn.addr))?;
        let t0 = Instant::now();
        let mut interrupted = false;
        let mut chunk = [0u8; 8192];
        loop {
            if !conn.pending.trim().is_empty() {
                match scm::parse_status(&conn.pending) {
                    scm::ParseStatus::Complete => {
                        let text = std::mem::take(&mut conn.pending);
                        let answer = Response::parse(&text).map_err(|e| format!("unreadable response: {e}"))?;
                        if matches!(answer.outcome, Outcome::Streamed(_)) || answer.id != rid {
                            continue;
                        }
                        return Ok(answer);
                    }
                    scm::ParseStatus::Invalid(_) => {
                        let text = std::mem::take(&mut conn.pending);
                        return Err(format!("not a response: {}", text.trim()));
                    }
                    scm::ParseStatus::Incomplete => {}
                }
            }
            let _ = conn.stream.set_read_timeout(Some(Duration::from_millis(200)));
            match conn.stream.read(&mut chunk) {
                Ok(0) => return Err(format!("{} closed the connection", conn.addr)),
                Ok(n) => conn.pending.push_str(&String::from_utf8_lossy(&chunk[..n])),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if !interrupted && t0.elapsed() > timeout {
                        interrupted = true;
                        interrupt(&conn.addr);
                    } else if interrupted && t0.elapsed() > timeout + Duration::from_secs(10) {
                        return Err(format!("{} did not answer, even interrupted", conn.addr));
                    }
                }
                Err(e) => return Err(format!("{}: {e}", conn.addr)),
            }
        }
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

fn entry(k: &str, v: Val) -> Val {
    Val::Pair(Box::new(Val::Keyword(k.into())), Box::new(v))
}

fn conn_id(args: &[Val]) -> u32 {
    val_num(args, 0) as u32
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    // `(%remote-start program port)` — run PROGRAM --serve on PORT (0: a
    // free one) and connect: the connection's id. Raises when it cannot.
    interp.define_fn("%remote-start", |args| {
        let program = val_text(args, 0).unwrap_or_else(|| "scm".into());
        Ok(Val::Int(start(&program, val_num(args, 1).max(0.0) as u16)? as i64))
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
        Ok(match answer.outcome {
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
            other => return Err(format!("unexpected answer to eval: {other:?}")),
        })
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
        assert_eq!(s.eval_to_string("(list-ref *last-error* 2)").unwrap(), "\"vector-ref: index 9 out of range\"");
        assert!(s.eval_to_string("(list-ref *last-error* 3)").unwrap().contains("(vector-ref (vector 1 2) 9)"));
        s.call("(remote-stop)");
    }
}
