//! S29 (docs/s29-local-macros.md): a LOCAL macro -- one bound by
//! `let-syntax`, `letrec-syntax`, or a library body -- is expanded once per
//! call site, not once per evaluation. T0 caches the expansion; T1 expands
//! it at compile time, so a `match` from `(srfi 204)` compiles to the same
//! kind of code as the `cond` it stands for.
//!
//! The danger in caching is that a `syntax-rules` template's free names
//! mean what they meant WHERE THE MACRO WAS DEFINED, and inside a procedure
//! that is a different frame on every call. An expansion whose renames
//! pointed at the first call's frame would make every later call read the
//! first call's variables. The cases below are the ways that could show,
//! and each runs on both tiers with and without a collection at every
//! safepoint (which empties the caches between any two steps). A cache
//! that bakes the frame in fails the first three lines, and fails them on
//! T0 -- checked by breaking it on purpose.

use scm::{Tier, Value, Vm, write_str};

/// Every case is one element of the answer, so a failure says which.
const ADVERSARIAL: &str = r#"
(import (srfi 204))
(define env (interaction-environment))
;; 1. a template reads an enclosing parameter; a different value per call
(define (f a) (let-syntax ((get-a (syntax-rules () ((_) a)))) (get-a)))
;; 2. shadowed at the use site: still the definition's `a`
(define (g a) (let-syntax ((get-a (syntax-rules () ((_) a)))) (let ((a 'inner)) (get-a))))
;; 3. an expansion containing a lambda, kept across calls
(define (mk a) (let-syntax ((get-a (syntax-rules () ((_) (lambda () a))))) (get-a)))
(define k1 (mk 1))
(define k2 (mk 2))
;; 4. a closure in the body that uses the macro
(define (mk2 a) (let-syntax ((m (syntax-rules () ((_) a)))) (lambda () (m))))
(define j1 (mk2 'one))
(define j2 (mk2 'two))
;; 5. recursive letrec-syntax: a hygienic temporary beside the user's `t`
(define (h x y)
  (letrec-syntax ((my-or (syntax-rules ()
                           ((_) #f) ((_ e) e)
                           ((_ e r ...) (let ((t e)) (if t t (my-or r ...)))))))
    (let ((t 5)) (my-or x y t))))
;; 6. mutual recursion
(define (parity)
  (letrec-syntax ((ev? (syntax-rules () ((_) 'even) ((_ x . r) (od? . r))))
                  (od? (syntax-rules () ((_) 'odd) ((_ x . r) (ev? . r)))))
    (list (ev? 1 2 3) (ev? 1 2))))
;; 7. let-syntax is not recursive: a template's own name is the outer one
(define (outer-m)
  (let-syntax ((m (syntax-rules () ((_) 'outer))))
    (let-syntax ((m (syntax-rules () ((_) (list 'inner (m))))))
      (m))))
;; 8. the macro's name shadowed on one path only
(define (s flag)
  (let-syntax ((m (syntax-rules () ((_) 'macro))))
    (if flag (let ((m (lambda () 'proc))) (m)) (m))))
;; 9. a macro-defining macro
(define-syntax def-getter
  (syntax-rules () ((_ name var body) (let-syntax ((name (syntax-rules () ((_) var)))) body))))
(define (p x) (def-getter gx x (gx)))
;; 10. eval: ONE call cell inside two different let-syntax forms and a let
(define form (list 'm))
(define (ev10)
  (list (eval (list 'let-syntax '((m (syntax-rules () ((_) 1)))) form) env)
        (eval (list 'let-syntax '((m (syntax-rules () ((_) 2)))) form) env)
        (eval (list 'let '((m (lambda () 3))) form) env)))
;; 11. one (syntax-rules ...) cell in two nested binding forms
(define spec '(syntax-rules () ((_) v)))
(define (ev11)
  (eval (list 'let '((v 1))
              (list 'let-syntax (list (list 'm spec))
                    (list 'let '((v 2))
                          (list 'let-syntax (list (list 'n spec)) '(list (m) (n))))))
        env))
;; 12. many calls, allocating between them
(define (q a b) (let-syntax ((sum (syntax-rules () ((_) (+ a b))))) (sum)))
(define (churn n acc)
  (if (= n 0) acc (begin (make-vector 20 n) (churn (- n 1) (+ acc (q n 1))))))
;; 13. a tail call through let-syntax does not grow the stack
(define (count-down n)
  (let-syntax ((dec (syntax-rules () ((_ x) (- x 1)))))
    (if (= n 0) 'done (count-down (dec n)))))
;; 14. a definition in a let-syntax body (T1 declines; T0 runs it)
(define (d x) (let-syntax ((m (syntax-rules () ((_ v) (define v 10))))) (m y) (+ x y)))
;; 15. a closure in a let-syntax, after a new global macro forces a recompile
(define kk (let ((a 7)) (let-syntax ((m (syntax-rules () ((_) a)))) (lambda () (m)))))
(define r15 (kk))
(define-syntax zzz (syntax-rules () ((_) 0)))
;; 16. match: literals, variables, vectors, pairs
(define (mt x)
  (match x
    (:a 'kw-a)
    (#(a b) (list 'vec a b))
    ((h . t) (list 'pair h t))
    (5 'five)
    (y (list 'other y))))
;; 17. `set!` through a local macro's template refuses, as T0's `bind` does
(define (st a) (let-syntax ((bump (syntax-rules () ((_) (set! a 99))))) (bump) a))
(list (list (f 1) (f 2) (f 3))
      (list (g 1) (g 2))
      (list (k1) (k2) (k1))
      (list (j1) (j2))
      (list (h #f #f) (h 1 #f) (h #f 2))
      (parity)
      (outer-m)
      (list (s #t) (s #f) (s #t))
      (list (p 1) (p 2))
      (ev10)
      (ev11)
      (churn 300 0)
      (count-down 100000)
      (list (d 1) (d 2))
      (list r15 (kk) (j1) (j2))
      (map mt (list :a #(1 2) '(1 2) 5 "s" #(3 4)))
      (guard (e (#t (error-object-message e))) (st 1)))
"#;

const EXPECTED: &str = "((1 2 3) (1 2) (1 2 1) (one two) (5 1 2) (odd even) (inner outer) \
(proc macro proc) (1 2) (1 2 3) (1 2) 45450 done (11 12) (7 7 one two) \
(kw-a (vec 1 2) (pair 1 (2)) five (other \"s\") (vec 3 4)) \"set!: unbound variable: a\")";

fn run(tier: Tier, stress: bool) -> String {
    let mut vm = Vm::with_tier(tier);
    vm.engine.set_gc_stress(stress);
    match vm.eval_str(ADVERSARIAL) {
        Ok(v) => write_str(&vm.engine, v),
        Err(e) => format!("failed: {e}"),
    }
}

#[test]
fn local_macros_resolve_in_each_calls_own_frame() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, false), EXPECTED, "on {tier:?}");
    }
}

#[test]
fn local_macros_resolve_in_each_calls_own_frame_under_gc_stress() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, true), EXPECTED, "on {tier:?} under gc stress");
    }
}

#[track_caller]
fn eval(vm: &mut Vm, src: &str) -> String {
    let v = vm.eval_str(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    write_str(&vm.engine, v)
}

fn global(vm: &mut Vm, name: &str) -> Value {
    let Value::Symbol(s) = vm.engine.sym(name) else { unreachable!() };
    vm.global(s).expect("defined")
}

/// How many names expansion has invented so far: a fresh symbol's id is
/// the count of symbols before it.
fn names_made(vm: &mut Vm) -> u32 {
    vm.engine.interner.fresh("probe").0
}

/// The cost this stage removed, pinned by counting rather than timing: a
/// `match` in a procedure is compiled once, so calling it expands nothing,
/// invents no names and declines nothing. Before, every call re-expanded
/// the whole tree -- `(match x (:a 1) (:b 2) (_ 3))` was ~430 µs where
/// the `cond` is 0.1 -- and every expansion minted names that are never
/// freed.
#[test]
fn a_match_in_a_procedure_compiles_and_expands_nothing_per_call() {
    let mut vm = Vm::with_tier(Tier::T1);
    eval(&mut vm, "(import (srfi 204))");
    eval(
        &mut vm,
        "(define (f x) (match x (:a 1) (:b 2) (#(p q) (+ p q)) ((h . t) h) (_ 3)))",
    );
    let f = global(&mut vm, "f");
    let declines = vm.tier_fallbacks();
    vm.call_procedure(f, vec![Value::Fixnum(0)]).unwrap();
    assert_eq!(vm.tier_fallbacks(), declines, "declined: {:?}", vm.last_decline());
    let before = names_made(&mut vm);
    let kw = vm.eval_str(":b").unwrap();
    for _ in 0..1_000 {
        assert_eq!(vm.call_procedure(f, vec![kw]).unwrap(), Value::Fixnum(2));
    }
    // One: the probe's own.
    assert_eq!(names_made(&mut vm) - before, 1, "calls of a compiled match minted names");
}

/// T1: a `let-syntax` nothing captures is a flat scope, so a procedure
/// using one allocates nothing at all, as a `let` would not.
#[test]
fn a_let_syntax_in_a_compiled_procedure_allocates_nothing() {
    let mut vm = Vm::with_tier(Tier::T1);
    eval(
        &mut vm,
        "(define (f a b) (let-syntax ((sum (syntax-rules () ((_ x) (+ x a b))))) (sum 1)))",
    );
    let f = global(&mut vm, "f");
    vm.call_procedure(f, vec![Value::Fixnum(1), Value::Fixnum(2)]).unwrap();
    let before = vm.engine.allocations();
    for i in 0..1_000 {
        let r = vm.call_procedure(f, vec![Value::Fixnum(i), Value::Fixnum(2)]).unwrap();
        assert_eq!(r, Value::Fixnum(i + 3));
    }
    let made = vm.engine.allocations() - before;
    assert_eq!(made, 0, "1,000 calls allocated {made} heap objects");
}

/// T0 expands a local macro's call site once and reuses it until the next
/// collection, so a loop of calls mints names only at the first call and
/// again after each collection, which empties the cache.
#[test]
fn t0_reuses_a_local_expansion_between_collections() {
    let mut vm = Vm::with_tier(Tier::T0);
    eval(&mut vm, "(import (srfi 204))");
    eval(&mut vm, "(define (f x) (match x (:a 1) (_ 3)))");
    eval(&mut vm, "(define (g a) (let-syntax ((m (syntax-rules () ((_) (list a))))) (m)))");
    let (f, g) = (global(&mut vm, "f"), global(&mut vm, "g"));
    vm.call_procedure(f, vec![Value::Fixnum(0)]).unwrap();
    vm.call_procedure(g, vec![Value::Fixnum(0)]).unwrap();
    let gcs = vm.engine.collections();
    let before = names_made(&mut vm);
    for i in 0..200 {
        assert_eq!(vm.call_procedure(f, vec![Value::Fixnum(i)]).unwrap(), Value::Fixnum(3));
        let r = vm.call_procedure(g, vec![Value::Fixnum(i)]).unwrap();
        assert_eq!(write_str(&vm.engine, r), format!("({i})"));
    }
    let (minted, gcs) = (names_made(&mut vm) - before, vm.engine.collections() - gcs);
    eprintln!("T0: 200 calls of each minted {minted} names across {gcs} collections");
    // Re-expanding per call minted ~50 a call (10,000 here); the cache
    // mints a match's ~50 again only after a collection has emptied it.
    assert!(minted <= 100 * (gcs as u32 + 1), "{minted} names in 200 calls, {gcs} collections");
}
