//! S8's exit criteria, as tests.
//!
//! > a socket client and the terminal client share one API with no
//! > divergence; a runaway form is interruptible; a backtrace names frames
//! > and spans
//!
//! The first clause is the one that needed a design rather than a feature,
//! and the way it is tested here is the way it is *enforced*: there is only
//! [`Session::handle`], so a client has nothing to diverge in. What can still
//! diverge is the **wire format** — a response that loses information on the
//! way out would be a divergence in practice — so the tests below put every
//! outcome through the wire and back.

use scm::{Framing, Op, Outcome, Request, Response, Session};

fn ask(s: &mut Session, op: Op) -> Outcome {
    s.handle(Request { id: 7, op }).outcome
}

fn eval(s: &mut Session, code: &str) -> Outcome {
    ask(s, Op::Eval { code: code.into(), source: None })
}

#[test]
fn the_environment_persists_across_requests() {
    // §8 obligation 1: incremental evaluation against a persistent
    // interactive environment. Each request is a separate call; the state is
    // not.
    let mut s = Session::new().unwrap();
    eval(&mut s, "(define counter 0)");
    for _ in 0..3 {
        eval(&mut s, "(set! counter (+ counter 1))");
    }
    assert!(matches!(
        eval(&mut s, "counter"),
        Outcome::Value { ref value, .. } if value == "3"
    ));
}

#[test]
fn redefinition_takes_effect_in_existing_closures() {
    // §8 obligation 2, and the one the plan calls "cheap to design in,
    // expensive to retrofit". A global is a mutable CELL, never a value
    // inlined at the point of use -- so a closure that captured `helper`
    // calls the NEW helper, which is the whole of what makes a SLIME-style
    // session useful.
    let mut s = Session::new().unwrap();
    eval(&mut s, "(define (helper x) (* x 2))");
    eval(&mut s, "(define (outer x) (helper (+ x 1)))");
    assert!(matches!(
        eval(&mut s, "(outer 4)"),
        Outcome::Value { ref value, .. } if value == "10"
    ));

    // Redefine the callee only.
    eval(&mut s, "(define (helper x) (* x 100))");
    assert!(
        matches!(eval(&mut s, "(outer 4)"), Outcome::Value { ref value, .. } if value == "500"),
        "a redefined procedure must be seen by callers defined earlier"
    );

    // And through a value captured in a closure's environment, not just a
    // global reference.
    eval(&mut s, "(define make-adder (lambda (n) (lambda (x) (helper (+ x n)))))");
    eval(&mut s, "(define add3 (make-adder 3))");
    eval(&mut s, "(define (helper x) (- x))");
    assert!(
        matches!(eval(&mut s, "(add3 1)"), Outcome::Value { ref value, .. } if value == "-4"),
        "a live closure must see the redefinition too"
    );
}

#[test]
fn output_comes_back_with_the_value() {
    // A socket client cannot see the server's stdout, so what a form printed
    // has to travel in the answer.
    let mut s = Session::new().unwrap();
    match eval(&mut s, r#"(begin (display "one") (newline) (display 2) 'done)"#) {
        Outcome::Value { value, output } => {
            assert_eq!(value, "done");
            assert_eq!(output, "one\n2");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn output_survives_a_failure() {
    // What a form printed before it broke is usually the most useful part.
    let mut s = Session::new().unwrap();
    match eval(&mut s, r#"(begin (display "got this far") (car '()))"#) {
        Outcome::Failed {
            message,
            output,
            host_stopped,
            ..
        } => {
            assert!(message.contains("car"), "{message}");
            assert_eq!(output, "got this far");
            assert!(!host_stopped, "a script error is not the host stopping");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn capturing_output_does_not_change_top_level_semantics() {
    // The trap this avoided: capturing by wrapping the source in
    // `(parameterize (…) (lambda () …))` would put a top-level `define`
    // inside a lambda, where it becomes a LOCAL and silently vanishes.
    // Rebinding the parameter slot from Rust instead keeps the form at top
    // level.
    let mut s = Session::new().unwrap();
    eval(&mut s, "(define top-level-thing 99)");
    assert!(matches!(
        eval(&mut s, "top-level-thing"),
        Outcome::Value { ref value, .. } if value == "99"
    ));
    // And the redirection is undone afterwards: a later form's output is
    // captured freshly, not appended to the last one's.
    eval(&mut s, r#"(display "first")"#);
    match eval(&mut s, r#"(display "second")"#) {
        Outcome::Value { output, .. } => assert_eq!(output, "second"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_host_stopping_is_distinguishable_from_the_script_failing() {
    // A client should not offer "retry" for a fuel exhaustion the way it
    // might for a typo, so the difference is in the protocol and not only in
    // the message.
    let mut s = Session::new().unwrap();
    s.interp_mut().set_fuel(Some(20_000));
    match eval(&mut s, "(let loop () (loop))") {
        Outcome::Failed { host_stopped, .. } => assert!(host_stopped),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_runaway_form_is_interruptible() {
    // The exit criterion's second clause. The flag is an `Arc<AtomicBool>`,
    // so it is reachable from a thread that is NOT waiting on the session --
    // which is why `:interrupt` never queues behind the form it stops.
    let mut s = Session::new().unwrap();
    let flag = s.interrupt_handle();
    let t = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(60));
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    match eval(&mut s, "(let loop () (loop))") {
        Outcome::Failed {
            message,
            host_stopped,
            ..
        } => {
            assert!(host_stopped);
            assert!(message.contains("interrupt"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    t.join().unwrap();
    // And the session is still usable, which is the difference between an
    // interrupt and a crash.
    assert!(matches!(
        eval(&mut s, "(* 6 7)"),
        Outcome::Value { ref value, .. } if value == "42"
    ));
}

#[test]
fn framing_is_the_readers_answer_not_a_second_one() {
    // §8 obligation 5. The prompt's continuation and an editor pane's RET ask
    // this same question over this same door, so they cannot disagree about
    // whether a form is finished.
    let mut s = Session::new().unwrap();
    for (code, want) in [
        ("(+ 1 2)", Framing::Complete),
        ("(+ 1", Framing::Incomplete),
        ("\"unterminated", Framing::Incomplete),
        ("#| open", Framing::Incomplete),
        ("(1 . 2 . 3)", Framing::Invalid),
        (")", Framing::Invalid),
    ] {
        assert_eq!(
            ask(&mut s, Op::Parse { code: code.into() }),
            Outcome::Framed(want),
            "{code:?}"
        );
    }
}

#[test]
fn introspection_answers_what_a_name_is() {
    // §8 obligation 6: docstrings as VALUES. ray generates its manual from
    // live docstrings, so this is a requirement and not a nicety.
    let mut s = Session::new().unwrap();
    eval(
        &mut s,
        r#"(define (area r &named units) "The area of a circle." (* 3 r r))"#,
    );
    match ask(&mut s, Op::Describe { name: "area".into() }) {
        Outcome::Described(Some(d)) => {
            assert_eq!(d.kind, "procedure");
            assert_eq!(d.doc.as_deref(), Some("The area of a circle."));
            // The spelling a Scheme or Racket reader knows, not the marker
            // (S21b): `:units units`, which is also legal input.
            assert_eq!(d.signature.as_deref(), Some("(area r :units units)"));
        }
        other => panic!("{other:?}"),
    }
    // An unbound name is "not bound", not an error.
    assert_eq!(
        ask(&mut s, Op::Describe { name: "no-such-name".into() }),
        Outcome::Described(None)
    );
    // A leading string is only a docstring when something FOLLOWS it, or
    // `(define (greeting) "hello")` would lose its value to the manual.
    eval(&mut s, r#"(define (greeting) "hello")"#);
    assert!(matches!(
        eval(&mut s, "(greeting)"),
        Outcome::Value { ref value, .. } if value == "\"hello\""
    ));
    match ask(&mut s, Op::Describe { name: "greeting".into() }) {
        Outcome::Described(Some(d)) => assert_eq!(d.doc, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn completion_hides_hygienes_renamed_identifiers() {
    // A macro's template identifiers are renamed to names no source text can
    // write. Offering `tmp.1274` as a completion would be leaking the
    // implementation into the user's face.
    let mut s = Session::new().unwrap();
    eval(
        &mut s,
        "(define-syntax swap!
           (syntax-rules () ((_ a b) (let ([tmp a]) (set! a b) (set! b tmp)))))
         (define p 1) (define q 2) (swap! p q)",
    );
    match ask(&mut s, Op::Complete { prefix: "".into() }) {
        Outcome::Candidates(names) => {
            assert!(names.iter().any(|n| n == "vector-ref"), "real names are offered");
            let leaked: Vec<&String> = names
                .iter()
                .filter(|n| n.contains('.') && n.parse::<f64>().is_err())
                .collect();
            assert!(leaked.is_empty(), "hygiene leaked into completion: {leaked:?}");
        }
        other => panic!("{other:?}"),
    }
}

// ---- the wire format, which is where a divergence could still hide ------

/// Round-trip a request through its text form, the way a socket client does.
fn over_the_wire(s: &mut Session, text: &str) -> String {
    let req = Request::parse(text).unwrap_or_else(|e| panic!("parse {text:?}: {e}"));
    s.handle(req).to_wire()
}

#[test]
fn the_wire_carries_every_outcome_without_loss() {
    let mut s = Session::new().unwrap();
    assert!(over_the_wire(&mut s, "(:op ping :id 1)").contains(":status ok :version"));

    let w = over_the_wire(
        &mut s,
        r#"(:op eval :id 2 :code "(begin (display \"out\") 42)")"#,
    );
    assert_eq!(w, r#"(:id 2 :status ok :value "42" :output "out")"#);

    let w = over_the_wire(&mut s, r#"(:op eval :id 3 :code "(car 5)")"#);
    assert!(w.contains(":status error") && w.contains("not a pair") && w.contains(":host-stopped #f"));

    let w = over_the_wire(&mut s, r#"(:op parse :id 4 :code "(+ 1")"#);
    assert_eq!(w, "(:id 4 :status ok :framing incomplete)");

    let w = over_the_wire(&mut s, r#"(:op complete :id 5 :prefix "vector-r")"#);
    assert_eq!(w, r#"(:id 5 :status ok :candidates ("vector-ref"))"#);

    let w = over_the_wire(&mut s, "(:op describe :id 6 :name car)");
    assert!(w.contains(":bound #t") && w.contains(r#":kind "primitive""#));
}

#[test]
fn a_symbol_and_a_string_both_name_an_op() {
    // An Emacs client writes `:op eval`, a Python one writes `:op "eval"`.
    // Refusing either would be a protocol that favours one client.
    assert_eq!(
        Request::parse("(:op ping :id 1)").unwrap(),
        Request::parse(r#"(:op "ping" :id 1)"#).unwrap()
    );
    assert_eq!(
        Request::parse("(:op describe :id 1 :name car)").unwrap(),
        Request::parse(r#"(:op describe :id 1 :name "car")"#).unwrap()
    );
}

#[test]
fn an_unknown_key_is_ignored_so_a_newer_client_still_works() {
    let r = Request::parse(r#"(:op ping :id 3 :client "emacs" :protocol 2)"#).unwrap();
    assert_eq!(r.id, 3);
    assert_eq!(r.op, Op::Ping);
}

#[test]
fn a_malformed_request_is_answered_not_fatal() {
    let mut s = Session::new().unwrap();
    for bad in [
        "(:op frobnicate :id 1)",
        "(:op eval :id 1)",
        "(:op :id 1)",
        "(eval 1)",
        "(:op)",
        "42",
        "",
    ] {
        match Request::parse(bad) {
            Err(why) => assert!(!why.is_empty(), "{bad:?} must say what was wrong"),
            // If it parses, it must at least answer rather than panic.
            Ok(req) => {
                let _ = s.handle(req);
            }
        }
    }
    // And the session is unharmed.
    assert!(matches!(
        eval(&mut s, "(+ 1 1)"),
        Outcome::Value { ref value, .. } if value == "2"
    ));
}

#[test]
fn a_response_is_its_own_frame() {
    // The framing rule in both directions: a response is ONE datum, so a
    // client reads until `parse_status` says complete. That has to hold even
    // when the payload contains parens, quotes and newlines.
    let mut s = Session::new().unwrap();
    let w = over_the_wire(
        &mut s,
        r#"(:op eval :id 1 :code "(begin (display \"a)(b \\\"c\\\"\n\") '(1 2))")"#,
    );
    assert!(
        matches!(scm::parse_status(&w), scm::ParseStatus::Complete),
        "not a single datum: {w}"
    );
    let parsed = Request::parse(&w);
    // It reads as a datum; it is not a *request*, which is fine -- the point
    // is that a client can find its boundary.
    assert!(parsed.is_err() || parsed.is_ok());
    assert!(w.contains(r#":value "(1 2)""#), "{w}");
}

#[test]
fn the_terminal_and_wire_paths_cannot_diverge() {
    // The exit criterion's first clause. Both clients build an `Op` and call
    // `handle`; the only thing unique to the socket is the text form. So:
    // for every op, calling `handle` directly and going through the wire must
    // agree.
    let mut direct = Session::new().unwrap();
    let mut wired = Session::new().unwrap();
    let cases = [
        ("(:op ping :id 1)", Op::Ping),
        (
            r#"(:op eval :id 1 :code "(define (f x) (* x 3))")"#,
            Op::Eval { code: "(define (f x) (* x 3))".into(), source: None },
        ),
        (r#"(:op eval :id 1 :code "(f 5)")"#, Op::Eval { code: "(f 5)".into(), source: None }),
        (
            r#"(:op eval :id 1 :code "(car 1)")"#,
            Op::Eval { code: "(car 1)".into(), source: None },
        ),
        (
            r#"(:op complete :id 1 :prefix "str")"#,
            Op::Complete { prefix: "str".into() },
        ),
        ("(:op describe :id 1 :name f)", Op::Describe { name: "f".into() }),
        (
            r#"(:op parse :id 1 :code "(a (b")"#,
            Op::Parse { code: "(a (b".into() },
        ),
    ];
    for (text, op) in cases {
        let from_wire = Request::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e}"));
        assert_eq!(from_wire.op, op, "the wire produced a different op for {text:?}");
        let a: Response = direct.handle(Request { id: 1, op });
        let b: Response = wired.handle(from_wire);
        assert_eq!(a.outcome, b.outcome, "divergence on {text:?}");
    }
}

// ---- the backtrace: §8 obligation 4, the last exit criterion ------------

fn failed(s: &mut Session, code: &str) -> (String, Vec<String>) {
    match s.handle(Request {
        id: 1,
        op: Op::Eval {
            code: code.into(),
            source: Some("<t>".into()),
        },
    }).outcome
    {
        Outcome::Failed { message, backtrace, .. } => (message, backtrace),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn a_backtrace_names_frames_and_spans() {
    // The exit criterion's third clause. Every line is a pending call, named
    // and placed, innermost first.
    let mut s = Session::new().unwrap();
    let (msg, bt) = failed(
        &mut s,
        "(define (inner v) (+ 0 (vector-ref v 99)))\n\
         (define (middle v) (* 1 (inner v)))\n\
         (define (outer) (- 0 (middle (vector 1 2 3))))\n\
         (outer)",
    );
    assert!(msg.contains("out of range"), "{msg}");
    assert!(bt.len() >= 4, "too few frames: {bt:#?}");
    assert!(bt[0].starts_with("(vector-ref v 99)"), "{:?}", bt[0]);
    assert!(bt[1].starts_with("(+ 0 (vector-ref v 99))"), "{:?}", bt[1]);
    assert!(bt[2].starts_with("(* 1 (inner v))"), "{:?}", bt[2]);
    // And every one of them carries a location: the source name, a line and
    // a column.
    for f in &bt {
        assert!(f.contains("at <t>:"), "no location on {f:?}");
    }
    // Lines are the ones a person would point at.
    assert!(bt[1].contains("<t>:1:19"), "{:?}", bt[1]);
    assert!(bt[2].contains("<t>:2:20"), "{:?}", bt[2]);
}

#[test]
fn a_tail_call_leaves_no_frame_and_the_backtrace_says_where_anyway() {
    // §5's whole point is that a call in tail position leaves the stack as
    // deep as it found it -- so a chain of tail calls fails with an EMPTY
    // stack, and a stack-only backtrace would say nothing at all. Correct
    // Scheme, useless diagnostic. The innermost call is tracked separately
    // for exactly this case.
    let mut s = Session::new().unwrap();
    let (_, bt) = failed(
        &mut s,
        "(define (inner v) (vector-ref v 99))\n\
         (define (middle v) (inner v))\n\
         (define (outer) (middle (vector 1 2 3)))\n\
         (outer)",
    );
    assert_eq!(bt.len(), 1, "a tail chain should leave exactly one line: {bt:#?}");
    assert!(bt[0].starts_with("(vector-ref v 99)"), "{:?}", bt[0]);
    assert!(bt[0].contains("at <t>:1:19"), "{:?}", bt[0]);
}

#[test]
fn a_deep_recursion_does_not_bury_the_useful_frames() {
    // A non-tail recursion 5,000 deep would otherwise print 5,000 identical
    // lines and hide the message.
    let mut s = Session::new().unwrap();
    let (_, bt) = failed(
        &mut s,
        "(define (down n) (if (= n 0) (car '()) (+ 0 (down (- n 1)))))\n(down 5000)",
    );
    assert!(bt.len() <= 34, "backtrace not bounded: {} lines", bt.len());
    assert_eq!(bt.last().map(String::as_str), Some("…"));
}

#[test]
fn a_form_built_at_run_time_has_no_location_rather_than_a_wrong_one() {
    // A form a macro or `quasiquote` produced was never written anywhere.
    // Pointing at the macro's definition as if the user had typed it there
    // would be worse than saying nothing.
    let mut s = Session::new().unwrap();
    let (_, bt) = failed(&mut s, "(eval (list 'car (list 'quote '())))");
    // The form is still named -- knowing WHAT failed is most of the value --
    // but it carries no location, because it has none.
    assert_eq!(bt.len(), 1, "{bt:#?}");
    assert_eq!(bt[0], "(car '())", "the writer abbreviates quote: {bt:#?}");
    assert!(!bt[0].contains(" at "), "a form built at run time claimed a place");

    // And a located caller still places itself, so a run-time form nested
    // inside real source does not erase the trail.
    let (_, bt) = failed(
        &mut s,
        "(define (viaeval f) (+ 0 (eval f)))\n(viaeval (list 'car (list 'quote '())))",
    );
    assert!(bt.iter().any(|f| f.contains("at <t>:1:21")), "{bt:#?}");
}

#[test]
fn the_backtrace_is_fresh_for_each_failure() {
    let mut s = Session::new().unwrap();
    let (_, first) = failed(&mut s, "(define (a) (+ 0 (car '())))\n(a)");
    let (_, second) = failed(&mut s, "(define (b) (* 1 (vector-ref (vector) 0)))\n(b)");
    assert!(first[0].contains("car"), "{first:#?}");
    assert!(second[0].contains("vector-ref"), "{second:#?}");
    assert!(!second.iter().any(|f| f.contains("car")), "stale frames: {second:#?}");
}

#[test]
fn a_caught_error_does_not_leave_a_backtrace_behind_for_the_next_one() {
    // `guard` catches, the frames unwind, and a later unrelated failure must
    // not inherit them.
    let mut s = Session::new().unwrap();
    let swallowed = s.handle(Request {
        id: 1,
        op: Op::Eval {
            code: "(guard (e (else 'fine)) (car '()))".into(),
            source: Some("<t>".into()),
        },
    });
    assert!(matches!(swallowed.outcome, Outcome::Value { .. }));
    let (_, bt) = failed(&mut s, "(+ 0 (vector-ref (vector) 0))");
    assert!(bt[0].contains("vector-ref"), "{bt:#?}");
}

#[test]
fn the_wire_carries_the_backtrace() {
    let mut s = Session::new().unwrap();
    let req = Request::parse(
        r#"(:op eval :id 9 :name "mybuffer.scm" :code "(+ 0 (car '()))")"#,
    )
    .unwrap();
    let w = s.handle(req).to_wire();
    assert!(w.contains(":backtrace ("), "{w}");
    assert!(w.contains("mybuffer.scm"), "the client's name must be used: {w}");
    assert!(
        matches!(scm::parse_status(&w), scm::ParseStatus::Complete),
        "a response with a backtrace must still be one datum: {w}"
    );
}

#[test]
fn locations_do_not_accumulate_forever() {
    // There is one location per form, so the table is the size of the
    // *program*, not of the execution -- but a REPL session evaluates new
    // programs forever, so the collector has to prune the entries for forms
    // it freed. Without that, this grows without bound.
    let mut s = Session::new().unwrap();
    let mut peak = 0usize;
    for i in 0..400 {
        let code = format!("(let ([x {i}]) (+ x (* x 2) (- x 1)))");
        s.handle(Request {
            id: i,
            op: Op::Eval {
                code,
                source: Some(format!("<t:{i}>")),
            },
        });
        peak = peak.max(s.interp_mut().loc_count());
    }
    let settled = s.interp_mut().loc_count();
    assert!(
        settled < peak || peak < 4_000,
        "locations are not pruned: peak {peak}, settled {settled}"
    );
}

#[test]
fn stopping_a_runaway_says_where_it_was() {
    // The host stopping a script used to return the PREVIOUS failure's
    // backtrace, because `safepoint`'s `?` returned before the capture ran.
    // A client was shown frames from an unrelated error, which is worse than
    // being shown none. Found by driving the protocol over a socket and
    // reading the replies rather than by a test.
    //
    // And it is the right thing to want: "where was it when I stopped it" is
    // exactly the question C-c asks.
    let mut s = Session::new().unwrap();

    // First, an unrelated failure, to leave a backtrace lying around.
    let (_, stale) = failed(&mut s, "(+ 0 (vector-ref (vector) 9))");
    assert!(stale[0].contains("vector-ref"), "{stale:#?}");

    // Now a runaway, stopped by the host.
    s.interp_mut().set_fuel(Some(40_000));
    let (msg, bt) = failed(
        &mut s,
        "(define (inner) (let loop () (loop)))\n\
         (define (mid) (+ 0 (inner)))\n\
         (define (outer) (* 1 (mid)))\n\
         (outer)",
    );
    assert!(msg.contains("fuel"), "{msg}");
    assert!(!bt.is_empty(), "a stopped runaway must say where it was");
    assert!(
        !bt.iter().any(|f| f.contains("vector-ref")),
        "the previous failure's frames leaked: {bt:#?}"
    );
    // The innermost line is the loop it was actually spinning in, which a
    // stack-only trace could not give: every call in that chain is a tail
    // call.
    assert!(bt[0].starts_with("(loop)"), "{bt:#?}");
    assert!(bt.iter().any(|f| f.contains("(+ 0 (inner))")), "{bt:#?}");
}

// ── S8b: the break REPL, and the REPL's conveniences ─────────────────────

#[test]
fn a_break_handler_sees_live_frames_and_can_return_a_value() {
    use scm::{BreakAction, Value, Vm};
    let mut vm = Vm::new();
    vm.eval_str("(define (f x) (* 2 (vector-ref (vector 1 2) x)))").unwrap();
    let seen: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let log = seen.clone();
    vm.set_break_handler(Some(Box::new(move |vm, e| {
        log.borrow_mut().push(format!("{e}"));
        let frames = vm.live_frames();
        log.borrow_mut().push(frames.iter().map(|f| f.what.clone()).collect::<Vec<_>>().join(" | "));
        let env = frames[0].env;
        let bindings = vm.bindings_of(env);
        log.borrow_mut().push(format!("{:?}", bindings.iter().map(|(n, v)| format!("{n}={v:?}")).collect::<Vec<_>>()));
        // Evaluate in the frame, with the stack below still live.
        let v = vm.eval_in("(+ x 100)", env).unwrap();
        log.borrow_mut().push(format!("{v:?}"));
        // An error inside the handler does not re-break: it just fails.
        assert!(vm.eval_in("(car 1)", env).is_err());
        BreakAction::Return(Value::Fixnum(21))
    })));
    let v = vm.eval_str("(f 5)").unwrap();
    assert_eq!(v, Value::Fixnum(42), "the failed expression became 21, and (* 2 21) went on");
    let seen = seen.borrow();
    assert!(seen[0].contains("out of range"), "{}", seen[0]);
    assert!(seen[1].starts_with("(vector-ref (vector 1 2) x) | (* 2 (vector-ref"), "{}", seen[1]);
    assert_eq!(seen[2], r#"["x=Fixnum(5)"]"#);
    assert_eq!(seen[3], "Fixnum(105)");
    // Abort unwinds as if there were no handler; a guarded error never breaks.
    vm.set_break_handler(Some(Box::new(|_, _| panic!("a guarded error must not break"))));
    assert_eq!(vm.eval_str("(guard (e (#t 'caught)) (f 9))").unwrap(), vm.engine.sym("caught"));
    vm.set_break_handler(Some(Box::new(|_, _| BreakAction::Abort)));
    assert!(vm.eval_str("(f 9)").is_err());
    assert_eq!(vm.eval_str("(f 0)").unwrap(), Value::Fixnum(2), "the session is intact afterwards");
}

#[test]
fn load_macroexpand_values_and_pretty_results() {
    let mut s = Session::new().unwrap();
    let dir = std::env::temp_dir().join(format!("scm-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.scm");
    std::fs::write(&file, "(define (triple x) (* 3 x))\n(define loaded 'yes)\n").unwrap();
    let code = format!("(load {:?}) (list (triple 4) loaded)", file.to_str().unwrap());
    match s.handle(Request { id: 1, op: Op::Eval { code, source: None } }).outcome {
        Outcome::Value { value, .. } => assert_eq!(value, "(12 yes)"),
        other => panic!("{other:?}"),
    }
    // An error inside a loaded file names the file.
    std::fs::write(&file, "(define a 1)\n(car a)\n").unwrap();
    let code = format!("(load {:?})", file.to_str().unwrap());
    match s.handle(Request { id: 2, op: Op::Eval { code, source: None } }).outcome {
        Outcome::Failed { message, backtrace, .. } => {
            assert!(message.contains("not a pair"), "{message}");
            assert!(backtrace.iter().any(|f| f.contains("lib.scm:2")), "{backtrace:?}");
        }
        other => panic!("{other:?}"),
    }
    let shown = |s: &mut Session, code: &str| match s.handle(Request { id: 3, op: Op::Eval { code: code.into(), source: None } }).outcome {
        Outcome::Value { value, .. } => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(shown(&mut s, "(macroexpand-1 '(when #t 1 2))"), "(if #t (begin 1 2) (if #f #f))");
    assert_eq!(shown(&mut s, "(macroexpand '(unless #f 1))"), "(if #f (if #f #f) (begin 1))");
    assert_eq!(shown(&mut s, "(macroexpand '(+ 1 2))"), "(+ 1 2)");
    // Multiple values on separate lines; a long result broken over lines.
    assert_eq!(shown(&mut s, "(values 1 'two \"three\")"), "1\ntwo\n\"three\"");
    let long = shown(&mut s, "(let loop ([i 0] [acc '()]) (if (= i 12) acc (loop (+ i 1) (cons (list i 'abcdefghij (* i i)) acc))))");
    assert!(long.lines().count() == 12, "{long}");
    assert!(long.starts_with("((11 abcdefghij 121)\n  (10 abcdefghij 100)"), "{long}");
    assert!(long.lines().all(|l| l.len() <= 80));
    let _ = std::fs::remove_dir_all(&dir);
}

// --- S22: the protocol in both directions ----------------------------------
//
// S8 tested the wire one way, because only one way existed: a response was
// formatted by the server and read by Emacs. A Rust client needs the other
// half, and the risk it introduces is the one §8 names -- two clients
// formatting the same request separately are two clients that can disagree.
// So the tests are round trips: whatever `to_wire` says, `parse` must read
// back as the value it started from, for every variant.

#[test]
fn a_request_round_trips_through_the_wire() {
    let requests = [
        Request { id: 1, op: Op::Ping },
        Request { id: 2, op: Op::Interrupt },
        Request { id: 3, op: Op::Eval { code: "(+ 1 2)".into(), source: None } },
        Request {
            id: 4,
            op: Op::Eval { code: "(+ 1 2)".into(), source: Some("scene.scm".into()) },
        },
        Request { id: 5, op: Op::Complete { prefix: "vector-".into() } },
        Request { id: 6, op: Op::Describe { name: "car".into() } },
        Request { id: 7, op: Op::Parse { code: "(+ 1".into() } },
        // The payload that would break a naive formatter: a quote, a
        // backslash, a newline and unbalanced parens inside a string.
        Request {
            id: 8,
            op: Op::Eval { code: "(display \"a)(b \\\"c\\\"\n\")".into(), source: None },
        },
    ];
    for req in requests {
        let wire = req.to_wire();
        assert!(
            matches!(scm::parse_status(&wire), scm::ParseStatus::Complete),
            "not a single datum: {wire}"
        );
        assert_eq!(Request::parse(&wire).unwrap(), req, "{wire}");
    }
}

#[test]
fn a_response_round_trips_through_the_wire() {
    // Every outcome the server can produce, including the ones whose shape
    // is decided by which keys are present rather than by a tag.
    let described = scm::Description {
        name: "area".into(),
        signature: Some("(area r)".into()),
        doc: Some("Circle area.".into()),
        kind: "procedure".into(),
        value: "#<procedure area>".into(),
    };
    let responses = [
        Response { id: 1, outcome: Outcome::Pong { version: "0.0.1".into() } },
        Response {
            id: 2,
            outcome: Outcome::Value { value: "42".into(), output: "out\n".into() },
        },
        Response {
            id: 3,
            outcome: Outcome::Failed {
                message: "car: not a pair: 5".into(),
                output: "printed first".into(),
                host_stopped: false,
                backtrace: vec!["(car 5)  at s.scm:1:1".into()],
            },
        },
        // An interrupt, which a client must not offer to retry.
        Response {
            id: 4,
            outcome: Outcome::Failed {
                message: "interrupted".into(),
                output: String::new(),
                host_stopped: true,
                backtrace: vec![],
            },
        },
        Response { id: 5, outcome: Outcome::Candidates(vec!["vector-ref".into()]) },
        // Empty is not the same as absent: no candidates still reads back
        // as `Candidates`, not as a value.
        Response { id: 6, outcome: Outcome::Candidates(vec![]) },
        Response { id: 7, outcome: Outcome::Described(Some(described)) },
        Response { id: 8, outcome: Outcome::Described(None) },
        Response { id: 9, outcome: Outcome::Framed(Framing::Complete) },
        Response { id: 10, outcome: Outcome::Framed(Framing::Incomplete) },
        Response { id: 11, outcome: Outcome::Framed(Framing::Invalid) },
        Response { id: 12, outcome: Outcome::Malformed("unknown op `frobnicate`".into()) },
    ];
    for res in responses {
        let wire = res.to_wire();
        assert!(
            matches!(scm::parse_status(&wire), scm::ParseStatus::Complete),
            "not a single datum: {wire}"
        );
        assert_eq!(Response::parse(&wire).unwrap(), res, "{wire}");
    }
}

#[test]
fn a_described_value_without_a_signature_or_doc_round_trips() {
    // Both are optional, and the wire omits them rather than sending "".
    // Reading `""` back as `Some("")` would be a quiet divergence.
    let res = Response {
        id: 1,
        outcome: Outcome::Described(Some(scm::Description {
            name: "x".into(),
            signature: None,
            doc: None,
            kind: "variable".into(),
            value: "7".into(),
        })),
    };
    let wire = res.to_wire();
    assert!(!wire.contains(":signature") && !wire.contains(":doc"), "{wire}");
    assert_eq!(Response::parse(&wire).unwrap(), res);
}

#[test]
fn a_real_response_reads_back_into_the_same_outcome() {
    // The round trips above build responses by hand. This one takes what the
    // session actually answered -- so a new field that `to_wire` learns and
    // `parse` does not would fail here even if nobody updated the list.
    let mut s = Session::new().unwrap();
    eval(&mut s, "(define (area r) \"Circle area.\" (* 3 r r))");
    for op in [
        Op::Ping,
        Op::Eval { code: "(begin (display \"o\") (area 2))".into(), source: None },
        Op::Eval { code: "(car 5)".into(), source: Some("t.scm".into()) },
        Op::Describe { name: "area".into() },
        Op::Describe { name: "nope".into() },
        Op::Complete { prefix: "vector-r".into() },
        Op::Parse { code: "(+ 1".into() },
    ] {
        let answer = s.handle(Request { id: 1, op: op.clone() });
        let back = Response::parse(&answer.to_wire())
            .unwrap_or_else(|e| panic!("{op:?} -> {e}"));
        assert_eq!(back, answer, "{op:?}");
    }
}

#[test]
fn an_unknown_key_in_a_response_is_ignored_so_an_older_server_still_works() {
    // The mirror of `an_unknown_key_is_ignored_so_a_newer_client_still_works`.
    // A client that refused what it did not recognise would make every
    // protocol addition a flag day.
    let r = Response::parse(r#"(:id 1 :status ok :value "42" :output "" :elapsed-ms 3)"#).unwrap();
    assert_eq!(r.outcome, Outcome::Value { value: "42".into(), output: String::new() });
}

#[test]
fn a_malformed_response_is_an_error_not_a_panic() {
    // A client talks to whatever is on the port, which may not be scm at all.
    for text in [
        "",
        "not a datum",
        "(:id 1)",                                   // no :status
        "(:id 1 :status sideways)",                  // not a status
        "(:id 1 :status ok :framing maybe)",         // not a framing
        "(:id 1 :status error)",                     // an error with no message
        "(:id 1 :status ok :bound #t :value \"7\")", // bound with no kind
        "(:op eval :id 1 :code \"(+ 1 2)\")",        // a request, not a response
    ] {
        assert!(Response::parse(text).is_err(), "accepted {text:?}");
    }
}
