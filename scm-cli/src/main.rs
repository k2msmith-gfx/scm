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

use std::cell::RefCell;
use std::io::{self, BufRead, Read, Write};
use std::rc::Rc;
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter, MatchingBracketHighlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::Editor;

use scm::{BreakAction, Framing, Op, Outcome, Request, Response, Session};

/// The running session's interrupt flag, so a `SIGINT` handler can reach it.
static INTERRUPT: OnceLock<std::sync::Arc<AtomicBool>> = OnceLock::new();

// `signal(2)`, declared rather than depending on `libc` — this crate has no
// dependencies and one extern is cheaper than acquiring one.
unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
    fn isatty(fd: i32) -> i32;
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
    #[cfg(feature = "jit")]
    if matches!(std::env::var("SCM_TIER").as_deref(), Ok("t2") | Ok("T2")) {
        scm_llvm::install_t2(session.interp_mut().vm_mut());
        if let Some(n) = std::env::var("SCM_T2_HOT").ok().and_then(|s| s.parse().ok()) {
            session.interp_mut().vm_mut().set_native_threshold(n);
        }
    }
    let _ = INTERRUPT.set(session.interrupt_handle());
    unsafe { signal(SIGINT, on_sigint as *const () as usize) };

    if let Some(addr) = serve {
        return serve_forever(session, &addr);
    }
    match rest.first().map(String::as_str) {
        Some("-e") => match rest.get(1) {
            Some(src) => run_once(&mut session, src, "<-e>"),
            None => {
                eprintln!("scm: -e wants an expression");
                ExitCode::from(2)
            }
        },
        Some(path) => match std::fs::read_to_string(path) {
            Ok(src) => run_once(&mut session, &src, path),
            Err(e) => {
                eprintln!("scm: {path}: {e}");
                ExitCode::from(2)
            }
        },
        None => repl(session),
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

/// A script's `(current-input-port)` is the process's stdin. The runtime's
/// default is an empty string port, which is right for an embedded host
/// (plan §7a: a script must not read the host's stdin), so it is the client
/// that connects the two, and only this client: the REPL and `--serve` own
/// the terminal and the socket themselves. The port is lazy -- nothing is
/// read until the script reads -- so a script that never does cannot block.
fn connect_stdin(session: &mut Session) {
    let code = "(define current-input-port (make-parameter (%stdin-port)))".to_string();
    let _ = ask(session, 0, Op::Eval { code, source: Some("<stdin>".into()) });
}

fn run_once(session: &mut Session, src: &str, source: &str) -> ExitCode {
    connect_stdin(session);
    let r = ask(session, 1, Op::Eval { code: src.into(), source: Some(source.into()) });
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
            message,
            output,
            backtrace,
            ..
        } => {
            print!("{output}");
            eprintln!("{message}");
            for f in &backtrace {
                eprintln!("  in {f}");
            }
            ExitCode::FAILURE
        }
        other => {
            eprintln!("scm: unexpected outcome {other:?}");
            ExitCode::FAILURE
        }
    }
}

/// The terminal client. With a terminal: line editing, history, multi-line
/// entry (`RET` inside an unfinished form inserts a newline, decided by the
/// same parse the socket client asks for), tab completion over the global
/// names, and the break REPL on an uncaught error. Without one -- a pipe,
/// a test -- plain lines, so `echo '(+ 1 2)' | scm` still works.
fn repl(session: Session) -> ExitCode {
    let version = env!("CARGO_PKG_VERSION");
    let session = Rc::new(RefCell::new(session));
    let names: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    refresh_names(&session, &names);
    let tty = unsafe { isatty(0) } != 0;
    println!("scm {version} — C-c interrupts, C-d leaves, ,help for commands");
    println!(
        "{} globals. (apropos \"vector\") lists names, (doc 'map) and (signature 'map) explain one.",
        names.borrow().len()
    );

    let mut editor: Option<Rc<RefCell<Line>>> = None;
    if tty {
        match Line::new(Rc::clone(&names)) {
            Ok(ed) => {
                let ed = Rc::new(RefCell::new(ed));
                // The break REPL reads from the same editor; the handler
                // gets the machine directly because the session is busy
                // with the evaluation that broke.
                let for_break = Rc::clone(&ed);
                let names_b = Rc::clone(&names);
                session.borrow_mut().interp_mut().vm_mut().set_break_handler(Some(Box::new(
                    move |vm, e| break_loop(vm, e, &for_break, &names_b),
                )));
                editor = Some(ed);
            }
            Err(e) => eprintln!("scm: no line editing ({e}); plain input"),
        }
    }

    let mut plain = io::stdin().lock().lines();
    let mut pending = String::new();
    let mut id = 0u64;
    let mut entry = 0u64;
    let mut last_backtrace: Vec<String> = Vec::new();

    loop {
        // One complete form, or a comma command.
        let src = if let Some(ed) = &editor {
            match ed.borrow_mut().read("scm> ") {
                Ok(text) => text,
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => break,
                Err(e) => {
                    eprintln!("scm: {e}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            let prompt = if pending.is_empty() { "scm> " } else { "...  " };
            print!("{prompt}");
            let _ = io::stdout().flush();
            let Some(line) = plain.next() else {
                println!();
                break;
            };
            let line = match line {
                Ok(l) => l,
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
            if pending.is_empty() && line.trim_start().starts_with(',') {
                line
            } else {
                pending.push_str(&line);
                pending.push('\n');
                id += 1;
                let framed = session.borrow_mut().handle(Request { id, op: Op::Parse { code: pending.clone() } });
                if matches!(framed.outcome, Outcome::Framed(Framing::Incomplete)) {
                    continue;
                }
                std::mem::take(&mut pending)
            }
        };
        if src.trim().is_empty() {
            continue;
        }
        if let Some(f) = INTERRUPT.get() {
            f.store(false, Ordering::Relaxed);
        }

        // Comma commands are the client's, not the language's.
        let trimmed = src.trim();
        let (code, timed) = if let Some(cmd) = trimmed.strip_prefix(',') {
            let (word, rest) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
            let rest = rest.trim();
            match word {
                "q" | "quit" | "exit" => break,
                "help" | "h" | "?" => {
                    print_help();
                    continue;
                }
                "bt" => {
                    if last_backtrace.is_empty() {
                        println!("no failure yet");
                    }
                    for f in &last_backtrace {
                        println!("  in {f}");
                    }
                    continue;
                }
                "load" | "l" => (format!("(load {})", scheme_string(rest)), false),
                "time" | "t" => (rest.to_string(), true),
                "expand" | "e" => (format!("(macroexpand '{rest})"), false),
                "expand1" | "e1" => (format!("(macroexpand-1 '{rest})"), false),
                "tier" => ("(%tier-stats)".to_string(), false),
                other => {
                    println!("unknown command ,{other}; ,help lists them");
                    continue;
                }
            }
        } else {
            (src.clone(), false)
        };

        id += 1;
        entry += 1;
        let t0 = std::time::Instant::now();
        let outcome = session
            .borrow_mut()
            .handle(Request { id, op: Op::Eval { code, source: Some(format!("<repl:{entry}>")) } })
            .outcome;
        let took = t0.elapsed();
        match outcome {
            Outcome::Value { value, output } => {
                print!("{output}");
                if value != "#<unspecified>" {
                    println!("{value}");
                }
            }
            Outcome::Failed { message, output, backtrace, .. } => {
                print!("{output}");
                eprintln!("{message}");
                for f in &backtrace {
                    eprintln!("  in {f}");
                }
                last_backtrace = backtrace;
            }
            other => eprintln!("scm: unexpected outcome {other:?}"),
        }
        if timed {
            println!("{:.3} ms", took.as_secs_f64() * 1e3);
        }
        refresh_names(&session, &names);
    }
    if let Some(ed) = &editor {
        ed.borrow_mut().save();
    }
    ExitCode::SUCCESS
}

fn refresh_names(session: &Rc<RefCell<Session>>, names: &Rc<RefCell<Vec<String>>>) {
    let mut s = session.borrow_mut();
    let mut all = s.interp_mut().complete("");
    all.sort();
    *names.borrow_mut() = all;
}

fn print_help() {
    println!("  ,load FILE      evaluate a file at top level       ,time EXPR   time one form");
    println!("  ,expand FORM    expand every macro in a form       ,expand1     one step");
    println!("  ,bt             the last failure's backtrace       ,tier        what the engines ran");
    println!("  ,quit           leave (so does C-d)");
    println!("  At a break: ,frames  ,frame N  ,locals  ,return EXPR  ,abort  -- and any");
    println!("  expression, evaluated in the selected frame.");
}

fn scheme_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The break REPL (plan §8). The frames are live: inspect them, evaluate
/// in any of them, then abort the evaluation or return a value in place
/// of the expression that failed.
fn break_loop(vm: &mut scm::Vm, e: &scm::VmError, ed: &Rc<RefCell<Line>>, names: &Rc<RefCell<Vec<String>>>) -> BreakAction {
    println!("break: {e}");
    let frames = vm.live_frames();
    show_frames(&frames);
    println!("  ,abort leaves, ,return EXPR continues with a value, ,help lists the rest");
    {
        let mut all: Vec<String> = vm.global_names().into_iter().map(str::to_string).collect();
        all.sort();
        *names.borrow_mut() = all;
    }
    let mut selected = 0usize;
    loop {
        let prompt = format!("break[{selected}]> ");
        let line = match ed.borrow_mut().read(&prompt) {
            Ok(l) => l,
            Err(_) => {
                println!("abort");
                return BreakAction::Abort;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let env = frames.get(selected).and_then(|f| f.env);
        if let Some(cmd) = line.strip_prefix(',') {
            let (word, rest) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
            match word {
                "abort" | "a" | "q" => return BreakAction::Abort,
                "frames" | "bt" => show_frames(&frames),
                "frame" | "f" => match rest.trim().parse::<usize>() {
                    Ok(n) if n < frames.len() => {
                        selected = n;
                        println!("  [{n}] {}", frames[n].what);
                    }
                    _ => println!("  ,frame N with N below {}", frames.len()),
                },
                "locals" | "l" => {
                    let bindings = vm.bindings_of(env);
                    if bindings.is_empty() {
                        println!("  no local bindings here");
                    }
                    for (name, v) in bindings {
                        println!("  {name} = {}", scm::writer::pretty_str(&vm.engine, v, 70));
                    }
                }
                "return" | "r" => match vm.eval_in(rest, env) {
                    Ok(v) => return BreakAction::Return(v),
                    Err(e) => println!("  {e}"),
                },
                "help" | "h" | "?" => print_help(),
                other => println!("  unknown command ,{other}"),
            }
            continue;
        }
        match vm.eval_in(line, env) {
            Ok(v) => {
                if v != scm::Value::Unspecified {
                    println!("{}", scm::writer::pretty_str(&vm.engine, v, 80));
                }
            }
            Err(e) => println!("  {e}"),
        }
    }
}

fn show_frames(frames: &[scm::FrameView]) {
    for (i, f) in frames.iter().enumerate() {
        match &f.loc {
            Some(l) => println!("  [{i}] {}  at {l}", f.what),
            None => println!("  [{i}] {}", f.what),
        }
    }
}

// ---- line editing -----------------------------------------------------------

/// rustyline, with the session's answers behind its hooks: completion over
/// the global names, and "is this form finished?" from the reader.
struct Line {
    editor: Editor<Hooks, DefaultHistory>,
    history: Option<std::path::PathBuf>,
}

struct Hooks {
    names: Rc<RefCell<Vec<String>>>,
    brackets: MatchingBracketHighlighter,
}

impl Line {
    fn new(names: Rc<RefCell<Vec<String>>>) -> Result<Self, ReadlineError> {
        let mut editor: Editor<Hooks, DefaultHistory> = Editor::new()?;
        editor.set_helper(Some(Hooks { names, brackets: MatchingBracketHighlighter::new() }));
        let history = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".scm_history"));
        if let Some(h) = &history {
            let _ = editor.load_history(h);
        }
        Ok(Line { editor, history })
    }

    fn read(&mut self, prompt: &str) -> Result<String, ReadlineError> {
        let text = self.editor.readline(prompt)?;
        if !text.trim().is_empty() {
            let _ = self.editor.add_history_entry(text.as_str());
        }
        Ok(text)
    }

    fn save(&mut self) {
        if let Some(h) = &self.history {
            let _ = self.editor.save_history(h);
        }
    }
}

impl Completer for Hooks {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let head = &line[..pos];
        let start = head
            .rfind(|c: char| c.is_whitespace() || "()[]'`,\"".contains(c))
            .map(|i| i + 1)
            .unwrap_or(0);
        let prefix = &head[start..];
        if prefix.is_empty() {
            return Ok((pos, Vec::new()));
        }
        let names = self.names.borrow();
        let candidates = names
            .iter()
            .filter(|n| n.starts_with(prefix))
            .map(|n| Pair { display: n.clone(), replacement: n.clone() })
            .collect();
        Ok((start, candidates))
    }
}

impl Hinter for Hooks {
    type Hint = String;
}

impl Highlighter for Hooks {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> std::borrow::Cow<'l, str> {
        self.brackets.highlight(line, pos)
    }
    fn highlight_char(&self, line: &str, pos: usize, kind: CmdKind) -> bool {
        self.brackets.highlight_char(line, pos, kind)
    }
}

impl Validator for Hooks {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        if input.trim_start().starts_with(',') {
            return Ok(ValidationResult::Valid(None));
        }
        Ok(match scm::parse_status(input) {
            scm::ParseStatus::Incomplete => ValidationResult::Incomplete,
            _ => ValidationResult::Valid(None),
        })
    }
}

impl rustyline::Helper for Hooks {}

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
