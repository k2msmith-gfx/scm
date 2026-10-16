//! S13's exit criterion (plan §9): T1/T2 agree on everything; redefining
//! a global at the REPL takes effect in compiled callers.

use scm::{Value, Vm};
use scm_llvm::install_t2;

fn t2() -> Vm {
    let mut vm = Vm::new();
    install_t2(&mut vm);
    // Compile everything on its first run: the point here is agreement,
    // not amortisation.
    vm.set_native_threshold(1);
    vm
}

fn show(vm: &mut Vm, src: &str) -> String {
    match vm.eval_str(src) {
        Ok(v) => scm::write_str(&vm.engine, v),
        Err(e) => format!("error: {e}"),
    }
}

/// Every program here runs hot enough to be compiled, and the result is
/// compared with T1's.
const PROGRAMS: &[&str] = &[
    "(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))) (fib 22)",
    "(define (tak x y z) (if (not (< y x)) z (tak (tak (- x 1) y z) (tak (- y 1) z x) (tak (- z 1) x y)))) (tak 18 12 6)",
    "(let loop ([i 0] [acc '()]) (if (= i 200) (length acc) (loop (+ i 1) (cons (* i i) acc))))",
    "(define (sum-to n) (let loop ([i 0] [s 0]) (if (> i n) s (loop (+ i 1) (+ s i))))) (sum-to 100000)",
    "(define (mk k) (lambda (x) (+ x k))) (let loop ([i 0] [s 0]) (if (= i 300) s (loop (+ i 1) (+ s ((mk i) 1)))))",
    "(define v (make-vector 100 0)) (let loop ([i 0]) (when (< i 100) (vector-set! v i (* 2 i)) (loop (+ i 1)))) (let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (vector-ref v i)))))",
    "(let loop ([i 0] [acc 0]) (if (= i 100) acc (loop (+ i 1) (+ acc (guard (e (#t 1)) (if (odd? i) (raise 'x) 0))))))",
    "(define p (make-parameter 1)) (let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (parameterize ([p i]) (p))))))",
    "(let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (call/cc (lambda (k) (if (even? i) (k 10) 1)))))))",
    "(define (f x) (define y (* x 2)) (define (g z) (+ y z)) (g x)) (let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (f i)))))",
    "(let loop ([i 0] [s 0.0]) (if (= i 100) s (loop (+ i 1) (+ s (* 1.5 i)))))",
    "(let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (apply + (list i i))))))",
    "(let loop ([i 0] [v '()]) (if (= i 60) (length v) (loop (+ i 1) (cons [i (+ i 1)] v))))",
    "(define (count-str s) (let loop ([i 0] [n 0]) (if (= i (string-length s)) n (loop (+ i 1) (if (char=? (string-ref s i) #\\a) (+ n 1) n))))) (let loop ([i 0] [t 0]) (if (= i 100) t (loop (+ i 1) (+ t (count-str \"banana\")))))",
    "(let loop ([i 0] [x 0]) (if (= i 100) x (loop (+ i 1) (begin (set! x (+ x i)) x))))",
    "(define-record-type point (make-point x y) point? (x point-x) (y point-y)) (let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (point-x (make-point i 0))))))",
    "(let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (let-values ([(a b) (values i 1)]) (+ a b))))))",
    "(define (deep n) (if (= n 0) 0 (+ 1 (deep (- n 1))))) (deep 3000)",
    "(let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (+ s (string->number (number->string i))))))",
    "(let loop ([i 1] [s 0]) (if (> i 60) s (loop (+ i 1) (+ s (expt 2 (modulo i 50))))))",
];

#[test]
fn t2_agrees_with_t1_on_hot_programs() {
    let mut native_units = 0;
    for src in PROGRAMS {
        let mut a = Vm::new();
        let mut b = t2();
        let want = show(&mut a, src);
        let got = show(&mut b, src);
        assert_eq!(got, want, "{src}");
        native_units += b.native_units();
    }
    assert!(native_units >= PROGRAMS.len() as u64, "T2 compiled {native_units} units over {} programs", PROGRAMS.len());
}

#[test]
fn errors_in_native_code_are_the_same_errors_with_the_same_backtrace() {
    let mut a = Vm::new();
    let mut b = t2();
    let src = "(define (inner v i) (+ 0 (vector-ref v i)))\n\
               (define (middle v i) (* 1 (inner v i)))\n\
               (define (outer i) (- 0 (middle (vector 1 2 3) i)))\n\
               (let loop ([i 0] [s 0]) (if (= i 200) s (loop (+ i 1) (+ s (outer (modulo i 3))))))\n\
               (outer 99)";
    let ea = show(&mut a, src);
    let eb = show(&mut b, src);
    assert_eq!(ea, eb);
    assert!(eb.contains("out of range"), "{eb}");
    assert!(b.native_units() > 0);
    let ba: Vec<String> = a.backtrace().iter().map(|f| f.what.clone()).collect();
    let bb: Vec<String> = b.backtrace().iter().map(|f| f.what.clone()).collect();
    assert_eq!(ba, bb);
    // Fixnum overflow leaves the native fast path and **widens** in T1's
    // primitive (S18), rather than raising as it used to. The property
    // that matters is unchanged and is the reason this assertion is here:
    // whatever the fast path bails out to, the three tiers agree. The
    // leaf compiler is `f64` throughout and T2's fast path is fixnum
    // only, so an exact 2^70 can only come from the primitive.
    let big = "(define (dbl x) (* x 2)) (let loop ([i 0] [x 1]) (if (= i 70) x (loop (+ i 1) (dbl x))))";
    assert_eq!(show(&mut t2(), big), show(&mut Vm::new(), big));
    assert_eq!(show(&mut t2(), big), "1180591620717411303424");
    // And it is exact, not a flonum that prints without a dot.
    let exact = "(define (dbl x) (* x 2)) (exact? (let loop ([i 0] [x 1]) (if (= i 70) x (loop (+ i 1) (dbl x)))))";
    assert_eq!(show(&mut t2(), exact), "#t");
}

#[test]
fn redefining_a_global_takes_effect_in_compiled_callers() {
    let mut vm = t2();
    let src = "(define (f a b) (+ a b)) (let loop ([i 0] [s 0]) (if (= i 100) s (loop (+ i 1) (f s i))))";
    assert_eq!(show(&mut vm, src), "4950");
    assert!(vm.native_units() > 0);
    // `+` itself, which the fast path guards on.
    vm.eval_str("(define (+ a b) (- a b))").unwrap();
    assert_eq!(show(&mut vm, "(f 10 3)"), "7");
    // And a user function: the cell is read on every call.
    vm.eval_str("(define (f a b) (* a b))").unwrap();
    assert_eq!(show(&mut vm, "(f 10 3)"), "30");
}

#[test]
fn a_tail_loop_in_native_code_keeps_the_stack_flat_and_is_interruptible() {
    let mut vm = t2();
    vm.eval_str("(define (loop i) (if (< i 200000) (loop (+ i 1)) i))").unwrap();
    let v = vm.eval_str("(loop 0)").unwrap();
    assert_eq!(v, Value::Fixnum(200000));
    assert!(vm.native_units() > 0);
    assert!(vm.stack_high_water() < 16, "{}", vm.stack_high_water());
    // Fuel counts safepoints, which native code takes at calls and loop
    // back-edges rather than per op, so a budget lasts longer under T2
    // than T1. It still runs out.
    vm.set_fuel(Some(100));
    let r = vm.eval_str("(loop 0)");
    assert!(matches!(r, Err(scm::VmError::OutOfFuel)), "{r:?}");
}
