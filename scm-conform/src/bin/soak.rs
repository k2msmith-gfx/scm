//! `scm-soak` -- sustained allocation, for as long as you ask.
//!
//! The suite's GC tests collect at every safepoint, which finds a collector
//! bug immediately. This finds the other kind: a few cells retained per
//! round, or a non-moving heap fragmenting, neither of which is visible
//! except over time.
//!
//!     cargo run --release -p scm-conform --bin scm-soak -- --rounds 20000
//!     cargo run --release -p scm-conform --bin scm-soak -- --minutes 60
//!
//! It prints a sample line periodically so a long run shows its shape, and
//! exits non-zero if either number trended up.

use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rounds: Option<usize> = None;
    let mut minutes: Option<u64> = None;
    let mut every = 500usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--rounds" => {
                rounds = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "--minutes" => {
                minutes = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "--every" => {
                every = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(500);
                i += 2;
            }
            other => {
                eprintln!("scm-soak: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let deadline = minutes.map(|m| Instant::now() + Duration::from_secs(m * 60));
    let target = rounds.unwrap_or(if deadline.is_some() { usize::MAX } else { 5_000 });

    let mut soak = scm_conform::soak::Soak::new();
    let start = Instant::now();
    let mut done = 0usize;

    while done < target {
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            break;
        }
        let batch = every.min(target - done);
        if let Err((round, e)) = soak.run(batch) {
            eprintln!("scm-soak: round {} failed: {e}", done + round);
            std::process::exit(1);
        }
        done += batch;
        let s = soak.samples.last().expect("a sample per round");
        println!(
            "{:>8} rounds  {:>6.0}s  live {:>7}  slots {:>8}  collections {:>7}",
            done,
            start.elapsed().as_secs_f64(),
            s.live,
            s.slots,
            s.collections
        );
    }

    let v = soak.verdict();
    println!("\n{v}");

    // Generous bounds: this is looking for a trend, not for jitter. A real
    // leak of even one cell per round shows as a ratio far above these over
    // thousands of rounds.
    let mut bad = false;
    if v.live_growth() > 1.10 {
        println!("FAIL: live objects grew {:.3}x -- something is retained", v.live_growth());
        bad = true;
    }
    if v.slot_growth() > 1.10 {
        println!(
            "FAIL: the slot high-water grew {:.3}x while live grew {:.3}x -- fragmentation",
            v.slot_growth(),
            v.live_growth()
        );
        bad = true;
    }
    if bad {
        std::process::exit(1);
    }
    println!("steady: neither live objects nor the heap high-water trended up");
}
