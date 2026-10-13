//! S5, slice 3: `make-parameter`, `parameterize`, and a seeded RNG.
//!
//! Two things here are load-bearing for ray rather than for conformance:
//! `parameterize` is what 24 corpus files' `dyn`/`setdyn` become, and the
//! RNG-behind-a-parameter is why a `:seed` replays -- `grow-branches` is only
//! reproducible because ray threads `:ray-rng` through it (survey §4).

use scm::{Vm, write_str};

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
