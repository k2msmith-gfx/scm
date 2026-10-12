# S0 — LLVM feasibility spike: results

2026-10-12. Code in [`spikes/s0-llvm`](../spikes/s0-llvm) and
[`spikes/s0-nollvm`](../spikes/s0-nollvm). This is the stage whose failure
would have invalidated [the plan](plan.md); it did not fail.

Environment: aarch64 macOS 26.5.1, rustc 1.93.0, inkwell 0.10.0,
llvm-sys 211.1.1, LLVM **21.1.8** from
`nixpkgs#llvmPackages_21.llvm.dev`.

## Verdict

| exit criterion | result |
|---|---|
| inkwell builds and verifies IR | **pass** |
| JIT'd `fn(i64)->i64` returns the right answer, aarch64-darwin | **pass** — ORCv2/LLJIT and legacy MCJIT both |
| …on x86-64-linux | **not verified** — see below |
| default `cargo build` works with no LLVM present | **pass** |

```
B: MCJIT   triple_plus_one(14) = 43
C: LLJIT   triple_plus_one(14) = 43
D: LLJIT   times_ten_plus_one(14) = 141
   and the first function still works: 43

latency: LLJIT create 390.541µs, 1st module add+lookup 1.496709ms, 2nd 868.292µs
```

## What this changes in the plan

**Nothing structural.** §2.1's recommendation stands and is now demonstrated
rather than inferred: build IR with inkwell, JIT with `llvm-sys::orc2`.

Worth noting that **MCJIT still works** in LLVM 21, so inkwell's
`ExecutionEngine` is not dead today. It is still the wrong thing to build on —
it is deprecated, it has no lazy or incremental story, and it would have to be
migrated eventually. But it is a legitimate fallback if the ORC wrapper turns
out to be a sinkhole, which lowers risk #1 slightly.

## Six things learned that are not in the plan

1. **LLVM 21 removed `LLVMOrcThreadSafeContextGetContext`.** The context flow
   therefore *inverts* from what the older tutorials show: you no longer ask
   the ThreadSafeContext for an `LLVMContextRef`, you create your own context
   and hand it over with
   `LLVMOrcCreateNewThreadSafeContextFromLLVMContext`. This was the only real
   blocker hit, and it cost one compile error to find.

2. **ORC adopts the context, so inkwell must not own it.** The working shape
   is `Context::create()` → `raw()` → `mem::forget` the owned handle → wrap it
   back as a non-owning `ContextRef` for IR building. Getting this wrong is a
   double free, not a compile error. The safe LLJIT wrapper the plan budgets
   for exists precisely to encapsulate this once.

3. **Symbol lookup takes the unmangled name, even on macOS.**
   `LLVMOrcLLJITLookup(jit, &addr, "triple_plus_one")` resolves without a
   leading underscore; LLJIT applies the data-layout mangling itself.

4. **Warm-up is sub-millisecond per function** (1.5 ms for the first module
   including LLJIT's own lazy initialization, 0.87 ms for the second; LLJIT
   construction itself is 0.39 ms). Two consequences: the JIT must be *lazy*,
   never eager at boot, or it eats the 10 ms budget in §5 of the plan; and at
   this cost, JIT-compiling a hot Scheme procedure on demand at the REPL is
   comfortably interactive.

5. **Incremental module addition works as the REPL needs.** A second module
   added after the first had already been compiled and called resolves fine,
   and the first function remains callable. That is the mechanical basis for
   §1's (6)-vs-(7) resolution — though note this spike proved *addition*, not
   *invalidation*; redefining a symbol already resolved in the same JITDylib
   is a separate experiment and should be run before S11 is designed.

6. **No `MAP_JIT` or entitlement work was needed** — LLJIT handled W^X on
   Apple Silicon internally. Important caveat: this was an unsigned CLI
   binary. A signed app with the hardened runtime (which is what the modeler
   would ship as) still needs `com.apple.security.cs.allow-jit`, and that is
   **not** verified here. Risk #3 in the plan shrinks but does not close.

## Still open from S0

- **x86-64 Linux is unverified.** This machine has no container runtime, VM or
  configured Linux host, so it could not be tested. One command on the NixOS
  box closes it:

      LLVM=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')
      cd spikes/s0-llvm && PATH="$LLVM/bin:$PATH" LLVM_SYS_211_PREFIX="$LLVM" cargo run --release

  Expected output is the block above, with `host: x86_64 / linux`. The risk is
  low — x86-64 is LLVM's best-supported JIT target and has no W^X complication
  — but "low risk" is not "verified", and S0 does not close until it is.

- **LLVM version churn is real and faster than assumed.** Cargo volunteered
  that `llvm-sys` is already published at **231.0.1** (LLVM 23) while this
  spike pins 211. Two major LLVM releases ahead of the version we just
  validated, with inkwell carrying one feature flag per version. The flake pin
  is not optional bookkeeping; it is what keeps the build reproducible while
  that moves underneath.

## The feature gate

From `spikes/s0-nollvm`, in an environment with `LLVM_SYS_211_PREFIX` unset and
no `llvm-config` on `PATH`:

```
default features:  Finished `release` profile — exit 0
--features jit:    error: No suitable version of LLVM was found system-wide or pointed
```

Which is exactly the contract: the embeddable crate builds anywhere, and
asking for the JIT without a toolchain fails loudly rather than silently
degrading. The plan's "`--no-default-features` CI job from the first commit
that has a `Cargo.toml`" should assert both halves, not just the first.
