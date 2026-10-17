//! Every claim in `docs/subset.md`, checked.
//!
//! That document is the normative statement of what the language is, and a
//! normative document that nothing verifies is just an old opinion. The
//! README has the same arrangement for its code; this is the same idea
//! applied to the sentences that define the language.
//!
//! When one of these fails, the question is which is wrong -- the code or
//! the document -- and the answer is sometimes the document.

use scm::{Interp, Val};

fn eval(src: &str) -> String {
    let mut vm = scm::Vm::new();
    match vm.eval_str(src) {
        Ok(v) => scm::write_str(&vm.engine, v),
        Err(e) => format!("error: {e}"),
    }
}

fn raises(src: &str) -> bool {
    scm::Vm::new().eval_str(src).is_err()
}

#[test]
fn the_standard_libraries_all_import() {
    // "All of these import and are a no-op, because the standard bindings
    // are the prelude."
    for lib in [
        "(scheme base)", "(scheme write)", "(scheme read)", "(scheme char)",
        "(scheme cxr)", "(scheme inexact)", "(scheme lazy)",
        "(scheme case-lambda)", "(scheme eval)", "(scheme file)",
        "(scheme time)", "(scheme process-context)", "(scheme repl)",
        "(scheme load)", "(scheme r5rs)", "(scheme complex)",
    ] {
        assert_eq!(eval(&format!("(begin (import {lib}) 'ok)")), "ok", "{lib}");
    }
}

#[test]
fn an_import_is_not_a_promise() {
    // "(import (scheme complex)) is accepted and make-rectangular is then
    // an unbound variable. The import is not a promise."
    assert_eq!(eval("(begin (import (scheme complex)) 'ok)"), "ok");
    assert!(eval("(make-rectangular 1 2)").contains("unbound variable"));
}

#[test]
fn the_claimed_features_are_claimed_and_true() {
    let f = eval("(features)");
    for claimed in ["r7rs", "ieee-float", "exact-closed", "scm"] {
        assert!(f.contains(claimed), "{claimed} not claimed: {f}");
    }
    // `exact-closed` means what it says.
    assert_eq!(eval("(* 9223372036854775807 2)"), "18446744073709551614");
    assert_eq!(eval("(exact? (* 9223372036854775807 2))"), "#t");
    // `ieee-float`: doubles, with the usual specials.
    assert_eq!(eval("(= (/ 1.0 0.0) (/ 1.0 0.0))"), "#t");
    assert_eq!(eval("(= (/ 0.0 0.0) (/ 0.0 0.0))"), "#f");
}

#[test]
fn the_unclaimed_features_are_unclaimed_and_absent() {
    let f = eval("(features)");
    for absent in ["ratios", "exact-complex", "full-unicode"] {
        assert!(!f.contains(absent), "{absent} claimed: {f}");
    }
    // `ratios`: a rational literal is a read error, and / is inexact.
    assert!(raises("'1/2"));
    assert_eq!(eval("(/ 8 3)"), "2.6666666666666665");
    assert_eq!(eval("(/ 8 4)"), "2");
    // `exact-complex`.
    assert!(eval("(make-rectangular 1 2)").contains("unbound"));
}

#[test]
fn the_documented_surface_is_present() {
    // The list the document says the suite exercises.
    assert_eq!(eval("(begin (define-record-type p (mk a) p? (a pa)) (pa (mk 7)))"), "7");
    assert_eq!(
        eval("(begin (define-syntax m (syntax-rules () ((_ (a ...) ...) '((a ...) ...)))) (m (1 2) (3)))"),
        "((1 2) (3))"
    );
    assert_eq!(eval("(guard (e (#t (list 'caught e))) (raise 'boom))"), "(caught boom)");
    assert_eq!(eval("(with-exception-handler (lambda (e) 99) (lambda () (+ 1 (raise-continuable 'c))))"), "100");
    assert_eq!(eval("(let ((o '())) (dynamic-wind (lambda () (set! o (cons 'in o))) (lambda () 'body) (lambda () (set! o (cons 'out o)))) o)"), "(out in)");
    assert_eq!(eval("(begin (define p (make-parameter 1)) (parameterize ((p 2)) (p)))"), "2");
    assert_eq!(eval("(force (delay (+ 1 2)))"), "3");
    assert_eq!(eval("(force (make-promise 7))"), "7");
    assert_eq!(eval("((case-lambda ((a) 'one) ((a b) 'two)) 1 2)"), "two");
    assert_eq!(eval("(let-values (((a b) (values 1 2))) (+ a b))"), "3");
    assert_eq!(eval("(with-output-to-string (lambda () (display 'hi)))"), "\"hi\"");
    assert_eq!(eval("(utf8->string (string->utf8 \"ok\"))"), "\"ok\"");
    assert_eq!(eval("(eval '(+ 1 2) (interaction-environment))"), "3");
}

#[test]
fn deviation_one_continuations_escape_but_do_not_re_enter() {
    // Escaping works, in both directions the document names.
    assert_eq!(eval("(call/cc (lambda (k) (+ 1 (k 41))))"), "41");
    assert_eq!(
        eval("(call/cc (lambda (k) (let loop ((i 0)) (if (= i 100) (k 'done) (begin (loop (+ i 1)) 'no)))))"),
        "done"
    );
    // Re-entering a dead extent raises rather than misbehaving.
    assert!(raises("(begin (define k #f) (+ 1 (call/cc (lambda (c) (set! k c) 1))) (k 2))"));
}

#[test]
fn deviation_three_colon_names_are_keywords_but_the_reader_can_be_told_otherwise() {
    assert_eq!(eval("(keyword? ':at)"), "#t");
    assert_eq!(eval("(symbol? ':at)"), "#f");
    // And it is a reader mode, not a property of the evaluator: with
    // `colon_keywords` off, `:at` is the symbol R7RS says it is.
    let mut vm = scm::Vm::new();
    let opts = scm::reader::ReaderOptions { colon_keywords: false };
    let mut r = scm::reader::Reader::with_options(":at", opts);
    let v = r.read(&mut vm.engine).unwrap().unwrap();
    assert!(matches!(v, scm::value::Value::Symbol(_)), "{v:?}");
}

#[test]
fn deviation_four_an_import_copies_its_bindings() {
    // A library that assigns its own exported variable after the import
    // does not reach the importer.
    let src = "(define-library (d t)
                 (export v bump!)
                 (import (scheme base))
                 (begin (define v 1) (define (bump!) (set! v 99) v)))
               (import (d t))
               (let ((before v) (returned (bump!))) (list before returned v))";
    assert_eq!(eval(src), "(1 99 1)");
}

#[test]
fn the_documented_lambda_list_holds() {
    // A default runs only when the argument is absent, so an explicit #f
    // survives -- the sentence the whole design exists for.
    assert_eq!(eval("(define (f &opt (smooth #t)) smooth) (f #f)"), "#f");
    assert_eq!(eval("(define (f &opt (smooth #t)) smooth) (f)"), "#t");
    // An absent argument with no default is #f.
    assert_eq!(eval("(define (f &opt m) m) (f)"), "#f");
    // The markers are ordered, and out of order is an error.
    assert!(eval("(define (f &named a &opt b) 1)").contains("out of order"));
    // Three spellings, one procedure.
    let call = "(list (g #(1 1 1)) (g #(1 1 1) :at #(0 1 0) :material :brass))";
    let want = "((#(1 1 1) #(0 0 0) #f) (#(1 1 1) #(0 1 0) :brass))";
    for formals in [
        "(g size #:at (at #(0 0 0)) #:material material)",
        "(g size :at (at #(0 0 0)) :material material)",
        "(g size &named (at #(0 0 0)) material)",
    ] {
        assert_eq!(
            eval(&format!("(define {formals} (list size at material)) {call}")),
            want,
            "formals {formals}"
        );
    }
    // A keyword parameter does not open a section: a required one may follow.
    assert_eq!(eval("(define (f #:at at b) (list at b)) (f 5 :at 1)"), "(1 5)");
    // One canonical output, whichever spelling was written.
    assert_eq!(
        eval("(define (f a #:at at) at) (signature 'f)"),
        "\"(f a :at at)\""
    );
}

#[test]
fn the_documented_additions_hold() {
    // 2. Keywords are a type of their own, and `#:at` is the same datum.
    assert_eq!(eval("(list (keyword? :at) (symbol? :at) (eq? :at #:at))"), "(#t #f #t)");

    // 3. `[a b c]` EVALUATES its elements; `#(a b c)` is a constant.
    assert_eq!(eval("(let ((x 2)) [x x])"), "#(2 2)");
    assert_eq!(eval("(let ((x 2)) '#(x x))"), "#(x x)");
    // ... and reads as a list where a form expects one.
    assert_eq!(eval("(let ([x 1] [y 2]) (+ x y))"), "3");
    assert_eq!(eval("(cond [#f 1] [else 2])"), "2");
    assert_eq!(eval("(case 2 [(1) 'one] [(2) 'two] [else 'other])"), "two");
    // ... and a vector always prints `#(…)`, whichever spelling built it.
    assert_eq!(eval("[1 2]"), "#(1 2)");
    assert_eq!(eval("(equal? [1 2] #(1 2))"), "#t");

    // 4. Vectors grow, in place, returning the vector.
    assert_eq!(eval("(define v (vector 1)) (vector-push! v 2 3) v"), "#(1 2 3)");
    assert_eq!(eval("(define v (vector)) (eq? v (vector-push! v 1))"), "#t");
    // One vector type: the grown one is still a plain vector.
    assert_eq!(eval("(define v (vector 1)) (vector-push! v 2) (vector? v)"), "#t");

    // 5. `format`, in the spellings the document names.
    assert_eq!(eval(r#"(format "~a ~s" "x" "x")"#), r#""x \"x\"""#);
    assert_eq!(eval(r#"(format "~d~~~n" 5)"#), "\"5~\\n\"");
    assert_eq!(eval(r#"(format "~,2f" 3.14159)"#), r#""3.14""#);
    // Anything else is an error that names the directive, and a left-over
    // argument is an error -- both sentences in the document.
    assert!(eval(r#"(format "~b" 5)"#).contains("unknown directive ~b"));
    assert!(eval(r#"(format "~a" 1 2)"#).contains("left over"));
}

#[test]
fn the_documented_limits_hold() {
    // Exact integers are bounded, and the bound reports.
    assert!(eval("(expt 2 1099511627776)").contains("too large"));
    // Bitwise refuses a bignum rather than truncating.
    assert!(eval("(begin (import (srfi 151)) (bitwise-and (expt 2 63) 1))").contains("64 bits"));
    // Char-sets are Latin-1.
    let cs = "(begin (import (srfi 14)) ";
    assert_eq!(eval(&format!("{cs}(char-set-size char-set:full))")), "256");
    assert_eq!(eval(&format!("{cs}(char-set-contains? char-set:full (integer->char 955)))")), "#f");
    assert!(eval(&format!("{cs}(char-set (integer->char 955)))")).contains("Latin-1"));
    // A predicate is the Unicode-correct way in, as the document says.
    assert_eq!(
        eval("(begin (import (srfi 13)) (string-index \"a\\x3bb;b\" char-alphabetic? 1 2))"),
        "1"
    );
    // equal-hash terminates on a cycle.
    assert_eq!(eval("(begin (define l (list 1 2)) (set-cdr! (cdr l) l) (exact? (equal-hash l)))"), "#t");
}

#[test]
fn the_host_can_bound_a_script() {
    use scm::embed::Error;
    let mut i = Interp::new().unwrap();
    i.set_fuel(Some(10_000));
    assert!(matches!(i.eval("(let spin () (spin))"), Err(Error::OutOfFuel)));

    // The cap is reported only after a collection has been tried, which is
    // the sentence in the document worth checking: garbage must not count.
    let mut i = Interp::new().unwrap();
    i.set_heap_cap(Some(60_000));
    let churn = "(define (b n) (let loop ((i n) (acc '())) (if (= i 0) acc (loop (- i 1) (cons i acc)))))
                 (let loop ((i 0) (t 0)) (if (= i 30) t (loop (+ i 1) (+ t (car (b 5000))))))";
    assert_eq!(i.eval(churn).map(|r| i.value(&r).unwrap()).unwrap(), Val::Int(30));

    let mut i = Interp::new().unwrap();
    i.set_heap_cap(Some(20_000));
    assert!(matches!(
        i.eval("(let loop ((i 0) (acc '())) (if (= i 200000) acc (loop (+ i 1) (cons i acc))))"),
        Err(Error::OutOfMemory)
    ));
}

#[test]
fn val_conversion_truncates_rather_than_refusing() {
    // The document first said "deeper structure arrives as Val::Opaque",
    // which this test caught as wrong on its first run: the value is
    // **truncated**, not refused. The levels within the limit convert and
    // only what is beyond them is Opaque, so a host sees a prefix of the
    // structure rather than nothing.
    let mut i = Interp::new().unwrap();
    let deep = i
        .eval("(let loop ((i 0) (acc '())) (if (= i 400) acc (loop (+ i 1) (list acc))))")
        .unwrap();

    let mut v = i.value(&deep).unwrap();
    let mut depth = 0;
    loop {
        match v {
            Val::List(items) if items.len() == 1 => {
                depth += 1;
                v = items.into_iter().next().unwrap();
            }
            other => {
                v = other;
                break;
            }
        }
    }
    assert_eq!(v, Val::Opaque, "the bottom of a too-deep value is Opaque");
    assert!(
        (100..=130).contains(&depth),
        "converted {depth} levels, expected the documented limit of about 128"
    );

    // And the whole thing is still reachable through Scheme, which is what
    // the document tells the reader to do.
    assert!(i.write(&deep).unwrap().starts_with("(((("));
}
