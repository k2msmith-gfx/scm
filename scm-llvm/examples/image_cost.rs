//! What does a texel function that *samples an image* cost?
//!
//! The S11 number was a procedural texture: arithmetic only, no data. This
//! is the other half of ray's texture work -- a bilinear tap into a loaded
//! image -- which is what `(srfi 4)` and `(srfi 63)` plus the leaf
//! compiler's snapshot of them are for.
use std::time::Instant;

use scm_llvm::{Shape, compile};

const SRC: &str = r#"
(import (srfi 4) (srfi 63))
(define w 512)
(define img (make-array a:flor64b 512 512))
(let loop ((i 0))
  (when (< i 512)
    (let inner ((j 0))
      (when (< j 512)
        (array-set! img (/ (+ (* i 512.0) j) 262144.0) i j)
        (inner (+ j 1))))
    (loop (+ i 1))))

(define (bilinear y x)
  (let* ((y0 (exact (floor y))) (x0 (exact (floor x)))
         (y1 (modulo (+ y0 1) 512)) (x1 (modulo (+ x0 1) 512))
         (fy (- y y0)) (fx (- x x0))
         (a (array-ref img y0 x0)) (b (array-ref img y0 x1))
         (c (array-ref img y1 x0)) (d (array-ref img y1 x1)))
    (+ (* (- 1.0 fy) (+ (* (- 1.0 fx) a) (* fx b)))
       (* fy (+ (* (- 1.0 fx) c) (* fx d))))))
"#;

fn main() {
    let mut vm = scm::Vm::new();
    vm.eval_str(SRC).unwrap();
    let t = Instant::now();
    let f = compile(&mut vm, "bilinear", Shape::Scalar).unwrap();
    println!("compile              {:8.2} ms", t.elapsed().as_secs_f64() * 1e3);

    const N: usize = 512;
    let t = Instant::now();
    let mut sink = 0.0;
    for i in 0..N * N {
        sink += f.call(&[(i / N) as f64 + 0.3, (i % N) as f64 + 0.7]).unwrap();
    }
    let native = t.elapsed().as_nanos() as f64 / (N * N) as f64;
    println!("native               {native:8.1} ns/tap");

    let m = 48;
    let t = Instant::now();
    vm.eval_str(&format!(
        "(let loop ((i 0) (acc 0.0))
           (if (= i {m}) acc
             (loop (+ i 1)
               (let inner ((j 0) (a acc))
                 (if (= j {m}) a
                   (inner (+ j 1) (+ a (bilinear (+ i 0.3) (+ j 0.7)))))))))"
    ))
    .unwrap();
    let interp = t.elapsed().as_nanos() as f64 / (m * m) as f64;
    println!("interpreted (T1)     {interp:8.1} ns/tap   ({:.0}x)", interp / native);
    std::hint::black_box(sink);
}
