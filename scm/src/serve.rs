//! The REPL protocol over TCP, for any host (docs/repl.md).
//!
//! `scm --serve` was the only server, its loop in scm-cli. A host that
//! embeds scm and has a loop of its own -- lux's render loop, which must
//! draw between requests -- needs the same server without that loop, and
//! plan §8 is explicit about the alternative: *"building the loop first and
//! bolting a socket on later is how the Janet side ended up with an eval
//! channel per consumer."* So the server is here, and both use it.
//!
//! A listener thread accepts connections and a thread per connection reads
//! requests -- framing is the reader, as `parse_status` says -- and passes
//! them to the thread that owns the [`Session`]. That thread answers them
//! when it chooses: [`Server::serve_forever`] blocks, as `scm --serve`
//! does; [`Server::serve_pending`] answers what has arrived and returns, as
//! a frame loop wants. Two requests never need the session: `interrupt`,
//! which is out of band precisely because the session may be busy with what
//! it stops, and `stream`, which is the connection's own setting.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crate::repl::{Op, Outcome, Request, Response, Session};

struct Job {
    req: Request,
    reply: mpsc::Sender<Response>,
    /// Where output goes WHILE the request runs: `Some` only for an `eval`
    /// from a connection that asked (S25). The session serves every
    /// connection from one thread, so the job carries it -- a sink
    /// installed per connection would stream one client's output to
    /// another.
    stream: Option<TcpStream>,
}

/// A REPL server on a TCP address, answering on the session's own thread.
pub struct Server {
    jobs: mpsc::Receiver<Job>,
    addr: SocketAddr,
}

impl Server {
    /// Listen on ADDR (`127.0.0.1:0` for a free port), stopping a running
    /// evaluation through INTERRUPT -- the session's
    /// [`interrupt_handle`](Session::interrupt_handle).
    pub fn bind(addr: &str, interrupt: Arc<AtomicBool>) -> std::io::Result<Server> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let (tx, jobs) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                let flag = interrupt.clone();
                std::thread::spawn(move || connection(stream, tx, flag));
            }
        });
        Ok(Server { jobs, addr })
    }

    /// The address it listens on, with the port a `:0` was given.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Answer every request that has arrived, and return without waiting.
    /// ON_ANSWER sees each request and its response -- how a host learns
    /// that a definition was sent, to resume what an error paused. Answers
    /// how many it served.
    pub fn serve_pending(&self, session: &mut Session, mut on_answer: impl FnMut(&Request, &Response)) -> usize {
        let mut n = 0;
        while let Ok(job) = self.jobs.try_recv() {
            serve(session, job, &mut on_answer);
            n += 1;
        }
        n
    }

    /// Answer requests until every connection and the listener are gone --
    /// which, the listener living forever, is never.
    pub fn serve_forever(&self, session: &mut Session) {
        while let Ok(job) = self.jobs.recv() {
            serve(session, job, &mut |_, _| {});
        }
    }
}

fn serve(session: &mut Session, Job { req, reply, stream }: Job, on_answer: &mut impl FnMut(&Request, &Response)) {
    let answer = match stream {
        // S25: report output as it is produced, framed as its own datum,
        // down the connection that asked for it. The sink is installed for
        // this request only, so a chunk cannot be attributed to the wrong
        // `:id`.
        Some(mut socket) => {
            let id = req.id;
            session.interp_mut().set_output_sink(Some(Box::new(move |text: &str| {
                let chunk = Response { id, outcome: Outcome::Streamed(text.to_string()) };
                // A dead socket is not worth failing the evaluation for: the
                // answer will fail to send too, and that is where it shows.
                let _ = socket.write_all(format!("{}\n", chunk.to_wire()).as_bytes());
                let _ = socket.flush();
            })));
            let answer = session.handle(req.clone());
            session.interp_mut().set_output_sink(None);
            answer
        }
        None => session.handle(req.clone()),
    };
    on_answer(&req, &answer);
    let _ = reply.send(answer);
}

fn pong(id: u64) -> Response {
    Response { id, outcome: Outcome::Pong { version: env!("CARGO_PKG_VERSION").to_string() } }
}

fn connection(mut stream: TcpStream, tx: mpsc::Sender<Job>, flag: Arc<AtomicBool>) {
    let mut buf = String::new();
    let mut chunk = [0u8; 4096];
    // Per connection, not per session: two clients of one session may
    // disagree about wanting this, and a client that never asks sees
    // exactly the bytes it saw before S25.
    let mut streaming = false;
    loop {
        // Framing IS the reader: read until what we have is a complete datum
        // (plan §8 obligation 5, reused rather than reinvented).
        while !matches!(crate::parse_status(&buf), crate::ParseStatus::Complete) || buf.trim().is_empty() {
            if matches!(crate::parse_status(&buf), crate::ParseStatus::Invalid(_)) {
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
            Err(why) => Response { id: 0, outcome: Outcome::Malformed(why) },
            Ok(req) => match req.op {
                // The connection's own setting; the session keeps no
                // streaming state, the connection owns the socket.
                Op::Stream { on } => {
                    streaming = on;
                    pong(req.id)
                }
                // Out of band, which is the point: the session may be busy
                // with the very form this is stopping.
                Op::Interrupt => {
                    flag.store(true, Ordering::Relaxed);
                    pong(req.id)
                }
                _ => {
                    let (rtx, rrx) = mpsc::channel();
                    // Only an `eval` produces output worth streaming, and
                    // only a connection that asked gets it.
                    let to_stream = if streaming && matches!(req.op, Op::Eval { .. }) { stream.try_clone().ok() } else { None };
                    if tx.send(Job { req, reply: rtx, stream: to_stream }).is_err() {
                        return;
                    }
                    match rrx.recv() {
                        Ok(r) => r,
                        Err(_) => return,
                    }
                }
            },
        };
        let line = format!("{}\n", response.to_wire());
        if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
            return;
        }
    }
}
