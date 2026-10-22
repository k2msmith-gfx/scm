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

**P1 as built, 2026-10-18 (ray `scm-port`, branched from `main` @fe22ddd,
tagged `api-refactor-complete`).** The merge was a fast-forward rather than
the `--no-ff` §4 wrote down — Kevin's call on 10-16 — so the tag marks the
branch point instead of a merge commit. What landed:

- `schemers = { path = "../scm/scm", optional = true }` behind a ray
  feature `scm`; `ray-scm` (`src/bin/ray-scm.rs`) is a stdin REPL and
  `--serve ADDR` speaks scm's own protocol, so the Emacs mode connects
  unchanged. Port 4009 by convention.
- `ray/src/api/scheme.rs`, the sibling `janet.rs` promised: `Val` ↔
  `ApiValue`, `%api-call` / `%api-error` / `%verb?` /
  `%api-value-roundtrip`, and the shims generated for **every** verb (183
  by the generator's count), in the familiar spelling S21 was built for —
  `(define (add-sphere :at (at #f) …) "doc" …)`. `the_api_layer_names_no_vm`
  now forbids `scm::` outside that file as it forbids `janet_` outside the
  other.
- **Three decisions that were not in this plan, made at the one place that
  could make them.** (1) *No nil:* `#f` in a slot declared `Bool` crosses
  as an explicit false, in any other slot as absent, inside a list as a
  false — §3(b)'s six producers are closed by the parameter's declared
  type rather than checked per verb. (2) *Nil out is `#f`, not
  unspecified:* an unspecified value is true in Scheme, so a lookup's miss
  would take the wrong `if` branch with no error; the price is `#f` echoed
  after every setter at the REPL. (3) *The error is parked, not returned:*
  `define_fn_with` prefixes an `Err` with the function's name, and the
  refusals are verbatim or nothing, so the shim raises `(error msg)`
  itself — `janet.rs`'s idiom, kept for the same reason.
- **Left for P2, visibly:** `ApiValue::Map` refuses to cross
  (`"a dictionary cannot cross into Scheme yet"`), so `(render-settings)`
  and every curve value fail at the REPL today; the aside (`fit-curve`'s
  deviation) is parked in one global `%api-aside` rather than given an
  idiom; the backtrace of a refusal names the shim's `(error e)` line in
  `api.rs` rather than the call site; `%rerun-form` (the modeler's door)
  is not registered. A whole number returns **exact** (`320`, not `320.0`).
- Verified in order: the seven tests in `scheme.rs` (set-film moves the
  store, every verb defined with its docstring byte-for-byte, a refusal
  and a type error verbatim through `guard`, `#f` absent-vs-false, a
  keyword bag, the round trip), then the four build configurations.

**P2 — the verb bridge (0.5; scm's B1).** `#[verb]`'s shim generator emits
Scheme as it emits Janet: keywords, `&opt`, `&named`, **error strings
verbatim**, and numeric coercion at the boundary so no `->f` appears in
authored code. `api/scheme/api.md` generated through the same `--gen-docs`
path and guarded by the same byte-comparison test.
Exit: **the conformance suite passes from Scheme, for all 178 verbs.** Not
Rust unit tests — A0's lesson is the reason this line is here: seventeen Rust
tests passed while a generated shim silently dropped every option, because
each test built the argument shape by hand.

**P2 as built, 2026-10-18.** Exit met: **335 of the corpus's 336 cases go
through the generated Scheme shims**, every verb in the registry is covered,
and the driver prints its one skip rather than hiding it. What the stage
decided, each at the one place that could decide it:

- **The dictionary is an association list with keyword keys** —
  `((:degree . 3) (:points (0 0 0) (1 1 1)))` — both directions. §3(a)
  left this open and recommended a constructor for the *material*; a curve
  is the other case, it has no constructor, and an alist is R7RS data that
  `assq` reads, `write` prints and a Racket programmer has seen. A hash
  table was the alternative and prints `#<hash-table>`, which is where a
  curve stops being inspectable at a REPL. The curve's **key order is
  kept**, which an alist does and a hash table would not, and a saved scene
  prints curves inline so a different order would be a different file.
  Implemented as `%api-wire` / `%api-unwire` **in Scheme, in the shim**,
  because an alist cannot reach Rust as data: a dotted pair is an improper
  list and `Val` carries one as `Opaque`. On the wire a map is a vector
  headed by `%map`.
- **A bag needs the nil rule's other half, and it is one sentence**:
  *inside a bag, `#f` is a false only as a keyword's value, and absent
  anywhere else.* Two calls need opposite readings — `(set-bloom nil)`
  turns bloom off, where a lone nil is the whole argument, and
  `:smooth #f` is the §4.8 hazard whose promotion to "default" turns flat
  facets smooth with no error. A lone `#f` cannot be an option value since
  options come in pairs, so position decides, and `add-sphere-mesh`'s two
  spellings are pinned in a test that reads the flag back off the recorded
  form.
- **The aside is a parameter named after it.** `(fit-deviation)` rather
  than Janet's `(dyn :fit-deviation)` — defined on first use, and
  `parameterize` rebinds it.
- **`api/scheme/api.md`** is generated by `ray-scm --gen-docs api/scheme`
  and guarded byte-for-byte by `generated_scheme_manual_is_up_to_date`, so
  all three manuals now have a drift guard. Its preamble carries the six
  reading rules above, because the docstrings are the Janet manual's text
  and still say `nil` and `[x y z]`.
- **No `->f` anywhere**: an exact `2` and `(/ 3 2)` both reach a number
  parameter, tested.
- **One performance finding, and it was the stage's own doing.** Wiring
  rebuilt every argument structure at first, since `map` is the obvious way
  to walk one, and an `(add-mesh …)` of 20,000 points and 20,000 triangles
  spent 209 ms there — the whole cost of the call, so a mesh-heavy scene
  had doubled. Detection allocates nothing where rebuilding allocates a
  cell per sublist, so the wire now looks before it rebuilds: **209 ms →
  68 ms, and the whole call 199 ms → 64 ms**, release. What remains is the
  look, ~160,000 Scheme calls at ~0.4 µs, which is the price of a rule only
  Scheme can apply. §7's "no user-visible speedup except P5's" needs its
  mirror image written down: **no user-visible slowdown either**, and a
  boundary that walks has to be measured on the corpus's biggest argument
  rather than its typical one.

Two findings that are **ray's, not the port's**, both pinned by tests:

- **`%rerun-form` refuses every rest-bag verb**, and Janet refuses
  identically on `main` (checked the same day). `call_form` maps a recorded
  `:at` onto a *declared* parameter and a bag verb declares one parameter
  called `args`, so the shape verbs were never re-runnable through that
  door. The modeler's callers are all surfaces, which declare their
  options, which is why nobody had hit it.
- **`Bag` reads a dual-form call as positional OR keyword and never half of
  each** (`Bag::val`: any leading positional turns named lookup off), so
  `(add-sphere #f :radius 2)` refuses. Worth knowing before P3 writes
  recipes.

### First Janet-against-scm numbers in ray, 2026-10-18

Asked for after P2 landed; no comparison existed before this. Both binaries
from **one** `--release --features janet,scm,oidn` build, same machine, best
of 3 (boot best of 5), in-script monotonic clocks for the work so process
spawn is excluded. The scripts are matched form for form and are in the
session's scratchpad; the calls are identical, `(add-mesh pts tris)` on
both sides.

| | Janet | scm | |
|---|---|---|---|
| boot: process start to exit, nothing to do | 16.7 ms | **6.1 ms** | scm 2.7× faster |
| build loop: 2,000 spheres, trig per shape | **4.2 ms** | 12.1 ms | Janet 2.9× |
| the same, per shape | **2.10 µs** | 6.06 µs | |
| 20,000 points + 20,000 triangles built in-language | **2.2 ms** | 11.3 ms | Janet 5.1× |
| `(add-mesh …)` with them — the crossing | **6.0 ms** | 61.1 ms | Janet 10.2× |
| the same crossing, dictionary look bypassed | 6.0 ms | **6.9 ms** | a wash |

**The value model crossing is a wash, and that is the result that matters.**
6.9 ms against 6.0 ms for 40,000 structures over the boundary: the generated
shim, `ApiValue`, the dispatcher and the verb cost the same in both
languages, which is what P2 was built to find out and could not be known by
reading.

**The 10× is entirely §3's dictionary look**, measured rather than inferred:
the look alone is **53–55 ms** of the 60 ms shim. It exists only because
Rust cannot see a dotted pair — `embed::Val` has no pair variant, so an
alist arrives as `Opaque` and the alist rule has to be applied in Scheme,
on every argument, because a dictionary can be anywhere in one. Narrowing
it per verb does not work: `add-mesh`'s `vertices` and `indices` are
declared `Any` precisely because the verb reads them itself, so the verbs
with the biggest arguments are exactly the ones a type mask cannot skip.

**Recommendation, for the branch rather than for P2: add `Val::Pair` to
scm's `embed` and delete the Scheme-side wire.** It removes the cost
instead of reducing it — the crossing becomes the 6.9 ms already measured,
level with Janet — and `Val` is `#[non_exhaustive]` for exactly this. It is
a Track A change serving Track B.

**DONE the same day, at Kevin's word.** `scm::Val::Pair(Box<Val>,
Box<Val>)`: a pair chain ending in `'()` is still a `List`, so a host that
only knows `List` sees exactly what it saw before, and an improper one is
the cons chain it is. One variant, one fold, three sites in `embed.rs`
(both readers and the builder), and **one existing test had to change its
mind** — `what_has_no_host_form_says_so_instead_of_guessing` asserted that
`'(1 . 2)` was `Opaque`, which was the limitation, not a property. A cycle
is still `Opaque`, caught by the length cap rather than the improper arm,
because a cycle's tail is always another pair. ray's `%api-wire`,
`%api-unwire`, `%api-any?`, `%api-alist?` and the `%map` wire tag are all
deleted; the rule is `as_alist` in `scheme.rs`, applied during the walk
`to_api` was already doing.

**The rule has a subtlety that cost an afternoon, and it is cons being
cons.** An entry has two spellings that are *the same datum*: `(cons :k v)`
is a pair, but when `v` is itself a list that very cons **is** a proper
list, so `(:points (0 0 0) (1 1 1))` arrives as `Val::List` and never as a
`Val::Pair`. The first Rust rule looked only for pairs and refused every
real curve. An entry's value is its **cdr** under either spelling, which is
what the Scheme rule it replaced computed as `(cdar l)`.

| | Janet | scm before | scm after |
|---|---|---|---|
| boot | 18.4 ms | 6.6 ms | 6.6 ms |
| build loop, 2,000 spheres | 4.8 ms | 12.1 ms | **6.3 ms** |
| the `add-mesh` crossing | 6.3 ms | 61.1 ms | **7.8 ms** |

**The wire was taxing every call, not only the mesh ones** — which is
obvious afterwards and was not predicted: the build loop halved too,
because each of its 2,000 calls was walking a three-element vector for
dictionaries that could not be there. Janet's own figures moved 5–10%
between runs on an idle machine, so **1.24× on the crossing and 1.31× on
the loop are the same class**, not a measured deficit. The remaining ~1.5 ms
on the mesh is the Rust-side alist check: 40,000 `entry` calls that each
refuse at their first element.

Every gate on both sides is green, conformance unmoved at 1112/71, zero
clippy warnings.

**One hazard this turned up for P3, recorded where the printer will meet
it: the two languages print the same `f64` differently.** Janet writes 14
significant digits (`3.06161713146292e-17`), scm writes round-trip 17
(`3.0616171314629196e-17`), and `1e+21` against `1.0e+21`. P4 compares
`Form` trees rather than text precisely so this cannot matter — but anyone
who reaches for a text diff between a scene saved under each language will
find differences that are not differences.

**The look cannot be fixed in Scheme, and that is measured too.** On the
same argument: walking only the two spines is 2.9 ms, *visiting* all
160,000 cells with a one-line `touch` is **26–29 ms**, and the shipped
predicate is 52–54 ms. So a perfect Scheme predicate still costs ~27 ms —
**four times the entire rest of the call** — because the work is visiting
every cell in interpreted code. Rust already walks the same structure
inside `to_api` as part of the 6.9 ms.

The asymmetry has a one-sentence cause. **Janet's dictionary is a distinct
type the boundary recognises in passing; Scheme's alist is a *shape*, and a
shape can only be recognised by inspecting every element** — which is
exactly the information `Val` withholds. Nothing about the choice of an
alist was wrong (§3's reasons stand, and it reads well); what is wrong is
where the recognition has to happen.

Worth fixing before P4 rather than after: P4 loads all 191 scenes, and the
tax is ~2.5 µs a vertex, so a 200,000-vertex import pays half a second of
pure look.

**scm's interpreter is 3–5× slower at script arithmetic and list
building**, which is the known price and not a surprise: [vs-steel.md](vs-steel.md)
puts T1 at 0.91× Steel on ordinary code, and the Steel POC measured the
same shape at ~6× Janet. §7 already says this branch buys no speedup except
P5's, and the 2026-09-12 finding stands — scene loading is not
interpreter-bound, with a ~3,200× gap between interpreter time and the real
work.

**Boot's 2.7× will narrow and should not be banked.** Janet's 16.7 ms loads
8,161 prelude lines; scm's 6.1 ms installs 183 generated shims and has **no
ray prelude at all** until P3. Re-measure it there.

**A by-product worth recording, because it retires an old complaint.** The
Steel POC's standing finding about ray was that `%add-mesh` cost 22.8 ms for
a 3,507-vertex mesh — 6.5 µs a vertex — against 5.5 ms for the same data as
a flat `Vec<f64>`, "a 4× boundary gap on every mesh-heavy scene". Janet's
crossing here is 6.0 ms for 20,000 vertices, **0.30 µs a vertex**: roughly
20× cheaper per vertex, across different builds of ray but the same machine
class. A3's one dispatcher appears to have fixed it as a side effect.

**P3 — the printer and the prelude (0.4).** `Form::to_scheme()` beside
`to_janet()` (`ray/src/form.rs:376`), carrying §3.5's spelling table and §3's
(a)–(d) — including the one decision §3.5 leaves open, the net's constructor;
and
`scripts/scm/` — §3's residue table, written in Scheme: **68 procedures and
4 macros**, with the seven vector helpers deleted rather than translated and
the four prims and ~13 accessors registered rather than rewritten. Exit: every corpus scene, saved and
printed as Scheme, **reads back to an equal datum** in scm. That is scm's own
S1 round-trip gate, now run over ray's real output.

**P3 as built, 2026-10-18 — the printer is done, the prelude is half.**

The printer (`ray/src/form.rs`'s `Form::to_scheme`, `Line::to_scheme`)
carries §3.5's table and closes two of §3's open questions. **The net needed
no new verb**: P2's alist crossing let it print as the association list
`nurbs-net` already returns and `add-nurbs-surface` already reads, which is
better than either option §3(a) offered. And **one rule decides every
container** — a constant when everything inside it is data, a constructor
call when something has to evaluate — forced by a real scene holding
`[(load-texture …)]`, where a quoted vector would store the list.

Proved end to end on real scenes rather than hand-built forms, and the
hardest one in the corpus works: `dining-room-baked.janet`, with six
textures, split mesh files, includes, groups and materials, goes to Scheme,
loads under scm, saves back as Janet, and **renders to the same hash as the
original**. A plain scene and a grouped scene both round-trip
**byte-identically** through scm.

**What a saved scene can call turned out to be a closed, short list**, which
is the measurement that scoped this stage: of every head
`snapshot`/`src/api` can emit, exactly four are not registry verbs — `%tex`,
`def`, `with-group` and `include`. So the prelude's *loading* half is
complete at four items, and everything else in §3's residue is an
*authoring* helper that no saved scene ever names.

- `material` with its eager validation and its once-per-session specular
  advice; `shader-code`; the three packed-buffer unpackers; `with-scene`,
  `with-group`, `defshape`, `defgroup`, `in-group`; `include`. The seven
  vector helpers are deleted, as §3 said.
- `scene_store::open_group` / `close_group` are **lifted out of the Janet
  prim**, so both bindings share one rule about when a group is stored, and
  the Scheme side registers the prims rather than reimplementing them —
  which is what §3's "registered rather than rewritten" asks for.
- A captured group body crosses as DATA through a new `to_form`, simpler
  than Janet's because a Scheme reader already distinguishes a list from a
  vector, where Janet needs a parse flag to know which tuple was a call.
- `save-scene` picks its dialect from the extension, and a `.scm` scene's
  split mesh parts are Scheme too.

**A CAPTURED BODY CANNOT CROSS DIALECTS, and that is now a rule rather than
a bug.** `with-group` records its body as the author wrote it, and that body
is arbitrary code: a Scheme session's is `(vector-ref %tex 5)` where Janet's
is `(%tex 5)`. Saving a Scheme session as `.janet` emitted the former and
the reload died on `unknown symbol vector-ref`. The origin now records
*which* dialect captured it, and a save to the other one **expands** the
entry instead — which is what `repro: false` has always meant and is
dialect-neutral by construction. Found only because the render comparison
needed the Scheme→Janet direction.

**Two pre-existing ray findings, neither the port's:**

- **`save-scene` can write a scene that cannot be reloaded.**
  `examples/models/lighthouse.janet` saves `(with-group :railing (railing))`
  — the captured body calls a function defined in the *original* scene file,
  which the save does not carry. The Janet save fails to reload with
  `unknown symbol railing`, so this is not about Scheme at all. Source
  capture records the text, and the text can reference an environment the
  saved file does not reconstruct. P4 has to exempt such scenes or ray has
  to stop claiming them reproducible.
- The corpus read gate is **opt-in, not a leg**, because 190 scenes in one
  process is an hour and the cost is the scene *build* (`arcology` over a
  minute, the terrain scenes ~20 s each). Last run reached 188 of 218 files:
  **140 printed as Scheme and read back, 48 not-a-scene, 0 failures.**

**THE PRELUDE IS DONE — 67 definitions, 1,351 lines**, with three items
deliberately left to the stages that own them. Every port was checked
against the Janet original rather than read over, and four of them
numerically:

| | checked how |
|---|---|
| `load-gltf` | loads a real model, fails cleanly on a missing one, records a form that prints back |
| `make-texture` | the same generator in both languages gives a **byte-identical texture** |
| `camera-pixel` | three points, **identical to the last digit** |
| `camera-match` | the docstring's own two-vanishing-point example, **all six outputs identical** |
| `strut-mesh` | 163 verts and 216 tris both sides, same first vertex, normal and index |
| `sphere-points` | five points of an unjittered lattice, identical |
| `add-implicit` | a clipped gyroid round-trips byte-identically and **renders to the same hash** |
| `camera-frame` | all six fields identical |
| `render-settings` | the documents differ in **three explained lines** and nothing else |

**§3(d) IS RESOLVED, with the mechanism the plan recommended.**
`camera-match` reached `set-camera` out of the environment at call time so
the modeler's shadowing definition won; scm copies bindings on import, so
`*set-camera*` and `*set-film*` are parameters that default to the verb. A
host that needs to intercept the write **binds the parameter** instead of
shadowing a name — an explicit indirection the host owns, which is exactly
what §3(d) asked for, and it is cheaper to read than the lookup it replaces.

**Six prim bodies were lifted into shared Rust** (`scene_store`), so the two
bindings cannot disagree: `open_group`, `close_group`, `load_gltf_group`,
`add_generated_texture`, `add_implicit_field`, `frame_bounds`. Janet's prims
are now three lines over each. Two conventions stopped being Janet's on the
way: the material override is an **argument** rather than the pending slot
`mat-emit` filled, and `%fnum` shares `form::format_num` so both languages'
settings documents agree rather than nearly agreeing.

**Four findings the ports turned up, each now carrying its comment:**

- **A verb's `[x y z]` comes back as a LIST**, and `(scm linalg)` wants a
  vector. Nothing in the value model can tell a coordinate from a list of
  three radii, so `%v3` coerces where a verb's triple meets vector maths.
- **In a captured form, a list with a SYMBOL head is a call and anything
  else is data.** Janet got that from its reader's bracket flag; Scheme has
  to get it from the head. Recording an implicit surface found it — the
  field tree `(:intersect 0 …)` went in as a call and came back as "a
  form's head must name a verb, not Int(0)".
- **An empty list is an empty dictionary.** `(material)` with no options is
  `'()`, and in Scheme that is both — so `mat_builder_of` reads an empty
  list as the default material. A non-empty list is still refused.
- **A `:seed` reproduces within a language, not across them.** The same
  seeded tree has the same structure (15 branches at depth 4) and different
  geometry, because Janet's `math/rng` and the Scheme generator are
  different algorithms. **Kevin's call**: exempt seeded generators from
  P4's render comparison, or move the RNG into Rust so both agree — which
  would change what existing Janet seeds produce.
- **The nil→`#f` collapse is visible in exactly one place**, and it is a
  view: `render-settings` cannot tell "denoise never set" from "set to
  false", so it says `off` where Janet says `default`. Both are true.

**What is left, and each belongs to another stage rather than to P3:**

| what | whose | why |
|---|---|---|
| `field-bake`, and `add-implicit`'s authored-field branch | **P5** | the host must call a Scheme procedure per sample — the crossing the leaf compiler exists to make fast. `add-implicit` refuses it by name rather than half-working, and the whole corpus holds exactly ONE authored field |
| `origin-form`, `follow-curves` | **B5** | modeler-facing: 30 call sites in `modeler/scripts`, **0 in `examples/`** |

**And one number P5 should carry**: a 256² texture is Janet 502 ns/texel
against Scheme 1,728 — **3.4×**. A texture-heavy scene is materially slower
under Scheme until the leaf compiler lands, which makes P5 more valuable
rather than less.

**P4 — the corpus converts and agrees (0.3; scm's B2).** For each of the 191
renderable scenes: load under Janet → save → print Scheme → load under scm →
save → **compare the two `Form` trees**.

**And compare the IMAGES too — this stage's reason for not doing so was
wrong, measured 2026-10-18.** The plan said "scene files, not image bytes"
on the authority of a note claiming rayon's work-stealing reorders float
accumulation. It does not: `render.rs` parallelises over *pixels*, a pixel's
samples are summed inside one task, and every random sequence is seeded from
the pixel's global index. Checked by rendering one scene three times under
`:whitted` and twice under `:path` — **byte-identical PNGs every time**,
while the reported `Mpix/s` moved by 1.7x. The annotation stamp that used to
burn a timing into the pixels is **no longer used**, and was off by default.

So an image hash is available and is the *stronger* of the two checks,
because it sees what the authoring form cannot describe: the Steel POC
caught `%add-cuboid` building a mesh where ray builds an analytic cuboid,
with the images indistinguishable and only the clock showing it. Do both —
the form trees say the scenes are the same description, the hashes say they
are the same picture.

**Kevin's ruling, 2026-10-18: a scene that draws random numbers is compared
by its SCENE, not its pixels.** "If they can't generate exactly the same
results between Janet and scm, then leave [the generators] in scm and
exclude from the rendering test compare."

Why they cannot: Janet's `math/rng` and the Scheme generator are different
algorithms, so the same seed walks a different sequence. Matching exactly
would mean reproducing `janet_rng_u32` and its seeding bit for bit. The
state's *shape* is recoverable — `struct JanetRNG { uint32_t a, b, c, d; }`
is in the installed `janet.h`, so it is a four-word 32-bit generator Scheme
could carry — but the mixing function and the seed constants are in
`math.c`, which the Nix package does not ship, and three sample outputs are
not enough to identify an algorithm *and* its warm-up. So: not matched, and
the exclusion stands until someone has the source in front of them.

**The exclusion is narrow, and only the image half of the check is dropped.**
A generator's recorded form is identical in both languages — the same call
with the same `:seed` — so the **form-tree comparison still covers these
scenes**, and it is the comparison that proves the port. Only the pixels are
skipped.

**Measured, so the skip list is not curated:** 24 of the 191 renderable
scenes draw a random number while building — 14 call `math/random`, 10
`math/seedrandom`, 5 `grow-branches`, 5 `sphere-points`, 2 `strut-mesh`, 1
`math/rng` (the sets overlap). The rule a gate can apply is textual: a scene
mentioning any of those names is compared by scene only, and says so in its
report.

**And note what this is NOT.** Janet is reproducible against *itself* either
way: the same generator scene rendered twice gives one hash, seeded or not
(measured). So the default RNG is deterministically seeded, the 167 other
scenes are fully render-comparable, and nothing here weakens the check for
them.

**Kevin's other ruling, same day: compare renders, but not the slow files.**
So the image half is **bounded and reported**, exactly as
`run_corpus.sh` already bounds its own run: render both sides at a cheap
setting, give each scene a timeout, and print which scenes were skipped for
time so a skip can never be read as a pass. Note *where* the time goes
before choosing the knobs — at 64×48 and one sample the render is trivial
and what remains is the **scene build**: `fruit-bowl` spends 15.22 s in
`make-texture` and the terrain scenes spend ~20 s in fBm. That is why a
timeout is the right instrument and a lower sample count is not: it bounds
the cost wherever the cost actually is. It also means the slow scenes are
precisely the ones P5 makes fast, so the skip list should shrink on its own
rather than being curated. Reuse `run_corpus.sh`'s classifier for
the 27 non-scenes; name the 8 `eval`-shaped files as exemptions in the gate
itself, so the exemption is visible rather than a silent skip.
Note what a save *is*, because it bounds the claim: `save-scene` preserves
**state, not source shape** — loops, helpers and `defshape` come out as the
flat calls they made. This stage proves the corpus's *scenes* port. It does
not prove its *source* ports, and nothing on this branch does.

**P4 MET, 2026-10-18 — all 218 example scenes, nothing failing.** The whole
corpus, four ray processes and two renders per scene, 150 s per step:

| | scenes |
|---|---|
| scene AND render agree | 150 |
| scene agrees, render skipped — the scene draws random numbers | 21 |
| scene agrees, nothing to render — mesh data, no camera | 19 |
| not a scene — mesh data, or another harness | 9 |
| over the step timeout | 0 |
| ray cannot reload its own save (not the port) | 3 |
| differ only by a zero's sign, which is Janet's (not the port) | 16 |
| **FAILED** | **0** |

So **190 of the 218 are compared and agree**, 150 of them on their pixels
as well; 9 are not scenes; and the 19 remaining are ray's or Janet's, named
one by one below. No scene is skipped silently and no class is curated — a
scene lands in a column because of what it DID, which is why four of these
columns were discovered rather than designed.

**The driver, and the five things it found.**
`scripts/dev/scm_corpus.py` in ray. Per scene: load under Janet and save
twice over, once as Scheme and once as Janet; put the Janet save through
Janet (the **control**) and the Scheme save through scm (the **subject**);
compare the two results byte for byte; then render both and compare hashes.

**Why a control rather than the first save.** `save-scene` is not idempotent
in its own language — a scene holding a GENERATED texture says "referenced
by their live ids" the first time and "a texture with no path of its own"
the second, and a group whose members cannot be expanded comes back empty
and an empty group is deliberately not stored, so it vanishes. Measured on
`arcology.janet` and `baked-field.janet`. Comparing Scheme's round trip
against Janet's cancels every one of those, because both sides have been
through one load-and-save. What is left is the port.

Four classes of disagreement came out of it, and **two of them are the
port's, two are not**:

1. **A material's key order is the host language's, not the scene's** (the
   port's, fixed). Janet printed `:reflectivity` before `:specular`,
   Scheme the other way, and five scenes differed by nothing else
   (`calder-arch-2`, `calder-spire`, the three `krell` scenes). A Janet
   dictionary is a hash table and hands its pairs over in bucket order; a
   Scheme association list hands them over in source order. `sort_pairs`
   already sorted — but only at `Lit9`, and a captured group body prints at
   full precision. Now it sorts at every precision, in both printers.
2. **A material arriving as a CALL had to print as Janet's literal** (the
   port's, fixed earlier in P4). A scene that went out through Scheme comes
   back with `(material …)` inside a captured body; printing it verbatim
   made the Janet output differ from Janet's own by exactly the material's
   spelling. Each dialect now renders a material in its own idiom whichever
   shape it arrived in.
3. **A dialect's SPELLING has to come back canonical** (the port's, fixed).
   The texture header binds `%tex` to a vector of `load-texture` results;
   Janet's data structures are callable so a material says `(%tex 5)`, and a
   Scheme vector is not, so the Scheme printer renders the same index as
   `(vector-ref %tex 5)`. Reading that back as a call to `vector-ref` made
   the captured body non-portable, so the entry was EXPANDED rather than
   re-emitted — identical geometry, in block layout, and
   `dining-room-baked.janet` differed from Janet's own save by six newlines.
   The reader now turns the spelling back into the canonical `%tex` head, so
   one shape sits in the store whichever dialect typed it and each printer
   renders its own sugar. The lesson generalises past this case: a printer
   that spells something for its dialect owes a reader that spells it back,
   or the store ends up holding two shapes for one value.
4. **Janet cannot round-trip the sign of a zero** (not the port). Ten
   scenes — the five ASK-21s, two Calders, three Krells — differed in one
   coordinate: a curve whose first and last control points hold the same
   numbers, the first `-0` and the last `0`, came back from JANET with the
   last one `-0`. Janet's compiler puts a literal tuple in the constant
   pool, deduplicates value-equal constants, and in Janet `-0` equals `0`,
   so both points become the one that got there first. Three literal points
   reproduce it with no curve and no ray involved. **Scheme keeps the file,
   Janet rewrites it**, so the port is the faithful side.

   **Either orientation shows up, and both are the same fault.** Which copy
   survives is whichever the list holds first: on `ask-21-drawn` the control
   gained a `-0` the file did not have, and on
   `ask-21-measured/ask21-controls` it lost one the file did. A first cut at
   the test assumed the first orientation and turned the second into a
   failure. What the test actually asserts is narrow in the way that
   matters: every differing token must be a zero on BOTH sides, differing in
   nothing but its sign. A number that moved by any amount at all is still a
   failure.
5. **Three scenes ray cannot reload from its own save** (not the port,
   pre-existing). Its save contains `(with-group :railing (railing))`
   — a captured body calling a function that lived in the original scene and
   is absent from the save — so the reload dies on `unknown symbol railing`
   in Janet and `unbound variable: railing` in Scheme. **A partial save is
   not a successful reload**: `load-scene` aborts but ray's stdin REPL runs
   the next form, so the `save-scene` after it wrote a 39-line file from a
   48-line scene and the file's existence read as success. The driver reads
   what ray SAID, not whether a file appeared. The other two are
   `fall-tree.janet` and `tree-with-leaves.janet`, which fail differently
   and for a reason of Janet's: **"ran out of internal registers"** — a
   generated tree's save is one enormous form and Janet's compiler has a
   per-function register budget. Both are ray's own saves failing in ray's
   own language, and scm loads neither file either because the driver never
   gets that far.

**P5's prerequisite, measured 2026-10-19: the corpus writes its texel
functions as closures, and the leaf compiler cannot read a closure's
environment.** Every `make-texture` call in ray's 25 texture scenes passes
an INLINE LAMBDA that closes over local numbers — `(make-texture res res
(fn [u v] … rings warp early late …))` in `wood.janet`, the same shape in
`marquetry.janet` and `fruit-bowl.janet`. S11 resolves a free variable as a
local, a global procedure, a pure primitive or a global table; a variable
that is none of those is unbound. Probed directly rather than inferred:

```
(define (plank rings warp) (lambda (u v) (* rings (+ u (* warp v)))))
(define tex (plank 7.0 0.25))
→ `rings` is unbound (in tex)          ; the closure
→ Ok(arity 2)                          ; the same body with a global instead
```

So P5 starts in THIS repo, not in ray: the leaf compiler needs a free
variable to resolve through the compiled procedure's captured environment
(a number as a constant, a vector as the snapshot item (2) already takes),
and an entry point that takes a PROCEDURE VALUE, because `make-texture`
receives `f` as an argument and never as a global name. **S11b.**

**T2 is not the alternative, also measured (S13):** 1.3× on `tak` and `fib`,
0.65× across the 53 programs once compilation is paid for, and the native
code is 6% of the profile — the call protocol is the ceiling. The leaf
compiler's 77 ns/texel against Janet's 4,112 is the only lever that moves
this number, and it is restricted to a subset for exactly that reason.

**P5 — compiled texels (0.4; scm's B3, goal (d)).** The one measured number
in the whole argument: `make-texture` at **4,112 ns/texel**, one 2048² texture
at **17.2 s**, `fruit-bowl.janet`'s whole scene build at **15.22 s**. S11's
leaf compiler, behind a ray feature so a plain build needs no LLVM. Exit: the
new number printed beside the old one, and the baked image compared against
the interpreted bake. `make-texture` is already a bake, so this changes
nothing observable except the seconds.

**P5 MET, 2026-10-19 — 18x faster than Janet, and the same image.**
ray's `scripts/dev/texel_bake.py`, `texel_bake.scm`, `texel_bake.janet`;
`make-texture`'s fast path is `%bake-texels` in `ray/src/api/scheme.rs`,
behind ray's new `jit` feature.

The fixture is `examples/procedural/wood.janet`'s own texture — the quintic
fade, the 127.1/311.7 hash, five octaves, the two-colour blend — translated
faithfully and baked three ways on one machine, 512x512:

| | ns/texel | one 512² texture |
|---|---|---|
| `make-texture`, scm **compiled** | **215** | **56 ms** |
| `make-texture`, janet | 3,925 | 1,029 ms |
| `make-texture-interpreted`, scm | 30,900 | 8,102 ms |

**18.2x against Janet and 144x against the interpreter.** The compiled
figure INCLUDES the ~20 ms compile, which is 76 ns of it amortised over
262,144 texels; the steady-state texel is ~140 ns.

**And the images are byte-identical — all three of them.** That is the exit
criterion's other half, and the comparison is stronger than it was asked to
be: not only does the compiled bake match the interpreted one, it matches
JANET's bake of the same texel, bit for bit. `%bake-texels` carries the
prelude's `%texel-byte` rule into Rust and multiplies by the same reciprocal
the Scheme loop does, so the two produce one image rather than two
similar-looking ones.

**The fallback is the design, not a safety net.** `%bake-texels` answers
`#f` when the texel procedure is outside the leaf subset — printing the
refusal, which names the call that did it — and the prelude runs the
interpreted loop. Without the `jit` feature the prim answers `#f` always.
So the Scheme has no build test in it, a scene behaves identically either
way, and ray's five gate legs are untouched: the feature needs LLVM, so it
is in no leg, exactly as `scm-llvm` is not a default workspace member here.

**What this cost on the scm side was a stage of its own**, because the
corpus writes its texel functions as closures and S11 could only compile a
global: see S11b above, which is the prerequisite and where most of the
work was.

**The path dependency makes a lone clone of ray unbuildable, and that is
P6's problem rather than P1's (measured 2026-10-18).** A missing optional
path dependency breaks even a *default* `cargo build` — Cargo resolves the
manifest before it looks at features, so `scm = []` being off does not save
it; verified with a throwaway crate pointing at a path that does not exist.
So while `scm-port` lives, ray builds only on a machine with `../scm`
beside it, and `git clone ray && cargo build` fails at the manifest with
*No such file or directory*.

Nothing breaks today: ray's CI triggers on push to **`main`**, on
`pull_request` and on `workflow_dispatch` only, and this is a branch. But it
is a **merge blocker for P6**, which has to pick one of: a git dependency
(needs `k2msmith-gfx/scm` reachable by the build — it is private, so this
means a token or making it public), publishing `schemers` to crates.io
(which [plan.md](plan.md) gates on Track B finishing, i.e. on this), or
vendoring. Decide it in P6 rather than at the merge.

**P6 — scm becomes the default; Janet stays reachable (0.2).** Flip the
default feature, run the corpus gate on the scm build, and keep
`--features janet` building with its tests passing. Exit: the merge.

**P6 as built, 2026-10-19 — the flip and the blocker, with two steps left.**

`default = ["scm"]` in ray's manifest. A bare `cargo build` is the Scheme
build and produces `ray-scm`; `--no-default-features` is pure Rust;
`--features janet` is one flag away and green on its own (884 tests) and
beside the Scheme (901). Counts for the legs are in ray's CLAUDE.md, which
now spells all seven.

**The flip moved a leg that nobody would have noticed.** `modeler` depended
on ray with default features, and Cargo unifies features across a workspace
build — so `cargo test --no-default-features` at the root still compiled
the Scheme in, because the modeler asked for it. The pure leg reported 681
where it should report 664. Fixed by `default-features = false` on the
modeler's dependency, which is also the honest statement: the modeler names
the scripting layer it wants.

**THE MERGE BLOCKER IS RESOLVED, and the plan's reason for it was wrong.**
It said the git-dependency option needed "a token or making it public"
because the scm repository is private. It is not: `k2msmith-gfx/scm` is
PUBLIC (ray is the private one). So ray now takes `schemers` and `scm-llvm`
from `https://github.com/k2msmith-gfx/scm.git` with `branch = "main"`, with
no token, no crates.io release and no vendored copy, and Cargo.lock pins
the commit. Publishing the crate stays [plan.md](plan.md)'s business and is
no longer in ray's way.

Developing the two together is a gitignored `.cargo/config.toml` holding a
`paths` override at the sibling checkout — `paths` rather than `[patch]`
because a patch is recorded in Cargo.lock and toggling it would never leave
the working tree clean.

**What is left, and both need Kevin:**

1. **Push scm.** The pinned commit is the last PUSHED one, which predates
   S11b and the embedding accessor, so a clone of ray today would fetch an
   scm that cannot compile a closure. Push, then `cargo update -p schemers
   -p scm-llvm` in ray to move the pin, and verify a clone with no `../scm`
   beside it builds.
2. **The merge itself.**

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
- **It does not make output stream, and it does not need to.** Checked
  2026-10-22, both sides: scm captures a form's output into a string port and
  hands it back with the answer (measured — 2.00 s of silence, then
  everything), and so does the side it replaces. `scene_store.rs` says it
  outright: *the TCP REPL, the modeler's eval channel and the agent's `eval`
  tool all hand back the value plus whatever `:out` captured*. So this is
  **parity, not a regression**, and no stage here is gated on it. It also does
  not reach ray's long operations — a render broadcasts framebuffer strips to
  a display thread, not text, so the 20 s terrain build was never telling a
  script anything. The open question, and the sequencing argument for leaving
  it until after the modeler port, is [plan.md](plan.md) §13.3.

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
