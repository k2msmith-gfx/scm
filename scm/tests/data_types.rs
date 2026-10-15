//! S5, slice 2: vectors, strings, characters, `format`, errors.
//!
//! The survey's §7 checklist is what this is measured against, not R7RS's
//! table of contents -- three of these entries exist because ray's scene
//! corpus needs them and the standard does not have them.

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

// ---- vectors, and the growable addition --------------------------------

#[test]
fn vector_basics() {
    assert_eq!(eval("(make-vector 3 0)"), "#(0 0 0)");
    assert_eq!(eval("(make-vector 2)"), "#(#f #f)");
    assert_eq!(eval("(let ([v [1 2 3]]) (vector-set! v 1 'x) v)"), "#(1 x 3)");
    assert_eq!(eval("(vector->list [1 2 3])"), "(1 2 3)");
    assert_eq!(eval("(list->vector '(1 2 3))"), "#(1 2 3)");
    assert_eq!(eval("(vector-copy [1 2])"), "#(1 2)");
    assert_eq!(eval("(let ([v [1 2]]) (vector-fill! v 9) v)"), "#(9 9)");
    assert_eq!(eval("(vector-map (lambda (x) (* x x)) [1 2 3])"), "#(1 4 9)");
    assert_eq!(eval("(vector? [1 2])"), "#t");
    assert_eq!(eval("(vector? '(1 2))"), "#f");
    assert_eq!(eval("(vector? #(1 2))"), "#t");
}

#[test]
fn vectors_grow() {
    // R7RS vectors are fixed-length and the standard has no growable
    // sequence, but the corpus does `array/push` 248 times across 48 files
    // (survey §2). One vector type that grows beat a second type with a
    // wrapper, which is what the survey had recommended.
    assert_eq!(
        eval("(let ([v (make-vector 0)]) (vector-push! v 1) (vector-push! v 2) v)"),
        "#(1 2)"
    );
    // The idiom the conversion will produce, one-for-one with `array/push`.
    assert_eq!(
        eval("(let ([pts (make-vector 0)])
                (for i 0 4 (vector-push! pts [i 0 i]))
                (vector-length pts))"),
        "4"
    );
    assert_eq!(eval("(vector-push! (make-vector 0) 1 2 3)"), "#(1 2 3)");
    assert_eq!(eval("(let ([v [1 2]]) (vector-pop! v))"), "2");
    assert!(eval_err("(vector-pop! (make-vector 0))").contains("empty vector"));
    // It is still the same type, so `vector?` and the rest keep working.
    assert_eq!(
        eval("(let ([v (make-vector 0)]) (vector-push! v 1) (vector? v))"),
        "#t"
    );
}

#[test]
fn the_recommended_idiom_is_still_the_standard_one() {
    // Plan §4.9: the growable vector is a compatibility path, and
    // accumulate-and-reverse is what a model writes -- so that path must be
    // first-class, which means flat.
    let mut vm = Vm::new();
    let before = vm.stack_high_water();
    vm.eval_str(
        "(define pts
           (let loop ((i 0) (acc '()))
             (if (= i 20000) (reverse acc) (loop (+ i 1) (cons i acc)))))",
    )
    .expect("accumulate");
    assert!(
        vm.stack_high_water() - before < 100,
        "accumulate-and-reverse grew the stack by {}",
        vm.stack_high_water() - before
    );
    assert_eq!(
        vm.eval_str("(length pts)")
            .map(|v| write_str(&vm.engine, v))
            .unwrap(),
        "20000"
    );
}

// ---- strings -----------------------------------------------------------

#[test]
fn string_basics() {
    assert_eq!(eval(r#"(string-length "hello")"#), "5");
    assert_eq!(eval(r#"(string-ref "hello" 1)"#), "#\\e");
    assert_eq!(eval(r#"(string-append "a" "b" "c")"#), "\"abc\"");
    assert_eq!(eval(r#"(substring "hello" 1 3)"#), "\"el\"");
    assert_eq!(eval(r#"(substring "hello" 2)"#), "\"llo\"");
    assert_eq!(eval(r#"(string->list "ab")"#), "(#\\a #\\b)");
    assert_eq!(eval(r#"(list->string (list #\a #\b))"#), "\"ab\"");
    assert_eq!(eval(r#"(string-upcase "aB")"#), "\"AB\"");
    assert_eq!(eval(r#"(string=? "a" "a" "a")"#), "#t");
    assert_eq!(eval(r#"(string<? "a" "b")"#), "#t");
    assert_eq!(eval(r#"(string-ci=? "AB" "ab")"#), "#t");
    assert_eq!(eval("(make-string 3 #\\x)"), "\"xxx\"");
    assert_eq!(
        eval(r#"(let ([s (make-string 2 #\a)]) (string-set! s 1 #\b) s)"#),
        "\"ab\""
    );
    assert_eq!(eval("(string? \"x\")"), "#t");
    assert_eq!(eval("(string-join (list \"a\" \"b\") \"-\")"), "\"a-b\"");
}

#[test]
fn r7rs_string_takes_characters_and_says_so() {
    // Janet's `string` stringifies and concatenates; R7RS's takes characters.
    // §4.9 says the standard spelling wins, so the error has to point at the
    // right alternative rather than just refusing -- 75 corpus uses of
    // `(string a b)` will hit this during conversion.
    assert_eq!(eval(r#"(string #\a #\b)"#), "\"ab\"");
    let e = eval_err(r#"(string "a" "b")"#);
    assert!(e.contains("expects characters"), "got {e}");
    assert!(e.contains("string-append"), "the error should say what to use: {e}");
    // And the converted form works.
    assert_eq!(eval(r#"(string-concat "n=" 2 " " :aces)"#), "\"n=2 :aces\"");
}

#[test]
fn conversions_between_the_name_ish_types() {
    assert_eq!(eval(r#"(string->symbol "abc")"#), "abc");
    assert_eq!(eval("(symbol->string 'abc)"), "\"abc\"");
    // Janet's `(keyword …)`, 101 corpus uses.
    assert_eq!(eval(r#"(string->keyword "at")"#), ":at");
    assert_eq!(eval("(keyword->string :at)"), "\"at\"");
    assert_eq!(eval("(eq? (string->keyword \"at\") :at)"), "#t");
    // Janet's `scan-number`, 45 corpus uses. R7RS returns #f rather than
    // raising, which is what makes it usable on untrusted text.
    assert_eq!(eval(r#"(string->number "42")"#), "42");
    assert_eq!(eval(r#"(string->number "-2.5e3")"#), "-2500.0");
    assert_eq!(eval(r#"(string->number "not a number")"#), "#f");
    assert_eq!(eval(r#"(string->number "")"#), "#f");
    assert_eq!(eval("(number->string 1.5)"), "\"1.5\"");
    // The flonum round trip holds here too: 1.0 must not come back as "1".
    assert_eq!(eval("(number->string 1.0)"), "\"1.0\"");
}

// ---- characters --------------------------------------------------------

#[test]
fn character_operations() {
    assert_eq!(eval("(char->integer #\\A)"), "65");
    assert_eq!(eval("(integer->char 65)"), "#\\A");
    assert_eq!(eval("(char=? #\\a #\\a)"), "#t");
    assert_eq!(eval("(char<? #\\a #\\b)"), "#t");
    assert_eq!(eval("(char-upcase #\\a)"), "#\\A");
    assert_eq!(eval("(char-alphabetic? #\\a)"), "#t");
    assert_eq!(eval("(char-numeric? #\\5)"), "#t");
    assert_eq!(eval("(char-whitespace? #\\space)"), "#t");
    assert_eq!(eval("(char? #\\a)"), "#t");
    assert!(eval_err("(integer->char -1)").contains("Unicode scalar value"));
}

// ---- format ------------------------------------------------------------

#[test]
fn format_uses_the_scheme_spelling() {
    // SRFI-28's `~a`/`~s`, not Janet's `%s` -- a model writing Scheme writes
    // the former (plan §4.9).
    assert_eq!(eval(r#"(format "~a" 42)"#), "\"42\"");
    assert_eq!(eval(r#"(format "~s" "x")"#), r#""\"x\"""#);
    assert_eq!(eval(r#"(format "~a" "x")"#), "\"x\"");
    assert_eq!(eval(r#"(format "a~~b")"#), "\"a~b\"");
    assert_eq!(eval(r#"(format "~a and ~a" 1 2)"#), "\"1 and 2\"");
    assert_eq!(eval(r#"(format "~a" :aces)"#), "\":aces\"");
    assert_eq!(eval(r#"(format "~a" '(1 2))"#), "\"(1 2)\"");
}

#[test]
fn format_has_fixed_precision_because_a_readout_needs_it() {
    // SRFI-28 has no precision directive, and the modeler formats one on
    // every drag event. `~,3f` is Common Lisp's spelling, which a model also
    // knows (survey §3).
    assert_eq!(eval(r#"(format "~,3f" 3.14159)"#), "\"3.142\"");
    assert_eq!(eval(r#"(format "~,0f" 2.5)"#), "\"2\"");
    assert_eq!(eval(r#"(format "~,2f" 1)"#), "\"1.00\"");
    assert_eq!(
        eval(r#"(format "move ~a: ~,3f" :dx 1.23456)"#),
        "\"move :dx: 1.235\""
    );
}

#[test]
fn format_complains_about_its_own_misuse() {
    assert!(eval_err(r#"(format "~a ~a" 1)"#).contains("not enough arguments"));
    assert!(eval_err(r#"(format "~a" 1 2)"#).contains("left over"));
    assert!(eval_err(r#"(format "~q" 1)"#).contains("unknown directive"));
    assert!(eval_err(r#"(format "~")"#).contains("at the end"));
}

// ---- errors, and the predicates ---------------------------------------

#[test]
fn error_is_a_refusal_not_a_crash() {
    let e = eval_err(r#"(error "bad thing" 42 '(1 2))"#);
    assert!(e.contains("bad thing"), "got {e}");
    assert!(e.contains("42"), "irritants should be included: {e}");
    // And the VM is still usable, which is half of requirement 8.
    let mut vm = Vm::new();
    assert!(vm.eval_str(r#"(error "no")"#).is_err());
    assert_eq!(
        vm.eval_str("(+ 1 2)").map(|v| write_str(&vm.engine, v)).unwrap(),
        "3"
    );
}

#[test]
fn assert_names_the_expression_that_failed() {
    // 11 corpus files use `assert`. The expression is quoted into the
    // message, which is only possible because `quote` strips renames.
    let e = eval_err("(assert (= 1 2))");
    assert!(e.contains("assertion failed"), "got {e}");
    assert!(e.contains("(= 1 2)"), "should name the expression: {e}");
    assert_eq!(eval("(assert (= 1 1))"), "#<unspecified>");
}

#[test]
fn the_remaining_predicates() {
    assert_eq!(eval("(boolean? #f)"), "#t");
    assert_eq!(eval("(procedure? car)"), "#t");
    assert_eq!(eval("(procedure? (lambda (x) x))"), "#t");
    assert_eq!(eval("(procedure? 1)"), "#f");
    assert_eq!(eval("(exact-integer? 1)"), "#t");
    assert_eq!(eval("(exact-integer? 1.0)"), "#f");
    assert_eq!(eval("(exact? 1)"), "#t");
    assert_eq!(eval("(even? 4)"), "#t");
    assert_eq!(eval("(odd? 3)"), "#t");
    assert_eq!(eval("(positive? 1)"), "#t");
    assert_eq!(eval("(negative? -1)"), "#t");
    assert_eq!(eval("(nan? (/ 0.0 0.0))"), "#t");
}

// ---- the numeric tower, as far as §2.3 claims it ----------------------

#[test]
fn the_numeric_tower() {
    assert_eq!(eval("(sqrt 4)"), "2");
    assert_eq!(eval("(sqrt 2)"), "1.4142135623730951");
    assert_eq!(eval("(expt 2 10)"), "1024");
    assert_eq!(eval("(expt 2.0 0.5)"), "1.4142135623730951");
    assert_eq!(eval("(floor 1.7)"), "1.0");
    assert_eq!(eval("(floor 2)"), "2");
    assert_eq!(eval("(ceiling 1.2)"), "2.0");
    assert_eq!(eval("(truncate -1.7)"), "-1.0");
    // R7RS `round` rounds to even.
    assert_eq!(eval("(round 2.5)"), "2.0");
    assert_eq!(eval("(round 3.5)"), "4.0");
    assert_eq!(eval("(abs -3)"), "3");
    assert_eq!(eval("(quotient 7 2)"), "3");
    assert_eq!(eval("(remainder 7 2)"), "1");
    assert_eq!(eval("(modulo -7 2)"), "1");
    assert_eq!(eval("(min 1 2.0)"), "1.0");
    assert_eq!(eval("(max 1 2)"), "2");
    assert_eq!(eval("(exact->inexact 1)"), "1.0");
    assert_eq!(eval("(number? 1)"), "#t");
    // R7RS's two-argument atan IS Janet's math/atan2, which is one less
    // thing for a conversion to get wrong.
    assert_eq!(eval("(atan 0 1)"), "0.0");
    assert_eq!(eval("(inc 1)"), "2");
    assert_eq!(eval("(dec 1)"), "0");
}

#[test]
fn everything_survives_gc_stress() {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    // Structural rather than a magic length: the vector grew to 20, every
    // entry formatted, and the join produced each one.
    let v = vm
        .eval_str(
            r#"(define v (make-vector 0))
               (for i 0 20 (vector-push! v (format "~a:~,2f" i (* i 1.5))))
               (list (vector-length v)
                     (vector-ref v 2)
                     (string-length (string-join (vector->list v) ",")))"#,
        )
        .expect("the new layer under gc stress");
    assert_eq!(write_str(&vm.engine, v), "(20 \"2:3.00\" 162)");
}

// ── equal? on graphs ─────────────────────────────────────────────────────
//
// R7RS: "equal? must always terminate even if its arguments contain
// cycles." The R7RS `equal` benchmark found the naive walk looping on a
// cycle and taking 100! steps on a tree with sharing.

#[test]
fn equal_terminates_on_cycles_and_is_linear_on_shared_trees() {
    assert_eq!(
        eval("(let ([a (list 1 2)] [b (list 1 2)] [c (list 1 3)])
                (set-cdr! (cdr a) a) (set-cdr! (cdr b) b) (set-cdr! (cdr c) c)
                (list (equal? a b) (equal? a c) (equal? a (list 1 2))))"),
        "(#t #f #f)"
    );
    // A tree whose unshared size is 60!, compared in well under a second.
    assert_eq!(
        eval("(begin
                (define (ml n x) (if (zero? n) '() (cons x (ml (- n 1) x))))
                (define (mt n) (if (zero? n) '() (ml n (mt (- n 1)))))
                (list (equal? (mt 60) (mt 60)) (equal? (mt 12) (mt 11))))"),
        "(#t #f)"
    );
    // Two cycles of different period that unroll to the same stream are
    // equal? -- the coinductive reading, which is what the standard means.
    assert_eq!(
        eval("(let ([a (list 1 1)] [b (list 1)])
                (set-cdr! (cdr a) a) (set-cdr! b b)
                (equal? a b))"),
        "#t"
    );
}
