//! S3's exit criteria: a cyclic structure is collected, there is no leak
//! under `set!` cycles, and the whole thing survives a shrunk heap that
//! collects at every safepoint.
//!
//! Every test here runs with `set_gc_stress(true)` unless it is measuring
//! something about the trigger policy. Collector bugs are not deterministic
//! by nature -- a missed root only shows up once the slot has been reused for
//! something else -- so the way to make them deterministic is to collect
//! constantly and let the reuse happen immediately.

use scm::{Value, Vm, VmError, write_str};

fn stressed() -> Vm {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    vm
}

#[track_caller]
fn eval_stressed(src: &str) -> String {
    let mut vm = stressed();
    let v = vm
        .eval_str(src)
        .unwrap_or_else(|e| panic!("eval {src:?} under gc stress: {e}"));
    write_str(&vm.engine, v)
}

#[test]
fn the_whole_interpreter_survives_collecting_constantly() {
    // If a root is missing anywhere in the machine, this is where it shows:
    // every one of these allocates, and the collector runs between every
    // 1024 steps with nothing but the real root set to go on.
    assert_eq!(eval_stressed("(+ 1 2 3)"), "6");
    assert_eq!(eval_stressed("(define (f x) (* x x)) (f 7)"), "49");
    assert!(
        eval_stressed(
            "(define (loop i acc) (if (= i 0) acc (loop (- i 1) (cons i acc)))) (loop 2000 '())"
        )
        .len()
            > 1000
    );
    assert_eq!(eval_stressed("(define r 2) [0 (* 2 r) 0]"), "[0 4 0]");
    assert_eq!(
        eval_stressed("(define (adder n) (lambda (x) (+ x n))) ((adder 3) 4)"),
        "7"
    );
    assert_eq!(eval_stressed("'(1 (2 (3 (4))))"), "(1 (2 (3 (4))))");
    assert_eq!(
        eval_stressed("(define (sum i) (if (= i 0) 0 (+ i (sum (- i 1))))) (sum 5000)"),
        "12502500"
    );
}

#[test]
fn a_cycle_is_collected() {
    // The reason `Rc` was rejected (plan §3.3). Two lines of Scheme make a
    // cycle, and a reference count never reclaims it.
    let mut vm = Vm::new();
    vm.eval_str("(define a (cons 1 2))").expect("a");
    let before = vm.collect_now().live;

    // `a` now points at itself through its own cdr.
    vm.eval_str("(set! a (cons 1 2))").expect("rebind");
    vm.eval_str("(define (make-cycle) (define p (cons 1 2)) p)")
        .expect("def");
    // Build a cycle and then drop the only reference to it.
    vm.eval_str(
        "(define (churn n)
           (if (= n 0)
               'done
               (begin
                 (define p (cons 1 '()))
                 (churn (- n 1)))))
         (churn 500)",
    )
    .expect("churn");
    let after = vm.collect_now().live;
    assert!(
        after <= before + 64,
        "500 discarded pairs were not reclaimed: {before} -> {after}"
    );
}

#[test]
fn a_set_cycle_does_not_leak() {
    // The explicit version: a closure that captures the environment that
    // holds the closure. A refcount would keep both forever.
    let mut vm = Vm::new();
    vm.eval_str("(define (spin) 0)").expect("warm");
    let base = vm.collect_now().live;
    for _ in 0..200 {
        vm.eval_str(
            "(define (make-self-ref)
               (define me #f)
               (set! me (lambda () me))
               me)
             (make-self-ref)",
        )
        .expect("self ref");
    }
    let after = vm.collect_now().live;
    assert!(
        after < base + 200,
        "self-referential closures leaked: {base} -> {after}"
    );
}

#[test]
fn a_tail_loop_is_flat_in_memory() {
    // The half of S2's exit criterion S2 could not deliver. The same loop at
    // two trip counts a hundred times apart must reach the same live-object
    // count, because each iteration's environment dies the moment the next
    // iteration replaces it.
    let live_after = |n: i64| {
        let mut vm = Vm::new();
        vm.engine.set_min_heap(1024);
        let src = format!("(define (loop i) (if (< i {n}) (loop (+ i 1)) i)) (loop 0)");
        let v = vm.eval_str(&src).expect("loop");
        assert_eq!(v, Value::Fixnum(n));
        (vm.collect_now().live, vm.engine.slot_count())
    };
    let (live_small, slots_small) = live_after(1_000);
    let (live_big, slots_big) = live_after(200_000);
    assert_eq!(
        live_small, live_big,
        "live set grew with the trip count: {live_small} -> {live_big}"
    );
    // And the heap's high-water mark stays bounded, which is the thing that
    // was 8.9 GB before there was a collector.
    assert!(
        slots_big < slots_small * 4,
        "heap high-water grew with the trip count: {slots_small} -> {slots_big}"
    );
}

#[test]
fn slots_are_reused_rather_than_relocated() {
    // Non-moving: a freed slot comes back, so the heap does not keep growing,
    // and an ObjRef that is still live never changes.
    let mut vm = Vm::new();
    vm.engine.set_min_heap(256);
    vm.eval_str("(define keep (cons 'a 'b))").expect("keep");
    let kept = vm.eval_str("keep").expect("read keep");
    let Value::Obj(before) = kept else {
        panic!("expected a heap object")
    };
    for _ in 0..50 {
        vm.eval_str("(define junk (cons 1 (cons 2 (cons 3 '()))))")
            .expect("junk");
    }
    vm.collect_now();
    let still = vm.eval_str("keep").expect("read keep again");
    assert_eq!(
        still,
        Value::Obj(before),
        "a live object moved; the heap is supposed to be non-moving"
    );
    assert_eq!(write_str(&vm.engine, still), "(a . b)");
}

#[test]
fn the_heap_cap_is_an_error_not_an_abort() {
    // Requirement 8: a runaway allocation is a failed evaluation, and the VM
    // is still usable afterwards.
    let mut vm = Vm::new();
    vm.engine.set_min_heap(64);
    vm.engine.set_heap_cap(Some(50_000));
    let err = vm
        .eval_str("(define (grow i acc) (grow (+ i 1) (cons i acc))) (grow 0 '())")
        .expect_err("should hit the cap");
    assert_eq!(err, VmError::OutOfMemory);

    vm.engine.set_heap_cap(None);
    let v = vm.eval_str("(+ 1 2)").expect("still alive");
    assert_eq!(v, Value::Fixnum(3));
}

#[test]
fn source_forms_held_across_an_evaluation_stay_rooted() {
    // `eval_str` holds the forms it has not evaluated yet in a Rust local,
    // which the collector cannot see. Under stress, an unrooted form would be
    // swept between the first and second expression.
    let mut vm = stressed();
    let v = vm
        .eval_str(
            "(define (f x) (cons x x))
             (define a (f 1))
             (define b (f 2))
             (define c (f 3))
             (no-such-procedure 0)
             (cons a (cons b c))",
        )
        .map(|v| write_str(&vm.engine, v));
    // `list?` is not defined yet, so this fails -- but it must fail with the
    // right error rather than crashing on a swept form.
    let err = format!("{}", v.expect_err("the name is deliberately undefined"));
    assert!(
        err.contains("unbound variable: no-such-procedure"),
        "got {err}"
    );

    let mut vm = stressed();
    let out = vm
        .eval_str(
            "(define (f x) (cons x x))
             (define a (f 1))
             (define b (f 2))
             (cons a b)",
        )
        .expect("rooted forms");
    assert_eq!(write_str(&vm.engine, out), "((1 . 1) 2 . 2)");
}

#[test]
fn a_handle_keeps_a_value_alive() {
    let mut vm = Vm::new();
    let v = vm.eval_str("(cons 'x 'y)").expect("pair");
    let h = vm.engine.root(v);
    assert_eq!(vm.engine.rooted_count(), 1);
    // Nothing in Scheme refers to it, so without the handle it would go.
    for _ in 0..100 {
        vm.eval_str("(cons 1 2)").expect("junk");
    }
    vm.collect_now();
    assert_eq!(write_str(&vm.engine, v), "(x . y)");

    vm.engine.unroot(h);
    assert_eq!(vm.engine.rooted_count(), 0);
}

#[test]
fn marking_does_not_recurse_on_a_long_list() {
    // The marker walks a worklist rather than the C stack, so a 200k-element
    // list is a long loop and not a stack overflow. Same reason the
    // interpreter has an explicit stack.
    let mut vm = Vm::new();
    vm.eval_str(
        "(define (build i acc) (if (= i 0) acc (build (- i 1) (cons i acc))))
         (define big (build 200000 '()))",
    )
    .expect("build");
    let stats = vm.collect_now();
    assert!(
        stats.live >= 200_000,
        "the long list should still be live: {stats:?}"
    );
    // And `equal?` on it is iterative too.
    let v = vm.eval_str("(equal? big big)").expect("equal");
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn collection_happens_without_being_asked() {
    // The trigger policy, not the stress mode: allocate well past the floor
    // and the VM should have collected on its own.
    let mut vm = Vm::new();
    vm.engine.set_min_heap(1024);
    vm.eval_str("(define (loop i) (if (= i 0) 'done (begin (cons i i) (loop (- i 1))))) (loop 20000)")
        .expect("loop");
    assert!(
        vm.engine.collections() > 0,
        "nothing triggered a collection in 20k allocations"
    );
    assert!(
        vm.engine.slot_count() < 20_000,
        "the heap grew to {} slots when it should have reused",
        vm.engine.slot_count()
    );
}

#[test]
fn the_expansion_cache_survives_collection_pressure() {
    // The cache is keyed on the macro call's heap cell and is CLEARED by
    // every collection, so under stress it is cold every single time -- which
    // is exactly the case that would expose a dangling key or an expansion
    // freed while it was still being evaluated. If clearing were done after
    // the roots were gathered, or if a cached expansion were not reachable
    // from the step in flight, this is where it would show up.
    let mut vm = stressed();
    let src = "(define (clamp x)
                 (cond [(< x 0) 0]
                       [(> x 10) 10]
                       [else (let ([y (* x 2)]) (when (>= y 0) (quotient y 2)))]))
               (let loop ([i -3] [acc '()])
                 (if (= i 14) (reverse acc) (loop (+ i 1) (cons (clamp i) acc))))";
    let v = vm.eval_str(src).expect("macro-heavy code under gc stress");
    assert_eq!(
        write_str(&vm.engine, v),
        "(0 0 0 0 1 2 3 4 5 6 7 8 9 10 10 10 10)"
    );
}

#[test]
fn evaluating_a_quoted_form_does_not_mutate_it() {
    // The cheap way to memoise an expansion is to overwrite the macro call's
    // own cell in place. This deliberately does not do that: a program can
    // still SEE a quoted form it handed to `eval`, and
    // `(when #t 1)` must not turn into `(if #t (begin 1))` underneath it.
    let mut vm = stressed();
    let src = "(define form '(when #t 42))
               (list (eval form) (eval form) form)";
    let v = vm.eval_str(src).expect("eval a quoted macro use twice");
    assert_eq!(write_str(&vm.engine, v), "(42 42 (when #t 42))");
}
