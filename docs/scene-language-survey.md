# What ray's scene files require of the language

2026-10-12, measured against ray @19bb6c5: 219 scene files in `examples/`,
21,348 forms, 1,342 distinct heads.

This is the "vision toward porting the Janet scenes" half of the project,
written down as a checklist rather than carried as an intention. It is
deliberately **not** about ray calling scm -- that is the adoption project
(plan §7, Track B). It is about one question scm can answer on its own:

> If a scene file were converted to Scheme, is there a language under it that
> can run it?

Everything here is answerable without a renderer, which is why it belongs to
scm's own track.

---

## 1. The surface is smaller than feared, with four sharp exceptions

Scene files use a modest slice of Janet. Counted by construct, not guessed:

| | what the corpus uses |
|---|---|
| binding | `def` 4116, `defn` 827, `defn-` 276, `var` 229, `let` 195, `set` 152 |
| conditionals | `if` 275, `when` 74, `cond` 48, `unless` 22, `case` 8, `when-let` 6, `if-let` 5 |
| iteration | **`seq` 253**, `for` 174, `each` 156, `loop` 30, `while` 25, `repeat` 20 |
| higher-order | `map` 343, `reverse` 23, `filter` 7, `reduce` 3, `sort`/`sort-by` 3 |
| sequences | **`array/push` 248**, `length` 158, `get-in` 77, `put` 65, `get` 51, `first`/`last` 44 |
| math | `math/sin` 295, `math/cos` 265, `max` 243, `math/abs` 148, `min` 146, `math/sqrt` 125, `math/floor` 114, `math/pow` 105, `inc` 84, `dec` 67, `math/acos` 28, `mod`/`%` 45, `math/round` 16, `math/random` 15, `math/atan2` 6 |
| strings | `string` 75, **`scan-number` 45**, `string/format` 14 |
| dynamic | `setdyn` 33, `dyn` 26, `with-dyns` 1 |
| other | `keyword` 101, `default` 53, `os/getenv` 52, `printf` 45, `print` 25, `os/clock` 20, `nil?` 13, `assert` 11, `try` 9 |

And the reassuring absence: **`defmacro` appears zero times in the scene
corpus.** The 4 procedural macros the ray-steel census found are in the
*prelude*, not in scenes. Whatever `syntax-rules` cannot express is a Track B
problem, not a scene problem.

Most of the above maps straight across: `def`/`defn` → `define`, `fn` →
`lambda`, `do` → `begin`, `set` → `set!`, `var` → `define`, the predicates and
the math to their R7RS spellings (`math/pow` → `expt`, and R7RS's two-argument
`atan` already *is* `math/atan2`). `let`, `when`, `unless`, `cond`, `case`,
`and`, `or` are ordinary derived forms and arrive with `syntax-rules` at S5.

Four things do not map, and they are the actual content of this survey.

---

## 2. `array/push` — 248 uses, 48 files, and R7RS vectors cannot grow

The single largest gap, and it is structural rather than cosmetic. Janet's
arrays are growable; **R7RS vectors are fixed-length and there is no
growable sequence in the standard at all.** 248 uses across 48 of 219 files is
not something to work around at each site.

The scene idiom is always the same shape -- accumulate into an array inside a
loop, then hand the array to a verb:

```janet
(def pts @[])
(for i 0 n (array/push pts [x y z]))
(add-loft pts ...)
```

Three ways out, and this needs deciding before S5 designs the stdlib:

1. **A growable vector type** (`make-growable-vector`, `vector-push!`, …) as a
   documented non-standard addition. Preserves the idiom one-for-one, so the
   conversion is mechanical. Costs a second vector-ish type in the language
   and in every `vector?`-shaped predicate.
2. **Convert to accumulate-and-reverse** -- `(let loop ((i 0) (acc '())) …)`
   building a list, reversed at the end. Pure R7RS, no new type, and it
   rewrites the *shape* of 48 files rather than translating them. An LLM can
   do it, but it is the kind of rewrite that introduces the
   compiles-runs-looks-right-and-is-wrong bug the conversion hazards warn
   about.
3. **`(scheme vector)`-style resizing helpers over a boxed vector** -- a
   `vector` in a mutable box, reallocated on growth. Hides inside a small
   library, keeps one vector type, costs an indirection nobody will notice in
   a scene build.

**Recommendation: (3), presented as (1).** One underlying vector type, a small
growable wrapper in the stdlib, and the push/pop names the corpus already
uses. Decide at S5, not at conversion time.

## 3. `format` does not exist in R7RS, and the corpus needs it

`string/format` 14, `printf` 45, `errorf` 2 -- 28 of 219 files. The ray-steel
POC hit exactly this and had to rebuild printf from `quotient`,
`number->string` and manual padding; the ray-steel conclusions list it as one
of the three stdlib holes that turned up in every area they looked at.

scm ships a `format`. It is not optional and it is not large.

## 4. Seeded randomness, with reproducibility

`math/random` / `math/rng` in 6 files, but **`:seed` appears in 17** -- and
ray's `:ray-rng` dynamic variable is load-bearing: `grow-branches` is only
replayable because the seed is threaded through it. R7RS has no RNG at all.

So scm needs a seedable generator *as a value* (not a global), reachable
through a parameter so `parameterize` can rebind it the way `with-dyns` does.
This is also the one place where "same scene, same output" depends on the
language rather than on the renderer.

## 5. `seq` — 253 uses, the second most common non-trivial form

Janet's `seq` is a comprehension: `(seq [i :range [0 n]] body)`. It has no
R7RS equivalent, and `do`/named `let` are the usual stand-ins. It is a macro,
so `syntax-rules` can have it at S5 -- but with 253 uses the *name and shape*
should be kept rather than rewritten, because 253 rewritten loops is 253
chances to be subtly wrong. The same goes for `for`, `each` and `loop`.

> **Superseded 2026-10-16, and by two things at once.** `seq`, `for`, `each`
> and `repeat` are implemented under Janet's names in `prelude.scm:250-280`,
> and they work; what no longer holds is the *argument above for keeping the
> names*, so the question of where they live is reopened.
>
> First, **the 253 loops were never going to be converted mechanically.** This
> section assumed a textual conversion of scene *source*. §8a retired that: a
> scene converts by being loaded and re-saved through a printer, and **a save
> preserves state, not source shape** -- loops, helpers and `defshape` come
> out as the flat calls they made. No `seq` in the corpus survives a save to
> be rewritten, rightly or wrongly. The 253 only matter for hand-written
> sources, which is ray's prelude and modeler (B5).
>
> Second, **Kevin ruled on how those should read**: *"I don't necessarily want
> the scripts to look like Janet. I think it is better to make them look like
> Scheme."* So B5's translator targets `for-each`, `map` and named `let`, and
> the compatibility names have no consumer that wants them.
>
> They are still the right 30 lines to have written -- they are tested, and a
> file mid-translation can lean on them -- but as **globals in the core
> prelude of a general-purpose embeddable Scheme** (goal 2) `for` and `each`
> are a surprise rather than a service. `~/devel/ray/docs/scm-port-plan.md`
> §3.5 recommends moving them to an optional `(scm janet-compat)` library.
> Nothing in this repository's libraries uses them -- only their own tests --
> so the move is mechanical. **Not done; recorded, because the reason it
> should happen is here and not there.**

---

## 6. What this does not resolve

`eval`, `eval-string`, `postwalk` and `curenv` appear in **8 files** -- that is
`homoiconic.janet` and its neighbours, already the plan's open question 3. The
survey does not make it easier; it does confirm the blast radius is 8 files out
of 219, which is small enough to leave for last and large enough not to
pretend it is zero.

`tuple?` in 4 files distinguishes Janet's immutable tuples from its mutable
arrays. scm has one vector type, so that distinction is gone -- already in the
conversion hazards (§4.8), and 4 files is the measured cost.

---

## 7. The standing checklist

What scm owes, for a converted scene to run. Status as of this survey:

| | status |
|---|---|
| keywords as first-class values | **done** (S1/S2) |
| `[x y z]` evaluating its elements | **done** (S1/S2) |
| `&opt` / `&named` / `&keys`, defaults that preserve `#f` | **done** |
| proper tail calls, so loops do not grow | **done** (S2) |
| a collector, so a scene build does not leak | **done** (S3) |
| `let`, `when`, `unless`, `cond`, `case`, `and`, `or`, `do` | S5 (`syntax-rules`) |
| `seq`, `for`, `each`, `loop`, `while`, `repeat` | S5, under Janet's names -- but §5's *reason* for the names is superseded, and the port rules against using them in authored Scheme |
| `map`, `filter`, `reduce`, `sort`, `reverse`, `length` | S5 (`(scheme base)` + list library) |
| **a growable array** (`array/push`) | **S5, decision needed** (§2) |
| **`format`** | **S5, not optional** (§3) |
| **a seedable RNG as a parameter** | **S5** (§4) |
| `string->number` (`scan-number`), `string->keyword` (`keyword`) | S5 |
| `get-in` / `put` / nested access | S5 |
| `parameterize` for `dyn`/`setdyn` | S5 |
| `get-environment-variable` (`os/getenv`) | S9 (`(scheme process-context)`) |
| `error` / `assert` / `guard` | S9 |
| `eval` and a walkable AST | open question 3 |

## 8a. DEFERRED 2026-10-12: §8's design is superseded, and the decisions belong to the port

§8 below was written when a saved ray scene was **Janet source**, so a
conversion had to be textual. It is not any more. ray's `tool-refactor` branch
landed A2 of its `docs/rust-api-plan.md`: the retained store now holds a
**neutral `Form`** — `{ head, args: Vec<FormVal>, layout, precision }`, the
call that built an entry as *data* — and `src/snapshot.rs` is an **ungated**
Rust writer over it. A binding with no Janet VM can save a scene.

Kevin's framing: the point of that design is *to make porting ray to a
different scene-file format easy, as long as the format is a lisp dialect.*
ray's §3.6 says the same thing and bounds it — a second printer is "six
decisions, all local to one function", because `Form` is a call tree and so
homoiomorphic to an s-expression and to nothing else.

**So the conversion step §8 describes should not be built. The printer is the
converter**, and it belongs to the port (Track B), not here — a saved-file
spelling outlives the decision, and the port is what will say what it needs.
**Deferred until ray/modeler actually move to scm.**

What is worth keeping is the measurement, so the port starts from facts rather
than from this survey again. Of ray's six decisions, **scm already answers
four as no-ops** — verified by running them, not by reading:

| `FormVal` | Janet emits | scm | |
|---|---|---|---|
| `Key` | `:at` | `:at` | **identical** — and no `#:` prefix, which is the thing Kevin disliked in Steel |
| `Tuple` | `[0 1 0]` | `[0 1 0]` | **identical on READ** -- §4.5: a bracket vector is a constructor and numbers self-evaluate. **Corrected 2026-10-16: scm does not print it back with brackets.** `writer.rs:231` is deliberate and argued -- several accepted input spellings, **one canonical output**, always `#(...)`, because the flag is not part of the value and printing it would make two `equal?` vectors print differently depending on how each was born. Harmless for the port, and worth knowing why: ray's `Form::to_scheme()` owns the saved-file spelling and scm never writes the scene file |
| `Num` / `NumFull` | `0.10000000000000001`, `1e+21`, `1e+06`, `6.1232339957367574e-17` | every one reads to the correct `f64` | **no-op**. scm re-prints shortest-round-trip (`0.1`), which differs textually and loses nothing, and scm is not the one writing `.janet` |
| `Str` / `Sym` / `Call` | `"a\"b"`, `lambertian`, `(%tex 0)` | same | **identical** |
| `Array` | `@[…]` | `[…]` | collapses: one vector type (already in §6's hazards) |
| `Nil` | `nil` | `#f` | §4 decided it — "there is no nil in this language", and an absent argument binds `#f`. 6 producers in ray |
| **`Struct` / `Table`** | `{:k v}` / `@{:k v}` | **nothing** | **the one real gap** |

And the gap is small, measured rather than feared: **exactly two producers in
all of ray** emit a dictionary — the material literal (`src/snapshot.rs:336`)
and the NURBS net (`:938`) — and both are keyword-keyed.

scm already has the machinery for whatever spelling wins, which is why this is
safe to defer rather than risky. `&keys` collects keyword arguments into an
alist, so a dictionary constructor is one line:

```scheme
(define (%dict &keys kv) kv)
(%dict :albedo [0.78 0.76 0.72] :shininess 32 :shader :lambertian)
;=> ((:albedo . [0.78 0.76 0.72]) (:shininess . 32) (:shader . :lambertian))
```

The three candidate spellings — a constructor call, a bare keyword plist, or a
quoted assoc list — are written up in this session's transcript and all three
run today. Nothing in scm has to change first.

## 8. The gate this makes possible, without ray

A converted scene can be **executed standalone**, which is stronger than
reading it:

1. `save-scene` in the existing ray (Janet) flattens an example to a list of
   verb calls -- that is what ray's own migration plan relies on, and it needs
   no changes to ray.
2. Convert that flat list to Scheme, textually.
3. In scm, define every ray verb as a **stub that records its name and
   arguments** and returns a plausible handle.
4. Run the scene. Compare the recorded call sequence against the Janet flat
   list, verb for verb and argument for argument.

No renderer, no embedding, no ray build. It proves the language expresses the
corpus *and* that the conversion was faithful, which is the whole of what scm
can honestly owe before the adoption project begins. "Renders identically" is
a comparison between two ray builds and belongs to Track B.
