# S31 — calls and returns inside the loop

2026-11-05, aarch64 macOS (Apple M3 Pro). The second of the three items
[s14-flat-frames-plan.md](s14-flat-frames-plan.md) left under "what next",
taken straight after S30 ([s30-superinstructions.md](s30-superinstructions.md))
because S30 showed the dispatches were not where the time was: a 2-arg
call still cost ~57 ns against Janet's 12, and `fbm`'s `hash2` is four
calls around a dozen arithmetic ops.

## What a call did

`Op::Call` → `do_call`: look the closure up in the arena, match it against
seven conditions, clone the caller's `Rc<Code>` into a `Frame::Code` and
push it, clone the callee's `Rc<Code>`, push the callee's `Frame::Code`,
return `Step::Run` to the trampoline (`eval_inner`), which counts a tick,
pops the callee's 80-byte frame again, resets `expansions`, and calls
`run_code`, which asks `native_for` whether this unit has native code.
`Op::Return` → `Step::Return(v)` → the trampoline: a tick, `current_call`
reset, pop the caller's frame, check it is not a `Rethrow`, `resume` --
one arm of a match over every T0 frame kind -- push `v`, call `run_code`,
ask `native_for` again. Two `Rc` increments and decrements, four 80-byte
frame moves, two trampoline round trips, two `native_for` checks, per call.

## What S31 does

`do_call` no longer pushes anything for a compiled flat-parameter callee:
it answers `Called::Enter { code, env, at, suspend }` and `run_code` does
the rest in place -- pushes ITS OWN frame (moving its `Rc<Code>` into it,
not cloning one), reserves the callee's slots, and continues its loop with
`code`, `pc`, `env` and `base` rebound. `Op::Return` looks at the frame
above `floor` (the stack depth when this `run_code` began, so a return
never crosses a host re-entry or an outer trampoline): a `Frame::Code`
whose unit has no native code is popped and resumed in the same loop. Any
other frame -- a T0 frame, a `Catch`, a native caller -- goes to the
trampoline as before.

The frames are the same frames in the same order, so a backtrace, a break
handler's view, an escape continuation, `dynamic-wind`, fuel and the
collector see exactly what they saw. The `do_call` check itself shrank
from seven conditions to three: `flat_params` already means a plain lambda
list (`body_unit` sets it only then), so arity and the macro generation
are all that is left to compare.

T2 is untouched: a flat unit never has native code (`native_for` declines
them), so in-place entry is always the loop that would have run it, and a
return into a native caller still goes through the trampoline, which
resumes the native function at its pc.

## S14d, revisited

S14d built "calls inside `run_code`" on 2026-10-24, measured a call 1%
faster and `fbm` 1%, and dropped it. This is the same idea and it is worth
1.3× on a call and 1.5× on `fib`. The difference is what was left in the
path: S14d kept the trampoline for the return and the clones for the
frames, and this takes the whole round trip out in both directions. The
lesson for the record: measure the piece you removed, not the idea.

## Measured

Minimum of six alternating runs, the S30 binary against this one, same
machine, load average ~2.5. The "before S30" column is the S30 record's
own measurement of the commit before it, the same evening, so the last
column is what S30 and S31 together bought:

| per operation (T1) | before S30 | S30 | S31 | S31 / S30 | S30+S31 speedup |
|---|---|---|---|---|---|
| empty `do` iteration | 19.9 ns | 15.3 ns | 15.0 ns | 1.02× | 1.33× |
| 2-arg call | 66.5 ns | 58.1 ns | 44.5 ns | 1.30× | 1.49× |
| call + `let*` of 4 | 105 ns | 91.7 ns | 76.5 ns | 1.20× | 1.37× |
| `hash2` | 167 ns | 161 ns | 133 ns | 1.21× | 1.26× |
| `fbm` | 4.58 µs | 4.55 µs | 3.85 µs | 1.18× | 1.19× |
| `(fib 25)` | 13.4 ms | 12.8 ms | 8.35 ms | 1.53× | 1.60× |

The Steel set ([vs-steel.md](vs-steel.md)), T1, both binaries back to back
on a quiet machine: `fib` 143 → 96 ms (1.49×), `tak` 150 → 110 ms (1.36×),
`closures` 155 → 141 ms, `lists` 1841 → 1639 ms, `fbm` 78 → 66 ms; `loop`,
`vec`, `strings` and `callcc` unchanged. **Geometric mean 1.59× → 1.81×**
Steel's speed. Every program that is mostly calls moved; nothing regressed.

**From the start of S14** (2026-10-24, `fbm` 8.5 µs, a call 166 ns): a call
is now 44 ns (3.8×) and `fbm` 3.85 µs (2.2×). Janet's `fbm` is 1.16 µs; the
gap is 3.3×, from 7.3×.

## What a call still does

Look the closure up in the arena and match it; clone the callee's
`Rc<Code>`; push the caller's frame (80 bytes, of which 32 are written);
`current_call` written twice; the callee's slot reservation. The next
things to take out, in order of likely return: the `Rc` clone per call
(the callee is slot 0 of the flat frame and so rooted; a frame could hold
the code by a non-owning handle), and the frame's size. Neither is
measured yet.

## Gate

`cargo test` all green (26 in `scm/tests/flat_frames.rs`, four of them
new: 100,000-deep non-tail recursion, a return into a T0 caller, an escape
and a `guard` from inside in-place calls; a fuel stop inside them; a host
re-entry whose returns stop at its floor; the backtrace through them).
`cargo clippy --all-targets` 0 warnings. Conformance **1112 / 71,
unmoved**. Fuzzer seeds 42, 7, 1234, 99, 2024, 31337 × 3000 and a
`--stress` run: 0 mismatches (the skip counts differ from S30's runs
because S30's arm 57 changed the program stream, checked by running seed
42 at the S30 commit: the same 6). Benchmarks, boot, `scm-llvm` 41/0 and
the Emacs protocol test pass.
