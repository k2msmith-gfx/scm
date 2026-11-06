# S33 — a primitive called from the loop

2026-11-05, aarch64 macOS (Apple M3 Pro). After S30–S32 the programs still
behind were the primitive-bound ones: string and vector loops, list
walks. Profiled on long-running versions of the Steel set's `strings` and
`vec`, the primitives themselves were small (`string_ref` 69 samples,
`char_eq` 112, `vector_ref` 51) and **`do_call`'s own self time was half
of the string scan** (621 of ~1200) and a quarter of the vector loop.

## What was tried, in order, each measured alone

Minimum of three or four alternating runs on a loaded machine (another
agent's gates were running; absolute numbers swing, the alternation is
what to read):

1. **`arith_fast` inlined, two fixnums first.** The profile charged it a
   quarter of the vector loop. Measured: ~2%, noise. Kept, since it is
   the shape the function should have had, but it was never the cost.
2. **`current_call` set only when a primitive fails.** Two 16-byte stores
   per call. Measured: nothing. Kept for the same reason.
3. **The plain-primitive call made from `run_code` itself**, not inside
   `do_call`. `do_call` is a large function with a large frame, and every
   primitive call paid its prologue and its 40-byte `Result` return for a
   branch that is ten lines long. The `call!` macro now has that branch
   verbatim and enters `do_call` only for everything else.

   | loop | S32 | S33 | |
   |---|---|---|---|
   | `car`/`cdr`/`null?` walk | 76.7 ns | 56.2 ns | 1.37× |
   | `vector-ref` | 53.0 ns | 47.2 ns | 1.12× |
   | `string-ref` + `char=?` | 62.7 ns | 55.9 ns | 1.12× |

4. **A primitive in tail position returns in place** (S31's path, which
   the inlined branch had been bypassing): `(define (first p) (car p))`
   called in a loop, 53.3 → 50.6 ns.

The lesson is the same as S31's: the cost of a call was the machinery
around it, and the way to find which piece is to remove pieces and
measure, since the profile charged `do_call`'s self time without saying
which of its lines.

## Measured

The Steel set ([vs-steel.md](vs-steel.md)), T1, the S32 binary against
this one back to back, under a load average of 26 (ratios, not absolutes):
`lists` 1629 → 1337 ms (1.22×), `loop` 1.11×, `vec` 1.10×, `fib` and
`closures` 1.09×, `fbm` 1.08×, `callcc` 1.06×, `tak` 1.07×, `strings`
level (it is `string-append` copying, not calls). **Geometric mean 1.83× →
1.99× Steel's speed, 1.09× for the stage.** Every program with a primitive
call in its loop moved, which after S30–S32 is most of them.

## Gate

`cargo test` 524 green; `cargo clippy --all-targets` 0 warnings.
Conformance **1112 / 71, unmoved**. Fuzzer seeds 42, 7, 1234, 99, 2024 ×
3000 and seed 11 × 2000 under `--stress`: 0 mismatches. Benchmarks, boot,
`scm-llvm` 42/0 and the Emacs protocol test pass. No soak: nothing here
allocates or collects differently.
