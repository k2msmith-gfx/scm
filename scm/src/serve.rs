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
//!
//! **Events** (S40) are what the host says between requests: a connection
//! that asked (`(:op ping :events #t)`) is sent each
//! [`Event`] the host pushes with [`Server::push_event`]. So that an event
//! can never land in the middle of another datum, every write to a
//! connection -- answer, streamed chunk, event -- goes through that
//! connection's one locked writer; and so that a send's answer always
//! comes before an event it caused, the session's thread writes the answer
//! itself before the host's callback runs or the next frame is drawn.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use crate::repl::{Event, Op, Outcome, Request, Response, Session};

/// A connection's one way out. Answers are written by the session's
/// thread, pongs by the connection's, events by whichever thread the host
/// pushes from: the lock keeps each datum whole.
type Writer = Arc<Mutex<TcpStream>>;

/// Write TEXT and a newline down W, as one datum. False when the socket is
/// dead.
fn send(w: &Writer, text: &str) -> bool {
    let mut socket = w.lock().unwrap_or_else(|e| e.into_inner());
    socket.write_all(format!("{text}\n").as_bytes()).is_ok() && socket.flush().is_ok()
}

struct Job {
    req: Request,
    /// Where the answer goes, written by the session's thread (S40).
    writer: Writer,
    /// Told when the answer is written, so the connection reads its next
    /// request only then, as before.
    done: mpsc::Sender<()>,
    /// Stream output WHILE the request runs: true only for an `eval` from a
    /// connection that asked (S25). The session serves every connection
    /// from one thread, so the job carries it -- a sink installed per
    /// connection would stream one client's output to another.
    stream: bool,
}

/// The connections that asked for events, and the event a connection is
/// sent the moment it asks (see [`Server::retain_event`]). One lock for
/// both, so a connection subscribing while an event is pushed gets either
/// the retained one or the push, never neither.
#[derive(Default)]
struct Subscribers {
    conns: Vec<(u64, Writer)>,
    retained: Option<String>,
}

/// A REPL server on a TCP address, answering on the session's own thread.
pub struct Server {
    jobs: mpsc::Receiver<Job>,
    addr: SocketAddr,
    subscribers: Arc<Mutex<Subscribers>>,
}

impl Server {
    /// Listen on ADDR (`127.0.0.1:0` for a free port), stopping a running
    /// evaluation through INTERRUPT -- the session's
    /// [`interrupt_handle`](Session::interrupt_handle).
    pub fn bind(addr: &str, interrupt: Arc<AtomicBool>) -> std::io::Result<Server> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let (tx, jobs) = mpsc::channel::<Job>();
        let subscribers = Arc::new(Mutex::new(Subscribers::default()));
        let subs = subscribers.clone();
        std::thread::spawn(move || {
            let next = AtomicU64::new(1);
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                let flag = interrupt.clone();
                let subs = subs.clone();
                let id = next.fetch_add(1, Ordering::Relaxed);
                std::thread::spawn(move || connection(id, stream, tx, flag, subs));
            }
        });
        Ok(Server { jobs, addr, subscribers })
    }

    /// Send EVENT to every connection that asked for events (S40), now,
    /// from the calling thread. Answers how many it reached; a connection
    /// whose socket has died is dropped from the list.
    ///
    /// What lux pushes when a frame's error pauses it -- after the send
    /// that caused it was answered `ok`, so the editor would otherwise
    /// never hear -- and when it resumes.
    pub fn push_event(&self, event: &Event) -> usize {
        self.event_sender().push_event(event)
    }

    /// The event a connection is sent the moment it asks for events, or
    /// none: the state an editor arriving late needs. lux retains `paused`
    /// while it is paused and clears it on resuming, so an editor that
    /// connects to a paused program hears why at once rather than never.
    /// Retaining sends nothing now; [`push_event`](Server::push_event)
    /// does.
    pub fn retain_event(&self, event: Option<&Event>) {
        self.event_sender().retain_event(event)
    }

    /// A handle that pushes events from another thread -- the server
    /// itself stays on the session's.
    pub fn event_sender(&self) -> EventSender {
        EventSender { subscribers: self.subscribers.clone() }
    }

    /// How many connections have asked for events.
    pub fn subscribers(&self) -> usize {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).conns.len()
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

/// Pushes events to a [`Server`]'s subscribers from any thread
/// ([`Server::event_sender`]); `Send`, `Sync` and cheap to clone.
#[derive(Clone)]
pub struct EventSender {
    subscribers: Arc<Mutex<Subscribers>>,
}

impl EventSender {
    /// [`Server::push_event`].
    pub fn push_event(&self, event: &Event) -> usize {
        let text = event.to_wire();
        let mut subs = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
        subs.conns.retain(|(_, w)| send(w, &text));
        subs.conns.len()
    }

    /// [`Server::retain_event`].
    pub fn retain_event(&self, event: Option<&Event>) {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner()).retained = event.map(Event::to_wire);
    }
}

fn serve(session: &mut Session, Job { req, writer, done, stream }: Job, on_answer: &mut impl FnMut(&Request, &Response)) {
    let answer = if stream {
        // S25: report output as it is produced, framed as its own datum,
        // down the connection that asked for it. The sink is installed for
        // this request only, so a chunk cannot be attributed to the wrong
        // `:id`.
        let id = req.id;
        let sink = writer.clone();
        session.interp_mut().set_output_sink(Some(Box::new(move |text: &str| {
            let chunk = Response { id, outcome: Outcome::Streamed(text.to_string()) };
            // A dead socket is not worth failing the evaluation for: the
            // answer will fail to send too, and that is where it shows.
            send(&sink, &chunk.to_wire());
        })));
        let answer = session.handle(req.clone());
        session.interp_mut().set_output_sink(None);
        answer
    } else {
        session.handle(req.clone())
    };
    // The answer is written HERE, before the host hears of the request: an
    // event the host pushes because of it -- `resumed` from the callback, or
    // `paused` from the next frame -- then always follows it on the wire.
    // A dead socket shows on the connection's next read.
    send(&writer, &answer.to_wire());
    on_answer(&req, &answer);
    let _ = done.send(());
}

fn pong(id: u64) -> Response {
    Response { id, outcome: Outcome::Pong { version: env!("CARGO_PKG_VERSION").to_string() } }
}

fn connection(conn: u64, mut stream: TcpStream, tx: mpsc::Sender<Job>, flag: Arc<AtomicBool>, subs: Arc<Mutex<Subscribers>>) {
    let Ok(writer) = stream.try_clone() else { return };
    let writer: Writer = Arc::new(Mutex::new(writer));
    serve_connection(conn, &mut stream, &writer, &tx, &flag, &subs);
    // Gone, however it went: no more events for it.
    subs.lock().unwrap_or_else(|e| e.into_inner()).conns.retain(|(id, _)| *id != conn);
}

fn serve_connection(
    conn: u64,
    stream: &mut TcpStream,
    writer: &Writer,
    tx: &mpsc::Sender<Job>,
    flag: &Arc<AtomicBool>,
    subs: &Arc<Mutex<Subscribers>>,
) {
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
                // S40, the connection's own setting too. The pong goes
                // first, under the list's lock, so the answer to the ping
                // precedes the retained event and any pushed one.
                Op::Events { on } => {
                    let mut s = subs.lock().unwrap_or_else(|e| e.into_inner());
                    s.conns.retain(|(id, _)| *id != conn);
                    if !send(writer, &pong(req.id).to_wire()) {
                        return;
                    }
                    if on {
                        if let Some(retained) = &s.retained
                            && !send(writer, retained)
                        {
                            return;
                        }
                        s.conns.push((conn, writer.clone()));
                    }
                    continue;
                }
                // Out of band, which is the point: the session may be busy
                // with the very form this is stopping.
                Op::Interrupt => {
                    flag.store(true, Ordering::Relaxed);
                    pong(req.id)
                }
                _ => {
                    let (done, written) = mpsc::channel();
                    // Only an `eval` produces output worth streaming, and
                    // only a connection that asked gets it.
                    let stream = streaming && matches!(req.op, Op::Eval { .. });
                    if tx.send(Job { req, writer: writer.clone(), done, stream }).is_err() {
                        return;
                    }
                    // The session's thread writes the answer (S40); wait for
                    // it, so requests on one connection are still answered
                    // one at a time.
                    if written.recv().is_err() {
                        return;
                    }
                    continue;
                }
            },
        };
        if !send(writer, &response.to_wire()) {
            return;
        }
    }
}
