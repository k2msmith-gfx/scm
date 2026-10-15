//! Is per-texel noise viable, or does it still have to be baked?
//!
//! `orange.janet` computes its noise fields "ONCE into small tables rather
//! than hashed per texel", because "fbm is four octaves of four hashes --
//! sixteen math/sin per call". This measures the same shape of work both
//! ways, so the answer is a number rather than a hope.
use std::time::Instant;
use scm::{Value, Vm};
use scm_llvm::{Shape, compile};

const KERNEL: &str = r#"
(import (scm noise))
;; the corpus's own texture shape: four octaves of fbm per texel
(define (texel u v) (perlin-2-fbm (* u 8.0) (* v 8.0) 4))
;; and a whole 256x256 sheet of it, summed so nothing is optimised away
(define (sheet n)
  (let loop ((i 0) (acc 0.0))
    (if (= i (* n n))
        acc
        (let ((u (/ (exact->inexact (modulo i n)) (exact->inexact n)))
              (v (/ (exact->inexact (quotient i n)) (exact->inexact n))))
          (loop (+ i 1) (+ acc (texel u v)))))))
"#;

fn main() {
    let mut vm = Vm::new();
    vm.eval_str(KERNEL).expect("kernel");
    let sheet = compile(&mut vm, "sheet", Shape::Scalar).expect("sheet should compile");
    let texel = compile(&mut vm, "texel", Shape::Scalar).expect("texel should compile");

    const N: usize = 256;
    let texels = (N * N) as f64;

    let t = Instant::now();
    let native = sheet.call(&[N as f64]).unwrap();
    let native_s = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let v = vm.eval_str(&format!("(sheet {N})")).expect("interpreted");
    let interp_s = t.elapsed().as_secs_f64();
    let interp = match v { Value::Flonum(x) => x, _ => panic!() };

    println!("a {N}x{N} sheet, four octaves of fbm per texel");
    println!("  native       {:9.1} ms   {:8.1} ns/texel", native_s * 1e3, native_s / texels * 1e9);
    println!("  interpreted  {:9.1} ms   {:8.1} ns/texel", interp_s * 1e3, interp_s / texels * 1e9);
    println!("  speedup      {:9.0}x", interp_s / native_s);
    println!("  agree        {}", (native - interp).abs() < 1e-6 * interp.abs().max(1.0));

    // and the cost of one texel called from outside, which is what a Rust
    // renderer would actually do per sample
    let t = Instant::now();
    let reps = 1_000_000;
    let mut sink = 0.0;
    for i in 0..reps {
        sink += texel.call(&[(i % 997) as f64 / 997.0, (i % 991) as f64 / 991.0]).unwrap();
    }
    let per = t.elapsed().as_secs_f64() / reps as f64;
    println!("  one texel called from Rust: {:.1} ns  (sink {sink:.3})", per * 1e9);
}
