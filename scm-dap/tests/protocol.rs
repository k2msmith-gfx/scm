//! The adapter over its real transport, against a program that really
//! fails.
//!
//! What has to be true, and what nothing else checks:
//!
//!   1. the break **stops** -- a `stopped` event arrives rather than the
//!      error unwinding as it would with no handler at all;
//!   2. the frames are **live**: locals can be read and expressions
//!      evaluated *in a frame's environment*, which is the one thing a
//!      post-mortem backtrace cannot give;
//!   3. `,return EXPR` really continues the program, so the value appears
//!      in what the program goes on to print -- the hardest claim here and
//!      the one most likely to be quietly wrong;
//!   4. the adapter admits what it cannot do (breakpoints, stepping)
//!      instead of accepting and ignoring.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Dap {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    seq: i64,
    /// Every `output` event seen so far, whenever it was seen.
    ///
    /// Accumulated in `read` rather than collected on demand, because since
    /// S25 output arrives **while** the program runs -- so a chunk can land
    /// before the `stopped` event, and a reader that only started
    /// collecting afterwards threw it away. That is how a real client has
    /// to behave too.
    said: String,
}

impl Dap {
    fn start() -> Dap {
        let mut child = Command::new(env!("CARGO_BIN_EXE_scm-dap"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn scm-dap");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut dap = Dap { child, stdin, out, seq: 0, said: String::new() };
        let caps = dap.request("initialize", serde_json::json!({"adapterID": "scm"}));
        assert_eq!(caps["success"], true, "{caps}");
        // Stepping and breakpoints are absent from the capabilities on
        // purpose; a client must not be told they exist.
        assert_eq!(caps["body"]["supportsStepBack"], false);
        assert!(caps["body"].get("supportsStepInTargetsRequest").is_none());
        dap
    }

    fn write(&mut self, body: serde_json::Value) {
        let text = body.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", text.len(), text).unwrap();
        self.stdin.flush().unwrap();
    }

    fn send(&mut self, command: &str, arguments: serde_json::Value) -> i64 {
        self.seq += 1;
        let seq = self.seq;
        self.write(serde_json::json!({
            "seq": seq, "type": "request", "command": command, "arguments": arguments
        }));
        seq
    }

    /// Send, then return the response to *that* request, skipping events --
    /// `stopped`, `output` and `terminated` all arrive unasked.
    fn request(&mut self, command: &str, arguments: serde_json::Value) -> serde_json::Value {
        let seq = self.send(command, arguments);
        loop {
            let msg = self.read();
            if msg["type"] == "response" && msg["request_seq"] == seq {
                return msg;
            }
        }
    }

    fn read(&mut self) -> serde_json::Value {
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "adapter closed");
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
        let msg: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        if msg["type"] == "event" && msg["event"] == "output" {
            self.said.push_str(msg["body"]["output"].as_str().unwrap_or_default());
        }
        msg
    }

    /// Wait for a named event.
    fn event(&mut self, name: &str) -> serde_json::Value {
        loop {
            let msg = self.read();
            if msg["type"] == "event" && msg["event"] == name {
                return msg["body"].clone();
            }
        }
    }

    /// Everything the program printed, read on until it terminates.
    fn drain_output(&mut self) -> String {
        loop {
            let msg = self.read();
            if msg["type"] == "event" && msg["event"] == "terminated" {
                return self.said.clone();
            }
        }
    }
}

impl Drop for Dap {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A program on disk, since `launch` takes a path.
fn program(name: &str, text: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("scm-dap-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn an_uncaught_error_stops_with_its_frames_live() {
    let path = program(
        "break.scm",
        "(define (inner n) (vector-ref (vector 1 2) n))\n\
         (define (outer k) (* 2 (inner k)))\n\
         (display (outer 7))\n",
    );
    let mut dap = Dap::start();
    let launched = dap.request("launch", serde_json::json!({"program": path}));
    assert_eq!(launched["success"], true, "{launched}");

    let stopped = dap.event("stopped");
    assert_eq!(stopped["reason"], "exception", "{stopped}");
    assert!(
        stopped["description"].as_str().unwrap_or_default().contains("out of range"),
        "{stopped}"
    );

    // The stack, innermost first, with the file and line it was written at.
    let stack = dap.request("stackTrace", serde_json::json!({"threadId": 1}));
    let frames = stack["body"]["stackFrames"].as_array().unwrap();
    assert!(frames.len() >= 2, "too few frames: {stack}");
    assert!(
        frames[0]["name"].as_str().unwrap_or_default().contains("vector-ref"),
        "{:?}", frames[0]
    );
    assert_eq!(frames[0]["source"]["name"], "break.scm", "{:?}", frames[0]);
    assert_eq!(frames[0]["line"], 1, "{:?}", frames[0]);

    // The locals of the innermost frame, which is what a backtrace cannot
    // give: `n` is 7 and it is the reason the program broke.
    let scopes = dap.request("scopes", serde_json::json!({"frameId": 0}));
    let reference = scopes["body"]["scopes"][0]["variablesReference"].clone();
    let vars = dap.request("variables", serde_json::json!({"variablesReference": reference}));
    let names: Vec<&str> = vars["body"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert!(names.contains(&"n"), "no local `n`: {vars}");
    let n = vars["body"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "n")
        .unwrap();
    assert_eq!(n["value"], "7", "{n}");
}

#[test]
fn an_expression_evaluates_in_the_selected_frames_environment() {
    // Not in the global environment: `k` exists only in `outer`'s frame,
    // and an adapter that evaluated at top level would say "unbound".
    //
    // The calls are deliberately **not** in tail position -- see
    // `a_tail_call_leaves_no_frame_to_select` below for why that matters.
    let path = program(
        "frames.scm",
        "(define (inner n) (car n))\n\
         (define (outer k) (* 1 (inner k)))\n\
         (display (+ 0 (outer 5)))\n",
    );
    let mut dap = Dap::start();
    dap.request("launch", serde_json::json!({"program": path}));
    dap.event("stopped");
    let stack = dap.request("stackTrace", serde_json::json!({"threadId": 1}));
    let frames = stack["body"]["stackFrames"].as_array().unwrap().clone();
    assert!(frames.len() >= 2, "{stack}");

    // Frame 1 is `outer`'s, where `k` is bound.
    let got = dap.request(
        "evaluate",
        serde_json::json!({"expression": "k", "frameId": 1, "context": "repl"}),
    );
    assert_eq!(got["body"]["result"], "5", "{got}");

    // A real evaluation in that environment, not a lookup.
    let got = dap.request(
        "evaluate",
        serde_json::json!({"expression": "(* k 100)", "frameId": 1, "context": "repl"}),
    );
    assert_eq!(got["body"]["result"], "500", "{got}");

    // The innermost frame has `n` and not `k`: the environments are
    // distinct, which is the whole point of selecting one.
    let got = dap.request(
        "evaluate",
        serde_json::json!({"expression": "n", "frameId": 0, "context": "repl"}),
    );
    assert_eq!(got["body"]["result"], "5", "{got}");

    // A bad expression is refused, not a crash.
    let got = dap.request(
        "evaluate",
        serde_json::json!({"expression": "(car 5)", "frameId": 0, "context": "repl"}),
    );
    assert_eq!(got["success"], false, "{got}");
}

#[test]
fn a_tail_call_leaves_no_frame_to_select() {
    // §5's whole point, arriving in a debugger: a call in tail position
    // leaves the stack exactly as deep as it found it. So this program --
    // three nested calls, every one of them in tail position -- stops with
    // ONE frame, and the `k` of `outer` is not inspectable because
    // `outer`'s frame no longer exists.
    //
    // This is correct Scheme and a thin stack. It is recorded as a test
    // because the first version of the test above assumed the frame would
    // be there, and a user will assume the same thing.
    let path = program(
        "tail.scm",
        "(define (inner n) (car n))\n\
         (define (outer k) (inner k))\n\
         (outer 5)\n",
    );
    let mut dap = Dap::start();
    dap.request("launch", serde_json::json!({"program": path}));
    dap.event("stopped");
    let stack = dap.request("stackTrace", serde_json::json!({"threadId": 1}));
    let frames = stack["body"]["stackFrames"].as_array().unwrap();
    assert_eq!(frames.len(), 1, "a tail call left a frame behind: {stack}");
    // The one frame is the innermost call that was *entered*, which the VM
    // tracks separately precisely so an all-tail-call failure still says
    // where it broke.
    assert!(frames[0]["name"].as_str().unwrap().contains("car"), "{stack}");
    let got = dap.request(
        "evaluate",
        serde_json::json!({"expression": "k", "frameId": 0, "context": "repl"}),
    );
    assert_eq!(got["success"], false, "`k` cannot be in scope here: {got}");
}

#[test]
fn return_continues_the_program_with_a_value() {
    // The claim that is hardest to get right: `,return 21` must make the
    // failed expression produce 21, so the program carries on and prints
    // 42. If it merely unwound, nothing would be printed at all.
    let path = program(
        "resume.scm",
        "(define (half) (car '()))\n(display (* 2 (half)))\n",
    );
    let mut dap = Dap::start();
    dap.request("launch", serde_json::json!({"program": path}));
    dap.event("stopped");
    let got = dap.request(
        "evaluate",
        serde_json::json!({"expression": ",return 21", "frameId": 0, "context": "repl"}),
    );
    assert_eq!(got["success"], true, "{got}");
    let said = dap.drain_output();
    assert!(said.contains("42"), "the program did not continue: {said:?}");
}

#[test]
fn continue_unwinds_because_the_error_already_happened() {
    let path = program("abort.scm", "(display \"before\")\n(car '())\n(display \"after\")\n");
    let mut dap = Dap::start();
    dap.request("launch", serde_json::json!({"program": path}));
    dap.event("stopped");
    let got = dap.request("continue", serde_json::json!({"threadId": 1}));
    assert_eq!(got["success"], true, "{got}");
    let said = dap.drain_output();
    // What ran before the error is kept -- it is usually the most useful
    // part of a failure -- and what came after did not run.
    assert!(said.contains("before"), "{said:?}");
    assert!(!said.contains("after"), "execution resumed past the error: {said:?}");
    assert!(said.contains("not a pair") || said.contains("car"), "no diagnosis: {said:?}");
}

#[test]
fn a_program_that_works_just_runs() {
    let path = program("fine.scm", "(display (* 6 7))\n");
    let mut dap = Dap::start();
    dap.request("launch", serde_json::json!({"program": path}));
    let said = dap.drain_output();
    assert!(said.contains("42"), "{said:?}");
}

#[test]
fn breakpoints_and_stepping_are_refused_rather_than_ignored() {
    // A debugger that accepted a breakpoint and never honoured it would be
    // worse than one that says it has none.
    let path = program("bp.scm", "(car '())\n");
    let mut dap = Dap::start();
    let got = dap.request(
        "setBreakpoints",
        serde_json::json!({"source": {"path": path}, "breakpoints": [{"line": 1}]}),
    );
    let bps = got["body"]["breakpoints"].as_array().unwrap();
    assert_eq!(bps[0]["verified"], false, "{got}");
    assert!(bps[0]["message"].as_str().unwrap().contains("no breakpoints"), "{got}");

    dap.request("launch", serde_json::json!({"program": path}));
    dap.event("stopped");
    for step in ["next", "stepIn", "stepOut"] {
        let got = dap.request(step, serde_json::json!({"threadId": 1}));
        assert_eq!(got["success"], false, "{step} was not refused: {got}");
        assert!(got["message"].as_str().unwrap().contains("cannot step"), "{got}");
    }
}

#[test]
fn a_launch_without_a_program_is_refused_with_a_reason() {
    let mut dap = Dap::start();
    let got = dap.request("launch", serde_json::json!({}));
    assert_eq!(got["success"], false, "{got}");
    assert!(got["message"].as_str().unwrap().contains("program"), "{got}");

    let got = dap.request("launch", serde_json::json!({"program": "/nope/missing.scm"}));
    assert_eq!(got["success"], false, "{got}");
}

#[test]
fn the_console_has_the_programs_output_at_the_break() {
    // S25, and the reason this adapter wanted it. The capture used to
    // return its string only when the evaluation *ended*, and the break
    // handler fires inside that evaluation -- so the Debug Console was
    // empty at exactly the moment a user was choosing between `,return`
    // and abort, and the clue arrived after the choice. Measured then: 0
    // output events by the time the client was stopped.
    let path = program(
        "clue.scm",
        "(display \"IMPORTANT CLUE\\n\")\n(display \"step 2 done\\n\")\n(car '())\n",
    );
    let mut dap = Dap::start();
    dap.request("launch", serde_json::json!({"program": path}));
    let stopped = dap.event("stopped");
    assert_eq!(stopped["reason"], "exception", "{stopped}");

    // Both writes are already here, before anything was continued.
    assert!(
        dap.said.contains("IMPORTANT CLUE") && dap.said.contains("step 2 done"),
        "the console is still empty at the break: {:?}",
        dap.said
    );
}
