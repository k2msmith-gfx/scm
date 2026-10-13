//! Requirement 2, as a program: embed scm in a Rust application.
//!
//! The body below is the whole of it. S7's exit criterion is "under 30
//! lines", and what that is really testing is that a host needs no knowledge
//! of the heap, the collector or the control stack to drive the language.
//!
//!     cargo run -p scm --example host

use scm::{Interp, Val};

fn main() -> Result<(), scm::EmbedError> {
    let mut scm = Interp::new()?;

    // A host function that captures state -- here a tally, in ray's case the
    // scene store.
    let count = std::cell::Cell::new(0);
    scm.define_fn("add-box", move |args| {
        count.set(count.get() + 1);
        println!("  verb #{} got {args:?}", count.get());
        Ok(Val::Int(count.get()))
    })?;

    // Requirement 8: a runaway script is the host's problem to bound, not to
    // survive by luck. Fuel counts SAFEPOINTS, which fall every 1024 steps --
    // so this is a budget in the millions of steps, not thousands.
    scm.set_fuel(Some(200_000));
    scm.set_heap_cap(Some(1_000_000));

    let handles = scm.eval(
        r#"(let loop ([i 0] [acc '()])
             (if (= i 3) (reverse acc)
                 (loop (+ i 1) (cons (add-box :at [i 0 0] :size 2) acc))))"#,
    )?;
    println!("scene handles: {}", scm.write(&handles)?);

    match scm.eval("(let spin () (spin))") {
        Err(e) => println!("runaway stopped: {e}"),
        Ok(_) => unreachable!("an infinite loop cannot return"),
    }
    Ok(())
}
