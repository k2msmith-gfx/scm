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

/// The generator has to actually generate libraries, and the programs have
/// to actually *use* what they import.
///
/// This test exists because the first version of the library generator
/// failed exactly here and silently: the libraries were emitted and
/// imported, but the only path that called an imported procedure was the
/// generic one-in-fifty `call` arm picking a name out of all of them, so
/// across forty programs a library procedure was called about twice. The
/// work looked done and was testing almost nothing. Coverage of a fuzzer's
/// own output is worth asserting, because nothing else will notice it
/// drifting back to zero.
#[test]
fn the_generator_exercises_the_library_system() {
    let mut with_library = 0;
    let mut uses = 0;
    let mut modifiers = std::collections::BTreeSet::new();
    for index in 0..120u64 {
        let mut g = scm_conform::fuzz::Gen::new(90u64.wrapping_mul(1_000_003).wrapping_add(index));
        let p = g.program();
        if !p.contains("(define-library") {
            continue;
        }
        with_library += 1;
        for m in ["(import (prefix", "(import (only", "(rename (except", "(rename reverse",
                  "(rename length", "(rename abs"] {
            if p.contains(m) {
                modifiers.insert(m);
            }
        }
        // The body after `(define result` is where a program says what it
        // computed; a library name appearing only in its own definition
        // proves nothing.
        let body = p.split("(define result").nth(1).unwrap_or("");
        if body.contains("lf") || body.contains("lg") || body.contains("lmp")
            || body.contains("lm") || body.contains("lset") || body.contains("lv")
            || body.contains("px") || body.contains("rn")
        {
            uses += 1;
        }
    }
    assert!(with_library >= 40, "only {with_library} of 120 programs defined a library");
    assert!(uses >= 15, "only {uses} of {with_library} library programs used an import");
    assert!(modifiers.len() >= 3, "import modifiers barely covered: {modifiers:?}");
}
