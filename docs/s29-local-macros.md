# S29 — local macros, expanded once

**Done 2026-11-03.** `scm/src/vm.rs` (`local_expansions`, `local_rules`,
`scope_frame`, the local-macro arm of `eval_step`, `lookup`),
`scm/src/syntax.rs` (`origin`, `expand_scoped`, `with_def_env`),
`scm/src/heap.rs` (`AliasEnv`), `scm/src/vm/compile.rs` (`Var::Macro`,
`resolve`/`resolve_exact`, `let_syntax`, `let_scope`),
`scm/tests/local_macros.rs`, `scm-conform/bench/macros.scm`.

## What was slow

ray's house style is SRFI 204 `match`, and a `match` in a procedure cost
17–430 µs a call where the `cond` it stands for costs 0.1. Worse, the cost
**grew with the number of calls ever made**: the same three-clause match
measured 49 µs a call over a thousand calls and 290–430 µs over a hundred
thousand, and a vector pattern 193 µs against 4,417 µs.

The brief that started this named the `let-syntax` probes in `204.sld`
(`%if-id`, `%if-wild`) and the uncached local-macro branch of the
evaluator. Both were real; neither was most of it. Measured, in order of
weight:

1. **A library's own macros took the uncached path.** `match` is a global
   macro (an import copies it into the macro table), so its use was
   expanded once and cached (S5b). But its expansion names `%clauses`,
   `%m`, `%if-id` … through renames whose alias is the *library frame*, and
   T0 finds a macro there with the ordinary `lookup`, which is the LOCAL
   macro branch — expanded on every evaluation, with no cache. So every
   step of a match's expansion after the first was redone per call.
2. **T1 compiled none of it.** The compiler's `resolve` declined any renamed
   name whose library frame did not bind it — `if`, `let`, `pair?`, every
   global a library template mentions — with "a reference through a
   locally defined macro", and declined a library macro outright as "a
   local macro". Any procedure containing a library macro use ran on T0.
   `let-syntax` itself was handed to T0 (`Op::Interp`), or declined the
   whole unit in tail position.
3. **Every expansion leaked.** A rename mints a symbol that is never freed
   and records an alias whose environment is a **collector root**. A local
   macro inside a procedure recorded that call's frame, so every call
   added a permanently rooted frame, and every later collection marked all
   of them. That is the growth with call count.
4. `let-syntax` re-parsed its rules and allocated a macro object and a
   frame per entry (1.06 µs of the 1.2 µs a bare `let-syntax` cost).

## The fix

**T0 caches local expansions.** `local_expansions` is the S5b cache for
a call whose head is a macro found in a frame, keyed on the call's cell
and cleared on every collection, like the global one. An entry is valid
only for what it was made with (`LocalMacro`): the macro *object*, for a
library's or an internal `define-syntax`'s macro — whose renames name its
own definition frame, so the expansion is a pure function of form and
object — or the parse *origin*, for a `let-syntax` macro (below).
`local_rules` parses a binding form's rules once, keyed on the form's cell
and the binding's index.

**T1 expands at compile time.** A `let-syntax`/`letrec-syntax` compiles as
a scope whose slots hold the macro objects as constants: flat (a store per
macro, no allocation) unless something captures it. Every use of a name in
it is expanded by the compiler, as is a finished library's macro, found in
its frame. A renamed name absent from a finished library's frame resolves
to its global, which is what T0 does at run time: once a library has
loaded, nothing defines into its frame again (`sealed_library_frame`).

## Soundness: why a cached expansion cannot read the wrong frame

A `syntax-rules` expansion depends on the form, the rules, and — through
hygiene only — the macro's definition environment. Matching consults no
environment (literals compare resolved names). The environment enters
when `rename` records each fresh name's alias: `x.37 → (x, def_env)`, and
`lookup` resolves an otherwise unbound `x.37` as `x` in `def_env`. For a
`let-syntax` in a procedure, `def_env` is **that call's frame**. Cache the
expansion and the second call evaluates `x.37`, whose alias still names
the first call's frame: it reads the first call's `x`. Checked by breaking
it on purpose — expanding origin-keyed entries with frame renames — and
the tests below fail on T0 with `(1 1 1)` for `(1 2 3)`, a stale value
for a closure made on a later call, and a wrong sum in the GC-churn loop
(emptying the cache at each collection does not save it: two calls fit
between collections).

So the alias of a `let-syntax` macro's rename does not name a frame. It is
`AliasEnv::Scope { origin, recursive }`: the parse it came from, found at
**lookup** time as the innermost frame on the *use-site* chain that binds a
macro with that origin — the `letrec-syntax` frame itself, or for
`let-syntax` its parent. That is the right frame for every evaluation
because:

- a renamed name is only evaluated lexically inside the binding form that
  made it (the expansion is code at the use site; `quote` strips renames),
  so that form's frame is on the chain, from any later call too;
- at most one such frame is: an origin is per binding FORM (keyed on the
  form's cell, not the spec's, since a template can put one spec into two
  nested forms — case 11 below), and a form cannot be lexically inside
  itself;
- T0 uses the scoped renames only when that frame holds the very macro
  object being expanded. A macro object reached any other way — one that
  escaped its form as a value — expands with renames into its own frame,
  uncached by origin, exactly as before.

T1 resolves the same way at compile time: a `Scope` rename is resolved in
the compile-time scopes outside (or, recursive, up to and including) the
`let-syntax`'s, with the depth counted from where the code runs. A
`let-syntax` scope a nested closure resolves through is marked captured,
so it exists at run time if that closure is ever compiled again from its
environment (a new global macro invalidates compiled code) or run by T0.
`set!` through such a rename declines, because T0's `bind` follows a rename
only to a global (it raises; case 17).

A side effect: `Scope` aliases root nothing, so the leak in (3) is gone for
`let-syntax`, and with every call site cached, names are minted once per
call site per collection rather than per evaluation.

## Tests

`scm/tests/local_macros.rs`, every case on T0 and T1, with and without a
collection at every safepoint: a template reading an enclosing parameter
across calls; the same with the name shadowed at the use site; an
expansion containing a lambda, kept across calls; a closure in the body
using the macro; recursive `letrec-syntax` with a hygienic temporary beside
the user's own `t`; mutual recursion; `let-syntax` not seeing its own name;
the macro shadowed on one path only; a macro-defining macro; **one call
cell evaluated inside two different `let-syntax` forms and a `let`** (via
`eval` of a shared list); one spec cell in two nested forms; a GC-churn
loop; a tail call through `let-syntax` at depth 100,000; a definition in a
`let-syntax` body (T1 declines); a closure surviving a recompile forced by a
new global macro; `match` over literals, vectors and pairs; `set!` through
a template. These two tests pass on the old code too — the answers did not
change — and fail on a cache that bakes the frame in.

The speed is pinned by counting, as S14 pins flat frames, not by timing:
a compiled `match` calls 1,000 times with no decline and no name minted; a
`let-syntax` procedure allocates nothing in 1,000 calls; T0 mints at most a
match's names once per collection (the old code minted 9,416 in the same
loop). All three fail on the old code.

## Numbers

`scm-conform/bench/macros.scm`, µs per call, best of three. Before is
9b4c488 at 1,000 calls (at 100,000 it is several times worse, per (3));
after is at 100,000.

| case | before T1 | after T1 | after T0 |
|---|---|---|---|
| `cond` on a keyword | 0.075 | 0.109 | 1.76 |
| a global syntax-rules macro | 0.062 | 0.069 | 1.58 |
| one `let-syntax` in the body | 1.06 | 0.075 | 1.61 |
| `(match x (y y))` | 16.9 | 0.108 | 3.2 |
| `(match x (:a 1) (_ 3))` | 34.7 | 0.179 | 4.8 |
| `(match x (:a 1) (:b 2) (_ 3))` | 48.8 | 0.203 | 5.2 |
| four `vector-ref`s | 0.114 | 0.125 | 2.0 |
| `(match v (#(a b c d) …))` | 193 | 0.448, then 0.268 | 25.8 |
| `car`/`cadr` with checks | 0.148 | 0.155 | 4.3 |
| `(match l ((a b . _) …) (_ 0))` | 156 | 0.221 | 23.0 |

On T1 a match is within 2× of the hand-written test (the hand-written
code checks nothing; the match checks types and lengths). The residue is
mostly `204.sld`'s failure thunk: each clause with a successor binds
`(lambda () <next clauses>)`, a closure and its frame per call.

The vector pattern was still 3.6× after the VM fix because `204.sld`
matched `#(p …)` by converting the vector to a list — four pairs per call.
A second commit matches a vector pattern without an ellipsis in place
(`%mvec`: one `vector-length` check, then `vector-ref`s), 0.448 → 0.268
µs; one with an ellipsis, or more than 16 elements, keeps the list path.
`a_fixed_vector_pattern_allocates_no_list` pins it (6 objects a call
before, 2 after — the same as a predicate pattern).

## Not done

- **The global expansion cache still ignores redefinition.** A call cell
  expanded with a global macro and evaluated again after that macro is
  redefined reuses the old expansion until the next collection. Unchanged
  here; noted because the local cache, keyed on the macro object, does not
  have the problem.
- **T1 still declines** a `let-syntax` body with internal definitions, an
  internal `define-syntax`, and a macro found in a frame T0 made (other than
  a library's). All run on T0, now with the cache.
- Renames are still minted per expansion and never freed; the cache bounds
  how often that happens, not the table.
