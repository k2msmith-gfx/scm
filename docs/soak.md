# The soak: sustained use, as opposed to intense use

2026-10-16. Written because a readiness review asked a question the test
suite could not answer: *can closures, cyclic data, mutation and garbage
collection behave correctly under sustained use?*

The existing GC tests collect at **every safepoint**. That is the right
instrument for finding a collector bug — a value held only in a Rust local
shows up immediately and deterministically, which is why it is an S3 exit
criterion rather than a debug nicety. But it is an instrument for
*intensity*, and two failures are invisible to it:

- a slow **leak**, a few cells retained per round, which one round cannot
  show and a thousand cannot hide;
- **fragmentation**, which a non-moving collector can suffer: the live set
  holds still while the heap's high-water mark climbs.

## What it runs

Rounds of ordinary allocating work, deliberately mixed, with several shapes
chosen because they are the ones a collector gets wrong: closures over
mutable state; a cycle made unreachable before the round ends; a structure
mutated *after* it is reachable from a vector; string ports; records; a
hash table filled and dropped; deep non-tail recursion, so frames are
allocated and released; an escape continuation abandoning frames
mid-flight; and exact integers past the word, so bignums are allocated too.

After each round it forces a collection and records two numbers:

| | what it means if it climbs |
|---|---|
| **live objects** | something is retained |
| **slot high-water** | the heap is fragmenting, if live is holding still |

The verdict compares the **worst** reading in the last fifth of the run
against the worst in the fifth after warmup — a maximum rather than a mean,
because a leak that only shows at a peak is still a leak, and a ratio
rather than a difference, because the absolute size is the round's business.

## The result

    cargo run --release -p scm-conform --bin soak -- --minutes 12

**2,000,000 rounds, and 2,000,001 collections. Both numbers
bit-identical from the first sample to the last: 6,223 live objects and
11,081 slots.** Not "within tolerance" — unchanged, for twelve and a half
minutes and one collection per round.

| | start | end | ratio |
|---|---|---|---|
| live objects | 6,223 | 6,223 | 1.000× |
| slot high-water | 11,081 | 11,081 | 1.000× |

A leak of a single cell per round would have added two million cells.
There is nothing there to find, and now there is a way to keep checking.

    2000000 rounds, 2000001 collections
      live  6223 -> 6223  (1.000x)
      slots 11081 -> 11081  (1.000x)
    steady: neither live objects nor the heap high-water trended up

## As a gate

`cargo test -p scm-conform --test soak` runs 600 rounds and asserts both
ratios stay within 1.05, which a one-cell-per-round leak would exceed
easily. A second test asserts the workload is **worth** watching — at least
20 collections in 40 rounds, and a live set over 500 — because a round that
allocated nothing would hold perfectly steady and prove nothing at all.

The long run is the binary, and `--minutes` is the argument that matters.
