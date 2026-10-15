//! S9: the rest of R7RS-small.
//!
//! This file starts with the batch that looked like *absences* in the
//! conformance report and were mostly **bugs** — the same shape as the pass
//! that took 65 wrong answers down to 11. Grouping the failures by cause is
//! what showed it: fourteen of them *raised* rather than answering wrongly,
//! which means the feature existed and was broken, not missing.

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
fn err_of(src: &str) -> String {
    let mut vm = Vm::new();
    format!("{}", vm.eval_str(src).expect_err("this should not evaluate"))
}

#[test]
fn string_copy_takes_the_range_r7rs_gives_it() {
    // Six conformance assertions, and not an absence: `string-copy` existed
    // and refused its own second argument.
    assert_eq!(eval(r##"(string-copy "abcd")"##), "\"abcd\"");
    assert_eq!(eval(r##"(string-copy "abcd" 1)"##), "\"bcd\"");
    assert_eq!(eval(r##"(string-copy "abcd" 1 3)"##), "\"bc\"");
    assert_eq!(eval(r##"(string-copy "" 0 0)"##), "\"\"");
    assert!(err_of(r##"(string-copy "ab" 0 9)"##).contains("out of bounds"));
    assert!(err_of(r##"(string-copy "ab" 2 1)"##).contains("out of bounds"));
}

#[test]
fn the_other_two_conversions_take_it_too() {
    assert_eq!(eval(r##"(string->vector "abcd" 1 3)"##), "#(#\\b #\\c)");
    assert_eq!(eval(r##"(string->vector "ab")"##), "#(#\\a #\\b)");
    assert_eq!(eval(r"(vector->string (vector #\a #\b #\c) 1)"), "\"bc\"");
    assert_eq!(eval(r"(vector->string (vector #\a #\b #\c) 0 2)"), "\"ab\"");
}

#[test]
fn vector_map_and_for_each_take_several_vectors() {
    // R7RS, and they stop at the shortest -- like `map`.
    assert_eq!(eval("(vector-map (lambda (x) (* x 10)) (vector 1 2 3))"), "#(10 20 30)");
    assert_eq!(eval("(vector-map + (vector 1 2) (vector 10 20))"), "#(11 22)");
    assert_eq!(
        eval("(vector-map list (vector 1 2 9) (vector 3 4))"),
        "#((1 3) (2 4))"
    );
    assert_eq!(
        eval(
            "(let ([acc '()])
               (vector-for-each (lambda (a b) (set! acc (cons (+ a b) acc)))
                                (vector 1 2) (vector 10 20))
               acc)"
        ),
        "(22 11)"
    );
}

#[test]
fn an_inexact_has_a_numerator_and_a_denominator() {
    // The old code refused these and blamed the absent `ratios` feature,
    // which was the wrong excuse: an f64 is already exactly p / 2^k, so its
    // rational form needs no rational *type*. R7RS wants the answer inexact
    // because the argument was.
    assert_eq!(eval("(list (numerator 5.5) (denominator 5.5))"), "(11.0 2.0)");
    assert_eq!(eval("(list (numerator 0.75) (denominator 0.75))"), "(3.0 4.0)");
    assert_eq!(eval("(list (numerator 2.0) (denominator 2.0))"), "(2.0 1.0)");
    assert_eq!(eval("(list (numerator 6) (denominator 6))"), "(6 1)");
    // What `ratios` genuinely costs is still visible next door: §4.5 makes
    // `(/ 6 4)` the flonum 1.5, so this cannot be the exact 2.
    assert_eq!(eval("(denominator (/ 6 4))"), "2.0");
    // And a non-rational is an error rather than a wrong answer.
    assert!(err_of("(denominator (/ 1.0 0.0))").contains("not rational"));
    assert!(err_of("(numerator (/ 0.0 0.0))").contains("not rational"));
}

#[test]
fn a_string_escape_may_continue_a_line() {
    // R7RS §6.7 is `\ <intraline whitespace>* <line ending> <intraline
    // whitespace>*`, and only the middle of that worked: a backslash
    // immediately before a newline was fine, but trailing spaces after the
    // backslash -- which no editor shows and every editor leaves behind --
    // made it an unknown escape.
    assert_eq!(
        eval("(read (open-input-string \"\\\"line 1\\\\\\n   continued\\\"\"))"),
        "\"line 1continued\""
    );
    assert_eq!(
        eval("(read (open-input-string \"\\\"line 1\\\\  \\n \\t continued\\\"\"))"),
        "\"line 1continued\""
    );
    // `\|` is required too, so a symbol's delimiter can be written in a
    // string the same way.
    assert_eq!(
        eval("(read (open-input-string \"\\\"a\\\\|b\\\"\"))"),
        "\"a|b\""
    );
}

#[test]
fn datum_labels_read_as_well_as_write() {
    // The writer already EMITTED `#0=`/`#0#` -- it had to, or `write` would
    // spin on a cycle -- so the printer produced output its own reader could
    // not read. A round-trip hole in a feature of mine, and only an outside
    // suite would find it, because I tested the direction I wrote first.
    assert_eq!(
        eval(r##"(read (open-input-string "(#0=(1 2 3) #0#)"))"##),
        "((1 2 3) (1 2 3))"
    );
    // Shared, not copied: that is the whole point of a label.
    assert_eq!(
        eval(r##"(let ([x (read (open-input-string "(#0=(1 2 3) #0#)"))]) (eq? (car x) (cadr x)))"##),
        "#t"
    );
    // An actual cycle.
    assert_eq!(eval(r##"(cadr (read (open-input-string "#0=(1 . #0#)")))"##), "1");
    assert_eq!(eval(r##"(list? (read (open-input-string "#0=(1 . #0#)")))"##), "#f");
    // And it round-trips: this is the property that was broken.
    assert_eq!(
        eval(r##"(read (open-input-string "#0=(1 . #0#)"))"##),
        "#0=(1 . #0#)"
    );
    // Inside a vector, and with a label other than 0.
    assert_eq!(
        eval(r##"(read (open-input-string "#7=#(1 #7#)"))"##),
        "#0=#(1 #0#)"
    );
    // A label is scoped to its datum, so two forms may both use `#0=`.
    assert_eq!(
        eval(
            r##"(let ([p (open-input-string "#0=(1) #0=(2)")])
                 (list (read p) (read p)))"##
        ),
        "((1) (2))"
    );
    // Malformed labels refuse rather than guess.
    assert!(err_of("(read (open-input-string \"#0#\"))").contains("has no"));
    assert!(err_of("(read (open-input-string \"#0=#0#\"))").contains("defines nothing"));
}

#[test]
fn the_fold_case_directives_work_and_stop_at_a_bar_symbol() {
    assert_eq!(eval(r##"(read (open-input-string "#!fold-case ABC"))"##), "abc");
    assert_eq!(
        eval(r##"(read (open-input-string "#!fold-case #!no-fold-case ABC"))"##),
        // scm is case-sensitive, so an upper-case symbol needs no bars: it
        // reads back as itself.
        "ABC"
    );
    assert_eq!(eval(r##"(read (open-input-string "ABC"))"##), "ABC");
    // R7RS: a `|…|` symbol is NOT folded, which is the whole reason the
    // syntax exists.
    assert_eq!(
        eval(r##"(read (open-input-string "#!fold-case |MiXeD|"))"##),
        "MiXeD"
    );
    // A prefix is not the directive: the delimiter check stops
    // `#!fold-caseX` from swallowing the `X`.
    assert!(err_of("(read (open-input-string \"#!fold-caseX\"))").contains("unsupported"));
}

// ---- batch 2: records, multiple values, and binary ports ---------------

#[test]
fn a_dotted_pattern_matches_a_proper_list_too() {
    // `(p1 … pn . prest)` matches n OR MORE elements, with `prest` taking the
    // n-th cdr -- which for a proper list of exactly n elements is `()`. The
    // matcher demanded an exact length, so a dotted pattern matched an
    // *improper* input and refused a proper one. That made every dotted
    // sub-pattern useless, and `define-record-type`'s
    // `(field accessor . modifier)` is exactly that shape.
    let m = "(define-syntax d (syntax-rules () [(_ (a . c)) '(a c)]))";
    assert_eq!(eval(&format!("{m} (d (1 2 3))")), "(1 (2 3))");
    assert_eq!(eval(&format!("{m} (d (1))")), "(1 ())");
    assert_eq!(eval(&format!("{m} (d (1 . 2))")), "(1 2)");
    // At the top level of a pattern, and nested under an ellipsis.
    let t = "(define-syntax t (syntax-rules () [(_ a . c) '(a c)]))";
    assert_eq!(eval(&format!("{t} (t 1 2 3)")), "(1 (2 3))");
    let e2 = "(define-syntax e2 (syntax-rules () [(_ (a b . c) ...) '((a b c) ...)]))";
    assert_eq!(
        eval(&format!("{e2} (e2 (1 2 3) (4 5))")),
        "((1 2 (3)) (4 5 ()))"
    );
    // A bracket vector as input has no cdr chain to point into, so the tail
    // has to be built -- which is why the matcher allocates.
    let b = "(define-syntax b (syntax-rules () [(_ (a . c)) '(a c)]))";
    assert_eq!(eval(&format!("{b} (b [1 2 3])")), "(1 (2 3))");
}

#[test]
fn define_values_handles_all_three_shapes() {
    assert_eq!(eval("(let () (define-values () (values)) 'ok)"), "ok");
    assert_eq!(eval("(let () (define-values (x) (values 1)) x)"), "1");
    assert_eq!(eval("(let () (define-values (x y) (values 1 2)) (+ x y))"), "3");
    // A bare symbol takes the LIST of all the values.
    assert_eq!(eval("(let () (define-values x (values 1 2)) (apply + x))"), "3");
    // And a dotted formal takes the leftovers.
    assert_eq!(
        eval("(let () (define-values (x y . z) (values 1 2 3 4)) (+ x y (car z) (cadr z)))"),
        "10"
    );
    // At top level too, not only inside a body.
    assert_eq!(eval("(define-values (p q) (values 7 8)) (list p q)"), "(7 8)");
}

#[test]
fn records_are_their_own_types() {
    let pare = "(define-record-type <pare> (kons x y) pare? (x kar set-kar!) (y kdr))";
    assert_eq!(eval(&format!("{pare} (pare? (kons 1 2))")), "#t");
    assert_eq!(eval(&format!("{pare} (pare? (cons 1 2))")), "#f");
    assert_eq!(eval(&format!("{pare} (list (kar (kons 1 2)) (kdr (kons 1 2)))")), "(1 2)");
    assert_eq!(
        eval(&format!("{pare} (let ([k (kons 1 2)]) (set-kar! k 3) (kar k))")),
        "3"
    );
    // A field with no modifier gets no setter, which is the point of
    // declaring it that way.
    assert!(err_of(&format!("{pare} (set-kdr! (kons 1 2) 9)")).contains("unbound"));

    // Two types with the same field names stay distinct, because the type is
    // compared by identity and not by name.
    let two = format!(
        "{pare} (define-record-type <other> (mk x y) other? (x ox) (y oy))"
    );
    assert_eq!(eval(&format!("{two} (list (other? (kons 1 2)) (pare? (mk 1 2)))")), "(#f #f)");
    assert_eq!(
        eval(&format!("{two} (guard (e (else 'refused)) (kar (mk 1 2)))")),
        "refused"
    );

    // A constructor may name a SUBSET of the fields, in its own order; the
    // rest are unspecified. That is why the constructor builds an alist
    // rather than trusting argument position.
    let p3 = "(define-record-type <p3> (mk3 c a) p3? (a fa) (b fb) (c fc))";
    assert_eq!(eval(&format!("{p3} (list (fa (mk3 30 10)) (fb (mk3 30 10)) (fc (mk3 30 10)))")), "(10 #f 30)");
}

#[test]
fn list_set_mutates_in_place() {
    assert_eq!(eval("(let ([l (list 1 2 3)]) (list-set! l 1 'two) l)"), "(1 two 3)");
}

#[test]
fn binary_ports_carry_bytes() {
    assert_eq!(
        eval(
            "(let ([in (open-input-bytevector (bytevector 1 2 3 4 5))])
               (list (read-u8 in) (peek-u8 in) (read-u8 in)
                     (read-bytevector 2 in) (read-u8 in)
                     (eof-object? (read-u8 in))))"
        ),
        "(1 2 2 #u8(3 4) 5 #t)"
    );
    assert_eq!(
        eval(
            "(let ([out (open-output-bytevector)])
               (write-u8 9 out)
               (write-bytevector (bytevector 7 8) out)
               (write-bytevector (bytevector 1 2 3) out 1 2)
               (get-output-bytevector out))"
        ),
        "#u8(9 7 8 2)"
    );
    // `read-bytevector!` fills a range and answers how many landed.
    assert_eq!(
        eval(
            "(let ([tgt (make-bytevector 5 0)]
                   [in (open-input-bytevector (bytevector 10 20 30))])
               (let ([n (read-bytevector! tgt in 1 4)])
                 (list n tgt (eof-object? (read-bytevector! tgt in)))))"
        ),
        "(3 #u8(0 10 20 30 0) #t)"
    );
    // A short read near the end returns what there was; eof means NOTHING
    // was available, which is the distinction a caller loops on.
    assert_eq!(
        eval(
            "(let ([in (open-input-bytevector (bytevector 1 2))])
               (list (read-bytevector 10 in) (eof-object? (read-bytevector 10 in))))"
        ),
        "(#u8(1 2) #t)"
    );
}

#[test]
fn a_binary_port_is_not_a_textual_one() {
    // R7RS keeps them apart on purpose: `read-char` on a binary port is an
    // error, not a reinterpretation of the bytes.
    assert_eq!(
        eval(
            "(let ([b (open-input-bytevector (bytevector 65))]
                   [t (open-input-string \"A\")])
               (list (binary-port? b) (textual-port? b) (input-port? b) (u8-ready? b)
                     (binary-port? t) (textual-port? t)))"
        ),
        "(#t #f #t #t #f #t)"
    );
    assert!(
        err_of("(read-char (open-input-bytevector (bytevector 65)))").contains("port"),
    );
    assert!(
        err_of("(read-u8 (open-input-string \"A\"))").contains("binary input port"),
    );
    // And an output port is an output port whichever bytes it carries.
    assert_eq!(
        eval("(let ([o (open-output-bytevector)]) (list (output-port? o) (input-port? o)))"),
        "(#t #f)"
    );
}

#[test]
fn flush_output_port_exists_even_though_it_does_nothing() {
    // Nothing in memory needs flushing and stdout is already buffered by
    // Rust, so this is a genuine no-op -- but portable code calls it, so it
    // has to be there.
    assert_eq!(eval("(flush-output-port)"), "#<unspecified>");
    assert_eq!(
        eval("(let ([o (open-output-string)]) (flush-output-port o) 'ok)"),
        "ok"
    );
}

// ---- batch 4: continuations, dynamic-wind, and inexact integers --------

#[test]
fn an_integer_may_be_inexact() {
    // R7RS's integer division is about INTEGERS, and `32.0` is one. Refusing
    // a flonum outright was the wrong reading: the exactness of the
    // arguments decides the exactness of the RESULT, not whether the call is
    // legal.
    assert_eq!(eval("(remainder -13 -4.0)"), "-1.0");
    assert_eq!(eval("(quotient 7.0 2)"), "3.0");
    assert_eq!(eval("(modulo -13 4.0)"), "3.0");
    assert_eq!(eval("(list (gcd 32.0 -36) (lcm 32.0 -36))"), "(4.0 288.0)");
    // The zero shortcut has to carry exactness too.
    assert_eq!(eval("(lcm 32.0 0)"), "0.0");
    // And all-exact stays exact, which is the half that was already right.
    assert_eq!(eval("(list (quotient 7 2) (gcd 32 -36) (lcm 32 -36))"), "(3 4 288)");
    // A non-integral flonum is still an error.
    assert!(err_of("(quotient 7.5 2)").contains("integers"));
    // As is one too large for an i64 -- `as i64` would saturate, which is a
    // wrong answer rather than a refusal.
    assert!(err_of("(quotient 1e30 2)").contains("integers"));
}

#[test]
fn a_continuation_is_a_procedure() {
    // Missed when escape continuations landed: the new heap object had to be
    // added to five matches, and `procedure?` was the one with no compiler
    // error to catch it.
    assert_eq!(eval("(call-with-current-continuation procedure?)"), "#t");
    assert_eq!(eval("(procedure? (make-parameter 1))"), "#t");
    assert_eq!(eval("(list (procedure? car) (procedure? (lambda () 1)) (procedure? 4))"), "(#t #t #f)");
}

#[test]
fn member_and_assoc_take_a_comparator() {
    assert_eq!(eval(r##"(member "B" '("a" "b" "c") string-ci=?)"##), "(\"b\" \"c\")");
    assert_eq!(eval("(assoc 2.0 '((1 1) (2 4) (3 9)) =)"), "(2 4)");
    // The default is still `equal?`.
    assert_eq!(eval("(member 2 '(1 2 3))"), "(2 3)");
    assert_eq!(eval("(assoc 'b '((a . 1) (b . 2)))"), "(b . 2)");
    assert_eq!(eval("(member 9 '(1 2))"), "#f");
}

#[test]
fn dynamic_wind_runs_after_on_every_way_out() {
    // Three exits, one mechanism. That is the point: a normal return is a
    // chain of one `after`, and an unwind is a chain of however many winds
    // it crossed. Three separate mechanisms is how this usually goes wrong.
    let log = "(define log '())
               (define (note s) (set! log (cons s log)))
               (define (seen) (reverse log))";

    // Normal: the BODY's value leaves, not `after`'s.
    assert_eq!(
        eval(&format!(
            "{log}
             (let ([r (dynamic-wind (lambda () (note 'before))
                                    (lambda () (note 'body) 'result)
                                    (lambda () (note 'after)))])
               (list r (seen)))"
        )),
        "(result (before body after))"
    );

    // Escaping out through it.
    assert_eq!(
        eval(&format!(
            "{log}
             (let ([r (call-with-current-continuation
                        (lambda (k)
                          (dynamic-wind (lambda () (note 'before))
                                        (lambda () (k 'escaped))
                                        (lambda () (note 'after)))))])
               (list r (seen)))"
        )),
        "(escaped (before after))"
    );

    // An error through it: `after` runs and the condition still arrives.
    assert_eq!(
        eval(&format!(
            "{log}
             (let ([r (guard (e (else 'caught))
                        (dynamic-wind (lambda () (note 'before))
                                      (lambda () (car '()))
                                      (lambda () (note 'after))))])
               (list r (seen)))"
        )),
        "(caught (before after))"
    );

    // Nested, escaping from the inside: both run, innermost first.
    assert_eq!(
        eval(&format!(
            "{log}
             (call-with-current-continuation
               (lambda (k)
                 (dynamic-wind (lambda () (note 'out-before))
                   (lambda () (dynamic-wind (lambda () (note 'in-before))
                                            (lambda () (k 'gone))
                                            (lambda () (note 'in-after))))
                   (lambda () (note 'out-after)))))
             (seen)"
        )),
        "(out-before in-before in-after out-after)"
    );
}

#[test]
fn an_inner_guard_beats_an_outer_handler() {
    // SRFI-34 example 5, and the bug it found: `raise` dispatches through
    // the handler stack at the raise point, so a guard that only caught the
    // UNWIND lost to any `with-exception-handler` installed outside it --
    // the outer handler ran and the guard never saw the condition. R7RS
    // orders handlers innermost-first, so `guard` installs one too.
    let t4 = "(define (t4 v out)
                (call-with-current-continuation
                  (lambda (k)
                    (with-exception-handler
                      (lambda (x) (display \"reraised \" out) (write x out)
                                  (display #\\! out) (k 'zero))
                      (lambda ()
                        (guard (c ((positive? c) 'positive) ((negative? c) 'negative))
                          (raise v)))))))
              (define (run v)
                (let* ([o (open-output-string)] [val (t4 v o)])
                  (list val (get-output-string o))))";
    // The guard matches, so the outer handler never runs and nothing is
    // written.
    assert_eq!(eval(&format!("{t4} (run 1)")), "(positive \"\")");
    assert_eq!(eval(&format!("{t4} (run -1)")), "(negative \"\")");
    // No clause matches, so it re-raises OUTWARD -- not back into itself.
    assert_eq!(eval(&format!("{t4} (run 0)")), "(zero \"reraised 0!\")");
}

#[test]
fn guard_still_catches_what_never_went_through_raise() {
    // A machine error does not pass through the handler stack at all, so
    // `%try` is still needed -- which is why `guard` does both.
    assert_eq!(eval("(guard (e (else 'caught)) (car '()))"), "caught");
    assert_eq!(eval("(guard (e (else 'caught)) (vector-ref (vector) 0))"), "caught");
    assert_eq!(eval("(guard (e (else 'caught)) (raise 'x))"), "caught");
    // A non-matching inner guard falls through to the outer one.
    assert_eq!(
        eval("(guard (e (else 'outer)) (guard (e2 ((number? e2) 'num)) (raise 'not-a-number)))"),
        "outer"
    );
    // And the host's own stop is still not catchable (requirement 8).
    let mut vm = Vm::new();
    vm.set_fuel(Some(20_000));
    assert!(
        vm.eval_str("(guard (e (else 'caught)) (let loop () (loop)))")
            .is_err(),
        "out-of-fuel must not be catchable, even now that guard installs a handler"
    );
}
