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
    assert_eq!(eval(r##"(string->vector "abcd" 1 3)"##), "[#\\b #\\c]");
    assert_eq!(eval(r##"(string->vector "ab")"##), "[#\\a #\\b]");
    assert_eq!(eval(r"(vector->string (vector #\a #\b #\c) 1)"), "\"bc\"");
    assert_eq!(eval(r"(vector->string (vector #\a #\b #\c) 0 2)"), "\"ab\"");
}

#[test]
fn vector_map_and_for_each_take_several_vectors() {
    // R7RS, and they stop at the shortest -- like `map`.
    assert_eq!(eval("(vector-map (lambda (x) (* x 10)) (vector 1 2 3))"), "[10 20 30]");
    assert_eq!(eval("(vector-map + (vector 1 2) (vector 10 20))"), "[11 22]");
    assert_eq!(
        eval("(vector-map list (vector 1 2 9) (vector 3 4))"),
        "[(1 3) (2 4)]"
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
