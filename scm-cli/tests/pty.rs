#![cfg(unix)]
//! The terminal client over a real pty -- the one path `boot.rs`, the
//! `--serve` tests and the Emacs check all miss.
//!
//! The REPL has two input branches, and only one of them was ever tested.
//! Piped input goes through plain `lines()`, prints its own prompt and
//! flushes it; a terminal hands the prompt to rustyline, which writes
//! through its own handle and knows nothing of Rust's stdout buffer. So
//! `(display "hello")` -- no trailing newline, so nothing flushes the
//! `LineWriter` -- printed nothing at the prompt and surfaced an entry
//! later glued to the next result, while `echo '(display "hello")' | scm`
//! was correct throughout. Reproducing that needs a pty, which is this
//! file: `posix_openpt` and a child whose controlling terminal it is.
//!
//! The asserts are per entry, not over the whole transcript, because the
//! bug was never lost output -- it was output on the wrong side of a
//! prompt. Each `send` returns only what arrived after it.

use std::ffi::c_char;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// Declared rather than depending on `libc`, as `main.rs` does for `signal`
// and `isatty`. `posix_openpt` and friends are POSIX and in libc proper --
// `openpty(3)` would be fewer lines but lives in libutil on Linux and in
// libSystem on macOS, so it would cost a `#[link]` attribute per platform.
unsafe extern "C" {
    fn posix_openpt(flags: i32) -> i32;
    fn grantpt(fd: i32) -> i32;
    fn unlockpt(fd: i32) -> i32;
    fn ptsname(fd: i32) -> *mut c_char;
    fn open(path: *const c_char, flags: i32) -> i32;
    fn setsid() -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
}

const O_RDWR: i32 = 2;

// The one constant that differs between the two platforms this runs on.
#[cfg(target_os = "macos")]
const TIOCSCTTY: u64 = 0x2000_7461;
#[cfg(not(target_os = "macos"))]
const TIOCSCTTY: u64 = 0x540E;

/// No data for this long means the REPL has said all it means to say.
const IDLE: Duration = Duration::from_millis(400);
/// A hung child fails the test instead of hanging the suite.
const CAP: Duration = Duration::from_secs(20);

/// A `scm` REPL on the far end of a pty, with everything it has written
/// so far. The reader thread owns one clone of the master; `send` writes
/// through another.
struct Repl {
    child: Child,
    input: File,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl Repl {
    fn start() -> Repl {
        // posix_openpt / grantpt / unlockpt / ptsname: the portable way to
        // a master-slave pair. `ptsname` returns a static buffer, so the
        // name is copied before anything else can call it.
        let master = unsafe { posix_openpt(O_RDWR) };
        assert!(master >= 0, "posix_openpt failed");
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        assert_eq!(unsafe { grantpt(master.as_raw_fd()) }, 0, "grantpt failed");
        assert_eq!(unsafe { unlockpt(master.as_raw_fd()) }, 0, "unlockpt failed");
        let name = unsafe { ptsname(master.as_raw_fd()) };
        assert!(!name.is_null(), "ptsname failed");
        let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_owned();

        let slave = unsafe { open(name.as_ptr(), O_RDWR) };
        assert!(slave >= 0, "open {name:?} failed");
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };

        let stdin = slave.try_clone().expect("dup slave");
        let stdout = slave.try_clone().expect("dup slave");
        let stderr = slave.try_clone().expect("dup slave");
        let ctty = slave.as_raw_fd();

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_scm"));
        cmd.stdin(Stdio::from(stdin)).stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr));
        // A plain terminal: rustyline refuses `dumb`, and the test should
        // not inherit whatever the developer's terminal happens to be.
        cmd.env("TERM", "xterm-256color");
        // `Line::new` puts its history at `$HOME/.scm_history`, so a test
        // run must not write the developer's.
        cmd.env("HOME", env!("CARGO_TARGET_TMPDIR"));
        unsafe {
            cmd.pre_exec(move || {
                // Leave the test runner's session and adopt the pty, so
                // that rustyline's `/dev/tty` is this pty and not the
                // terminal `cargo test` was launched from.
                if setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if ioctl(ctty, TIOCSCTTY, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().expect("spawn scm");
        // The parent's own slave handles go now: while any is open the
        // master never reports EOF.
        drop(slave);

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut reader = File::from(master.try_clone().expect("dup master"));
        let into = Arc::clone(&seen);
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            // A pty master reads `EIO` once the slave is gone, which is
            // this end's EOF rather than a failure.
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                into.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });

        let mut repl = Repl { child, input: File::from(master), seen };
        let banner = repl.quiet(0);
        assert!(
            banner.contains("scm> "),
            "no prompt from the pty REPL; got {banner:?}"
        );
        assert!(
            !banner.contains("no line editing"),
            "the REPL fell back to plain input, so this test would not be \
             testing the terminal branch at all: {banner:?}"
        );
        repl
    }

    /// Everything written since byte `from`, once the REPL has gone quiet.
    fn quiet(&mut self, from: usize) -> String {
        let deadline = Instant::now() + CAP;
        let mut last_len = self.seen.lock().unwrap().len();
        let mut last_change = Instant::now();
        loop {
            thread::sleep(Duration::from_millis(25));
            let len = self.seen.lock().unwrap().len();
            if len != last_len {
                last_len = len;
                last_change = Instant::now();
            } else if len > from && last_change.elapsed() >= IDLE {
                break;
            }
            if Instant::now() >= deadline {
                panic!("the REPL went quiet for {CAP:?} with nothing to say");
            }
        }
        let seen = self.seen.lock().unwrap();
        String::from_utf8_lossy(&seen[from..]).into_owned()
    }

    fn len(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The whole transcript as the terminal would be showing it.
    fn screen(&self) -> Vec<String> {
        let seen = self.seen.lock().unwrap();
        Screen::render(&String::from_utf8_lossy(&seen))
    }

    /// One entry: the bytes the REPL wrote in answer to this form alone.
    fn send(&mut self, form: &str) -> String {
        let from = self.len();
        self.input.write_all(form.as_bytes()).expect("write to pty");
        self.input.write_all(b"\r").expect("write to pty");
        self.input.flush().expect("flush pty");
        self.quiet(from)
    }
}

impl Drop for Repl {
    fn drop(&mut self) {
        let _ = self.input.write_all(&[4]); // C-d
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the terminal *shows*, which is not what the REPL wrote.
///
/// The first version of this file stripped the escapes and asserted on the
/// remaining text. That is precisely the wrong model, and it passed on a
/// bug the user could see: rustyline redraws its prompt with `\r` followed
/// by erase-to-end-of-line, so an entry's last line is wiped unless it
/// ended with a newline. Dropping the escapes drops the erase -- the one
/// sequence that decides whether anything is visible.
///
/// So: a cursor, a line buffer, and the four operations that matter --
/// `\r`, `\n`, backspace, and `ESC[K`. `ESC[<n>C` moves right because the
/// prompt redraw uses it; every other CSI is ignored, which is right for
/// the colour and bracketed-paste ones and harmless for the rest.
#[derive(Default)]
struct Screen {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

impl Screen {
    fn render(bytes: &str) -> Vec<String> {
        let mut s = Screen::default();
        let mut chars = bytes.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => match chars.next() {
                    Some('[') => {
                        let mut params = String::new();
                        let mut final_byte = None;
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                final_byte = Some(c);
                                break;
                            }
                            params.push(c);
                        }
                        match final_byte {
                            // Erase from the cursor to the end of the line.
                            Some('K') if !params.starts_with('?') => s.erase_to_end(),
                            // Cursor forward, which the redraw uses to put
                            // the cursor after the prompt.
                            Some('C') => s.col += params.parse::<usize>().unwrap_or(1),
                            _ => {}
                        }
                    }
                    // OSC, ended by BEL or ESC \.
                    Some(']') => {
                        while let Some(c) = chars.next() {
                            if c == '\x07' {
                                break;
                            }
                            if c == '\x1b' && chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    _ => {}
                },
                '\r' => s.col = 0,
                '\n' => {
                    s.row += 1;
                    s.col = 0;
                }
                '\x08' => s.col = s.col.saturating_sub(1),
                c if c.is_control() => {}
                c => s.put(c),
            }
        }
        s.lines
    }

    fn line(&mut self) -> &mut String {
        while self.lines.len() <= self.row {
            self.lines.push(String::new());
        }
        &mut self.lines[self.row]
    }

    fn put(&mut self, c: char) {
        let col = self.col;
        let line = self.line();
        // Columns are chars here: the REPL writes no wide ones, and a byte
        // index would split the UTF-8 in the banner's em dash.
        let mut cs: Vec<char> = line.chars().collect();
        while cs.len() <= col {
            cs.push(' ');
        }
        cs[col] = c;
        *line = cs.into_iter().collect();
        self.col += 1;
    }

    fn erase_to_end(&mut self) {
        let col = self.col;
        let line = self.line();
        *line = line.chars().take(col).collect();
    }
}

/// `hello` is built with `string-append` so that it cannot be confused
/// with the terminal's echo of the form that produced it: the echo shows
/// `"he"` and `"llo"`, and only the output is the whole word.
///
/// The assert is on the rendered screen, not the byte stream, because
/// those disagreed on exactly this bug -- see [`Screen`].
#[test]
fn display_without_a_newline_is_visible_on_the_terminal() {
    let mut repl = Repl::start();

    repl.send("(display (string-append \"he\" \"llo\"))");
    let screen = repl.screen();
    assert!(
        screen.iter().any(|l| l.contains("hello")),
        "`hello` is nowhere on the terminal. Either it was never flushed, or \
         it was flushed onto the line rustyline then redrew: the prompt redraw \
         is `\\r` + erase-to-end-of-line, so an entry whose output does not end \
         in a newline gets wiped. The REPL must end that line itself. \
         Screen was {screen:#?}"
    );

    // The symptom that named the bug: the buffered text came out glued to
    // whatever the next form printed.
    let hellos = repl.screen().iter().filter(|l| l.contains("hello")).count();
    repl.send("(+ 1 2)");
    let screen = repl.screen();
    assert!(
        screen.iter().any(|l| l.trim() == "3" || l.ends_with('3')),
        "the next form printed no value: {screen:#?}"
    );
    assert_eq!(
        screen.iter().filter(|l| l.contains("hello")).count(),
        hellos,
        "the previous entry's output arrived again, an entry late: {screen:#?}"
    );
}

/// The failure arm needs the same newline, and has one consequence more:
/// the message goes to stderr, which is unbuffered, so it could overtake
/// the output produced before it.
///
/// Reaching that arm from a terminal goes through the break REPL, because
/// an uncaught error at a tty breaks rather than returns -- `,abort` is
/// what turns the break into the `Outcome::Failed` this is about. Note
/// what the break itself does *not* show: the form's output is still
/// inside `eval_capturing`'s port while the break prompt is up, so this
/// asserts after the abort and not at the break.
#[test]
fn output_precedes_the_error_that_followed_it() {
    let mut repl = Repl::start();

    let broke = repl.send("(begin (display (string-append \"ou\" \"t\")) (car (list)))");
    assert!(
        broke.contains("break: car: not a pair"),
        "expected the break REPL, got {broke:?}"
    );

    repl.send(",abort");
    let screen = repl.screen();
    let out = screen
        .iter()
        .position(|l| l.contains("out") && !l.contains("string-append"))
        .unwrap_or_else(|| panic!("the output before the error is not on screen: {screen:#?}"));
    let err = screen
        .iter()
        .position(|l| l.contains("not a pair") && !l.contains("break:"))
        .unwrap_or_else(|| panic!("the error message is not on screen: {screen:#?}"));
    assert!(
        out <= err,
        "the error reached the terminal before the output that preceded it, \
         because stderr is unbuffered and stdout was not flushed: {screen:#?}"
    );
}
