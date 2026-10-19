# S11 — the leaf compiler: results

2026-10-13, aarch64 macOS, LLVM 21.1.8 from nix, `scm-llvm/`.

```
LLVM=$(nix build --no-link --print-out-paths 'nixpkgs#llvmPackages_21.llvm.dev')
LLVM_SYS_211_PREFIX=$LLVM cargo test -p scm-llvm            # the gate
LLVM_SYS_211_PREFIX=$LLVM cargo run --release -p scm-llvm --example texel_cost
```

## Exit criterion, met

Plan §9: *a compiled field and texel function run concurrently from a rayon
pool and agree with T0; the restricted subset rejects a non-pure call with a
message naming why.*

- `scm-llvm/tests/leaf.rs` compiles ray's real texture stack — `hash2` →
  `fade` → `vnoise` → `fbm` → the colour blend — plus a signed-distance field
  (sphere smooth-unioned with a box), a table lookup through `modulo`,
  `cond`/`when`/`and`/`or`/`not`, nested loops, `quotient` and `expt`.
  **Fifteen scalar functions agree with T0 bit for bit on 600 samples**, and
  two texel functions on 128.
- A 256×256 texel grid computed from a rayon pool is identical to the serial
  result, after the `Vm` that compiled it has been dropped, and a handle
  cloned into another thread keeps the code alive after the original is
  gone.
- Eleven refusals, each naming the call: `` `display` is not a pure primitive
  (reached through `impure-io`) (in impure-reach)``.

```
compile                 20.05 ms      (LLJIT creation + O2, one leaf)
native                   76.8 ns/texel     fbm, 4 octaves
T0                    69273.8 ns/texel     (902x)
```

S5b measured 26 ns for a single `vnoise`; this is four of them plus the
loop, so the per-call cost is the same order. The 902× against T0 is not a
speedup claim (S5b said why: T0 is the oracle, not the engine); the number
that matters to ray is 77 ns against leaf-survey's 4,112 ns in Janet.

## What was decided, in the code

**Numbers are `f64` throughout.** Where T0 keeps an exact integer — a loop
counter, `(sqrt 4)`, `(/ 6 2)` — the compiled code holds the same value
inexactly, and the differential test compares numerically. The one place
this shows is at the boundary: T0's `modulo` and `vector-ref` *demand* an
exact integer, so a test that hands T0 `-7.0` and the compiled code `-7`
is comparing two different calls. The fixture spells those arguments as
integers for T0.

**Booleans exist only in test position.** The condition of an `if`, under
`and`/`or`/`not`, the numeric predicates, and `#t`/`#f`. That is where the
corpus has them. A boolean bound as a *value* is refused with a message —
except that `or` expands through a hygienic temporary, `(let ((t e)) (if t
t …))`, so a `let` in test position binds a boolean when its init looks
like one. The alternative, a second value type, would have been a type
inference pass for a case nothing in the corpus writes.

**Item (2), captured tables, is a snapshot with a bounds check.** A global
vector of numbers becomes a private LLVM constant array at compile time.
`vector-set!` on the global afterwards does not reach compiled code — that
is what "read-only snapshot" in §6 means, and S11a's recompile-on-redefine
is how it catches up. An index outside the table branches to a trap block
that returns NaN, because a leaf has no way to raise and must never read
outside its tables; a NaN pixel is visible, a wild read is not. A table
holding a non-number is refused naming the index. §6 called this "the
awkward one"; it was forty lines once the shape was named.

**Item (5), the 3-float return, is behind the target hook** §2.5 asked for.
`Vec3Abi::Registers` emits `{double, double, double}` by value — an AAPCS64
homogeneous aggregate in `d0`–`d2`, matching a `repr(C)` struct on the Rust
side, verified bit for bit here. `Vec3Abi::OutPointer` emits
`void f(Vec3*, …)` for everything else, because SysV x86-64 would return a
24-byte aggregate through memory anyway. Both paths exist; only one is
tested, and that is the honest state until Linux is caught up.

**One ORC LLJIT per compiled leaf**, behind an `Arc` in the `Compiled`
handle, which is `Send + Sync`. Cloning shares the code; the LLJIT is
disposed after the last clone, so a render worker holding a handle across a
pass cannot have its code pulled out from under it — the property S11a's
generations are built on. Forty LLJITs for forty textures costs 20 ms each
to create and has not been measured as a problem; resource trackers on one
JIT are the next step if it is.

**The analysis's list and the codegen's are the same list.** The S5b probe
accepted `cons`, `car`, `length` and more as pure and then could not emit
them, which gave two vocabularies of refusal. `PURE_PRIMS` is now exactly
what `codegen` can build, so an accepted procedure compiles.

**`map` is specialised at the call site** (item 4), before the head is
classified — S5b's sharpest finding, kept. `(map f (list a b c))`,
`(vector …)`, `[a b c]` and a quoted list all count as literal sequences; a
`lambda` is inlined once per element; anything else is refused as "unknown
length".

## What is not here

- **S11a**, live recompilation with a generation counter: the handle's
  lifetime is the prerequisite and it is done; the generation is ray's
  `live.rs` pattern bound to it.
- **x86-64**: the `OutPointer` path is written and unverified.
- **`scm-cli` integration**: nothing in the binary compiles a leaf yet. A
  `jit` feature on `scm-cli` pulling `scm-llvm` is the §5 "fails loudly"
  gate in its final form; today the gate is that `scm-llvm` is not a
  default workspace member, so a root `cargo build` needs no toolchain and
  `-p scm-llvm` without one fails in `llvm-sys`.
- **Integers as a type**: a leaf that needs 64-bit integer semantics —
  bit operations, exact arithmetic past 2^53 — is outside the subset.
  Nothing in the corpus does.

## S11a — live recompilation (2026-10-13)

`scm_llvm::Live`, `scm-llvm/src/live.rs`, tests in `tests/live.rs`.

Plan §3.2a's third consequence: redefine a texture while a render is
running and rayon workers hold a native function pointer. S11 gave the
handle its lifetime; this adds ray's `live.rs` pattern on top:

- **A generation**, bumped on every successful replacement. A pass calls
  `snapshot()` once at its start and gets the code *and* its generation;
  the clone keeps that code alive for the pass, whatever happens meanwhile,
  and a pass that wants to abandon early compares `generation()` between
  strips.
- **Staleness is a comparison, not a guess.** The VM now bumps a version on
  every store to a global (`Vm::global_version`, one `+= 1` in the single
  write path). A `Live` records the versions of the leaf, every helper it
  reaches and every table or constant it captured; `refresh` recompiles
  only when one of them has moved. Twenty textures and a `(define
  unrelated 42)` at the REPL cost nothing.
- **A refusal keeps the old code running.** Redefine `texel` with a
  `display` in it and `refresh` returns the refusal for the REPL to show,
  the generation does not move, the render goes on, and the deps stay
  stale so the corrected definition is picked up next time.
- **`vector-set!` on a captured table moves no binding**, so it is not
  staleness; `recompile` is the explicit re-snapshot for that case.

Exit criterion met: a render thread running passes back to back, each
checked for self-consistency against its own snapshot, while the main
thread redefines the texture twenty times and refreshes after each. No
crash; the pass in flight finishes with the old code; the next pass uses
the new; the last pass's generation matches the definition it used.


## S11b — a leaf compiled from a closure (2026-10-19)

`scm_llvm::compile_proc`, `verdict_proc`; `scm-llvm/tests/leaf_closure.rs`;
the number in `examples/closure_texel_cost.rs`.

**Why, and it is not a hypothetical.** Track B's P5 is ray's texel bake, so
the leaf to compile is whatever `make-texture` is handed — and every one of
the 25 texture scenes in ray's corpus hands it an INLINE LAMBDA that closes
over the numbers and colours of the procedure that built it:

```scheme
(define (wood-texture early late rings warp res)
  (make-texture res res
    (lambda (u v)
      (let* ((r (ring u v rings warp))
             (fiber (* 0.06 (- (vnoise (* u 4.0) (* v 70.0)) 0.5))))
        (map (lambda (e l) (+ e (* (- l e) r) (* e fiber))) early late)))))
```

S11 resolved a free variable as a local, a global procedure, a pure
primitive or a global table. `rings` is none of those, so the verdict was
`` `rings` is unbound (in tex)`` — and there was no entry point that could
take the procedure at all, because `compile` starts from a global name and a
texel function is an argument.

**Three things were needed, and the corpus named them in order** — each
refusal pointed at the next, which is why the gaps were found by handing the
analysis ray's real texel rather than by reading:

1. **A free variable resolves through the captured environment**, before the
   globals, which is both lexical order and safe: a top-level procedure has
   no environment at all (the global table is a separate structure), so only
   a procedure created inside another one has a frame. Verified rather than
   assumed. A number becomes a constant in the emitted code.
2. **`map` over two sequences**, zipped. `(map (lambda (e l) …) early late)`
   is how every texture in the corpus blends two colours, and S11
   specialised one sequence only.
3. **A sequence may be a captured variable.** `early` and `late` are the
   procedure's arguments, so the length that makes the unroll possible is
   known from the frame rather than from the source.

**A captured procedure falls through to the global path**, and that is not a
detail: a library's procedures close over the library's own frame, so
`value-noise-2` finds `noise-fade` in its environment *as well as* in the
global table. Taking the environment's answer for everything refused five of
`(scm noise)` for "holding a procedure". Only DATA is read from a frame.
Caught by the S11 gate within a minute of the change, which is what that
gate is for.

**Each emitted function carries the frame its body came from**, rather than
the compile carrying one — so a helper's free variables are its own, and two
closures from one procedure cannot be confused. `oak` and `pine` from the
same `wood-texel` hold different colours under the same names, and the test
asserts both against the interpreter *and* that they differ from each other,
because a test where they happen to agree proves nothing.

**A captured table is a table.** The analysis accepts a captured sequence,
so codegen must be able to emit one, or the two vocabularies of refusal this
file exists to prevent come straight back. Same snapshot, same bounds check;
only where the name is found differs.

### The number

ray's own `wood.janet` texel — three 5-octave `fbm` calls, a `vnoise`, and
the two-colour blend — compiled from the closure value:

```
compile                 20.1 ms
native                 203.5 ns/texel   (512x512)
interpreted (T1)     25083.6 ns/texel   (123x)

one 512x512 texture:   53.4 ms native, 6575.5 ms interpreted
```

**Janet is not in that table on purpose.** The 4,112 ns/texel from
[leaf-survey.md](leaf-survey.md) is a *different* texture, and a ratio
against it would be arithmetic rather than measurement. The Janet number for
this texel is measured on ray's side, where both languages can run the same
scene, which is P5's own exit criterion.

### What is not here

- **Staleness of a captured value.** `Live` recompiles when a global the
  leaf reads has been rebound; a captured value lives in a frame, and the
  frame changes only by the closure being made again. For ray that is the
  texture being redefined, which produces a new procedure value and so a new
  compile. Nothing tracks it *inside* one closure, because nothing can
  change there.
- **A captured procedure as a call target.** It falls through to the global
  lookup, so a helper that exists only in a frame — a lambda bound by the
  enclosing `let` and called by the texel — is still refused. No scene in
  the corpus writes one.

## Sampling data: numeric vectors and arrays (2026-10-14)

The S11 number was a *procedural* texture: arithmetic only, no data. The
other half of ray's texture work is a tap into a loaded image, and until
now that fell outside the compiled subset -- so `(srfi 4)` and
`(srfi 63)` paid off in scripting and not in a render. They do now.

| | |
|---|---|
| compile a bilinear sampler | 12.5 ms |
| **native, per bilinear tap** | **3.3 ns** |
| interpreted on T1, per tap | 934.7 ns |
| | **280×** |

That is a 512×512 `f64` image, four taps and a blend per call.

### Where the data lives, and why it differs by type

A **plain `vector`** global stays what it was: a private LLVM constant
array, so a read at a known index folds. That is right for the small
precomputed lookup tables the corpus has.

A **`numvector`** or **`array`** global becomes a *pointer to a snapshot
held by the compiled handle*. Baking a megabyte into the IR would cost
compile time, module size and a full recompile on every change, for
nothing the optimiser can use. The snapshot is an `Arc<Vec<f64>>` that
moves into the `Compiled`, so:

- the data outlives every caller, including a rayon worker still calling
  an older generation after `Live::refresh` installed a newer one;
- `Compiled` stays `Send + Sync`, because an `Arc<Vec<f64>>` is;
- the test that drops the `Vm` -- and with it the heap holding the
  array -- and then renders 65,536 taps from a rayon pool says so.

Both are **snapshots**, which is plan §6 item (2) as written: a leaf reads
the data as it was when it was compiled. Mutating an element afterwards
does not reach code already compiled, and there is a test for that too,
because the alternative is a render whose pixels change under it mid-pass.
`Live::recompile` is how a leaf catches up with an element write; a
*rebinding* moves a version and `Live::refresh` catches that by itself.

### An array read is the strides, inline

`(array-ref img y x)` compiles to one bounds check per subscript -- each
against its own dimension, so an index inside the store but outside the
shape is still refused -- then a multiply and add per dimension, then the
load. The shape, strides and offset are read at compile time, so a
transposed or reversed view costs a sign, not a branch.

### Two things fixed on the way

**A captured table was found by matching its length.** `load` searched the
table map for an entry whose length matched, with a comment admitting the
flaw. Two captured tables of the same size would read the wrong one. The
store is now passed through from the caller that resolved it, and a test
compiles a leaf reading one element from each of two eight-element
vectors.

**The analysis now judges the primitive, not the name.** It compared the
*symbol's* name against the pure list, so `f64vector-ref` -- which is an
alias of `numvector-ref`, as are its nine siblings -- would have needed
ten entries. It asks which primitive the global holds instead, so one
entry covers them all, `(define my+ +)` is accepted, and
`(define out display)` is refused with a message that names `display`.

### Still outside the subset

Writing shared data: `numvector-set!` and `array-set!` are refused,
because a leaf runs on N render threads at once and there is no safe way
to let it write what they all read. An index out of range reads NaN rather
than raising, which is the one place a leaf and T0 deliberately differ --
a leaf has no way to raise. The other is the index itself: the subset has
one numeric type, so an index arrives integral-but-inexact where T0 wants
an exact integer.
