//! `let` and named `let` are core forms, not macros (plan §6, S5b).
//!
//! The behaviour has to be identical to the macro that used to produce
//! `((lambda (v ...) body) init ...)`, so these are mostly the properties
//! that expansion gave for free and a special form has to earn: parallel
//! binding, a fresh scope even with no bindings, bracket bindings, hygiene
//! when a template introduces the binding, and `let` losing its syntactic
//! meaning when something shadows the name.

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
fn err(src: &str) -> String {
    let mut vm = Vm::new();
    format!(
        "{}",
        vm.eval_str(src).expect_err("this should not evaluate")
    )
}

#[test]
fn bindings_are_parallel_and_evaluated_outside() {
    // The whole difference between `let` and `let*`: every init sees the
    // ENCLOSING scope, so `y` here gets the outer `x`.
    assert_eq!(eval("(let ([x 1]) (let ([x 2] [y x]) (list x y)))"), "(2 1)");
    assert_eq!(eval("(let ([x 1] [y 2]) (+ x y))"), "3");
    // And left to right, which is observable through a side effect.
    assert_eq!(
        eval("(define o '()) (let ([a (begin (set! o (cons 1 o)) 1)]
                                  [b (begin (set! o (cons 2 o)) 2)]) o)"),
        "(2 1)"
    );
}

#[test]
fn no_bindings_still_opens_a_scope() {
    // `(let () ...)` is how an internal `define` is given somewhere to land.
    // A special form that skipped allocating a frame here would put the
    // definition in the enclosing scope instead.
    assert_eq!(eval("(define x 'outer) (let () (define x 'inner) x)"), "inner");
    assert_eq!(eval("(define x 'outer) (let () (define x 'inner) x) x"), "outer");
}

#[test]
fn bracket_bindings_work_everywhere_parens_do() {
    // Plan §4.9: `[x 1]` is a bracket *vector*, and every binding form has to
    // accept it. As a macro the matcher did this; a special form has to.
    assert_eq!(eval("(let ([x 1] (y 2)) (+ x y))"), "3");
    assert_eq!(eval("(let loop ([i 0]) (if (= i 3) i (loop (+ i 1))))"), "3");
}

#[test]
fn a_named_let_loops_and_is_a_real_procedure() {
    assert_eq!(
        eval("(let loop ([i 0] [acc '()]) (if (= i 4) (reverse acc) (loop (+ i 1) (cons i acc))))"),
        "(0 1 2 3)"
    );
    // `loop` is a value, not just a jump target -- a scene can hand it to
    // `map`, so it has to be a closure however it is compiled.
    assert_eq!(eval("(let loop ([i 0]) (procedure? loop))"), "#t");
    assert_eq!(eval("(let loop ([i 1]) (if (= i 1) (map loop '(2)) i))"), "(2)");
}

#[test]
fn a_named_let_keeps_proper_tail_calls() {
    // The macro produced a `letrec` cell and an ordinary call, which the VM
    // already tail-called properly. The core form must not have lost that:
    // 10^6 iterations in a fixed amount of control stack.
    let mut vm = Vm::new();
    let v = vm
        .eval_str("(let loop ([i 0]) (if (= i 1000000) i (loop (+ i 1))))")
        .expect("a long loop must not grow the stack");
    assert_eq!(write_str(&vm.engine, v), "1000000");
    assert!(
        vm.stack_high_water() < 64,
        "control stack reached {}",
        vm.stack_high_water()
    );
}

#[test]
fn a_template_introduced_binding_stays_hygienic() {
    // The reason `let` being a macro was safe: the template's `tmp` was
    // renamed. As a core form the renamed symbol is what gets bound, so the
    // user's own `tmp` must still be untouched.
    let src = "(define-syntax swap!
                 (syntax-rules ()
                   ((_ a b) (let ([tmp a]) (set! a b) (set! b tmp)))))
               (define tmp 1)
               (define other 2)
               (swap! tmp other)
               (list tmp other)";
    assert_eq!(eval(src), "(2 1)");
}

#[test]
fn shadowing_let_makes_it_an_ordinary_variable() {
    // A local called `let` is a variable, not syntax -- the same rule every
    // other special form follows. If the dispatch did not check for a lexical
    // binding first, this would be a syntax error instead of a call.
    assert_eq!(eval("((lambda (let) (let 1 2)) +)"), "3");
    // And the body of a `let` can still refer to a variable named `let`.
    assert_eq!(eval("(let ([let 7]) let)"), "7");
}

#[test]
fn a_malformed_let_says_what_was_wrong() {
    assert!(err("(let ([x]) x)").contains("each binding is"), "{}", err("(let ([x]) x)"));
    assert!(err("(let ([x 1]))").contains("body"), "{}", err("(let ([x 1]))"));
    assert!(err("(let)").contains("binding list"), "{}", err("(let)"));
    // A named `let` reports its own name on an arity mistake, which is what
    // naming the closure buys.
    assert!(
        err("(let loop ([i 0]) (loop 1 2))").contains("loop"),
        "{}",
        err("(let loop ([i 0]) (loop 1 2))")
    );
}

#[test]
fn the_derived_forms_built_on_let_still_work() {
    // `let*`, `letrec` and `do` are still macros and expand to `let`, so this
    // is the check that the core form is a drop-in for what they expect.
    assert_eq!(eval("(let* ([x 1] [y (+ x 1)] [z (+ y 1)]) (list x y z))"), "(1 2 3)");
    assert_eq!(
        eval("(letrec ([even? (lambda (n) (if (= n 0) #t (odd? (- n 1))))]
                       [odd?  (lambda (n) (if (= n 0) #f (even? (- n 1))))])
                (list (even? 10) (odd? 10)))"),
        "(#t #f)"
    );
    assert_eq!(
        eval("(do ([i 0 (+ i 1)] [acc '() (cons i acc)]) ((= i 3) (reverse acc)))"),
        "(0 1 2)"
    );
    assert_eq!(eval("(let-values ([(a b) (values 1 2)]) (list a b))"), "(1 2)");
}
