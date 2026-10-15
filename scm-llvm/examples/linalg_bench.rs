//! How much does writing a kernel against the component layer buy?
use std::time::Instant;
use scm::{Value, Vm, write_str};
use scm_llvm::{Shape, compile};

// A kernel that does real linear algebra work in a loop: accumulate the
// squared distance from a moving point to a fixed one, N times.
const KERNEL: &str = r#"
(import (scm linalg))
(define (work n px py pz)
  (let loop ((i 0) (acc 0.0))
    (if (= i n)
        acc
        (let* ((t (* 0.001 (exact->inexact i)))
               (qx (+ px t)) (qy (+ py (* 2.0 t))) (qz (+ pz (* 3.0 t))))
          (loop (+ i 1)
                (+ acc (v3-distance-squared* px py pz qx qy qz)
                   (v3-dot* qx qy qz px py pz)))))))
"#;

fn main() {
    let mut vm = Vm::new();
    vm.eval_str(KERNEL).expect("kernel");
    let f = compile(&mut vm, "work", Shape::Scalar).expect("work should compile");

    const N: f64 = 200000.0;
    let t = Instant::now();
    let native = f.call(&[N, 1.0, 2.0, 3.0]).unwrap();
    let native_ms = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let v = vm.eval_str(&format!("(work {N} 1.0 2.0 3.0)")).expect("interpreted");
    let interp_ms = t.elapsed().as_secs_f64() * 1000.0;
    let interp = match v { Value::Flonum(x) => x, _ => panic!("{}", write_str(&vm.engine, v)) };

    println!("native      {native_ms:8.2} ms  -> {native}");
    println!("interpreted {interp_ms:8.2} ms  -> {interp}");
    println!("agreement   {}", (native - interp).abs() < 1e-6 * interp.abs().max(1.0));
    println!("speedup     {:.0}x", interp_ms / native_ms);
}
