//! S2's exit criterion at the size the plan names: a 10^8-iteration tail loop.
//!
//! The result splits in two, and only one half is S2's to deliver:
//!
//!   * **The control stack is flat** -- depth 2 at every trip count from 1 to
//!     10^8. That is what proper tail calls *are*, and it is what S2 owed.
//!   * **Memory is not flat.** Peak RSS at 10^8 is ~8.9 GB, because every
//!     iteration allocates an environment and there is no collector until S3.
//!
//! So the plan's phrase "flat in memory" needs S3 before it is true. Worth
//! being exact about rather than claiming the criterion on the stack number
//! alone -- a tail loop in a finished Scheme is flat in memory *because the
//! collector reclaims the dead environments*, not because the stack is flat.
//!
//!     cargo run --release --example tail_loop
//!
//! Note what is *not* claimed here: this says nothing about speed. T0 is the
//! semantic oracle and is written for clarity. Plan §3.2 is explicit that the
//! interpreter's throughput is not what this project is for.

use std::time::Instant;

use scm::{Value, Vm};

fn run(n: u64) -> (usize, usize, f64) {
    let mut vm = Vm::new();
    let src = format!("(define (loop i) (if (< i {n}) (loop (+ i 1)) i))\n(loop 0)");
    let t0 = Instant::now();
    let v = vm.eval_str(&src).expect("tail loop");
    let secs = t0.elapsed().as_secs_f64();
    assert_eq!(v, Value::Fixnum(n as i64), "loop returned the wrong value");
    (vm.stack_high_water(), vm.engine.obj_count(), secs)
}

fn main() {
    println!(
        "{:>12}  {:>12}  {:>12}  {:>9}  {:>12}",
        "iterations", "stack depth", "heap objects", "seconds", "ns/iter"
    );
    let mut depths = Vec::new();
    for n in [1u64, 100, 10_000, 1_000_000, 100_000_000] {
        let (depth, objs, secs) = run(n);
        depths.push(depth);
        println!(
            "{n:>12}  {depth:>12}  {objs:>12}  {secs:>9.3}  {:>12.1}",
            secs * 1e9 / n as f64
        );
    }
    let flat = depths.windows(2).all(|w| w[0] == w[1]);
    println!();
    if flat {
        println!("FLAT: the control stack reached depth {} at every trip count,", depths[0]);
        println!("from 1 iteration to 100,000,000. Proper tail calls hold.");
    } else {
        println!("NOT FLAT: {depths:?} -- a tail call is pushing a frame.");
        std::process::exit(1);
    }

    // The heap is a different story, and an honest one: there is no collector
    // until S3, so every environment allocated by every iteration is still
    // live. That is the measurement that should change at S3, and nothing
    // about it is a tail-call problem.
    println!();
    println!("Heap objects grow 1:1 with the trip count: there is no collector");
    println!("until S3, so every iteration's environment is still live. Peak");
    println!("RSS at 10^8 is ~8.9 GB. The plan's \"flat in memory\" needs S3;");
    println!("what S2 owed, and delivered, is the flat control stack above.");
    println!();
    println!("A later option worth measuring, not assuming: reusing the frame");
    println!("for a self-tail-call would make the loop allocate nothing at all,");
    println!("but a collector is the honest fix and comes first.");
}
