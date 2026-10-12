# scm

An R7RS-small Scheme in Rust: embeddable as a crate, runnable standalone, with
an LLVM-backed JIT and native AOT compilation on x86-64 Linux and aarch64
macOS — and ray's scripting language.

**Status: planning**, S0 closed on aarch64 macOS. Development targets
**aarch64 macOS first**; x86-64 Linux is sequenced later, not dropped
(plan §2.5). The plan is in
[docs/plan.md](docs/plan.md); the LLVM feasibility results are in
[docs/s0-findings.md](docs/s0-findings.md).

## Requirements

Language-side: R7RS-small (qualified — see plan §3.1), embedding as a crate
with **no LLVM in the default build**, native compilation for
`x86_64-unknown-linux-gnu` and `aarch64-apple-darwin`, LLVM via `inkwell` for
IR, a standalone `scm` binary, a JIT, a strong REPL, and a runtime that cannot
abort its host.

Consumer-side, from `~/devel/ray/docs/scheme-plan.md` @5c2c2f6: keyword and
optional arguments plus a keyword datatype (87 of ray's 218 verbs depend on
the convention), verbatim error strings, dynamic variables, a surviving macro
set, **compiled field functions callable from rayon workers**, and the Janet
API preserved until a Scheme replaces it verb-for-verb.

Continuations are a three-rung ladder, and **one-shot is the cheap middle
option** both source documents skipped: escape-only is nearly free, one-shot
needs only that a frame chain outlive its extent (no copying, frames stay
mutable), and only multi-shot forces an immutable-frame discipline that taxes
ordinary code. One-shot is also the rung with a named use — ray already passes
an explicit continuation `k` for minibuffer reads, and a resumable modeler
tool is a paused computation written today as a state machine.

## Shape, in one paragraph

An explicit heap-allocated frame stack is the system of record, which makes
proper tail calls, GC roots, interrupts and live-frame backtraces one problem
instead of four. Above it: an AST interpreter kept forever as the semantic
oracle, a bytecode VM as the default engine, a **leaf compiler** that turns a
restricted pure subset into thread-safe native functions, and — last, and
optionally — a general LLVM JIT and AOT. LLVM is behind a feature flag that is
off by default, so `cargo build` works with no toolchain installed.

The leaf compiler is the capability that justifies the project: today an
authored implicit field *cannot trace*, because ray's VM is thread-local. The
general JIT is explicitly **not** about making scripting faster — there is no
Janet hot spot to speed up.

See [docs/plan.md](docs/plan.md) for the argument, the merge, the staging and
the risks.
