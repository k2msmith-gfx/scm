//! `quasiquote`, expanded into ordinary code rather than interpreted.
//!
//! It needs no frame of its own: the template becomes a tree of `%qq-cons`,
//! `%qq-append` and `%qq-list->vector` calls, which the machine then evaluates
//! like anything else. The only part worth being careful about is depth --
//! only an unquote at depth 1 evaluates, and deeper ones are rebuilt as data.

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
fn a_template_with_no_unquote_is_just_a_quote() {
    assert_eq!(eval("`a"), "a");
    assert_eq!(eval("`(1 2 3)"), "(1 2 3)");
    assert_eq!(eval("`()"), "()");
    assert_eq!(eval("`\"s\""), "\"s\"");
    assert_eq!(eval("`(a (b (c)))"), "(a (b (c)))");
}

#[test]
fn unquote_evaluates() {
    assert_eq!(eval("(define x 2) `(a ,x b)"), "(a 2 b)");
    assert_eq!(eval("`(1 ,(+ 1 1) 3)"), "(1 2 3)");
    assert_eq!(eval("(define x 2) `,x"), "2");
    // In a nested list, and in an improper tail.
    assert_eq!(eval("(define x 2) `(a (b ,x))"), "(a (b 2))");
    assert_eq!(eval("`(1 . ,(+ 1 1))"), "(1 . 2)");
}

#[test]
fn unquote_splicing_splices() {
    assert_eq!(eval("(define l (list 1 2)) `(a ,@l b)"), "(a 1 2 b)");
    assert_eq!(eval("(define l '()) `(a ,@l b)"), "(a b)");
    assert_eq!(eval("(define l (list 1 2)) `(,@l)"), "(1 2)");
    assert_eq!(eval("(define l (list 1)) `(,@l . 9)"), "(1 . 9)");
    // Splicing a vector is an extension, and useful once a scene has
    // accumulated its points into one.
    assert_eq!(eval("`(a ,@[1 2] b)"), "(a 1 2 b)");
}

#[test]
fn vectors_keep_the_brackets_they_were_written_with() {
    // Plan §4.5: `[…]` and `#(…)` mean different things, so a template must
    // rebuild the kind it started as.
    assert_eq!(eval("(define x 5) `[1 ,x]"), "[1 5]");
    assert_eq!(eval("(define x 5) `#(1 ,x)"), "#(1 5)");
    assert_eq!(eval("(define l (list 2 3)) `[1 ,@l]"), "[1 2 3]");
    assert_eq!(eval("`[[1] [2]]"), "[[1] [2]]");
}

#[test]
fn nesting_tracks_depth() {
    // The part that catches a naive implementation. An unquote inside a
    // *nested* quasiquote is data, not code -- unless it is itself nested one
    // level deeper, in which case it evaluates.
    assert_eq!(eval("`(a `(b ,(c)))"), "(a `(b ,(c)))");
    assert_eq!(eval("(define x 9) `(1 `(2 ,(3 ,x)))"), "(1 `(2 ,(3 9)))");
    assert_eq!(eval("(define x 1) `(a ,x `(b ,x))"), "(a 1 `(b ,x))");
    // R7RS's own example.
    assert_eq!(
        eval("(define name 'foo) `(a `(b ,(c ,(+ 1 2))))"),
        "(a `(b ,(c 3)))"
    );
}

#[test]
fn the_runtime_helpers_cannot_be_shadowed() {
    // The expansion is built in Rust, so if it referenced `cons` and
    // `append` by their ordinary names a local rebinding would change what a
    // template means. It uses `%qq-…` names instead.
    assert_eq!(
        eval("(define x 2) (let ([cons list] [append list] [list->vector list]) `(a ,x))"),
        "(a 2)"
    );
}

#[test]
fn a_template_works_inside_a_macro() {
    // Which is most of what quasiquote is for -- and it has to survive the
    // renaming, since the template's `quasiquote` is itself a renamed
    // identifier by the time it is seen.
    let src = "(define-syntax swap2
                 (syntax-rules () ((_ a b) `(,b ,a))))
               (swap2 1 2)";
    assert_eq!(eval(src), "(2 1)");
}

#[test]
fn quasiquote_survives_gc_stress() {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            "(define (row i) `(row ,i ,@(list (* i 2) (* i 3))))
             (let loop ((i 0) (acc '()))
               (if (= i 20) (length acc) (loop (+ i 1) (cons (row i) acc))))",
        )
        .expect("quasiquote under gc stress");
    assert_eq!(write_str(&vm.engine, v), "20");
}
