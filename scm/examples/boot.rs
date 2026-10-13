//! What does `Vm::new` cost now that the prelude is Scheme?
//!
//! Plan §7 budgets < 10 ms for a whole process at S10, and the way it gets
//! there is a build-time image rather than compiling the prelude at every
//! start. This measures the thing that image would replace -- the number to
//! beat, measured rather than assumed.

use std::time::Instant;

use scm::Vm;

fn main() {
    // Warm the allocator and any lazy statics.
    let _ = Vm::new();
    let n = 200;
    let t0 = Instant::now();
    for _ in 0..n {
        let vm = Vm::new();
        std::hint::black_box(vm.engine.obj_count());
    }
    let per = t0.elapsed().as_secs_f64() / n as f64;
    let vm = Vm::new();
    println!("Vm::new               {:8.3} ms", per * 1e3);
    println!("  live objects after  {}", vm.engine.obj_count());
    println!("  globals defined     {}", vm.global_count());
    println!();
    println!("Plan §7 budgets < 10 ms for a whole process at S10, reached by");
    println!("deserializing a build-time image instead of compiling the");
    println!("prelude at every start. This is the cost that image replaces.");
}
