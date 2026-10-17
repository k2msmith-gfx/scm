# Replacing Janet with scm — the port plan

*Plan, 2026-10-16, written against ray @a687b9d (`tool-refactor`) and scm
@5290218. Intent from before the build: where this and the code disagree, the
code wins.*

**Paths.** This plan is about work in two repositories, so a bare path is
**this** one and a `ray/` prefix is the other — `scm/src/vm.rs` against
`ray/src/snapshot.rs`. ray's own documents are named rather than linked,
since they are not siblings of this file. It lived in ray's `docs/` for its
first day; it moved here because every stage it describes depends on this
repository and because ray's branch has other hands on it.

This is the ray-side plan for **Track B** — ray adopting scm as its scripting
language. It supersedes ray's `docs/scheme-plan.md` §7's S3, S4, S6 and
S7, which were the consumer-side stages written on 2026-10-12 *before scm
existed*; scm is now built (S1–S20 in [plan.md](plan.md) §9) and what
those stages guessed at can be measured instead. `scheme-plan.md` §1 (why),
§2 (what the API demands) and §5 (the inherited hazards) still stand and are
not repeated here.

The sibling documents are [plan.md](plan.md) §9 Track B (B1–B5, the
same work sized from the other side), [subset.md](subset.md) (the
normative statement of what the language is), and
[scene-language-survey.md](scene-language-survey.md) (what the corpus requires,
measured).

---

## 1. Kevin's framing, 2026-10-16

Four rulings, and they set the shape of everything below:

1. **The API work merges first.** `tool-refactor` lands on `main` when A4 is
   done. The port does not begin on `tool-refactor`.
2. **The port is its own branch** off `main` — `scm-port` — created after that
   merge.
3. **That branch stops after the corpus and the texels.** The verb bridge, the
   printer, the 218-file example corpus converted and rendering identically,
   and compiled texel functions. The ~18,792 lines of modeler Janet and the
   2,895-line prelude move on a later branch.
4. **Janet comes out eventually, and this branch is the first step toward it,
   not the step that does it.** Both VMs are feature-gated for the life of the
   branch; scm becomes the default at the merge; `--features janet` still
   builds and still passes its tests when the branch lands. The *deletion* of
   the `janet` feature is the next branch's exit criterion.
5. **The code reads as Scheme or Racket, not as a different language.**
   Stated twice, the second time sharper: *"I don't necessarily want the
   scripts to look like Janet. I think it is better to make them look like
   Scheme"*, then *"if a Scheme or Racket programmer looked at the code in ray
   or the modeler, it would be very familiar, not a different language."*

   The second phrasing is the operative one, and it is a **stronger and more
   testable** bar than the first. "Not Janet-looking" is satisfied by avoiding
   a spelling; *familiar* is only satisfied if a Racket programmer can read a
   page of ray's Scheme with nothing needing explanation. It also raises the
   target: Racket is the Scheme that actually *has* keyword arguments and
   optional defaults, so there is a real convention to be familiar toward and
   not just a vacuum to avoid filling with Janet.

   §3.5 is the spelling table that follows from it — and §3.6 is the harder
   half, because measuring the language against *familiar* found one place
   where scm itself fails, in the most-read code there is.

Ruling 4 is a deliberate, time-boxed exception to the rule
ray's `docs/rust-api-plan.md` §4 holds every A-stage to — *every stage
deletes the Janet it replaces in the same commit*. The reason is that the
port's only real gate needs both VMs **in one build**: §5's P4 loads a scene
under Janet, saves it, prints it as Scheme, loads *that* under scm, and
compares the two `Form` trees. Delete `05-materials.janet` and there is
nothing left to differentially test the scm material against. A cross-language
port has an oracle available for free, exactly as A2's dialect-independent
emitter did, and giving it up to keep a rule would be the expensive kind of
tidiness.

---

## 2. What is already paid for — measured on both sides

This is the part that makes a ~2.0-unit branch plausible rather than hopeful.
Sizes below are in the API plan's unit (T2 = 1.0).

**On ray's side, as of A3+A5 complete (§4.62, §4.77):**

| | |
|---|---|
| **178 verbs in Rust**, on one guarded dispatcher | a second binding adds no per-verb code |
| the `#[verb]` inventory with **five consumers**, one of them the shim generator | `ray/src/api/janet.rs` is 727 lines; the Scheme one is a sibling, not a rewrite |
| the store holds a **neutral `Form`**, and `ray/src/snapshot.rs` is an **ungated** Rust writer over it | a binding with no Janet VM can save a scene |
| `ray/pyray/tests/conformance.py` | the shape the scm conformance pass copies |
| A5's mesh verbs | a script can build a polygon model, so the port does not inherit that gap |
| `ray/scripts/dev/run_corpus.sh`, with its classifier for what is and is not a scene | 191 renderable of 218 `.janet` files; do not rediscover the other 27 |

**On scm's side, verified by running it, not by reading it:**

| the corpus needs | scm |
|---|---|
| `:at` as a keyword, no `#:` prefix | **identical** spelling to Janet |
| `[0 1 0]` evaluating and *printing back* with brackets | **done** (S1/S2) |
| `&opt` / `&named` / `&keys`, defaults preserving an explicit `#f` | **done** (S4) — so a generated Scheme shim is near-identical in shape to the Janet one |
| `array/push` × 248 | `vector-push!` on the **one** vector type (`scm/src/prim.rs:1186`) — the survey's separate growable type was dropped as unnecessary |
| `format` | **done**, native (`scm/src/prim.rs:1757`) |
| seeded RNG as a *value*, rebindable like `with-dyns` | `(srfi 27)` / `(srfi 194)`, plus `parameterize` |
| `seq` 253, `for` 174, `each` 156, `repeat` 20, `inc`/`dec` 151 | **Janet's names, in scm's prelude** (`scm/src/prelude.scm:250–280`) — `(seq (i 0 n) expr)` and `(seq (x in lst) expr)`; Janet's `:range` spelling is the converter's problem |
| the seven one-line vector helpers §4.55 kept in Janet | `(scm linalg)` — vec3/vec4/mat4/quaternion, ~95 procedures. **These do not get ported; they get deleted.** |
| a compiled, VM-free leaf for texels | S11, `scm-llvm/` — compiled with the `Vm`, dropped, then called from `rayon` workers |
| a host that can bound a runaway script | `set_fuel`, `set_heap_cap`, `interrupt_handle`, `eval_with_timeout` |

And the handoff itself was rehearsed: on **2026-10-16** an out-of-workspace
crate depending on both by path was built and run, doing exactly the port's
moves — keyword-argument verbs, a verb calling back into Scheme through
`define_fn_with`, a `Proc` held across 20,000 evaluations and a collection,
the embedded libraries reachable from the host, fuel bounding a runaway, and a
leaf compiled then called from `rayon` workers. The dependency is:

```toml
schemers = { path = "../scm/scm" }        # then `use scm::…`
scm-llvm  = { path = "../scm/scm-llvm" }  # P5 only, behind a ray-side feature
```

`scm-llvm` needs LLVM 21 **at build time, always**, so it goes behind its own
ray feature exactly as `scm-cli` does on scm's side. A plain `cargo build`
must keep needing no toolchain.

---

## 3. The six things that do NOT port, and what each costs

Measured, so the branch starts from facts rather than from this section again.

**(a) The dictionary — and under ruling 5 it mostly dissolves.**
`FormVal::Struct` (`ray/src/snapshot.rs:362`, the material literal) and
`FormVal::Table` (`ray/src/snapshot.rs:964`, the NURBS net) are **the only two
producers in all of ray**, and both are keyword-keyed. scm has no dictionary
literal.

The first draft of this section recommended a `%dict` procedure collecting
`&keys` into an alist. **Withdrawn**: that is a Janet literal transliterated,
which is what ruling 5 says not to do. A Scheme script writes a **named
constructor with keyword arguments**, and for the material one already exists:

```scheme
;; what a saved scene emits today, Janet (dining-room-baked.janet:31)
(add-plane :y -0.04 :material {:albedo [0.18 0.15 0.13] :shininess 32
                               :shader :lambertian} :name :ground)

;; what it emits as Scheme
(add-plane :y -0.04 :material (material :albedo #(0.18 0.15 0.13)
                                        :shininess 32 :shader :lambertian)
           :name :ground)
```

`material` **is** that constructor already — *"Build a material value and
return it. Pass it to any shape call"* (`ray/05-materials.janet:40`) — so the
printer emits the call that the author would have written, and the gap is
gone. The NURBS net has no such constructor: `(nurbs-net name)` is a *reader*
that returns the net as data. So that one needs either a seven-keyword
constructor or keyword arguments on `add-nurbs-surface`. **One verb, decided
in P3** — against two dictionary spellings and a `%dict` in every saved file.

**(b) `nil` → `#f`.** Six producers. scm has no nil, and an absent argument
binds `#f` (survey §4). Nothing to build; it has to be *checked* at each of
the six, because a scene that reads `#f` where it meant "absent" is the
compiles-runs-looks-right class of bug.

**(c) The spelling collapse, and the vector spelling ruling 5 picks.**
`@[…]`/`[…]` and `@{…}`/`{…}` become one form each — scm has one vector type,
and §6 of the survey already carries the `tuple?` cost (4 files). `#` comments
become `;`, **and the trap runs both ways: in Janet `;` is SPLICE, not a
comment.**

What a tuple is *spelled* as is the ruling's biggest single consequence,
because the corpus has **28,736 of them** and one 208-line saved scene has
**1,161**. Three candidates, and the first is wrong on evidence:

| | |
|---|---|
| `#(0 1 0)` everywhere | **wrong.** A saved tuple can hold a **call**: `(def %tex [(load-texture "…")])` at `ray/dining-room-baked.janet:6`. In R7RS a vector constant's elements are *data*, so the `load-texture` would never run. `FormVal::Tuple`'s own doc already notes the parenthesised form evaluates — the bracket form does too, and `#(…)` does not |
| `[0 1 0]` | compact, identical to the Janet original, and **the least Scheme-looking of the three**: in Racket, Guile and Chez, `[ ]` *is* `( )`, so `[0 1 0]` there means applying `0`. It reads only in scm |
| **`#(0 1 0)` when every element is a literal, `(vector …)` when one is not** | **recommended.** Plain R7RS both ways, self-evaluating (R7RS 4.1.2: vector constants evaluate to themselves), **one character** longer than the Janet original, and correct for the `%tex` case. The cost is two spellings in one file, which is one `if` in the printer |

Uniform `(vector 0 1 0)` is the honest alternative — one spelling, no branch —
and it costs +7 characters × 28,736, which is a visibly noisier scene file.
Worth stating so the recommendation is a choice and not an oversight.

And the heads come across too: a saved scene's `def` is emitted as
**`define`**. The printer owns the head names, so this is free.

**(d) `camera-match`'s shadow does not port — a new finding, and the
mechanism matters more than the two verbs.** §4.77 ruled `camera-pixel` and
`camera-match` stay in Janet because the `:apply` branch resolves
`set-camera` out of `(curenv)` **at call time**, so the modeler's shadowing
definition (defined third in `unify.janet`) wins. scm **copies bindings on
import** — a standing decision, [libraries.md](libraries.md) — so that
late-lookup trick has nowhere to live in a Janet-free ray. §4.77 made "reaches
a shadowed binding" a *reason a verb stays in Janet*; this branch has to
replace the reason with a mechanism, and the cheapest is an explicit
indirection the host owns (a settable hook the modeler installs) rather than
re-inventing dynamic lookup. Two verbs, one decision, and it should be made in
P3 where the prelude is written and not discovered in P4.

**(e) `eval` / `eval-string` / `postwalk` / `curenv` — 8 of 219 example
files** (`homoiconic.janet` and its neighbours). scm has `eval` (S9) but no
`curenv` and no `postwalk`. Blast radius is small enough to leave for last and
large enough not to pretend it is zero: these 8 are **named and exempted** in
P4's gate, and converted or retired on the later branch.

**(f) Brackets — two of the three uses port, and the third is 662 call
sites.** Kevin asked, 2026-10-16, whether the port plans on Scheme's extended
`[ ]`. It is not a plan item: scm decided it at S1 and the three cases differ,
verified by running each.

| Janet's use of `[ ]` | scm |
|---|---|
| **a value** — `[0 1 0]`, evaluating its elements | **identical.** `[a b c]` reads as a vector flagged `bracket`, which is a *constructor* while `#(a b c)` is a constant (`scm/src/reader.rs:12`). R7RS reserves `[ ] { }` for future extensions, so this costs no conformance |
| **grouping, Racket/Guile house style** — `(let ([x 1]) …)`, `(cond [p e] …)` | **accepted.** The flag makes a bracket vector paren-ish where a list is expected (`scm/src/syntax.rs:32`); verified in `let`, `let*`, `do`, `case`, named `let`, `cond`, `guard`, `define-record-type` and `syntax-rules` clauses. scm's own prelude is written this way |
| **a parameter list** — `(defn f [x y] …)`, `(fn [x] …)` | **refused**: `(lambda [x y] …)` is *"malformed parameter list"*. The one Janet bracket shape that does not port |

The third is the only one with a cost, and it is **not on this branch**: a
saved scene is flat calls, so the corpus has no `defn` in it after P4's
conversion. It lands on B5, where the count is **662 bracket parameter lists**
across the 57 `.janet` files outside `examples/` — the prelude, the modeler
and `scripts/`. A translator rewrites every one of them to parens.

**DECIDED by ruling 5: no.** The question was whether scm should accept a
bracket parameter list, turning B5's 662 rewrites into zero. It should not —
`(define (f x y) …)` is what Scheme looks like, and widening a datum-use
deviation from R7RS to spare a translator that is rewriting those files anyway
buys the wrong thing. B5 rewrites the 662. Nothing changes in scm, and
[subset.md](subset.md) needs no new line.

Note that the *grouping* use needs no ruling, because it already **is** Scheme
house style: `(let ([x 1]) …)` is how Racket, Chez and R6RS code is written,
and scm's own prelude is written that way. Ruling 5 does not ask for parens
there.

One thing not to expect of brackets in the other direction: **scm prints every
vector as `#(…)`**, whatever spelling built it (`scm/src/writer.rs:231` — one
canonical output, deliberately, so two `equal?` vectors cannot print
differently depending on how each was born). The saved-file spelling is
therefore **ray's printer's** responsibility and nothing that survives a trip
through scm. [scene-language-survey.md](scene-language-survey.md) §8a claimed the opposite and was
corrected on 2026-10-16.

### 3.5 What "look like Scheme" decides, spelling by spelling

Ruling 5 in one table, so P3 implements a decision rather than re-arguing one
per form. "Owner" is what has to change: the **printer** is
`Form::to_scheme()`, the **translator** is B5's tool over hand-written Janet.

| Janet | Scheme | owner |
|---|---|---|
| `(def x …)` | `(define x …)` | printer |
| `{:albedo [.8 .8 .8]}` | `(material :albedo #(.8 .8 .8) …)` — the constructor, not a dict | printer |
| `@{:cols 4 …}` (the net) | one new constructor, or keyword args on `add-nurbs-surface` | P3 decision |
| `[0 1 0]`, `@[0 1 0]` | `#(0 1 0)`; `(vector …)` when an element is not a literal | printer |
| `nil` | `#f` | printer |
| `#` comment | `;` comment | printer |
| `(defn f [x y] …)` | `(define (f x y) …)` | translator (662 sites) |
| `(fn [x] …)` | `(lambda (x) …)` | translator |
| `(each x xs …)` | `(for-each (lambda (x) …) xs)` | translator |
| `(seq [i :range [0 n]] e)` | `(map …)`, or a named `let` | translator |
| `(for i 0 n …)`, `(repeat n …)` | a named `let`, or `do` | translator |
| `(inc n)` / `(dec n)` | `(+ n 1)` / `(- n 1)` | translator |
| `(let [x 1] …)` | `(let ((x 1)) …)` **or** `(let ([x 1]) …)` — both are Scheme | translator |
| `:at`, `:material` | **`:at`, `:material` — unchanged** | — |

**The keywords stay `:at`, and that is not an exception to the ruling.** R7RS
has no keyword datatype at all, so there is no idiomatic Scheme spelling to
move toward: `#:at` is Racket's and Guile's, `:at` is Clojure's and Janet's,
and both are extensions. scm reads both and emits `:at` (`scm/src/reader.rs:1`).
Keeping it costs nothing, 87 of ray's verbs depend on the convention, and
`#:` was the thing Kevin disliked about Steel — a reason recorded before this
plan existed and not weakened by it.

**A consequence for scm, worth raising there rather than here.** scm's core
prelude defines `seq`, `for`, `each`, `repeat`, `inc` and `dec` *as globals*
(`scm/src/prelude.scm:250–280`), on the survey's argument that keeping Janet's names
saves 253+174+156 loops from being rewritten wrong. Ruling 5 retires that
argument twice over: the translator now targets Scheme idiom, and those loops
were never going to be converted mechanically anyway — a **saved scene is flat
calls**, so the corpus's `seq`s do not survive a save to be translated at all.
They now cost rather than save: a general-purpose embeddable Scheme that
ships `for` and `each` as globals is surprising, and goal 2 is to distribute
one. **Recommendation: move them to an optional `(scm janet-compat)` library.**
Nothing in scm's own libraries uses them — only their own tests — so the move
is mechanical, and B5 can import it for a file mid-translation and drop the
import when the file is finished.

### 3.6 The familiarity test, and the one place the LANGUAGE fails it

§3.5 is about what the printer emits. This is about whether the language
underneath it reads as Scheme — a different question, and the one ruling 5's
second phrasing actually asks. Measured by running each case rather than
recalling it:

| what a Scheme or Racket programmer writes | scm today |
|---|---|
| `(define (f a) …)`, named `let`, `do`, `case`, `cond` | same |
| `(let ([x 1]) …)` — brackets as grouping | **accepted** |
| `#(0 1 0)`, `(vector 0 1 0)` | same |
| **`(define (f a [b 0]) …)` — optional with a default** | **works already.** Racket's exact spelling, and nothing in this plan had noticed |
| `(f 1 #:at 5)` — a keyword at a call site | **works**, and `:at 5` is the same datum |
| `guard`, `error-object-message`, `error-object-irritants` | R7RS, present (S9) |
| `make-parameter` / `parameterize` | present |
| `define-record-type` | present |
| `define-library` / `import` | present (S15) |
| `format` with `~a` / `~s` | present, SRFI 28's spelling |
| **`(define (f a #:at at) …)` — a keyword PARAMETER** | **refused**: *"expected NAME or (NAME default)"*. scm requires `(define (f a &named at) …)` |

So the language passes the bar almost everywhere, and two of the rows are
better than this plan assumed. The failure is the last one, and it is not a
small place to fail: a keyword **parameter** is how all 178 generated verb
shims are declared, and `&opt`/`&named`/`&keys` is Common Lisp's convention by
way of Janet. A Racket programmer recognises the *call* and then does not
recognise the *definition* — which is exactly "a different language" in the
code they would read most.

**Decided and BUILT, 2026-10-16 — scm S21 @7b5891d.** A keyword datum where
`&named` would go marks a keyword parameter, with `(at default)` for a
default, and `&opt`/`&named`/`&keys` stay legal so nothing existing breaks:

```scheme
;; what 178 generated shims will look like
(define (add-box size #:at (at #(0 0 0)) #:material material) …)

;; and because `:at` and `#:at` are the same datum in scm, this is the same
;; declaration — so house style can match the call sites
(define (add-box size :at (at #(0 0 0)) :material material) …)
```

It was one place in `parse_formals`, which already handled `(NAME default)`
for the optional case, and **one parser feeds all three tiers** so T1 and T2
needed nothing. [subset.md](subset.md) gained a section for the lambda list — the grammar
had never been written down in the normative document — with
`scm/tests/subset.rs` checking its sentences. Every gate on that side is
green, including the 53 R7RS benchmark programs, which turned out **not** to
have been running: `target/r7rs-benchmarks` was a symlink into a deleted
scratch directory, so the gate skipped in 0.00 s.

So P2's shims are written in the familiar spelling from the first one, rather
than being generated `&named` and migrated later.

**And the keyword token itself stays `:at`** (Kevin, same day), which is the
one place the familiarity bar is deliberately not followed to its end. Racket
and Guile both write `#:at`; `:at` is Clojure's and Janet's. It stays because
it was chosen on its looks when Steel's `#:` was rejected, because 113 verbs
in the manual take keyword arguments and 14,706 keyword tokens are in the
corpus, and because nothing is ambiguous either way — scm reads both, so a
Racket programmer or an LLM writing `#:at` is never wrong. Recorded as a
preference knowingly kept, not an oversight.

#### What the bar asks of B5 beyond spelling

Three structural things, because "familiar" is not only tokens. None is extra
work — each replaces a Janet mechanism that has to be replaced anyway:

| the modeler's Janet | the familiar Scheme |
|---|---|
| 18 prelude files loaded in order, one global environment | `define-library` units with explicit `export`/`import` |
| `(try (…) ([err] …))` | `guard`, with `error-object-message` / `error-object-irritants` |
| `setdyn` / `dyn` / `with-dyns` (33 + 26 + 1 uses) | `make-parameter` / `parameterize` |

And the first of those **converges with §3(d)**, which is worth noticing
because it turns a problem into a reason. Janet's load-order shadowing —
`unify.janet` defining `set-camera` third so the modeler's version wins — has
no equivalent once the code is in libraries, because importing copies
bindings. §3(d) already concluded that `camera-match` needs an explicit
host-owned hook instead. The structure the familiarity bar asks for is the
same structure that forces the hook, so the two findings want the same thing
and B5 does not have to choose between them.

### The §4.77 residue IS the port's scope

The 178 verbs in Rust come across for free; the port's actual hand-written
work is the list of what §4.77 ruled *stays* in Janet. In a Janet-free ray
every one of these needs a scm counterpart, so the closing inventory is the
new prelude's table of contents:

| staying in Janet by ruling | what the scm port owes it |
|---|---|
| `material`, `make-texture`, `field-bake`, `add-implicit`'s closure branch, `in-group` | a host closure through `define_fn_with` — the rehearsal crate proved this path |
| `load-gltf` (via the `*notify*` var) | a parameter, not a dynamic var |
| `render-settings` + its generator (§4.76 — a document for a person to read) | rewrite in Scheme; it is a view, and views are cheap |
| `camera-pixel`, `camera-match` | (d) above |
| `vadd` and the six other vector helpers | **deleted** — `(scm linalg)` |
| `mat-emit`, `shader-code` | private, follow their callers |
| `12-snapshot.janet`'s five, the sync trio | parked by Kevin; still parked |

The four kept prims (`%object-parent`, `%transform-object`,
`%add-point-light`, `%entry-size`) and the ~13 `%get-*` accessors §4.76 keeps
alive are **already Rust**. They need scm *registration*, not rewriting — a
P2 line item, not a P3 one.

### In what language? Scheme — and the ruling already says so

The question answers itself out of §4.77's own sentence, which is worth
reading literally:

> a verb that exists to reach host-level bindings IS host glue, and belongs
> in the host.

The ruling was never *belongs in Janet*. It was *belongs in the host*. Replace
the host and the glue follows it: **these get rewritten in Scheme, not in
Rust.** Writing them in Rust would be re-arguing on a branch what was decided
on the merits in §4.43, §4.55, §4.76 and §4.77 — and it would re-import the
problem each ruling found, which is that a verb reaching into the host's own
environment cannot be expressed by a verb that has no host.

**The size, measured.** The whole remaining prelude is **2,895 lines holding
68 `defn` and 4 `defmacro`** — and the four macros are `with-scene`,
`with-group`, `defshape` and `defgroup` (`ray/11-retained.janet:129–193`), all
ordinary binding-and-body shapes that `syntax-rules` expresses directly. A
good fraction of those 2,895 lines is prose that has already moved into Rust
`#[verb]` docstrings for the 178. So P3 is rewriting dozens of procedures, not
thousands of lines, which is why it is sized 0.4 and not 2.0.

One trap to carry in while doing it: **colon-initial names read as keywords**
in scm, so they cannot be macro names, procedure names or `syntax-rules`
literals. It cost `(srfi 42)` its `:generator-proc` on scm's side.

**And one group where scm genuinely changes the answer, which is worth not
acting on yet.** §4.43's five — `material`, `make-texture`, `field-bake`,
`add-implicit`'s closure branch, `in-group` — are script-side *because they
take a closure*, and a Rust verb could not call a **Janet** closure without
inventing a callback channel. scm has that channel already: `define_fn_with`,
plus a `Proc` held across 20,000 evaluations and a collection, both exercised
in the 2026-10-16 rehearsal. So those five *could* become Rust verbs taking a
procedure argument, and the obstacle that kept them out was Janet's, not
theirs.

**Recommendation: not on this branch.** Rewrite them in Scheme at P3 — it is
mechanical and it keeps P3 small — and record them as candidates for the
branch after. The natural moment is P5's successor, because
`make-texture` is reopened there anyway, and a compiled texel function **is**
the closure crossing the boundary. Deciding it twice costs less than deciding
it early and wrong.

---

## 4. The branch mechanics

```
# on devel/ray, once A4 is green on all five legs
git switch tool-refactor && <the five-leg gate>
git switch main && git merge --no-ff tool-refactor && git tag api-refactor-complete
git switch -c scm-port
```

Two rules for the life of the branch:

- **Rebase onto `main`, do not let it drift silently.** The port's edits are
  nearly disjoint from anything else: `ray/src/api/scheme.rs` (new),
  `ray/src/form.rs`'s printer, `scripts/scm/` (new), and the feature wiring in
  three `Cargo.toml`s. A4 touches `script.rs` and the Janet mirrors, which the
  port does not.
- **Four build configurations stay green, every stage**: no features, `janet`,
  `scm`, and `janet,scm`. The fourth is the one P4's gate needs and the one
  nobody will think to try.

---

## 5. The stages

Sizes in T2 = 1.0. Each independently landable; each ends with what it did not
finish written down, as every A-stage did.

**P0 — A4, then the merge (0.4, and it is ray-side work, not the port).**
A language-neutral conformance suite driven off the inventory, `script.rs`
split by module, the dead Janet mirrors deleted, `api/` regenerated, the three
design docs updated, and the `pyray` `Bag::read` bug §4.57 found. The
conformance suite is the part the port depends on: build it so a **third**
binding plugs in, because the scm one is the first consumer of that claim.
Exit: the five-leg gate, then the merge and the tag.

**P1 — the dependency and the smoke test (0.2).** The path dependency, a
`scm` feature, one `Engine` per thread alongside the thread-local store
(`REGISTRY` keeps its `thread_local` semantics — unchanged, and the modeler's
scene-VM worker still depends on it), and **one verb reachable from Scheme by
hand**. Exit: the four configurations build, and `(set-film 320 240)` moves a
scene from a scm REPL.

**P2 — the verb bridge (0.5; scm's B1).** `#[verb]`'s shim generator emits
Scheme as it emits Janet: keywords, `&opt`, `&named`, **error strings
verbatim**, and numeric coercion at the boundary so no `->f` appears in
authored code. `api/scheme/api.md` generated through the same `--gen-docs`
path and guarded by the same byte-comparison test.
Exit: **the conformance suite passes from Scheme, for all 178 verbs.** Not
Rust unit tests — A0's lesson is the reason this line is here: seventeen Rust
tests passed while a generated shim silently dropped every option, because
each test built the argument shape by hand.

**P3 — the printer and the prelude (0.4).** `Form::to_scheme()` beside
`to_janet()` (`ray/src/form.rs:376`), carrying §3.5's spelling table and §3's
(a)–(d) — including the one decision §3.5 leaves open, the net's constructor;
and
`scripts/scm/` — §3's residue table, written in Scheme: **68 procedures and
4 macros**, with the seven vector helpers deleted rather than translated and
the four prims and ~13 accessors registered rather than rewritten. Exit: every corpus scene, saved and
printed as Scheme, **reads back to an equal datum** in scm. That is scm's own
S1 round-trip gate, now run over ray's real output.

**P4 — the corpus converts and agrees (0.3; scm's B2).** For each of the 191
renderable scenes: load under Janet → save → print Scheme → load under scm →
save → **compare the two `Form` trees**. Scene files, not image bytes —
`reference_render_is_nondeterministic`. Reuse `run_corpus.sh`'s classifier for
the 27 non-scenes; name the 8 `eval`-shaped files as exemptions in the gate
itself, so the exemption is visible rather than a silent skip.
Note what a save *is*, because it bounds the claim: `save-scene` preserves
**state, not source shape** — loops, helpers and `defshape` come out as the
flat calls they made. This stage proves the corpus's *scenes* port. It does
not prove its *source* ports, and nothing on this branch does.

**P5 — compiled texels (0.4; scm's B3, goal (d)).** The one measured number
in the whole argument: `make-texture` at **4,112 ns/texel**, one 2048² texture
at **17.2 s**, `fruit-bowl.janet`'s whole scene build at **15.22 s**. S11's
leaf compiler, behind a ray feature so a plain build needs no LLVM. Exit: the
new number printed beside the old one, and the baked image compared against
the interpreted bake. `make-texture` is already a bake, so this changes
nothing observable except the seconds.

**P6 — scm becomes the default; Janet stays reachable (0.2).** Flip the
default feature, run the corpus gate on the scm build, and keep
`--features janet` building with its tests passing. Exit: the merge.

Total: **2.0** after P0's 0.4.

---

## 6. The gates

Every stage, the whole set. Four of these are ray's existing legs and are not
negotiable for being inconvenient.

```
cargo test                                                    # pure ray
cargo test -p modeler                                         # pure modeler
nix develop .#janet -c cargo test --features janet,oidn
nix develop .#janet -c cargo test -p modeler --features janet,oidn
MODELER_HEADLESS=1 nix develop .#janet -c python3 modeler/tools/run_all.py
cargo test --features scm                                     # new
cargo test --features janet,scm                               # new, and P4 needs it
scripts/dev/run_corpus.sh                                     # 191 scenes, 0 failures
```

Plus: the conformance suite from Scheme (P2 on); all three manuals
byte-identical with `api/scheme/api.md` joining them; and the 6-language
embedding benchmark suite as the performance baseline — on benchmarks that
reflect the real workload, a procedural generator loop and a **field
evaluation**, *not* scene loading, which is not interpreter-bound.

And a rule inherited from scm's side, because it has already paid for itself
there: **a gate that passes suspiciously fast is not passing.** Check a gate's
inputs exist before trusting it.

---

## 7. What this branch does NOT do

Said plainly, so a later stage cannot quietly absorb it:

- **It does not delete Janet.** The feature, `libjanet`, the Nix shell's janet
  input, and the port-4008 channel that ships Janet *source strings* from
  `modeler/tools/`'s 102 tests all survive the merge.
- **It does not move the modeler.** 18,792 lines, plus the 2,895-line prelude:
  scm's B5, sized 2.0, divisible by file. The Steel POC measured a prelude
  translation at 99.6% mechanical — believe the number, it was earned, but
  note it was measured for a different dialect.
- **It does not do goal (a)** — compiled fields callable from rayon workers
  (scm's B4, 0.8). The purity restriction that avoids the thread-local store
  collision costs nothing the survey could find: there is nothing in the
  corpus that wants scene data from a leaf function, and all 16
  `add-implicit` call sites compose Rust-side combinators.
- **It does not buy a user-visible speedup**, except P5's. The 2026-09-12
  measurement found no Janet hot spot in scene loading — a ~3,200× gap between
  interpreter time and the real work.
- **It does not make the UI chrome one language.** Two VMs live in one process
  from P1 until B5 lands. That is the cost of ruling 3, and it is the right
  cost: the alternative is one branch carrying 21,000 lines of translation.

## 8. Risks, worst first

1. **Two VMs in one process, for the length of two branches.** The hazard that
   matters is not memory, it is `with_builder`: it is a `RefCell`, reentrancy
   double-borrows and **hangs with no panic**
   (`reference_modeler_paint_never_touches_vm`). A second language that can
   re-enter it doubles the paths into that bug. Every scm host closure (§3's
   residue table) is such a path. Guard it the way `api::call` is guarded, at
   the one entry point, in P1 — before there are five of them.
2. **A long-lived branch while A4 and the modeler move on `main`.** Mitigated
   by §4's disjointness and by rebasing rather than merging back and forth.
3. **The 8 `eval`-shaped files.** Exempted, not solved, and the exemption is
   in the gate where it can be counted.
4. **"99.6% mechanical" is a number from a different dialect.** It is quoted
   for B5, not for this branch; nothing here depends on it being true.
5. **A panic cannot unwind out of `extern "C"` — it aborts**, and one killed
   the modeler mid-edit over a zero weight (@7d8c8a1). scm's embedding returns
   `Result` rather than unwinding, which is why it is listed fifth rather than
   first — but Rust calling *compiled* leaf code (P5) can no longer assume a
   Rust stack, and that is new.
