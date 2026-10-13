//! The `scm` binary: two clients of one REPL.
//!
//! Plan §8 is emphatic that the prompt is **one client among several**, and
//! that "building the loop first and bolting a socket on later is how the
//! Janet side ended up with an eval channel per consumer". So neither client
//! here contains a REPL. Both turn bytes into a [`scm::Request`], hand it to
//! [`scm::Session::handle`], and turn the [`scm::Response`] back into bytes —
//! and `--serve` exists in this first version precisely so the terminal path
//! never becomes the only one that works.
//!
//!     scm                    # the terminal client
//!     scm --serve 127.0.0.1:7000
//!     scm FILE | scm -e EXPR
//!
//! Multi-line entry asks the server `:op parse` rather than calling the
//! reader itself — the same answer over the same door, so an editor pane's
//! `RET` and this prompt's continuation can never disagree.

use std::io::{self, BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use scm::{Framing, Op, Outcome, Request, Response, Session};

/// The running session's interrupt flag, so a `SIGINT` handler can reach it.
static INTERRUPT: OnceLock<std::sync::Arc<AtomicBool>> = OnceLock::new();

// `signal(2)`, declared rather than depending on `libc` — this crate has no
// dependencies and one extern is cheaper than acquiring one.
unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}

const SIGINT: i32 = 2;

/// Async-signal-safe: a relaxed store to an `AtomicBool` and nothing else.
/// The VM notices at its next safepoint (requirement 8).
extern "C" fn on_sigint(_: i32) {
    if let Some(flag) = INTERRUPT.get() {
        flag.store(true, Ordering::Relaxed);
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut fuel: Option<u64> = None;
    let mut serve: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--fuel" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse::<u64>().ok()) {
                    Some(n) => fuel = Some(n),
                    None => {
                        eprintln!("scm: --fuel wants a number");
                        return ExitCode::from(2);
                    }
                }
            }
            "--serve" => {
                i += 1;
                match args.get(i) {
                    Some(a) => serve = Some(a.clone()),
                    None => {
                        eprintln!("scm: --serve wants an address, e.g. 127.0.0.1:7000");
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            "--version" => {
                println!("scm {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }

    let mut session = match Session::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("scm: could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    session.interp_mut().set_fuel(fuel);
    let _ = INTERRUPT.set(session.interrupt_handle());
    unsafe { signal(SIGINT, on_sigint as *const () as usize) };

    if let Some(addr) = serve {
        return serve_forever(session, &addr);
    }
    match rest.first().map(String::as_str) {
        Some("-e") => match rest.get(1) {
            Some(src) => run_once(&mut session, src),
            None => {
                eprintln!("scm: -e wants an expression");
                ExitCode::from(2)
            }
        },
        Some(path) => match std::fs::read_to_string(path) {
            Ok(src) => run_once(&mut session, &src),
            Err(e) => {
                eprintln!("scm: {path}: {e}");
                ExitCode::from(2)
            }
        },
        None => repl(&mut session),
    }
}

fn usage() {
    println!("usage: scm [--fuel N] [--serve ADDR] [FILE | -e EXPR]");
    println!();
    println!("  no arguments   start the terminal REPL");
    println!("  FILE           run a file");
    println!("  -e EXPR        evaluate one expression and print it");
    println!("  --serve ADDR   serve the REPL protocol over TCP (see docs/repl.md)");
    println!("  --fuel N       stop after N safepoints; a runaway is an error,");
    println!("                 not a hang (plan requirement 8)");
    println!();
    println!("  C-c interrupts a running form; C-d leaves.");
}

/// One request, through the same door every client uses.
fn ask(session: &mut Session, id: u64, op: Op) -> Response {
    session.handle(Request { id, op })
}

fn run_once(session: &mut Session, src: &str) -> ExitCode {
    let r = ask(session, 1, Op::Eval { code: src.into() });
    match r.outcome {
        Outcome::Value { value, output } => {
            print!("{output}");
            // A definition or a `for` returns nothing worth printing.
            if value != "#<unspecified>" {
                println!("{value}");
            }
            ExitCode::SUCCESS
        }
        Outcome::Failed {
            message, output, ..
        } => {
            print!("{output}");
            eprintln!("{message}");
            ExitCode::FAILURE
        }
        other => {
            eprintln!("scm: unexpected outcome {other:?}");
            ExitCode::FAILURE
        }
    }
}

fn repl(session: &mut Session) -> ExitCode {
    let version = match ask(session, 0, Op::Ping).outcome {
        Outcome::Pong { version } => version,
        _ => "?".into(),
    };
    println!("scm {version} — C-c interrupts, C-d leaves");
    println!(
        "{} globals. (apropos \"vector\") lists names, (doc 'map) and (signature 'map) explain one.",
        session.interp_mut().complete("").len()
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut pending = String::new();
    let mut id = 0u64;

    loop {
        let prompt = if pending.is_empty() { "scm> " } else { "...  " };
        print!("{prompt}");
        let _ = io::stdout().flush();

        let Some(line) = lines.next() else {
            println!();
            return ExitCode::SUCCESS;
        };
        let line = match line {
            Ok(l) => l,
            // A C-c during `read_line` surfaces as an interrupted read, not
            // as EOF: clear the pending form and offer a fresh prompt rather
            // than exiting.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                pending.clear();
                println!();
                continue;
            }
            Err(e) => {
                eprintln!("scm: {e}");
                return ExitCode::FAILURE;
            }
        };

        if pending.is_empty() && line.trim().is_empty() {
            continue;
        }
        pending.push_str(&line);
        pending.push('\n');

        id += 1;
        // The server decides whether the form is finished -- the same
        // question an editor pane asks, answered the same way.
        match ask(session, id, Op::Parse { code: pending.clone() }).outcome {
            Outcome::Framed(Framing::Incomplete) => continue,
            Outcome::Framed(Framing::Invalid) => {
                // The message comes from evaluating it, which reports better
                // than the framing answer does.
            }
            _ => {}
        }

        let src = std::mem::take(&mut pending);
        // A form may have been interrupted at the prompt before it ran.
        if let Some(f) = INTERRUPT.get() {
            f.store(false, Ordering::Relaxed);
        }
        id += 1;
        match ask(session, id, Op::Eval { code: src }).outcome {
            Outcome::Value { value, output } => {
                print!("{output}");
                if value != "#<unspecified>" {
                    println!("{value}");
                }
            }
            Outcome::Failed {
                message, output, ..
            } => {
                print!("{output}");
                eprintln!("{message}");
            }
            other => eprintln!("scm: unexpected outcome {other:?}"),
        }
    }
}

// ---- the socket client --------------------------------------------------

/// Serve the protocol. The [`Session`] never leaves this thread.
///
/// Connections are read on their own threads and forward requests over a
/// channel, for one reason that is the whole design: **`:interrupt` must not
/// queue behind the evaluation it is meant to stop.** The interpreter is
/// single-threaded, so an eval holds it until it finishes; a reading thread
/// therefore sets the interrupt flag *itself* -- it is an `Arc<AtomicBool>`,
/// which is `Send` -- and never waits for the session.
fn serve_forever(mut session: Session, addr: &str) -> ExitCode {
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("scm: cannot serve on {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let local = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| addr.to_string());
    // Printed on one line and flushed, so a test or a client can wait for it.
    println!("scm serving on {local}");
    let _ = io::stdout().flush();

    let (tx, rx) = mpsc::channel::<(Request, mpsc::Sender<Response>)>();
    let flag = session.interrupt_handle();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let tx = tx.clone();
            let flag = flag.clone();
            std::thread::spawn(move || connection(stream, tx, flag));
        }
    });

    // The session's own loop. Single-threaded on purpose (plan §3.3).
    while let Ok((req, reply)) = rx.recv() {
        let _ = reply.send(session.handle(req));
    }
    ExitCode::SUCCESS
}

fn connection(
    mut stream: TcpStream,
    tx: mpsc::Sender<(Request, mpsc::Sender<Response>)>,
    flag: std::sync::Arc<AtomicBool>,
) {
    let mut buf = String::new();
    let mut chunk = [0u8; 4096];
    loop {
        // Framing IS the reader: read until what we have is a complete datum
        // (plan §8 obligation 5, reused rather than reinvented).
        while !matches!(scm::parse_status(&buf), scm::ParseStatus::Complete) || buf.trim().is_empty()
        {
            if matches!(scm::parse_status(&buf), scm::ParseStatus::Invalid(_)) {
                break;
            }
            let n = match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
        let text = std::mem::take(&mut buf);
        let response = match Request::parse(&text) {
            Err(why) => Response {
                id: 0,
                outcome: Outcome::Malformed(why),
            },
            Ok(req) => {
                if matches!(req.op, Op::Interrupt) {
                    // Out of band, which is the point: the session may be
                    // busy with the very form this is stopping.
                    flag.store(true, Ordering::Relaxed);
                    Response {
                        id: req.id,
                        outcome: Outcome::Pong {
                            version: env!("CARGO_PKG_VERSION").to_string(),
                        },
                    }
                } else {
                    let (rtx, rrx) = mpsc::channel();
                    if tx.send((req, rtx)).is_err() {
                        return;
                    }
                    match rrx.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    }
                }
            }
        };
        let line = format!("{}\n", response.to_wire());
        if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
            return;
        }
    }
}
