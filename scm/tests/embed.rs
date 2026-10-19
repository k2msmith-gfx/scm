//! S7's exit criteria, as tests.
//!
//! The stage exists because three things were measured first (plan §7a): a
//! host-held value silently became a different object after a collection, a
//! host could not register a closure at all, and there were 88 panic sites
//! behind 95 public items. The first and third are here; the second is S7b.

use std::sync::atomic::Ordering;
use std::time::Duration;

use scm::{Interp, Proc, Val};
use scm::embed::Error;

#[test]
fn a_held_value_survives_collection() {
    // THE bug this stage exists for. Before `Rooted`, this printed
    // `#<environment>` on the second line: `eval_str` returned a bare Value,
    // the slot was reused, and `ObjRef` is a raw index with no generation.
    let mut it = Interp::new().unwrap();
    let held = it.eval("(list 1 2 3)").unwrap();
    assert_eq!(it.write(&held).unwrap(), "(1 2 3)");

    // Enough allocation to collect several times over.
    it.eval("(let loop ([i 0]) (if (= i 200000) i (loop (+ i 1))))")
        .unwrap();

    assert_eq!(it.write(&held).unwrap(), "(1 2 3)");
    assert_eq!(
        it.value(&held).unwrap(),
        Val::List(vec![Val::Int(1), Val::Int(2), Val::Int(3)])
    );
}

#[test]
fn dropping_a_handle_releases_it() {
    // `Drop` cannot reach the engine, so a dropped handle is queued and
    // released at the next entry point. If the queue were never drained this
    // would be a leak of one root per value the host ever saw.
    let mut it = Interp::new().unwrap();
    let before = it.rooted_count();
    {
        let _a = it.eval("(list 1)").unwrap();
        let _b = it.eval("(list 2)").unwrap();
        assert_eq!(it.rooted_count(), before + 2);
    }
    // The release happens on entry, not on drop.
    let _ = it.eval("1").unwrap();
    assert!(
        it.rooted_count() <= before + 1,
        "handles leaked: {} vs {}",
        it.rooted_count(),
        before
    );
}

#[test]
fn a_handle_from_another_interpreter_is_refused() {
    // Both of these are indices into a per-interpreter table, so without the
    // owner check a foreign handle would silently name *this* interpreter's
    // value at the same index -- a wrong answer rather than an error.
    let mut a = Interp::new().unwrap();
    let mut b = Interp::new().unwrap();
    let from_a = a.eval("'alpha").unwrap();
    let from_b = b.eval("'beta").unwrap();
    assert_eq!(b.write(&from_a), Err(Error::ForeignHandle));
    assert_eq!(a.write(&from_b), Err(Error::ForeignHandle));
    // And each still works with its own.
    assert_eq!(a.write(&from_a).unwrap(), "alpha");
    assert_eq!(b.write(&from_b).unwrap(), "beta");
}

#[test]
fn errors_arrive_as_results_not_panics() {
    let mut it = Interp::new().unwrap();
    assert!(matches!(it.eval("(car '())"), Err(Error::Script(_))));
    assert!(matches!(it.eval("(undefined-thing)"), Err(Error::Script(_))));
    assert!(matches!(it.eval("(+ 1"), Err(Error::Incomplete)));
    assert!(matches!(it.eval("(1 . 2 . 3)"), Err(Error::Syntax(_))));
    // And the interpreter is still usable after each refusal.
    let v = it_eval(&mut it, "(+ 1 2)");
    assert_eq!(it.write(&v).unwrap(), "3");
}

#[test]
fn a_raised_object_reaches_the_host_as_a_value() {
    // Not just its printed form: a host wants to inspect what was raised.
    let mut it = Interp::new().unwrap();
    match it.eval("(raise (list 'bad 42))") {
        Err(Error::Raised { value, .. }) => {
            assert_eq!(
                it.value(&value).unwrap(),
                Val::List(vec![Val::Sym("bad".into()), Val::Int(42)])
            );
        }
        other => panic!("expected a raise, got {other:?}"),
    }
}

#[test]
fn a_runaway_script_is_killable_from_the_host() {
    // Requirement 8, all three ways. None of them is catchable by the script,
    // which is the point -- `(guard (e (else …)) …)` must not swallow the
    // host saying stop.
    let mut it = Interp::new().unwrap();
    it.set_fuel(Some(50_000));
    assert_eq!(it.eval("(let loop () (loop))"), Err(Error::OutOfFuel));

    let mut it = Interp::new().unwrap();
    it.set_fuel(Some(50_000));
    assert_eq!(
        it.eval("(guard (e (else 'caught)) (let loop () (loop)))"),
        Err(Error::OutOfFuel),
        "out-of-fuel must not be catchable"
    );

    let mut it = Interp::new().unwrap();
    it.set_heap_cap(Some(100_000));
    assert_eq!(
        it.eval("(let loop ([acc '()]) (loop (cons 1 acc)))"),
        Err(Error::OutOfMemory)
    );
}

#[test]
fn another_thread_can_interrupt_a_running_script() {
    let mut it = Interp::new().unwrap();
    let flag = it.interrupt_handle();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        flag.store(true, Ordering::Relaxed);
    });
    assert_eq!(it.eval("(let loop () (loop))"), Err(Error::Interrupted));
    t.join().unwrap();
    // The interpreter survives being stopped.
    let v = it_eval(&mut it, "(* 6 7)");
    assert_eq!(it.write(&v).unwrap(), "42");
}

#[test]
fn a_wall_clock_timeout_stops_a_runaway() {
    let mut it = Interp::new().unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        it.eval_with_timeout("(let loop () (loop))", Duration::from_millis(80)),
        Err(Error::Interrupted)
    );
    assert!(started.elapsed() < Duration::from_secs(5), "the watchdog did not fire");
    // And a script that finishes in time is not disturbed by the watchdog.
    let v = it
        .eval_with_timeout("(+ 1 2)", Duration::from_millis(500))
        .unwrap();
    assert_eq!(it.write(&v).unwrap(), "3");
}

#[test]
fn host_values_go_in_and_come_back() {
    let mut it = Interp::new().unwrap();
    for v in [
        Val::Null,
        Val::Bool(true),
        Val::Int(-7),
        Val::Float(1.5),
        Val::Char('λ'),
        Val::Str("hi \"there\"".into()),
        Val::Sym("a-symbol".into()),
        // The §4 decision ray needs: a keyword is its own type, spelled :at.
        Val::Keyword("at".into()),
        Val::Bytes(vec![1, 2, 255]),
        Val::List(vec![Val::Int(1), Val::Keyword("k".into())]),
        Val::Vector(vec![Val::Float(0.5), Val::Str("s".into())]),
    ] {
        let r = it.make(&v).unwrap();
        assert_eq!(it.value(&r).unwrap(), v, "round trip of {v:?}");
    }
    // A keyword is NOT a symbol, in the host's view as in Scheme's.
    let k = it.make(&Val::Keyword("at".into())).unwrap();
    assert_eq!(it.write(&k).unwrap(), ":at");
    assert_ne!(it.value(&k).unwrap(), Val::Sym("at".into()));
}

#[test]
fn what_has_no_host_form_says_so_instead_of_guessing() {
    let mut it = Interp::new().unwrap();
    // A port and a cycle are Opaque rather than a lie or a hang. The cycle
    // is the one that matters: `set-cdr!` makes them constructible, and an
    // unbounded walk would never return. A procedure is the exception: it
    // comes back as a callable `Proc`.
    //
    // An improper list was in this list and is not any more — it crosses as
    // `Val::Pair`, which `an_improper_list_crosses_as_the_cons_chain_it_is`
    // covers. The cycle is still caught, by the length cap rather than by
    // the improper arm, because a cycle's tail is always another pair.
    let port = it.eval("(open-input-string \"x\")").unwrap();
    assert_eq!(it.value(&port).unwrap(), Val::Opaque);
    for src in ["car", "(lambda (x) x)"] {
        let r = it.eval(src).unwrap();
        assert!(matches!(it.value(&r).unwrap(), Val::Proc(_)), "{src}");
    }
    let cyclic = it.eval("(let ([x (list 1 2)]) (set-cdr! (cdr x) x) x)").unwrap();
    assert_eq!(it.value(&cyclic).unwrap(), Val::Opaque);
    // And Opaque cannot be smuggled back in as if it were a value.
    assert!(it.make(&Val::Opaque).is_err());
}

#[test]
fn eqv_is_the_scheme_question_handles_are_not() {
    let mut it = Interp::new().unwrap();
    let a = it.eval("(define s 'same) s").unwrap();
    let b = it.eval("s").unwrap();
    // Two different handles ...
    assert_ne!(a, b);
    // ... on the same value.
    assert!(it.eqv(&a, &b).unwrap());
}

#[track_caller]
fn it_eval(it: &mut Interp, src: &str) -> scm::Rooted {
    it.eval(src).unwrap_or_else(|e| panic!("eval {src:?}: {e}"))
}

// ---- S7b: host functions that can capture ------------------------------

#[test]
fn a_host_function_can_capture_state() {
    // The gap that made Track B's B1 unwritable: `PrimFn` is a bare `fn`
    // pointer, so a host function could capture NOTHING, and every ray verb
    // needs the scene store. Here the "store" is a Vec the closure owns.
    use std::cell::RefCell;
    use std::rc::Rc;

    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let mut it = Interp::new().unwrap();
    {
        let log = Rc::clone(&log);
        it.define_fn("add-box", move |args| {
            log.borrow_mut().push(format!("{args:?}"));
            // A plausible handle, which is what a recording stub returns.
            Ok(Val::Int(log.borrow().len() as i64))
        })
        .unwrap();
    }
    let v = it
        .eval("(list (add-box :size [1 2 3]) (add-box :at [0 0 0]))")
        .unwrap();
    assert_eq!(it.write(&v).unwrap(), "(1 2)");
    assert_eq!(log.borrow().len(), 2);
    // The keyword survived as a keyword, and the bracket vector as a vector.
    assert!(log.borrow()[0].contains(r#"Keyword("size")"#), "{:?}", log.borrow()[0]);
    assert!(log.borrow()[0].contains("Vector"), "{:?}", log.borrow()[0]);
}

#[test]
fn a_host_function_refusing_is_an_ordinary_scheme_error() {
    // The host's CODE failing is catchable; the host saying STOP is not.
    // That distinction is requirement 8, and this is the catchable half.
    let mut it = Interp::new().unwrap();
    it.define_fn("picky", |args| match args {
        [Val::Int(n)] if *n > 0 => Ok(Val::Int(n * 2)),
        _ => Err("wants one positive integer".into()),
    })
    .unwrap();

    let v = it.eval("(picky 21)").unwrap();
    assert_eq!(it.write(&v).unwrap(), "42");

    let caught = it.eval("(guard (e (else 'caught)) (picky -1))").unwrap();
    assert_eq!(it.write(&caught).unwrap(), "caught");

    match it.eval("(picky -1)") {
        Err(Error::Script(m)) => assert!(m.contains("picky") && m.contains("positive"), "{m}"),
        other => panic!("expected a script error, got {other:?}"),
    }
}

#[test]
fn a_panicking_host_function_does_not_take_the_vm_with_it() {
    // In ray this is the @7d8c8a1 bug with a new door: a panic cannot unwind
    // out of `extern "C"` and aborts the process. Here it has to become an
    // ordinary error, and the interpreter has to still work afterwards.
    let mut it = Interp::new().unwrap();
    it.define_fn("boom", |_| panic!("deliberate")).unwrap();

    match it.eval("(boom)") {
        Err(Error::Script(m)) => assert!(m.contains("boom") && m.contains("panicked"), "{m}"),
        other => panic!("expected a script error, got {other:?}"),
    }
    // Not poisoned: a host function panicking is contained, unlike a panic
    // in the machine itself.
    assert!(!it.is_poisoned());
    let v = it.eval("(+ 2 2)").unwrap();
    assert_eq!(it.write(&v).unwrap(), "4");
    // And it is catchable, since it arrived as a Scheme error.
    let caught = it.eval("(guard (e (else 'caught)) (boom))").unwrap();
    assert_eq!(it.write(&caught).unwrap(), "caught");
}

#[test]
fn a_host_function_sees_every_val_it_was_sent() {
    let mut it = Interp::new().unwrap();
    let seen: std::rc::Rc<std::cell::RefCell<Vec<Val>>> = Default::default();
    {
        let seen = std::rc::Rc::clone(&seen);
        it.define_fn("note", move |args| {
            seen.borrow_mut().extend(args.iter().cloned());
            Ok(Val::Unspecified)
        })
        .unwrap();
    }
    it.eval(r#"(note 1 2.5 #\a "s" 'sym :kw '(1 2) [3 4] #t '())"#)
        .unwrap();
    assert_eq!(
        *seen.borrow(),
        vec![
            Val::Int(1),
            Val::Float(2.5),
            Val::Char('a'),
            Val::Str("s".into()),
            Val::Sym("sym".into()),
            Val::Keyword("kw".into()),
            Val::List(vec![Val::Int(1), Val::Int(2)]),
            Val::Vector(vec![Val::Int(3), Val::Int(4)]),
            Val::Bool(true),
            Val::Null,
        ]
    );
}

#[test]
fn a_host_function_cannot_hold_a_heap_reference_across_a_collection() {
    // Why the signature takes `Val` and not `Value`: there is nothing for a
    // host function to hold wrongly. It gets data, and the data is already
    // copied out of the heap before it is called.
    let mut it = Interp::new().unwrap();
    it.define_fn("sum", |args| {
        let mut total = 0i64;
        for a in args {
            if let Val::List(items) = a {
                for i in items {
                    if let Val::Int(n) = i {
                        total += n;
                    }
                }
            }
        }
        Ok(Val::Int(total))
    })
    .unwrap();
    // Allocate hard around the call so a collection certainly happens.
    let v = it
        .eval(
            "(let loop ([i 0] [acc 0])
               (if (= i 2000) acc
                   (loop (+ i 1) (+ acc (sum (list 1 2 3))))))",
        )
        .unwrap();
    assert_eq!(it.write(&v).unwrap(), "12000");
}

// ── host re-entrancy ─────────────────────────────────────────────────────
//
// S7 left one thing open: "a host function cannot re-enter the VM". Ray's
// verbs take callbacks, so it has to.

#[test]
fn a_host_function_can_call_a_scheme_procedure() {
    let mut i = Interp::new().unwrap();
    // `(each-twice f x)`: the host calls f twice and sums.
    i.define_fn_with("each-twice", |host, args| {
        let [Val::Proc(f), x] = args else { return Err("expects a procedure and a value".into()) };
        let a = host.call(f, std::slice::from_ref(x))?;
        let b = host.call(f, std::slice::from_ref(x))?;
        match (a, b) {
            (Val::Int(a), Val::Int(b)) => Ok(Val::Int(a + b)),
            _ => Err("not integers".into()),
        }
    })
    .unwrap();
    let v = i.eval("(each-twice (lambda (n) (* n 10)) 4)").unwrap();
    assert_eq!(i.value(&v).unwrap(), Val::Int(80));
    // Nested both ways: Scheme -> host -> Scheme -> host -> Scheme.
    let v = i.eval("(each-twice (lambda (n) (each-twice (lambda (m) (+ m 1)) n)) 1)").unwrap();
    assert_eq!(i.value(&v).unwrap(), Val::Int(8));
    // A Scheme error inside the callback reaches the host as a message, and
    // the host function's own failure is an ordinary catchable error.
    let v = i.eval("(guard (e (#t (error-object-message e))) (each-twice (lambda (n) (car n)) 4))").unwrap();
    assert!(matches!(i.value(&v).unwrap(), Val::Str(s) if s.contains("not a pair")));
    // And the session is intact: the stacks came back.
    let v = i.eval("(+ 1 2)").unwrap();
    assert_eq!(i.value(&v).unwrap(), Val::Int(3));
}

#[test]
fn a_callback_survives_collection_and_can_be_called_later_from_outside() {
    let mut i = Interp::new().unwrap();
    let kept: std::rc::Rc<std::cell::RefCell<Option<Proc>>> = Default::default();
    let slot = kept.clone();
    i.define_fn_with("on-change", move |_host, args| {
        let [Val::Proc(f)] = args else { return Err("expects a procedure".into()) };
        *slot.borrow_mut() = Some(f.clone());
        Ok(Val::Unspecified)
    })
    .unwrap();
    i.eval("(let ([count 0]) (on-change (lambda (x) (set! count (+ count x)) count)))").unwrap();
    // Churn the heap: the stored procedure must stay rooted.
    i.eval("(let loop ([i 0] [acc '()]) (if (= i 20000) (length acc) (loop (+ i 1) (cons (make-vector 3 i) acc))))").unwrap();
    let f = kept.borrow().clone().unwrap();
    assert_eq!(i.call(&f, &[Val::Int(5)]).unwrap(), Val::Int(5));
    assert_eq!(i.call(&f, &[Val::Int(7)]).unwrap(), Val::Int(12), "its closed-over state persists");
    // A procedure inside a list argument is rooted too.
    i.define_fn_with("first-proc", |host, args| {
        let [Val::List(items)] = args else { return Err("expects a list".into()) };
        let Some(Val::Proc(f)) = items.first() else { return Err("no procedure".into()) };
        host.call(f, &[])
    })
    .unwrap();
    let v = i.eval("(first-proc (list (lambda () 'called) 2 3))").unwrap();
    assert_eq!(i.value(&v).unwrap(), Val::Sym("called".into()));
}

#[test]
fn a_panic_in_a_reentrant_host_function_leaves_the_machine_usable() {
    let mut i = Interp::new().unwrap();
    i.define_fn_with("boom", |host, args| {
        let [Val::Proc(f)] = args else { return Err("expects a procedure".into()) };
        // Enter Scheme, then panic with that call's frames behind us.
        let _ = host.call(f, &[]);
        panic!("host bug");
    })
    .unwrap();
    let r = i.eval("(boom (lambda () (+ 1 1)))");
    let msg = format!("{}", r.unwrap_err());
    assert!(msg.contains("panicked"), "{msg}");
    let v = i.eval("(* 6 7)").unwrap();
    assert_eq!(i.value(&v).unwrap(), Val::Int(42));
}

#[test]
fn an_exact_integer_too_wide_for_a_word_crosses_the_boundary_as_text() {
    // S18 made exact integers unbounded, which left a hole here: a bignum
    // result reached the host as `Val::Opaque`, so a host could not see a
    // number the language had just computed. `Val::BigInt` carries it as
    // decimal text -- text rather than a bignum type, because the crate has
    // no dependencies and exposing the internal one would make every host
    // take it.
    let mut i = Interp::new().unwrap();

    let r = i.eval("(* 9223372036854775807 2)").unwrap();
    assert_eq!(i.value(&r).unwrap(), Val::BigInt("18446744073709551614".into()));
    // What fits a word is still an `Int`, so a host matching on `Int` keeps
    // working for every value it used to see.
    let r = i.eval("42").unwrap();
    assert_eq!(i.value(&r).unwrap(), Val::Int(42));

    // Both directions, losslessly.
    let text = "1267650600228229401496703205376";
    let built = i.make(&Val::BigInt(text.into())).unwrap();
    assert_eq!(i.write(&built).unwrap(), text);
    assert_eq!(i.value(&built).unwrap(), Val::BigInt(text.into()));

    // And normalised on the way in: one integer, one representation,
    // whichever side of the boundary built it.
    let small = i.make(&Val::BigInt("7".into())).unwrap();
    assert_eq!(i.value(&small).unwrap(), Val::Int(7));

    // A host function may return one, and Scheme does arithmetic on it.
    i.define_fn("big", |_| Ok(Val::BigInt("1000000000000000000000".into())))
        .unwrap();
    let r = i.eval("(+ (big) 1)").unwrap();
    assert_eq!(i.write(&r).unwrap(), "1000000000000000000001");

    // Text that is not an exact integer is the host's mistake and is
    // reported rather than silently becoming something else.
    assert!(matches!(i.make(&Val::BigInt("1.5".into())), Err(Error::Syntax(_))));
}

#[test]
fn an_improper_list_crosses_as_the_cons_chain_it_is() {
    // Added for ray's port, which needs an ASSOCIATION LIST to cross: a
    // language with no dictionary type spells one `((:a . 1) (:b . 2))`,
    // and that is a proper list whose every element is an improper pair.
    // Those elements used to arrive as `Val::Opaque`, so a host could not
    // tell an alist from any other list of unknowable things and had to
    // apply the rule in Scheme — which ray measured at 53 ms against a
    // 6.9 ms boundary on a 40,000-element argument.
    let mut i = Interp::new().unwrap();

    // One cons cell.
    let r = i.eval("(cons 1 2)").unwrap();
    assert_eq!(i.value(&r).unwrap(), Val::Pair(Box::new(Val::Int(1)), Box::new(Val::Int(2))));

    // A partly-improper list nests, innermost last.
    let r = i.eval("'(1 2 . 3)").unwrap();
    assert_eq!(
        i.value(&r).unwrap(),
        Val::Pair(
            Box::new(Val::Int(1)),
            Box::new(Val::Pair(Box::new(Val::Int(2)), Box::new(Val::Int(3))))
        )
    );

    // A PROPER list is still a `List`, which is the compatibility promise:
    // a host that only knows `List` sees exactly what it saw before.
    let r = i.eval("'(1 2 3)").unwrap();
    assert_eq!(i.value(&r).unwrap(), Val::List(vec![Val::Int(1), Val::Int(2), Val::Int(3)]));

    // The shape the port is here for.
    let r = i.eval("'((:degree . 3) (:closed . #t))").unwrap();
    let Val::List(items) = i.value(&r).unwrap() else { panic!("an alist is a proper list") };
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        Val::Pair(Box::new(Val::Keyword("degree".into())), Box::new(Val::Int(3)))
    );

    // Both directions: built on the host side, it reads back as Scheme data.
    let built = i
        .make(&Val::List(vec![Val::Pair(
            Box::new(Val::Keyword("at".into())),
            Box::new(Val::Float(1.5)),
        )]))
        .unwrap();
    assert_eq!(i.write(&built).unwrap(), "((:at . 1.5))");
    i.define_fn("alist", |_| {
        Ok(Val::List(vec![Val::Pair(Box::new(Val::Keyword("k".into())), Box::new(Val::Int(7)))]))
    })
    .unwrap();
    let r = i.eval("(cdr (assq ':k (alist)))").unwrap();
    assert_eq!(i.value(&r).unwrap(), Val::Int(7));

    // A list whose tail is an object rather than an immediate — the other
    // improper arm — keeps its tail too.
    let r = i.eval("(cons 1 \"tail\")").unwrap();
    assert_eq!(
        i.value(&r).unwrap(),
        Val::Pair(Box::new(Val::Int(1)), Box::new(Val::Str("tail".into())))
    );

    // And a procedure inside one is still rooted, which is the rooting
    // reader's own job: this goes through the host-function path.
    i.define_fn_with("car-of", |host, args| match args {
        [Val::Pair(car, _)] => match car.as_ref() {
            Val::Proc(f) => host.call(f, &[Val::Int(20)]),
            other => Ok(other.clone()),
        },
        _ => Err("wants one pair".into()),
    })
    .unwrap();
    let r = i.eval("(car-of (cons (lambda (x) (* x 2)) 'tail))").unwrap();
    assert_eq!(i.value(&r).unwrap(), Val::Int(40));
}
