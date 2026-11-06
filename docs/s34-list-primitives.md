# S34 — the list primitives in the loop, and the tick moved

2026-11-05, aarch64 macOS (Apple M3 Pro), the stage after
[S33](s33-primitive-calls.md). The programs still behind were the list
walks: `lists`' second half, `assv` over a 200-entry alist, cost 53 ns a
step, and every `car`, `cdr`, `null?` and `eqv?` in it was a full
primitive call -- S30's folded op covered arithmetic and comparisons only.

## What was done

1. **`car`, `cdr`, `null?`, `pair?`, `not`, `zero?`, `eq?`, `eqv?`,
   `cons`, `vector-ref`, `string-ref` and `char=?` take S30's path.**
   `ArithOp` grew the variants (`arity()` and `yields_bool()` replace the
   two places that spelled "unary means `Neg`" and "a comparison can take
   its branch"), `arith_op` names them, and `prim_fast` computes the cases
   the primitive would, declining to it for everything else: the wrong
   type, an index out of range, two heap objects under `eqv?` (bignums
   compare by value, which only the primitive knows). Every answer and
   every error is therefore still the primitive's, a redefined `car` is
   still seen, and every predicate folds into the `if` it tests. `cons`
   allocates in the loop, which is safe because `alloc` never collects: a
   collection comes only at a safepoint, which pushes the frame first.

2. **The tick is paid at a call and a backward jump, not at every op.**
   Every loop passes through one of those, so a runaway is still stopped
   and a safepoint still comes round; the increment, compare and branch
   had been a measurable share of a straight-line op. The unit of fuel is
   unchanged -- a safepoint -- but a safepoint now comes every 1024 calls
   or jumps rather than every 1024 ops, so a budget that stopped a
   million-call recursion at 2,000 needs 500 (`flat_frames.rs`).

Two things measured and found not to be the cost, kept as shape only:
`Engine::obj` marked `#[inline]` (reverted) and the callee check written
as a `matches!` instead of `Value`'s derived `==`, which the profile had
shown as an out-of-line call (kept, 0%).

## Measured, each alone

| loop | S33 | after 1 | after 1 + 2 |
|---|---|---|---|
| `car`/`cdr`/`null?` walk, 20M steps | 850 ms | 780 ms (1.09×) | 760 ms (1.12×) |
| `assv`, 20M steps | 1050 ms | 1045 ms (1.00×) | 950 ms (1.11×) |
| `loop`, 5M with `modulo` | 210 ms | | 200 ms |

Smaller than the primitive-call stage before it. Profiled after step 1,
the walk was 78% in `run_code`'s own code, 9% in `do_call` (the self
tail call) and 8% in the `drain` that slides a tail call's arguments
down: the folded op is cheaper than a call, but not by the margin a call
to `car` suggested, because the general op -- read the site, read the
callee, read two operands, check the callee, truncate, branch or push --
is itself some 4 ns. **The per-op cost of the loop is now the floor**, and
the next multiple is a different dispatch (unchecked slot access, or a
register machine), which is a separate decision and not a stage.

## Against Steel

The S33 binary against this one, interleaved, load average ~8 (the
soak running on one core):

| program | Steel | S33 | S34 | S34 / S33 | Steel / S33 | Steel / S34 |
|---|---|---|---|---|---|---|
| `startup` | 55 ms | 3 ms | 3 ms | 1.09× | 16.16× | 17.60× |
| `fib` | 198 ms | 86 ms | 88 ms | 0.97× | 2.31× | 2.24× |
| `tak` | 178 ms | 102 ms | 99 ms | 1.03× | 1.74× | 1.79× |
| `loop` | 403 ms | 210 ms | 212 ms | 0.99× | 1.92× | 1.90× |
| `vec` | 218 ms | 77 ms | 79 ms | 0.98× | 2.83× | 2.77× |
| `strings` | 86 ms | 51 ms | 53 ms | 0.95× | 1.70× | 1.61× |
| `closures` | 180 ms | 127 ms | 128 ms | 0.99× | 1.42× | 1.40× |
| `lists` | 1864 ms | 1329 ms | 1189 ms | 1.12× | 1.40× | 1.57× |
| `fbm` | 146 ms | 62 ms | 61 ms | 1.01× | 2.35× | 2.38× |
| `callcc` | 212 ms | 78 ms | 78 ms | 1.00× | 2.72× | 2.72× |
| **geometric mean, excluding startup** | | | | **1.00×** | **1.98×** | **1.99×** |

**1.00× on the set; `lists` 1.12×**, the one program whose loop is list
primitives and a self tail call; `strings` at 0.95× is 2 ms of noise
(five alternations at 10 ms resolution: equal). The stage's value is the
finding above -- where the floor is -- more than the number.

### The day, 2026-11-05

The commit before S30 (`f70d5fd`) against this one, Steel 0.8.2 beside
both, interleaved on the same sitting:

| program | Steel | before S30 | S34 | S34 / before | Steel / before | Steel / S34 |
|---|---|---|---|---|---|---|
| `startup` | 55 ms | 3 ms | 3 ms | 1.01× | 16.53× | 16.73× |
| `fib` | 198 ms | 152 ms | 89 ms | 1.70× | 1.30× | 2.22× |
| `tak` | 178 ms | 151 ms | 99 ms | 1.52× | 1.18× | 1.79× |
| `loop` | 400 ms | 253 ms | 212 ms | 1.19× | 1.58× | 1.89× |
| `vec` | 216 ms | 89 ms | 79 ms | 1.13× | 2.43× | 2.74× |
| `strings` | 86 ms | 50 ms | 52 ms | 0.97× | 1.73× | 1.67× |
| `closures` | 180 ms | 149 ms | 129 ms | 1.16× | 1.21× | 1.40× |
| `lists` | 1866 ms | 2287 ms | 1189 ms | 1.92× | 0.82× | 1.57× |
| `fbm` | 147 ms | 79 ms | 61 ms | 1.28× | 1.87× | 2.40× |
| `callcc` | 212 ms | 82 ms | 78 ms | 1.05× | 2.57× | 2.71× |
| **geometric mean, excluding startup** | | | | **1.29×** | **1.54×** | **1.99×** |

**1.29× on itself, 1.54× → 1.99× Steel's speed**, from S30
superinstructions (1.03×), S31 calls in the loop (1.14×), S32 the
collector (1.00× here, 1.37× on the scene-shaped workload it is for),
S33 primitives in the loop (1.09×) and S34 (1.00×). The call-shaped
programs moved most (`fib` 1.70×, `tak` 1.52×, `lists` 1.92×, the one
program that was behind Steel and is now 1.57× ahead); the ones inside
large primitives (`strings`, `callcc`) hardly at all, as they should
not have.

## Gate

`cargo test` 525 green (one new test, `flat_frames.rs`
`fused_list_primitives_are_the_primitive_in_every_case`; the fuzzer's arm
58 carries every fused shape, including the errors, through all three
engines); `cargo clippy --all-targets` 0 warnings. Conformance **1112 / 71, unmoved**. Fuzzer seeds 42, 7, 1234, 99, 2024 ×
3000 and seed 11 × 2000 under `--stress`: 0 mismatches. Benchmarks, boot,
`scm-llvm` 42/0 and the Emacs protocol test pass. The 60-minute soak, run
because the tick cadence decides when a collection comes round:
15,004,500 rounds and as many collections, live 6163 → 6163, slot
high-water 9453 → 9453, "steady".
