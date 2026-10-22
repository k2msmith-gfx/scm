//! Where the server's answers about *names* come from.
//!
//! The design decision of this stage, and the reason it is worth a file:
//! **completion and hover come from a running session, not from a parse of
//! the file.** Ask a live scm what it has, and the editor knows the
//! procedure you defined at the prompt a second ago, the record accessor a
//! macro generated, and the docstring as the value carries it. Parse the
//! file instead and you know only what the text says -- which for a Lisp
//! is the smaller half, and is the difference between completion and
//! SLIME's completion.
//!
//! So this is a third client of the one door (`scm/docs/repl.md`): the
//! editor, the prompt, and the LSP, all against one `--serve`. S22 built
//! `Request::to_wire` and `Response::parse` for exactly this.
//!
//! **Nothing here evaluates anything.** Only `complete`, `describe` and
//! `ping` are ever sent. An editor that evaluated a buffer to analyse it
//! would run a scene's side effects on every keystroke.

use std::io::{Read, Write};
use std::net::TcpStream;

use scm::{Description, Op, Request, Response, Session};

pub enum Backend {
    /// A `scm --serve` elsewhere: the session the user is working in.
    Remote { stream: TcpStream, pending: String, addr: String },
    /// This process's own interpreter. The fallback, so a cold buffer with
    /// no server running still completes over the prelude's 386 names.
    Local(Box<Session>),
    /// Neither could be had. Completion is empty rather than absent, which
    /// is what an editor can cope with.
    None,
}

impl Backend {
    /// Prefer the running session; fall back to one of our own.
    pub fn new(addr: &str) -> (Backend, String) {
        match TcpStream::connect(addr) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                let mut b = Backend::Remote {
                    stream,
                    pending: String::new(),
                    addr: addr.to_string(),
                };
                // A port that accepts and is not scm would otherwise fail
                // on the first completion, where a user cannot see why.
                match b.ask(Op::Ping) {
                    Some(Response { outcome: scm::Outcome::Pong { version }, .. }) => {
                        (b, format!("connected to scm {version} on {addr}"))
                    }
                    _ => Self::local(&format!("{addr} is not an scm server")),
                }
            }
            Err(e) => Self::local(&format!("no server on {addr} ({e})")),
        }
    }

    fn local(why: &str) -> (Backend, String) {
        match Session::new() {
            Ok(s) => (
                Backend::Local(Box::new(s)),
                format!("{why}; using this process's own session"),
            ),
            Err(e) => (Backend::None, format!("{why}; and no local session either ({e})")),
        }
    }

    pub fn describe(&mut self, name: &str) -> Option<Description> {
        match self.ask(Op::Describe { name: name.to_string() })?.outcome {
            scm::Outcome::Described(d) => d,
            _ => None,
        }
    }

    pub fn complete(&mut self, prefix: &str) -> Vec<String> {
        match self.ask(Op::Complete { prefix: prefix.to_string() }) {
            Some(Response { outcome: scm::Outcome::Candidates(names), .. }) => names,
            _ => Vec::new(),
        }
    }

    fn ask(&mut self, op: Op) -> Option<Response> {
        match self {
            Backend::None => None,
            Backend::Local(s) => Some(s.handle(Request { id: 1, op })),
            Backend::Remote { stream, pending, addr } => {
                match request(stream, pending, op) {
                    Ok(r) => Some(r),
                    Err(_) => {
                        // The server went away -- restarted, or stopped. Do
                        // not keep failing silently for the rest of the
                        // editing session: fall back, and say so once.
                        let (next, note) = Self::local(&format!("lost {addr}"));
                        eprintln!("scm-lsp: {note}");
                        *self = next;
                        None
                    }
                }
            }
        }
    }
}

/// One request over the wire. Framing is the reader, as everywhere else in
/// this protocol: read until what is held is a complete datum.
fn request(stream: &mut TcpStream, pending: &mut String, op: Op) -> std::io::Result<Response> {
    let text = format!("{}\n", Request { id: 1, op }.to_wire());
    stream.write_all(text.as_bytes())?;
    stream.flush()?;
    let mut chunk = [0u8; 4096];
    loop {
        if !pending.trim().is_empty() {
            match scm::parse_status(pending) {
                scm::ParseStatus::Complete => {
                    let text = std::mem::take(pending);
                    return Response::parse(&text)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                }
                scm::ParseStatus::Invalid(e) => {
                    pending.clear();
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                }
                scm::ParseStatus::Incomplete => {}
            }
        }
        match stream.read(&mut chunk)? {
            0 => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
            n => pending.push_str(&String::from_utf8_lossy(&chunk[..n])),
        }
    }
}
