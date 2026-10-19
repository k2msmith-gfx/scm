//! S11b's number: ray's own `wood.janet` texel, which is a CLOSURE over the
//! board's colours and ring count, compiled from the procedure value the way
//! `make-texture` will hand it over.
//!
//! The comparison here is native against interpreted for the SAME texel.
//! Janet's 4,112 ns/texel from leaf-survey is a DIFFERENT texture and is not
//! a ratio anyone may take from this file; the Janet number for this texel is
//! measured on ray's side, where both can run the same scene.
use std::time::Instant;

use scm_llvm::{Shape, compile_proc};

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

(define tex (wood-texel (list 0.26 0.13 0.06) (list 0.13 0.055 0.022) 34.0 0.45))
"#;

fn main() {
    let mut vm = scm::Vm::new();
    vm.eval_str(WOOD).unwrap();
    let proc = vm.eval_str("tex").unwrap();

    let t = Instant::now();
    let tex = compile_proc(&mut vm, "wood", proc, Shape::Vec3).expect("the closure compiles");
    println!("compile              {:8.2} ms", t.elapsed().as_secs_f64() * 1e3);

    const N: usize = 512;
    let t = Instant::now();
    let mut sink = 0.0;
    for i in 0..N * N {
        let c = tex
            .call3(&[(i % N) as f64 / N as f64, (i / N) as f64 / N as f64])
            .unwrap();
        sink += c.x + c.y + c.z;
    }
    let native = t.elapsed().as_nanos() as f64 / (N * N) as f64;
    println!("native               {native:8.1} ns/texel   ({N}x{N})");

    let m = 24;
    let t = Instant::now();
    vm.eval_str(&format!(
        "(let loop ((i 0) (acc 0.0)) (if (= i {m}) acc (loop (+ i 1) \
         (let inner ((j 0) (a acc)) (if (= j {m}) a (inner (+ j 1) \
         (+ a (apply + (tex (/ i {m}.0) (/ j {m}.0)))))))))) "
    ))
    .unwrap();
    let interp = t.elapsed().as_nanos() as f64 / (m * m) as f64;
    println!(
        "interpreted ({:?})   {interp:8.1} ns/texel   ({:.0}x)",
        vm.tier(),
        interp / native
    );
    // A 512² texture, which is what a plank or a table top asks for.
    println!(
        "\none 512x512 texture: {:6.1} ms native, {:6.1} ms interpreted",
        native * 512.0 * 512.0 / 1e6,
        interp * 512.0 * 512.0 / 1e6
    );
    std::hint::black_box(sink);
}
