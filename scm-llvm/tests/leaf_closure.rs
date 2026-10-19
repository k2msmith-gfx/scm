//! S11b — a leaf compiled from a PROCEDURE VALUE, whose free variables
//! resolve through the environment it closed over.
//!
//! This is ray's texel idiom and not a hypothetical one: every
//! `make-texture` call in ray's 25 texture scenes passes an inline lambda
//! that closes over the numbers and colours of the procedure that built it,
//! and `make-texture` receives it as an argument, so there is no global to
//! compile by name. Before this stage the analysis said ``rings` is
//! unbound`.
use rayon::prelude::*;
use scm::{Value, Vm};
use scm_llvm::{Shape, compile_proc, verdict_proc};

/// `examples/procedural/wood.janet`, in the Scheme the port writes: five
/// top-level helpers, and a texel closing over two colours, a ring count
/// and a warp.
const WOOD: &str = r#"
(define (fract x) (- x (floor x)))
(define (hash2 x y)
  (fract (* 43758.5453 (sin (+ (* x 12.9898) (* y 78.233))))))
(define (fade t) (* t t (- 3.0 (* 2.0 t))))
(define (mix a b t) (+ a (* t (- b a))))
(define (vnoise x y)
  (let* ((ix (floor x)) (iy (floor y))
         (fx (fade (- x ix))) (fy (fade (- y iy))))
    (mix (mix (hash2 ix iy) (hash2 (+ ix 1.0) iy) fx)
         (mix (hash2 ix (+ iy 1.0)) (hash2 (+ ix 1.0) (+ iy 1.0)) fx)
         fy)))
(define (fbm x y)
  (let loop ((i 0) (sum 0.0) (amp 0.0) (freq 1.0) (w 0.5))
    (if (= i 5)
        (/ sum amp)
        (loop (+ i 1) (+ sum (* w (vnoise (* x freq) (* y freq))))
              (+ amp w) (* freq 2.0) (* w 0.5)))))
(define (ring u v rings warp)
  (let* ((a (- (* u 3.0) 0.4))
         (b (* v 0.35))
         (dist (+ (sqrt (+ (* a a) (* b b))) (* warp (- (fbm (* u 1.6) v) 0.5)))))
    (expt (+ 0.5 (* 0.5 (sin (+ (* dist rings 2) (* 6 (fbm (* u 2.5) (* v 1.5))))))) 2.2)))

(define (wood-texel early late rings warp)
  (lambda (u v)
    (let* ((r (ring u v rings warp))
           (fiber (* 0.06 (- (vnoise (* u 4.0) (* v 70.0)) 0.5))))
      (map (lambda (e l) (+ e (* (- l e) r) (* e fiber))) early late))))

(define oak (wood-texel (list 0.26 0.13 0.06) (list 0.13 0.055 0.022) 34.0 0.45))
(define pine (wood-texel (list 0.80 0.66 0.44) (list 0.55 0.40 0.22) 90.0 0.20))
"#;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.eval_str(WOOD).expect("the fixture loads");
    vm
}

/// The interpreted colour, component by component, so the comparison is
/// against the engine that is the oracle rather than against a transcript.
fn interpreted(vm: &mut Vm, call: &str) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (k, slot) in out.iter_mut().enumerate() {
        let v = vm
            .eval_str(&format!("(list-ref {call} {k})"))
            .expect("the interpreter runs the texel");
        *slot = match v {
            Value::Flonum(x) => x,
            Value::Fixnum(n) => n as f64,
            other => panic!("a colour component is not a number: {other:?}"),
        };
    }
    out
}

#[test]
fn rays_texel_compiles_from_the_value_and_agrees_with_the_interpreter() {
    let mut vm = vm();
    let proc = vm.eval_str("oak").unwrap();
    let tex = compile_proc(&mut vm, "oak", proc, Shape::Vec3).expect("the closure compiles");
    assert_eq!(tex.arity(), 2);

    let mut worst = 0.0f64;
    for j in 0..8 {
        for i in 0..8 {
            let (u, v) = ((i as f64 + 0.5) / 8.0, (j as f64 + 0.5) / 8.0);
            let got = tex.call3(&[u, v]).expect("a colour");
            let want = interpreted(&mut vm, &format!("(oak {u} {v})"));
            for (a, b) in [got.x, got.y, got.z].iter().zip(want.iter()) {
                worst = worst.max((a - b).abs());
            }
        }
    }
    assert_eq!(worst, 0.0, "compiled and interpreted differ by {worst:e}");
}

/// THE CAPTURED VALUES ARE THE CLOSURE'S OWN. Two texels from one procedure
/// hold different `rings` and different colours under the same names, and
/// each compiled leaf must carry its own — the bug this stage could most
/// easily have had.
#[test]
fn two_closures_from_one_procedure_do_not_share_their_captures() {
    let mut vm = vm();
    let a = vm.eval_str("oak").unwrap();
    let b = vm.eval_str("pine").unwrap();
    let oak = compile_proc(&mut vm, "oak", a, Shape::Vec3).expect("oak compiles");
    let pine = compile_proc(&mut vm, "pine", b, Shape::Vec3).expect("pine compiles");

    for (label, leaf, call) in
        [("oak", &oak, "(oak 0.3 0.7)"), ("pine", &pine, "(pine 0.3 0.7)")]
    {
        let got = leaf.call3(&[0.3, 0.7]).unwrap();
        let want = interpreted(&mut vm, call);
        assert_eq!([got.x, got.y, got.z], want, "{label} took the wrong captures");
    }
    // And they really are different boards, or the test above proves nothing.
    assert_ne!(
        oak.call3(&[0.3, 0.7]).unwrap(),
        pine.call3(&[0.3, 0.7]).unwrap()
    );
}

/// A captured TABLE is the same snapshot a captured global is, read with
/// `vector-ref`, with the bounds check item (2) already had.
#[test]
fn a_captured_table_is_snapshotted_like_a_global_one() {
    let mut vm = Vm::new();
    vm.eval_str(
        r#"
        (define (stripes palette n)
          (lambda (u v)
            (let ((i (modulo (exact (floor (* u n))) 4)))
              (list (vector-ref palette i) (* 0.5 v) (vector-ref palette i)))))
        (define tex (stripes (vector 0.1 0.4 0.7 1.0) 8))
        "#,
    )
    .unwrap();
    let proc = vm.eval_str("tex").unwrap();
    let leaf = compile_proc(&mut vm, "stripes", proc, Shape::Vec3).expect("compiles");
    for i in 0..8 {
        let u = (i as f64 + 0.5) / 8.0;
        let got = leaf.call3(&[u, 0.25]).unwrap();
        let want = interpreted(&mut vm, &format!("(tex {u} 0.25)"));
        assert_eq!([got.x, got.y, got.z], want, "at u={u}");
    }
}

/// A refusal says what the closure caught, because "unbound" would be a lie
/// when the variable is right there holding a string.
#[test]
fn a_captured_value_a_leaf_cannot_read_says_what_it_is() {
    let mut vm = Vm::new();
    vm.eval_str(
        r#"
        (define (labelled tag) (lambda (u v) (* u v tag)))
        (define tex (labelled "oak"))
        "#,
    )
    .unwrap();
    let proc = vm.eval_str("tex").unwrap();
    let why = verdict_proc(&mut vm, "labelled", proc).expect_err("a string is not readable");
    let msg = why.to_string();
    assert!(msg.contains("`tag`"), "{msg}");
    assert!(msg.contains("captured from a closure"), "{msg}");
    assert!(msg.contains("oak"), "the refusal should print what it holds: {msg}");
}

/// The corpus's colour blend is `(map f early late)` — two sequences — and
/// unrolling it needs both to have a known length. Different lengths are a
/// refusal that says the lengths.
#[test]
fn a_map_over_two_sequences_of_different_lengths_is_refused() {
    let mut vm = Vm::new();
    vm.eval_str(
        r#"
        (define (blend a b)
          (lambda (u v) (map (lambda (x y) (+ (* u x) (* v y))) a b)))
        (define tex (blend (list 1.0 2.0 3.0) (list 1.0 2.0)))
        "#,
    )
    .unwrap();
    let proc = vm.eval_str("tex").unwrap();
    let why = verdict_proc(&mut vm, "blend", proc).expect_err("the lengths differ");
    assert!(why.to_string().contains("different lengths"), "{why}");
}

/// The property a render pass needs: the compiled closure is callable from
/// every worker at once and gives one answer.
#[test]
fn a_compiled_closure_runs_from_a_rayon_pool() {
    let mut vm = vm();
    let proc = vm.eval_str("oak").unwrap();
    let tex = compile_proc(&mut vm, "oak", proc, Shape::Vec3).expect("compiles");
    const N: usize = 128;
    let serial: Vec<f64> = (0..N * N)
        .map(|i| tex.call3(&[(i % N) as f64 / N as f64, (i / N) as f64 / N as f64]).unwrap().x)
        .collect();
    let parallel: Vec<f64> = (0..N * N)
        .into_par_iter()
        .map(|i| tex.call3(&[(i % N) as f64 / N as f64, (i / N) as f64 / N as f64]).unwrap().x)
        .collect();
    assert_eq!(serial, parallel);
}
