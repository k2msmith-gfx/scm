//! `scm::serve`: the REPL server for a host with a loop of its own.
//!
//! lux draws a frame, answers what has arrived, draws the next: the session
//! never blocks waiting for a client. `scm --serve`'s own tests and the
//! Emacs check drive `serve_forever`; these drive `serve_pending`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use scm::{Event, Op, Server, Session, Val};

fn connect(server: &Server) -> (TcpStream, BufReader<TcpStream>) {
    let s = TcpStream::connect(server.local_addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let r = BufReader::new(s.try_clone().unwrap());
    (s, r)
}

/// Send REQUEST, then serve pending requests -- as a frame loop would,
/// between frames -- until the answer line arrives.
fn ask(server: &Server, session: &mut Session, out: &mut TcpStream, input: &mut BufReader<TcpStream>, request: &str) -> String {
    out.write_all(request.as_bytes()).unwrap();
    out.flush().unwrap();
    let start = Instant::now();
    while server.serve_pending(session, |_, _| {}) == 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "no request arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
    line
}

#[test]
fn a_frame_loop_answers_between_frames() {
    let mut session = Session::new().unwrap();
    let server = Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
    let (mut out, mut input) = connect(&server);
    // Nothing has arrived: serving returns at once, having served nothing.
    assert_eq!(server.serve_pending(&mut session, |_, _| {}), 0);
    let a = ask(&server, &mut session, &mut out, &mut input, "(:op eval :id 1 :code \"(define (f x) (* x 6)) (f 7)\")");
    assert!(a.contains(":id 1") && a.contains("42"), "{a}");
    // State persists between requests: the definition is the session's.
    let a = ask(&server, &mut session, &mut out, &mut input, "(:op eval :id 2 :code \"(f 2)\")");
    assert!(a.contains("12"), "{a}");
    let a = ask(&server, &mut session, &mut out, &mut input, "(:op complete :id 3 :prefix \"f\")");
    assert!(a.contains(":id 3"), "{a}");
}

#[test]
fn the_host_sees_each_request_and_its_answer() {
    // How lux learns that a definition was sent, to resume what an error
    // paused.
    let mut session = Session::new().unwrap();
    let server = Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
    let (mut out, mut input) = connect(&server);
    out.write_all(b"(:op eval :id 7 :code \"(car 5)\")").unwrap();
    let mut seen = vec![];
    let start = Instant::now();
    while seen.is_empty() {
        server.serve_pending(&mut session, |req, resp| seen.push((req.id, matches!(req.op, Op::Eval { .. }), resp.to_wire())));
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    let (id, eval, wire) = &seen[0];
    assert_eq!((*id, *eval), (7, true));
    assert!(wire.contains("car"), "{wire}");
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
    assert!(line.contains(":id 7"), "{line}");
}

#[test]
fn an_interrupt_needs_no_session() {
    // Answered by the connection's thread, so a session busy with a
    // runaway -- or a frame loop that has not served yet -- still hears it.
    let session = Session::new().unwrap();
    let flag = session.interrupt_handle();
    let server = Server::bind("127.0.0.1:0", flag.clone()).unwrap();
    let (mut out, mut input) = connect(&server);
    out.write_all(b"(:op interrupt :id 9)").unwrap();
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
    assert!(line.contains(":id 9"), "{line}");
    assert!(flag.load(std::sync::atomic::Ordering::Relaxed), "the session's flag is set");
}

// ---- events (S40) --------------------------------------------------------

fn line(input: &mut BufReader<TcpStream>) -> String {
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
    line.trim_end().to_string()
}

/// Ask for events on a connection; the pong is answered by its own thread.
fn subscribe(out: &mut TcpStream, input: &mut BufReader<TcpStream>, server: &Server, want: usize) -> String {
    out.write_all(b"(:op ping :id 1 :events #t)").unwrap();
    let pong = line(input);
    let start = Instant::now();
    while server.subscribers() < want {
        assert!(start.elapsed() < Duration::from_secs(10), "never subscribed");
        std::thread::sleep(Duration::from_millis(1));
    }
    pong
}

#[test]
fn an_event_reaches_only_the_connections_that_asked() {
    let mut session = Session::new().unwrap();
    let server = Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
    let (mut a, mut a_in) = connect(&server);
    let (mut b, mut b_in) = connect(&server);
    let pong = subscribe(&mut a, &mut a_in, &server, 1);
    assert!(pong.contains(":id 1") && pong.contains(":status ok"), "{pong}");
    let paused = Event::new(
        "paused",
        vec![
            ("message", Val::Str("view: car: not a pair: 5".into())),
            ("backtrace", Val::List(vec![Val::Str("(car x)  at v.scm:3:5".into())])),
        ],
    );
    assert_eq!(server.push_event(&paused), 1);
    assert_eq!(Event::parse(&line(&mut a_in)).unwrap(), paused);
    // B never asked: the next thing it reads is the answer to its own
    // request, exactly the bytes it read before S40.
    let answer = ask(&server, &mut session, &mut b, &mut b_in, "(:op eval :id 4 :code \"(+ 1 2)\")");
    assert_eq!(answer.trim_end(), "(:id 4 :status ok :value \"3\" :output \"\")");
    // Asked to stop, A hears no more; a dead connection is dropped.
    a.write_all(b"(:op ping :id 2 :events #f)").unwrap();
    assert!(line(&mut a_in).contains(":id 2"));
    assert_eq!(server.subscribers(), 0);
    assert_eq!(server.push_event(&Event::new("resumed", vec![])), 0);
}

#[test]
fn an_answer_comes_before_the_event_it_caused() {
    // lux resumes from the callback when a definition is sent, and pushes
    // `resumed` there: the send's `ok` must still be first on the wire.
    let mut session = Session::new().unwrap();
    let server = Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
    let (mut out, mut input) = connect(&server);
    subscribe(&mut out, &mut input, &server, 1);
    out.write_all(b"(:op eval :id 5 :code \"(define (view) 1)\")").unwrap();
    let start = Instant::now();
    while server.serve_pending(&mut session, |_, _| {
        server.push_event(&Event::new("resumed", vec![]));
    }) == 0
    {
        assert!(start.elapsed() < Duration::from_secs(10), "no request arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(line(&mut input).starts_with("(:id 5 :status ok"));
    assert_eq!(line(&mut input), "(:event resumed)");
}

#[test]
fn a_late_subscriber_hears_the_retained_event() {
    // An editor that connects to a program already paused hears why at
    // once, after the answer to its ping; once cleared, nothing.
    let session = Session::new().unwrap();
    let server = Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
    let paused = Event::new("paused", vec![("message", Val::Str("frame: oops".into()))]);
    server.retain_event(Some(&paused));
    let (mut a, mut a_in) = connect(&server);
    assert!(subscribe(&mut a, &mut a_in, &server, 1).contains(":id 1"));
    assert_eq!(Event::parse(&line(&mut a_in)).unwrap(), paused);
    server.retain_event(None);
    let (mut b, mut b_in) = connect(&server);
    subscribe(&mut b, &mut b_in, &server, 2);
    b.write_all(b"(:op interrupt :id 6)").unwrap();
    assert!(line(&mut b_in).contains(":id 6"), "nothing came between the pong and this answer");
}

#[test]
fn streamed_output_and_events_never_interleave() {
    // Every write to a connection holds its one writer: a burst of events
    // pushed from another thread while an eval streams arrives as whole
    // datums, each readable.
    let mut session = Session::new().unwrap();
    let server = Server::bind("127.0.0.1:0", session.interrupt_handle()).unwrap();
    let (mut out, mut input) = connect(&server);
    out.write_all(b"(:op stream :id 1 :on #t)").unwrap();
    line(&mut input);
    out.write_all(b"(:op ping :id 2 :events #t)").unwrap();
    line(&mut input);
    while server.subscribers() < 1 {
        std::thread::sleep(Duration::from_millis(1));
    }
    let pusher = {
        let server = server.event_sender();
        std::thread::spawn(move || {
            let big = "x".repeat(5000);
            for i in 0..200 {
                server.push_event(&Event::new("tick", vec![("n", Val::Int(i)), ("pad", Val::Str(big.clone()))]));
            }
        })
    };
    // Read on a thread of its own: the eval writes ~1 MB from this thread's
    // serve_pending, more than a socket buffers.
    let reader = std::thread::spawn(move || {
        let (mut events, mut answered) = (0, false);
        while events < 200 || !answered {
            let l = line(&mut input);
            if let Ok(ev) = Event::parse(&l) {
                assert_eq!(ev.get("n"), Some(&Val::Int(events)), "in order and whole");
                events += 1;
            } else {
                let r = scm::Response::parse(&l).unwrap_or_else(|e| panic!("{e}: {}", &l[..l.len().min(80)]));
                answered |= r.id == 3 && !matches!(r.outcome, scm::Outcome::Streamed(_));
            }
        }
    });
    out.write_all(b"(:op eval :id 3 :code \"(let loop ((i 0)) (when (< i 200) (display (make-string 5000 #\\\\y)) (flush-output-port) (loop (+ i 1))))\")").unwrap();
    let start = Instant::now();
    while server.serve_pending(&mut session, |_, _| {}) == 0 {
        assert!(start.elapsed() < Duration::from_secs(10), "no request arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
    pusher.join().unwrap();
    reader.join().unwrap();
}
