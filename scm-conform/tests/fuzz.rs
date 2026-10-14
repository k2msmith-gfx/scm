//! The differential fuzzer as a gate: a fixed seed, so a failure is a
//! program to replay, not a coincidence.

use scm_conform::fuzz::{Verdict, compare};

fn report(mismatches: &[scm_conform::fuzz::Mismatch]) -> String {
    let mut s = String::new();
    for m in mismatches.iter().take(3) {
        s.push_str(&format!("\n--- {}-{} ---\n{}\n", m.seed, m.index, m.program));
        for (tier, v) in &m.tiers {
            s.push_str(&format!("{tier}: {v:?}\n"));
        }
    }
    s
}

#[test]
fn t0_and_t1_agree_on_random_programs() {
    let (mismatches, skipped) = compare(7, 120, false, None);
    assert!(mismatches.is_empty(), "{} mismatches: {}", mismatches.len(), report(&mismatches));
    assert!(skipped < 12, "{skipped} of 120 programs ran out of fuel; the generator is making runaways");
}

#[test]
fn t1_agrees_under_gc_stress() {
    // Every safepoint collects: a value held only in a Rust local shows up
    // here as a wrong answer or a panic.
    let (mismatches, _) = compare(11, 30, true, None);
    assert!(mismatches.is_empty(), "{}", report(&mismatches));
}

#[test]
fn programs_mostly_run_to_a_value() {
    // The generator's job is to produce programs that *mean* something: if
    // most of them error, the comparison is of error messages only.
    let mut values = 0;
    for index in 0..100u64 {
        let mut g = scm_conform::fuzz::Gen::new(3u64.wrapping_mul(1_000_003).wrapping_add(index));
        let p = g.program();
        if matches!(scm_conform::fuzz::run(&p, scm::Tier::T1, false, None), Verdict::Value { .. }) {
            values += 1;
        }
    }
    assert!(values >= 80, "only {values} of 100 programs produced a value");
}
