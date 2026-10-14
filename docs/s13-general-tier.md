# S13 — T2, the general tier: what it bought, measured

2026-10-13, aarch64 macOS, LLVM 21.1.8. `scm/src/vm/native.rs` (the runtime
half, no LLVM), `scm-llvm/src/t2.rs` (the code generator). Opt in with
`cargo build --release -p scm-cli --features jit` and `SCM_TIER=t2`;
`SCM_T2_HOT=N` sets how many runs earn a compile.

## The shape

T1's `Code`, compiled op for op to a native function that **still borrows
the stack** (plan §5): it returns to the Rust loop at every call that is
not a plain primitive, and is resumable at any call site through a `switch`
on pc at its entry. So proper tail calls, escape continuations, `%try`,
`dynamic-wind`, interrupts, fuel and the collector are inherited from T1
with no new mechanism -- the "real technical risk" in §3.2, `musttail`, was
never taken, and `call/cc` unwinds a native frame exactly as it unwinds a
bytecode one, because they are the same `Frame::Code`.

What the native code keeps for itself: the operand stack at **fixed
offsets** (every pc has a static depth, so no stack pointer exists at run
time); constants, jumps and the boolean test as instructions; the two
innermost frames and the global table read **directly** through pinned
layouts (`layouts_native_code_relies_on` is the pin); and fixnum
`+ - * < > <= >= =` inline behind a guard on the callee value itself. The
guard is the invalidation story: the callee was loaded from the global cell
a moment ago, so **a redefinition at the REPL is seen on the next
execution and there is nothing to invalidate**. The exit criterion's second
clause is met by construction, and `redefining_a_global_takes_effect_in_compiled_callers`
redefines `+` under a compiled caller to prove it.

A `Code` is compiled after `native::HOT` runs (400). Compile results are
per `Code`, so every closure from one lambda shares one native function.

## Agreement

- The conformance suite under T2: **1106 passed, 74 failed, 1180 reached,
  identical failure lines** to T0 and T1.
- The 53-program benchmark gate passes under T2 with every answer the same.
- Twenty hot programs through every op kind -- loops, closures, vectors,
  records, `guard`, `parameterize`, `call/cc`, internal defines, `set!`,
  flonums, `apply`, `let-values`, 3000-deep recursion -- agree with T1.
- An error in native code is the same error with the **same backtrace**,
  frame for frame; fixnum overflow leaves the fast path and raises T1's
  error.
- A 200,000-iteration tail loop keeps the frame stack under 16 and runs out
  of fuel. (Fuel counts safepoints, which native code takes at calls and
  loop back-edges rather than per op, so a budget lasts longer under T2.)

## Speed

| | T1 | T2 | |
|---|---|---|---|
| `(tak 24 16 8)` | 0.28 s | 0.21 s | 1.3× |
| `(fib 32)` | 0.75 s | 0.57 s | 1.3× |
| the 53 programs, reduced sizes | 8.0 s | 12.4 s | **0.65×** |
| the same with compilation never triggered | 8.0 s | 8.4 s | |

IR optimisation passes changed nothing measurable (`SCM_T2_PASSES=1` to
check), so they are off; the compile cost left is the backend's own, about
4 s across the 53 programs, and it is why T2 loses on short programs.

**Where the time is.** Sampling `fib` under T2: the native code is **6%**
of the profile. The rest is the call protocol -- the Rust loop, the frame
push, the environment allocation per call -- and the collector. Compiling
the body better cannot move that; the plan's accepted price, *"frames in
memory cap straight-line speed; the general JIT tier will not turn a Scheme
call into a native call with arguments in registers"*, is exactly the
ceiling measured here.

## The decision this poses

S13 was budgeted at 1.5+ and gated on measurement. About 0.4 is spent and
the mechanism is complete and agrees everywhere. The next multiple is a
different piece of work:

- **native-to-native calls** that do not return to the Rust loop, with
  frames still pushed for the collector and the backtrace;
- **an environment pool**, deleting the `Vec` per call frame -- which would
  help T1 as much;
- **unboxed locals** across those calls, which is where a real general tier
  gets its single-digit multiple.

That is the "several times the work" §3.2 predicted, and ray's own
measurement is that its non-leaf code is not interpreter-bound. The leaf
compiler is 199× T1 on the texel loop; T2 is 1.3×. The honest state: T2
works, costs more than it gives on short programs, and the plan's option to
stop here is a live one.
