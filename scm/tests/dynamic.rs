//! S5, slice 3: `make-parameter`, `parameterize`, and a seeded RNG.
//!
//! Two things here are load-bearing for ray rather than for conformance:
//! `parameterize` is what 24 corpus files' `dyn`/`setdyn` become, and the
//! RNG-behind-a-parameter is why a `:seed` replays -- `grow-branches` is only
//! reproducible because ray threads `:ray-rng` through it (survey §4).

use scm::embed::Error;
use scm::{Interp, Tier, Val, Vm, write_str};

#[track_caller]
fn eval(src: &str) -> String {
    let mut vm = Vm::new();
    let v = vm
        .eval_str(src)
        .unwrap_or_else(|e| panic!("eval {src:?}: {e}"));
    write_str(&vm.engine, v)
}

#[track_caller]
fn eval_err(src: &str) -> String {
    let mut vm = Vm::new();
    match vm.eval_str(src) {
        Ok(v) => panic!(
            "{src:?} should have failed, gave {}",
            write_str(&vm.engine, v)
        ),
        Err(e) => format!("{e}"),
    }
}

// ---- parameters --------------------------------------------------------

#[test]
fn a_parameter_is_callable_and_rebindable() {
    assert_eq!(eval("(define p (make-parameter 1)) (p)"), "1");
    assert_eq!(
        eval("(define p (make-parameter 1)) (list (p) (parameterize ([p 2]) (p)) (p))"),
        "(1 2 1)"
    );
    assert_eq!(eval("(parameter? (make-parameter 1))"), "#t");
    assert_eq!(eval("(parameter? 1)"), "#f");
    // Setting the current binding directly, which Racket allows and a scene
    // converted from `setdyn` needs.
    assert_eq!(
        eval("(define p (make-parameter 1)) (p 5) (p)"),
        "5"
    );
}

#[test]
fn parameterize_nests_and_restores_in_order() {
    assert_eq!(
        eval("(define p (make-parameter 'a))
              (list (p)
                    (parameterize ([p 'b])
                      (list (p) (parameterize ([p 'c]) (p)) (p)))
                    (p))"),
        "(a (b c b) a)"
    );
}

#[test]
fn parameterize_binds_several_at_once() {
    assert_eq!(
        eval("(define p (make-parameter 1))
              (define q (make-parameter 2))
              (parameterize ([p 10] [q 20]) (list (p) (q)))"),
        "(10 20)"
    );
}

#[test]
fn a_converter_runs_on_the_initial_value_and_on_every_rebinding() {
    // R7RS says the converter is applied to anything stored in the
    // parameter -- INCLUDING the initial value, which the first version of
    // this got wrong and this test originally encoded. Applying it means
    // calling a procedure, which a primitive cannot do, so both
    // `make-parameter` and `parameterize` do the call in Scheme and the
    // machine evaluates it like any other.
    assert_eq!(
        eval("(define p (make-parameter 1 (lambda (x) (* x 10))))
              (list (p) (parameterize ([p 2]) (p)) (p))"),
        "(10 20 10)"
    );
}

#[test]
fn an_error_inside_parameterize_still_restores() {
    // THE test. An error discards the frames that would have restored the
    // binding, so the unwind has to do it -- otherwise a failed
    // `parameterize` leaks its value into everything afterwards, which is
    // the sort of bug that looks like the next thing's fault.
    let mut vm = Vm::new();
    vm.eval_str("(define p (make-parameter 'outer))").unwrap();
    assert!(
        vm.eval_str("(parameterize ([p 'inner]) (car 5))").is_err(),
        "the body should have failed"
    );
    assert_eq!(
        vm.eval_str("(p)").map(|v| write_str(&vm.engine, v)).unwrap(),
        "outer",
        "a failed parameterize leaked its binding"
    );
}

#[test]
fn an_interrupt_inside_parameterize_also_restores() {
    // Same requirement, different unwind: out of fuel rather than an error.
    let mut vm = Vm::new();
    vm.eval_str("(define p (make-parameter 'outer))").unwrap();
    vm.eval_str("(define (spin i) (spin (+ i 1)))").unwrap();
    vm.set_fuel(Some(50));
    assert!(vm.eval_str("(parameterize ([p 'inner]) (spin 0))").is_err());
    vm.set_fuel(None);
    assert_eq!(
        vm.eval_str("(p)").map(|v| write_str(&vm.engine, v)).unwrap(),
        "outer"
    );
}

#[test]
fn parameterize_is_still_a_tail_position_for_its_body() {
    // The body's last expression is in tail position *within* the body, but
    // the restore frame means a loop driven from inside a parameterize does
    // not grow without bound either.
    let mut vm = Vm::new();
    vm.eval_str("(define p (make-parameter 0))").unwrap();
    let before = vm.stack_high_water();
    vm.eval_str(
        "(parameterize ([p 1])
           (let loop ((i 0)) (if (< i 20000) (loop (+ i 1)) i)))",
    )
    .expect("loop");
    assert!(
        vm.stack_high_water() - before < 50,
        "a loop inside parameterize grew the stack by {}",
        vm.stack_high_water() - before
    );
}

#[test]
fn parameterize_rejects_a_non_parameter() {
    assert!(eval_err("(parameterize ([1 2]) 3)").contains("not a parameter"));
}

// ---- randomness --------------------------------------------------------

#[test]
fn the_same_seed_gives_the_same_sequence() {
    // The whole point. ray's `:seed` is only meaningful if this holds.
    let one = eval("(with-seed 7 (lambda () (list (random-integer 1000) (random-integer 1000))))");
    let two = eval("(with-seed 7 (lambda () (list (random-integer 1000) (random-integer 1000))))");
    assert_eq!(one, two);
    let other = eval("(with-seed 8 (lambda () (list (random-integer 1000) (random-integer 1000))))");
    assert_ne!(one, other, "different seeds should differ");
}

#[test]
fn the_generator_is_a_value_behind_a_parameter_not_a_global() {
    // Which is what lets a seed be scoped to one build rather than to the
    // process -- the shape ray's `:ray-rng` has.
    assert_eq!(eval("(random-source? (current-random-source))"), "#t");
    assert_eq!(eval("(random-source? (make-random-source 1))"), "#t");
    let src = "(define outer (current-random-source))
               (define inner
                 (parameterize ([current-random-source (make-random-source 1)])
                   (current-random-source)))
               (list (eq? outer (current-random-source)) (eq? outer inner))";
    assert_eq!(eval(src), "(#t #f)");
}

#[test]
fn an_explicit_source_can_be_passed() {
    // So a scene can hold two independent streams without parameterizing.
    let src = "(define a (make-random-source 3))
               (define b (make-random-source 3))
               (list (= (random-integer 10000 a) (random-integer 10000 b))
                     (= (random-integer 10000 a) (random-integer 10000 b)))";
    assert_eq!(eval(src), "(#t #t)");
}

#[test]
fn random_real_is_in_range_and_random_integer_is_uniform_enough() {
    // Not a statistical test; a check that the rejection sampling is not
    // obviously broken and nothing escapes its bound.
    let src = "(with-seed 99 (lambda ()
                 (let loop ((i 0) (lo 1.0) (hi 0.0) (sum 0))
                   (if (= i 2000)
                       (list (>= lo 0.0) (< hi 1.0) (> sum 0))
                       (let ([r (random-real)] [n (random-integer 10)])
                         (loop (+ i 1) (min lo r) (max hi r) (+ sum n)))))))";
    assert_eq!(eval(src), "(#t #t #t)");
    // Every value is inside the bound.
    let src = "(with-seed 5 (lambda ()
                 (let loop ((i 0) (ok #t))
                   (if (= i 1000)
                       ok
                       (let ([n (random-integer 4)])
                         (loop (+ i 1) (and ok (>= n 0) (< n 4))))))))";
    assert_eq!(eval(src), "#t");
}

#[test]
fn random_integer_wants_a_positive_bound() {
    assert!(eval_err("(random-integer 0)").contains("positive exact bound"));
    assert!(eval_err("(random-integer -1)").contains("positive exact bound"));
}

#[test]
fn the_janet_spellings_work_too() {
    // A converted scene reaches for these.
    assert_eq!(eval("(with-seed 1 (lambda () (< (random) 1.0)))"), "#t");
    assert_eq!(eval("(with-seed 1 (lambda () (< (random-int 10) 10)))"), "#t");
}

#[test]
fn parameters_and_sources_survive_gc_stress() {
    // Parameter values and the dynamic binding stack are both roots that
    // nothing else refers to, and an RNG is a mutable heap object reachable
    // only through one of them.
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            "(define p (make-parameter 0))
             (parameterize ([p 1] [current-random-source (make-random-source 11)])
               (let loop ((i 0) (acc '()))
                 (if (= i 30)
                     (list (p) (length acc))
                     (loop (+ i 1) (cons (random-integer 100) acc)))))",
        )
        .expect("parameters under gc stress");
    assert_eq!(write_str(&vm.engine, v), "(1 30)");
}

// ---- %try: catchable errors, and what is deliberately not -------------

#[test]
fn try_catches_an_error_and_yields_the_condition() {
    // `%try` yields the *condition object*, not a string -- a Rust refusal
    // arrives as an error object, which is what lets `guard` ask it for a
    // message and irritants. An error object prints as its message.
    assert_eq!(eval("(%try (+ 1 2))"), "(#t . 3)");
    assert_eq!(eval("(%try (car 5))"), "(#f . car: not a pair: 5)");
    assert_eq!(
        eval("(cdr (%try (car 5)))"),
        "car: not a pair: 5"
    );
    assert_eq!(eval("(error-object? (cdr (%try (car 5))))"), "#t");
    assert_eq!(
        eval("(error-object-message (cdr (%try (undefined-thing))))"),
        "\"unbound variable: undefined-thing\""
    );
    // And the VM carries on.
    assert_eq!(eval("(%try (car 5)) (+ 1 1)"), "2");
}

#[test]
fn try_does_not_swallow_the_hosts_stop() {
    // Requirement 8: interrupt, fuel and the heap cap are the host saying
    // stop, so a script must not be able to ignore them by wrapping the
    // runaway in `%try`.
    let mut vm = Vm::new();
    vm.eval_str("(define (spin i) (spin (+ i 1)))").unwrap();
    vm.set_fuel(Some(50));
    let err = vm.eval_str("(%try (spin 0))").expect_err("fuel is not catchable");
    assert_eq!(err, scm::VmError::OutOfFuel);

    let mut vm = Vm::new();
    vm.eval_str("(define (grow i acc) (grow (+ i 1) (cons i acc)))")
        .unwrap();
    vm.engine.set_min_heap(64);
    vm.engine.set_heap_cap(Some(50_000));
    let err = vm
        .eval_str("(%try (grow 0 '()))")
        .expect_err("the heap cap is not catchable");
    assert_eq!(err, scm::VmError::OutOfMemory);
}

#[test]
fn a_caught_error_still_restores_parameterize() {
    // The Catch frame records the dynamic-binding depth, so catching an error
    // from inside a `parameterize` puts the old value back just as an
    // uncaught one does.
    assert_eq!(
        eval("(define p (make-parameter 'outer))
              (list (car (%try (parameterize ([p 'inner]) (car 5)))) (p))"),
        "(#f outer)"
    );
}

#[test]
fn try_nests() {
    assert_eq!(eval("(%try (%try (car 5)))"), "(#t #f . car: not a pair: 5)");
    assert_eq!(
        eval("(car (%try (let ([r (%try (car 5))]) (if (car r) 'no (error \"outer\")))))"),
        "#f"
    );
}

// ---- guard, raise, with-exception-handler ------------------------------

#[test]
fn guard_catches_and_binds_the_condition() {
    assert_eq!(
        eval("(guard (e (#t (list 'caught (error-object-message e)))) (error \"bad\" 1 2))"),
        "(caught \"bad\")"
    );
    assert_eq!(
        eval("(guard (e (#t (error-object-irritants e))) (error \"bad\" 1 2))"),
        "(1 2)"
    );
    // A body that does not raise returns normally and the clauses never run.
    assert_eq!(eval("(guard (e (#t 'caught)) 42)"), "42");
}

#[test]
fn guard_clauses_behave_like_cond() {
    assert_eq!(
        eval("(guard (e ((symbol? e) (list 'sym e)) (else 'other)) (raise 'boom))"),
        "(sym boom)"
    );
    assert_eq!(
        eval("(guard (e ((symbol? e) 'sym) (else (list 'other e))) (raise 42))"),
        "(other 42)"
    );
    assert_eq!(eval("(guard (e ((assv 1 '((1 . one))) => cdr)) (raise 'x))"), "one");
}

#[test]
fn a_guard_that_matches_nothing_re_raises() {
    // It must not swallow the condition, which a `cond`-shaped fall-through
    // would otherwise do by returning #f.
    let e = eval_err("(guard (e ((string? e) 'str)) (raise 42))");
    assert!(e.contains("42"), "got {e}");
    // And an outer guard sees it.
    assert_eq!(
        eval("(guard (e (#t (list 'outer e)))
                (guard (e2 ((string? e2) 'inner)) (raise 7)))"),
        "(outer 7)"
    );
}

#[test]
fn a_rust_refusal_arrives_as_an_error_object() {
    // So `guard` works uniformly over what the implementation raises and what
    // a script raises.
    assert_eq!(eval("(guard (e ((error-object? e) 'yes)) (car 5))"), "yes");
    assert_eq!(
        eval("(guard (e (#t (error-object-message e))) (car 5))"),
        "\"car: not a pair: 5\""
    );
    assert_eq!(eval("(guard (e (#t (error-object? e))) (raise 42))"), "#f");
}

#[test]
fn with_exception_handler_runs_at_the_raise_point() {
    // The difference from `guard`: the handler runs BEFORE any unwinding, so
    // `raise-continuable` can return a value into the middle of an
    // expression.
    assert_eq!(
        eval("(with-exception-handler (lambda (c) 99) (lambda () (+ 1 (raise-continuable 0))))"),
        "100"
    );
    // The handler is removed while it runs, so a raise inside it reaches the
    // next one out -- which `parameterize`'s unwind gives for free.
    assert_eq!(
        eval("(with-exception-handler
                 (lambda (c) (list 'outer c))
                 (lambda ()
                   (with-exception-handler
                     (lambda (c) (raise-continuable (list 'inner c)))
                     (lambda () (raise-continuable 1)))))"),
        "(outer (inner 1))"
    );
}

#[test]
fn a_non_continuable_raise_whose_handler_returns_is_an_error() {
    // R7RS: if a `raise` handler returns, a secondary exception is raised.
    let e = eval_err("(with-exception-handler (lambda (c) 'ignored) (lambda () (raise 'x)))");
    assert!(e.contains('x'), "got {e}");
}

#[test]
fn guard_restores_parameters_and_handlers() {
    assert_eq!(
        eval("(define p (make-parameter 'outer))
              (list (guard (e (#t (p))) (parameterize ([p 'inner]) (error \"x\"))) (p))"),
        "(outer outer)"
    );
    // The handler stack is itself a parameter, so it unwinds the same way.
    assert_eq!(
        eval("(guard (e (#t (length (%handlers))))
                (with-exception-handler (lambda (c) c) (lambda () (error \"x\"))))"),
        "0"
    );
}

#[test]
fn guard_survives_gc_stress() {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            "(let loop ((i 0) (acc '()))
               (if (= i 30)
                   (length acc)
                   (loop (+ i 1)
                         (cons (guard (e (#t (error-object-message e)))
                                 (error (format \"n ~a\" i)))
                               acc))))",
        )
        .expect("guard under gc stress");
    assert_eq!(write_str(&vm.engine, v), "30");
}

// ---- S28: every condition reaches the handler; guard clauses after unwind --
//
// Two R7RS bugs, both tiers. T2's agreement is in `scm-llvm/tests/t2.rs`.
//
// 1. `error`, and every refusal from Rust (`(car 5)`, a host function's
//    `Err`), unwound straight past `with-exception-handler`: only the
//    prelude's `raise` consulted the handler stack. Now the VM calls the
//    handler at the raise point for all of them.
// 2. `guard` ran its clauses AT THE RAISE POINT (inside its handler) for a
//    `raise`, so a `dynamic-wind` after in the body ran after the clause and
//    a `parameterize` in the body was visible to it. R7RS 4.2.7: the clauses
//    run in the dynamic environment of the `guard`.

const TIERS: [Tier; 2] = [Tier::T0, Tier::T1];

fn interp(tier: Tier) -> Interp {
    let mut it = Interp::new().unwrap();
    it.vm_mut().set_tier(tier);
    it
}

#[track_caller]
fn show(it: &mut Interp, src: &str) -> String {
    let r = it.eval(src).unwrap_or_else(|e| panic!("{src}: {e:?}"));
    it.write(&r).unwrap()
}

#[test]
fn error_reaches_with_exception_handler() {
    for tier in TIERS {
        let mut it = interp(tier);
        assert_eq!(
            show(
                &mut it,
                "(call/cc (lambda (k)
                   (with-exception-handler
                     (lambda (e) (k (list (error-object? e) (error-object-message e) (error-object-irritants e))))
                     (lambda () (+ 1 (error \"boom\" 1 'two))))))"
            ),
            "(#t \"boom\" (1 two))",
            "{tier:?}"
        );
        // It runs at the raise point: a parameterize and a dynamic-wind
        // around the `error` are both still in force.
        assert_eq!(
            show(
                &mut it,
                "(define p (make-parameter 'outer)) (define log '())
                 (call/cc (lambda (k)
                   (with-exception-handler
                     (lambda (e) (k (list (p) log)))
                     (lambda ()
                       (parameterize ([p 'inner])
                         (dynamic-wind (lambda () #f)
                                       (lambda () (error \"x\"))
                                       (lambda () (set! log (cons 'after log)))))))))"
            ),
            "(inner ())",
            "{tier:?}"
        );
        assert_eq!(show(&mut it, "log"), "(after)", "{tier:?}");
    }
}

#[test]
fn a_rust_refusal_reaches_with_exception_handler_too() {
    for tier in TIERS {
        let mut it = interp(tier);
        assert_eq!(
            show(
                &mut it,
                "(call/cc (lambda (k)
                   (with-exception-handler
                     (lambda (e) (k (list (error-object? e) (error-object-message e) (error-object-irritants e))))
                     (lambda () (car 5)))))"
            ),
            "(#t \"car: not a pair: 5\" ())",
            "{tier:?}"
        );
        // A host function's refusal, likewise.
        it.define_fn("refuse", |_| Err("refused: no".into())).unwrap();
        assert_eq!(
            show(
                &mut it,
                "(call/cc (lambda (k)
                   (with-exception-handler
                     (lambda (e) (k (error-object-message e)))
                     (lambda () (refuse)))))"
            ),
            "\"refuse: refused: no\"",
            "{tier:?}"
        );
    }
}

#[test]
fn a_handler_that_returns_from_error_leaves_the_error_as_it_was() {
    // `error` is non-continuable: the handler runs, returns, and the error
    // goes on outward. The host sees what it would have seen with no
    // handler -- same variant, same text.
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define seen '())");
        let e = it
            .eval("(with-exception-handler (lambda (e) (set! seen (cons 'h seen)) 0) (lambda () (+ 1 (error \"boom\" 1))))")
            .unwrap_err();
        assert!(matches!(&e, Error::Raised { text, .. } if text == "boom 1"), "{tier:?}: {e:?}");
        let e = it
            .eval("(with-exception-handler (lambda (e) (set! seen (cons 'h seen)) 0) (lambda () (car 5)))")
            .unwrap_err();
        assert_eq!(e, Error::Script("car: not a pair: 5".into()), "{tier:?}");
        assert_eq!(show(&mut it, "seen"), "(h h)", "{tier:?}");
    }
}

#[test]
fn the_secondary_raise_reaches_the_next_handler_out() {
    // R7RS: when a `raise` handler returns, "a secondary exception is raised
    // in the same dynamic environment as the handler" -- the next one out.
    for tier in TIERS {
        let mut it = interp(tier);
        for raiser in ["(raise 'x)", "(error \"x\")"] {
            let src = format!(
                "(call/cc (lambda (k)
                   (with-exception-handler
                     (lambda (e) (k 'outer))
                     (lambda ()
                       (with-exception-handler (lambda (e) 'inner-returned) (lambda () {raiser}))))))"
            );
            assert_eq!(show(&mut it, &src), "outer", "{tier:?} {raiser}");
            // And a guard outside catches it.
            let src = format!(
                "(guard (e (#t 'guarded))
                   (with-exception-handler (lambda (e) 'inner-returned) (lambda () {raiser})))"
            );
            assert_eq!(show(&mut it, &src), "guarded", "{tier:?} {raiser}");
        }
        // raise-continuable is unchanged.
        assert_eq!(
            show(&mut it, "(with-exception-handler (lambda (c) 42) (lambda () (+ 1 (raise-continuable 'c))))"),
            "43",
            "{tier:?}"
        );
    }
}

#[test]
fn guard_clauses_run_after_the_body_has_unwound() {
    for tier in TIERS {
        let mut it = interp(tier);
        show(&mut it, "(define log '()) (define (note x) (set! log (cons x log)))");
        // The after runs BEFORE the clause, for every way in.
        for raiser in ["(raise 'x)", "(error \"x\")", "(car 5)", "(raise-continuable 'x)"] {
            let src = format!(
                "(set! log '())
                 (guard (e (#t (note 'clause)))
                   (dynamic-wind (lambda () #f) (lambda () {raiser}) (lambda () (note 'after))))
                 (reverse log)"
            );
            assert_eq!(show(&mut it, &src), "(after clause)", "{tier:?} {raiser}");
        }
        // A parameterize in the body is not visible to the clauses.
        assert_eq!(
            show(
                &mut it,
                "(define p (make-parameter 'outer))
                 (list (guard (e (#t (p))) (parameterize ([p 'inner]) (raise 'x)))
                       (guard (e (#t (p))) (parameterize ([p 'inner]) (error \"x\"))))"
            ),
            "(outer outer)",
            "{tier:?}"
        );
        // A clause that raises reaches the handler outside the guard, not
        // the guard's own.
        assert_eq!(
            show(
                &mut it,
                "(guard (e (#t (list 'outer e)))
                   (guard (e (#t (raise (list 'again e)))) (raise 'x)))"
            ),
            "(outer (again x))",
            "{tier:?}"
        );
    }
}

#[test]
fn guard_still_returns_every_value_and_re_raises_what_it_does_not_match() {
    for tier in TIERS {
        let mut it = interp(tier);
        assert_eq!(
            show(&mut it, "(call-with-values (lambda () (guard (e (#t 0)) (values 1 2 3))) list)"),
            "(1 2 3)",
            "{tier:?}"
        );
        // No clause matches: re-raised from the guard's dynamic
        // environment, so the inner after has run once and an outer guard
        // sees the same object.
        show(&mut it, "(define log '())");
        assert_eq!(
            show(
                &mut it,
                "(guard (e (#t (list 'outer e log)))
                   (guard (e ((string? e) 'inner))
                     (dynamic-wind (lambda () #f) (lambda () (raise 7)) (lambda () (set! log (cons 'after log))))))"
            ),
            "(outer 7 (after))",
            "{tier:?}"
        );
        // A refusal that no clause matches reaches the host as itself.
        let e = it.eval("(guard (e ((string? e) 'no)) (car 5))").unwrap_err();
        assert_eq!(e, Error::Script("car: not a pair: 5".into()), "{tier:?}");
        let e = it.eval("(guard (e ((string? e) 'no)) (error \"boom\" 1))").unwrap_err();
        assert!(matches!(&e, Error::Raised { text, .. } if text == "boom 1"), "{tier:?}: {e:?}");
        // SRFI-34 example 5: a guard inside a handler wins.
        assert_eq!(
            show(
                &mut it,
                "(with-exception-handler (lambda (c) 'handler)
                   (lambda () (guard (e (#t 'guard)) (raise 'x))))"
            ),
            "guard",
            "{tier:?}"
        );
    }
}

#[test]
fn a_handler_outside_a_host_call_is_not_called_inside_it() {
    // A host function's nested evaluation is its own segment: a condition in
    // a callback goes to handlers installed inside the callback, and
    // otherwise leaves as an `Err` for the host -- which returns it, and only
    // then does the handler outside see it. An outer handler that escaped
    // from inside would unwind past the host's Rust frame.
    for tier in TIERS {
        let mut it = interp(tier);
        it.define_fn_with("call-it", |host, args| {
            let [Val::Proc(f)] = args else { return Err("expects a procedure".into()) };
            host.call(f, &[])
        })
        .unwrap();
        assert_eq!(
            show(
                &mut it,
                "(guard (e (#t (list 'caught (error-object-message e))))
                   (call-it (lambda () (error \"boom\"))))"
            ),
            "(caught \"call-it: boom\")",
            "{tier:?}"
        );
        assert_eq!(
            show(
                &mut it,
                "(call-it (lambda () (guard (e (#t (list 'inside e))) (raise 'x))))"
            ),
            "(inside x)",
            "{tier:?}"
        );
        // An inner handler that returns: the error leaves the callback.
        assert_eq!(
            show(
                &mut it,
                "(define hits '())
                 (guard (e (#t (list 'outer (error-object-message e) hits)))
                   (call-it (lambda ()
                     (with-exception-handler (lambda (e) (set! hits (cons 'inner hits)) 0)
                                             (lambda () (car 5))))))"
            ),
            "(outer \"call-it: car: not a pair: 5\" (inner))",
            "{tier:?}"
        );
        assert_eq!(show(&mut it, "(+ 1 2)"), "3", "{tier:?}");
    }
}

#[test]
fn an_after_that_raises_under_a_guard_is_caught_by_it() {
    // The guard's escape is in progress when the after raises; the guard's
    // continuation must still be live for its handler to escape to again.
    for tier in TIERS {
        let mut it = interp(tier);
        assert_eq!(
            show(
                &mut it,
                "(define log '())
                 (list (guard (e (#t (list 'caught e)))
                         (dynamic-wind (lambda () #f)
                           (lambda ()
                             (dynamic-wind (lambda () #f)
                                           (lambda () (raise 'first))
                                           (lambda () (set! log (cons 'a log)) (raise 'second))))
                           (lambda () (set! log (cons 'b log)))))
                       (reverse log))"
            ),
            "((caught second) (a b))",
            "{tier:?}"
        );
    }
}

#[test]
fn guard_and_handlers_survive_gc_stress() {
    for tier in TIERS {
        let mut it = interp(tier);
        it.vm_mut().engine.set_gc_stress(true);
        assert_eq!(
            show(
                &mut it,
                "(let loop ((i 0) (acc '()))
                   (if (= i 20)
                       (list (length acc) (car acc))
                       (loop (+ i 1)
                             (cons (call/cc (lambda (k)
                                     (with-exception-handler
                                       (lambda (e) (k (guard (e2 (#t (error-object-message e2)))
                                                        (error (format \"n ~a\" i)))))
                                       (lambda () (car i)))))
                                   acc))))"
            ),
            "(20 \"n 19\")",
            "{tier:?}"
        );
    }
}
