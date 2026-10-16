//! The soak, bounded so it can be a gate.
//!
//! The long run is `cargo run --release -p scm-conform --bin soak --
//! --minutes 60`, and it is the one that would catch a slow fragmentation
//! trend. This is the short version: enough rounds that a leak of even one
//! cell per round is unmistakable, few enough that it belongs in
//! `cargo test`.
//!
//! What it adds over `scm/tests/gc.rs` is **duration rather than
//! intensity**. Those tests collect at every safepoint, which finds a
//! collector bug at once; this watches whether the live set and the heap's
//! high-water mark hold still over hundreds of rounds of ordinary work.

use scm_conform::soak::Soak;

#[test]
fn sustained_allocation_reaches_a_steady_state() {
    let mut soak = Soak::new();
    soak.run(600).expect("the soak workload itself must run");
    let v = soak.verdict();

    // A leak of one cell per round would be ~600 cells over this run,
    // against a live set in the low thousands -- well outside 1.05x.
    assert!(
        v.live_growth() <= 1.05,
        "live objects grew {:.3}x: something is retained\n{v}",
        v.live_growth()
    );
    // Growing here while live holds still is what fragmentation looks like
    // in a non-moving collector.
    assert!(
        v.slot_growth() <= 1.05,
        "the slot high-water grew {:.3}x: fragmentation\n{v}",
        v.slot_growth()
    );
}

/// The workload has to be worth watching: a round that allocates nothing
/// would hold perfectly steady and prove nothing at all.
#[test]
fn the_soak_workload_actually_allocates() {
    let mut soak = Soak::new();
    soak.run(40).expect("the soak workload itself must run");
    let v = soak.verdict();
    assert!(
        v.collections >= 20,
        "only {} collections in 40 rounds: the workload is too light to be a soak",
        v.collections
    );
    assert!(
        v.early_live > 500,
        "a live set of {} is too small to show a trend in",
        v.early_live
    );
}
