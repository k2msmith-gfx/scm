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

// ---- the wrong answers the suite reported, one test each --------------

#[test]
fn an_ellipsis_escape_still_substitutes_pattern_variables() {
    // `(... template)` means an ellipsis inside has no special meaning -- but
    // the pattern variables are still substituted. Returning the template
    // untouched left `x` as the symbol `x`.
    let m = "(define-syntax esc (syntax-rules () ((_ x) (quote (... (x ...))))))";
    assert_eq!(eval(&format!("{m} (esc 100)")), "(100 ...)");
}

#[test]
fn a_macro_defining_macro_keeps_its_variables_apart() {
    // The inner pattern's variable is already renamed by the outer expansion,
    // and the inner template may mention the *user's* identifier of the same
    // name. Keying pattern variables by the RESOLVED name collapsed the two,
    // so this gave 1 instead of x.
    let src = "(define-syntax foo
                 (syntax-rules ()
                   ((foo bar y)
                    (define-syntax bar (syntax-rules () ((bar x) 'y))))))
               (foo bar x)
               (bar 1)";
    assert_eq!(eval(src), "x");
}

#[test]
fn mixed_exact_inexact_comparison_is_mathematically_correct() {
    // 9007199254740993 is not representable as an f64, so comparing through
    // f64 made it equal to 9007199254740992.0.
    assert_eq!(eval("(= 9007199254740992.0 9007199254740993)"), "#f");
    assert_eq!(eval("(= 2.0 2)"), "#t");
    assert_eq!(eval("(= 2.5 2)"), "#f");
}

#[test]
fn string_to_number_requires_the_whole_string() {
    assert_eq!(eval(r#"(string->number "1 2")"#), "#f");
    assert_eq!(eval(r#"(string->number "1x")"#), "#f");
    assert_eq!(eval(r#"(string->number "  12  ")"#), "12");
}

#[test]
fn alternate_exponent_markers_are_accepted() {
    // R7RS no longer *requires* s/f/d/l, and they cost three lines -- so an
    // older file or a model that writes `1d2` gets 100.0 and not a symbol.
    assert_eq!(eval("(list 1e2 1s2 1S2 1f2 1d2 1L2)"), "(100.0 100.0 100.0 100.0 100.0 100.0)");
    assert_eq!(eval("(list +InF.0 -iNF.0)"), "(+inf.0 -inf.0)");
}

#[test]
fn a_symbol_spelled_like_a_number_is_barred() {
    // Otherwise `write` emits something the reader reads back as a number,
    // which breaks the round-trip the whole printer is built around.
    for (src, want) in [
        (r#"(string->symbol "+inf.0")"#, "|+inf.0|"),
        (r#"(string->symbol "+i")"#, "|+i|"),
        (r#"(string->symbol "1")"#, "|1|"),
        (r#"(string->symbol ".")"#, "|.|"),
        (r#"(string->symbol "+")"#, "+"),
        (r#"(string->symbol "...")"#, "..."),
    ] {
        assert_eq!(eval(&format!("(write {src})")).trim(), "#<unspecified>");
        assert_eq!(
            eval(&format!("(with-output-to-string (lambda () (write {src})))")),
            format!("\"{}\"", want.replace('\\', "\\\\")),
            "{src}"
        );
    }
}

#[test]
fn flonums_print_in_the_scheme_spelling() {
    // Rust's Display never uses exponent form, so 5e-324 came out as a
    // 324-digit decimal; and Scheme wants a point in the mantissa and a sign
    // on the exponent.
    assert_eq!(eval("5e-324"), "5.0e-324");
    assert_eq!(eval("1.7976931348623157e308"), "1.7976931348623157e+308");
    assert_eq!(eval("1e21"), "1.0e+21");
    // Ordinary magnitudes stay in positional form.
    assert_eq!(eval("(list 1.5 0.001 1e20 1.0 -0.0)"), "(1.5 0.001 100000000000000000000.0 1.0 -0.0)");
}

#[test]
fn ranges_on_the_bytevector_and_string_conversions() {
    assert_eq!(eval(r#"(string->utf8 "ABC" 1)"#), "#u8(66 67)");
    assert_eq!(eval(r#"(string->utf8 "ABC" 1 2)"#), "#u8(66)");
    assert_eq!(eval("(utf8->string (bytevector 0 65 66 67) 1)"), "\"ABC\"");
    assert_eq!(
        eval(r#"(let ([o (open-output-string)]) (write-string "abc def" o 4) (get-output-string o))"#),
        "\"def\""
    );
}

#[test]
fn an_overlapping_bytevector_copy_behaves_as_if_buffered() {
    // Copying forward in place overwrote a byte before it was read:
    // #u8(1 1 1 4 5) instead of #u8(1 1 2 4 5).
    assert_eq!(
        eval("(let ([bv (bytevector 1 2 3 4 5)]) (bytevector-copy! bv 1 bv 0 2) bv)"),
        "#u8(1 1 2 4 5)"
    );
}

#[test]
fn reading_from_a_closed_port_refuses() {
    assert_eq!(
        eval(r#"(let ([in (open-input-string "abc")])
                  (close-input-port in)
                  (guard (e (else 'error)) (read-char in)))"#),
        "error"
    );
}

#[test]
fn features_is_honest() {
    // §2.3's whole obligation for not implementing the numeric tower: the
    // feature list says what is actually true. The absences are the point.
    let f = eval("(features)");
    for present in ["r7rs", "ieee-float"] {
        assert!(f.contains(present), "{present} should be claimed: {f}");
    }
    for absent in ["exact-closed", "ratios", "full-unicode", "exact-complex"] {
        assert!(!f.contains(absent), "{absent} must NOT be claimed: {f}");
    }
}

#[test]
fn the_system_interface() {
    assert_eq!(eval(r#"(string? (get-environment-variable "PATH"))"#), "#t");
    assert_eq!(eval(r#"(get-environment-variable "NO_SUCH_VAR_12345")"#), "#f");
    assert_eq!(eval("(list? (command-line))"), "#t");
    assert_eq!(eval("(inexact? (current-second))"), "#t");
    assert_eq!(eval("(exact? (jiffies-per-second))"), "#t");
    assert_eq!(eval("(file-exists? \".\")"), "#t");
    assert_eq!(eval("(file-exists? \"/no/such/file\")"), "#f");
    // A missing file raises a FILE error, which file-error? identifies --
    // stubbing that would have been a lie dressed as a pass.
    assert_eq!(
        eval(r#"(file-error? (guard (e (else e)) (delete-file "/no/such/file")))"#),
        "#t"
    );
    assert_eq!(
        eval(r#"(file-error? (guard (e (else e)) (open-input-file "/no/such/file")))"#),
        "#t"
    );
}

#[test]
fn a_datum_comment_cannot_comment_out_the_dot() {
    // `#;` comments out the next DATUM, and a lone `.` is not one -- it is a
    // list's tail marker. Treating it as a datum silently dropped the dot and
    // read `(a #;. b)` as `(a b)` instead of refusing it.
    for src in ["(a #;. b)", "(#; #;x #;y . z)", "(#; #;x . z)", "."] {
        assert_eq!(
            eval(&format!("(guard (e (else 'error)) (read (open-input-string {:?})))", src)),
            "error",
            "{src} should not read"
        );
    }
    // The legitimate neighbours still read, which is what makes the above a
    // distinction rather than a blanket refusal.
    assert_eq!(eval(r#"(read (open-input-string "(a . #;b c)"))"#), "(a . c)");
    assert_eq!(eval(r#"(read (open-input-string "(a #; #;b c d)"))"#), "(a d)");
}

#[test]
fn applys_final_list_is_not_optional() {
    // `(apply proc arg1 ... args)` -- the list is part of the signature, so
    // `(apply +)` is an arity error and not a long way to write `(+)`.
    assert_eq!(eval("(guard (e (else 'error)) (apply +))"), "error");
    assert_eq!(eval("(apply + '(1 2))"), "3");
    assert_eq!(eval("(apply + 1 2 '(3))"), "6");
}

#[test]
fn list_p_returns_on_a_circular_list() {
    // R7RS requires `list?` to ANSWER on a cycle, not to hang on one. The
    // obvious cdr-walk spins forever as soon as set-cdr! exists -- which is
    // the same consequence that made `write` need datum labels.
    assert_eq!(eval("(let ((x (list 'a))) (set-cdr! x x) (list? x))"), "#f");
    assert_eq!(
        eval("(let ((x (list 'a 'b 'c))) (set-cdr! (cddr x) x) (list? x))"),
        "#f"
    );
    // The ordinary answers are unchanged, including the improper tail.
    assert_eq!(eval("(list (list? '()) (list? '(1 2 3)) (list? '(a . b)) (list? 4))"), "(#t #t #f #f)");
}
