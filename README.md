# scm

An embeddable R7RS-small Scheme written in Rust, with no dependencies.

Three engines run every program and have to agree on the result: an AST
interpreter that acts as the semantic reference, a bytecode VM that is the
default, and an optional LLVM tier. A fourth compiler takes a restricted
pure subset of the language — a texture, a noise field — and emits native
code that needs no VM, allocates nothing, and is `Send + Sync`.

```rust
let mut scm = Interp::new()?;
scm.define_fn("host-add", |args| match args {
    [Val::Int(a), Val::Int(b)] => Ok(Val::Int(a + b)),
    _ => Err("host-add: wants two exact integers".into()),
})?;
let r = scm.eval("(host-add 20 22)")?;
assert_eq!(scm.value(&r)?, Val::Int(42));
```

## What it is for

scm is meant to be embedded in a Rust program that needs a scripting
language it can trust. It was written for one in particular:
[ray](https://github.com/k2msmith-gfx/ray), a procedural 3D modelling and
rendering system. That explains the priorities, which are narrower than
"a Scheme".

**A runaway script cannot take the host down.** This is a requirement, not
a feature, and it is why every entry point returns `Result`. Fuel limits
how much work a script may do, a heap cap limits how much memory it may
use, an interrupt handle lets another thread stop it, and a timeout limits
wall-clock time. If a panic does escape, the interpreter is marked poisoned
rather than pretending its state is still meaningful. Testing with
deliberately hostile programs turned up four ways a script could kill its
process; all four are now ordinary Scheme errors that `guard` can catch.

**A host is never handed a dangling value.** The embedding API exists
because the problem was real: a host held the result of an eval, ran more
script, and the value it held quietly became a different object. `Rooted`
keeps a value alive and in place. `Val` is the host's own view of a value,
containing no interpreter-internal references.

**Authored code can become native code.** This is the capability that
justifies the project. A renderer's worker threads cannot call into a
thread-local VM, so an implicit field written in Scheme could not be traced
at all. The leaf compiler turns a pure numeric procedure into a plain
function those threads can call. Measured speedups are **138× to 406×**
over the bytecode VM on noise and vector kernels, and a bilinear lookup
into a 512² image takes **3.3 ns** against 935 ns interpreted.

**Startup time is a design constraint.** The whole process — load, prelude,
evaluate, exit — has a 10 ms budget and currently takes **2.9 ms**, of
which 1.8 ms is just loading the binary. A test fails if that regresses.

**No dependencies.** The crate pulls nothing from crates.io. That is why
the bignums are hand-written, and why a wide exact integer crosses the host
boundary as text instead of exposing an internal type. The default feature
set is empty, so plain `cargo build` needs no LLVM.

One thing scm is *not* for: the general JIT is not about making scripting
faster. There was no hot spot that needed speeding up. It exists so that
authored code can leave the VM entirely.

## Compatibility

[docs/subset.md](docs/subset.md) is the normative statement of what the
language is: the baseline, the feature identifiers, the four deviations,
the resource limits, and what is deliberately not promised. Every claim in
it is checked by `scm/tests/subset.rs`, so a sentence that stops being true
breaks the build. What follows is a summary.

**R7RS-small, with qualifications.** The conformance suite is
Chibi-Scheme's `r7rs-tests.scm`, written by other people against the spec,
vendored unedited and run unchanged:

    cargo run --release -p scm-conform -- scm-conform/suites/r7rs-tests.scm out.md

**1112 pass and 71 fail** of the 1183 assertions reached. The first
baseline was 45.9%. A percentage with an unexplained remainder is a
marketing number, so [docs/conformance.md](docs/conformance.md) accounts
for the whole remainder: every numeric failure is the one optional feature
declined below, and the rest are `syntax-rules` hygiene corners, each one
named.

`(features)` is the contract, and each identifier is claimed only when it
is actually true:

| claimed | `r7rs` `ieee-float` `exact-closed` `scm`, plus the platform's |
|---|---|
| **not claimed** | `ratios` `exact-complex` `full-unicode` |

### The four deliberate deviations

Each one is a decision with a reason behind it, not an unfinished corner.

- **Continuations are escape-only.** `call/cc` works for escaping: early
  return, generators that never resume, exception-like control flow. A
  re-entrant continuation raises an error instead of misbehaving. This is a
  genuine deviation from R7RS and it has a cost — `(srfi 158)` loses its
  lazy coroutine generator, which now runs eagerly and raises past a cap
  rather than hanging.
- **No exact rationals.** This is load-bearing rather than unfinished.
  `(/ 8 3)` is the flonum `2.666…`, which matches the language scm
  replaces, so a converted scene computes what the original computed. In a
  sibling implementation, `8/3` silently turned a numerical benchmark into
  exact-rational arithmetic with no warning at all; that is the measured
  reason for avoiding the alternative. See
  [docs/numeric-tower.md](docs/numeric-tower.md).
- **`:at` reads as a keyword, not as a symbol.** In standard Scheme a colon
  is just an ordinary identifier character, so `:at` is simply a symbol
  whose name happens to start with one, and `(symbol? ':at)` is `#t`. In scm
  it is a *keyword* instead — a distinct type that is not a symbol, is not
  `eq?` to the symbol `at`, and evaluates to itself:

  ```scheme
  (list (symbol? ':at) (keyword? ':at) (eq? ':at 'at))   ; => (#f #t #f)
  (sphere :at (vec3 0 0 0) :radius 3)                    ; no quoting needed
  ```

  The reason is that ray's API is keyword-shaped and 87 of its 218 verbs use
  the convention. When the argument binder walks a call's arguments it has to
  decide, for each one, whether it is a *label* naming the next argument or a
  *value*. If `:at` were a symbol that would be a guess, because a symbol is
  also a perfectly good value to pass. A separate type leaves nothing to
  guess. `#:at` reads as the same keyword, and is never printed.

  The cost is that code wanting the *symbol* `:at` must write `'|:at|`, and
  that a colon-initial name can never be a procedure name, a macro name or a
  `syntax-rules` literal. `ReaderOptions::colon_keywords` turns the whole
  behaviour off, which is how the conformance suite runs.
- **Importing copies bindings.** A library is an environment frame, and an
  import copies its exports. So if a library `set!`s one of its own
  exported variables after the fact, importers do not see the change. R7RS
  makes `set!` on an imported binding an error, and no library in the wild
  relies on the other behaviour. All engines implement the same
  simplification, and the differential fuzzer checks that they agree.

Exact integers *are* unbounded — `(* 9223372036854775807 2)` is exact —
with a ceiling of 2^20 bits, because "unbounded" must not mean "allowed to
exhaust the host". Strings are `Vec<char>`, so `string-ref` is O(1).
`full-unicode` is unclaimed because there is no case folding and no Unicode
digit values. Char-sets are Latin-1; for Unicode-correct matching the
string procedures accept a predicate, and `char-alphabetic?` and its
relatives are correct over the whole range.

## What Scheme code it will run

This is the practical question, so here is a concrete answer rather than a
pointer to a standard.

**Other people's test suites run.** Chibi-Scheme's `r7rs-tests.scm`, written
against the spec by someone with no interest in scm, runs unedited and
**1112 assertions pass**. ecraven's `r7rs-benchmarks` corpus also runs: 53
of its programs fall inside the declared subset, and **all 53 produce the
recorded answers** under both engines. Code written by strangers is the
strongest evidence available — stronger than examples written here.

**Every `(scheme …)` library imports**, because the standard bindings *are*
the prelude, which makes importing one a no-op:

    (import (scheme base) (scheme write) (scheme char) (scheme inexact)
            (scheme lazy) (scheme case-lambda) (scheme eval) (scheme read)
            (scheme file) (scheme time) (scheme process-context) (scheme cxr))

`define-record-type`, `syntax-rules` with nested ellipses, `guard` and
`raise`, `dynamic-wind`, `parameterize`, `delay`/`force`/`make-promise`,
`case-lambda`, string ports, bytevectors, and `eval` with
`interaction-environment` are all present, and all exercised by the suite
above.

**One sharp edge.** Importing *any* `(scheme …)` name succeeds, including
one scm does not implement. `(import (scheme complex))` is accepted, and
then `make-rectangular` turns out to be an unbound variable. You find the
gap at the call site rather than at the import.

### What will not run

| | |
|---|---|
| a **re-entrant** continuation | raises. Escaping works; resuming a dead extent does not. So no resumable generators, no `amb`, no coroutines |
| exact rationals | `1/2` is a read error, not a number. `(/ 8 3)` is a flonum |
| complex numbers | `make-rectangular` and friends are absent |
| `:at` as a *symbol* | it reads as a keyword unless `colon_keywords` is off; `'\|:at\|` still gets you the symbol |
| Unicode case *folding* in `string-ci=?` | `full-unicode` is unclaimed; `char-upcase` declines any mapping that is not a single character, as R7RS requires |
| bitwise ops on a bignum | they are 64 bits wide and say so rather than truncating |

Everything on that list fails loudly. The rule throughout is that anything
scm cannot do raises an error instead of returning a plausible wrong
answer. That is also why `(/ 8 3)` being a flonum is a stated decision and
not a silent approximation.

## Libraries built in

**37 libraries are embedded in the binary**, so `scm` needs no library
directory next to it and a standalone binary carries them too. Twenty-one
distinct SRFIs:

| | |
|---|---|
| lists, control | `(srfi 1)` `(srfi 2)` `(srfi 8)` `(srfi 26)` `(srfi 111)` |
| destructuring | `(srfi 204)` — `match`, the Wright-Cartwright-Shinn matcher |
| strings, chars | `(srfi 13)` `(srfi 14)` `(srfi 130)` |
| vectors, arrays | `(srfi 4)` `(srfi 63)` `(srfi 133)` `(srfi 160 base)` + 10 element types |
| tables, comparators | `(srfi 69)` `(srfi 125)` `(srfi 128)` |
| numbers, bits | `(srfi 27)` `(srfi 151)` `(srfi 194)` |
| iteration | `(srfi 42)` `(srfi 132)` `(srfi 158)` |
| scm's own | `(scm test)` `(scm hash-table)` `(scm numvector)` `(scm linalg)` `(scm noise)` |

Each one is written from its specification rather than copied from a
reference implementation, and tested on both engines, so every procedure is
one this project can actually change. Where a specification is genuinely
silent, the reference implementation is read to settle the question instead
of guessed at, and [docs/srfi.md](docs/srfi.md) names every place that
happened.

Two of the libraries are not standards at all. `(scm linalg)` provides
vec3, vec4, mat4 and quaternions; `(scm noise)` provides value, Perlin,
simplex and Worley noise with fbm, turbulence and ridged sums. Both exist
because no SRFI covers either and a renderer needs both. Every procedure in
`(scm noise)` is leaf-compilable, and a test asserts it by name.

Your own libraries are found as `acme/util.sld` on `.` or
`SCM_LIBRARY_PATH`; see [docs/libraries.md](docs/libraries.md).

```scheme
(define-library (acme util)
  (export twice)
  (import (scheme base))
  (begin (define (twice x) (* 2 x))))

(import (prefix (acme util) u:))
(u:twice 21)
```

## The Rust API

    cargo doc --open -p schemers

**The package is `schemers`; the library it builds is `scm`.** The name
`scm` was already taken on crates.io by an unrelated source-control crate,
and renaming the library to match would have churned the repository, the
binary and every `use` in it for nobody's benefit. So a dependent names one
and writes the other:

    [dependencies]
    schemers = "0.0.1"

```rust
use scm::{Interp, Val};
```

It is not published yet. Until then, take it as a git dependency:

    schemers = { git = "https://github.com/k2msmith-gfx/scm" }

`scm::embed` is the surface with a stability obligation attached. The other
modules are `pub` only because the CLI, the conformance runner and the LLVM
crate are built on them; `embed` is all a host should need.

Every Rust snippet below is also `scm/tests/readme.rs`, so `cargo test`
fails if this section goes stale. The prose claims next to the snippets are
checked there too, since those are the sentences most likely to rot
unnoticed.

### The five types

| type | what it is |
|---|---|
| `Interp` | the only thing a host constructs; owns one VM and its heap |
| `Rooted` | a handle that keeps a value alive and in place, released on `Drop` |
| `Val` | the host's view of a value, containing no internal references |
| `Proc` | a Scheme procedure the host can hold and call later; clones share one root |
| `Host` | what a re-entrant host function receives, so it can call back into Scheme |

`Val` covers `Null` `Unspecified` `Bool` `Int` `BigInt` `Float` `Char`
`Str` `Sym` `Keyword` `List` `Pair` `Vector` `Bytes` `Proc`, plus `Opaque`
for anything with no host representation — a port, a parameter, a cycle.

A `BigInt` is decimal text: lossless in both directions, and it normalises,
so an integer that fits in a word is always `Int` no matter which side
built it.

A proper list is a `List` and an improper one is a `Pair` chain. That
distinction matters to a host, because an association list — the dictionary
of a language that has no dictionary type — then crosses as the data it
actually is, and the host can recognise `((:a . 1) (:b . 2))` without
asking Scheme to inspect it.

### Evaluating, and exchanging values

```rust
use scm::{Interp, Val};

let mut scm = Interp::new()?;

let r = scm.eval("(map (lambda (x) (* x x)) '(1 2 3))")?;
assert_eq!(scm.value(&r)?, Val::List(vec![Val::Int(1), Val::Int(4), Val::Int(9)]));
assert_eq!(scm.write(&r)?, "(1 4 9)");

// Build a value from the host side and hand it in.
let v = scm.make(&Val::Vector(vec![Val::Str("a".into()), Val::Keyword("at".into())]))?;
assert_eq!(scm.write(&v)?, "#(\"a\" :at)");
```

`eval_named` attaches a source name for backtraces. `display` is `display`
rather than `write`. `pretty` wraps to a given width. `eqv` compares two
handles. `backtrace` returns the frames of the last error, live, because
the frame stack is the system of record.

### Calling in both directions

```rust
// A plain host function.
scm.define_fn("clamp", |args| match args {
    [Val::Float(x)] => Ok(Val::Float(x.clamp(0.0, 1.0))),
    _ => Err("clamp: wants one real".into()),
})?;

// One that calls back into Scheme: it gets a `Host` and may re-enter.
scm.define_fn_with("twice-apply", |host, args| match args {
    [Val::Proc(f), Val::Int(n)] => {
        let once = host.call(f, &[Val::Int(*n)])?;
        host.call(f, &[once])
    }
    _ => Err("twice-apply: wants a procedure and an integer".into()),
})?;

// And a Scheme procedure held on the host side and called later.
let r = scm.eval("(lambda (x) (+ x 1))")?;
if let Val::Proc(f) = scm.value(&r)? {
    assert_eq!(scm.call(&f, &[Val::Int(41)])?, Val::Int(42));
}
```

An `Err(String)` returned by a host function becomes an ordinary Scheme
error that `guard` can catch, because the host's *code* failing is not the
same as the host saying stop. A panic inside one is caught too, and named.

### Bounding a script

| | |
|---|---|
| `set_fuel(Some(n))` | bound the work; returns `Err(OutOfFuel)` instead of hanging |
| `set_heap_cap(Some(bytes))` | bound the memory; returns `Err(OutOfMemory)` |
| `interrupt_handle()` | an `Arc<AtomicBool>` another thread can set |
| `eval_with_timeout(src, dur)` | bound wall-clock time |
| `is_poisoned()` | a previous call panicked, so the VM's state is unknown |

`Error` distinguishes `Syntax` from `Incomplete`, which is what an editor
needs in order to tell "that is wrong" from "keep typing". `Raised` carries
the raised object, rooted, so a host can inspect it rather than only read
its printed form.

### Editors

`scm::repl` is the protocol spoken by the terminal, the TCP socket and the
Emacs mode below: framing, completion, describe, interrupt, and a break
REPL where an uncaught error stops with the frames still live. A client
parses no Scheme at all — everything it knows comes from the server. See
[docs/repl.md](docs/repl.md), and **Emacs** below for the client built on
it.

## Running it

    cargo build --release
    ./target/release/scm                          # a REPL
    ./target/release/scm -e '(+ 1 2 3)'           # one expression
    ./target/release/scm scene.scm                # a file
    ./target/release/scm --fuel 10000 scene.scm   # bound a runaway
    ./target/release/scm --serve 127.0.0.1:7000   # the protocol over TCP
    ./target/release/scm build hello.scm          # -> ./hello, needs no toolchain

An unclosed paren continues the entry instead of reporting an error. The
terminal has line editing, history, completion, `,help`, and a break REPL
where you can inspect locals and `,return` a value.

`SCM_TIER=t0` selects the interpreter, `t1` the bytecode VM (the default,
6.7× the interpreter on the benchmark suite with identical results), and
`t2` the LLVM tier if the binary was built with it.

## Emacs

[`emacs/scm-mode.el`](emacs/README.md) is a SLIME-shaped mode for editing
`.scm` files: a REPL in a window, completion, describe, eldoc and
interrupt. Opening the first `.scm` file starts `scm --serve`, connects,
and shows the REPL. There is nothing to launch by hand.

| key | in a `.scm` buffer |
|---|---|
| `C-x C-e` | eval the sexp before point |
| `C-c C-c`, `C-M-x` | eval the top-level form around point |
| `C-c C-r` / `C-c C-k` | eval the region / the whole buffer |
| `C-c C-z` | go to the REPL |
| `C-c C-d` | describe the symbol at point: signature, kind, docstring, value |
| `C-c C-b` | interrupt a runaway — out of band, on a second connection |
| `TAB` | complete over the server's global names |

In the `*scm*` buffer, `RET` sends a form once the server says it is
complete, and inserts a newline when the server says more is coming. That
is the *incomplete* versus *invalid* distinction doing visible work: the
editor never guesses where a form ends, and it needs no Scheme parser to
find out. Errors arrive with their message and backtrace, and eldoc shows a
procedure's signature as you type.

Highlighting is `scheme-mode`'s, plus the two spellings it does not know:
`[…]` vectors, and the lambda list's `&opt`/`&named`/`&keys`. Keyword
highlighting is scheme-mode's own and covers `:at`, `#:at` and every shape
scm reads (`:tex-scale`, `:-x`, `:k?`). The mode's test asserts this rather
than assuming it, since a parent mode's rules are not this project's to
promise.

### Setting it up

Doom, in `packages.el` and `config.el`:

```elisp
(package! scm-mode :recipe (:local-repo "~/devel/scm/emacs" :files ("scm-mode.el")))

(use-package! scm-mode
  :mode "\\.scm\\'"
  :config
  (setq scm-program "~/devel/scm/target/release/scm"))
```

Then run `doom sync`. **Leave Doom's `:lang scheme` module off**, or geiser
will also claim `.scm` buffers. `M-x scm-repl` shows the REPL by hand and
`M-x scm-start` restarts the server. [emacs/README.md](emacs/README.md) has
the plain-Emacs setup and the remaining keys.

### It is tested, from batch Emacs

    cd emacs && SCM=$PWD/../target/release/scm emacs --batch -l test-scm-mode.el

That drives the real server rather than a mock: eval, output, framing,
completion, describe, an error with its backtrace, string escaping in both
directions, the REPL buffer, and an interrupt of a runaway. The mode is the
protocol's first client, so what it exercises is the protocol.

## The LLVM tiers

`scm-llvm` is deliberately not a default workspace member: a root
`cargo build` needs no LLVM, and `-p scm-llvm` without one fails loudly
rather than quietly building something lesser. It holds two things.

The **leaf compiler** takes a pure numeric procedure to a native function,
and refuses anything else with a message naming the offending call
([docs/s11-leaf.md](docs/s11-leaf.md)). A leaf can be recompiled while a
render is running.

The **general tier** compiles bytecode to native code at 1.3× the VM, which
[docs/s13-general-tier.md](docs/s13-general-tier.md) explains is the call
protocol's ceiling rather than the generated code's.

### What you need

**LLVM 21.1.x with its development files** — `llvm-config`, headers and
libraries, not just a `clang` binary. `llvm-sys` is pinned at `211`, so 21
is the only major version that will work, and it must be findable either on
`PATH` or through `LLVM_SYS_211_PREFIX`. Nothing else is required: the LLVM
crates are the only dependencies anywhere in this workspace.

    # nix (what this is developed against)
    export LLVM_SYS_211_PREFIX=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')

    # macOS, Homebrew — llvm@21 is 21.1.8, keg-only
    brew install llvm@21
    export LLVM_SYS_211_PREFIX=$(brew --prefix llvm@21)

    # Debian / Ubuntu, via apt.llvm.org
    sudo apt install llvm-21-dev
    export LLVM_SYS_211_PREFIX=/usr/lib/llvm-21

    cargo test -p scm-llvm                      # the leaf compiler
    cargo build --release -p scm-cli --features jit   # then SCM_TIER=t2 works

Only the nix route is exercised on the development machine. The other two
are the standard prefixes for an LLVM 21.1 development install, which is
all `llvm-sys` asks for.

**On NixOS the prefix alone is not enough.** `llvm-config --libs` asks for
`-lffi -lz -lxml2`, and those three are not on the default library search
path, so the link fails:

    rust-lld: error: unable to find library -lffi

Point the linker at them as well:

    export LIBRARY_PATH=$(nix build --no-link --print-out-paths 'nixpkgs#libffi^out')/lib:\
    $(nix build --no-link --print-out-paths 'nixpkgs#zlib^out')/lib:\
    $(nix build --no-link --print-out-paths 'nixpkgs#libxml2^out')/lib

The `^out` matters on all three, because all three are multi-output. Plain
`nixpkgs#libxml2` resolves to a `bin` output that carries no shared library
at all, and plain `nixpkgs#libffi` prints *two* paths, which puts a newline
in the middle of `LIBRARY_PATH` and quietly breaks the entry. On a Homebrew
or apt install the system linker finds all three already and
`LIBRARY_PATH` is unnecessary.

One more thing worth knowing: `cargo build -p scm-llvm` succeeding proves
nothing here. The crate is an rlib, so building it never invokes the linker
and finishes in a couple of seconds even with libffi, zlib and libxml2 all
unreachable. `cargo test -p scm-llvm` is the command that links, so it is
the one that tells you the toolchain is really in place.

Without any of them you get this, which is `llvm-sys` talking rather than
scm:

    error: No suitable version of LLVM was found system-wide or pointed
           to by LLVM_SYS_211_PREFIX.

And one last trap: if that error persists while the prefix *is* set and
`llvm-config` works, the build script's not-found verdict has been cached,
and `rerun-if-env-changed` sees no change because the variable's value is
the same and only the store path behind it moved. Fix it with
`cargo clean -p llvm-sys` — and note that `clean -p` only cleans the
profile you name, so do `--release` separately.

## Testing

    cargo test                                                        # ~370 tests
    cargo run --release -p scm-conform -- scm-conform/suites/r7rs-tests.scm out.md
    cargo run --release -p scm-conform --bin scm-fuzz -- --seed 42 --count 2000
    cargo run --release -p scm-conform --bin soak -- --minutes 60
    cd emacs && SCM=$PWD/../target/release/scm emacs --batch -l test-scm-mode.el

The **soak** tests duration rather than intensity. The GC tests collect at
every safepoint, which finds a collector bug immediately; the soak instead
runs rounds of ordinary allocating work — closures over mutable state, a
cycle made unreachable, a mutated reachable structure, string ports,
records, hash tables, deep recursion, an escape continuation, bignums — and
watches whether the live set and the heap's high-water mark hold still.
Over **two million rounds** both were bit-identical: 6,223 live objects and
11,081 slots, start to finish. A leak of one cell per round would be
unmissable, and so would a non-moving heap fragmenting.

The **differential fuzzer** is the main instrument. Random programs run
under every engine, and the engines must agree on value, output and error
message. It generates libraries, imports under every modifier, bignum
arithmetic, records, macros and escape continuations.
[docs/fuzz.md](docs/fuzz.md) records what it has found — including a
session where the generator emitted libraries and imported them but almost
never *called* them, which is why it now asserts coverage of its own
output.

## Benchmarks

The corpus is **ecraven's `r7rs-benchmarks`**, cloned rather than vendored.
It is used as two separate instruments that answer different questions.

    scm-conform/bench/run.py --fetch        # clone the suite
    cargo test -p scm-conform --test benchmarks   # the language gate
    scm-conform/bench/run.py --small        # timings at reduced sizes

**As a language gate**, at reduced sizes, inside `cargo test`: all **53**
programs in the declared subset run and return the recorded answers, under
both engines, with **zero** compiler declines. That is a correctness
instrument, and it is the one that gates a commit.

**As a performance instrument**, at the suite's published sizes with a
120-second cap per program:

| | T0 (interpreter) | T1 (bytecode) |
|---|---|---|
| finished within 120 s | 3 | **33** |
| timed out | 41 | 20 |
| outside the subset | 4 | 4 |

T1 is **6.7×** T0 across the reduced suite (53.7 s → 8.0 s), 5.0× on
`fib 30`, and 17× on `nboyer`. The outliers are the ones you would expect:
`sum1` (1.0×) spends its time reading a file, which is the reader's time,
and `equal` (1.2×) compares in Rust either way.

Against **Chibi-Scheme's published times**, for the 29 programs that finish
and have a Chibi figure, the geometric mean of T1 ÷ Chibi is **0.6×** — so
T1 is somewhat ahead of Chibi, which is where a bytecode interpreter over
boxed values should be. (T0 measured 13× slower than Chibi on `fib`.)
Details are in [docs/s12-bytecode.md](docs/s12-bytecode.md).

The leaf compiler's numbers are not in this class, because it is not
competing in it: **138×–406×** over T1 on noise and vector kernels, and
3.3 ns for a bilinear image tap against 935 ns interpreted. It earns that
by dropping everything a Scheme normally carries.

## Status and platform

Stages S0 through S19 are done, apart from two entries that are honestly
something else. Done: the reader, the interpreter, the GC, the lambda list,
`syntax-rules`, the embedding API, the REPL protocol, the rest of
R7RS-small, the boot budget, the leaf compiler, live recompilation, the
bytecode tier, standalone binaries, thirteen SRFI tranches, linear algebra,
noise, unbounded exact integers, and a fuzzer that generates libraries.

The two exceptions, because a status list that rounds up is not worth
reading. **S6 is deferred** to the ray port: it planned to convert ray's
scene corpus and run it against stub verbs, and that work turned out to
belong on the other side of the bridge. **S13**, the general LLVM tier, is
recorded as *mechanism done, decision open* — it works and agrees with the
other engines everywhere, at 1.3× the bytecode VM, and whether to spend the
effort on the next multiple is a separate decision.

[docs/plan.md](docs/plan.md) §9 has the table, with each row's exit
criterion and whether it was met.

Development targets **aarch64 macOS first**. x86-64 Linux is sequenced
later, not dropped. Rust 1.85 or later.

## License

MIT or Apache-2.0, at your option.
