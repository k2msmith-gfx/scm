//! S11's exit criterion (plan §9): a compiled field and texel function run
//! concurrently from a rayon pool and agree with T0; the restricted subset
//! rejects a non-pure call with a message naming why.

use rayon::prelude::*;
use scm::{Value, Vm};
use scm_llvm::{Compiled, Refusal, Shape, Vec3, compile, ir, verdict};

/// ray's real texture stack in the Scheme that exists today, from the S5b
/// probe, plus a field, predicates, tables and the corpus's own spellings.
const FIXTURE: &str = r#"
(define (fract x) (- x (floor x)))
(define (hash2 x y)
  (fract (* 43758.5453 (sin (+ (* x 12.9898) (* y 78.233))))))
(define (fade t) (* t t (- 3.0 (* 2.0 t))))
(define (mix a b t) (+ a (* t (- b a))))

(define (vnoise x y)
  (let* ([xi (floor x)] [yi (floor y)]
         [xf (- x xi)]  [yf (- y yi)]
         [u (fade xf)]  [v (fade yf)])
    (mix (mix (hash2 xi yi) (hash2 (+ xi 1.0) yi) u)
         (mix (hash2 xi (+ yi 1.0)) (hash2 (+ xi 1.0) (+ yi 1.0)) u)
         v)))

(define (fbm x y octaves)
  (let loop ([i 0] [amp 0.5] [freq 1.0] [sum 0.0])
    (if (= i octaves)
        sum
        (loop (+ i 1) (* amp 0.5) (* freq 2.0)
              (+ sum (* amp (vnoise (* x freq) (* y freq))))))))

;; the colour return, in the corpus's spelling
(define (texel u v)
  (let ([n (fbm (* u 8.0) (* v 8.0) 4)])
    (map (lambda (c) (mix 0.1 c n)) (list 0.8 0.5 0.2))))

;; and in the bracket spelling, with a conditional colour
(define (checker u v)
  (let ([n (vnoise (* u 4.0) (* v 4.0))])
    (if (< n 0.5) [n (* n n) 0.1] [0.9 (- 1.0 n) n])))

;; a field: a sphere smoothly unioned with a box, min/max/abs/sqrt
(define (sd-sphere x y z r) (- (sqrt (+ (* x x) (* y y) (* z z))) r))
(define (sd-box x y z b)
  (let* ([qx (- (abs x) b)] [qy (- (abs y) b)] [qz (- (abs z) b)])
    (+ (sqrt (+ (square (max qx 0.0)) (square (max qy 0.0)) (square (max qz 0.0))))
       (min (max qx (max qy qz)) 0.0))))
(define (smin a b k)
  (let ([h (max (- k (abs (- a b))) 0.0)])
    (- (min a b) (/ (* h h) (* 4.0 k)))))
(define (field x y z)
  (smin (sd-sphere x y z 1.0) (sd-box (- x 0.5) y z 0.7) 0.3))

;; item (2): a precomputed table, read through a closure, with modulo
(define heights (list->vector (list 0.1 0.4 0.9 0.3)))
(define (h-at i) (vector-ref heights (modulo i (vector-length heights))))

;; predicates, cond, when, and, or, not, nested loops, quotient, expt
(define (classify x y)
  (cond [(and (> x 0.0) (> y 0.0)) 1.0]
        [(or (< x -1.0) (not (>= y -1.0))) 2.0]
        [(zero? (remainder (exact (floor x)) 2)) 3.0]
        [else (expt 2.0 (quotient (exact (floor y)) 3))]))
(define (grid-sum n)
  (let outer ([i 0] [acc 0.0])
    (if (>= i n) acc
        (outer (+ i 1)
               (let inner ([j 0] [a acc])
                 (if (>= j n) a
                     (inner (+ j 1) (+ a (h-at (+ i j))))))))))
(define (ridge x y)
  (let loop ([k 0] [acc 0.0])
    (if (= k 3) acc (loop (+ k 1) (+ acc (abs (- 0.5 (vnoise (* x k) y))))))))
(define (stepper x)
  (let loop ([x x] [n 0])
    (when (> n 100) (* n 2))   ; a one-armed `if` in a multi-form body
    (if (< x 1.0) n (loop (/ x 2.0) (+ n 1)))))

;; --- and these must be refused, each for a different reason ---
(define (impure-io x) (begin (display x) x))
(define *tally* 0)
(define (impure-global x) (begin (set! *tally* (+ *tally* 1)) x))
(define (impure-mutate v) (begin (vector-set! v 0 1.0) v))
(define (impure-reach x) (* 2.0 (impure-io x)))
(define (unknown-map xs) (map (lambda (c) (* c 2.0)) xs))
(define (bool-value x) (let ([big (> x 1.0)]) (if big 1.0 0.0)))
(define (non-tail-loop n) (let loop ([i n]) (if (= i 0) 0.0 (+ 1.0 (loop (- i 1))))))
(define (allocates x) (let ([v (vector x x)]) (vector-ref v 0)))
(define (variadic a . rest) a)
"#;

fn fixture() -> Vm {
    let mut vm = Vm::new();
    vm.eval_str(FIXTURE).expect("the fixture loads");
    vm
}

fn t0(vm: &mut Vm, src: &str) -> f64 {
    match vm.eval_str(src) {
        Ok(Value::Flonum(v)) => v,
        Ok(Value::Fixnum(v)) => v as f64,
        other => panic!("T0 on {src}: {other:?}"),
    }
}

fn t0_3(vm: &mut Vm, src: &str) -> Vec3 {
    let v = vm.eval_str(src).unwrap();
    let mut out = [0.0; 3];
    let mut cur = v;
    for slot in out.iter_mut() {
        let (car, cdr) = match cur {
            Value::Obj(r) => match vm.engine.obj(r) {
                scm::heap::Obj::Pair(a, d) => (*a, *d),
                scm::heap::Obj::Vector { items, .. } => {
                    for (i, s) in out.iter_mut().enumerate() {
                        *s = match items[i] {
                            Value::Flonum(x) => x,
                            Value::Fixnum(n) => n as f64,
                            other => panic!("{other:?}"),
                        };
                    }
                    return Vec3 { x: out[0], y: out[1], z: out[2] };
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        };
        *slot = match car {
            Value::Flonum(x) => x,
            Value::Fixnum(n) => n as f64,
            other => panic!("{other:?}"),
        };
        cur = cdr;
    }
    Vec3 { x: out[0], y: out[1], z: out[2] }
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[test]
fn scalar_leaves_agree_with_t0_bit_for_bit() {
    let mut vm = fixture();
    let cases: &[(&str, usize)] = &[
        ("fract", 1), ("hash2", 2), ("fade", 1), ("mix", 3), ("vnoise", 2), ("fbm", 3),
        ("sd-sphere", 4), ("sd-box", 4), ("smin", 3), ("field", 3), ("h-at", 1),
        ("classify", 2), ("grid-sum", 1), ("ridge", 2), ("stepper", 1),
    ];
    let mut samples = 0;
    for &(name, arity) in cases {
        let f = compile(&mut vm, name, Shape::Scalar).unwrap_or_else(|r| panic!("{name}: {r}"));
        assert_eq!(f.arity(), arity, "{name}");
        for i in 0..40 {
            let mut args = Vec::new();
            for k in 0..arity {
                // A spread of values: negative, fractional, integral, and
                // small integers for the loop counts and table indexes.
                let v = match (name, k) {
                    ("fbm", 2) | ("grid-sum", 0) => (i % 5) as f64,
                    ("h-at", _) => (i as f64) - 7.0,
                    ("stepper", _) => 1.0 + (i * i) as f64 * 3.7,
                    _ => -2.5 + (i * 7 + k * 3) as f64 * 0.173,
                };
                args.push(v);
            }
            let native = f.call(&args).unwrap();
            // T0's `modulo` and `vector-ref` want exact integers where the
            // compiled code holds the same value as an f64: spell those
            // arguments as integers so the comparison is of the same call.
            let spelled: Vec<String> = args
                .iter()
                .enumerate()
                .map(|(k, a)| match (name, k) {
                    ("fbm", 2) | ("grid-sum", 0) | ("h-at", 0) => format!("{}", *a as i64),
                    _ => format!("{a:?}"),
                })
                .collect();
            let src = format!("({name} {})", spelled.join(" "));
            let interp = t0(&mut vm, &src);
            assert!(same(native, interp), "{src}: native {native:?} vs T0 {interp:?}");
            samples += 1;
        }
    }
    assert!(samples >= 600);
}

#[test]
fn texel_functions_return_three_floats_and_agree_with_t0() {
    let mut vm = fixture();
    for name in ["texel", "checker"] {
        let f = compile(&mut vm, name, Shape::Vec3).unwrap_or_else(|r| panic!("{name}: {r}"));
        for i in 0..64 {
            let (u, v) = ((i % 8) as f64 / 8.0 + 0.031, (i / 8) as f64 / 8.0 + 0.017);
            let native = f.call3(&[u, v]).unwrap();
            let interp = t0_3(&mut vm, &format!("({name} {u:?} {v:?})"));
            assert!(same(native.x, interp.x) && same(native.y, interp.y) && same(native.z, interp.z),
                "{name} {u} {v}: {native:?} vs {interp:?}");
        }
    }
}

#[test]
fn compiled_code_runs_from_a_rayon_pool_and_outlives_the_vm() {
    let (texel, field) = {
        let mut vm = fixture();
        (compile(&mut vm, "texel", Shape::Vec3).unwrap(), compile(&mut vm, "field", Shape::Scalar).unwrap())
        // the Vm is dropped here: compiled code must not depend on it
    };
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    assert_send_sync(&texel);
    const N: usize = 256;
    let serial: Vec<Vec3> = (0..N * N).map(|i| texel.call3(&[(i % N) as f64 / N as f64, (i / N) as f64 / N as f64]).unwrap()).collect();
    let parallel: Vec<Vec3> = (0..N * N).into_par_iter().map(|i| texel.call3(&[(i % N) as f64 / N as f64, (i / N) as f64 / N as f64]).unwrap()).collect();
    assert_eq!(serial, parallel);
    let fields: Vec<f64> = (0..N * N)
        .into_par_iter()
        .map(|i| field.call(&[(i % N) as f64 / 64.0 - 2.0, (i / N) as f64 / 64.0 - 2.0, 0.3]).unwrap())
        .collect();
    assert!(fields.iter().any(|v| *v < 0.0) && fields.iter().any(|v| *v > 0.0), "the field has an inside and an outside");
    // And a handle cloned into another thread keeps the code alive after
    // the original is dropped.
    let clone = texel.clone();
    drop(texel);
    let h = std::thread::spawn(move || clone.call3(&[0.5, 0.5]).unwrap());
    assert_eq!(h.join().unwrap(), serial[N * (N / 2) + N / 2]);
}

#[test]
fn refusals_name_the_call_and_the_chain() {
    let mut vm = fixture();
    let refused = |vm: &mut Vm, name: &str| -> Refusal {
        match compile(vm, name, Shape::Scalar) {
            Err(r) => r,
            Ok(_) => panic!("{name} should have been refused"),
        }
    };
    let r = refused(&mut vm, "impure-io");
    assert!(r.message.contains("`display` is not a pure primitive"), "{r}");
    let r = refused(&mut vm, "impure-reach");
    assert!(r.message.contains("`display`") && r.message.contains("reached through `impure-io`"), "{r}");
    assert_eq!(r.via, vec!["impure-reach"]);
    let r = refused(&mut vm, "impure-global");
    assert!(r.message.contains("`set! *tally*` mutates a global"), "{r}");
    let r = refused(&mut vm, "impure-mutate");
    assert!(r.message.contains("`vector-set!` is not a pure primitive"), "{r}");
    let r = refused(&mut vm, "unknown-map");
    assert!(r.message.contains("`map` over a sequence of unknown length"), "{r}");
    let r = refused(&mut vm, "bool-value");
    assert!(r.message.contains("boolean"), "{r}");
    let r = refused(&mut vm, "non-tail-loop");
    assert!(r.message.contains("tail position"), "{r}");
    let r = refused(&mut vm, "allocates");
    assert!(r.message.contains("`vector` allocates"), "{r}");
    let r = refused(&mut vm, "variadic");
    assert!(r.message.contains("rest parameter"), "{r}");
    let r = refused(&mut vm, "no-such-function");
    assert!(r.message.contains("unbound"), "{r}");
    // The wrong shape is a refusal too, not a wrong answer.
    let r = match compile(&mut vm, "texel", Shape::Scalar) { Err(r) => r, Ok(_) => panic!() };
    assert!(r.message.contains("`map` produces a sequence"), "{r}");
    let r = match compile(&mut vm, "field", Shape::Vec3) { Err(r) => r, Ok(_) => panic!() };
    assert!(r.message.contains("does not produce a 3-element colour"), "{r}");
    // And the Display form carries the chain.
    assert!(format!("{}", refused(&mut vm, "impure-reach")).contains("(in impure-reach)"));
}

#[test]
fn the_verdict_alone_reports_calls_and_captures() {
    let mut vm = fixture();
    let s = verdict(&mut vm, "texel").unwrap();
    assert_eq!(s.arity, 2);
    assert_eq!(s.calls.iter().cloned().collect::<Vec<_>>(), ["fade", "fbm", "fract", "hash2", "mix", "vnoise"]);
    let s = verdict(&mut vm, "grid-sum").unwrap();
    assert_eq!(s.captured.iter().cloned().collect::<Vec<_>>(), ["heights"]);
}

#[test]
fn a_captured_table_is_a_snapshot_with_a_bounds_check() {
    let mut vm = fixture();
    let old = compile(&mut vm, "h-at", Shape::Scalar).unwrap();
    assert_eq!(old.call(&[1.0]), Some(0.4));
    vm.eval_str("(vector-set! heights 1 0.75)").unwrap();
    // The compiled code holds the table as it was: that is what "snapshot"
    // means, and S11a's recompile-on-redefine is how it catches up.
    assert_eq!(old.call(&[1.0]), Some(0.4));
    let new = compile(&mut vm, "h-at", Shape::Scalar).unwrap();
    assert_eq!(new.call(&[1.0]), Some(0.75));
    // `h-at` wraps with modulo, so index it directly through a raw reader.
    vm.eval_str("(define (raw i) (vector-ref heights i))").unwrap();
    let raw = compile(&mut vm, "raw", Shape::Scalar).unwrap();
    assert_eq!(raw.call(&[3.0]), Some(0.3));
    assert!(raw.call(&[4.0]).unwrap().is_nan(), "out of range reads NaN, never memory");
    assert!(raw.call(&[-1.0]).unwrap().is_nan());
    // A table that is not all numbers is refused with the index named.
    vm.eval_str("(define mixed (vector 1.0 'a)) (define (bad i) (vector-ref mixed i))").unwrap();
    let r = compile(&mut vm, "bad", Shape::Scalar).unwrap_err();
    assert!(r.message.contains("`mixed` holds `a` at index 1"), "{r}");
}

#[test]
fn the_ir_allocates_no_frames() {
    let mut vm = fixture();
    for name in ["vnoise", "fbm", "field", "grid-sum"] {
        let text = ir(&mut vm, name, Shape::Scalar).unwrap();
        assert_eq!(text.matches("alloca").count(), 0, "{name}:\n{text}");
    }
    let text = ir(&mut vm, "texel", Shape::Vec3).unwrap();
    assert!(text.contains("scm_leaf_texel"), "{text}");
    assert_eq!(text.matches("alloca").count(), 0, "{text}");
}

#[test]
fn a_handle_is_small_and_describes_itself() {
    let mut vm = fixture();
    let f: Compiled = compile(&mut vm, "fade", Shape::Scalar).unwrap();
    let d = format!("{f:?}");
    assert!(d.starts_with("#<leaf fade /1 Scalar @0x"), "{d}");
    assert_eq!(f.call(&[0.5, 0.5]), None, "the wrong arity is None, not UB");
    assert_eq!(f.call3(&[0.5]), None, "the wrong shape is None, not UB");
}
