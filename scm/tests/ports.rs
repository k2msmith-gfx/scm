//! Textual ports.
//!
//! Most of what the R7RS suite reaches through ports is the reader and the
//! printer that have existed since S1 — `read` is that reader pointed at a
//! port, and `(write x out)` plus `get-output-string` is how the suite gets
//! at the writer. So this is mostly an *exposure* of work already done, which
//! is why a 231-assertion block cost so little.

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
fn reading_datums_from_a_string_port() {
    assert_eq!(
        eval(r#"(let ([p (open-input-string "(1 2) foo 3")])
                  (list (read p) (read p) (read p) (eof-object? (read p))))"#),
        "((1 2) foo 3 #t)"
    );
    // The port's cursor advances by exactly one datum, so what is left is
    // still readable as the next one.
    assert_eq!(
        eval(r#"(let ([p (open-input-string "1 2")]) (read p) (read-char p) (read p))"#),
        "2"
    );
}

#[test]
fn reading_characters_and_lines() {
    assert_eq!(
        eval(r#"(let ([p (open-input-string "ab")])
                  (list (read-char p) (peek-char p) (read-char p) (eof-object? (read-char p))))"#),
        "(#\\a #\\b #\\b #t)"
    );
    assert_eq!(
        eval("(let ([p (open-input-string \"ab\\ncd\")])
                (list (read-line p) (read-line p) (eof-object? (read-line p))))"),
        "(\"ab\" \"cd\" #t)"
    );
    assert_eq!(
        eval(r#"(read-string-all (open-input-string "abc"))"#),
        "\"abc\""
    );
}

#[test]
fn writing_to_a_string_port() {
    assert_eq!(
        eval(r#"(let ([o (open-output-string)])
                  (write "hi" o) (display " there" o) (get-output-string o))"#),
        "\"\\\"hi\\\" there\""
    );
    assert_eq!(
        eval(r#"(let ([o (open-output-string)]) (write-char #\x o) (write-string "yz" o)
                  (get-output-string o))"#),
        "\"xyz\""
    );
    assert_eq!(
        eval("(let ([o (open-output-string)]) (newline o) (get-output-string o))"),
        "\"\\n\""
    );
}

#[test]
fn write_and_display_differ_through_a_port_too() {
    // The same distinction the printer already made: `write` is
    // round-trippable, `display` is for people.
    assert_eq!(
        eval(r#"(let ([o (open-output-string)]) (write #\a o) (display #\a o)
                  (get-output-string o))"#),
        "\"#\\\\aa\""
    );
}

#[test]
fn output_can_be_redirected_for_a_dynamic_extent() {
    // The point of making `current-output-port` a parameter rather than a
    // global: this is what ray writes as `(with-dyns [:out buf] (snapshot))`.
    assert_eq!(
        eval(r#"(with-output-to-string (lambda () (display "captured") (newline)))"#),
        "\"captured\\n\""
    );
    // And it nests and restores.
    assert_eq!(
        eval(r#"(with-output-to-string
                  (lambda ()
                    (display "a")
                    (display (with-output-to-string (lambda () (display "inner"))))
                    (display "b")))"#),
        "\"ainnerb\""
    );
}

#[test]
fn a_truncated_datum_is_a_read_error_and_says_so() {
    // R7RS distinguishes a read error from other conditions, and the suite
    // tests it — so the error object records its kind rather than being
    // guessed at afterwards.
    assert_eq!(
        eval(r#"(guard (e (#t (read-error? e))) (read (open-input-string ")")))"#),
        "#t"
    );
    assert_eq!(
        eval(r#"(guard (e (#t (read-error? e))) (read (open-input-string "\"")))"#),
        "#t"
    );
    // An ordinary refusal is not a read error.
    assert_eq!(eval("(guard (e (#t (read-error? e))) (car 5))"), "#f");
    assert_eq!(eval("(guard (e (#t (file-error? e))) (car 5))"), "#f");
}

#[test]
fn the_port_predicates() {
    assert_eq!(eval(r#"(input-port? (open-input-string "a"))"#), "#t");
    assert_eq!(eval("(output-port? (open-output-string))"), "#t");
    assert_eq!(eval(r#"(output-port? (open-input-string "a"))"#), "#f");
    assert_eq!(eval(r#"(textual-port? (open-input-string "a"))"#), "#t");
    assert_eq!(eval(r#"(binary-port? (open-input-string "a"))"#), "#f");
    assert_eq!(eval(r#"(port? (open-input-string "a"))"#), "#t");
    assert_eq!(eval("(port? 1)"), "#f");
    assert_eq!(eval(r#"(char-ready? (open-input-string "a"))"#), "#t");
    assert_eq!(eval("(eof-object? (eof-object))"), "#t");
}

#[test]
fn closing_a_port() {
    assert_eq!(
        eval(r#"(let ([p (open-input-string "a")])
                  (list (input-port-open? p) (begin (close-port p) (input-port-open? p))))"#),
        "(#t #f)"
    );
    // Writing to a closed port refuses rather than silently dropping the text.
    assert_eq!(
        eval(r#"(let ([o (open-output-string)])
                  (close-port o)
                  (guard (e (#t 'refused)) (display "x" o)))"#),
        "refused"
    );
    assert_eq!(
        eval(r#"(call-with-port (open-output-string)
                  (lambda (o) (display "x" o) (get-output-string o)))"#),
        "\"x\""
    );
}

#[test]
fn the_round_trip_holds_through_ports_too() {
    // S1's gate, now expressible in Scheme — which is exactly how the suite
    // tests the reader and writer, and why this block was cheap.
    let src = r##"(define (round-trip str)
                   (let* ([v (read (open-input-string str))]
                          [o (open-output-string)])
                     (write v o)
                     (get-output-string o)))
                 (list (round-trip "(1 2.5 \"s\" #\\a)")
                       (round-trip "[1 2]")
                       (round-trip "#(1 2)")
                       (round-trip "1.0")
                       (round-trip ":at"))"##;
    assert_eq!(
        eval(src),
        r##"("(1 2.5 \"s\" #\\a)" "[1 2]" "#(1 2)" "1.0" ":at")"##
    );
}

#[test]
fn ports_survive_gc_stress() {
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            r#"(let loop ((i 0) (acc '()))
                 (if (= i 20)
                     (length acc)
                     (loop (+ i 1)
                           (cons (with-output-to-string
                                   (lambda () (write (list i (* i 2)))))
                                 acc))))"#,
        )
        .expect("ports under gc stress");
    assert_eq!(write_str(&vm.engine, v), "20");
}

// ── the port argument is optional ────────────────────────────────────────
//
// R7RS makes the port optional on every input primitive, defaulting to
// `current-input-port`. The R7RS benchmark harness reads its three inputs
// with a bare `(read)`, and every one of its 57 programs failed on it.

#[test]
fn input_primitives_default_to_current_input_port() {
    assert_eq!(
        eval(r#"(parameterize ([current-input-port (open-input-string "42 x\ny")])
                  (list (read) (read-char) (peek-char) (read-line) (read-line) (eof-object? (read))))"#),
        r#"(42 #\space #\x "x" "y" #t)"#
    );
    assert_eq!(
        eval(r#"(parameterize ([current-input-port (open-input-bytevector (bytevector 7 8))])
                  (list (peek-u8) (read-u8) (u8-ready?) (read-bytevector 5)))"#),
        "(7 7 #t #u8(8))"
    );
    // The runtime's default input port is empty: an embedded script does not
    // get its host's stdin. Connecting the two is `scm-cli`'s decision.
    assert_eq!(eval("(eof-object? (read))"), "#t");
}

#[test]
fn the_stdin_port_is_an_input_port_before_it_is_read() {
    // Lazy on purpose: installing it must not touch stdin, or `scm -e` blocks
    // under any harness that leaves stdin open. Reading it is covered by the
    // CLI, which is the only thing that creates one.
    assert_eq!(
        eval("(let ([p (%stdin-port)]) (list (input-port? p) (textual-port? p) (output-port? p)))"),
        "(#t #t #f)"
    );
}
