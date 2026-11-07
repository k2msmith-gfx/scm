//! Internal `define`s in a leaf: a run of them opening a body is `let*`.
//!
//! ray's texels are written this way. shark45's `concrete` defines `n`,
//! `grit`, `joint` and `g` in order and returns a colour of `g`, and its
//! noise helper `h2` defines `s` the same way; until this the whole texture
//! was refused and baked interpreted. Definitions that are `letrec*` and not
//! `let*` -- an init that reaches its own name or a later one -- are still
//! refused, and so is an internal procedure.
use scm::{Value, Vm};
use scm_llvm::{Shape, compile, compile_proc, verdict, verdict_proc};

/// shark45.scm's apron texture as ray's scene writes it, less the
/// `make-texture` around the lambda.
const CONCRETE: &str = r#"
(define (h2 ix iy)
  (define s (* 43758.5453 (sin (+ (* ix 127.1) (* iy 311.7)))))
  (- s (floor s)))
(define (fade t) (* t t t (+ (* t (- (* t 6) 15)) 10)))
(define (vnoise x y)
  (let* ((ix (floor x)) (iy (floor y))
         (fx (fade (- x ix))) (fy (fade (- y iy)))
         (a (h2 ix iy)) (b (h2 (+ ix 1) iy))
         (c (h2 ix (+ iy 1))) (d (h2 (+ ix 1) (+ iy 1)))
         (t (+ a (* (- b a) fx))) (u (+ c (* (- d c) fx))))
    (+ t (* (- u t) fy))))
(define concrete
  (lambda (u v)
    (define n
      (let loop ((o 0) (n 0) (w 0.5) (f 3))
        (if (= o 5)
            n
            (loop (+ o 1) (+ n (* w (vnoise (* u f) (* v f)))) (* w 0.5) (* f 2)))))
    (define grit (* 0.05 (- (vnoise (* u 190) (* v 190)) 0.5)))
    (define joint (if (or (< (abs (- u 0.5)) 0.004)
                       (< (abs (- v 0.5)) 0.004)) -0.10 0))
    (define g (+ 0.14 (* 0.08 (- n 0.45)) grit joint))
    [g (* g 0.985) (* g 0.95)]))
"#;

fn number(v: Value) -> f64 {
    match v {
        Value::Flonum(x) => x,
        Value::Fixnum(n) => n as f64,
        other => panic!("not a number: {other:?}"),
    }
}

/// The interpreter's colour, the oracle. `[a b c]` is a vector.
fn interpreted(vm: &mut Vm, call: &str) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (k, slot) in out.iter_mut().enumerate() {
        let v = vm.eval_str(&format!("(vector-ref {call} {k})")).expect("the interpreter runs the texel");
        *slot = number(v);
    }
    out
}

#[test]
fn rays_concrete_texel_compiles_and_agrees_with_the_interpreter() {
    let mut vm = Vm::new();
    vm.eval_str(CONCRETE).expect("the fixture loads");
    let proc = vm.eval_str("concrete").unwrap();
    let tex = compile_proc(&mut vm, "concrete", proc, Shape::Vec3).unwrap_or_else(|r| panic!("{r}"));

    let mut worst = 0.0f64;
    // 0.5 is on the saw-cut joint, so the grid crosses it.
    for j in 0..9 {
        for i in 0..9 {
            let (u, v) = (i as f64 / 8.0, j as f64 / 8.0 + 0.001);
            let got = tex.call3(&[u, v]).expect("a colour");
            let want = interpreted(&mut vm, &format!("(concrete {u} {v})"));
            for (a, b) in [got.x, got.y, got.z].iter().zip(want.iter()) {
                worst = worst.max((a - b).abs());
            }
        }
    }
    assert_eq!(worst, 0.0, "compiled and interpreted differ by {worst:e}");
}

/// Definitions open a procedure body, a `let` body and a `map` lambda's
/// body, and the scalar path reads each the same way.
#[test]
fn definitions_open_every_kind_of_body() {
    let mut vm = Vm::new();
    vm.eval_str(
        r#"
        (define (f x y)
          (define a (* x 2.0))
          (define b (+ a y))
          (let ((c 1.5))
            (define d (* c b))
            (- d a)))
        (define (g x)
          (apply + (map (lambda (k) (define s (* k x)) (+ s 1.0)) (list 1.0 2.0 3.0))))
        "#,
    )
    .unwrap();
    let f = compile(&mut vm, "f", Shape::Scalar).unwrap_or_else(|r| panic!("{r}"));
    for (x, y) in [(0.0, 0.0), (1.25, -3.0), (-7.5, 0.125)] {
        let want = number(vm.eval_str(&format!("(f {x} {y})")).unwrap());
        assert_eq!(f.call(&[x, y]), Some(want), "f {x} {y}");
    }
    // Whether `apply` is in the subset is not this test's business; the
    // lambda's definitions must not be what stops it.
    if let Err(why) = verdict(&mut vm, "g") {
        assert!(!why.to_string().contains("define"), "the lambda's define was refused: {why}");
    }
}

fn refusal(src: &str, name: &str) -> String {
    let mut vm = Vm::new();
    vm.eval_str(src).unwrap();
    let proc = vm.eval_str(name).unwrap();
    verdict_proc(&mut vm, name, proc).expect_err("this must be refused").to_string()
}

/// `letrec*` is not `let*` where an init reaches its own name or a later
/// one, so those stay refusals -- by name, conservatively.
#[test]
fn letrec_shaped_definitions_are_refused_and_say_why() {
    let own = refusal("(define (p x) (define n (+ n x)) n)", "p");
    assert!(own.contains("`n` refers to itself"), "{own}");

    let later = refusal("(define (p x) (define a (* b x)) (define b 2.0) a)", "p");
    assert!(later.contains("`a` refers to `b`, defined after it"), "{later}");

    // Scope cuts both ways: `concrete`'s loop variable `n` hides the outer
    // one, but a `let` INIT is outside the `let`, so this `n` is the outer.
    let init = refusal("(define (p x) (define n (let ((m n)) (* m x))) n)", "p");
    assert!(init.contains("`n` refers to itself"), "{init}");
}

#[test]
fn an_internal_procedure_is_refused_and_says_where_it_goes() {
    let msg = refusal("(define (p x) (define (sq y) (* y y)) (sq x))", "p");
    assert!(msg.contains("`sq` is an internal procedure"), "{msg}");
    assert!(msg.contains("define it at top level"), "{msg}");
}

#[test]
fn a_definition_after_an_expression_is_refused() {
    let msg = refusal("(define (p x) (define a x) (* a 2.0) (define b a) b)", "p");
    assert!(msg.contains("`b` comes after an expression"), "{msg}");
}
