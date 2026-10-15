//! S2's exit criteria, plus the semantics the explicit stack exists to make
//! possible.
//!
//! The headline one: **a tail loop must be flat.** It is measured on the
//! control stack's high-water mark rather than on memory, because the stack
//! depth is the thing that either grows or does not -- RSS would only tell us
//! about the allocator.

use scm::{Value, Vm, VmError, write_str};

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
        Ok(v) => panic!("{src:?} should have failed, gave {}", write_str(&vm.engine, v)),
        Err(e) => format!("{e}"),
    }
}

#[test]
fn atoms_and_arithmetic() {
    assert_eq!(eval("42"), "42");
    assert_eq!(eval("(+ 1 2 3)"), "6");
    assert_eq!(eval("(- 10 3)"), "7");
    assert_eq!(eval("(- 5)"), "-5");
    assert_eq!(eval("(* 2 3 4)"), "24");
    assert_eq!(eval("(+ 1 2.5)"), "3.5");
    assert_eq!(eval("(< 1 2 3)"), "#t");
    assert_eq!(eval("(< 1 3 2)"), "#f");
    assert_eq!(eval("(= 2 2.0)"), "#t");
}

#[test]
fn division_is_inexact_without_ratios() {
    // Plan §4.5. `(/ 8 4)` stays exact; `(/ 8 3)` becomes inexact rather than
    // the rational 8/3, which is what silently turned Steel's lorenz into
    // bignum arithmetic.
    assert_eq!(eval("(/ 8 4)"), "2");
    assert_eq!(eval("(/ 8 3)"), "2.6666666666666665");
    assert_eq!(eval("(/ 2)"), "0.5");
    assert!(eval_err("(/ 1 0)").contains("division by exact zero"));
}

#[test]
fn fixnum_overflow_is_an_error_not_a_widening() {
    // The settled decision: losing exactness quietly is how a render comes
    // out wrong instead of raising.
    let e = eval_err("(* 9223372036854775807 2)");
    assert!(e.contains("overflow"), "got {e}");
    assert!(e.contains("exact-closed"), "got {e}");
}

#[test]
fn closures_and_lexical_scope() {
    assert_eq!(eval("((lambda (x) (* x x)) 7)"), "49");
    assert_eq!(
        eval("(define (adder n) (lambda (x) (+ x n))) ((adder 3) 4)"),
        "7"
    );
    // A closure outlives the call that made it, which is why environments and
    // the control stack are separate structures.
    assert_eq!(
        eval("(define f (lambda () (define c 0) (lambda () (set! c (+ c 1)) c))) (define g (f)) (g) (g) (g)"),
        "3"
    );
}

#[test]
fn variadic_and_rest_parameters() {
    assert_eq!(eval("((lambda args args) 1 2 3)"), "(1 2 3)");
    assert_eq!(eval("((lambda (a . rest) rest) 1 2 3)"), "(2 3)");
    assert_eq!(eval("((lambda (a . rest) a) 1)"), "1");
}

#[test]
fn arity_errors_name_the_procedure() {
    let e = eval_err("(define (f a b) a) (f 1)");
    assert!(e.contains("f expects 2 required arguments, got 1"), "got {e}");
}

#[test]
fn define_set_and_globals_are_cells() {
    assert_eq!(eval("(define x 1) (set! x 2) x"), "2");
    assert!(eval_err("(set! nope 1)").contains("unbound"));
    assert!(eval_err("undefined-thing").contains("unbound variable"));
    // Redefinition takes effect for callers that already closed over the
    // name, because a global is a cell and never an inlined value (plan §1).
    assert_eq!(
        eval("(define (f) (g)) (define (g) 1) (define a (f)) (define (g) 2) (+ a (f))"),
        "3"
    );
}

#[test]
fn brackets_construct_and_hash_vectors_do_not() {
    // The payoff of the reader's `bracket` flag (plan §4.5): same elements,
    // different meaning. The difference is in the *value* -- one has been
    // evaluated, the other has not -- and not in how it prints, because
    // both print canonically (plan §4.9).
    assert_eq!(eval("(define r 2) [0 (* 2 r) 0]"), "#(0 4 0)");
    assert_eq!(eval("'#(0 (* 2 r) 0)"), "#(0 (* 2 r) 0)");
    assert_eq!(eval("#(1 2 3)"), "#(1 2 3)");
    assert_eq!(eval("[]"), "#()");
    assert_eq!(eval("(vector-ref [10 20 30] 1)"), "20");
    assert_eq!(eval("(vector-length [1 2 3])"), "3");
}

#[test]
fn keywords_self_evaluate_and_are_not_symbols() {
    assert_eq!(eval(":aces"), ":aces");
    assert_eq!(eval("(keyword? :aces)"), "#t");
    assert_eq!(eval("(symbol? :aces)"), "#f");
    assert_eq!(eval("(symbol? 'aces)"), "#t");
    assert_eq!(eval("(eq? :aces :aces)"), "#t");
    assert_eq!(eval("(equal? :aces 'aces)"), "#f");
}

#[test]
fn quote_if_and_begin() {
    assert_eq!(eval("'(1 2 3)"), "(1 2 3)");
    assert_eq!(eval("(if #t 1 2)"), "1");
    assert_eq!(eval("(if #f 1 2)"), "2");
    assert_eq!(eval("(if #f 1)"), "#<unspecified>");
    // Only #f is false.
    assert_eq!(eval("(if 0 'yes 'no)"), "yes");
    assert_eq!(eval("(if '() 'yes 'no)"), "yes");
    assert_eq!(eval("(begin 1 2 3)"), "3");
    assert_eq!(eval("(begin)"), "#<unspecified>");
}

#[test]
fn a_local_may_shadow_a_special_form_name() {
    // `if` is syntax only when nothing shadows it. A verb called `begin` --
    // and the modeler's tool lifecycle has one -- must stay callable.
    assert_eq!(eval("((lambda (begin) (begin 1 2)) (lambda (a b) (+ a b)))"), "3");
}

// ---- the exit criterion -------------------------------------------------

#[test]
fn a_tail_loop_is_flat() {
    // The measurement, not an assertion by inspection: run the same tail loop
    // at two very different trip counts and insist the control stack reached
    // exactly the same depth.
    let depth = |n: i64| {
        let mut vm = Vm::new();
        let src = format!("(define (loop i) (if (< i {n}) (loop (+ i 1)) i)) (loop 0)");
        let v = vm.eval_str(&src).expect("loop");
        assert_eq!(v, Value::Fixnum(n));
        vm.stack_high_water()
    };
    let shallow = depth(10);
    let deep = depth(100_000);
    assert_eq!(
        shallow, deep,
        "tail recursion grew the control stack: {shallow} -> {deep}"
    );
}

#[test]
fn mutual_tail_recursion_is_flat_too() {
    let depth = |n: i64| {
        let mut vm = Vm::new();
        let src = format!(
            "(define (even? n) (if (= n 0) #t (odd? (- n 1))))
             (define (odd? n) (if (= n 0) #f (even? (- n 1))))
             (even? {n})"
        );
        vm.eval_str(&src).expect("mutual");
        vm.stack_high_water()
    };
    assert_eq!(depth(10), depth(50_000));
}

#[test]
fn a_tail_call_from_begin_and_if_is_still_a_tail_call() {
    let depth = |n: i64| {
        let mut vm = Vm::new();
        let src = format!(
            "(define (loop i)
               (begin
                 0
                 (if (< i {n}) (loop (+ i 1)) i)))
             (loop 0)"
        );
        vm.eval_str(&src).expect("loop");
        vm.stack_high_water()
    };
    assert_eq!(depth(10), depth(50_000));
}

#[test]
fn non_tail_recursion_grows_the_explicit_stack_not_the_c_stack() {
    // Deep *non-tail* recursion is supposed to grow -- it has pending work.
    // What matters is that it grows a Vec rather than the C stack, so a
    // runaway cannot abort the host (requirement 8). 200k frames would have
    // overflowed a recursive-descent interpreter long ago.
    let mut vm = Vm::new();
    let v = vm
        .eval_str("(define (sum i) (if (= i 0) 0 (+ i (sum (- i 1))))) (sum 200000)")
        .expect("deep non-tail recursion");
    assert_eq!(v, Value::Fixnum(200000 * 200001 / 2));
    assert!(
        vm.stack_high_water() > 200000,
        "expected a deep explicit stack, got {}",
        vm.stack_high_water()
    );
}

// ---- requirement 8: the host's controls --------------------------------

#[test]
fn fuel_stops_a_runaway() {
    let mut vm = Vm::new();
    vm.set_fuel(Some(50));
    let err = vm
        .eval_str("(define (spin i) (spin (+ i 1))) (spin 0)")
        .expect_err("should run out of fuel");
    assert_eq!(err, VmError::OutOfFuel);
}

#[test]
fn an_interrupt_from_another_thread_stops_a_runaway() {
    // This is the throttle on AI-generated code that the Steel evaluation
    // found Steel would not have given ray either.
    let mut vm = Vm::new();
    let flag = vm.interrupt_handle();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let err = vm
        .eval_str("(define (spin i) (spin (+ i 1))) (spin 0)")
        .expect_err("should be interrupted");
    assert_eq!(err, VmError::Interrupted);
}

#[test]
fn the_vm_survives_what_it_refused() {
    // A failed evaluation must leave the VM usable, or "cannot abort the
    // host" is only half kept.
    let mut vm = Vm::new();
    assert!(vm.eval_str("(car 1)").is_err());
    assert!(vm.eval_str("(+ 1 'a)").is_err());
    vm.set_fuel(Some(10));
    assert!(vm.eval_str("(define (spin i) (spin (+ i 1))) (spin 0)").is_err());
    vm.set_fuel(None);
    let v = vm.eval_str("(+ 1 2)").expect("still alive");
    assert_eq!(v, Value::Fixnum(3));
}

#[test]
fn errors_say_what_and_who() {
    assert!(eval_err("(car 1)").contains("car: not a pair"));
    assert!(eval_err("(+ 1 'a)").contains("+: not a number"));
    assert!(eval_err("(1 2)").contains("not applicable"));
    assert!(eval_err("()").contains("empty application"));
}

#[test]
fn a_global_has_a_version_that_moves_on_every_store() {
    let mut vm = scm::Vm::new();
    let x = vm.engine.interner.symbol("x");
    let y = vm.engine.interner.symbol("y");
    assert_eq!(vm.global_version(x), 0);
    vm.eval_str("(define x 1)").unwrap();
    assert_eq!(vm.global_version(x), 1);
    vm.eval_str("(set! x 2) (define x (vector 1))").unwrap();
    assert_eq!(vm.global_version(x), 3);
    // In-place mutation of the value is not a store to the binding.
    vm.eval_str("(vector-set! x 0 5)").unwrap();
    assert_eq!(vm.global_version(x), 3);
    assert_eq!(vm.global_version(y), 0);
}
