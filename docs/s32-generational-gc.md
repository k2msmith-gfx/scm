# S32 — a generational collector: sticky marks, a young log, a write barrier

2026-11-05, aarch64 macOS (Apple M3 Pro). Branch `gc-generational`, built
in a worktree so main stayed usable. The third item after S30 and S31: the
collector, for list-heavy code.

## The evidence first, because the first reading was stale

[vs-steel.md](vs-steel.md) said a third of `lists`' time was the collector
re-marking 300,000 live objects each cycle. That was measured before S14.
Sampled now, `lists` has **no collector in its profile at all** --
`run_code` 57%, `do_call` 26%, the sweep 1% -- because S14 stopped
allocating an environment per call and the threshold rule already bounds
marking to about one visit per allocation.

So a workload shaped like ray's was built instead: a scene-sized live set
(100,000 four-element lists, 500,000 objects) and a loop making points as
lists and keeping nothing (`alloc.scm`, 2M steps, ~6 conses each). There
the collector is **30%** of the profile (483 of ~1600 samples in
`collect`), and that is the case a generational scheme exists for: a large
old set re-marked every time the young garbage doubles it.

## What the collector was

Non-moving mark-sweep over a slot arena (`Vec<Slot>`, freed slots on a
free list), stop-the-world, an explicit worklist. Collect when the live
count has doubled, with a decaying threshold. Every collection marks the
whole live set from the roots and sweeps the whole arena.

## What S32 adds, on top of it rather than instead of it

- **Sticky marks.** `marks[i]` survives the collection that set it, so it
  means "old": an object that has lived through a collection. A full
  collection clears every mark first and is the collector as it always
  was.
- **A young log.** `alloc` pushes the slot index onto `young`. A minor
  collection sweeps exactly that list, not the arena.
- **A write barrier in `obj_mut`.** It is the one way to change an
  object, so it is the one place an old→young reference can be made:
  an old object about to be written is pushed onto `remembered`, once
  (`remembered_bits`). Every write counts, whether or not it stores a
  reference, because telling them apart would cost more than the scan.
- **A minor collection** marks from the roots and from the children of
  every remembered object, and stops at anything already marked; it never
  looks inside an unwritten old object. Survivors keep their marks and
  are old from then on.
- **The full collection runs on its own** when the old generation has
  doubled since the last full (`old >= full_threshold`), under stress
  every eighth collection, and after any minor collection that leaves the
  heap over its cap, so a heap one full collection would empty is never
  `OutOfMemory`. `collect_now` -- a host or a test asking -- is always
  full, because "collect now" means the cycle you just dropped.

`SCM_GC_FULL_ONLY=1` makes every collection full: the collector as it
was, in the same binary, for an A/B and for ruling the minor collector
out of a suspected bug.

The trigger for *a* collection is unchanged (used ≥ the decaying
threshold, about 2× live), so the caches `Vm::collect` drops are dropped
no more often than before.

**The invariant every mutation path must keep:** a reference from an old
object to a young one exists only if the old object has been through
`obj_mut` since the last collection. `alloc` and the sweep are the only
other writers of a slot, and neither stores a reference into an existing
object. A new `&mut Slot` path anywhere else would silently break
collection; CLAUDE.md now says so.

## Measured

`alloc.scm`, minimum of three, main (S31) against the branch:

| | main | S32 | |
|---|---|---|---|
| 2M steps, 500k live | 2.36 s | 1.72 s | **1.37×** |
| collector share of the profile | 30% | 2% | |

The micro-benchmarks that have no allocation in them (a call, a loop,
`hash2`, `fbm`, `fib`) are within ±5% in both directions, as they should
be: the barrier is two bit tests on a path those never take.

The Steel set ([vs-steel.md](vs-steel.md)), T1, main against the branch
back to back: `fib` 96 → 90 ms, every other program within ±4 ms,
**geometric mean 1.81× → 1.80× Steel, i.e. unchanged**. None of those ten
has the shape this is for -- `lists`' live set is one 100k list and its
time is calls, not collection -- so this is the right answer, and the
useful one: the barrier and the log cost nothing measurable where they do
no good.

## Against ray

A ray worktree with its `schemers` dependency pointed at this branch
(2026-11-05, ray at main): `cargo test` 1187 + 274, the oidn+particles leg
1261 + 274, the UI harness 111/111 -- all green, so nothing in ray's
prelude, verbs or the modeler writes a heap object past the barrier.

The same with `--features jit,oidn`, because T2 and the leaf compiler
hold slot pointers past the barrier: ray 1190 + 274 green, the texel bake
driver `PASS` (157× against the interpreter, compiled and interpreted
images identical), and `fruit-bowl.scm` with compiled texels 2.8 s with
and without `SCM_GC_FULL_ONLY=1`.

The scene build that prompted the collector work, `fruit-bowl.scm`, does
not move: ~31 s both with `SCM_GC_FULL_ONLY=1` and without, three runs
each under load. Sampled, it has **no collector in it at all** -- 11,777
of ~16,000 samples in `run_code` and the rest `arith_fast`, `do_call`,
`sin` and `floor`: it is the texel noise code interpreted, which the jit
feature compiles natively in a real build. So for ray today S32 is
insurance for the scenes that do keep a large live set under churn (the
`alloc.scm` shape: a model held while points are made and dropped), not a
speedup for the still-life corpus. The scene that would show it is one
with a big retained mesh and a long Scheme loop over it.

## Gate

`cargo test` 524 green (19 in `scm/tests/gc.rs`, three of them new: a
young object stored into an old vector, pair and closure variable
survives minor collections under stress; minor collections free young
garbage without a full one and `collect_now` frees an old cycle; a full
collection runs on its own once the old generation has doubled).
`cargo clippy --all-targets` 0 warnings. Conformance **1112 / 71,
unmoved**. Fuzzer seeds 42, 7, 1234, 99 × 3000 and seeds 5, 6, 8, 9 ×
2000 under `--stress` (now a minor collection at every safepoint and a
full one every eighth, so a missed barrier frees a live young object
within a few steps): 0 mismatches. Benchmarks (run with
`R7RS_BENCHMARKS` pointed at the main tree's inputs -- the worktree's
first run "passed" in 0.00 s with none, the trap CLAUDE.md names), boot,
`scm-llvm` 41/0 and the Emacs protocol test pass. The 60-minute soak: 12,408,500 rounds and as many collections, live 6163 → 6163, slot high-water 8990 → 8990, "steady".
