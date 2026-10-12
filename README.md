# scm

An R7RS-small Scheme in Rust: embeddable as a crate, runnable standalone,
with an LLVM-backed JIT and native AOT compilation on x86-64 Linux and
aarch64 macOS.

**Status: planning.** No code yet. The plan is in
[docs/plan.md](docs/plan.md).

## Requirements

1. R7RS-small conformance, reported as a measured number against external suites.
2. Easy embedding in a Rust application, as a crate — which means the default
   build has no LLVM dependency.
3. Native compilation for `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`.
4. LLVM as the code generator, via `inkwell` for IR.
5. A standalone `scm` binary.
6. JIT compilation.
7. Strong REPL support — which, with ray as the first consumer, means an eval
   *protocol* with interrupt/timeout/fuel and live-frame backtraces, of which
   the terminal prompt is one client.
8. (Added) A runtime that cannot abort the host.

## Shape, in one paragraph

An explicit heap-allocated frame stack is the system of record, which makes
`call/cc`, proper tail calls and GC roots one problem instead of three. Above
it sit four tiers: an AST interpreter kept forever as the semantic oracle, a
bytecode VM as the default engine, an LLVM JIT, and LLVM AOT. LLVM is behind a
feature flag that is off by default, so `cargo build` works with no toolchain
installed. Every tier runs every conformance test and they must agree.

First consumer is ray and the modeler, so embedding, the REPL and boot time
all land before the JIT.

See [docs/plan.md](docs/plan.md) for the argument, the staging, and the risks.
