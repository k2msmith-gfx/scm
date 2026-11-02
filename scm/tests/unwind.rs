//! Two bugs ray's Janet-retirement run found (2026-11-01), as tests on
//! both tiers. T2's agreement is in `scm-llvm/tests/t2.rs`.
//!
//! 1. An error that escaped to the HOST skipped every pending
//!    `dynamic-wind` after-thunk: only a `%try`/`guard` on the stack ran
//!    them. R7RS runs `after` whenever control leaves the extent, and an
//!    error unwinding to the top level leaves it.
//! 2. `(- 0.0)` answered `0.0`: unary minus was the fold `0 - x`, which
//!    loses the sign of zero. It is negation.

use std::sync::atomic::Ordering;
use std::time::Duration;

use scm::embed::Error;
use scm::{BreakAction, Interp, Tier, Val, Vm, write_str};

const TIERS: [Tier; 2] = [Tier::T0, Tier::T1];

fn interp(tier: Tier) -> Interp {
    let mut it = Interp::new().unwrap();
    it.vm_mut().set_tier(tier);
    it
}

fn show(it: &mut Interp, src: &str) -> String {
    let r = it.eval(src).unwrap_or_else(|e| panic!("{src}: {e:?}"));
    it.write(&r).unwrap()
}

fn script_error(r: Result<scm::Rooted, Error>) -> String {
    match r {
        Err(Error::Script(m)) => m,
        Err(Error::Raised { text, .. }) => text,
        Err(e) => panic!("expected a script error, got {e:?}"),
        Ok(_) => panic!("expected an error, got a value"),
    }
}

// ---- 1. dynamic-wind on an uncaught error ------------------------------

#[test]
fn an_after_thunk_runs_when_an_error_reaches_the_host() {
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define done #f)");
        let msg = script_error(it.eval(
            "(dynamic-wind (lambda () #f) (lambda () (error \"x\")) (lambda () (set! done #t)))",
        ));
        assert!(msg.contains('x'), "{tier:?}: the original error arrives: {msg}");
        assert_eq!(show(&mut it, "done"), "#t", "{tier:?}");
    }
}

#[test]
fn a_raised_object_still_arrives_after_the_afters_have_collected() {
    // The raised object lives only in the parked error while the afters
    // run, and an after that allocates enough collects.
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define done #f)");
        let r = it.eval(
            "(dynamic-wind (lambda () #f)
                           (lambda () (raise (list 'payload 1 2 3)))
                           (lambda () (let loop ([i 0] [acc '()])
                                        (if (= i 100000) (set! done #t)
                                            (loop (+ i 1) (cons i acc))))))",
        );
        match r {
            Err(Error::Raised { value, .. }) => {
                assert_eq!(it.write(&value).unwrap(), "(payload 1 2 3)", "{tier:?}")
            }
            other => panic!("{tier:?}: {other:?}"),
        }
        assert_eq!(show(&mut it, "done"), "#t", "{tier:?}");
    }
}

#[test]
fn nested_winds_unwind_innermost_first_to_the_host() {
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define log '()) (define (note x) (set! log (cons x log)))");
        script_error(it.eval(
            "(dynamic-wind (lambda () (note 'outer-in))
               (lambda ()
                 (dynamic-wind (lambda () (note 'inner-in))
                               (lambda () (car '()))
                               (lambda () (note 'inner-out))))
               (lambda () (note 'outer-out)))",
        ));
        assert_eq!(
            show(&mut it, "(reverse log)"),
            "(outer-in inner-in inner-out outer-out)",
            "{tier:?}"
        );
    }
}

#[test]
fn an_after_runs_in_the_dynamic_environment_of_its_wind() {
    // A `parameterize` inside the body is undone before `after` runs; one
    // outside the wind is still in force.
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define p (make-parameter 'top)) (define seen '())");
        script_error(it.eval(
            "(parameterize ([p 'around])
               (dynamic-wind (lambda () #f)
                 (lambda () (parameterize ([p 'inside]) (error \"boom\")))
                 (lambda () (set! seen (cons (p) seen)))))",
        ));
        assert_eq!(show(&mut it, "(list seen (p))"), "((around) top)", "{tier:?}");
    }
}

#[test]
fn a_raising_after_replaces_the_error_and_the_outer_afters_still_run() {
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define log '()) (define (note x) (set! log (cons x log)))");
        let msg = script_error(it.eval(
            "(dynamic-wind (lambda () #f)
               (lambda ()
                 (dynamic-wind (lambda () #f)
                   (lambda ()
                     (dynamic-wind (lambda () #f)
                                   (lambda () (error \"first\"))
                                   (lambda () (note 'a) (error \"second\"))))
                   (lambda () (note 'b))))
               (lambda () (note 'c)))",
        ));
        assert!(msg.contains("second"), "{tier:?}: the later error wins: {msg}");
        assert_eq!(show(&mut it, "(reverse log)"), "(a b c)", "{tier:?}");
    }
}

#[test]
fn a_caught_after_error_runs_the_rest_of_the_chain() {
    // The same mid-chain raise under a guard (a machine error, so `%try`
    // catches it and runs the chain): the after that raises is still inside
    // the guard, which takes the new error, and the outer after is not lost.
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define log '()) (define (note x) (set! log (cons x log)))");
        let v = show(
            &mut it,
            "(guard (e (#t (list (quote caught) e)))
               (dynamic-wind (lambda () #f)
                 (lambda ()
                   (dynamic-wind (lambda () #f)
                                 (lambda () (car (quote ())))
                                 (lambda () (note (quote a)) (raise (quote second)))))
                 (lambda () (note (quote b)))))",
        );
        assert_eq!(v, "(caught second)", "{tier:?}");
        assert_eq!(show(&mut it, "(reverse log)"), "(a b)", "{tier:?}");
    }
}

#[test]
fn the_after_thunks_run_on_a_host_call_too() {
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define done #f)");
        let f = it
            .eval("(lambda (x) (dynamic-wind (lambda () #f) (lambda () (car x)) (lambda () (set! done #t))))")
            .unwrap();
        let Val::Proc(f) = it.value(&f).unwrap() else { panic!("a procedure") };
        assert!(it.call(&f, &[Val::Int(1)]).is_err(), "{tier:?}");
        assert_eq!(show(&mut it, "done"), "#t", "{tier:?}");
    }
}

#[test]
fn an_uncaught_error_through_a_host_function_unwinds_both_sides() {
    // Scheme -> host -> Scheme -> error: the inner evaluation runs its own
    // after, the error crosses the host function, and the outer evaluation
    // runs the outer one.
    for tier in TIERS {
        let mut it = interp(tier);
        it.define_fn_with("host-call", |host, args| {
            let [Val::Proc(f)] = args else { return Err("expects a procedure".into()) };
            host.call(f, &[])
        })
        .unwrap();
        show(&mut it, "(define log '()) (define (note x) (set! log (cons x log)))");
        script_error(it.eval(
            "(dynamic-wind (lambda () #f)
               (lambda ()
                 (host-call (lambda ()
                   (dynamic-wind (lambda () #f) (lambda () (error \"deep\")) (lambda () (note 'inner))))))
               (lambda () (note 'outer)))",
        ));
        assert_eq!(show(&mut it, "(reverse log)"), "(inner outer)", "{tier:?}");
    }
}

#[test]
fn a_host_stop_does_not_run_after_thunks() {
    // The decision: interrupt, fuel and heap cap are the host saying stop,
    // and an `after` is arbitrary code. They propagate with afters unrun.
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define done #f)");
        it.set_fuel(Some(50));
        assert_eq!(
            it.eval("(dynamic-wind (lambda () #f) (lambda () (let loop () (loop))) (lambda () (set! done #t)))")
                .err(),
            Some(Error::OutOfFuel),
            "{tier:?}"
        );
        it.set_fuel(None);
        assert_eq!(show(&mut it, "done"), "#f", "{tier:?}: fuel");

        let flag = it.interrupt_handle();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        assert_eq!(
            it.eval("(dynamic-wind (lambda () #f) (lambda () (let loop () (loop))) (lambda () (set! done #t)))")
                .err(),
            Some(Error::Interrupted),
            "{tier:?}"
        );
        t.join().unwrap();
        assert_eq!(show(&mut it, "done"), "#f", "{tier:?}: interrupt");
    }
}

#[test]
fn the_break_handler_inspects_before_the_afters_run() {
    for tier in TIERS {
        let mut vm = Vm::with_tier(tier);
        vm.eval_str("(define done #f)").unwrap();
        let seen: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
        let log = seen.clone();
        vm.set_break_handler(Some(Box::new(move |vm, _| {
            let v = vm.eval_in("done", None).unwrap();
            log.borrow_mut().push(write_str(&vm.engine, v));
            BreakAction::Abort
        })));
        assert!(vm
            .eval_str("(dynamic-wind (lambda () #f) (lambda () (car 1)) (lambda () (set! done #t)))")
            .is_err());
        assert_eq!(*seen.borrow(), vec!["#f".to_string()], "{tier:?}: at the break, not yet unwound");
        let v = vm.eval_str("done").unwrap();
        assert_eq!(write_str(&vm.engine, v), "#t", "{tier:?}: aborting the break unwinds");
    }
}

#[test]
fn the_backtrace_is_the_original_failures() {
    for tier in TIERS {
        let mut vm = Vm::with_tier(tier);
        vm.eval_str("(define (f x) (vector-ref (vector 1 2) x))").unwrap();
        let e = vm
            .eval_str("(dynamic-wind (lambda () #f) (lambda () (f 9)) (lambda () (list 1 2)))")
            .unwrap_err();
        assert!(format!("{e}").contains("out of range"), "{tier:?}: {e}");
        let bt: Vec<String> = vm.backtrace().iter().map(|f| f.what.clone()).collect();
        assert!(bt.iter().any(|w| w.contains("vector-ref")), "{tier:?}: {bt:?}");
    }
}

// ---- 2. unary minus keeps the sign of zero -----------------------------

#[test]
fn unary_minus_is_negation() {
    for tier in TIERS {
        let mut it = interp(tier);
        for (src, want) in [
            ("(eqv? (- 0.0) -0.0)", "#t"),
            ("(eqv? (- -0.0) 0.0)", "#t"),
            ("(/ 1 (- 0.0))", "-inf.0"),
            ("(- 0.0)", "-0.0"),
            ("(- 0)", "0"),
            ("(eqv? (- 0) 0)", "#t"),
            ("(- 2.5)", "-2.5"),
            ("(+ -0.0)", "-0.0"),
            ("(* -1 0.0)", "-0.0"),
            // Through a compiled procedure, where T1's `Neg` fast path runs.
            ("(define (neg x) (- x)) (list (neg 0.0) (neg 0) (/ 1 (neg 0.0)) (neg -0.0))", "(-0.0 0 -inf.0 0.0)"),
            ("(let loop ([i 0] [x 0.0]) (if (= i 3) (/ 1 x) (loop (+ i 1) (- x))))", "-inf.0"),
        ] {
            assert_eq!(show(&mut it, src), want, "{tier:?}: {src}");
        }
    }
}
