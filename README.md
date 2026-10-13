# scm

An R7RS-small Scheme in Rust: embeddable as a crate, runnable standalone, with
an LLVM-backed JIT and native AOT compilation on x86-64 Linux and aarch64
macOS — and ray's scripting language.

**Status: S0–S11 done** on aarch64 macOS — reader, interpreter, GC, the
lambda list, `syntax-rules`, the embedding API, the REPL protocol, the rest
of R7RS-small, the boot budget, and the leaf compiler. Next is S11a, live
recompilation, then S12, the bytecode VM.
Development targets **aarch64 macOS first**; x86-64 Linux is sequenced
later, not dropped (plan §2.5). The plan is in [docs/plan.md](docs/plan.md);
the LLVM feasibility results are in [docs/s0-findings.md](docs/s0-findings.md).

## Running it

    cargo build --release
    ./target/release/scm                      # a REPL
    ./target/release/scm -e '(+ 1 2 3)'       # one expression
    ./target/release/scm scene.scm            # a file
    ./target/release/scm --fuel 10000 …       # bound a runaway
    ./target/release/scm --serve 127.0.0.1:7000   # the REPL protocol over TCP

Boot is budgeted at under 10 ms for the whole process and measured at 1.8 ms;
`cargo test` fails if it regresses (plan §7).

```
scm> (define (fact n)
...    (if (= n 0) 1 (* n (fact (- n 1)))))
scm> (fact 20)
2432902008176640000
scm> (let ([x 1] [y 2]) (+ x y))
3
scm> (seq (i 0 5) (* i i))
(0 1 4 9 16)
```

An unclosed paren continues the entry rather than reporting an error, which
is what S1's *incomplete* vs *invalid* distinction was for. The terminal and
the socket are two clients of one protocol, [docs/repl.md](docs/repl.md).

## The leaf compiler

    LLVM=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')
    LLVM_SYS_211_PREFIX=$LLVM cargo test -p scm-llvm
    LLVM_SYS_211_PREFIX=$LLVM cargo run --release -p scm-llvm --example texel_cost

`scm-llvm` compiles a pure numeric procedure — a texture, a field, a noise
function — to native code that is VM-free, allocation-free and `Send +
Sync`, and refuses anything else with a message naming the call. It is not
a default workspace member: a root `cargo build` needs no LLVM, and
`-p scm-llvm` without one fails loudly. A 4-octave `fbm` texture costs
**77 ns/texel** compiled against 69 µs interpreted; see
[docs/s11-leaf.md](docs/s11-leaf.md).

## Conformance

    cargo run --release -p scm-conform -- scm-conform/suites/r7rs-tests.scm out.md

Chibi-Scheme's `r7rs-tests.scm` — written by other people against the spec
— vendored unedited. The first baseline was **45.9%**; after S9 it is
**1103 of 1114, 99.0% of the declared subset**, and each of the 11 remaining
failures is explained individually: 4 are declines recorded before they were
measured, 7 are hygiene corners. A percentage with an unexplained remainder
is a marketing number. See [docs/conformance.md](docs/conformance.md).

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
