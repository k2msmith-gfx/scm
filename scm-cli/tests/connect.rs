#![cfg(unix)]
//! `scm --connect ADDR` (S22): the terminal prompt as a client of a server
//! somewhere else.
//!
//! What is worth testing is not that a request crosses a socket -- S8's
//! tests cover the protocol -- but the three things that are only true of
//! this client:
//!
//!   1. **the state is the server's.** Two prompts are two processes and one
//!      environment. If each quietly built a session of its own, every test
//!      of a single prompt would still pass and the feature would be
//!      pointless.
//!   2. **`C-c` arrives out of band.** The interrupt must not queue behind
//!      the evaluation it stops, and the client's end of that is a signal
//!      handler writing to a second connection -- a path no unit test
//!      reaches, because it only exists in a real process under a real
//!      signal.
//!   3. **a failure to connect is a message, not a panic**, since the thing
//!      on the far end may be absent or may not be scm.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Declared rather than depending on `libc`, as `main.rs` does for `signal`
// and `pty.rs` does for `posix_openpt`.
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
const SIGINT: i32 = 2;

/// A server on a port the kernel picks, and the address it actually got.
/// Port 0 rather than a constant: a fixed port makes a test that passes
/// alone and fails beside its own earlier run.
fn serve() -> (Child, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scm"))
        .args(["--serve", "127.0.0.1:0"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn scm --serve");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .expect("the server announces its address");
    let addr = line
        .trim()
        .rsplit(' ')
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(addr.contains(':'), "no address in {line:?}");
    (child, addr)
}

/// Drive a prompt with piped stdin and collect everything it said.
fn prompt(addr: &str, input: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_scm"))
        .args(["--connect", addr])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            c.stdin.take().unwrap().write_all(input.as_bytes())?;
            c.wait_with_output()
        })
        .expect("run scm --connect");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}


/// Drain a child's stream into a string a test can poll. A blocking
/// `read_line` would make every deadline below unenforceable: it is only
/// checked between reads, so a stream that says nothing hangs the test
/// instead of failing it. `pty.rs` drains the same way and for the same
/// reason.
fn drain(mut r: impl Read + Send + 'static) -> Arc<Mutex<String>> {
    let seen = Arc::new(Mutex::new(String::new()));
    let out = Arc::clone(&seen);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 1024];
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            out.lock().unwrap().push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    });
    seen
}

/// Wait for `want` to appear, or say what did appear instead.
fn wait_for(seen: &Arc<Mutex<String>>, want: &str, secs: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let so_far = seen.lock().unwrap().clone();
        if so_far.contains(want) || Instant::now() > deadline {
            return so_far;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn two_prompts_are_one_environment() {
    let (mut server, addr) = serve();

    let first = prompt(&addr, "(define from-the-first 41)\n(+ from-the-first 1)\n");
    assert!(first.contains("42"), "{first}");

    // The second process never defined it. If this fails, each prompt has a
    // session of its own and `--connect` is doing nothing.
    let second = prompt(&addr, "(* from-the-first 2)\n");
    assert!(second.contains("82"), "{second}");

    // And the server's version is what the banner reports, not this
    // binary's `env!` -- a remote prompt must not claim to be the server.
    assert!(second.contains(&format!("on {addr}")), "{second}");

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn an_interrupt_stops_a_runaway_and_leaves_the_session_alive() {
    let (mut server, addr) = serve();
    let mut client = Command::new(env!("CARGO_BIN_EXE_scm"))
        .args(["--connect", &addr])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn scm --connect");
    let mut stdin = client.stdin.take().unwrap();
    // A failure's message and backtrace go to stderr, a value to stdout, so
    // both are watched -- and watching only one is how the first version of
    // this test came to wait ten seconds for text that had already arrived
    // somewhere else.
    let out = drain(client.stdout.take().unwrap());
    let err = drain(client.stderr.take().unwrap());

    stdin.write_all(b"(let loop () (loop))\n").unwrap();
    stdin.flush().unwrap();
    // The form has to be *running* before the signal: an interrupt that
    // arrives first is cleared by the evaluation it was meant to stop.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(unsafe { kill(client.id() as i32, SIGINT) }, 0, "kill failed");

    // The whole point: the answer comes back on the *main* connection,
    // although the request that caused it went out on the control one.
    let said = wait_for(&err, "interrupted", 10);
    assert!(said.contains("interrupted"), "no interrupt got through: {said:?}");
    // And the host stopping is distinguishable from the script failing: the
    // frame says where it was, which a bare "interrupted" would not.
    assert!(said.contains("(loop)"), "no frame with the interrupt: {said:?}");

    // The session survived rather than being left wedged -- and if the
    // runaway had not stopped, this answer could never arrive at all.
    stdin.write_all(b"(* 6 7)\n").unwrap();
    stdin.flush().unwrap();
    let said = wait_for(&out, "42", 10);
    assert!(said.contains("42"), "the session did not survive: {said:?}");

    drop(stdin);
    let _ = client.kill();
    let _ = client.wait();
    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn a_form_spanning_lines_is_framed_by_the_servers_answer() {
    // The continuation asks the server `:op parse` -- the same question the
    // local prompt asks -- so a form split across lines must not be read as
    // three broken ones.
    let (mut server, addr) = serve();
    let said = prompt(&addr, "(+ 1\n   2\n   3)\n");
    assert!(said.contains('6'), "{said}");
    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn completion_comes_from_the_server() {
    // The names tab completion offers are the server's globals, so a name
    // defined by another client is completable here. The banner's count is
    // the observable half of that.
    let (mut server, addr) = serve();
    let before = prompt(&addr, "");
    prompt(&addr, "(define a-name-only-the-server-knows 1)\n");
    let after = prompt(&addr, "");
    // "386 globals. …" is the banner's second line; the first is the
    // version. Reading the first word of the whole output got "scm".
    let count = |s: &str| -> usize {
        s.lines()
            .find(|l| l.contains("globals"))
            .and_then(|l| l.split_whitespace().next())
            .and_then(|w| w.parse().ok())
            .unwrap_or(0)
    };
    assert_eq!(count(&after), count(&before) + 1, "before {before:?} after {after:?}");
    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn a_dead_address_is_a_message_and_a_failure_code() {
    // Port 1 on loopback: nothing is listening and nothing may bind it.
    let out = Command::new(env!("CARGO_BIN_EXE_scm"))
        .args(["--connect", "127.0.0.1:1"])
        .output()
        .expect("run scm --connect");
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("cannot connect to 127.0.0.1:1"), "{said}");
}

#[test]
fn serving_and_connecting_at_once_is_refused() {
    // They are opposite ends of one protocol; doing both would silently
    // ignore one of them.
    let out = Command::new(env!("CARGO_BIN_EXE_scm"))
        .args(["--serve", "127.0.0.1:0", "--connect", "127.0.0.1:7000"])
        .output()
        .expect("run scm");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("opposite ends"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// --- S25: output as it happens, over the socket ---------------------------

/// A raw protocol client, because these tests are about *when* bytes
/// arrive and the prompt would hide that behind its own printing.
struct Wire {
    stream: std::net::TcpStream,
    pending: String,
}

impl Wire {
    fn open(addr: &str) -> Wire {
        let stream = std::net::TcpStream::connect(addr).expect("connect");
        stream.set_nodelay(true).unwrap();
        Wire { stream, pending: String::new() }
    }

    fn send(&mut self, text: &str) {
        use std::io::Write as _;
        self.stream.write_all(format!("{text}\n").as_bytes()).unwrap();
        self.stream.flush().unwrap();
    }

    /// The next datum, and how long after `since` it arrived.
    fn next(&mut self, since: Instant) -> (String, Duration) {
        use std::io::Read as _;
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(at) = self.pending.find('\n') {
                let line = self.pending[..at].trim().to_string();
                self.pending.drain(..=at);
                if !line.is_empty() {
                    return (line, since.elapsed());
                }
                continue;
            }
            let n = self.stream.read(&mut chunk).expect("read");
            assert!(n > 0, "the server closed");
            self.pending.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    }
}

/// A form that prints, then runs for about `secs`, then prints again.
fn slow_printer(secs: f64) -> String {
    let code = format!(
        "(begin (display \"FIRST\") \
         (let ((stop (+ (current-second) {secs}))) \
           (let loop () (if (< (current-second) stop) (loop) #t))) \
         (display \"SECOND\") 42)"
    );
    format!(
        "(:op eval :id 2 :code \"{}\")",
        code.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

#[test]
fn output_arrives_while_the_form_is_still_running() {
    // The measurement this stage exists for. Before S25 a form that
    // printed and then ran for two seconds sent *nothing* for two seconds
    // and then everything at once; `docs/repl.md` records the number.
    let (mut server, addr) = serve();
    let mut w = Wire::open(&addr);
    w.send("(:op stream :id 1 :on #t)");
    let t0 = Instant::now();
    let (ack, _) = w.next(t0);
    assert!(ack.contains(":status ok"), "{ack}");

    let t0 = Instant::now();
    w.send(&slow_printer(2.0));
    let (first, at_first) = w.next(t0);
    assert!(first.contains(r#":status output :output "FIRST""#), "{first}");

    // The whole claim, in one assertion: the first chunk is here long
    // before the answer can be. A wall-clock *difference* rather than an
    // absolute, so a loaded machine makes this slower, not flaky.
    let mut last = first;
    let mut at_last = at_first;
    while !last.contains(":status ok :value") {
        let (line, at) = w.next(t0);
        last = line;
        at_last = at;
    }
    assert!(
        at_last.saturating_sub(at_first) > Duration::from_millis(1000),
        "the chunk was not early: first at {at_first:?}, answer at {at_last:?}"
    );
    // And `:output` still travels with the answer, on which every client
    // written before S25 depends.
    assert!(last.contains(r#":output "FIRSTSECOND""#), "{last}");

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn a_connection_that_does_not_ask_sees_exactly_what_it_saw_before() {
    // The compatibility guarantee, and the reason streaming is opt-in:
    // `scm-mode.el` dispatches by `:id` and removes the callback when it
    // fires, so an unsolicited message carrying an in-flight id would eat
    // the real answer. A client that never asks must see one datum.
    let (mut server, addr) = serve();
    let mut w = Wire::open(&addr);
    let t0 = Instant::now();
    w.send(&slow_printer(0.2));
    let (only, _) = w.next(t0);
    assert!(only.contains(":status ok :value"), "not the answer first: {only}");
    assert!(only.contains(r#":output "FIRSTSECOND""#), "{only}");
    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn two_clients_of_one_session_can_disagree_about_streaming() {
    // Streaming is per connection, not per session. If it were session
    // state, one client asking would start sending chunks down another
    // client's socket -- which that client would read as malformed.
    let (mut server, addr) = serve();
    let mut asked = Wire::open(&addr);
    let mut quiet = Wire::open(&addr);
    asked.send("(:op stream :id 1 :on #t)");
    let t0 = Instant::now();
    let _ = asked.next(t0);

    quiet.send(&slow_printer(0.2));
    let (only, _) = quiet.next(t0);
    assert!(only.contains(":status ok :value"), "the quiet client got a chunk: {only}");

    asked.send(&slow_printer(0.2));
    let (first, _) = asked.next(t0);
    assert!(first.contains(":status output"), "the asking client got no chunk: {first}");

    let _ = server.kill();
    let _ = server.wait();
}

#[test]
fn a_chunk_is_attributed_to_the_request_that_produced_it() {
    // The `:id` on a chunk is the eval's, so a client with several requests
    // in flight can tell whose output it is reading.
    let (mut server, addr) = serve();
    let mut w = Wire::open(&addr);
    w.send("(:op stream :id 1 :on #t)");
    let t0 = Instant::now();
    let _ = w.next(t0);
    w.send(r#"(:op eval :id 77 :code "(display \"hi\")")"#);
    let (chunk, _) = w.next(t0);
    assert!(chunk.starts_with("(:id 77 :status output"), "{chunk}");
    let _ = server.kill();
    let _ = server.wait();
}
