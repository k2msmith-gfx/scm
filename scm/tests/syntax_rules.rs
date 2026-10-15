//! S5, slice 1: `syntax-rules`, and the derived forms it buys.
//!
//! The tests that matter most are the hygiene ones. A `syntax-rules` that
//! expands patterns correctly but captures a user's variable is worse than
//! none, because it fails on the user's choice of name rather than on
//! anything they did.

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

// ---- hygiene ------------------------------------------------------------

#[test]
fn or_does_not_capture_the_users_t() {
    // The canonical test. `or` expands to `(let ((t e)) (if t t …))`, so an
    // unhygienic expander returns #f here -- the user's `t` gets shadowed by
    // the template's and the `if` sees the wrong one.
    assert_eq!(eval("(let ((t 5)) (or #f t))"), "5");
    assert_eq!(eval("(let ((t #f)) (or t 7))"), "7");
    // Nested, so the renaming has to be per-expansion rather than per-name.
    assert_eq!(eval("(let ((t 1)) (or #f (or #f t)))"), "1");
}

#[test]
fn cond_does_not_capture_either() {
    // `cond`'s `=>` clause and its one-armed clause both bind a temporary.
    assert_eq!(eval("(let ((t 9)) (cond (#f 1) (else t)))"), "9");
    assert_eq!(eval("(let ((t 3)) (cond (t) (else 'no)))"), "3");
    assert_eq!(
        eval("(let ((t 4)) (cond ((assv 'a '((a . 1))) => cdr) (else t)))"),
        "1"
    );
}

#[test]
fn a_template_reference_means_what_it_meant_where_the_macro_was_defined() {
    // Referential transparency, the other half. The template says `+`; the
    // call site has rebound `+` to `-`. The expansion must still add.
    let src = "
(define-syntax add2 (syntax-rules () ((_ a b) (+ a b))))
(let ((+ -)) (add2 3 2))";
    assert_eq!(eval(src), "5");
}

#[test]
fn a_user_may_shadow_a_macro_name() {
    // A macro applies only when nothing shadows it, the same rule the special
    // forms follow -- so a local called `when` is a variable.
    assert_eq!(eval("(let ((when (lambda (a b) (+ a b)))) (when 1 2))"), "3");
}

// ---- the matcher --------------------------------------------------------

#[test]
fn ellipsis_matches_zero_or_more() {
    let m = "(define-syntax lst (syntax-rules () ((_ x ...) (list x ...))))";
    assert_eq!(eval(&format!("{m} (lst)")), "()");
    assert_eq!(eval(&format!("{m} (lst 1)")), "(1)");
    assert_eq!(eval(&format!("{m} (lst 1 2 3)")), "(1 2 3)");
}

#[test]
fn ellipsis_with_a_fixed_head_and_tail() {
    let m = "(define-syntax mid (syntax-rules () ((_ a x ... b) (list a (list x ...) b))))";
    assert_eq!(eval(&format!("{m} (mid 1 9)")), "(1 () 9)");
    assert_eq!(eval(&format!("{m} (mid 1 2 3 9)")), "(1 (2 3) 9)");
}

#[test]
fn nested_ellipsis() {
    let m = "(define-syntax flat
               (syntax-rules () ((_ (a ...) ...) (list a ... ...))))";
    assert_eq!(eval(&format!("{m} (flat (1 2) (3 4))")), "(1 2 3 4)");
}

#[test]
fn ellipsis_over_a_compound_subtemplate() {
    let m = "(define-syntax pairs
               (syntax-rules () ((_ (k v) ...) (list (cons k v) ...))))";
    assert_eq!(
        eval(&format!("{m} (pairs (1 2) (3 4))")),
        "((1 . 2) (3 . 4))"
    );
}

#[test]
fn literals_match_only_themselves() {
    let m = "(define-syntax arrow
               (syntax-rules (=>) ((_ a => b) (list 'arrow a b))
                                  ((_ a b) (list 'plain a b))))";
    assert_eq!(eval(&format!("{m} (arrow 1 => 2)")), "(arrow 1 2)");
    assert_eq!(eval(&format!("{m} (arrow 1 2)")), "(plain 1 2)");
}

#[test]
fn underscore_matches_anything_without_binding() {
    let m = "(define-syntax second (syntax-rules () ((_ _ x) x)))";
    assert_eq!(eval(&format!("{m} (second 1 2)")), "2");
}

#[test]
fn rules_are_tried_in_order() {
    let m = "(define-syntax n
               (syntax-rules () ((_) 'none) ((_ a) 'one) ((_ a b ...) 'many)))";
    assert_eq!(eval(&format!("{m} (n)")), "none");
    assert_eq!(eval(&format!("{m} (n 1)")), "one");
    assert_eq!(eval(&format!("{m} (n 1 2 3)")), "many");
}

#[test]
fn no_matching_rule_is_an_error() {
    let e = eval_err("(define-syntax only1 (syntax-rules () ((_ a) a))) (only1 1 2)");
    assert!(e.contains("no matching syntax rule"), "got {e}");
}

#[test]
fn a_macro_that_expands_into_itself_is_an_error_not_a_hang() {
    // Requirement 8 again: a runaway must be reported, not spun on.
    let e = eval_err("(define-syntax spin (syntax-rules () ((_ x) (spin x)))) (spin 1)");
    assert!(e.contains("did not terminate"), "got {e}");
}

// ---- §4.9: bracket bindings, inherited by every form -------------------

#[test]
fn bracket_bindings_work_everywhere() {
    // Not implemented per-macro: the matcher treats a bracket vector as a
    // list, so every binding and clause form gets this at once.
    assert_eq!(eval("(let ([x 1] [y 2]) (+ x y))"), "3");
    assert_eq!(eval("(let* ([x 1] [y (+ x 1)]) y)"), "2");
    assert_eq!(eval("(letrec ([f (lambda (n) (if (= n 0) 1 (* n (f (- n 1)))))]) (f 5))"), "120");
    assert_eq!(eval("(cond [#f 1] [else 2])"), "2");
    assert_eq!(eval("(case 2 [(1) 'one] [(2) 'two] [else 'other])"), "two");
    assert_eq!(eval("(do ([i 0 (+ i 1)] [acc '() (cons i acc)]) [(= i 3) acc])"), "(2 1 0)");
    assert_eq!(eval("(let loop ([i 0]) (if (< i 3) (loop (+ i 1)) i))"), "3");
}

#[test]
fn a_bracket_in_an_expression_position_is_still_a_vector() {
    // Position decides, so this is unambiguous and both meanings coexist.
    assert_eq!(eval("(let ([v [1 2 3]]) (vector-ref v 1))"), "2");
    assert_eq!(eval("(let ([v [1 2 3]]) v)"), "#(1 2 3)");
}

// ---- the derived forms -------------------------------------------------

#[test]
fn let_and_friends() {
    assert_eq!(eval("(let ((x 1) (y 2)) (+ x y))"), "3");
    assert_eq!(eval("(let () 7)"), "7");
    assert_eq!(eval("(let* ((x 1) (y (+ x 1)) (z (+ y 1))) z)"), "3");
    assert_eq!(
        eval("(letrec ((even? (lambda (n) (if (= n 0) #t (odd? (- n 1)))))
                       (odd? (lambda (n) (if (= n 0) #f (even? (- n 1))))))
                (even? 100))"),
        "#t"
    );
}

#[test]
fn a_named_let_is_a_loop_and_is_still_a_tail_call() {
    assert_eq!(eval("(let loop ((i 0) (acc 0)) (if (= i 5) acc (loop (+ i 1) (+ acc i))))"), "10");
    let mut vm = Vm::new();
    let before = vm.stack_high_water();
    vm.eval_str("(let loop ((i 0)) (if (< i 50000) (loop (+ i 1)) i))")
        .expect("loop");
    let after = vm.stack_high_water();
    assert!(
        after - before < 50,
        "a named let grew the control stack by {}",
        after - before
    );
}

#[test]
fn and_or_when_unless() {
    assert_eq!(eval("(and)"), "#t");
    assert_eq!(eval("(and 1 2 3)"), "3");
    assert_eq!(eval("(and 1 #f 3)"), "#f");
    assert_eq!(eval("(or)"), "#f");
    assert_eq!(eval("(or #f #f 3)"), "3");
    assert_eq!(eval("(when #t 1 2)"), "2");
    // R7RS: the value of a `when` whose test is false is unspecified, and
    // `#f` would be legal but would make it indistinguishable from a body
    // that returned false on purpose.
    assert_eq!(eval("(when #f 1)"), "#<unspecified>");
    assert_eq!(eval("(unless #f 'yes)"), "yes");
    assert_eq!(eval("(unless #t 'no)"), "#<unspecified>");
    // Short-circuiting is real, not incidental.
    assert_eq!(eval("(define n 0) (and #f (set! n 1)) n"), "0");
    assert_eq!(eval("(define n 0) (or #t (set! n 1)) n"), "0");
}

#[test]
fn cond_and_case() {
    assert_eq!(eval("(cond (#f 1) (#t 2) (else 3))"), "2");
    assert_eq!(eval("(cond (#f 1) (else 3))"), "3");
    assert_eq!(eval("(cond (42))"), "42");
    assert_eq!(eval("(cond (#f 1))"), "#f");
    assert_eq!(eval("(case 'b ((a) 1) ((b c) 2) (else 3))"), "2");
    assert_eq!(eval("(case 'z ((a) 1) (else 3))"), "3");
    assert_eq!(eval("(case 2 ((1 2 3) 'small))"), "small");
}

#[test]
fn do_loops() {
    assert_eq!(
        eval("(do ((i 0 (+ i 1)) (acc 1 (* acc 2))) ((= i 5) acc))"),
        "32"
    );
    // A variable with no step keeps its value.
    assert_eq!(eval("(do ((i 0 (+ i 1)) (n 10)) ((= i 3) n))"), "10");
}

// ---- the list vocabulary ----------------------------------------------

#[test]
fn list_operations() {
    assert_eq!(eval("(map (lambda (x) (* x x)) '(1 2 3))"), "(1 4 9)");
    assert_eq!(eval("(map + '(1 2 3) '(10 20 30))"), "(11 22 33)");
    assert_eq!(eval("(filter (lambda (x) (> x 2)) '(1 2 3 4))"), "(3 4)");
    assert_eq!(eval("(reverse '(1 2 3))"), "(3 2 1)");
    assert_eq!(eval("(append '(1 2) '(3) '(4 5))"), "(1 2 3 4 5)");
    assert_eq!(eval("(list-ref '(a b c) 1)"), "b");
    assert_eq!(eval("(member 2 '(1 2 3))"), "(2 3)");
    assert_eq!(eval("(assoc 'b '((a 1) (b 2)))"), "(b 2)");
    assert_eq!(eval("(list? '(1 2))"), "#t");
    assert_eq!(eval("(list? '(1 . 2))"), "#f");
    assert_eq!(eval("(reduce + 0 '(1 2 3 4))"), "10");
    assert_eq!(eval("(cadr '(1 2 3))"), "2");
    assert_eq!(
        eval("(define n 0) (for-each (lambda (x) (set! n (+ n x))) '(1 2 3)) n"),
        "6"
    );
}

#[test]
fn map_is_a_loop_not_recursion() {
    // A scene maps over thousands of points, and the standard idiom -- which
    // is what a model writes -- must therefore be flat.
    let mut vm = Vm::new();
    vm.eval_str("(define big (let loop ((i 0) (acc '())) (if (= i 20000) acc (loop (+ i 1) (cons i acc)))))")
        .expect("build");
    let before = vm.stack_high_water();
    vm.eval_str("(define out (map (lambda (x) (+ x 1)) big))")
        .expect("map");
    assert!(
        vm.stack_high_water() - before < 100,
        "map grew the stack by {}",
        vm.stack_high_water() - before
    );
    assert_eq!(
        vm.eval_str("(length out)")
            .map(|v| write_str(&vm.engine, v))
            .unwrap(),
        "20000"
    );
}

// ---- the scene corpus's iteration shapes ------------------------------

#[test]
fn the_janet_iteration_names_survive() {
    // 253 `seq`, 174 `for`, 156 `each` in the corpus. Keeping the names means
    // the conversion translates rather than rewrites.
    assert_eq!(eval("(seq (i 0 4) (* i i))"), "(0 1 4 9)");
    assert_eq!(eval("(seq (x in '(1 2 3)) (* x 10))"), "(10 20 30)");
    assert_eq!(
        eval("(define n 0) (for i 0 5 (set! n (+ n i))) n"),
        "10"
    );
    assert_eq!(
        eval("(define n 0) (each x '(1 2 3) (set! n (+ n x))) n"),
        "6"
    );
    assert_eq!(eval("(define n 0) (repeat 4 (set! n (+ n 1))) n"), "4");
    // And they nest, which is how a scene builds a grid.
    assert_eq!(
        eval("(seq (i 0 2) (seq (j 0 2) (list i j)))"),
        "(((0 0) (0 1)) ((1 0) (1 1)))"
    );
}

#[test]
fn a_scene_shaped_fragment_runs() {
    // The shape a converted scene has: stub verbs, keywords, brackets,
    // computed vectors, a loop accumulating points.
    let src = r#"
(define calls '())
(define (record! name args) (set! calls (cons (cons name args) calls)))
(define (add-sphere &named at radius material)
  (record! 'add-sphere (list at radius material)))

(define (ring n r)
  (seq (i 0 n)
    (let* ([a (* 6.2831853 (/ i n))]
           [x (* r (cos a))]
           [z (* r (sin a))])
      [x 0 z])))

(for-each (lambda (p) (add-sphere :at p :radius 0.2)) (ring 4 1.0))
(length calls)
"#;
    assert_eq!(eval(src), "4");
}

#[test]
fn the_prelude_survives_gc_stress() {
    // The prelude is itself 200-odd lines of macros and closures, all live in
    // the globals. Collecting at every safepoint while using it is a decent
    // check that nothing in the expander drops a root.
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm
        .eval_str(
            "(let loop ((i 0) (acc '()))
               (if (= i 50)
                   (length (filter (lambda (x) (> x 10)) (map (lambda (x) (* x 2)) acc)))
                   (loop (+ i 1) (cons i acc))))",
        )
        .expect("prelude under gc stress");
    assert_eq!(write_str(&vm.engine, v), "44");
}
