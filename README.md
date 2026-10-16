# scm

An **embeddable R7RS-small Scheme in Rust, with no dependencies.** Three
engines run every program and must agree on all of it: an AST interpreter
kept as the semantic oracle, a bytecode VM as the default, and an optional
LLVM tier. A fourth compiler takes a restricted pure subset — a texture, a
noise field — to native code that is VM-free, allocation-free and
`Send + Sync`.

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

scm exists to be **hosted inside a Rust program** that needs a scripting
language it can trust, and specifically inside a renderer: it is the
scripting language for [ray](https://github.com/k2msmith-gfx/ray), a
procedural 3D modelling and rendering system. That origin explains the
shape, and the focus is narrower than "a Scheme":

**A runaway script cannot take the host down.** Not a feature — a
requirement, and the reason every entry point returns `Result`. Fuel
bounds work, a heap cap bounds memory, an interrupt handle stops a script
from another thread, and a timeout bounds wall-clock time. A caught panic
*poisons* the interpreter rather than pretending the VM is still
intelligible. Hostile-program testing found four ways a script could abort
its process; each one is now an ordinary Scheme error that `guard` can
catch.

**A host cannot be handed a dangling value.** The embedding API exists
because its absence was measured: a host held the result of an eval, ran
more script, and the value it was holding silently became a different
object. `Rooted` keeps a value alive and un-moved; `Val` is the host's
view with no internal reference in it.

**Authored code can become native code.** The capability that justifies
the project: a renderer's worker threads cannot call into a thread-local
VM, so an authored implicit field could not trace at all. The leaf
compiler turns a pure numeric procedure into a plain function those
threads can call — measured at **138× to 406×** over the bytecode VM on
noise and vector kernels, and **3.3 ns** for a bilinear tap into a 512²
image against 935 ns interpreted.

**Startup is a design constraint.** The whole process — load, prelude,
evaluate, exit — is budgeted at 10 ms and measured at **2.9 ms**, of which
1.8 ms is the binary's own load. A test fails if it regresses.

**No dependencies.** The crate takes nothing from crates.io, which is why
the bignums are hand-written and why a wide exact integer crosses the host
boundary as text rather than exposing an internal type. The default
feature set is empty, so `cargo build` needs no LLVM.

What it is *not* for: the general JIT is explicitly not about making
scripting faster. There was no hot spot to speed up. It exists so that
authored code can leave the VM entirely.

## Compatibility

**[docs/subset.md](docs/subset.md) is the normative statement** of what
the language is — the baseline, the feature identifiers, the four
deviations, the resource limits, and what is deliberately not promised.
Every claim in it is checked by `scm/tests/subset.rs`, so a sentence that
stops being true fails the build. The rest of this section is the summary.

**R7RS-small, qualified.** The conformance suite is Chibi-Scheme's
`r7rs-tests.scm` — written by other people, against the spec — vendored
unedited and run unchanged:

    cargo run --release -p scm-conform -- scm-conform/suites/r7rs-tests.scm out.md

**1112 passed, 71 failed** of 1183 assertions reached. The first baseline
was 45.9%. A percentage with an unexplained remainder is a marketing
number, so [docs/conformance.md](docs/conformance.md) accounts for the
remainder: every numeric failure left is the one declined optional feature
below, and the rest are `syntax-rules` hygiene corners, each named.

`(features)` is the contract, and it is kept honest — a feature identifier
is a promise, so each is claimed only when it is true:

| claimed | `r7rs` `ieee-float` `exact-closed` `scm`, plus the platform's |
|---|---|
| **not claimed** | `ratios` `exact-complex` `full-unicode` |

### The deliberate deviations

Four, and each is a decision with a reason rather than a gap:

- **Continuations are escape-only.** `call/cc` works for escaping — early
  return, generators that do not resume, exception-like control — and a
  re-entrant continuation raises instead of misbehaving. This is a real
  deviation from R7RS: it cost `(srfi 158)` its lazy coroutine generator,
  which now runs eagerly and raises past a cap rather than hanging.
- **No exact rationals**, and this one is load-bearing rather than
  unfinished. `(/ 8 3)` is the flonum `2.666…`, which matches the host
  language scm replaces, so a converted scene computes what the original
  computed. A sibling implementation's `8/3` silently turned a numerical
  benchmark into exact-rational arithmetic with no diagnostic; that is the
  measured reason not to want the alternative. See
  [docs/numeric-tower.md](docs/numeric-tower.md).
- **`:at` reads as a keyword**, not as the symbol R7RS says it is.
  Keywords are a first-class datatype because 87 of ray's 218 verbs depend
  on the convention. `ReaderOptions::colon_keywords` turns it off, which
  is how the conformance suite runs.
- **Importing copies bindings.** A library is an environment frame and an
  import copies its exports, so a library that `set!`s its own exported
  variable afterwards does not reach an importer. R7RS makes `set!` on an
  imported binding an error and no library in the wild does the other
  thing. Both engines implement the same simplification, which the
  differential fuzzer checks.

Exact integers *are* unbounded — `(* 9223372036854775807 2)` is exact —
with a ceiling of 2^20 bits, because "unbounded" must not mean "allowed to
exhaust the host". Strings are `Vec<char>`, so `string-ref` is O(1), and
`full-unicode` is unclaimed because there is no case folding or Unicode
digit value. Char-sets are Latin-1; for Unicode-correct matching the
string procedures take a predicate, and `char-alphabetic?` and friends are
correct over the whole range.

## What Scheme code it will run

The practical question, answered concretely rather than by pointing at a
standard.

**Other people's test suites run.** Chibi-Scheme's `r7rs-tests.scm`,
written against the spec by someone with no interest in scm, runs
unedited: **1112 assertions pass**. So does ecraven's `r7rs-benchmarks`
corpus — 53 of its programs are inside the declared subset and **all 53
produce the recorded answers** under both engines. That is the strongest
claim available: code written by strangers, not examples written here.

**All the `(scheme …)` libraries import**, because the standard bindings
*are* the prelude, so importing one is a no-op:

    (import (scheme base) (scheme write) (scheme char) (scheme inexact)
            (scheme lazy) (scheme case-lambda) (scheme eval) (scheme read)
            (scheme file) (scheme time) (scheme process-context) (scheme cxr))

`define-record-type`, `syntax-rules` with nested ellipses, `guard` and
`raise`, `dynamic-wind`, `parameterize`, `delay`/`force`/`make-promise`,
`case-lambda`, string ports, bytevectors, `eval` with
`interaction-environment` — all present and exercised by the suite above.

**One sharp edge to know about**: an import of *any* `(scheme …)` name
succeeds, including one scm does not implement. `(import (scheme complex))`
is accepted and then `make-rectangular` is an unbound variable. You find
the gap at the call site, not at the import.

### What will not run

| | |
|---|---|
| a **re-entrant** continuation | raises. Escaping works; resuming a dead extent does not. Generators that resume, `amb`, coroutines |
| exact rationals | `1/2` is a read error, not a number. `(/ 8 3)` is a flonum |
| complex numbers | `make-rectangular` and friends are absent |
| `:at` as a *symbol* | it reads as a keyword unless `colon_keywords` is off |
| Unicode case *folding* in `string-ci=?` | `full-unicode` is unclaimed; `char-upcase` declines a mapping that is not a single character, as R7RS requires |
| bitwise ops on a bignum | they are 64 bits wide and say so rather than truncating |

Everything on that list fails *loudly*. The design rule throughout is that
a thing scm cannot do raises rather than returning a plausible wrong
answer — which is why `(/ 8 3)` being a flonum is a decision and not a
silent approximation.

## Libraries built in

**36 libraries are embedded in the binary**, so `scm` needs no library
directory beside it and a built standalone binary carries them too. Twenty
distinct SRFIs:

| | |
|---|---|
| lists, control | `(srfi 1)` `(srfi 2)` `(srfi 8)` `(srfi 26)` `(srfi 111)` |
| strings, chars | `(srfi 13)` `(srfi 14)` `(srfi 130)` |
| vectors, arrays | `(srfi 4)` `(srfi 63)` `(srfi 133)` `(srfi 160 base)` + 10 element types |
| tables, comparators | `(srfi 69)` `(srfi 125)` `(srfi 128)` |
| numbers, bits | `(srfi 27)` `(srfi 151)` `(srfi 194)` |
| iteration | `(srfi 42)` `(srfi 132)` `(srfi 158)` |
| scm's own | `(scm test)` `(scm hash-table)` `(scm numvector)` `(scm linalg)` `(scm noise)` |

Every one is written against its specification rather than copied from a
reference implementation, and tested on both engines — so each is a
procedure this project can change. Where a specification is genuinely
silent, the reference implementation is read to settle the question rather
than guessed at, and [docs/srfi.md](docs/srfi.md) names each place that
happened.

Two of the libraries are not standards at all. `(scm linalg)` is
vec3/vec4/mat4/quaternions and `(scm noise)` is value, Perlin, simplex and
Worley noise with fbm, turbulence and ridged sums — written because **no
SRFI covers either** and a renderer needs both. Every procedure in `(scm
noise)` is leaf-compilable and a test asserts it by name.

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
`scm` was taken on crates.io by an unrelated source-control crate, and
renaming the library to match would have churned the repository, the
binary and every `use` in it to no one's benefit. So a dependent names one
and writes the other:

    [dependencies]
    schemers = "0.0.1"

```rust
use scm::{Interp, Val};
```

Not published yet; take it as a git dependency meanwhile:

    schemers = { git = "https://github.com/k2msmith-gfx/scm" }

`scm::embed` is the surface with the stability obligation. The other
modules are `pub` because the CLI, the conformance runner and the LLVM
crate are built on them, but `embed` is the one a host should need.

Every Rust snippet below is `scm/tests/readme.rs`, so `cargo test` fails if
this section goes stale — along with the prose claims beside them, which
are the sentences most likely to rot unnoticed.

### The five types

| type | what it is |
|---|---|
| `Interp` | the only thing a host constructs; owns one VM and its heap |
| `Rooted` | a handle that keeps a value alive and un-moved, released on `Drop` |
| `Val` | the host's view of a value, with no internal reference in it |
| `Proc` | a Scheme procedure a host can hold and call later; clones share one root |
| `Host` | what a re-entrant host function gets, so it can call back into Scheme |

`Val` covers `Null` `Unspecified` `Bool` `Int` `BigInt` `Float` `Char`
`Str` `Sym` `Keyword` `List` `Vector` `Bytes` `Proc`, and `Opaque` for
anything with no host representation — a port, a parameter, an improper
list. A `BigInt` is decimal text: lossless both ways, and it normalises,
so an integer that fits a word is always `Int` whichever side built it.

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

`eval_named` attributes a source name for backtraces, `display` is
`display` rather than `write`, `pretty` wraps to a width, `eqv` compares
two handles, and `backtrace` returns the frames of the last error — live,
because the frame stack is the system of record.

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

An `Err(String)` from a host function becomes an ordinary Scheme error,
catchable by `guard`, because the host's *code* failing is not the host
saying stop. A panic inside is caught too and named.

### Bounding a script

| | |
|---|---|
| `set_fuel(Some(n))` | bound the work; `Err(OutOfFuel)` rather than a hang |
| `set_heap_cap(Some(bytes))` | bound the memory; `Err(OutOfMemory)` |
| `interrupt_handle()` | an `Arc<AtomicBool>` another thread can set |
| `eval_with_timeout(src, dur)` | bound wall-clock time |
| `is_poisoned()` | a previous call panicked, so the VM's state is unknown |

`Error` distinguishes `Syntax` from `Incomplete`, which is what an editor
needs to tell "that is wrong" from "keep typing", and `Raised` carries the
raised object rooted so a host can inspect it rather than only read its
printed form.

### Editors

`scm::repl` is the protocol the terminal, the TCP socket and the Emacs mode
below all speak: framing, completion, describe, interrupt, and a break REPL
where an uncaught error stops with the frames live. A client parses no
Scheme — everything it knows comes from the server. See
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

An unclosed paren continues the entry rather than reporting an error. The
terminal has line editing, history, completion, `,help`, and a break REPL
you can inspect locals in and `,return` a value from.

`SCM_TIER=t0` selects the interpreter, `t1` the bytecode VM (the default,
6.7× the interpreter on the benchmark suite with identical results), and
`t2` the LLVM tier if the binary was built with it.

## Emacs

[`emacs/scm-mode.el`](emacs/README.md) is a SLIME-shaped mode for editing
`.scm` files: a REPL in a window, completion, describe, eldoc and
interrupt. Opening the first `.scm` file starts `scm --serve`, connects,
and shows the REPL — there is nothing to launch by hand.

| key | in a `.scm` buffer |
|---|---|
| `C-x C-e` | eval the sexp before point |
| `C-c C-c`, `C-M-x` | eval the top-level form around point |
| `C-c C-r` / `C-c C-k` | eval the region / the whole buffer |
| `C-c C-z` | go to the REPL |
| `C-c C-d` | describe the symbol at point: signature, kind, docstring, value |
| `C-c C-b` | interrupt a runaway — out of band, on a second connection |
| `TAB` | complete over the server's global names |

In the `*scm*` buffer, `RET` sends a form **once the server says it is
complete** and inserts a newline when it says more is coming. That is S1's
*incomplete* versus *invalid* distinction doing visible work: the editor
does not guess where a form ends, and it does not need a Scheme parser to
find out. Errors arrive with their message and backtrace; eldoc shows a
procedure's signature as you type.

Highlighting is `scheme-mode`'s plus scm's own spellings — `[…]` vectors,
`&opt`/`&named`/`&keys`, and `:keywords`.

### Setting it up

Doom, in `packages.el` and `config.el`:

```elisp
(package! scm-mode :recipe (:local-repo "~/devel/scm/emacs" :files ("scm-mode.el")))

(use-package! scm-mode
  :mode "\\.scm\\'"
  :config
  (setq scm-program "~/devel/scm/target/release/scm"))
```

Then `doom sync`. **Leave Doom's `:lang scheme` module off**, or geiser will
also claim `.scm` buffers. `M-x scm-repl` shows the REPL by hand and
`M-x scm-start` restarts the server. [emacs/README.md](emacs/README.md) has
the plain-Emacs setup and the rest of the keys.

### It is tested, from batch Emacs

    cd emacs && SCM=$PWD/../target/release/scm emacs --batch -l test-scm-mode.el

That drives the **real server** rather than a mock: eval, output, framing,
completion, describe, an error with its backtrace, string escaping both
ways, the REPL buffer, and an interrupt of a runaway. The mode is the first
client of the protocol, so what it exercises is the protocol.

## The LLVM tiers

`scm-llvm` is **not a default workspace member**: a root `cargo build`
needs no LLVM, and `-p scm-llvm` without one fails loudly rather than
quietly building something lesser. It holds two things. The **leaf
compiler** takes a pure numeric procedure to a native function and refuses
anything else with a message naming the call
([docs/s11-leaf.md](docs/s11-leaf.md)); a leaf can be recompiled under a
running render. The **general tier** compiles bytecode to native code at
1.3× the VM, which [docs/s13-general-tier.md](docs/s13-general-tier.md)
explains is the call protocol's ceiling rather than the code's.

### What you need

**LLVM 21.1.x with its development files** — `llvm-config`, headers and
libraries, not just a `clang` binary. `llvm-sys` is pinned at `211`, so 21
is the only major version that will do, and it must be findable either on
`PATH` or through `LLVM_SYS_211_PREFIX`. Nothing else: the LLVM crates are
the only dependencies anywhere in this workspace.

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

Only the nix route is exercised on the development machine; the other two
are the standard prefixes for an LLVM 21.1 development install, which is
all `llvm-sys` asks for.

Without any of them you get this, which is `llvm-sys` talking and not scm:

    error: No suitable version of LLVM was found system-wide or pointed
           to by LLVM_SYS_211_PREFIX.

And one trap worth knowing: if that error persists **while the prefix is
set and `llvm-config` works**, the build script's not-found verdict has
been cached and `rerun-if-env-changed` sees no change, because the
variable's value is the same and only the store path behind it moved. Fix
with `cargo clean -p llvm-sys` — and note `clean -p` only cleans the
profile you name, so `--release` separately.

## Testing

    cargo test                                                        # ~360 tests
    cargo run --release -p scm-conform -- scm-conform/suites/r7rs-tests.scm out.md
    cargo run --release -p scm-conform --bin scm-fuzz -- --seed 42 --count 2000
    cargo run --release -p scm-conform --bin soak -- --minutes 60
    cd emacs && SCM=$PWD/../target/release/scm emacs --batch -l test-scm-mode.el
    scm-conform/bench/vs-steel/run.py                                 # needs `steel` on PATH

The **soak** is the instrument for duration rather than intensity. The GC
tests collect at every safepoint, which finds a collector bug immediately;
the soak runs rounds of ordinary allocating work — closures over mutable
state, a cycle made unreachable, a mutated reachable structure, string
ports, records, hash tables, deep recursion, an escape continuation,
bignums — and watches whether the live set and the heap's high-water mark
hold still. Over **two million rounds** both were bit-identical: live 6,223
objects and 11,081 slots, start to finish. A leak of one cell per round
would be unmissable; so would a non-moving heap fragmenting.

The **differential fuzzer** is the main instrument: random programs under
every engine must agree on value, output and error message. It generates
libraries, imports under every modifier, bignum arithmetic, records,
macros and escape continuations. [docs/fuzz.md](docs/fuzz.md) records what
it found — including a session where the generator emitted libraries and
imported them but almost never *called* them, which is why it now asserts
coverage of its own output.

Against Steel on the ten benchmark programs both run: the bytecode VM is
0.91×, the LLVM tier 1.06×, startup 7× faster, escape continuations 2.6×
([docs/vs-steel.md](docs/vs-steel.md)).

## Benchmarks

The corpus is **ecraven's `r7rs-benchmarks`**, cloned rather than vendored,
and it is used as *two* instruments that answer different questions.

    scm-conform/bench/run.py --fetch        # clone the suite
    cargo test -p scm-conform --test benchmarks   # the language gate
    scm-conform/bench/run.py --small        # timings at reduced sizes

**As a language gate**, at reduced sizes, in `cargo test`: all **53**
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

T1 is **6.7×** T0 over the reduced suite as a whole (53.7 s → 8.0 s), 5.0×
on `fib 30`, 17× on `nboyer`. The outliers are the ones that should be:
`sum1` (1.0×) is reading a file, which is the reader's time, and `equal`
(1.2×) compares in Rust either way.

Against **Chibi-Scheme's published times** for the 29 programs that finish
and have a Chibi figure, the geometric mean of T1 ÷ Chibi is **0.6×** — so
T1 is somewhat ahead of Chibi, which is where a bytecode interpreter over
boxed values should be. (T0 measured 13× slower than Chibi on `fib`.)
Details in [docs/s12-bytecode.md](docs/s12-bytecode.md).

Against **Steel**, on ten programs both implementations run:

| | Steel ÷ T1 | Steel ÷ T2 |
|---|---|---|
| geometric mean, excluding startup | **0.91×** | **1.06×** |
| startup | 7.5× | 7.8× |
| an escape continuation per iteration | 2.6× | 2.7× |
| `lists` — map over 100k, 20 times, plus `assv` | 0.5× | 0.6× |

So: a little behind Steel on the bytecode tier, a little ahead with the
LLVM tier on, much faster to start, and much faster on escape
continuations. The honest weak spot is list-heavy allocation at 0.5×.
[docs/vs-steel.md](docs/vs-steel.md) has the per-program table and the
reading.

The numbers that are *not* in this class are the leaf compiler's, because
it is not competing in it: **138×–406×** over T1 on noise and vector
kernels, and 3.3 ns for a bilinear image tap against 935 ns interpreted.
It earns that by dropping everything a Scheme normally carries.

## Status and platform

Stages S0 through S19 are done, bar two entries that are honestly
something else: reader, interpreter, GC, the lambda list, `syntax-rules`,
the embedding API, the REPL protocol, the rest of R7RS-small, the boot
budget, the leaf compiler, live recompilation, the bytecode tier,
standalone binaries, thirteen SRFI tranches, linear algebra, noise,
unbounded exact integers, and a fuzzer that generates libraries.

The two exceptions, because a status list that rounds up is not worth
reading. **S6 is deferred** to the ray port: it was a plan to convert
ray's scene corpus and run it against stub verbs, and what it describes
turned out to belong on the other side of the bridge. **S13**, the general
LLVM tier, is recorded as *mechanism done, decision open* — it works and
agrees with the other engines everywhere, at 1.3× the bytecode VM, and
whether to spend the next multiple is a separate decision.

[docs/plan.md](docs/plan.md) §9 has the table, each row with its exit
criterion and whether it was met.

Development targets **aarch64 macOS first**; x86-64 Linux is sequenced
later, not dropped. Rust 1.85 or later.

## License

MIT or Apache-2.0, at your option.
