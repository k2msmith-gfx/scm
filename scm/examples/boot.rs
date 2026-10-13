//! What does `Vm::new` cost now that the prelude is Scheme?
//!
//! Plan §7 budgets < 10 ms for a whole process at S10, and expected to get
//! there with a build-time image rather than compiling the prelude at every
//! start. This measures the thing that image would replace. S10 ran it and
//! deferred the image: 0.37 ms is not worth an image format. The gate that
//! says when that changes is `scm-cli/tests/boot.rs`.

use std::time::Instant;

use scm::{Session, Vm};

fn time<T>(n: usize, mut f: impl FnMut() -> T) -> f64 {
    let t0 = Instant::now();
    for _ in 0..n {
        std::hint::black_box(f());
    }
    t0.elapsed().as_secs_f64() / n as f64 * 1e3
}

fn main() {
    // Warm the allocator and any lazy statics.
    let _ = Vm::new();
    let n = 200;
    let vm_new = time(n, Vm::new);
    let session_new = time(n, || Session::new().expect("session"));
    let vm = Vm::new();
    println!("Vm::new               {vm_new:8.3} ms");
    println!("Session::new          {session_new:8.3} ms   (what `scm -e` builds)");
    println!("  live objects after  {}", vm.engine.obj_count());
    println!("  globals defined     {}", vm.global_count());
    println!();
    println!("Plan §7 budgets < 10 ms for the whole process; the rest of it is");
    println!("exec and dynamic linking, which no image can reduce. Compare");
    println!("`scm --version` against `scm -e '(values)'` to see the split.");
}
