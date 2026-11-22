//! `scm::serve`: the REPL server for a host with a loop of its own.
//!
//! lux draws a frame, answers what has arrived, draws the next: the session
//! never blocks waiting for a client. `scm --serve`'s own tests and the
//! Emacs check drive `serve_forever`; these drive `serve_pending`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use scm::{Op, Server, Session};

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
