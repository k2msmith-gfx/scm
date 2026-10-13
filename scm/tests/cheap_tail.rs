//! The batch of procedures the conformance suite named, plus the two bugs it
//! found on the way in.

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
fn numeric_predicates_say_what_is_actually_true() {
    // Without `ratios` every number is an exact integer or a flonum, so these
    // answer about *this* implementation rather than about a tower it does
    // not have.
    assert_eq!(eval("(list (real? 1) (real? 1.5) (real? 'a))"), "(#t #t #f)");
    assert_eq!(eval("(list (integer? 2) (integer? 2.0) (integer? 2.5))"), "(#t #t #f)");
    assert_eq!(eval("(list (rational? 1) (rational? (/ 0.0 0.0)))"), "(#t #f)");
    assert_eq!(eval("(list (exact? 1) (inexact? 1.0))"), "(#t #t)");
    assert_eq!(eval("(list (finite? 1.0) (infinite? (/ 1.0 0.0)))"), "(#t #t)");
    assert_eq!(eval("(list (numerator 3) (denominator 3))"), "(3 1)");
}

#[test]
fn integer_arithmetic() {
    assert_eq!(eval("(list (gcd 12 18) (gcd) (lcm 4 6) (lcm 4 0))"), "(6 0 12 0)");
    assert_eq!(eval("(square 5)"), "25");
    assert_eq!(eval("(exact 2.0)"), "2");
    assert!(eval("(guard (e (#t (error-object-message e))) (exact 2.5))").contains("ratios"));
}

#[test]
fn exact_integer_sqrt_does_not_overflow() {
    // It did: `(r + 1) * (r + 1)` wrapped negative near i64::MAX, so the
    // search loop never ended. The suite tests the boundary and found it by
    // hanging the whole run.
    assert_eq!(eval("(call-with-values (lambda () (exact-integer-sqrt 17)) list)"), "(4 1)");
    assert_eq!(eval("(call-with-values (lambda () (exact-integer-sqrt 0)) list)"), "(0 0)");
    assert_eq!(
        eval("(call-with-values (lambda () (exact-integer-sqrt 9223372036854775807)) list)"),
        "(3037000499 5928526806)"
    );
}

#[test]
fn the_division_operators_agree_on_their_signs() {
    // `floor-remainder` takes the sign of the divisor and
    // `truncate-remainder` that of the dividend -- the same distinction
    // `modulo` got wrong, so these are tested against both negatives.
    assert_eq!(eval("(list (floor-quotient 13 -4) (floor-remainder 13 -4))"), "(-4 -3)");
    assert_eq!(
        eval("(list (truncate-quotient 13 -4) (truncate-remainder 13 -4))"),
        "(-3 1)"
    );
    assert_eq!(eval("(call-with-values (lambda () (floor/ 13 -4)) list)"), "(-4 -3)");
    assert_eq!(eval("(call-with-values (lambda () (truncate/ 13 -4)) list)"), "(-3 1)");
}

#[test]
fn character_and_string_case_operations() {
    assert_eq!(eval("(list (char<=? #\\a #\\a) (char>=? #\\b #\\a))"), "(#t #t)");
    assert_eq!(eval("(list (char-ci=? #\\A #\\a) (char-ci<? #\\a #\\B))"), "(#t #t)");
    assert_eq!(
        eval("(list (char-upper-case? #\\A) (char-lower-case? #\\A) (char-foldcase #\\A))"),
        "(#t #f #\\a)"
    );
    assert_eq!(eval("(list (digit-value #\\7) (digit-value #\\a))"), "(7 #f)");
    assert_eq!(eval(r#"(list (string<=? "a" "a") (string>=? "b" "a"))"#), "(#t #t)");
    assert_eq!(eval(r#"(list (string-ci<? "A" "b") (string-foldcase "AbC"))"#), "(#t \"abc\")");
    assert_eq!(eval(r#"(string-map char-upcase "abc")"#), "\"ABC\"");
    assert_eq!(
        eval(r#"(let ([n 0]) (string-for-each (lambda (c) (set! n (+ n 1))) "abc") n)"#),
        "3"
    );
}

#[test]
fn in_place_copies_and_conversions() {
    assert_eq!(
        eval(r#"(let ([s (make-string 5 #\.)]) (string-copy! s 1 "ab") s)"#),
        "\".ab..\""
    );
    assert_eq!(
        eval("(let ([v (make-vector 4 0)]) (vector-copy! v 1 [7 8]) v)"),
        "[0 7 8 0]"
    );
    assert_eq!(eval("(vector-append [1] [2 3])"), "[1 2 3]");
    assert_eq!(eval(r#"(string->vector "ab")"#), "[#\\a #\\b]");
    assert_eq!(eval(r#"(vector->string [#\a #\b])"#), "\"ab\"");
    assert_eq!(eval("(list-copy '(1 2 3))"), "(1 2 3)");
}

#[test]
fn mutable_pairs() {
    assert_eq!(eval("(let ([p (cons 1 2)]) (set-car! p 9) p)"), "(9 . 2)");
    assert_eq!(eval("(let ([p (cons 1 2)]) (set-cdr! p 9) p)"), "(1 . 9)");
}

#[test]
fn write_terminates_on_a_cycle() {
    // `set-cdr!` made cycles constructible for the first time, and the
    // writer looped forever -- it hung the entire conformance run. R7RS wants
    // datum labels, and only where a cycle makes the output infinite.
    assert_eq!(
        eval("(let ([c (list 1)]) (set-cdr! c c) (with-output-to-string (lambda () (write c))))"),
        "\"#0=(1 . #0#)\""
    );
    // Shared but ACYCLIC structure is printed twice, not labelled -- which is
    // what distinguishes `write` from `write-shared`.
    assert_eq!(
        eval("(let ([x (list 1 2 3)])
                (with-output-to-string (lambda () (write (list x x)))))"),
        "\"((1 2 3) (1 2 3))\""
    );
    assert_eq!(
        eval("(let ([x (list 1 2 3)])
                (with-output-to-string (lambda () (write-shared (list x x)))))"),
        "\"(#0=(1 2 3) #0#)\""
    );
    // A cycle through a vector, and one in the car rather than the cdr.
    assert_eq!(
        eval("(let ([v (make-vector 1 0)]) (vector-set! v 0 v)
                (with-output-to-string (lambda () (write v))))"),
        "\"#0=[#0#]\""
    );
    assert_eq!(
        eval("(let ([p (cons 1 2)]) (set-car! p p)
                (with-output-to-string (lambda () (write p))))"),
        "\"#0=(#0# . 2)\""
    );
}

#[test]
fn equality_on_booleans_and_symbols() {
    assert_eq!(eval("(list (boolean=? #t #t #t) (boolean=? #t #f))"), "(#t #f)");
    assert_eq!(eval("(list (symbol=? 'a 'a) (symbol=? 'a 'b))"), "(#t #f)");
}

#[test]
fn bytevectors() {
    assert_eq!(eval("(bytevector 1 2 3)"), "#u8(1 2 3)");
    assert_eq!(eval("(make-bytevector 2 7)"), "#u8(7 7)");
    assert_eq!(eval("(list (bytevector? (bytevector)) (bytevector? [1]))"), "(#t #f)");
    assert_eq!(eval("(bytevector-length (bytevector 1 2))"), "2");
    assert_eq!(eval("(bytevector-u8-ref (bytevector 9 8) 1)"), "8");
    assert_eq!(
        eval("(let ([b (make-bytevector 2 0)]) (bytevector-u8-set! b 0 5) b)"),
        "#u8(5 0)"
    );
    assert_eq!(eval("(bytevector-copy (bytevector 1 2 3) 1)"), "#u8(2 3)");
    assert_eq!(eval("(bytevector-append (bytevector 1) (bytevector 2))"), "#u8(1 2)");
    // Round-trips through UTF-8, including a multi-byte character.
    assert_eq!(eval(r#"(utf8->string (string->utf8 "héllo"))"#), "\"héllo\"");
    assert_eq!(eval(r#"(bytevector-length (string->utf8 "é"))"#), "2");
}

#[test]
fn promises_are_memoised() {
    assert_eq!(eval("(force (delay (+ 1 2)))"), "3");
    assert_eq!(eval("(force 7)"), "7");
    assert_eq!(eval("(force (make-promise 7))"), "7");
    assert_eq!(eval("(list (promise? (delay 1)) (promise? 1))"), "(#t #f)");
    // Forced at most once, which is the whole point of a promise.
    assert_eq!(
        eval("(define n 0)
              (define p (delay (begin (set! n (+ n 1)) n)))
              (list (force p) (force p) n)"),
        "(1 1 1)"
    );
}

#[test]
fn case_lambda_dispatches_on_arity() {
    let f = "(define f (case-lambda ((a) (list 1 a)) ((a b) (list 2 a b)) (args (list 'rest args))))";
    assert_eq!(eval(&format!("{f} (f 9)")), "(1 9)");
    assert_eq!(eval(&format!("{f} (f 9 8)")), "(2 9 8)");
    assert_eq!(eval(&format!("{f} (f 1 2 3)")), "(rest (1 2 3))");
}

#[test]
fn eval_evaluates_a_computed_form() {
    // The form is a value at run time, so this needs the machine -- but only
    // to evaluate it, so it is a step rather than a frame.
    assert_eq!(eval("(eval (list '+ 1 2) (interaction-environment))"), "3");
    assert_eq!(eval("(eval '(let ([x 2]) (* x x)) (interaction-environment))"), "4");
    assert_eq!(eval("(eval 42 (interaction-environment))"), "42");
}

#[test]
fn the_batch_survives_gc_stress() {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            r#"(let loop ((i 1) (acc '()))
                 (if (= i 20)
                     (length acc)
                     (loop (+ i 1)
                           (cons (list (gcd i 12)
                                       (force (delay (square i)))
                                       (utf8->string (string->utf8 "x")))
                                 acc))))"#,
        )
        .expect("the batch under gc stress");
    assert_eq!(write_str(&vm.engine, v), "19");
}
