//! `scm-dap` — the break REPL with a user interface.
//!
//! S8b's break REPL already stops an uncaught error **with the frames
//! live**, lets you read a frame's bindings, evaluate in its environment,
//! and then abort or continue with a value in place of the expression that
//! failed. All of that has only ever had a terminal prompt. This speaks the
//! Debug Adapter Protocol over stdio, so an editor can show the stack and
//! the locals instead — which not even `scm-mode.el` does.
//!
//! **What this is not.** There are no breakpoints and no stepping, because
//! the VM has no hook for either: `set_break_handler` fires on an error
//! nothing will catch, and that is the whole of it. The capabilities below
//! say so rather than claiming otherwise, and `setBreakpoints` answers
//! `verified: false` so a client marks the breakpoint unmet instead of
//! waiting for a stop that cannot come. A debugger that silently ignored a
//! breakpoint would be worse than one that admits it has none.
//!
//! **Why it runs the program itself**, unlike `scm-lsp`, which is a client
//! of a running `--serve`: a break handler runs *inside* the failing
//! evaluation with the stack intact. A socket carries values, not a live
//! stack, so the adapter has to be the process that owns the `Vm`.
//!
//! ## Threads
//!
//! The `Vm` is `Rc`-based and stays on one thread, so stdin is read on a
//! second thread that only ever *forwards*. Requests land in one queue
//! which **two** readers share: the adapter loop, and the break handler
//! while it is stopped. They never run at the same time — the handler runs
//! only while the adapter's thread is inside the VM — so sharing the queue
//! behind a mutex costs nothing and means there is exactly one place a
//! request can be taken from.
//!
//! An earlier version of this file gave the handler its own channels, which
//! deadlocks: nothing drains a reply channel while the thread that would
//! drain it is blocked inside the evaluation the handler is describing.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use serde_json::{Value as J, json};

use scm::{BreakAction, Session};

fn main() -> std::process::ExitCode {
    let (tx, rx) = channel::<J>();
    // Reading is all this thread does. One writer, on the session's thread,
    // so no lock is needed around stdout.
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        while let Some(msg) = read_message(&mut input) {
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
    let mut adapter = Adapter {
        seq: Arc::new(AtomicI64::new(0)),
        rx: Arc::new(Mutex::new(rx)),
        session: None,
        exited: false,
    };
    adapter.run()
}

/// One message out. Both the adapter and the break handler call this;
/// `io::stdout()` is itself locked, and the two never write at once.
fn send(seq: &AtomicI64, body: &mut J) {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("seq".into(), json!(seq.fetch_add(1, Ordering::Relaxed) + 1));
    }
    let text = body.to_string();
    let out = io::stdout();
    let mut out = out.lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", text.len(), text);
    let _ = out.flush();
}

fn read_message(input: &mut impl BufRead) -> Option<J> {
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
    serde_json::from_slice(&buf).ok()
}

struct Adapter {
    seq: Arc<AtomicI64>,
    rx: Arc<Mutex<Receiver<J>>>,
    session: Option<Box<Session>>,
    exited: bool,
}

impl Adapter {
    fn run(&mut self) -> std::process::ExitCode {
        while !self.exited {
            let msg = {
                let rx = self.rx.lock().expect("the reader thread never panics");
                rx.recv()
            };
            let Ok(msg) = msg else { break };
            self.dispatch(&msg);
        }
        std::process::ExitCode::SUCCESS
    }

    fn send(&mut self, mut body: J) {
        send(&self.seq, &mut body);
    }

    fn respond(&mut self, req: &J, body: J) {
        self.send(json!({
            "type": "response",
            "request_seq": req.get("seq").cloned().unwrap_or(J::Null),
            "success": true,
            "command": req.get("command").cloned().unwrap_or(J::Null),
            "body": body
        }));
    }

    fn refuse(&mut self, req: &J, why: &str) {
        self.send(json!({
            "type": "response",
            "request_seq": req.get("seq").cloned().unwrap_or(J::Null),
            "success": false,
            "command": req.get("command").cloned().unwrap_or(J::Null),
            "message": why
        }));
    }

    fn event(&mut self, name: &str, body: J) {
        self.send(json!({"type": "event", "event": name, "body": body}));
    }

    /// The program's own output, which cannot go to stdout: stdout is the
    /// protocol. It arrives in one piece when the program finishes, because
    /// `:output` in this runtime travels with the answer rather than
    /// streaming -- the same limit the Z4 spike measured.
    fn output(&mut self, category: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut text = text.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        self.event("output", json!({"category": category, "output": text}));
    }

    fn dispatch(&mut self, req: &J) {
        let command = req.get("command").and_then(J::as_str).unwrap_or("").to_string();
        let args = req.get("arguments").cloned().unwrap_or(J::Null);
        match command.as_str() {
            "initialize" => {
                self.respond(req, Self::capabilities());
                self.event("initialized", json!({}));
            }
            "launch" => self.launch(req, &args),
            "attach" => self.refuse(req, "scm-dap launches a program; there is nothing to attach to"),
            // Honest rather than silent: this adapter has no breakpoints,
            // so every one sent back is unverified and a client says so.
            "setBreakpoints" => {
                let unverified: Vec<J> = args
                    .pointer("/breakpoints")
                    .and_then(J::as_array)
                    .map(|bs| {
                        bs.iter()
                            .map(|b| {
                                json!({"verified": false,
                                       "line": b.get("line").cloned().unwrap_or(J::Null),
                                       "message": "scm has no breakpoints; execution stops at an \
                                                   uncaught error"})
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.respond(req, json!({"breakpoints": unverified}));
            }
            "setExceptionBreakpoints" => self.respond(req, json!({})),
            "threads" => self.respond(
                req,
                json!({"threads": [{"id": 1, "name": "scm"}]}),
            ),
            "configurationDone" => self.respond(req, json!({})),
            "disconnect" | "terminate" => {
                self.respond(req, json!({}));
                self.exited = true;
            }
            // Only reachable when the program is not running: while it is,
            // this thread is inside the VM and the break handler answers.
            "stackTrace" => self.respond(req, json!({"stackFrames": [], "totalFrames": 0})),
            "scopes" => self.respond(req, json!({"scopes": []})),
            "variables" => self.respond(req, json!({"variables": []})),
            "continue" => self.respond(req, json!({"allThreadsContinued": true})),
            "evaluate" => self.refuse(req, "nothing is running"),
            _ => self.respond(req, json!({})),
        }
    }

    fn capabilities() -> J {
        json!({
            // Everything absent is absent on purpose. The break handler is
            // the only stop this VM has: no stepping, no breakpoints, no
            // pause, no restart.
            "supportsConfigurationDoneRequest": true,
            "supportsEvaluateForHovers": true,
            "supportsTerminateRequest": true,
            "supportsExceptionInfoRequest": true,
            "exceptionBreakpointFilters": [],
            "supportsStepBack": false,
            "supportsRestartFrame": false,
            "supportsSetVariable": false
        })
    }

    fn launch(&mut self, req: &J, args: &J) {
        let Some(program) = args
            .pointer("/program")
            .and_then(J::as_str)
            .map(str::to_string)
        else {
            self.refuse(req, "launch needs a `program`: the path of a .scm file");
            return;
        };
        let src = match std::fs::read_to_string(&program) {
            Ok(s) => s,
            Err(e) => {
                self.refuse(req, &format!("{program}: {e}"));
                return;
            }
        };
        let mut session = match Session::new() {
            Ok(s) => Box::new(s),
            Err(e) => {
                self.refuse(req, &format!("could not start scm: {e}"));
                return;
            }
        };
        self.respond(req, json!({}));
        self.event("process", json!({
            "name": program, "systemProcessId": std::process::id(),
            "isLocalProcess": true, "startMethod": "launch"
        }));

        // The handler answers from inside the failing evaluation, which is
        // the only moment the stack it describes still exists. It reads the
        // same queue the adapter loop reads and writes for itself.
        let queue = Arc::clone(&self.rx);
        let seq = Arc::clone(&self.seq);
        session
            .interp_mut()
            .vm_mut()
            .set_break_handler(Some(Box::new(move |vm, err| {
                break_session(vm, err, &queue, &seq)
            })));

        // S25: report output as it is produced. Before this the capture
        // returned its string only when the evaluation *ended*, and the
        // break handler fires inside that evaluation -- so the Debug
        // Console was empty at exactly the moment a user was choosing
        // between `,return` and abort, with the clue they needed arriving
        // after the choice.
        let seq = Arc::clone(&self.seq);
        session.interp_mut().set_output_sink(Some(Box::new(move |text: &str| {
            let mut body = json!({
                "type": "event", "event": "output",
                "body": {"category": "stdout", "output": text}
            });
            send(&seq, &mut body);
        })));
        let (result, printed) = {
            let interp = session.interp_mut();
            // Still capturing as well: the sink is an addition, and a
            // client that reads only the end still gets everything.
            interp.eval_capturing_named(&src, Some(&program))
        };
        session.interp_mut().set_output_sink(None);
        // Not re-sent: the sink already delivered every byte of it as it
        // happened. `printed` is kept because the capture is still the
        // authority on what the program said, and a future client may want
        // it in one piece.
        let _ = &printed;
        match result {
            Ok(v) => {
                let text = session
                    .interp_mut()
                    .pretty(&v, 80)
                    .unwrap_or_else(|e| format!("#<unprintable: {e}>"));
                if text != "#<unspecified>" {
                    self.output("stdout", &text);
                }
                self.event("exited", json!({"exitCode": 0}));
            }
            Err(e) => {
                self.output("stderr", &format!("{e}"));
                for f in session.interp_mut().backtrace() {
                    self.output("stderr", &format!("  in {f}"));
                }
                self.event("exited", json!({"exitCode": 1}));
            }
        }
        self.event("terminated", json!({}));
        self.session = Some(session);
        self.exited = true;
    }
}

/// The break: a `stopped` event, then serve requests about the live frames
/// until the client says to carry on.
///
/// `continue` unwinds, because the error has already happened and there is
/// nothing to resume -- the choice the terminal prompt offers is abort or
/// return-a-value, and the Debug Console takes the same `,return EXPR` and
/// `,abort` spellings rather than inventing a second vocabulary for the
/// same two actions.
fn break_session(
    vm: &mut scm::Vm,
    err: &scm::VmError,
    queue: &Arc<Mutex<Receiver<J>>>,
    seq: &AtomicI64,
) -> BreakAction {
    let frames = vm.live_frames();
    let mut selected = 0usize;
    let emit = |mut body: J| send(seq, &mut body);
    emit(json!({
        "type": "event", "event": "stopped",
        "body": {
            "reason": "exception",
            "description": format!("{err}"),
            "text": format!("{err}"),
            "threadId": 1,
            "allThreadsStopped": true
        }
    }));

    loop {
        let req = {
            let rx = queue.lock().expect("the reader thread never panics");
            rx.recv()
        };
        let req = match req {
            Ok(r) => r,
            // The client went away mid-break; unwinding is the only safe
            // thing left, and it is what no handler at all would have done.
            Err(_) => return BreakAction::Abort,
        };
        let command = req.get("command").and_then(J::as_str).unwrap_or("").to_string();
        let args = req.get("arguments").cloned().unwrap_or(J::Null);
        let request_seq = req.get("seq").cloned().unwrap_or(J::Null);
        let ok = |body: J| {
            json!({"type": "response", "request_seq": request_seq, "success": true,
                   "command": command, "body": body})
        };
        match command.as_str() {
            "stackTrace" => {
                let stack: Vec<J> = frames
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let (source, line, column) = split_loc(f.loc.as_deref());
                        let mut frame = json!({
                            "id": i, "name": f.what.clone(),
                            "line": line, "column": column
                        });
                        // A form built at run time has no location, and
                        // says nothing rather than something wrong -- the
                        // same rule the backtrace follows.
                        if let Some(path) = source {
                            frame["source"] = json!({"name": file_name(&path), "path": path});
                        }
                        frame
                    })
                    .collect();
                let total = stack.len();
                emit(ok(json!({"stackFrames": stack, "totalFrames": total})));
            }
            "scopes" => {
                let id = args.pointer("/frameId").and_then(J::as_i64).unwrap_or(0);
                selected = id.max(0) as usize;
                emit(ok(json!({"scopes": [{
                    "name": "Locals",
                    // The frame's own id is the reference, so `variables`
                    // knows which environment to read.
                    "variablesReference": selected + 1,
                    "expensive": false
                }]})));
            }
            "variables" => {
                let r = args.pointer("/variablesReference").and_then(J::as_i64).unwrap_or(1);
                let idx = (r.max(1) - 1) as usize;
                let env = frames.get(idx).and_then(|f| f.env);
                let vars: Vec<J> = vm
                    .bindings_of(env)
                    .into_iter()
                    .map(|(name, v)| {
                        json!({
                            "name": name,
                            "value": scm::writer::pretty_str(&vm.engine, v, 70),
                            "variablesReference": 0
                        })
                    })
                    .collect();
                emit(ok(json!({"variables": vars})));
            }
            "evaluate" => {
                let expr = args.pointer("/expression").and_then(J::as_str).unwrap_or("").trim().to_string();
                if let Some(frame) = args.pointer("/frameId").and_then(J::as_i64) {
                    selected = frame.max(0) as usize;
                }
                let env = frames.get(selected).and_then(|f| f.env);
                // The terminal break REPL's own commands, so one vocabulary
                // serves both: `,abort` leaves, `,return EXPR` continues
                // with a value in place of what failed.
                if let Some(rest) = expr.strip_prefix(",return").or_else(|| expr.strip_prefix(",r ")) {
                    match vm.eval_in(rest.trim(), env) {
                        Ok(v) => {
                            emit(ok(json!({"result": scm::writer::pretty_str(&vm.engine, v, 70),
                                           "variablesReference": 0})));
                            return BreakAction::Return(v);
                        }
                        Err(e) => emit(json!({"type": "response", "request_seq": request_seq,
                                              "success": false, "command": command,
                                              "message": format!("{e}")})),
                    }
                } else if expr == ",abort" || expr == ",a" {
                    emit(ok(json!({"result": "aborting", "variablesReference": 0})));
                    return BreakAction::Abort;
                } else {
                    match vm.eval_in(&expr, env) {
                        Ok(v) => emit(ok(json!({
                            "result": scm::writer::pretty_str(&vm.engine, v, 70),
                            "variablesReference": 0
                        }))),
                        Err(e) => emit(json!({"type": "response", "request_seq": request_seq,
                                              "success": false, "command": command,
                                              "message": format!("{e}")})),
                    }
                }
            }
            "exceptionInfo" => {
                emit(ok(json!({
                    "exceptionId": "error",
                    "description": format!("{err}"),
                    "breakMode": "always"
                })));
            }
            "continue" => {
                emit(ok(json!({"allThreadsContinued": true})));
                return BreakAction::Abort;
            }
            "threads" => emit(ok(json!({"threads": [{"id": 1, "name": "scm"}]}))),
            "disconnect" | "terminate" => {
                emit(ok(json!({})));
                return BreakAction::Abort;
            }
            // Stepping does not exist here. Answering rather than ignoring,
            // so a client that sends one is not left waiting.
            "next" | "stepIn" | "stepOut" | "pause" => {
                emit(json!({"type": "response", "request_seq": request_seq, "success": false,
                            "command": command,
                            "message": "scm cannot step: the break is an error, not a pause"}));
            }
            _ => emit(ok(json!({}))),
        }
    }
}

/// `"scene.scm:12:3"` as its parts. A location is formatted by the VM, so
/// it is split from the right: a path may contain colons, a line and a
/// column may not.
fn split_loc(loc: Option<&str>) -> (Option<String>, i64, i64) {
    let Some(loc) = loc else { return (None, 0, 0) };
    let mut parts = loc.rsplitn(3, ':');
    let col = parts.next().and_then(|c| c.parse().ok());
    let line = parts.next().and_then(|l| l.parse().ok());
    let path = parts.next();
    match (path, line, col) {
        (Some(p), Some(l), Some(c)) => (Some(p.to_string()), l, c),
        // Not a location this adapter understands: name the frame and place
        // it nowhere, rather than at a line that is a guess.
        _ => (None, 0, 0),
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}
