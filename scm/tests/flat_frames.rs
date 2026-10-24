//! S14 (docs/s14-flat-frames-plan.md): a variable nothing captures lives in
//! a slot of its frame on the value stack, so binding it allocates nothing;
//! a variable something does capture is still an `Obj::Env`, and behaves
//! exactly as it did. The differential fuzzer checks the second half at
//! scale (its arms 50-53); these pin the shapes by name.

use scm::{Value, Vm, write_str};

#[track_caller]
fn eval(vm: &mut Vm, src: &str) -> String {
    let v = vm.eval_str(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    write_str(&vm.engine, v)
}

fn global(vm: &mut Vm, name: &str) -> Value {
    let Value::Symbol(s) = vm.engine.sym(name) else { unreachable!() };
    vm.global(s).expect("defined")
}

/// The point of S14: a procedure whose parameters and `let`s nothing
/// captures is called, binds and returns without one heap object.
#[test]
fn a_flat_call_allocates_nothing() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (f a b) (let* ((c (+ a b)) (d (* c c))) (- d a)))");
    let f = global(&mut vm, "f");
    // The first call compiles; count from the second.
    vm.call_procedure(f, vec![Value::Fixnum(1), Value::Fixnum(2)]).unwrap();
    let before = vm.engine.allocations();
    for i in 0..10_000 {
        let r = vm.call_procedure(f, vec![Value::Fixnum(i), Value::Fixnum(2)]).unwrap();
        assert_eq!(r, Value::Fixnum((i + 2) * (i + 2) - i));
    }
    let made = vm.engine.allocations() - before;
    assert_eq!(made, 0, "10,000 flat calls allocated {made} heap objects");
}

/// The control: the same procedure with a closure over its `let` must
/// still allocate its environment -- if this one allocated nothing too, the
/// test above would prove nothing.
#[test]
fn a_captured_let_still_allocates() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (g a b) (let ((c (+ a b))) ((lambda () c))))");
    let g = global(&mut vm, "g");
    vm.call_procedure(g, vec![Value::Fixnum(1), Value::Fixnum(2)]).unwrap();
    let before = vm.engine.allocations();
    for i in 0..1_000 {
        vm.call_procedure(g, vec![Value::Fixnum(i), Value::Fixnum(2)]).unwrap();
    }
    assert!(vm.engine.allocations() > before, "a captured let allocated nothing: is anything flat being captured?");
}

#[test]
fn a_closure_that_outlives_its_let_keeps_the_variable() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define counter (let ((n 0)) (lambda () (set! n (+ n 1)) n)))");
    assert_eq!(eval(&mut vm, "(begin (counter) (counter) (counter))"), "3");
    // A parameter captured by a closure that escapes the call.
    eval(&mut vm, "(define (adder k) (lambda (x) (+ x k)))");
    assert_eq!(eval(&mut vm, "((adder 10) 5)"), "15");
}

#[test]
fn shadowing_lets_and_set_of_flat_variables() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let ((s 1)) (+ (let ((s 10)) s) s))"), "11");
    assert_eq!(eval(&mut vm, "(let* ((s 1) (s (+ s 1)) (s (* s 10))) s)"), "20");
    eval(&mut vm, "(define (bump p) (set! p (+ p 1)) (let ((q p)) (set! q (* q 2)) (+ p q)))");
    assert_eq!(eval(&mut vm, "(bump 4)"), "15");
}

/// An internal define's slot is unbound until the define runs, in a flat
/// frame as in an `Obj::Env` -- same message as T0's.
#[test]
fn a_flat_define_is_unbound_until_it_runs() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (early) (let ((r v)) (define v 1) r))");
    let e = vm.eval_str("(early)").expect_err("v is read before its define");
    assert!(format!("{e}").contains("unbound variable: v"), "{e}");
    eval(&mut vm, "(define (late) (define v 1) (let ((w (+ v 1))) (define u (* w 3)) u))");
    assert_eq!(eval(&mut vm, "(late)"), "6");
}

/// A guard is a block, run as its own frame: the variables it reads are
/// captured, and it reads their current values.
#[test]
fn a_guard_reads_the_enclosing_flat_frame() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (safe p) (let ((q (* p 2))) (guard (e (#t (+ p q))) (set! q (+ q 1)) (raise 'x))))");
    assert_eq!(eval(&mut vm, "(safe 5)"), "16");
}

/// A deep tail self-call in a flat frame runs in constant stack.
#[test]
fn a_flat_tail_call_does_not_grow_the_stack() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (count n acc) (let ((m (- n 1))) (if (= n 0) acc (count m (+ acc 1)))))");
    assert_eq!(eval(&mut vm, "(count 200000 0)"), "200000");
}

/// `apply`, `map` and an escape continuation through flat frames.
#[test]
fn the_other_doors_into_a_flat_procedure() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (sq x) (let ((y (* x x))) y))");
    assert_eq!(eval(&mut vm, "(apply sq (list 7))"), "49");
    assert_eq!(eval(&mut vm, "(map sq (list 1 2 3))"), "(1 4 9)");
    assert_eq!(
        eval(&mut vm, "(call/cc (lambda (k) (define (walk n) (let ((m (sq n))) (if (> m 50) (k m) (walk (+ n 1))))) (walk 1)))"),
        "64"
    );
    // The escape left the value stack where it found it.
    assert_eq!(eval(&mut vm, "(+ (sq 3) (sq 4))"), "25");
}

// ── S14c: a named let whose name is only called in loop-tail position is a
// loop: its variables are slots and a self-call is a jump.

#[test]
fn a_loop_allocates_nothing() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (sum-to n) (let loop ((i 0) (acc 0)) (if (> i n) acc (loop (+ i 1) (+ acc i)))))");
    eval(&mut vm, "(define (count-do n) (do ((i 0 (+ i 1)) (acc 0 (+ acc 2))) ((= i n) acc)))");
    let f = global(&mut vm, "sum-to");
    let g = global(&mut vm, "count-do");
    vm.call_procedure(f, vec![Value::Fixnum(1)]).unwrap();
    vm.call_procedure(g, vec![Value::Fixnum(1)]).unwrap();
    let before = vm.engine.allocations();
    assert_eq!(vm.call_procedure(f, vec![Value::Fixnum(100_000)]).unwrap(), Value::Fixnum(5_000_050_000));
    assert_eq!(vm.call_procedure(g, vec![Value::Fixnum(100_000)]).unwrap(), Value::Fixnum(200_000));
    let made = vm.engine.allocations() - before;
    assert_eq!(made, 0, "200,000 loop iterations allocated {made} heap objects");
}

/// A closure made in the body captures that ITERATION's binding -- the
/// loop must stay a closure for this to hold, and it does.
#[test]
fn closures_made_in_a_loop_see_their_own_iteration() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(&mut vm, "(let loop ((i 0) (fs '())) (if (= i 3) (map (lambda (f) (f)) fs) (loop (+ i 1) (cons (lambda () i) fs))))"),
        "(2 1 0)"
    );
}

/// The name used as a value, called outside tail position, or with the
/// wrong count: each makes it the closure, with the closure's behaviour.
#[test]
fn a_loop_name_used_any_other_way_is_the_closure() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let fact ((n 5)) (if (= n 0) 1 (* n (fact (- n 1)))))"), "120");
    assert_eq!(eval(&mut vm, "(let me ((n 1)) (if (procedure? me) n 0))"), "1");
    assert_eq!(eval(&mut vm, "(let l ((n 3)) (if (= n 0) 'done (apply l (list (- n 1)))))"), "done");
    let e = vm.eval_str("(let l ((n 3)) (if (= n 0) 0 (l)))").expect_err("wrong count");
    assert!(format!("{e}").contains("expects 1 required argument"), "{e}");
}

/// An inner loop in tail position of an outer one may jump to either.
#[test]
fn an_inner_loop_jumps_to_the_outer() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(&mut vm, "(let outer ((i 0) (acc 0)) (if (= i 3) acc (let inner ((j 0) (acc acc)) (if (= j 4) (outer (+ i 1) acc) (inner (+ j 1) (+ acc 1))))))"),
        "12"
    );
    // A loop in NON-tail position of another is still a loop of its own.
    assert_eq!(
        eval(&mut vm, "(let outer ((i 0) (acc 0)) (if (= i 3) acc (outer (+ i 1) (+ acc (let inner ((j 0)) (if (= j 2) j (inner (+ j 1))))))))"),
        "6"
    );
}

/// A define in a loop body is unbound until it runs, on every iteration.
#[test]
fn a_loop_body_define_is_fresh_each_time() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let loop ((i 0) (acc 0)) (define d (* i 10)) (if (= i 3) acc (loop (+ i 1) (+ acc d))))"), "30");
}
