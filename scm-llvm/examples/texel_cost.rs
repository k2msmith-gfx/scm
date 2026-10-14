//! The number plan §3.2 is about: nanoseconds per texel, compiled against
//! T0, on ray's real texture stack.
use std::time::Instant;
use scm_llvm::{Shape, compile};

const SRC: &str = r#"
(define (fract x) (- x (floor x)))
(define (hash2 x y) (fract (* 43758.5453 (sin (+ (* x 12.9898) (* y 78.233))))))
(define (fade t) (* t t (- 3.0 (* 2.0 t))))
(define (mix a b t) (+ a (* t (- b a))))
(define (vnoise x y)
  (let* ([xi (floor x)] [yi (floor y)] [xf (- x xi)] [yf (- y yi)] [u (fade xf)] [v (fade yf)])
    (mix (mix (hash2 xi yi) (hash2 (+ xi 1.0) yi) u)
         (mix (hash2 xi (+ yi 1.0)) (hash2 (+ xi 1.0) (+ yi 1.0)) u) v)))
(define (fbm x y octaves)
  (let loop ([i 0] [amp 0.5] [freq 1.0] [sum 0.0])
    (if (= i octaves) sum
        (loop (+ i 1) (* amp 0.5) (* freq 2.0) (+ sum (* amp (vnoise (* x freq) (* y freq))))))))
(define (texel u v)
  (let ([n (fbm (* u 8.0) (* v 8.0) 4)])
    (map (lambda (c) (mix 0.1 c n)) (list 0.8 0.5 0.2))))
"#;

fn main() {
    let mut vm = scm::Vm::new();
    vm.eval_str(SRC).unwrap();
    let t = Instant::now();
    let texel = compile(&mut vm, "texel", Shape::Vec3).unwrap();
    println!("compile              {:8.2} ms", t.elapsed().as_secs_f64() * 1e3);
    const N: usize = 256;
    let t = Instant::now();
    let mut sink = 0.0;
    for i in 0..N * N {
        let c = texel.call3(&[(i % N) as f64 / N as f64, (i / N) as f64 / N as f64]).unwrap();
        sink += c.x + c.y + c.z;
    }
    let native = t.elapsed().as_nanos() as f64 / (N * N) as f64;
    println!("native               {native:8.1} ns/texel");
    let m = 32;
    let t = Instant::now();
    vm.eval_str(&format!(
        "(let loop ([i 0] [acc 0.0]) (if (= i {m}) acc (loop (+ i 1) (let inner ([j 0] [a acc]) (if (= j {m}) a (inner (+ j 1) (+ a (apply + (texel (/ i {m}.0) (/ j {m}.0))))))))))"
    )).unwrap();
    let interp = t.elapsed().as_nanos() as f64 / (m * m) as f64;
    println!("interpreted ({:?})   {interp:8.1} ns/texel   ({:.0}x)", vm.tier(), interp / native);
    std::hint::black_box(sink);
}
