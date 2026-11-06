# S30 — superinstructions: an arithmetic op with its operands, callee and branch folded in

2026-11-05, aarch64 macOS (Apple M3 Pro). The first of the three items
[s14-flat-frames-plan.md](s14-flat-frames-plan.md) left under "what next":
after S14, `run_code`'s dispatch and value-stack traffic were 66% of the
`fbm` profile, and the way to cut dispatches without changing the
compiler's shape is to make the commonest sequences one op.

## What was there

S14e compiled `(+ a b)` to `Global(+) <a> <b> Arith`: the callee pushed
first (T0's order), the operands, then one op that checks the callee is
still the primitive and computes two fixnums or flonums in place. With S14's
flat frames an operand is usually `Arg(i)` or `Const(c)`, so a loop's
`(= i n)` was `Global Arg Arg Arith JumpIfFalse` -- five dispatches, four of
them moving one value each.

## What S30 does

`Op::ArithSrc(i)` names `code.ariths[i]`, an `ArithSite`: the operation,
the primitive, the form, and

- **the operands' sources** -- `Src::Stack`, `Src::Arg(slot)` or
  `Src::Const(i)`. A flat-slot read or a constant has no effect and cannot
  fail, so it can be read by the op instead of pushed before it;
- **the callee**, `Some(global)` when NEITHER operand is on the stack: then
  nothing ran between where `Global` would have read it and the op, so
  reading it at the op is still T0's order. With an operand on the stack
  the `Global` op stays, because that operand's evaluation could redefine
  the operator and T0 read it first (`a_redefined_operator_is_honoured_in_order`
  pins exactly that);
- **the branch**, `Some(target)` when the op is a comparison and an `if`'s
  test: the fast path branches itself and skips the `JumpIfFalse` after it.
  The `JumpIfFalse` stays in the stream for the slow path (and for any
  jump that lands on it from an inner `if`'s end), which leaves the
  primitive's value and falls through to it.

The fast path is `arith_fast`, unchanged; the slow path lays the callee and
operands out on the stack exactly as `Arith` has them and calls `do_call`,
so every result and every error is still the primitive's, or the redefined
operator's.

So `(if (< i n) …)` is one op where it was five; `(+ i 1)` is one where it
was four; `(- x (floor x))` is `Global Global Arg Call ArithSrc` where it
was `Global Arg Global Arg Call Arith`.

### One rule about order

A first operand folds past a second that stays on the stack only when the
second's ops cannot write that slot: no `SetArg`/`SetArgDef` of it, and
no `JumpCall` (a loop jump reassigns the loop's slots). `(+ a (begin (set! a 5) 1))`
is 4 on T0 and stays `Global Arg … Arith`; `(+ a (if (> a 2) (* a 10) (- a)))`
folds `a`, which deletes its `Arg` from under the `if`'s jumps.

### Deleting an op

The compiler emits the operands as it always did, then looks at what it
emitted: an operand whose range is exactly one `Arg` or `Const` op is a
source, and its op is deleted (`delete_op`), moving every pc after it down
by one -- jump targets, the pending-call ranges a backtrace reads, the
slot-name ranges a break's view reads, fused branch targets and the tops of
open loops. A forward jump not yet patched holds 0, which no deletion
moves; every pc an enclosing form holds in a Rust local was taken before
the deleted op existed and so is at or before it.

### What is not fused

`Local` reads (a heap environment's variable) and `ArgDef` reads (an
internal define's slot, which may be unbound) stay ops. T2 declines a unit
with `ArithSrc` as it declines one with `Arith` (S14f is still owed).

## Measured

Minimum of six alternating runs of the S14 micro-benchmarks (the Steel
set's `fbm` as a procedure, `do` loops around each shape), the binary
before this change against the one after, same machine, load average
~2.5–3 (a browser was open; ratios are the part to read):

| per operation (T1) | before | after | |
|---|---|---|---|
| empty `do` iteration | 19.9 ns | 15.1 ns | 1.32× |
| 2-arg call | 66.5 ns | 57.2 ns | 1.16× |
| call + `let*` of 4 | 105 ns | 90 ns | 1.17× |
| `hash2` | 167 ns | 159 ns | 1.05× |
| `fbm` | 4.58 µs | 4.45 µs | 1.03× |
| `(fib 25)` | 13.4 ms | 12.7 ms | 1.06× |

The Steel set ([vs-steel.md](vs-steel.md)), T1, same session: `fib` 151 →
140 ms, `loop` 251 → 230 ms, `fbm` 79 → 76 ms, `callcc` 93 → 84 ms, the
rest within noise; **geometric mean 1.55× → 1.60×** Steel's speed.

**What it bought, honestly.** The empty loop is now four ops
(`ArithSrc`+branch, `ArithSrc`, `SetArg`, `JumpCall`) where it was nine,
and runs 1.3× faster -- so a dispatch is on the order of 1 ns, and what
each op does (the tick count, the bounds-checked fetch, the value-stack
push) is the rest. Anything with a call in it moved 3–17%, because a call
is still ~57 ns and `fbm`'s `hash2` is four of them (`hash2`, `fract`,
`sin`, `floor`) around a dozen arithmetic ops. S14's "next" list expected
"most of a register VM's cut in dispatches" from this item, and that is
what it got; it is just that dispatches were a smaller share of the time
than the 66% `run_code` figure suggested, since the value-stack traffic
inside `run_code` is per op and only partly per dispatch.

**So the next multiple is the call path**, S14's second item: `do_call`'s
seven-condition match on the closure, the two `Rc` clones, the
`Frame::Code` with an `Rc` in it pushed and dropped per call. The profile
after S30 should be taken before touching it.

## Gate

`cargo test` all green (22 in `scm/tests/flat_frames.rs`, five of them
new: every fused shape against the same operation through `apply`; a fused
comparison branching on the primitive's answer and on a redefined `<`; the
order rule with a `set!` in the second operand, an `if` and a loop there;
the backtrace through a fused op; a folded callee that is no longer a
procedure). `cargo clippy --all-targets` 0 warnings. Conformance
**1112 / 71, unmoved**. Fuzzer arm 57 generates every fused shape in one
expression (1157 of 3000 programs carry it, seed 31337); seeds 42, 7, 1234,
99, 2024 and 31337 × 3000 and a `--stress` run: 0 mismatches. Benchmarks,
boot, `scm-llvm` 41/0 and the Emacs protocol test pass. Soak not re-run:
nothing here allocates.
