//! **A REPL over pipes** — a program that reads a line, evaluates it, and
//! marks where its answer ends: `evcxr --ide-mode`, Rust's REPL (Kevin,
//! 2026-11-17: "launch something like evcxr and connect to it like with scm
//! or guile"). scm and Guile answer on a socket, in a protocol that says
//! which request each answer is for (`scm_binding/remote.rs`); a program
//! like this one only writes to its standard output, so the answer to a
//! request is everything it wrote up to the next end mark, and requests are
//! answered in the order they were sent.
//!
//! Like a terminal's and the agent's, a REPL's reader thread never touches a
//! pane: it queues [`Answer`]s and asks the host to come round
//! ([`crate::wake::wake`]); the host drains them with [`poll`]. The state is
//! one per UI thread, shared with the reader threads by `Arc`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// How a program frames what it says.
#[derive(Clone, Debug)]
pub struct Framing {
    /// Written after an answer that succeeded, and after one that failed.
    pub ok: String,
    pub failed: String,
    /// What the program writes before it reads a line — taken off the
    /// front of an answer, and everything before the first one (a banner)
    /// with it.
    pub prompt: String,
}

/// One request's answer: everything the program wrote for it, colours and
/// the prompt taken out.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub repl: u32,
    pub request: u64,
    pub ok: bool,
    pub output: String,
}

struct Repl {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    /// Requests sent and not yet answered, oldest first.
    pending: Arc<Mutex<VecDeque<u64>>>,
    alive: Arc<AtomicBool>,
}

struct Shared {
    answers: Mutex<VecDeque<Answer>>,
    repls: Mutex<HashMap<u32, Repl>>,
    next: AtomicU64,
}

thread_local! {
    static SHARED: Arc<Shared> = Arc::new(Shared {
        answers: Mutex::new(VecDeque::new()),
        repls: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
    });
}

fn shared() -> Arc<Shared> {
    SHARED.with(Arc::clone)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// TEXT without its terminal colours: the CSI sequences a program writes
/// whether or not anything is there to show them (evcxr colours its prompt
/// and its error reports with `NO_COLOR` set and no terminal).
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            // Parameters and intermediates, then one final byte.
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// Where PROGRAM is: itself when it names a file, else the first of that
/// name on the PATH, else cargo's own bin — `cargo install` puts a program
/// there, and an editor started from the dock has no shell's PATH to find
/// it by.
pub fn which(program: &str) -> Option<std::path::PathBuf> {
    let named = std::path::Path::new(program);
    if named.components().count() > 1 {
        return named.is_file().then(|| named.to_path_buf());
    }
    let on_path = std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(program)).find(|p| p.is_file()));
    on_path.or_else(|| {
        let home = std::env::var_os("CARGO_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cargo")))?;
        Some(home.join("bin").join(program)).filter(|p| p.is_file())
    })
}

/// What a request's stretch of output says, less the prompt in front of it
/// — and, the first time, whatever came before the prompt.
fn answer_text(raw: &[u8], prompt: &str, first: bool) -> String {
    let text = plain(&String::from_utf8_lossy(raw));
    let body = match text.find(prompt) {
        Some(at) if !prompt.is_empty() && (first || text[..at].trim().is_empty()) => &text[at + prompt.len()..],
        _ => &text,
    };
    body.trim_end().to_string()
}

/// Start PROGRAM with ARGS in CWD, to be spoken to a line at a time. Its
/// standard error is folded into its output, in the order it wrote them: an
/// error report comes on one, the mark that ends it on the other, and two
/// pipes would not say which was first.
pub fn start(program: &str, args: &[String], cwd: Option<&std::path::Path>, framing: Framing) -> Result<u32, String> {
    use std::process::{Command, Stdio};
    let shared = shared();
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(r#"exec "$0" "$@" 2>&1"#).arg(program).args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        return Err(format!("{program} has no pipes"));
    };
    let id = shared.next.fetch_add(1, Ordering::SeqCst) as u32;
    let pending = Arc::new(Mutex::new(VecDeque::new()));
    let alive = Arc::new(AtomicBool::new(true));
    lock(&shared.repls).insert(id, Repl { child, stdin, pending: Arc::clone(&pending), alive: Arc::clone(&alive) });
    let program = program.to_string();
    std::thread::spawn(move || read(&shared, id, stdout, &framing, &pending, &alive, &program));
    Ok(id)
}

/// A REPL's reader: its output cut into answers at the end marks, each
/// given to the oldest request still waiting; and, when the output ends,
/// every request left told so.
fn read(shared: &Shared, id: u32, mut out: std::process::ChildStdout, framing: &Framing, pending: &Mutex<VecDeque<u64>>, alive: &AtomicBool, program: &str) {
    use std::io::Read;
    let marks = [(framing.ok.as_bytes(), true), (framing.failed.as_bytes(), false)];
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut first = true;
    let answer = |request, ok, output| {
        lock(&shared.answers).push_back(Answer { repl: id, request, ok, output });
        crate::wake::wake();
    };
    loop {
        let n = out.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        // Every mark in what has arrived, earliest first.
        while let Some((at, len, ok)) = marks
            .iter()
            .filter(|(m, _)| !m.is_empty())
            .filter_map(|(m, ok)| buf.windows(m.len()).position(|w| w == *m).map(|at| (at, m.len(), *ok)))
            .min_by_key(|(at, _, _)| *at)
        {
            let text = answer_text(&buf[..at], &framing.prompt, first);
            buf.drain(..at + len);
            first = false;
            // An answer nobody asked for (the program spoke first) is dropped.
            if let Some(request) = lock(pending).pop_front() {
                answer(request, ok, text);
            }
        }
    }
    alive.store(false, Ordering::SeqCst);
    let last = answer_text(&buf, &framing.prompt, first);
    let waiting: Vec<u64> = lock(pending).drain(..).collect();
    for request in waiting {
        let said = if last.is_empty() { String::new() } else { format!(": {last}") };
        answer(request, false, format!("{program} exited{said}"));
    }
}

/// Put LINE to REPL `id` — one line; a program that takes several in one
/// request has its own way to write a newline, which the caller applies.
/// The request's number: its answer comes through [`poll`].
pub fn send(id: u32, line: &str) -> Result<u64, String> {
    use std::io::Write;
    let shared = shared();
    let mut repls = lock(&shared.repls);
    let repl = repls.get_mut(&id).ok_or("no such REPL")?;
    if !repl.alive.load(Ordering::SeqCst) {
        return Err("it has exited".into());
    }
    let request = shared.next.fetch_add(1, Ordering::SeqCst);
    // Waiting BEFORE the line is written: the answer may come before
    // `write` returns.
    lock(&repl.pending).push_back(request);
    let line = line.replace('\n', " ");
    if writeln!(repl.stdin, "{line}").and_then(|()| repl.stdin.flush()).is_err() {
        lock(&repl.pending).retain(|r| *r != request);
        return Err("it has exited".into());
    }
    Ok(request)
}

/// Every answer since the last call, oldest first.
pub fn poll() -> Vec<Answer> {
    lock(&shared().answers).drain(..).collect()
}

/// How many requests REPL `id` has yet to answer.
pub fn pending(id: u32) -> usize {
    lock(&shared().repls).get(&id).map_or(0, |r| lock(&r.pending).len())
}

/// Is REPL `id` running?
pub fn alive(id: u32) -> bool {
    lock(&shared().repls).get(&id).is_some_and(|r| r.alive.load(Ordering::SeqCst))
}

/// Interrupt what REPL `id` is evaluating, as Ctrl-C at its terminal would
/// (evcxr stops the code and stays up; the request answers as failed).
/// False when there is nothing to interrupt.
pub fn interrupt(id: u32) -> bool {
    let pid = match lock(&shared().repls).get(&id) {
        Some(r) if r.alive.load(Ordering::SeqCst) => r.child.id(),
        _ => return false,
    };
    std::process::Command::new("kill").args(["-INT", &pid.to_string()]).status().is_ok_and(|s| s.success())
}

/// Stop REPL `id`. Requests it had not answered are answered as failed, by
/// its reader, when its output ends.
pub fn stop(id: u32) {
    if let Some(mut repl) = lock(&shared().repls).remove(&id) {
        let _ = repl.child.kill();
        let _ = repl.child.wait();
    }
}

    /// A stand-in for `evcxr --ide-mode --disable-readline`, as measured
    /// against evcxr 0.21.1 (2026-11-17): a banner; a coloured prompt
    /// before each line it reads; the line's output; then U+0091 for a
    /// success or U+0092 for a failure, with no newline. A line with
    /// U+2028 in it is several lines of code.
    #[cfg(test)]
pub(crate) fn fake_evcxr(name: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("edit-pipe-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"#!/bin/sh
echo 'Welcome to evcxr. For help, type :help'
while :; do
  printf '\033[33m>> \033[0m'
  IFS= read -r line || exit 0
  case "$line" in
    *boom*) printf '\033[31m[E0425] Error:\033[0m cannot find value `boom` in this scope\n   ╭─[command:1:1]\n' >&2; printf '\302\222';;
    *forever*) sleep 30; printf '\302\221';;
    *quit*) echo 'going'; exit 3;;
    *) printf 'ran: %s\n' "$line"; printf '\302\221';;
  esac
done
"#;
        let tmp = dir.join("evcxr.new");
        std::fs::write(&tmp, script).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        let bin = dir.join("evcxr");
        std::fs::rename(&tmp, &bin).unwrap();
        bin
    }

#[cfg(test)]
mod tests {
    use super::*;

    fn framing() -> Framing {
        Framing { ok: "\u{91}".into(), failed: "\u{92}".into(), prompt: ">> ".into() }
    }

    fn wait(n: usize) -> Vec<Answer> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut got = Vec::new();
        while got.len() < n {
            got.extend(poll());
            assert!(std::time::Instant::now() < deadline, "never came: {got:?}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn colours_come_out_of_what_a_program_says() {
        assert_eq!(plain("\u{1b}[33m>> \u{1b}[0mok\r\n\u{1b}[38;5;246m╭─\u{1b}[0m"), ">> ok\n╭─");
        assert_eq!(answer_text("Welcome\n\u{1b}[33m>> \u{1b}[0m10\n".as_bytes(), ">> ", true), "10", "the banner goes with the first prompt");
        assert_eq!(answer_text(">> a >> b\n".as_bytes(), ">> ", false), "a >> b", "only the prompt in front");
        assert_eq!(answer_text("x >> y".as_bytes(), ">> ", false), "x >> y", "a prompt's text inside an answer is the answer's");
    }

    #[test]
    fn requests_are_answered_in_order_each_with_its_own_output() {
        let bin = fake_evcxr("order");
        let id = start(&bin.to_string_lossy(), &["--ide-mode".into()], None, framing()).expect("started");
        assert!(alive(id));
        let a = send(id, "let x = 5;").unwrap();
        let b = send(id, "boom").unwrap();
        // Several lines of code are one request: the caller's U+2028.
        let c = send(id, "fn sq() {\u{2028}}").unwrap();
        let got = wait(3);
        assert_eq!(got[0], Answer { repl: id, request: a, ok: true, output: "ran: let x = 5;".into() }, "no banner, no prompt");
        assert_eq!(got[1].request, b);
        assert!(!got[1].ok, "U+0092 is a failure");
        assert_eq!(got[1].output, "[E0425] Error: cannot find value `boom` in this scope\n   ╭─[command:1:1]", "what it wrote to stderr, in place, colours out");
        assert_eq!((got[2].request, got[2].output.as_str()), (c, "ran: fn sq() {\u{2028}}"));
        assert_eq!(pending(id), 0);

        // Interrupted, it stops; stopped with a request waiting, the request
        // is told so rather than left.
        let d = send(id, "run forever").unwrap();
        assert_eq!(pending(id), 1);
        let asked = std::time::Instant::now();
        stop(id);
        let got = wait(1);
        assert_eq!((got[0].request, got[0].ok), (d, false));
        assert!(got[0].output.contains("exited"), "{:?}", got[0]);
        assert!(asked.elapsed() < std::time::Duration::from_secs(10));
        assert!(!alive(id) && send(id, "late").is_err());
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    #[test]
    fn a_program_that_exits_says_so_to_whoever_is_waiting() {
        let bin = fake_evcxr("exits");
        let id = start(&bin.to_string_lossy(), &[], None, framing()).expect("started");
        let a = send(id, "quit").unwrap();
        let got = wait(1);
        assert_eq!(got[0].request, a);
        assert!(!got[0].ok && got[0].output.ends_with("exited: going"), "{:?}", got[0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while alive(id) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(send(id, "x").unwrap_err().contains("exited"));
        assert!(start("/nonexistent/evcxr", &[], None, framing()).is_ok(), "sh starts; the program's absence is its first answer");
        stop(id);
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }
}
