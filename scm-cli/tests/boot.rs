//! The boot budget (plan §7, S10): `scm -e '(values)'` in under 10 ms.
//!
//! This is the whole process -- `exec`, dynamic linking, `Vm::new` with the
//! prelude compiled from source, one eval, one print, exit -- because that is
//! what a `--batch` sweep pays per scene. The best of several runs is what
//! gets compared, so a loaded machine makes the test slower, not flaky: a
//! regression in the binary moves the minimum, scheduling noise does not.
//!
//! Plan §7 expected this number to be reached by deserializing a build-time
//! image. S10 measured first: compiling the prelude costs 0.37 ms, the whole
//! process 2.5 ms, 1.8 ms of which is `scm --version` -- the binary's own
//! load. The image is deferred, and this test is the thing that says when to
//! build it. See the S10 row in plan §9.

use std::process::Command;
use std::time::{Duration, Instant};

const BUDGET: Duration = Duration::from_millis(10);

fn best_of(n: usize, args: &[&str]) -> Duration {
    let exe = env!("CARGO_BIN_EXE_scm");
    let mut best = Duration::MAX;
    for _ in 0..n {
        let t0 = Instant::now();
        let out = Command::new(exe).args(args).output().expect("spawn scm");
        let took = t0.elapsed();
        assert!(out.status.success(), "scm {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
        best = best.min(took);
    }
    best
}

#[test]
fn a_whole_process_boots_under_budget() {
    let _ = best_of(2, &["--version"]); // warm the page cache
    let load = best_of(10, &["--version"]);
    let boot = best_of(10, &["-e", "(values)"]);
    eprintln!(
        "scm --version {:.2} ms, scm -e '(values)' {:.2} ms, budget {:?}",
        load.as_secs_f64() * 1e3,
        boot.as_secs_f64() * 1e3,
        BUDGET
    );
    assert!(
        boot < BUDGET,
        "boot budget blown: scm -e '(values)' took {:.2} ms (binary load alone {:.2} ms); \
         plan §7 budgets {:?}. If the prelude grew, S10's deferred image is now due.",
        boot.as_secs_f64() * 1e3,
        load.as_secs_f64() * 1e3,
        BUDGET
    );
}
