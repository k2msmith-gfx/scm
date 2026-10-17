# scm — guidance for coding agents

scm is an **embeddable R7RS-small Scheme in Rust with no dependencies**, and
the scripting language for the sibling `ray` project (a Rust raytracer and
3D modeller that currently embeds Janet). Three engines run every program
and must agree on all of it; a fourth compiler takes a restricted pure
subset to native code that renderer worker threads can call.

Read `README.md` first for what it is and the host API. This file is the
part a coding agent needs: the gates, the house rules, and the decisions
not to reopen.

## Start here

- **`docs/subset.md`** — **the normative statement of what the language
  is.** Read it before answering "does scm support X", and if you change
  what the language does, change it there. `scm/tests/subset.rs` checks
  every claim in it, so the document cannot quietly rot.
- **`docs/plan.md`** — the argument and the staging. §9 is the stage table:
  every stage, its size, its exit criterion, and whether it was met. If you
  are about to do a stage's worth of work, it gets a row.
- **`docs/<stage>.md`** — one record per stage. `conformance.md`,
  `fuzz.md`, `srfi.md`, `libraries.md`, `repl.md`, `s11-leaf.md`,
  `s12-bytecode.md`, `s13-general-tier.md`, `numeric-tower.md`,
  `linalg.md`, `noise.md`, `vs-steel.md`, `leaf-survey.md`,
  `scene-language-survey.md`.
- **`docs/scene-language-survey.md`** — what ray's 219 scene files actually
  require of the language, measured rather than guessed. **Scope work from
  this, not from the SRFI catalogue.**
- **`docs/port-plan.md`** — Track B: ray replacing Janet with scm, the plan
  for the work on ray's side. It lives here, not there, because every stage
  in it is gated on this repository. A bare path in it is this repo; a `ray/`
  prefix is the other. ray keeps a pointer at `docs/scm-port-plan.md`.

## Build / run

    cargo build --release                  # no LLVM needed, ever, by default
    ./target/release/scm                   # REPL
    SCM_TIER=t0|t1|t2 ./target/release/scm # pick an engine; t1 is default

The LLVM crate is **not a default workspace member** and needs a prefix:

    export LLVM_SYS_211_PREFIX=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')
    cargo test -p scm-llvm

Any **LLVM 21.1.x with development files** will do -- `llvm-sys` is pinned
at `211`, so only that major version. `brew --prefix llvm@21` (21.1.8) and
`/usr/lib/llvm-21` from apt.llvm.org are the other two prefixes; the README
lists all three.

**On NixOS the prefix is not enough**, because `llvm-config --libs` emits
`-lffi -lz -lxml2` and those are not on the default search path. Linking
fails at the *test* step with `rust-lld: error: unable to find library
-lffi`. Add them:

    export LIBRARY_PATH=$(nix build --no-link --print-out-paths 'nixpkgs#libffi^out')/lib:\
    $(nix build --no-link --print-out-paths 'nixpkgs#zlib^out')/lib:\
    $(nix build --no-link --print-out-paths 'nixpkgs#libxml2^out')/lib

The **`^out` on each is required**: all three are multi-output. Without
it `libxml2` resolves to its `bin` output, which has no `.so` at all, and
`libffi` prints *two* paths (`-man` as well), so the shell folds a newline
into `LIBRARY_PATH` and the entry silently stops resolving. Elsewhere the
system linker already finds all three and `LIBRARY_PATH` is unnecessary.

**`cargo build -p scm-llvm` is not evidence that LLVM works.** The crate is
an rlib, so a build never links and succeeds in seconds with libffi, zlib
and libxml2 all missing. Only `cargo test -p scm-llvm` links. This is the
"gate that passes suspiciously fast" rule in its LLVM form.

If `llvm-sys` says "No suitable version of LLVM was found" while the prefix
is set and `llvm-config` works: its not-found verdict is **cached** and
`rerun-if-env-changed` sees no change, because the variable's value is the
same and only the nix store path came back. Fix with
`cargo clean -p llvm-sys` — and note `clean -p` only cleans the profile you
name, so `--release` separately, and each feature set caches separately.

**Editing a `.sld` needs a rebuild**, because libraries are embedded with
`include_str!`. Set `SCM_LIBRARY_PATH=scm/lib` to let the on-disk copy win
instead, which is the loop for developing one.

## The gates

Nothing is done until these pass, and **commit only on green**:

    cargo test                             # ~343 tests
    cargo clippy --all-targets             # kept at ZERO warnings
    cargo run --release -p scm-conform -- scm-conform/suites/r7rs-tests.scm out.md
    cargo run --release -p scm-conform --bin scm-fuzz -- --seed 42 --count 2000
    cargo test -p scm-conform --test benchmarks
    cargo test -p scm-cli --test boot
    LLVM_SYS_211_PREFIX=… cargo test -p scm-llvm
    cd emacs && SCM=$PWD/../target/release/scm emacs --batch -l test-scm-mode.el
    cargo run --release -p scm-conform --bin soak -- --minutes 60

The **soak** is for duration rather than intensity, which is what the GC
tests cannot give: it watches whether the live set and the slot high-water
hold still over hundreds of thousands of rounds. `cargo test` runs a
bounded version; the binary is the long one.

The Emacs check drives the **real server** from batch Emacs rather than a
mock, so it is a protocol test as much as an editor one. `SCM` must be an
**absolute** path; a relative one fails the executable check.

The **differential fuzzer** is the main instrument: random programs under
every engine must agree on value, output and error message. It generates
libraries, imports under every modifier, bignums, records and escape
continuations. Conformance currently stands at **1112 passed / 71 failed**;
if you move it, say which direction and why in `docs/conformance.md`.

A gate that passes suspiciously fast is not passing. The benchmark gate
once ran in 0.00 s and "passed" because `target/r7rs-benchmarks` was a
broken symlink into a dead scratch directory. Check a gate's inputs exist
before trusting it.

## House rules

- **Every library is written against its specification**, not copied from a
  reference implementation, so every procedure is one this project can test
  and change. Where a spec is genuinely silent, read the reference to settle
  it and **say in `docs/srfi.md` that you did**.
- **Fetch the specification; do not recall it.** Two focused fetches —
  surface first, then exact wording for the hard procedures — is the
  pattern. Reference implementations come from
  `gh api repos/scheme-requests-for-implementation/srfi-NNN/contents/…`
  (srfi.schemers.org 404s on the impl paths).
- **One test program per library** in `scm/tests/srfi/`, run on **both
  tiers** from `scm/tests/srfi.rs`. Its value is its list of failures, so
  `()` is a pass whether a person reads it or `cargo test` asserts it.
- **The `scm` crate has no dependencies.** Standing constraint, not an
  accident: a host takes this crate and nothing else. It is why the bignums
  are hand-written and why `embed::Val` carries a wide exact integer as
  text. Do not add one without raising it first.
- **The package publishes as `schemers`; the library is `scm`.** `scm` was
  taken on crates.io. A dependent names `schemers` in its manifest and
  writes `use scm::…`.
- `embed::Val` and `embed::Error` are `#[non_exhaustive]`. Keep them so.
- Check a `.sld` for paren balance before building: an imbalance reports
  only as `read error: incomplete datum`, **with no line number**. A short
  scanner that tracks depth (skipping strings, `;` comments and `#\`
  literals) and flags every `(define ` not at depth 3 finds it at once.
- `grep -c pattern` **exits 1 when the count is zero**, so
  `… | grep -c warning && git commit …` silently skips the commit. Use
  `grep pattern | wc -l`, or do not chain.

## Decisions — do not reopen without new evidence

Each is argued in the doc named; they are decisions, not gaps.

- **No exact rationals.** `(/ 8 3)` is the flonum `2.666…`, matching the
  language scm replaces, so a converted scene computes what the original
  computed. A sibling implementation's `8/3` silently turned a numerical
  benchmark into exact-rational arithmetic with no diagnostic.
  `docs/numeric-tower.md` §5.
- **Exact integers are unbounded** (`exact-closed` is claimed), with a
  2^20-bit ceiling: unbounded must not mean "may exhaust the host".
- **Continuations are escape-only.** A real deviation, and it costs things —
  `(srfi 158)`'s coroutine generator runs eagerly and raises past a cap
  rather than hanging. Do not write anything needing re-entry.
- **`:at` is a keyword**, not a symbol. 87 of ray's 218 verbs depend on the
  convention. `ReaderOptions::colon_keywords` turns it off for conformance.
- **Importing copies bindings**, so a library that `set!`s its own exported
  variable afterwards does not reach the importer. `docs/libraries.md`.
- **Char-sets are Latin-1** and the **bitwise operations are 64 bits wide**;
  both refuse rather than truncate. For Unicode-correct matching pass a
  *predicate* — every `(srfi 13)`/`(srfi 130)` procedure takes one, and
  `char-alphabetic?` and friends are correct over the whole range.
- **T2 is done-as-is**: 1.3× T1, opt-in, agrees everywhere. The next
  multiple is a separate decision.
- **`(srfi 115)` regex is skipped** and `(srfi 152)` is not planned: measured
  demand in ray is ~5 pattern-shaped call sites, and `string/find` there is
  *literal* substring search, which `(srfi 13)` already serves.

## Traps that have cost real time

- **`f64` round-trips hide exact-integer bugs.** Three shipped briefly:
  `odd?` went through a double, so `2^64+1` read as `2^64` and lost the low
  bit; `equal-hash` hashed a bignum by heap *address*, so two `equal?`
  values hashed differently; `eqv?` compared addresses. When a number can
  live in the heap, audit every predicate and hash for "does it convert, or
  read the value?", and assert `eqv?`/`equal?`/`=`/`equal-hash` agree.
- **One integer, one representation.** Every exact-integer result
  normalises back to a fixnum when it fits. Skip it and `eqv?`,
  `equal-hash` and every `match` on `Value` disagree with themselves.
- **Three fold orders ship here and they differ**: `(srfi 1)` `fold` is
  `(f elem acc)`; `(srfi 133)` `vector-fold` is `(kons state elt)`;
  `(srfi 14)`/`(srfi 13)`/`(srfi 130)` are `(kons char state)`. Pin each
  with a non-commutative `kons`, never a sum.
- **Colon-initial names read as keywords**, so they cannot be macro names,
  procedure names or `syntax-rules` literals. Cost `(srfi 42)` its
  `:generator-proc`.
- **A library body's definitions are mutually recursive**, so a library
  cannot wrap a native procedure of the same name it exports.
- **When an old SRFI's name collides with R7RS, make it a superset, not a
  replacement.** `(srfi 13)`'s `string-map` takes one string plus a range
  where R7RS takes N strings; both shapes work, dispatched on argument type,
  so importing cannot break plain R7RS code.
- **A spec's "special case" may be an identity.** SRFI 13 says
  `string-compare` hands `proc=` the index `end1` "in the case of equality";
  implemented as a separate branch that is wrong, because the real rule is
  `start1 + match` always. A test caught it; reading did not.
- **When you add generation to the fuzzer, assert the coverage.** A version
  that emitted libraries and imported them but almost never *called* them
  looked finished and tested nearly nothing.

## The two tracks

**Track A** is scm itself and is where nearly all work has happened.
**Track B** is ray adopting scm — the verb bridge, the corpus rendering
identically, compiled texels, and moving ~19,000 lines of prelude and
modeller. Track B is **gated on ray's own API/tool refactor** landing on its
side. Publishing to crates.io is gated on Track B too: the port is the
testing that should come before anyone outside depends on this.

Do not determine the language's needs from ray's current simple examples.
The goals, in order, are: replace Janet in ray/modeler, and distribute scm
as a general-purpose embeddable Scheme.

### How ray will consume this (verified 2026-10-16)

An out-of-workspace crate depending on both by path was built and run, and
it did everything the port needs: keyword-argument verbs, a verb calling
back into Scheme through `define_fn_with`, a `Proc` held across 20,000
evaluations and a collection, the embedded libraries reachable from the
host, fuel bounding a runaway, and a leaf compiled with the `Vm` then
dropped and called from `rayon` workers.

    schemers = { path = "../scm/scm" }        # then `use scm::…`
    scm-llvm = { path = "../scm/scm-llvm" }   # only for compiled texels

Three things the port has to decide rather than inherit:

- **`scm-llvm` needs LLVM 21 at build time, always.** Depending on it
  unconditionally makes every ray build need a toolchain. Put it behind a
  ray-side feature, exactly as `scm-cli` does.
- **A git dependency needs the repo reachable.** While `k2msmith-gfx/scm`
  is private, a path dependency across the two working copies is the
  frictionless route.
- Ray is on stable / edition 2024, which satisfies scm's
  `rust-version = "1.85"`. No toolchain pin is needed on either side.
