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

// ── S14e: arithmetic and comparisons as one op, guarded on the callee being
// the primitive, with the primitive itself for every case not answered in
// place. Each of these is a case where a fast path could differ.

#[test]
fn inline_arithmetic_is_the_primitive_in_every_case() {
    let mut vm = Vm::new();
    let mut ev = |s: &str| eval(&mut vm, s);
    // fixnum overflow widens to a bignum, as the primitive does
    assert_eq!(ev("(let ((x 9223372036854775807)) (+ x 1))"), "9223372036854775808");
    assert_eq!(ev("(let ((x -9223372036854775808)) (- x))"), "9223372036854775808");
    // unary minus is 0 - x: (- 0.0) is 0.0, not -0.0
    assert_eq!(ev("(let ((z 0.0)) (eqv? (- z) 0.0))"), "#t");
    // mixed fixnum/flonum folds to a flonum
    assert_eq!(ev("(let ((a 2) (b 0.5)) (list (+ a b) (* a b) (- a b) (/ a b)))"), "(2.5 1.0 1.5 4.0)");
    // exact division stays the primitive's: exact when it divides, and the
    // exact-zero error
    assert_eq!(ev("(let ((a 8) (b 4)) (/ a b))"), "2");
    assert_eq!(ev("(let ((a 8) (b 3)) (exact? (/ a b)))"), "#f");
    let e = vm.eval_str("(let ((a 1) (b 0)) (/ a b))").expect_err("exact zero");
    assert!(format!("{e}").contains("division by exact zero"), "{e}");
}

#[test]
fn inline_comparisons_are_exact_where_the_primitive_is() {
    let mut vm = Vm::new();
    // a mixed comparison is exact, not by conversion: 2^53 + 1 is not 2^53.0
    assert_eq!(eval(&mut vm, "(let ((a 9007199254740993) (b 9007199254740992.0)) (= a b))"), "#f");
    assert_eq!(eval(&mut vm, "(let ((a 9007199254740993) (b 9007199254740992.0)) (> a b))"), "#t");
    // NaN compares false every way
    assert_eq!(eval(&mut vm, "(let ((n (/ 0.0 0.0))) (list (= n n) (< n 1.0) (>= n 1.0)))"), "(#f #f #f)");
    assert_eq!(eval(&mut vm, "(let ((a 3) (b 3)) (list (= a b) (<= a b) (< a b) (>= a b) (> a b)))"), "(#t #t #f #t #f)");
}

#[test]
fn inline_arithmetic_errors_are_the_primitives() {
    let mut vm = Vm::new();
    let e = vm.eval_str("(let ((a 1) (b \"x\")) (+ a b))").expect_err("not a number");
    let direct = vm.eval_str("(apply + (list 1 \"x\"))").expect_err("not a number");
    assert_eq!(format!("{e}"), format!("{direct}"));
}

/// A redefined `+` is seen on the next execution -- and an operand that
/// redefines it does NOT change which `+` the call already evaluated, as on
/// T0, where the callee is evaluated first.
#[test]
fn a_redefined_operator_is_honoured_in_order() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (add a b) (+ a b))");
    assert_eq!(eval(&mut vm, "(add 2 3)"), "5");
    eval(&mut vm, "(define saved +)");
    eval(&mut vm, "(set! + (lambda (a b) (* a b)))");
    assert_eq!(eval(&mut vm, "(add 2 3)"), "6");
    eval(&mut vm, "(set! + saved)");
    assert_eq!(eval(&mut vm, "(add 2 3)"), "5");
    assert_eq!(eval(&mut vm, "(define (f) (+ (begin (set! + -) 10) 1)) (let ((r (f))) (set! + saved) r)"), "11");
}

// ---- S30: superinstructions ----------------------------------------------

/// S30: an operation whose operands are flat slots or constants is one op
/// with the callee's read folded in; the result and every error are still
/// the primitive's, and a redefined operator is still seen.
#[test]
fn fused_arithmetic_is_the_primitive_in_every_case() {
    let mut vm = Vm::new();
    let mut ev = |s: &str| eval(&mut vm, s);
    ev("(define (f a b) (list (+ a b) (- a b) (* a b) (< a b) (= a b) (- a) (* a 2.5) (+ 1 b)))");
    assert_eq!(ev("(f 3 4)"), "(7 -1 12 #t #f -3 7.5 5)");
    // overflow, a bignum, a mixed exact comparison: the slow path
    assert_eq!(ev("(f 9223372036854775807 1)"), "(9223372036854775808 9223372036854775806 9223372036854775807 #f #f -9223372036854775807 23058430092136940000.0 2)");
    // the same through `apply`, which is the primitive with nothing fused
    ev("(define (ref a b) (list (apply + (list a b)) (apply - (list a b)) (apply * (list a b)) (apply < (list a b)) (apply = (list a b)) (apply - (list a)) (apply * (list a 2.5)) (apply + (list 1 b))))");
    for args in ["9223372036854775807 1", "9007199254740993 9007199254740992.0", "-9223372036854775808 -1", "2.5 0.5", "7 0.0", "3 4"] {
        assert_eq!(ev(&format!("(f {args})")), ev(&format!("(ref {args})")), "{args}");
    }
    let e = vm.eval_str("(f 1 \"x\")").expect_err("not a number");
    let direct = vm.eval_str("(apply + (list 1 \"x\"))").expect_err("not a number");
    assert_eq!(format!("{e}"), format!("{direct}"));
    // the folded callee is read at the op, so a redefinition is seen
    eval(&mut vm, "(define saved +)");
    eval(&mut vm, "(set! + (lambda (a b) (* a b)))");
    assert_eq!(eval(&mut vm, "(car (f 3 4))"), "12");
    eval(&mut vm, "(set! + saved)");
    assert_eq!(eval(&mut vm, "(car (f 3 4))"), "7");
}

/// A comparison that is an `if`'s test takes the branch itself; when the
/// fast path declines (a mixed pair, a non-number) the branch after it
/// still runs on the primitive's answer -- or the primitive's error.
#[test]
fn a_fused_comparison_branches_on_the_primitives_answer() {
    let mut vm = Vm::new();
    let mut ev = |s: &str| eval(&mut vm, s);
    ev("(define (g a b) (if (< a b) 'lt (if (= a b) 'eq 'gt)))");
    assert_eq!(ev("(list (g 1 2) (g 2 2) (g 3 2) (g 1.0 2.0) (g 2.5 2.5))"), "(lt eq gt lt eq)");
    // mixed exact: 2^53 + 1 against 2^53.0 is gt, not eq
    assert_eq!(ev("(g 9007199254740993 9007199254740992.0)"), "gt");
    assert_eq!(ev("(g 9007199254740992 9007199254740992.0)"), "eq");
    // the operands on the stack, both and one
    ev("(define (h a b) (if (< (* a 1) (* b 1)) 'lt 'ge))");
    assert_eq!(ev("(list (h 1 2) (h 2 1))"), "(lt ge)");
    ev("(define (k a b) (if (< a (* b 1)) 'lt 'ge))");
    assert_eq!(ev("(list (k 1 2) (k 2 1))"), "(lt ge)");
    // a redefined comparison is honoured
    ev("(define saved <)");
    ev("(set! < (lambda (a b) (> a b)))");
    assert_eq!(ev("(g 1 2)"), "gt");
    ev("(set! < saved)");
    assert_eq!(ev("(g 1 2)"), "lt");
    let e = vm.eval_str("(g 'a 1)").expect_err("not a number");
    let direct = vm.eval_str("(apply < (list 'a 1))").expect_err("not a number");
    assert_eq!(format!("{e}"), format!("{direct}"));
}

/// Reading a slot can move later than written only when nothing in between
/// writes it: a `set!` of the slot in the second operand keeps the read
/// where it was, and a jump inside the second operand is relocated.
#[test]
fn a_folded_read_keeps_its_order() {
    let mut vm = Vm::new();
    let mut ev = |s: &str| eval(&mut vm, s);
    ev("(define (s1 a) (+ a (begin (set! a 5) 1)))");
    assert_eq!(ev("(s1 3)"), "4");
    ev("(define (s2 a) (- a (begin (set! a 0) 1)))");
    assert_eq!(ev("(s2 3)"), "2");
    // the second operand holds an `if` (two jumps past the deleted read)
    ev("(define (j a) (+ a (if (> a 2) (* a 10) (- a))))");
    assert_eq!(ev("(list (j 3) (j 1))"), "(33 0)");
    // and a loop (not folded: a loop jump writes slots)
    ev("(define (l a) (+ a (let lp ((i 0)) (if (= i a) i (lp (+ i 1))))))");
    assert_eq!(ev("(l 4)"), "8");
    // an inner `if` whose end is the outer test's jump: the comparison's
    // branch fires only when the comparison itself ran
    ev("(define (t a c) (if (if c #t (< a 0)) 'yes 'no))");
    assert_eq!(ev("(list (t 1 #t) (t 1 #f) (t -1 #f))"), "(yes no yes)");
}

/// The pending-call ranges survive the deletions: a backtrace from inside
/// the second operand still names the fused call as pending, and one from
/// the fused op itself does not (its range is empty, as `Arith`'s was).
#[test]
fn a_backtrace_through_a_fused_op_names_the_pending_call() {
    use scm::{Op, Outcome, Request, Session};
    let mut s = Session::new().unwrap();
    let r = s.handle(Request { id: 1, op: Op::Eval { code: "(define (bt a) (+ a (car a))) (bt 1)".into(), source: Some("<t>".into()) } });
    let Outcome::Failed { message, backtrace, .. } = r.outcome else { panic!("expected a failure") };
    assert!(message.contains("not a pair"), "{message}");
    assert!(backtrace[0].starts_with("(car a)"), "{:?}", backtrace);
    assert!(backtrace[1].starts_with("(+ a (car a))"), "{:?}", backtrace);
    let r = s.handle(Request { id: 2, op: Op::Eval { code: "(define (bu a) (+ a 'x)) (bu 1)".into(), source: Some("<t>".into()) } });
    let Outcome::Failed { message, backtrace, .. } = r.outcome else { panic!("expected a failure") };
    assert!(message.contains("number"), "{message}");
    assert!(backtrace[0].starts_with("(+ a 'x)"), "{:?}", backtrace);
}

/// The folded callee is looked up when the op runs, with `Global`'s error.
#[test]
fn a_folded_callee_that_is_unbound_fails_as_a_global_read() {
    let mut vm = Vm::new();
    eval(&mut vm, "(define (u a b) (+ a b))");
    eval(&mut vm, "(define saved +)");
    // `+` removed from the globals is not something Scheme can do, so
    // redefine it as a non-procedure: the fused op must hand it to the
    // ordinary call path, whose error names the value.
    eval(&mut vm, "(set! + 42)");
    let e = vm.eval_str("(u 1 2)").expect_err("not a procedure");
    eval(&mut vm, "(set! + saved)");
    assert!(format!("{e}").contains("42") || format!("{e}").contains("not a procedure"), "{e}");
    assert_eq!(eval(&mut vm, "(u 1 2)"), "3");
}
