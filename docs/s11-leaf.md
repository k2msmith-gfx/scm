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
