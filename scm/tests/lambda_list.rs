//! §4's lambda list: `&opt`, `&named`, `&keys`.
//!
//! The semantic weight is all in one place -- an absent argument must be
//! distinguishable from `#f`, because ray documents three options that default
//! to true and treat `#f` as meaningful (`:smooth`, `camera-match`'s `:apply`,
//! `:repro`). A body-side default cannot tell the two apart; a lambda-list
//! default can, because the expression only runs when the argument is missing.

use scm::{Value, Vm, write_str};

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

// ---- &opt ---------------------------------------------------------------

#[test]
fn optional_positionals_fill_in_order() {
    assert_eq!(eval("(define (f a &opt b c) (list a b c)) (f 1 2 3)"), "(1 2 3)");
    assert_eq!(eval("(define (f a &opt b c) (list a b c)) (f 1 2)"), "(1 2 #f)");
    assert_eq!(eval("(define (f a &opt b c) (list a b c)) (f 1)"), "(1 #f #f)");
}

#[test]
fn an_absent_optional_without_a_default_is_false() {
    // Which keeps Janet's falsy-nil ergonomics: `(if material …)` translates
    // directly, with no supplied-predicate needed.
    assert_eq!(eval("(define (f &opt m) (if m 'given 'absent)) (f)"), "absent");
    assert_eq!(eval("(define (f &opt m) (if m 'given 'absent)) (f 1)"), "given");
}

#[test]
fn optional_defaults_run_only_when_absent() {
    assert_eq!(eval("(define (f a &opt (b 10)) (+ a b)) (f 1)"), "11");
    assert_eq!(eval("(define (f a &opt (b 10)) (+ a b)) (f 1 2)"), "3");
    // The point of the whole design: an explicit #f survives.
    assert_eq!(
        eval("(define (f &opt (smooth #t)) smooth) (f #f)"),
        "#f"
    );
    assert_eq!(eval("(define (f &opt (smooth #t)) smooth) (f)"), "#t");
}

#[test]
fn a_default_can_see_earlier_parameters() {
    // Evaluated left to right in the new frame, as in Racket and CL.
    assert_eq!(eval("(define (f a &opt (b (* 2 a))) (list a b)) (f 5)"), "(5 10)");
    assert_eq!(
        eval("(define (f a &opt (b (* 2 a)) (c (+ b 1))) (list a b c)) (f 5)"),
        "(5 10 11)"
    );
    assert_eq!(
        eval("(define (f a &opt (b (* 2 a)) (c (+ b 1))) (list a b c)) (f 5 1)"),
        "(5 1 2)"
    );
}

#[test]
fn a_default_is_an_expression_and_runs_on_the_explicit_stack() {
    // So it can call anything, including the procedure being defined.
    assert_eq!(
        eval("(define (fact n &opt (acc 1)) (if (= n 0) acc (fact (- n 1) (* n acc)))) (fact 10)"),
        "3628800"
    );
}

// ---- &named -------------------------------------------------------------

#[test]
fn named_arguments_match_by_name() {
    assert_eq!(
        eval("(define (f &named a b) (list a b)) (f :a 1 :b 2)"),
        "(1 2)"
    );
    // Order at the call site does not matter.
    assert_eq!(
        eval("(define (f &named a b) (list a b)) (f :b 2 :a 1)"),
        "(1 2)"
    );
    assert_eq!(eval("(define (f &named a b) (list a b)) (f :b 2)"), "(#f 2)");
    assert_eq!(eval("(define (f &named a b) (list a b)) (f)"), "(#f #f)");
}

#[test]
fn named_defaults_preserve_an_explicit_false() {
    // ray's `:smooth` is documented "default true (false = flat facets)". A
    // port that bound absent to #f and defaulted in the body would turn flat
    // facets smooth, with no error anywhere.
    let f = "(define (mesh &named (smooth #t)) (if smooth 'smooth 'flat))";
    assert_eq!(eval(&format!("{f} (mesh)")), "smooth");
    assert_eq!(eval(&format!("{f} (mesh :smooth #f)")), "flat");
    assert_eq!(eval(&format!("{f} (mesh :smooth #t)")), "smooth");
}

#[test]
fn a_misspelled_named_argument_is_an_error() {
    // Janet accepts this silently and the sphere keeps its default radius.
    let e = eval_err("(define (sphere &named radius) radius) (sphere :radiuss 2)");
    assert!(e.contains("unknown named argument :radiuss"), "got {e}");
    assert!(e.contains("sphere"), "the error should name the procedure: {e}");
}

#[test]
fn a_named_argument_without_a_value_is_an_error() {
    let e = eval_err("(define (f &named a) a) (f :a)");
    assert!(e.contains("named argument :a has no value"), "got {e}");
}

// ---- the disambiguation rule -------------------------------------------

#[test]
fn a_keyword_is_a_value_unless_it_names_a_parameter() {
    // The rule from plan §4, and the case that makes it necessary: a verb
    // that takes a keyword as a positional *value*.
    assert_eq!(eval("(define (set-tone-map k) k) (set-tone-map :aces)"), ":aces");
    assert_eq!(
        eval("(define (f a &opt b) (list a b)) (f :aces :reinhard)"),
        "(:aces :reinhard)"
    );
}

#[test]
fn a_declared_keyword_starts_the_named_section() {
    // `[center radius &opt material &named name]` called as
    // `(f c r :name :ball)` -- `:name` is declared, so it is not the material.
    assert_eq!(
        eval("(define (f a &opt material &named name) (list a material name)) (f 1 :name :ball)"),
        "(1 #f :ball)"
    );
    // And an undeclared keyword in the same slot IS the material.
    assert_eq!(
        eval("(define (f a &opt material &named name) (list a material name)) (f 1 :metal)"),
        "(1 :metal #f)"
    );
}

// ---- &keys and rest ----------------------------------------------------

#[test]
fn keys_collects_every_keyword_argument() {
    assert_eq!(
        eval("(define (f &keys opts) opts) (f :a 1 :b 2)"),
        "((:a . 1) (:b . 2))"
    );
    assert_eq!(eval("(define (f &keys opts) opts) (f)"), "()");
    // With `&keys`, an unknown keyword is the point rather than an error.
    assert_eq!(
        eval("(define (f &named a &keys opts) (list a opts)) (f :a 1 :z 9)"),
        "(1 ((:a . 1) (:z . 9)))"
    );
}

#[test]
fn a_dotted_rest_takes_leftover_positionals() {
    assert_eq!(eval("(define (f a . rest) rest) (f 1 2 3)"), "(2 3)");
    assert_eq!(
        eval("(define (f a &opt b . rest) (list a b rest)) (f 1 2 3 4)"),
        "(1 2 (3 4))"
    );
    assert_eq!(
        eval("(define (f &named k . rest) (list k rest)) (f 1 2 :k 3)"),
        "(3 (1 2))"
    );
}

// ---- malformed lambda lists -------------------------------------------

#[test]
fn markers_out_of_order_are_an_error() {
    assert!(eval_err("(lambda (&named a &opt b) a)").contains("out of order"));
    assert!(eval_err("(lambda (&opt a &opt b) a)").contains("out of order"));
    assert!(eval_err("(lambda (&keys k &named a) a)").contains("out of order"));
}

#[test]
fn keys_and_a_dotted_rest_cannot_both_appear() {
    assert!(
        eval_err("(lambda (&keys k . rest) k)").contains("cannot both appear"),
        "got {}",
        eval_err("(lambda (&keys k . rest) k)")
    );
}

#[test]
fn arity_errors_still_name_the_procedure_and_the_shortfall() {
    let e = eval_err("(define (f a b &opt c) a) (f 1)");
    assert!(e.contains("f expects 2 required arguments, got 1"), "got {e}");
    let e = eval_err("(define (f a) a) (f 1 2 3)");
    assert!(e.contains("more than it takes"), "got {e}");
}

// ---- the shape ray actually calls --------------------------------------

#[test]
fn a_verb_shaped_like_rays() {
    // `(add-sphere :at [x y z] :radius r &opt :material m :name k)` as the
    // lambda list it becomes -- the thing 87 of ray's 218 verbs need.
    let src = r#"
(define (add-sphere &named at (radius 1.0) material (name :anonymous))
  (list at radius material name))

(list (add-sphere :at [0 1 0] :radius 0.8)
      (add-sphere :at [1 2 3])
      (add-sphere :at [0 0 0] :radius 2 :name :ball))
"#;
    assert_eq!(
        eval(src),
        "((#(0 1 0) 0.8 #f :anonymous) (#(1 2 3) 1.0 #f :anonymous) (#(0 0 0) 2 #f :ball))"
    );
}

#[test]
fn the_gc_sees_default_expressions() {
    // A default is source that only runs when an argument is missing, so it
    // is the easiest thing in a closure to forget to trace. Under stress the
    // slot would be reused before the default ever ran.
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            "(define (f a &opt (b (cons a a)) &named (c (list 1 2 3)))
               (list a b c))
             (f 7)",
        )
        .expect("defaults under gc stress");
    assert_eq!(write_str(&vm.engine, v), "(7 (7 . 7) (1 2 3))");
}

#[test]
fn list_is_needed_by_these_tests_and_exists() {
    assert_eq!(eval("(list 1 2 3)"), "(1 2 3)");
    assert_eq!(eval("(list)"), "()");
    assert_eq!(eval("(length (list 1 2 3))"), "3");
}

#[test]
fn a_closure_still_prints_by_name() {
    assert_eq!(eval("(define (f a &opt b) a) f"), "#<procedure f>");
    assert_eq!(
        eval("(define (f) 1) (lambda (&named a) a)"),
        "#<procedure>"
    );
}

#[test]
fn tail_calls_are_still_tail_calls_with_the_new_binder() {
    let depth = |n: i64| {
        let mut vm = Vm::new();
        let src = format!(
            "(define (loop i &opt (step 1)) (if (< i {n}) (loop (+ i step)) i)) (loop 0)"
        );
        let v = vm.eval_str(&src).expect("loop");
        assert_eq!(v, Value::Fixnum(n));
        vm.stack_high_water()
    };
    // A default DOES push a frame while it is evaluated -- but it is popped
    // before the body runs, so the loop is still flat.
    assert_eq!(depth(10), depth(50_000));
}

// ---- §4.9: accept the standard spelling, emit ours ---------------------

#[test]
fn racket_style_optional_parameters_are_accepted() {
    // `(define (f a [b 10]) …)` is what a model writes, because it is
    // Racket's spelling. A required parameter cannot carry a default, so this
    // is unambiguous -- accepted, normalised to &opt, never printed.
    assert_eq!(eval("(define (f a [b 10]) (+ a b)) (f 1)"), "11");
    assert_eq!(eval("(define (f a [b 10]) (+ a b)) (f 1 2)"), "3");
    // Mixed with the declared spelling.
    assert_eq!(
        eval("(define (f a [b 2] &named (c 3)) (list a b c)) (f 1)"),
        "(1 2 3)"
    );
    // And a bracket default sees earlier parameters, same as `&opt` does.
    assert_eq!(eval("(define (f a [b (* 2 a)]) (list a b)) (f 5)"), "(5 10)");
}

#[test]
fn hash_colon_keywords_are_accepted_but_never_emitted() {
    // Reverses an earlier decision for a stated reason: a model emits Racket's
    // and Guile's `#:at` from habit, and LLM-generated scripts are a consumer
    // (plan §4.9). Nobody has to type it, which was the actual objection.
    assert_eq!(eval("(define (f &named at) at) (f #:at 7)"), "7");
    // One canonical output: the printer only ever writes `:at`.
    assert_eq!(eval("#:at"), ":at");
    assert_eq!(eval("(eq? :at #:at)"), "#t");
    assert_eq!(eval("(keyword? #:at)"), "#t");
}

// ---- S21: a keyword parameter spelled as a keyword ----------------------

#[test]
fn a_keyword_datum_declares_a_keyword_parameter() {
    // Racket's spelling, and the reason it is here: a keyword PARAMETER is how
    // every generated ray verb shim is declared, and `&named` is Common Lisp's
    // convention by way of Janet -- so a Racket reader recognised the call and
    // then not the definition (ray's scm-port-plan.md §3.6).
    assert_eq!(eval("(define (f a #:at at) (list a at)) (f 1 :at 2)"), "(1 2)");
    // A default, in both spellings of the spec.
    assert_eq!(eval("(define (f #:at (at 7)) at) (f)"), "7");
    assert_eq!(eval("(define (f #:at [at 7]) at) (f)"), "7");
    assert_eq!(eval("(define (f #:at (at 7)) at) (f :at 1)"), "1");
    // The name comes from what FOLLOWS the keyword, not from the keyword, so
    // one may differ from the other -- as Racket's `#:material [mat #f]` does.
    assert_eq!(
        eval("(define (f #:material mat) mat) (f :material :brass)"),
        ":brass"
    );
    // An absent one is #f, like `&named`, so the ray ergonomics hold.
    assert_eq!(eval("(define (f #:m m) (if m 'given 'absent)) (f)"), "absent");
}

#[test]
fn a_keyword_parameter_is_the_same_datum_either_way() {
    // `:at` and `#:at` are one datum, so a house style can match its call
    // sites either way and both declare the same parameter.
    assert_eq!(eval("(define (f :at a) a) (f :at 3)"), "3");
    assert_eq!(eval("(define (f #:at a) a) (f #:at 3)"), "3");
    assert_eq!(eval("(define (f :at a) a) (f #:at 3)"), "3");
}

#[test]
fn a_keyword_parameter_does_not_open_a_section() {
    // Required parameters may follow one, which is what Racket does: keyword
    // arguments are order-independent. This is why S21 is handled before the
    // section dispatch and not inside it.
    assert_eq!(eval("(define (f #:at at b) (list at b)) (f 5 :at 1)"), "(1 5)");
    // And it composes with the declared markers rather than replacing them.
    assert_eq!(
        eval("(define (f a #:at at &opt b &named (c 3)) (list a at b c)) (f 1 :at 2)"),
        "(1 2 #f 3)"
    );
    assert_eq!(
        eval("(define (f &named a #:b b) (list a b)) (f :a 1 :b 2)"),
        "(1 2)"
    );
}

#[test]
fn a_keyword_with_nothing_after_it_is_an_error() {
    let e = eval_err("(define (f #:at) 1)");
    assert!(e.contains("keyword parameter needs a name"), "got {e}");
    let e = eval_err("(define (f a #:at (at 1) #:m) 1)");
    assert!(e.contains("keyword parameter needs a name"), "got {e}");
}

#[test]
fn every_tier_binds_a_keyword_parameter_the_same_way() {
    // The gate this repository holds to: a compiler that disagrees with the
    // interpreter is a bug, not a tradeoff. One parser feeds all three, which
    // is the reason this passes -- and the reason to assert it anyway.
    assert_eq!(
        eval("(define (f a #:at (at 0) #:m m) (list a at m)) (list (f 1) (f 1 :at 2 :m 3))"),
        "((1 0 #f) (1 2 3))"
    );
}

#[test]
fn the_canonical_signature_prints_the_familiar_spelling() {
    // §4.9's policy holds -- several accepted input spellings, ONE canonical
    // output -- and S21b changes which one that is: the spelling a Scheme or
    // Racket reader knows, since a signature is read far more often than a
    // lambda list is written.
    for formals in ["(f a #:at at)", "(f a :at at)", "(f a &named at)"] {
        assert_eq!(
            eval(&format!("(define {formals} at) (signature 'f)")),
            "\"(f a :at at)\"",
            "formals {formals}"
        );
    }
    // An optional prints as Racket's `[b]`, and `&keys` and a dotted rest keep
    // their spellings, having no familiar equivalent to move to.
    assert_eq!(
        eval("(define (f a &opt b &named at &keys r) 1) (signature 'f)"),
        "\"(f a [b] :at at &keys r)\""
    );
    assert_eq!(
        eval("(define (f a . rest) 1) (signature 'f)"),
        "\"(f a . rest)\""
    );
}

#[test]
fn a_printed_signature_reads_back_with_the_same_call_shape() {
    // The nearest a signature can come to `write`'s round trip, and the reason
    // the familiar spellings are safe to print: what comes out is legal input
    // that declares the same arity, the same keywords and the same rest.
    //
    // It is a call SHAPE and not a definition -- a default is not shown, so
    // the re-declaration binds #f where the original had 1, which is why the
    // comparison passes `:m` explicitly.
    let src = r#"
(define (f a &opt b &named at (m 1) &keys r) (list a b at m r))
(define (g a [b] :at at :m m &keys r)        (list a b at m r))
(list (equal? (f 1 2 :at 3 :m 4 :z 9) (g 1 2 :at 3 :m 4 :z 9))
      (signature 'f)
      (signature 'g))
"#;
    assert_eq!(
        eval(src),
        r#"(#t "(f a [b] :at at :m m &keys r)" "(g a [b] :at at :m m &keys r)")"#
    );
}
