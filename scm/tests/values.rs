//! Multiple values.
//!
//! The one design decision: **a single value is not wrapped.** `(values x)`
//! *is* `x`, so a procedure that returns normally and one that returns one
//! value are indistinguishable, which is what R7RS requires and what keeps
//! `call-with-values` usable on any producer.

use scm::{Vm, write_str};

#[track_caller]
fn eval(src: &str) -> String {
    let mut vm = Vm::new();
    let v = vm
        .eval_str(src)
        .unwrap_or_else(|e| panic!("eval {src:?}: {e}"));
    write_str(&vm.engine, v)
}

#[test]
fn one_value_is_just_the_value() {
    assert_eq!(eval("(values 1)"), "1");
    assert_eq!(eval("(+ 1 (values 2))"), "3");
    // So a producer need not know it is one.
    assert_eq!(eval("(call-with-values (lambda () 7) list)"), "(7)");
    assert_eq!(eval("(call-with-values (lambda () (values 7)) list)"), "(7)");
}

#[test]
fn call_with_values_passes_them_all() {
    assert_eq!(eval("(call-with-values (lambda () (values 1 2)) list)"), "(1 2)");
    assert_eq!(eval("(call-with-values (lambda () (values)) list)"), "()");
    assert_eq!(eval("(call-with-values (lambda () (values 1 2)) +)"), "3");
    assert_eq!(
        eval("(call-with-values (lambda () (values 1 2 3)) (lambda (a . r) (list a r)))"),
        "(1 (2 3))"
    );
}

#[test]
fn let_values_binds_in_parallel() {
    assert_eq!(
        eval("(let-values ([(a b) (values 1 2)] [(c) (values 3)]) (list a b c))"),
        "(1 2 3)"
    );
    // Parallel, not sequential: the second expression must NOT see the first
    // binding, so a shadowed outer name still means the outer one.
    assert_eq!(
        eval("(define x 'outer)
              (let-values ([(x) (values 'inner)] [(y) x]) (list x y))"),
        "(inner outer)"
    );
    assert_eq!(eval("(let-values () 42)"), "42");
    // A rest formal.
    assert_eq!(
        eval("(let-values ([(a . r) (values 1 2 3)]) (list a r))"),
        "(1 (2 3))"
    );
}

#[test]
fn let_star_values_binds_sequentially() {
    // The difference from `let-values`, and the whole reason both exist.
    assert_eq!(
        eval("(let*-values ([(a) (values 1)] [(b) (values (+ a 1))]) (list a b))"),
        "(1 2)"
    );
    assert_eq!(
        eval("(define x 'outer)
              (let*-values ([(x) (values 'inner)] [(y) x]) (list x y))"),
        "(inner inner)"
    );
}

#[test]
fn define_values_defines_at_the_current_level() {
    assert_eq!(eval("(define-values (x y) (values 10 20)) (list x y)"), "(10 20)");
    assert_eq!(eval("(define-values (x) (values 1)) x"), "1");
    // Inside a body, so it is a definition rather than a lambda binding.
    assert_eq!(
        eval("(define (f) (define-values (a b) (values 1 2)) (+ a b)) (f)"),
        "3"
    );
}

#[test]
fn receive_is_the_readable_one_binding_case() {
    assert_eq!(eval("(receive (a b) (values 1 2) (list a b))"), "(1 2)");
    assert_eq!(eval("(receive (a . r) (values 1 2 3) (list a r))"), "(1 (2 3))");
    assert_eq!(eval("(receive all (values 1 2) all)"), "(1 2)");
}

#[test]
fn values_leaking_into_a_single_value_context_is_visible() {
    // R7RS makes this an error. Here it arrives as an opaque object, so the
    // printer names it rather than letting it look like something ordinary --
    // a silent `(1 2)` would be much worse.
    assert_eq!(eval("(values 1 2)"), "#<values 1 2>");
    assert_eq!(eval("(values)"), "#<values>");
}

#[test]
fn the_hygiene_in_let_values_is_doing_real_work() {
    // `%lv` spells its temporary `t` at every recursive level, which is only
    // safe because each expansion renames it. A user's own `t` must survive,
    // and so must several bindings at once.
    assert_eq!(
        eval("(let ([t 'mine])
                (let-values ([(a) (values 1)] [(b) (values 2)] [(c) (values 3)])
                  (list t a b c)))"),
        "(mine 1 2 3)"
    );
}

#[test]
fn values_survive_gc_stress() {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            "(let loop ((i 0) (acc '()))
               (if (= i 30)
                   (length acc)
                   (let-values ([(a b) (values i (* i 2))])
                     (loop (+ i 1) (cons (+ a b) acc)))))",
        )
        .expect("values under gc stress");
    assert_eq!(write_str(&vm.engine, v), "30");
}
