# The modeler moves to scm — Track B's B5, staged

2026-10-19, written after the port branch merged (ray `main`, P1–P6). The
sibling of [port-plan.md](port-plan.md): that one moved **ray**, this one
moves the **modeler**, which §7 of it deliberately left alone. A bare path
here is this repository; a `ray/` prefix is the other one.

Kevin's addition, the day this was written, and it sets the order: **"make
sure the modeler editor is converted so that it reads scheme files and all
of the 'slime-like' features are enabled. Same with repl."** So the editor
and the REPL come first, before any of the 6,538 lines move.

## 1. What is actually there, measured

| | |
|---|---|
| the modeler's Janet layer | **18,883 lines, 892 top-level definitions**, in **23 files**: `modeler/scripts/prelude/*.janet` (22 of them, 12,345 lines) and `unify.janet` (6,538), the bridge loaded after both preludes |
| the binding | `script.rs` + 10 modules, **8,245 lines** excluding its 5,345-line test module |
| prim bodies | **252** `extern "C-unwind"` functions |
| conditional compilation | **222** `scripting` sites, **21** `janet` sites |
| the UI harness | **1,861** `eval` calls across **140** Python files, every one of them Janet source |

**The prims are a narrow shape, which is the good news.** Across all 252
bodies the arguments are numbers (405 uses of `num_arg`) and strings (103
of `string_arg`) and essentially nothing else; the returns are nil (219),
number (170), a tuple of numbers (43), boolean (24), array (6), keyword (4)
and string (4). The median body is 10 lines and 56 of them are five lines or
fewer. A second binding for that is a table and a macro, not 252 decisions.

**The harness is the hard part.** It is the only thing in the repository
that exercises a modal mouse drag, a wheel detent or the options ladder, no
cargo suite can reach them, and it speaks Janet in 140 files. Nothing may
move in a way that costs that coverage.

**The split already happened on purpose.** `script.rs` was one 13,373-line
file; A4 broke it into ten modules, and its own comment says why: *"B5 moves
the modeler to a second VM divisible by file, and a flat file would have
made that one `#[cfg]` per prim instead of one per module."*

## 2. What the editor and the REPL need, specifically

The editor pane is the SCENE editor — it edits the file the viewport is
showing — so the language it has to speak is the one scenes are written in,
and that is already done on ray's side. Four things stand in the way:

1. **The send gate is spelled in Janet.** `editor-janet-buffer?` answers
   "a `.janet` file or an unnamed scratch", and all four SLIME commands
   (C-x C-e, C-M-x, C-c C-r, C-c C-b) refuse anything else with *"sends
   work on .janet buffers only"*.
2. **The evaluator is Janet's.** `editor-send-janet` calls `eval-in-env src
   *cmd-env*`. A `.scm` buffer needs a Scheme evaluator in the same
   process, on the same thread, editing the same retained store.
3. **The form scanner is Janet's reader.** `editor-scan-spans` treats `#`
   as a comment to end of line, understands Janet's backtick long strings,
   and rides the prefixes `@ ' ~ ,`. In Scheme `;` is the comment, `#(`
   opens a vector, `#t`/`#f`/`#\c` are atoms and `#| |#` is a block
   comment — so the same scanner on a Scheme buffer eats `#(0 1 0)` as a
   comment and loses the rest of the line.
4. **`open-scene` evaluates the file as Janet source.** A `.scm` scene
   reaches Janet's reader and fails somewhere past the real problem:
   *"unexpected end of source, ( opened at line 7"*, because `#(0 1 5)`
   was read as a comment.

## 2a. The strategy changed, 2026-10-20 — translate, do not interoperate

Kevin, on seeing M2 measured in sessions rather than hours: *"when we did
the SBCL lisp port of an earlier version of ray (from rust), the port was
done in one day and that was the entire renderer, here we are porting janet
files to scheme which both are lisp dialects. Why is this translation
process taking so long?"*

The answer is that it was not translation. The Lisp port was a REWRITE with
no interoperation: read Rust, write Lisp, compare images, one language out
the other end. What M0–M2 built instead was incremental interoperation —
every moved function kept working for its Janet callers, in the same
process, with both VMs loaded. That is where the time went:

- a bridge one way, then the other, then a value door;
- three resolution failures (vars, prims, a forgeable error marker);
- a name two VMs both register;
- a return shape that had to stay nested;
- feature wiring, because shims in a build with no second VM broke 39 tests.

**None of that is translating Janet into Scheme**, and all of it is deleted
at M4. The two files that moved took longer to make coexist than to rewrite.

**So the rest of M2 is a wholesale translation on a branch**, the way the
Lisp port was done: no shims, no doors, no dual registration. The harness is
the oracle exactly as the rendered image was, and a failure is bisected by
reverting files rather than by threading another call across a boundary.

What that needs first, and only this:

1. **Every prim reachable from Scheme.** Mechanical, permanent, and the
   only prerequisite — `prim!` plus the rewriter at
   `/tmp/convert_prims.py`'s successor.
2. **The harness's 1,861 Janet forms**, translated in the same sweep or
   left speaking Janet through a reader kept for them alone.

**What survives from M0–M2** is the permanent half: 248 prims declared once
for both VMs, the two moved files, and the knowledge in this document. The
scaffolding goes.

## 3. The stages

**M0 — the modeler speaks Scheme for SCENES (0.5).** A second `Interp` on
the main thread with ray's verb surface and ray's Scheme prelude —
`ray::api::scheme::install`, whose own documentation already says an
`Interp` edits the scene of the thread it lives on, which is the contract
the modeler's threading spine needs. Then: dialect by extension for
`open-scene`, the send gate and the REPL; a Scheme variant of the form
scanner; and the eval channel (port 4008) able to take either, which is
what later lets the harness migrate one file at a time.

No modeler prim is bound in M0. A `.scm` buffer can call every ray verb and
nothing of the modeler's own UI — which is exactly what a scene is.

Exit: open, edit, send and save a `.scm` scene in the modeler, with all four
SLIME commands; the UI harness unchanged and still 102/102.

**M0 as built, 2026-10-19.** `modeler/src/script/scheme.rs` (the second VM
and six prims), a `scm` feature on the modeler, and the dialect routing in
`unify.janet` and `prelude/08-editor.janet`.
`modeler/tools/test_scheme_buffers.py` is the gate: **19 checks**, and it
skips loudly on a build without the feature rather than failing.

What it does, in the order it was built:

- **A line of exactly `:scheme` or `:janet` on the eval channel switches
  dialect.** Protocol-level, so the 140 Python files of the harness can
  migrate one at a time (M3), and so can an Emacs client.
- **One store, two languages.** A sphere added from Scheme is in Janet's
  `(objects)` in the same process — asserted, because two VMs that did not
  share the store would be two scenes.
- **The scanner takes a dialect.** Janet's reads `#(0 1 0)` as a comment
  and loses the line: on the probe text it found **0** top-level forms
  where the Scheme reading finds 2.
- **`open-scene` routes by extension**, and `%scm-load` sets `(%including
  …)` first so a scene's own `(include …)` still resolves beside the file.
- **The REPL pane has its own dialect**, its own prompt, and asks the right
  reader whether a line is finished — Janet's parser called an unbalanced
  Scheme line complete, because the `#(` was a comment to it.
- **Completion and C-c C-d come from the Scheme VM** for a `.scm` buffer:
  778 candidates, narrowing to `add-sphere` on a prefix, and
  `(add-sphere . args)` for a description.

Two things the build found that reading would not have:

1. **`send-buffer` wraps the text in `(sync-begin)`/`(sync-end)`, and those
   are ray's JANET prelude rather than verbs** — so the Scheme VM has no
   name for them. The window now brackets the send from the Janet side,
   which is correct and not a workaround: the window is a property of the
   store, which both languages share.
2. **A test that reads the REPL transcript reads the wrong thing.** The
   first version asserted on pane rows and failed in both dialects — the
   Janet control failed identically — because the pane's focus dance is
   its own. Asserting on the STORE is both simpler and closer to what a
   send is for.

**M1 — the prim layer gets a second surface (0.8).** One declaration per
prim, two registrations.

**What the measurement says to build.** Across the 252 bodies the arguments
are numbers and strings and nothing else, and the returns are six shapes:

| return | prims | | argument | uses |
|---|---|---|---|---|
| nil | 219 | | `num_arg` | 405 |
| number | 170 | | `string_arg` | 103 |
| tuple of numbers | 43 | | anything else | 4 |
| boolean | 24 | | | |
| array | 6 | | | |
| keyword / string | 8 | | | |

(The return counts exceed 252 because a body with branches wraps more than
once.) So the surface a macro has to cover is: *f64 and &str in, and one of
nil / f64 / bool / String / keyword / Vec<f64> out.*

**The shape, which is ray's own lesson applied one layer up.** ray moved
183 verbs behind a typed record and generated Janet, Scheme and Python from
it. The modeler's prims are the same problem: a declaration carrying the
name and the types, a body written against neither VM, and two generated
wrappers.

```rust
prim! { "%pane-clear", fn pane_clear(id: f64) { textpane::clear(id as u32) } }
prim! { "%pane-scroll", fn pane_scroll(id: f64, rows: f64) -> f64 { … } }
```

Done module by module — the A4 split put them in ten, and its own comment
says it was for this — with the UI harness green after each, because the
harness is the only thing that drives a prim the way a hand does.

Exit: every prim reachable from both VMs, with a conformance corpus like
ray's `tests/conformance.jsonl` driving both.

**M1's macro as built, 2026-10-19** (`modeler/src/script/prims.rs`), with
`text.rs` as the first module through it — ten `%pane-*` prims, one
declaration each:

```rust
prim! {
    /// `(%pane-clear id)` — empty a pane's transcript.
    "%pane-clear", fn pane_clear(id: num) {
        crate::textpane::clear(id as u32);
        Out::Nil
    }
}
```

The body is written against neither VM. The macro puts it in a module named
after the function and generates the Janet wrapper, the registration and the
Scheme host function beside it — a module rather than pasted identifiers
because `guarded_cfn!` takes an IDENT, and inside the module `janet` is one.

Three things that had to be got right, and one that bit:

- **`Out::Nil` is `#f` in Scheme**, not unspecified: scm has no nil, and
  unspecified is TRUE there. Same rule ray's binding settled on.
- **Absent arguments read as zero on both sides**, matching `num_arg`'s
  default, or a prim would behave differently depending on who called it.
- **A keyword returns through a C string**, as every hand-written keyword
  return in the binding already does.
- **The module is `prims`, not `prim`.** Every prim file glob-imports its
  parent, so a module named `prim` shadows the `prim!` macro — and
  `#[macro_use]` scoping is textual, so the declaration has to come before
  every module that uses it.

Proven by calling `%pane-append` from Scheme and counting the row from
Janet: one body, two callers, one pane.

**The ordering risk, named in advance.** A module converted to the macro
changes 20-odd Janet-facing functions at once. The harness is the proof,
and it takes ten minutes and a binary that cannot be rebuilt while it runs
— the release artifact is shared across feature sets. So one module per
run, and no cargo build against `target/release/modeler` while a run is in
flight.

**M2 needs a BRIDGE, and that was not in the first draft of this plan.**
A prelude file's definitions call each other across files — `09-menus`
calls `02-scene`'s `tint-warm` — so a file translated to Scheme would
strand every Janet caller it has, and "divisible by file" would be a lie
however neatly the files are split.

So `%scm-call` (built 2026-10-19): apply a Scheme procedure to Janet
values. A moved section lives in `modeler/scripts/scm/`, and what stays in
the Janet prelude is a one-line shim per name:

```janet
(defn tint-warm
  "Tint the pattern warm (sunset amber)."
  [] (%scm-shim "tint-warm"))
```

Every caller — the menus, the keymap, the REPL, the agent — is untouched
and never learns which language answered. The shims come out when the last
Janet caller does.

**The bridge runs BOTH WAYS, and the second direction is what makes the
rest of the prelude movable.** `%scm-call` carries a Janet caller into
Scheme; `(janet "name" args…)` carries a moved section back into
everything that has not moved — the other prelude files, and every prim
still written only for Janet. Without it only a LEAF function could move,
and the prelude has almost none: `02-scene`'s tint commands were leaves and
they are the exception.

The door resolves a name by evaluating the WRAPPER `(fn [& a] (NAME ;a))`,
which is the one form that reaches everything the prelude calls. Two
earlier attempts each missed a kind, and each was found by a test rather
than by reading:

- `call_hook` refuses a VAR. Half the prelude's cross-section handles are
  vars — `*undo-clear*`, `*outliner-refresh*`, declared in `00-decls` and
  bound by a later section — and `janet_resolve` hands a var's REF back,
  an array and not a function, so it answered *"hook not a function"*.
- Resolving the bare name and type-checking it refuses a PRIM. A C
  function is `JANET_CFUNCTION`, not `JANET_FUNCTION`, so every `%`-prim in
  the binding was unreachable. The async-load test caught it — the only one
  that drives a moved command whose body calls a prim through this door.

The wrapper costs one eval per call; caching the compiled wrapper per name
is the obvious step if it ever matters.

**A failure crosses as a VALUE and the shim raises it.** A prim has no way
to raise in Janet: its only channel is `guarded_cfn`, which reports a Rust
panic as a DEFECT, and a script error is not one. So `%scm-call` answers a
TAGGED PAIR — `["%scm-ok" value]` or `["%scm-error" message]` — and
`%scm-shim` unwraps it. A typo in a moved name raises rather than quietly
answering nil.

**The tag is not decoration, and the first version got this wrong.** It
marked a failure with an `"error: "` prefix on the message; the modeler's
own loader answers `"error: no such file"` as a perfectly good VALUE, so
`(load-model "/nope.gltf")` raised the summary it was meant to return. A
marker the callee cannot forge is the only honest one.

**A moved section needs the `scm` feature, and says so where it is called.**
In a build without it the `%scm-call` fallback answers *"… lives in the
scheme image, and this modeler was built without the scm feature"*, which
the shim raises. That is the honest state of a transition: the alternative
is keeping both implementations, which is the duplication this is meant to
remove.

**M2 — the prelude moves (1.0).** 892 definitions across 23 files, and the
division B5 asked for is already there: 22 numbered prelude files in boot
order, plus `unify.janet` last. A file lands when the harness is green with
it, so the unit of work and the unit of proof are the same thing.

**The first section is across, 2026-10-19**: `02-scene.janet`'s four tint
commands, which were the first Janet the modeler ever had and are the M2
gate's own target — *"redefine a trivial command live and call it, no
rebuild, the window follows."* They live in
`modeler/scripts/scm/02-scene.scm`, loaded from disk like the Janet prelude
beside them, and the Rust test
`a_moved_prelude_section_answers_through_its_shim` asserts the whole path
by reading the KERNEL STATE at the end of it — Janet caller, shim,
`%scm-call`, the Scheme procedure, the `%set-tint` prim — because "the call
returned nil" is also what a shim forwarding to nothing would say.

A first draft of this document said the layer was one 6,538-line file and
that M2 would have to divide it first. That was `unify.janet` alone — the
bridge — and it missed `modeler/scripts/prelude/`, which is three times the
size and already split. Corrected the same day, before anything was built
on it.

### Moving a file: the recipe, and what decides the ORDER

Each file is the same five steps, and the harness is the gate after every
one of them:

1. **Convert the prims it calls** through `prim!` (M1), or leave them to the
   `janet` door. The door costs a round trip and reads honestly; converting
   is better where the prim is simple.
2. **Translate the definitions** into `modeler/scripts/scm/<file>.scm` and
   add the file to `load_scheme_prelude`'s list, which is ordered because
   load order is the dependency contract on this side too.
3. **Replace each Janet body with a shim**, keeping the DOCSTRING in Janet:
   `(doc load-model)` and the generated manual read Janet's, so a shim with
   no docstring empties both. The docstrings move with the last file.
4. **Run the harness**, all 103.
5. **Push.**

**What decides the order is the shape of the VALUES a file's functions
return, not its size.** The bridge carries numbers, strings, keywords,
booleans and lists of those. A function answering a Janet struct — `(perf)`
hands back `{:phase … :last …}` rows, `(hybrid-state)` a table — would
reach its Janet callers as something else, so those files wait for the
bridge to carry dictionaries, which `ApiValue::Map` and scm's association
lists already have the pieces for.

So the order is: files whose functions return nil, a number or a string
first; files that return structures after the bridge grows; and files that
own DATA other sections read — `04-theme`'s `*themes*` table, which
`(keys *themes*)` reaches directly — last or never, since a moved table is
not reachable by a Janet caller at all.

**A NAME IN JANET IS NOT A NAME: two VMs register `%get-camera`.** ray
registers one for the SCENE camera, the modeler one for the viewport, and
whichever registers last owns the name. The Janet prelude kept working
because it compiled against the modeler's binding before the shadowing —
`(camera)` answered the pose while a fresh `(%get-camera)` from the eval
channel answered nil, in the same session, and had done so long before the
port. A moved function resolves at CALL time and got ray's.

So a moved section calls the modeler's prims DIRECTLY, through `prim!`,
rather than through the `janet` door: installed into the second VM there is
no name to shadow. The door stays for what has not been converted, and the
shadowing is one more reason to convert rather than to forward.

**A moved function must keep the SHAPE of what it answers.** Converting
`%get-camera` first flattened `[eye target fov]` into seven numbers, which
compiles and breaks the dozen Janet callers that destructure it; `Out::Many`
exists for exactly that. And `set-camera!`'s refusal of `:up` — *"a
scene-camera key, use (set-camera …)"* — is not the same as its refusal of
a typo, which a translation flattened into one message until a test said so.

**M2a — the sync window becomes dialect-neutral (0.2), and M3 needs it.**
`%sync-begin`, `%sync-end` and `%sync-abort` are written against the Janet
FFI in `ray/src/scripting/store.rs`, and ray's Janet prelude wraps them as
`sync-begin`/`sync-end`. The Scheme VM has neither, which is why M0 brackets
a Scheme buffer send from the Janet side. That is correct — the window is a
property of the shared store — but it leaves a path that only exists in one
language, and a harness file sending a buffer in Scheme (M3) would have to
reach back into Janet to open it. The fix is the one P3 already used six
times: lift the body into `scene_store`, leave the Janet prim calling it,
and add the Scheme pair beside it.

**M3 — the harness migrates (0.5).** 1,861 eval sites, file by file,
against the dialect-aware channel M0 built. A file moves when its Scheme
form runs green twice.

**M4 — Janet becomes optional in the modeler (0.2).** The `janet` feature
stops being implied by `scm`, the 21 `janet` cfg sites answer to the
binding that is present, and `--features scm` alone builds a modeler with
no Janet in it. Janet still builds, as §7 of the port plan promises.

Total: **3.0**, against the single line B5 had in plan.md §9 (2.0), and the
difference is the harness, which that line did not count.

## 4. Decisions, so a later stage cannot quietly reopen them

- **Two VMs in the modeler for M0–M3**, exactly as ray carried two from P1
  to P6. The alternative is one branch translating 21,000 lines, which is
  the thing this staging exists to avoid.
- **The harness is never allowed to regress.** It runs before and after
  every stage, and a stage that cannot keep it at 102/102 is not finished.
- **No prim moves in M0.** The temptation is to bind "just the few the
  editor needs"; the editor needs none, because a scene is ray verbs.
- **The Scheme scanner is a variant, not a rewrite.** One scanner with a
  dialect argument, so a fix to the paren logic cannot land in one and not
  the other.

## 2b. The translation, 2026-10-20 — all 22 files, 13,133 lines

`scripts/dev/janet_to_scheme.py` now translates every prelude file, and the
concatenation of `00-janet-compat.scm` plus all 22 in boot order EVALUATES in
`ray-scm` with nothing left unresolved but prims the modeler has not
converted yet (`%add-pane`, `%set-pane-home`, `%set-pane-ortho`,
`%pane-anchor`, `%pane-append`, `%clear-status-toggles`, `%set-toolbox`,
`%op-list`, `%tool-desc`, `%scm-call`) and `curenv`, which is a DESIGN
question and not a missing function (§2c).

The translator reports nothing it could not decide. What it took to get
there, because each of these was a silent wrong answer rather than an error:

- **Janet's data structures are callable.** `(t :group)` is an index and
  `(set-theme :one-dark)` is a call, and the source does not say which. The
  decision is the closed world: every `defn` across all 22 files is
  collected first, every `%name` is a prim, and a one-argument call on
  anything else with a LITERAL key is an index. 390 sites, listed in
  `modeler/scripts/scm/INDEXED.txt`, which is there to be read.
- **`=` is arithmetic in Scheme and polymorphic in Janet.** `(= kind "3d")`
  raises under `=`; `(= n 2)` is quietly FALSE under `equal?`, because every
  Janet number is a float. A literal picks the word and everything else goes
  through `janet=`, which dispatches.
- **`case` matches with `eqv?`**, which is false for two equal strings — and
  the prelude dispatches on strings a prim answered. Every arm fell to the
  default: def-tool reported that each of its nine ops was "not a selection
  tool". `case` becomes a `cond` over `janet=`.
- **`(string :extrude)` is `"extrude"`, not `":extrude"`.** The colon
  belongs to the readable spelling (`%v`, `%j`), not to `(string x)`. With
  the colon, def-tool could not find a single kernel op.
- **Patterns are everywhere and Scheme has none**: `(def [w h] …)`,
  `(each [k f] …)`, `(when-let [[id kind] …] …)`, `(defn f [[nm kind]] …)`,
  `(seq [[px py] :in …] …)`. All flat, all positional, all through `nth` —
  and `fit` where `define-values` needs a fixed arity, since `(def [on]
  (%get-grid))` names one of four.
- **`break` cannot be a macro**, because a `%break` the macro introduced is
  invisible to the body that has to call it (hygiene). The translator names
  the escape continuation itself.
- **The short-fn reader was collapsing only atoms.** `|(= ($ 0) base)` left
  the lambda bodyless and spliced its body in beside it, so `(filter |… xs)`
  became a three-argument filter. One line, and it had been wrong since the
  reader was written.
- **The one `defmacro` is procedural** — `def-tool` reads `(%op-list)` and
  the Rust descriptor at expansion time — so `syntax-rules` cannot hold it.
  It becomes a FUNCTION whose template goes through `(eval … 
  (interaction-environment))`: the nine tools are defined when the file
  runs instead of when it is read, and nothing expands def-tool earlier.

## 2c. What is left, and it is not translation

- **`curenv`, 32 sites.** Janet environment reflection: `(get-in (curenv)
  ['%object-parent :value])` asks "is this prim present", and 00-decls and
  15-boot-audit walk the whole environment. Scheme's answer is a bound?
  predicate over `(interaction-environment)`, which is a prim to write, not
  a name to rename.
- **`os/stat`, `os/dir`, `os/date`, `os/time`, `os/clock`, `os/mkdir`,
  `os/rename`, `os/cwd`.** R7RS has `file-exists?`, `delete-file` and the
  port procedures, which covered `slurp`/`spit`; the rest wants a small
  `%os-*` prim family.
- **The eval path in 01-repl-eval and 08-editor** — `with-dyns`,
  `eval-in-env`, `parser/*`, `debug/stacktrace` — is Janet's REPL machinery,
  and the Scheme side of it is `%scm-eval`'s job in Rust. Those sites are a
  HAND port, not a mechanical one, and they are the only ones.

## 2d. M2c done, 2026-10-20 — the kernel calls the Scheme prelude

`MODELER_PRELUDE=scheme` boots the modeler on its translated prelude: all 23
files plus the unification bridge, ray's prelude re-installed over them, and
every kernel hook, dispatch and REPL form routed to the Scheme session.
Janet remains the default. `modeler/tools/test_scheme_prelude.py` drives
that modeler through authoring from both sides of the bridge, selection, the
outliner, a theme slot, the pane registry and a key through the hook path —
13 checks, and it is in the harness count (104 now).

**The three value rules, which are the whole lesson of this stage.** Each
was silent, and each broke something that looked like it worked:

1. **nil IS `#f` in Scheme**, so a prim that answers `false` to mean one
   thing and nil to mean another has one answer, not two. `%tool-desc` said
   `false` for "this tool writes its own interactive verb" and nil for "take
   the default"; the interactive half of nine tools was never defined.
2. **`when` and `unless` ARE VALUES in Janet** and answer nil when the test
   fails — Scheme answers `unspecified`, which is TRUE. `add-sphere` is
   `(unless (authoring-locked?) …)` over a lock that is `(when (live-active?)
   …)`, so with nothing live every authoring verb refused and the scene
   stayed empty with nothing raised anywhere.
3. **A `def` that rebinds a parameter is an assignment.** Janet's body `def`
   shadows sequentially; Scheme's internal define is `letrec*`, where the
   initialiser sees the new binding, so `(def who (or who "object"))` leaves
   `who` unbound in its own definition. 40 sites.

A fourth, of the same family: **`break` is an early RETURN as often as a
loop exit**, and `(unless (outliner-visible?) (break))` is the prelude's
`return`.

**381 of 385 prims cross.** `%undo-push` went after all, because `ApiValue`
NESTS — the array-of-arrays shape `prim!` will not carry is ordinary
structure in ray's own value model, which both VMs already convert. The four
left are `%tool-arm-script` and `%pick-arm` (they take CLOSURES and hand
them to the tool layer; what a scm `Proc` means to a tool that outlives the
call is a design question), `%get-repl-src` (Janet's own REPL C glue), and
nothing else.

**Nine ray prims gained Scheme twins** — `%shape-keys`, `%light-keys`,
`%lights`, `%get-film`, `%get-integrator`, `%entry-size`, `%origin-state`,
`%mat-literal`, `%sync-open?` — and `follow-curves` is now in ray's Scheme
prelude, written as Scheme rather than translated. That is the pattern for
the rest of ray's Janet-prelude residue: a reader over the one store belongs
in Rust once, and prose logic belongs in whichever prelude asks for it.

## 2e. M3, sized — the harness

`python3 scripts/dev/harness_dialect.py`:

```
1775 eval forms, 1536 read in both dialects
why the rest do not: bracket 128, atoms 80, vocab 62, shortfn 19, …
59 of 117 test files have NO Janet-only form.
```

So **87% of the forms are already portable** and the Janet-only ones cluster
in a dozen files. But that is only half of M3: the harness also compares
PRINTED output, and Janet prints an array as `@["a" "b"]` where Scheme
prints a list as `("a" "b")`. `test_options_ladder` caught exactly that
during M1 and is the reason `Out::Arr` exists. So a migrated test file is one
whose forms AND expectations have moved, and the honest estimate stands at
the plan's 0.5.

A whole-harness run under `MODELER_PRELUDE=scheme` fails early and often
today, and that is expected rather than informative: the forms are Janet.
The order to migrate in is the 59 files with no Janet-only form first.

## 2f. What is left after M2c, by name

Three probes keep the answer honest, and each is a script rather than a
memory:

```
python3 scripts/dev/harness_dialect.py           # M3's size
MODELER_PRELUDE=scheme … prim_gaps.py            # prims the sources name
MODELER_PRELUDE=scheme … name_gaps.py            # every name they call
```

(The last two live in the session scratchpad; they are ten lines each and
the pattern is in §2f's own numbers — read every `(name` out of
`modeler/scripts/scm/*.scm`, ask the image `(procedure? name)`.)

**1. ray's JANET prelude has a residue the Scheme prelude lacks**, and it is
exactly ten names: `snapshot`, `group-objects`, `ungroup-object`,
`origin-form`, `record-origin`, `sync-begin`, `sync-end`, `fnum`,
`to-world`, `view-help`. They are prelude LOGIC over prims, not verbs, so
P1's 183 shims never covered them. Three groups:

- `snapshot` and `fnum` are one-liners over things the Scheme image has
  (`save-scene` / `scene-source`, and the `%fnum` prim).
- `sync-begin`/`sync-end`/`sync-abort` need their prims installed for
  Scheme, and the sweep logic inside `sync_end` belongs in `scene_store`
  first — the lift P3 did six times, for the same reason.
- `group-objects`/`ungroup-object`/`origin-form`/`record-origin` carry
  `*group-parts*` and the material-literal spelling with them; they are the
  real port of that file's second half.

`follow-curves` and `*scene-file*` already crossed this way, and the pattern
held both times: a reader over the one store goes into Rust once and both
images call it; prose logic is written as Scheme in ray's prelude rather
than translated from Janet.

**2. Four prims stay hand-written** (§2d), and only two of them are a design
question: a Janet closure handed to the tool layer has no scm counterpart
yet.

**3. The compat layer's remaining shape** is the three eval-path families —
`parser/*`, `with-dyns`/`eval-string`/`debug/stacktrace`, and the agent's
sandbox (`table/setproto`) — plus the `os/*` family, which wants a small
`%os-*` prim set. Everything else the prelude calls is defined: the type
tests, the sequence shapes, `dyn`/`setdyn`, `flatten`, `partition`.

## 2g. The residue, half crossed — and the next hang

Of §2f's ten ray-prelude names, four crossed: `snapshot` and `fnum` are
one-liners over `save-scene`/`scene-source` and the `%fnum` prim, and
`sync-begin`/`sync-end`/`sync-abort` went the way P3's six went — the sweep
lifted into `scene_store` (`sync_begin`/`sync_end`/`sync_abort`), the Janet
prim reduced to a call, and a Scheme prim beside it. The six left are
`group-objects`, `ungroup-object`, `origin-form`, `record-origin`,
`to-world` and `view-help`, and the first four carry `*group-parts*` and
the material-literal spelling with them.

**The next thing a Scheme-prelude modeler hangs on is `open-scene`**, and it
is the re-entrancy trap rather than a missing name: the prelude's
`open-scene` reaches a .scm file through `%scm-load`, which is the Janet→
Scheme door — so under a Scheme prelude it asks the session to load a file
while the session is already borrowed, and the main thread stops with no
panic and no output. The fix is in the eval-path hand port (§2c): with the
prelude in Scheme, `open-scene` evaluates the file directly and `%scm-load`
goes back to being what it is, a door for Janet.

`test_save_load` is where that shows, and `os/stat` (the `%os-*` family,
§2f.3) is what `toggle-scene-editor` wants two lines later.

## 2h. Grouping crossed, and the residue is two

`group-objects` and `ungroup-object` are in ray's Scheme prelude, over five
new Scheme prims (`%group-objects`, `%ungroup-object`,
`%capture-group-parts`, `%compose-group-origin`, `%restore-group-origins`,
`%forget-origin`). The handoff between capture and compose — a thread-local
that has to STRADDLE the grouping, because a member's state and matrix are
unreadable once it is folded in — moved from the Janet prim module into
`snapshot.rs`, since a `--features scm` build has no `scripting::store` to
keep it in. Measured in ray-scm and in a Scheme-prelude modeler: two spheres
group to one entry and come back as two.

`*group-parts*` is an ALIST there rather than a table: ray's Scheme prelude
imports nothing (it is embedded and loads before any library), the thing
holds one entry per group a session makes by hand, and the operations are
remember and take.

**The residue is now two names**: `origin-form` and `record-origin`.
`origin-form` is the interesting one — it answers the recorded call AS DATA,
which needs a `Form`→`Val` conversion that does not exist yet (`%rerun-form`
covers "rebuild it", which is most of what the modeler wants, and it is
already installed). `record-origin` is a thin wrapper over a prim that is
installed; it is unported only because nothing has asked for it yet.

## 2i. Re-entering the session, and what the 59 measured

**A prim that dispatches back into the prelude is the shape M2c had left
broken.** `%route-escape` answers whether `(on-escape)` said true, the drag
seams call `on-release`, and `tools::to_janet` is written that way
throughout — so under a Scheme prelude those dispatches re-enter the
session, a second `with_session` is a double borrow, and scm reports it as
"the host function panicked", because from the outside that is what a panic
inside a host function is.

The fix is the `Host` every host function is already handed: `prim!` now
installs through `define_fn_with` and parks that host in a thread-local for
the length of its own body, and `scheme::call`, `eval_quiet` and
`eval_to_triple` take it when it is there. The invariant is narrow — set for
one call, cleared on the way out, and the `&mut` it came from is dormant
while the body runs, because the body is our own Rust.

**Then the 59 "portable" harness files were run against the Scheme
prelude**, which is the measurement §2e asked for. None passed, and the
reasons sort into four piles:

- 12 send `(def …)` or `(set …)` — Janet's binding forms, in the TEST.
- 8 hit one translation bug, now fixed: Janet's `map` and `filter` answer
  ARRAYS where this layer answers lists, so `(array/concat (map …) …)` was
  handed a list to grow. `janet-concat` and `array-slice` take any
  indexable now, as Janet's own do.
- 5 are "incomplete input" — bracket literals and short fns in the test.
- 2 were the re-entrancy panic above; 2 index a table with a variable key
  that the closed-world rule could not prove; 2 call `length` on a string.

So the remaining work really is M3, and `test_theme.py` shows the shape it
should take wherever it fits: the forms are the same datum in both
languages, so the file names its ONE difference (the formatter) at the top
and runs against either prelude. `Modeler.require_prelude(which)` is there
for the files where that does not fit.

## 2j. The callable data structure, decided at run time

The closed-world rule proves 377 index sites and leaves 54 it cannot: a name
the prelude does not define as a function, with a key that is not a literal,
is `(xs i)` over a list as often as it is a higher-order call. Guessing
either way is wrong somewhere.

So those sites go to `janet-index`, which decides the way Janet decides — at
RUN time, on what it was handed: `(if (procedure? f) (f k) (get f k))`. The
provable sites still read as `(get *edit* :text)`, because that is what a
reader wants to see and the proof is what makes it honest.

Two heads must never take that path, and both bit: **`quote`**, written out
in `def-tool`'s template as `(quote ,cmd)`, and **a PEG grammar**, whose
heads are `*`, `<-`, `any`, `if-not`, `set` and `to` — a grammar is DATA and
has to cross untouched. `NEVER_INDEX` names them.

**With that, the prelude side of the Scheme run is clean.** The 59 portable
harness files, run again: 1 passes (the migrated `test_theme`) and all 58
failures are now the TEST's own Janet — `(def …)`, `(try …)`, `true`,
`string/join`, `[0 1 0]`, and `((options-state) :camera)`, which in Scheme is
`(get (options-state) :camera)`. Nothing left in that pile belongs to the
prelude, the compat layer or the prims.

## 2k. The harness sends Janet, and the client translates it

M3 does not need 1,775 forms rewritten by hand. The harness keeps sending
Janet, and `modeler_client` translates each form on the way out when the
modeler's prelude is Scheme — through `scripts/dev/janet_to_scheme.py`, the
same code that produced the prelude the modeler is running. It needs the
whole vocabulary to do it: with only the builtins, `(tessellate! :b)` reads
as a table index and becomes `(get tessellate! :b)`.

Two guards earn their place:

- **A form already written in Scheme crosses untouched.** `#` begins a
  comment in Janet, so `#(0 1 0)` takes the rest of the line with it and the
  form arrives unbalanced — which the modeler waits for more input on and
  the test reads as `[no reply]`. A hang, from a reader.
- **The paren count is the proof.** A form the reader mis-takes comes out
  short, and a short form blocks the channel rather than failing, so an
  unbalanced translation is thrown away and the original goes instead.

What the client cannot do is move a test's EXPECTATIONS: Janet prints an
array as `@["a"]` where Scheme prints a list as `("a")`. Those are per file.

### And a warning, earned

**A Scheme-prelude harness run can WRITE TO THE REPO.** `save-scene` refuses
to overwrite a hand-written scene file, and the refusal read the file's
first 400 bytes with `file/open` — which the Scheme image does not have, so
the `try` around it answered "cannot read", which the guard takes as
permission. `test_save_guard` then overwrote
`examples/modeler/city-block.janet` with its own snapshot, 233 lines to 4,
and three Janet tests failed on the next run for a reason that had nothing
to do with the code they were testing.

The guard asks `file-head` now, which each image defines its own way (the
`dict` / `load-scene-source` pattern), and a Scheme-prelude run refuses as
it should. The lesson generalises: a guard that depends on a Janet function
inside a `try` is not a guard in the other image, and `git status` belongs
in the loop when running the harness against a half-ported prelude.

### Where M3 stands, measured

The 59 portable files against the Scheme prelude, with the client
translating: **2 pass**, and of the 57 failures **24 still name something**
(a handful of Janet words the compat layer has since gained — `distinct`,
`+=`, `table`, `identity` — plus `match`, `parse-all` and `fiber/new`, which
belong to the eval-path hand port) while **33 are expectation mismatches**:
`@["a"]` against `("a")`, a count, a printed table. Those are the per-file
work M3 is, and the number to watch.

## 2l. Where M3 landed, and what the rest of it belongs to

**24 of 117 harness files pass against the Scheme prelude**, from 1 when the
stage began. The work that got there was not 1,775 form rewrites:

1. **The compat layer defines the Janet SPELLINGS** — the names and the
   bracket-bound syntax (`def`, `set`, `fn`, `defn`, `try`, `each`, `seq`,
   `loop`, `do`, `if-not`, plus `string/*`, `array/*`, `math/*`, `os/*`,
   `true`/`false`/`nil`). A test's form is read by the modeler's own reader
   and evaluated by its own evaluator; only the image knows more names. That
   is categorically unlike the client-side translation this stage began with
   and removed, where the modeler never saw what the test wrote.
2. **`scripts/dev/harness_migrate.py` migrated 300-odd forms** whose spelling
   has a bilingual form, so every file stays in BOTH gates:
   `(*panes* :editor)` → `(get …)`, `|(…)` → `(fn [$] …)`, `(let [a 1] …)` →
   `(do (def a 1) …)`, `0xff00ff` → decimal, `{:glyph …}` → `(struct …)`.
3. **One number rule for the image.** A verb answered `1` and a prim
   answered `1.0`; `scheme::number_val` is now shared by both.
4. **Thirteen more prims crossed** — the `%os-*` family, the store readers
   (`%get-film`, `%get-denoise`, `%get-tone-map`, `%get-environment`,
   `%entry-size`, `%origin-state`, `%mat-literal`, `%sync-*`,
   `%object-transform`) and the grouping six.

**What is left is not M3's to finish**, and the measurement says so:

- **~50 behavioural differences.** Each needs its own diagnosis — a count,
  a geometry value, a refusal that reads differently. This IS the remaining
  substance of M3 and it is per-test work with no leverage left in it.
- **The eval path** (`match`, `with-dyns`, `parse-all`, `help`,
  `debug/stacktrace`): §2c named it a HAND port before this stage started,
  and four test files wait on it.
- **`%pick-arm` and `%tool-arm-script`** (3 files): the two prims that take
  Janet CLOSURES and hand them to the tool layer. What a scm `Proc` means to
  a tool that outlives the call is a design question, not a shape — §2d.
- **The `.rec` file format.** `test_recorder` writes a recording as Janet
  source (`@{:version 1 …}`) and the recorder reads it with whichever
  reader owns the prelude. A format that is one dialect's source is an M4
  decision, not a test edit.

### Two instruments, and what they cost to trust

`scripts/dev/harness_scheme.py` classifies each file's failure. Three
corrections before its numbers were worth quoting, and each had invalidated
a figure already reported:

- **A `RESULT:` line is what puts a file in the gate.** `run_all.py`
  collects only the files that print one, so judging by that string called a
  passing file a failure. It reads the exit code now. (The same audit found
  the gate was 104 of 117 files: twelve print no such line, seven of which
  are inspection scripts with no assertions, and five of which — including
  `test_theme` — genuinely self-assert and were silently uncounted.)
- **The port-squatter trap**, which `docs/modeler-screenshot-testing`
  records and this stage walked into anyway: 102 of 117 files had failed
  with "port 4008 is already in use", and every one was reported as a real
  failure. The runner waits for the port, and a collision is its own class.
- **An absent name must look like an identifier.** `unbound variable:
  0xff00ff` came out of a test's own output and read as a missing prim.

## 2m. Before M4 — the list

M4 makes Janet optional, and the harness is the only thing that drives a
modal drag, a wheel detent or the options ladder. So "before M4" means: the
harness green against `MODELER_PRELUDE=scheme`, as it is against Janet. 24
of 117 pass today; 90 fail; 3 time out. The 90 were grouped by cause, and
**three causes account for most of them** — a file is usually in more than
one group.

Ordered by how many files each item unblocks. ☐ = open.

**A. The Janet scene files — 64 of the 90 failing files reference one.**
☐ A1. Decide the rule: a Scheme-prelude modeler cannot evaluate a `.janet`
      scene (`eval-string` refuses, on purpose, in 99-scheme-only.scm). The
      harness loads `examples/*.janet` and its own `.janet` fixtures
      everywhere. The honest answer is `.scm` TWINS of every scene the
      harness loads, produced the way P4's corpus driver already produces
      them — load in Janet, `save-scene` as `.scm` — and a test that picks
      the twin by `Modeler.prelude()`. Not a per-test edit: one helper,
      `scene_path("examples/x")`, that answers `.janet` or `.scm`.
☐ A2. Generate the twins (a script under `scripts/dev/`), commit them beside
      the originals, and keep them in step: the corpus driver's comparison
      is the drift guard, since it already round-trips both.
☐ A3. Migrate the ~64 call sites to the helper. Mechanical.

A1 and A2 are DONE (2026-10-20). The rule: textual twins by
`janet_to_scheme.py`, committed beside the originals, with the corpus
driver's round-trip as the check. Eleven of the thirteen ray scenes verify
IDENTICAL to their Janet originals; `city-block` and `cookbook-fixtures`
are modeler scenes and verify inside the modeler at A3. Getting there
found five translation bugs, each silent, each in the compat layer's
guesses about SHAPE:

- `seq`'s body is a `do` and the LAST form is the element; wrapping the
  whole body in one `(list …)` gave two elements per iteration.
- `get` guessed "alist" from a list-of-pairs; a list of POINTS has that
  shape, and `(poly i)` with an integer is positional. The key's type
  decides now, and `eachp-of` tests the car for a key the same way.
- A ray verb answers its dictionary as an ALIST, so a scene that `put`s
  into a curve value needs `put!` to update an association in place.
- An ARRAY built with `array/push` is this layer's record, and at the verb
  boundary a record is `Val::Opaque`; `99-scheme-only.scm` now wraps
  `%api-call` to unwrap arrays — with a cons walk, because the dictionary
  it also carries is made of dotted pairs.
- `(include "x.janet")` must include the TWIN.

**Stiletto is deferred, and the reason is a number: 2 s in Janet, 515 s in
Scheme.** Its glyph outlines index a list inside a loop over its length,
and this layer's tuple is a LIST — O(n) to index, so the walk is quadratic
and `janet-length` re-walks per call. Janet's tuple is O(1). The honest fix
is translating `[…]` to a VECTOR (every compat accessor already takes one),
which is a prelude-wide change to make deliberately, not at nine minutes a
try. It also still fails after the wait, on an `array-items` of a list.

A3 is DONE (2026-10-20), and it is a migrator PASS, not an edit: a scene
path in a test — `SCENE = "examples/x.janet"`, `(open-scene "…")`,
`(eval-string (slurp "…"))`, a `/tmp/….janet` a test saves to — goes
through `Modeler.scene_path`, which answers the twin under the Scheme
prelude and the path itself under Janet. 45 sites in 32 files. A test that
AUTHORS a scene inline (six of them) calls `Modeler.scene_fixture(path,
janet_text)`, which writes the Janet and, under Scheme, the twin beside it
through the same translator. Two things the twins needed that the harness
found and the corpus could not: the CLI and C-o routed only `.janet` to
`open-scene` (a `.scm` boot model went to `load-model`), and `eval-string`
refused every string rather than reading Scheme — it now evaluates the
forms of its string in order.

**B. Booleans print as `#t`/`#f` — 36 files compare a reply to the text
`true`/`false`.**
☐ B1. One helper in `modeler_client`: `truthy(reply)`, which reads `true`,
      `#t` and a non-nil value the same way, and `falsy` for `false`/`#f`/
      `nil`. A COMPARISON helper, not a translation: the reply is what the
      modeler said, and the test is asking a yes/no question of it.
☐ B2. Migrate the 36 comparisons. Mechanical, and the same shape each time.

B is DONE (2026-10-20). `Modeler.truthy`/`falsy` read `true`/`#t`/a
value and `false`/`#f`/`nil`; the migrator's boolean pass rewrites `X ==
"nil"`, `X != "nil"`, `X == "true"`, `X == "false"` and `"true" in X` for
X a name or a call — 70 sites in 54 files — and patches the ten per-file
`check` helpers that tested `want in got`. The first version matched a
call's argument with a lazy `[^)]*`, which stopped at the `)` inside
`"(picking?)"` and mangled the one site it reached; the matcher is now a
balanced walk that respects string literals (`call_span`). A second
version then re-rewrote the patched helpers' own `want == "true"` on the
next run — a pass has to be idempotent, since the migrator runs to a
fixpoint, and the helper's `want` is a word, not a reply.

**C. A function's own parameters are not in the migrator's closed world —
39 files send a `fn`/`defn` whose body indexes a parameter.**
☐ C1. `harness_migrate.py`: `bound_names` must include the parameters of
      every `fn`, `defn`, `each` and `seq` in the sent forms, not only
      `def`/`let` bindings. `(defn %pc [nm] (reduce (fn [a r] (if (= (r 0)
      nm) (r 4) a)) …))` indexes `r`, which is bound by the lambda.
☐ C2. Re-run the migrator; re-gate on Janet (it must stay 105/105).

C is DONE (2026-10-20): `bound_names` reads the parameter vector of every
`fn`/`defn`/`each`/`seq`/`loop` in the sent forms; 10 more forms
rewrote. The same closed-world gap was in the TRANSLATOR, twice, and the
harness found both: a local `(defn at [i] …)` in 17-reference made `at` a
function everywhere, so `(drawing-point pane at)`'s `(at 0)` stayed a call
and Scheme refused to apply a vector — a parameter now shadows the function
set in its body; and `compat_names` read a `syntax-rules` template's
`(define n e)` as a definition of `n`, so every local `n` was a function
too — it reads top-level defines only now.

Two more migrator passes came out of the same run, both bilingual:
`(f a ;(g))` → `(apply f a (g))` (Janet's `apply` takes leading
arguments; a splice INTO A BRACKET, `[;vz :-z]`, has no spelling both
dialects read and is left alone), and `(= x y)` with no numeric literal →
`(deep= x y)`, since Scheme's `=` refuses a string; the compat layer's
`deep=` is `janet=` now, so 1 and 1.0 still agree as they do in Janet.

The compat gaps the twins and the sent vectors found: `mapcat` walked its
collection with Scheme's `map` (a sent `[[…]]` is a vector); `mod` was
missing (city-block uses it twelve times).

**Measured after A–C: 38 of 117 pass on the Scheme prelude (24 before),
47 real, 18 absent, 8 timeouts, 4 dialect, 2 printed; the Janet gate is
105/105.** The 14 new passes are the hybrid/display family and the scene
round trips (`save_load`, `delete_after_open`, `isoparms`, `light_glyphs`,
`nurbs_kind`…), which the twins unblocked. The pass set
(`modeler/tools/scheme-pass-set.txt`) is 39 files now — see `scripts/dev/harness_touched.py` for the
per-change loop that replaced re-running everything: touched files in
both dialects, `--pass-set` for the Scheme regression set, full gates at
a stage end.

Two per-file findings worth a line before L: `(%get-denoise)` reads `#f`
after a scene whose LAST form is `set-camera`, and 1 without it — the
Scheme camera path re-applies render settings; and `measure-lines` applies
a world point (`not applicable: (0.0 -9.57 -17.6)`) somewhere under
`measure-drawing`, after the `mapcat` fix let it get that far.

### The twins spell a tuple `[…]`, 2026-10-20

Kevin asked why a twin said `(set-camera :at (list 1.70 1.42 2.95) …)`.
There was no reason: the translator was written for the PRELUDE, where the
compat layer's tuple is a list so that `car`/`append`/`apply` and the
pattern walks read as Scheme, and the twins reused it unchanged. In a
scene the bracket is right — scm reads `[…]` as a vector that EVALUATES
its elements (Track B S1), the verbs take one, ray's Scheme manual writes
it, and it indexes in O(1). So `janet_to_scheme.py` has a scene mode
(decided by path: anything outside `modeler/scripts/prelude`), the writer
prints a bracket form with brackets, a twin is written BESIDE its
original, and `scripts/dev/scm_twins.py` is the drift guard A2 asked for:
Janet's round trip of the original against scm's of the twin, with the
compat layer in front. Eleven of twelve verify (the same eleven); the
first run found that `translate(path, fns)` REPLACED a scene's own `defn`s
with the prelude's set, so `(rib 1)` had become `(nth rib 1)` — two twins
failed with `+: not a number: #f` before that line was a union.

**Stiletto is not a tuple problem.** With vectors it still runs past 240 s,
and the source says why: four `make-texture` calls — 704×832, 320×1024,
256×512, 256×256, about 1.1 M texels — each texel an interpreted Scheme
closure doing glyph tests. That is P5's compiled-texels case exactly
(`--features jit`), which is in no leg on purpose; `scm_twins.py` reports
it as a timeout rather than failing the guard, and the jit build is the
way to verify it.

## 2n. D–K, 2026-10-20 — the second pile, and what it was made of

With A–C's shared causes gone the next full run read 38 pass, 47 "real",
and the real pile turned out to have shared causes of its own. Each was
found by one probe against a Scheme modeler with its log open, and each
unblocked a handful of files:

- **`((options-state) :param)` and `(rim :id)` never reached the migrator**
  — the FORM regex matched only `ev(`/`m.eval(` literals, and a per-file
  `check(label, form, want)` helper, `num(`, `drawn(` send forms too. Now
  any string literal whose first character is `(` is a sent form.
- **`vadd`/`vsub` in the camera bridge used `list-ref`**, and a test sends
  `[0 1 0]`, a vector. `vref` reads either.
- **A macro body is a body.** `when-let` expanded to `(begin body …)`, and
  scm hoists a `define` under `begin` to the enclosing lambda — so a nested
  `(do (def h …))` inside `gizmo-press`'s `when-let` shadowed the `h` the
  `when-let` bound, before it was initialised. The compat macros expand to
  `(let () body …)`, and the TRANSLATOR emits `(let () …)` for any
  `do`/`when`/`unless` block holding a `def` (`block`). No press had grabbed
  a handle under Scheme until then.
- **The prelude's own `dict` shadowed the compat layer's.** The Janet
  `dict` is identity (Janet's table is already what a prim reads); its
  translation loaded after 00-janet-compat.scm, and `%xf-arm` got a hash
  table — "a table is needed" — so no drag moved anything. The translator
  keeps a `COMPAT_OWNS` set (`dict`, `eval-file`, `eval-in-env`) and leaves
  a marker where the prelude defines one.
- **`set` answers its value in Janet**; `set!` answers nothing. `(or
  *comp-origin* (set *comp-origin* …))` handed `unspecified` to `vsub`. The
  translator emits `(begin (set! x v) x)`.
- **A string indexes to its byte**: `(<= 48 (key 0) 57)` is how the
  numeric-transform handler asks "is this a digit"; compat `nth` answered #f
  and every typed number raised. Also missing: `not=`, `all`, `mod`, the
  `print` family.
- **A parameter shadows the closed world** (§2m C); and `compat_names` read
  a `syntax-rules` template's `(define n e)` as a definition of `n`.
- **The verb shim's vector is positional**, and an absent `&opt`/`&named`
  parameter crossed as `#f` where Janet's shim sends `nil` — `(revolve c)`
  refused its own default tolerance ("must be a number, got false"). The
  shim wraps those parameters in `%absent-as-nil`, which turns `#f` into
  `unspecified`, which `to_api` already reads as `Nil`; a `Bool` parameter
  keeps its `#f`. (A first attempt filtered `:key #f` pairs out of the
  vector, which ate a keyword-VALUED parameter followed by an absent one:
  `(add-curve c :name :c)` came back as `curve-0`.)
- **An array at the PRIM boundary reads as nothing.** The embed `Val` has
  no record variant, so a `<janet-array>` handed to a `textv`/`nums`/`api`
  parameter was empty: `%set-toolbox` drew a palette of blank buttons.
  `prim!` records every installed name, `(%prim-names)` lists them, and
  99-scheme-only wraps each once so its arguments go through `unarray`.
  `(%show v)` does the same for what the eval channel PRINTS — the channel
  wraps a non-defining form in it, so an array answers as its items and a
  table as its alist instead of `#<<janet-array>>`.
- **The harness's Scheme over the prelude's arrays**: `(map f *handlers*)`,
  `(length arr)`, `(apply max arr)` are the SAME name in both dialects, so
  the migrator cannot respell them. 99-scheme-only redefines `map`,
  `for-each`, `apply`, `length`, `reverse` to read any sequence through
  `to-list` — captured idempotently, and with their rest arguments taken
  through `to-list` too, because a rest parameter is a VECTOR in this scm
  (`ray-scm` on disk was older and said list; probe the binary you run).
- **D — `match`**: one site in the prelude (13-outliner), flat tuple
  patterns `[:kw a b]`; the translator's `r_match` writes the `cond` with
  `janet=` on the head and `nth` binds. outliner and eval_adds_object pass.
- **E — the closure prims**: `tools::pick` and `tools::script_tool` keep a
  `Callback` — a rooted Janet function or a Scheme `Proc` — and fire it
  through `scheme::call_proc` (the parked host inside a prim, the session
  otherwise). `%pick-arm`/`%tool-arm-script` have Scheme twins.
- **F — the eval path**: `with-dyns` is a compat macro (`:out`/`:err`
  capture the body's printing into the buffer; other keys `setdyn` and
  restore), `eval-in-env`/`eval-string` read the string's forms in order,
  `debug/stacktrace` of a Janet fiber translates to `#f` (the guard's
  message is already in the buffer), `help` reads `%verb-doc` for a ray
  verb and scm's `doc` for the image's own definitions.
- **K — `origin-form`** reads `%origin-source` (the record's Scheme spelling
  now) with the material substituted; `%object-parent` and
  `%transform-object` have Scheme twins.

**Measured after D–K: 55 of 117 pass on the Scheme prelude** (38 after
A–C, 24 before), 42 real, 8 timeouts, 7 absent, 5 dialect; the Janet gate
105/105; the pass set is 55 files. The closure prims then found one more
rule: a procedure reaches a plain `define_fn` host function as
`Val::Opaque` — only `define_fn_with` is handed a `Proc`.

**I — the timeouts, and three more rules (2026-10-20, later).** Six of
the eight timeouts were not hangs: a wait loop reading Janet's spelling of
"nothing came back" — `val(…) in ("nil", "")` — never sees `#f`, so it
waits out its own deadline (`Modeler.nilish`, a migrator pass). The rest
of what the timeouts hid:

- **A string is its bytes.** `(each c src …)`, `(src i)`, `(get src i)`
  all answer a byte in Janet; the compat layer answered a char and the
  editor's span scanner compared `#\(` to 34, so no editor send authored
  anything (tree_live, live_coding).
- **A body is sequential.** `(f) (def n (count)) (use n)` counts AFTER the
  call in Janet; scm hoists an internal `define` to the top of its body, so
  `history-read` counted the shapes before evaluating the form that built
  them and every scene had "no stages". `seq_body` wraps a body from its
  first define-after-an-expression in `(let () …)`, as a post-walk over
  every lambda, function define and let in the output (`fix_bodies`) —
  the `eachp` rule builds lambdas `r_fn` never sees.
- **`(eval f)` with one argument is not the live top level** in scm;
  Janet's is. `janet-eval` evaluates in `(interaction-environment)`.
- **`parse-all` answers an ARRAY**, as Janet's does: a list of forms whose
  first car is a symbol reads as an alist to `eachp-of`.
- **A procedure reaches a `define_fn` host function as `Val::Opaque`**;
  only `define_fn_with` is handed a `Proc`. The closure prims are host-aware.
- **A rest-bag shim** (`(add-point-light . args)`) carries keyword pairs as
  given, so `:name #f` from a caller whose own `name` was absent needs the
  nil reading inside the bag (`%rest-absent-as-nil`, in place — never
  dropped).
- `unify.janet` is a prelude file beside `prelude/`, and the scene-mode
  rule is "outside modeler/scripts", not "outside prelude/".
- Two timeouts are not the port's: `test_scene_load_camera` loads stiletto
  (1.1 M interpreted texels, the jit case), and `test_agent_mcp` waits on
  the MCP server's own round trip.

**Measured after the GC fix: 68 of 117** — `test_curve_degree`, and nothing
lost. One revert came with it: `%rest-absent-as-nil` (§2n) turned `:key #f`
inside a REST BAG into absence, and `in_a_bag_false_is_a_value_but_not_an_
argument` forbids exactly that — `(add-sphere-mesh … :smooth #f)` means
flat facets and a promoted `#f` rounds a model off silently. A bag's
contents are the author's. The one call that needed it, `add-light`
forwarding a nil `:name`, omits the key at the call site, which is the same
call in both languages.

**Measured after J and the manual family: 67 of 117 pass** (63 after I,
55, 38, 24 at §2m), 38 real, 6 dialect, 3 absent, 3 timeouts; the Janet
gate is 110/110 — five files more than it was, because J1 added the
`RESULT:` line they were missing. The pass set is 67 files.

**Measured after I: 63 of 117 pass** (55 before it, 38 after A–C, 24 at
§2m), 41 real, 6 absent, 5 dialect, 2 timeouts — and the two left are the
two that are not the port's. Nothing regressed; the pass set is 63 files.

## 2o. The in-image manual, and the one defect that is not the port's

**The manual family (F's last piece).** ray's `(api)`/`(apropos)`/`(help)`/
`(view-help)` and the agent's three projections live in `scripts/help.janet`
and build their index by walking the Janet environment for a `:category`
stamp. None of that is translated, and none of it needs to be: in this
image the REGISTRY is the stamp. `%verb-index` answers every verb as
`(name category summary)` — the summary derived by the same rule
`api-summary` uses, so the two images' index lines read alike — `%verb-doc`
adds the `(defined in …)` footer from the registry's own grouping, scm
answers its globals (`(apropos)` with no argument is `global_names`) and
their docstrings (`doc`), and `*help-display*` is the hook the modeler
already binds for the editor's reading view. `api`, `api-display-taxonomy`,
`api-prompt`, `api-brief`, `api-names`, `view-help` and `all-bindings` are
in 99-scheme-only.scm; `help`/`view-help` are MACROS, as ray's are, because
a bare verb name is how anyone asks.

The catalog needed one more thing: **the vocabulary baseline is taken
before the modeler's layer**, which is where `script::boot` takes Janet's.
Snapshotted in pure Scheme (`(define %vocab-baseline-names (apropos))`)
because the compat layer that spells a table is one of the files about to
load; 99-scheme-only turns the names into the table `api-catalog` asks.
Taken after the prelude instead, everything is baseline and the catalog is
empty — which is what it was.

**`string/find` is a BINDING OF ITS OWN.** The compat layer aliases the
Janet spellings (`(define string/find string-find)`), and an alias holds
the closure that existed when it was written: rebinding `string-find` to a
fast version left every `string/find` caller — which is the harness and
most of the prelude — on the slow one. The agent catalog's exhaustive walk
took 31 s and the eval channel's read timed out; with both spellings
rebound to a host search it is 0.3 s, and a 344 KB scan went 5.5 s to
0.1 s. Two rules out of one bug: rebind the alias too, and a 25 M-iteration
character loop belongs in a host function.

**And the host search answers CHARACTER indices**, not bytes, because
`substring`/`string-ref`/`string-slice` in this layer count characters. The
first version answered bytes and `(string/slice s 12543 12622)` walked off
the end of a 12 586-character buffer — one em dash in a comment. The same
mistake, made twice in an hour: `length` on a string was byte-based for a
while for the same "Janet counts bytes" reason, and it broke the same way.
ONE UNIT FOR THE WHOLE LAYER, and it is the one the operations use. Where a
test compared a buffer to a file's byte count it now asks the image for
both lengths, which is the comparison it meant.

**A host function that re-enters must park its Host** — `prim!` does it and
my two hand-written closure prims did not, so arming a capture (which
echoes its prompt line, which dispatches back into the prelude) reached for
the session while the session was borrowed: "the host function `%pick-arm`
panicked".

**An scm GC panic, found by the history tool.** With `eachp-of` reading
arrays, `history-read` stages doric-column (7 of 48 forms), the tool arms,
`history-shots` renders frame 00 — and the `on-render-done` hook then
raises `the interpreter panicked: gc: a root points at free slot 3931`,
which poisons the image for the rest of the test. **It has a second reproduction now**, and the
two together say more than either: arming the curve tool panics the same
way (`[pick] curve-tool: the interpreter panicked: gc: a root points at
free slot 3931`), and that path holds a `scm::Proc` — the capture's `then`
closure — across a re-entrant call that echoes through the prelude.

So the shape is: a host function is called with a `Val::Proc` among its
arguments, it re-enters the interpreter (through the parked `Host`), and a
collection during that re-entry frees the slot the argument's root points
at. The embed API's contract is that a `Proc` is rooted while a clone
exists, so either the host call's arguments are not in the root set during
a re-entrant call, or the root table is not traced from there. Track A,
with a minimal reproduction to write — and it has to be a RUST test, not a
Scheme script: the same shape in pure Scheme (a global set to a closure, a
long allocation, then a call) is fine, because nothing there crosses the
host boundary. `define_fn_with` taking a procedure, calling back into
Scheme through the `Host`, allocating enough to collect, then calling the
procedure.

It blocks `test_history_tool` and `test_curve_degree`, and anything else
that hands a closure to a prim and then lets the event loop run.

**Known and left: nil versus false at a keyword parameter.** `(set-toolbox
:auto-hide false …)` and `(set-toolbox …)` both hand the prelude `#f`, and
its `(if (nil? auto-hide) 1 …)` reads the explicit false as "not given" —
the palette auto-hides, and `test_toolbox`'s panel rect is #f. The value
rule (nil IS #f) is right for the image; this one shape wants the Janet
source to spell its default differently, not the port to grow a nil.

**D. Janet's `match` — 4 files** (eval_adds_object, group_objects,
outliner, scene_editor).
☐ D1. Read the four usages. If they are the simple shapes (a keyword or a
      literal per arm), a `match` macro in the compat layer covers them —
      scm's `syntax-rules` matches vector patterns and the dispatch can be
      `janet=` at run time, as `case` already is.

**E. The two closure-taking prims — 3 files** (arc_tool, curve_degree,
curve_tool).
☐ E1. Decide what a scm `Proc` means to the tool layer: `%pick-arm` and
      `%tool-arm-script` take Janet closures and hand them to `tools/pick.rs`
      and `tools/script_tool.rs`, which call them later, from the kernel,
      possibly after the eval that made them has returned. A `Proc` is
      rooted while a clone exists (`scm::Proc`), so the shape is storable;
      the question is which image's prelude the callback runs in, and the
      answer under a Scheme prelude is the Scheme session through `Host`.
☐ E2. Convert the two prims; the `layering.rs` hatches stay by name.

**F. The eval path — 4 files** (agent_during_render, ai_catalog_modes,
history_tool, help_editor).
☐ F1. The hand port §2c named: `with-dyns`, `parse-all`, `help`,
      `debug/stacktrace`, and the agent sandbox (`table/setproto`). These are
      Janet's REPL machinery; the Scheme side of each is `%scm-eval`'s job in
      Rust, and `(help …)` wants the manual's text from the registry rather
      than an environment walk.

**G. Boot-time replay — 3 files** (toolbox, toolbox_sides, ui_config):
"the prelude installs :default", "the booted palette reads back as data".
☐ G1. Diagnose: 18-ui-config replays the saved toolbox over what every file
      above defines, and the Scheme boot order or the session file's dialect
      differs. Likely the `MODELER_SESSION_FILE` is Janet source.

**H. The `.rec` recording format — 1 file** (recorder).
☐ H1. A recording is written as Janet source (`@{:version 1 …}`) and read
      with whichever reader owns the prelude. Decide the format for M4 —
      the Scheme spelling, or a dialect-neutral one — and write both.

**I. The three timeouts** (agent_mcp, render_film, render_out) and
tree_live.
☐ I1. Run each alone with the modeler log open. A hang with no output under
      the Scheme prelude has been a double borrow or a self-recursion every
      time so far (§2i, §2k); these need the same look.

**J. The gate itself.**
J1 is DONE (2026-10-21), and the warning was the right one: the five got
their `RESULT:` line, the gate went 105 files to 110, and THREE of the five
failed on Janet the moment they were counted — all stale expectations
against features that had grown underneath them.

- `test_editor_help_menu` asserted "menu has 5 items"; Copy and Paste were
  appended to the editor menu later. What it means is that the four sends
  and view help are the first five sectors, which is what it checks now.
- `test_editor_send_menu` compared the armed pie to a printed vector of
  exactly those five labels, and `(*editor-menu*)` to `""` for "no menu" —
  it prints `nil` (and `#f` under Scheme), which is `Modeler.nilish`.
- `test_image_markdown_view` asserted `"# ray" not in reading`, and the
  fixture is the repo's own CLAUDE.md: `cargo test   # ray + scm` is a
  shell comment inside a code fence, which the reading view leaves alone,
  correctly. A heading mark is at a LINE START.

None of the three was a port defect, and none would have been found
without the line. 110/110 on Janet after the fixes.

☐ J2.
`test_chair.py` calls `%update-material`, which exists in neither
      dialect; the Janet run tolerates the error. Remove or replace it.

**K. Two names of ray's own prelude residue** (§2h).
☐ K1. `origin-form` — answers the recorded call AS DATA; needs a Form→Val
      conversion that does not exist yet (`%rerun-form` covers the rebuild).
☐ K2. `record-origin` — a thin wrapper over an installed prim; unported
      only because nothing has asked.

**L. Whatever the measurement says after A–C.** Those three remove the
causes shared across files; what is left is per-test, and
`scripts/dev/harness_scheme.py` names the file and the first failing line
for each. Expect the "real" pile to shrink to the genuinely behavioural.

**Then M4**: `janet` stops implying `scm`, the 21 `janet` cfg sites answer
to the binding that is present, `--features scm` alone builds a modeler
with no Janet in it, and Janet still builds (§7 of the port plan).

## 2p. L, first pass — six refusals nobody could hear

The per-file residue opened with the toolbox family (§2m G), and all three
files had the same shape of cause: **a refusal swallowed by a `try` whose
other reading is "nothing saved, use defaults"**. That reading is right —
00-session's docstring argues for it, and a missing session file must not
stop a boot — which is exactly why six separate defects sat under it. Each
was found by asking the image one question with the log open, and each took
one line to see once asked.

**The boot audit was the instrument, and it was broken first.** `boot-audit`
asks what a forward hook var holds through `(get-in (curenv) [sym :ref 0])`,
and the compat layer's env entry carries `:value` and `:doc` but no `:ref`,
so that path answered #f for every name in the image. For the six hook vars
whose 00-decls placeholder is a lambda, #f ≠ the placeholder and the check
passed vacuously; for the three whose placeholder is nil it matched and they
were reported unbound on every Scheme boot. Both halves were wrong and they
cancelled into something that looked like a finding. With `:ref` answered,
the three went quiet and the SIX spoke — which is how the next one surfaced.

**`script::boot`'s Scheme branch returns before boot's tail.** The Janet
branch loads the prelude, then `load_user_init`, ray's prelude, unify,
help.janet, `(ui-config-replay)` and `(boot-audit)`. The Scheme branch
returns where the prelude ends, and `load_scheme_prelude` reproduces the
middle of that list but not the last two. So no Scheme-prelude modeler had
ever replayed its saved UI configuration, and no Scheme-prelude modeler had
ever audited its own load order — in the image most likely to have a breach.
The replay and the audit now run at the end of the whole-prelude load, and
non-fatally, for the reason they are non-fatal there.

**`<` is where a sort fails and a file disappears.** Janet's comparisons are
polymorphic; Scheme's are arithmetic. Three places in this prelude sort by
name, and each raised `<: not a number: "b"` inside a `try` that reads an IO
failure the same way — so `session-write`, which sorts its keys, **silently
dropped every session store with more than one key in it**. The file kept
whatever one-key version had been written before, which is why the toolbox
came back unconfigured and why the symptom read as "the replay does not
work" rather than as a comparison. `test_recorder` was saying it out loud
the whole time (`wrote error: <: not a number: "scene"`), classified "real".
The fix is `janet<` and three siblings over the existing `janet-less?`,
picked by the same proof rule `r_eq` uses, so every numeric loop stays on
the host primitive and only a comparison that proves it is text dispatches.

**`parse` did not exist in this image.** Its caller at boot reads an unbound
variable as "no session state", so the absence was invisible for the whole
port. scm's own `read` is not a substitute and not by a small margin: `@{:a
1}` scans as the single symbol `@{:a`, and a `#` comment — which every
session file opens with three of — is "unsupported `# `". It is a reader
now, in the compat layer, for the subset the three Janet-data formats hold:
session.janet, the recorder's `.rec`, and a toolbox button's `:call` (which
is the TEXT of a form, parsed when the button is pressed). It answers this
layer's shapes, so a parsed `(curve-tool)` is a list `janet-eval` can run.
That also answers **H** for the `.rec` file: the format stays Janet data in
both images, because one reader is cheaper than two formats.

**And an array printed as `@(…)`, which is a spelling in no dialect.**
`readable` composed `@` with the readable of the LIST holding the items.
Janet writes `@[…]`, the new `parse` reads `@[…]`, and `Modeler.norm` folds
`@[` and `[` into `(` while leaving `@(` standing — so a Scheme-printed
array never matched the Janet reply it was being compared against either.
One line, three symptoms.

**The nil-versus-false item (§2o) is DONE and did not grow a nil.** Only two
verbs in the prelude distinguish an absent named parameter from one given as
`false` — `set-toolbox` (:auto-hide) and `set-reference` (:flip-x :flip-y
:on) — and both now take a `&keys` BAG and read it with a sentinel, through
00-decls' `opt-get`. A key the caller never wrote is not in the bag;
`:auto-hide false` is in it, holding false. That is the rule
`in_a_bag_false_is_a_value_but_not_an_argument` already settled in scm, used
from the other side: the bag is where the difference still exists. The
Scheme half of `opt-get` is the compat layer's (`COMPAT_OWNS`), because the
translator spells a bag as a flat rest list rather than a table.

**Regenerating also exposed a translator bug with no test to its name:**
`&named` was handled only by falling through the "plain" branch, so a lambda
list that OPENS with it translated correctly and one that follows `&opt`
turned the marker into a parameter — `(define (set-reference pane (path #f)
(&named #f) (opacity #f) …)`. Every keyword argument shifted by one, which
is the right answer by positional luck when they are passed in declaration
order and wrong otherwise: `(trace-curve :name 'x)` put the name in `depth`.
Two sites, both in 17-reference, neither covered. **A mistranslated lambda
list cannot fail loudly** — it is still a valid procedure — so the only
protection is reading the signatures the translator writes.

Measured after this pass: the Janet gate is 110/110, the modeler legs 186
and 248, and `test_toolbox` passes on the Scheme prelude. `ui_config` and
`recorder` are past persistence and now fail on injected-input chrome (a
recorded toolbox click arming its tool), which is the next thread rather
than this one.

### L, second pass — the overlay family, and two more lost distinctions

**81 of 117** (75 measured, then chair / object_names / camera_match, then
these three). The overlay "0 px" pile — `control_net`, `hybrid_wire_covered`,
`toon_hybrid` — was two causes and neither was a drawing pass:

**`(show-control-net false)` TOGGLED.** Four `show-*` verbs take `[&opt on]`
and ask `(nil? on)` to tell "not given, so flip" from "off"; nil IS false
here, so an explicit `false` read as absence and every frame after it was one
toggle out of step. The cage test measured 889 px of cage in the frame where
it had asked for none, which reads as an overlay ignoring its argument.

§2o's ruling holds — spell the Janet source differently rather than grow a
nil — and for a POSITIONAL argument the spelling is a **rest list: absence as
a LENGTH**, which is the same number in both languages. `arg-or` in 00-decls
is the positional sibling of `opt-get`; no call site changes and the
docstrings' `&opt on` still describes what a caller writes.

**A hex literal built at RUN TIME is in no reader.** The other two set a
sentinel ink with an f-string — `(set-color :toon-ink 0x{INK:06x})` — and the
migrator's hex pass rewrites SOURCE, so a literal that does not exist until
the test runs went out as `0x…`, which Scheme cannot read. The colour never
changed, the test counted a colour nothing was drawn in, and the pass it was
measuring had been working all along.

**The method that found both: shoot the same frames in both dialects and
HASH them.** Under Scheme the off and on frames were byte-identical while
Janet's differed 58 px to 889 — which says "no cage" and is wrong. The cage
was in the other frame. A pixel count tells you a number; the hash tells you
whether anything happened at all, and that is the question when a count is
zero.

Also fixed in this pass, each a one-liner with a long reach: `save-scene`'s
guard matched Janet's comment leader so a Scheme-prelude modeler refused its
OWN snapshot (`test_chair`); ray's `%nth` could not index a sequence a host
layer owns, which `%count` HID by being late-bound to the modeler's
array-aware `length` (`camera-match`); `camera-match` demanded `list?` where
`:image` two lines above asks `%count`; and `krell_match.py` is hand-
maintained, so the migrator never saw the Janet it sends.

## 2q. L is finished — 114 of 117, measured 2026-10-22

**`pass=114  timeout=3`**, and the three are exactly the three §2m excludes:
`agent_mcp` (waits on the MCP server's own round trip), `history_tool` and
`scene_load_camera` (both stiletto's 1.1 M interpreted texels, the
`--features jit` case that is in no leg on purpose).

**CORRECTED 2026-10-22, later the same day.** This section first gave
`history_tool`'s cause as §2m E's closure ruling. That was wrong, and
measuring it took four minutes: its capture path PASSES — arming, the chip
strip, scrubbing, the wheel detent, the first six checks — and it fails on
the line after, loading stiletto. Two defects were stacked and the first hid
the second. (1) The bake: 677 s measured, against ~7 s of Janet, so the 180 s
read answers `[no reply]` and nothing downstream gets a number. (2) Behind
it, **a scene defines `fit` and `history-read` evaluates the scene** —
`(def [forms stages why] …)` goes through `%macro-fit`, whose body `(fit coll
n)` resolves LATE, and `stiletto.scm:1428` is a one-argument camera helper.
So the prelude applied the aeroplane's `fit` to a collection and a 3,
eighteen stages after the fault. Janet does not have the bug because it
resolves a top-level call at COMPILE time.

Fixed in ray @2b896bae (`fit` → `%fit`). The rule `%macro-fit`'s own comment
states — a `%`-name is shadowed by nothing — had been applied to the macro's
ENTRY POINT and not to the helper, because the comment was written about a
parameter shadowing a template's free identifier and missed the other writer:
**the scene being READ defines into these globals too.** 295 of the 306
compat names are bare; one other collision exists and is dormant (`pair`,
two arguments, against `joined-patches.scm:78`'s three).

So the §2m E ruling gates NOTHING — which is what unblocked M4. The Janet gate held
110/110 on every batch and the five cargo legs read 664 / 681 / 186 / 248 /
901. **That is the end of M3** — `modeler/tools/scheme-pass-set.txt` lists
all 114.

`curve_tool` was on §2m E's blocked list and did not belong there: nothing
in it reaches a closure-taking prim. The ruling (which image's prelude a
stored `scm::Proc` runs in) still blocks `history_tool`, and is still
Kevin's.

### The twelve causes of the last pass, and the shape they share

Ten of the twelve are a DISTINCTION THIS IMAGE CANNOT HOLD, or a name that
answers in one image and not the other. Neither kind can refuse — which is
why every one of them arrived as a wrong answer somewhere else, usually two
layers from the cause:

| what was lost | how it arrived |
|---|---|
| `[&]` — Janet's "any arguments, bind none" — translated to `(lambda () #f)` | a fixture stub refused the call it existed to swallow: "got 2 arguments, which is more than it takes", blaming the caller. The same shape was in the AGENT POLICY FENCE: every fenced verb raised an arity error instead of its refusal sentence |
| a bracket literal is a VECTOR, and `pair?` on a vector is #f | `(group-objects [:a :b :c])` took the whole vector for ONE name and handed it to `string-append` |
| `record-origin` is Janet's PRIVATE wrapper; ray's Scheme calls the prim direct | a moved glTF import saved 7.3 MB of baked mesh with its textures dropped, instead of a 214-byte `(import-model …)` reference. The raise left `load-model` and reached the CLI, which prints and carries on |
| Janet resolves a top-level call at COMPILE time; Scheme resolves it late | `clear-models` called the `clear-scene` that is shadowed later in unify, so emptying a file's objects also forgot which file it was |
| a bare `&` inside a `let`/`each`/`when-let`/`loop` binding does not shadow the closed world | `(each pair snaps … (pair 0) …)` stayed a CALL, `restore-snapshot` applied a tuple, and the raise came out of the gizmo's ESCAPE-CANCEL — which reads as Escape not being wired up, and took eight other checks with it |
| `:divisions nil` means "clear what was recorded"; `#f` after a keyword in a bag is the author's own false | a closeness typed into the mesh chip rebuilt the surface at 12x8 anyway |
| the same, in an ARGUMENT DEFAULT: `:scene false` vs an absent `:scene` | every `(play-input path :delay 0)` in the suite arrived as "skip the scene restore" |
| `(string false)` is "false" in Janet and "" here, because `(string nil)` is "" and the two are one value | the curve tool's corner flags read `" true  "` where the test wanted four words |
| a tuple compared as NEITHER-LESS against every other tuple | `(sort-by first scored)` over `[[i len] c]` made every key equal; a stable sort then answered the pool's arrival order, and TAB offered `%add-sphere-light/unguarded` for `add-sp` |
| a negative slice index is `n + i + 1`, not `n + i` — Python's convention was written in | every negative slice in the prelude lost an extra element. `c` closes a curve by repeating the first pick, and `(slice world 0 -2)` recovers the open list, so a three-point loop became a two-point curve. A 2-point curve is a perfectly good curve |
| `%scm-eval` / `%scm-complete` are the SECOND VM's door, and under this prelude the prelude IS the second VM | the prim answers "this modeler was built without the scm feature" — true, and useless. Nine checks read as broken SLIME sends; the scene they were sent to was fine |
| no PEG in this image | `keyword-candidates` raised out of `symbol-candidates`, so the editor's TAB had no completion pool at all |

The other two are the harness asking a question that has two right answers:
a sort whose key has TIES (`(+ x z)` is the diagonal on a grid, so every
anti-diagonal is a tie, and the two images took different eight faces — a
cap 3/16 wide against 2/16, which clamps a notch earlier), and the 30-second
read timeout, which is Janet's number: 8,661 vertices sorted by a closure
key is 0.1 s there, 4.9 s here alone and **29.5 s** inside the test. Losing
that race does not look like a timeout — the form answers `[no reply]`, no
numbers get scraped, and it surfaces as `IndexError` three lines later.

### Two things worth keeping past this port

**A check that cannot fail is worse than a red one.** `((options-state)
{field})` is an index whose key the test COMPUTES, so the migrator's rule —
which wants a literal — left it a call. Both sides of "the replayed wheel
moved the chip to the same value" were then the string `error: not
applicable: #<<hash-table>>`, and comparing them passed. Two more of the
same kind were found by reading the detail text beside a PASS.

**An edit whose `old` is an earlier edit's `new` breaks idempotency for
both.** `harness_hand_edits.py` skips an edit whose replacement text is
already present; layering a second edit on the first leaves the file holding
neither text, and the next regeneration asserts. Fold it into the edit it
amends, and prove it by regenerating from `git checkout` to a byte-identical
tree. The same test's `new` ALSO skips an edit silently when its replacement
text happens to appear elsewhere in the file — that cost three `.janet`
buffer names that looked fixed and were not.

## 4a. M4 done, 2026-10-22 — `--features scm` is a modeler with no Janet in it

ray @eb7d13e4. `scm` gained `scripting`, so it carries the binding instead of
pulling an unreachable interpreter into a pure build. **`janet` still IMPLIES
`scm`** and that is not M2 residue: a moved section's Janet shim forwards to
the Scheme image, so a `janet` build with no second VM has nothing to forward
to. Three configurations now — pure (neither), `scm` (Scheme only), `janet`
(both).

It boots and it works: 23 prelude files, `add-sphere` answers `:s`,
`scene-stats` reports, and `(history-tool :src "…/doric-column.scm")` stages
7 and scrubs to 56 objects, with no Janet in the process. Legs
664/681/186/884/901/248 plus a NEW one at 209 (`cargo test -p modeler
--features scm`); Scheme harness 114, Janet 110/110.

**The stage was sized 0.2 as feature wiring, and that is not what it was.**
Three findings, each of which had been invisible for a structural reason:

- **`ray::scripting` is not the VM.** 52 of the 69 names the modeler took
  from it are scene-store accessors A1.5 had already lifted to
  `crate::scene_store`; `scripting.rs` re-exports them and ends with the
  invitation — "callers may move at leisure". 109 sites relocated
  (@4235c380). `src/layering.rs` cannot catch this class: it forbids
  `scripting::ffi` and `-> Janet`, and a store accessor reached through the
  VM module's namespace is neither. Four sites were not relocations at all —
  they asked `is_booted()`, whose body tests the Janet env pointer, to mean
  "is there a store to mirror into"; `scene_store::image_booted()` answers
  that where it belongs.
- **Eight `cfg(feature = "janet")` sites were on the wrong feature** — the
  recorder and caption prims, ordinary `prim!` declarations naming no VM and
  already in `install_scheme`'s list. Asking for `scm` alone deleted the
  prims and kept the list. Invisible for exactly as long as `janet` implied
  `scm`. **Verify a regated prim is PRESENT IN THE RUNNING BINARY**, not
  merely that the build compiles.
- **`boot()`'s own early return was the evidence the split was already
  real.** Under `MODELER_PRELUDE=scheme` it returned before the prelude
  files, before `register_api`, before ray's Janet prelude — so every Janet
  line after it was already dead on that path, and the VM, the vocabulary
  baseline and 355 registrations before it were built for a VM nothing would
  ask.

**Two gaps, named rather than hidden.** The async scene-load worker is
Janet-only, so under `scm` alone `%load-scene-async` does not exist and a
scene opens on the main thread — `test_async_load` and `test_scene_async_load`
pass under the Scheme prelude today only because that build still has Janet
to run the worker in. And there is no `init.scm`.

### `(%scm-ready?)` is a FEATURE PROBE, not a capability question

Worth its own heading because it cost a gate run and it generalises.

With the door prims gated out, `00-decls`' fallbacks answer in their place,
and the sentence they answered with — "this modeler was built without the scm
feature" — names a build. It is false under `MODELER_PRELUDE=scheme`, where
the image those prims reach is the one asking (§2p lost nine checks to
reading that refusal as a broken SLIME send), and false again under M4's
build, where the feature it blames is the only one ON.

The first fix was to override all seven names in `99-scheme-only.scm` to act
locally. Each override tested correctly in isolation — `(%scm-eval "(+ 1 2)")`
→ `("ok" "3" "")`, `%scm-call` applying the bound name, honest refusals for
unbound ones. The full pass set then failed `test_scheme_buffers`, because
that file asks `(%scm-ready?)` **whether to skip**: answering `#t` made it
stop skipping and then fail every check about a door that genuinely leads
nowhere from inside. The refusals were the right ANSWERS all along; only the
explanation was wrong, and it is fixed where it is written — "no second image
in this modeler to reach", true in all three builds.

**And a test that SKIPS counts as a pass in `--pass-set`.** So a change that
makes one stop skipping surfaces as a new failure, not as new coverage. No
probe in isolation can show you that; only the gate can.

## 5. M5 — scoped 2026-10-22, with one ruling left open

**M4 was the last stage this plan had, and the port is not finished**, for a
reason the stage list never had to say out loud: `MOVED` is three files. The
other twenty regenerate from Janet, byte-identical (confirmed twice during
M4). So editing the modeler's prelude still means editing **Janet** and
retranslating, and 19,291 lines of Janet are still maintained. M0–M4 bought a
SECOND runtime, not a replacement.

Where it stands, measured:

| | lines |
|---|---|
| Janet prelude (`prelude/*.janet` + `unify.janet`) — the source today | 19,291 |
| Generated Scheme mirror, 24 files | 25,769 |
| Hand-ported Scheme (`MOVED` + `99-scheme-only`) | 2,596 |
| Translator + migrator (Python) | 3,520 |
| Harness eval sites | 1,658 across 117 files |

### M5's priorities changed once the destination was said out loud

Kevin, 2026-10-22: the end state is **one extension language (scm) and a
complete API other languages layer on, like Python** — and *"I don't want to
completely get rid of janet yet because we know there are some things it does
better than scm."*

That reorders §5, and it exposes a contradiction in the first draft of it.
**"Keep Janet" and "Janet is the prelude's source of truth" cannot both be a
strategy.** Twenty of the twenty-three Scheme files are GENERATED from Janet,
so Janet is not being kept as an option — it is being kept as the MASTER,
with scm as the generated copy, and every prelude edit still goes through it.
So M5a is not a tidy-up placed in the middle; it is the precondition for
Janet being optional at all. It moves to the front of the staged work.

**And it adds a ruling that was not in the first draft: what IS Janet for?**
The advantage is real and measured, not a hunch — stiletto's texels are ~7 s
in Janet against 677 s here, and 8,661 vertices sorted by a closure key are
0.1 s against 4.9 s standalone (29.5 s inside the test, §2q). That is
interpreted throughput on closure-heavy numeric work, 30–70×, and P5's jit
(`--features jit`, compiled texels) is the intended answer and is in no leg.
So: does the jit close it, does Janet stay as a leaf-compute escape hatch, or
is slow authoring accepted with the bakes jitted? **The answer decides
whether Janet keeps a PRELUDE — expensive, two sources forever — or just a
VM, which is cheap.** Today it has a prelude by default, which is the
expensive option arrived at rather than chosen.

**The modeler's API is a LATER STAGE** (Kevin, same day: "ray only for now").
Nothing in M5 needs to anticipate it. Worth recording why it is a stage at
all: ray's verb layer is declare-once-bind-many — `src/api/*_api.rs` declares
and Janet, Scheme and `pyray` are three bindings, 178/178 verbs from Python
since A4 — while the modeler's 355 prims are declared once and bound TWICE,
into two in-process Lisps, with no registry a third language could
enumerate. An op in the modeler is reachable from two Lisps forever; an op in
ray is reachable from every binding ray grows.

**Which is why M5 gains a prerequisite, and it is nearly free:**

☑ **M5p — write the ray/modeler boundary down.** DONE 2026-10-22:
[`docs/ray-modeler-boundary-notes.md`](../../ray/docs/ray-modeler-boundary-notes.md)
in ray, indexed in `docs/README.md` and pointed at from CLAUDE.md. The rule
was recoverable from `src/geometry/ops/` versus
`modeler/src/tools/meshop.rs` and one comment in `mesh_api.rs`, and from
nowhere else — so an agent reaching for the obvious answer reaches for the
wrong crate, because the modeler is where the tool lives. Geometry and what
a save holds are ray's; the gesture is the modeler's; a ray verb takes a
NAME and a modeler verb takes the SELECTION.

Writing it measured three ops ray implements and ray cannot call — `Bridge`,
`BridgeFaces` and `BevelVertex` have no ray verb, so `pyray` cannot reach
them. That is the honest answer to "what modeling functions are fully
supported in just ray": nearly all, with three holes, and the holes are
verbs rather than geometry. (`SubdivideSmooth` looks like a fourth and is
not — `mesh-subdivide :smooth` reaches it.) Closing them is `mesh_api.rs`
work of the kind that file's own comment describes: assemble an `OpCtx`,
call the trait.

**Revised order: M5p (done) → client probe → M5a → the Janet ruling → M5c,
with M5b last and opportunistic.**

**M5d — the gaps, the leg and the client (0.3). The CLIENT PROBE IS THE FIRST
THING TO BUILD**, ahead of the leg decision it used to sit behind, because
the leg question needs a measurement that cannot be taken without it — and
since the modeler's default became `scm` (ray @b0dc7911) a plain `cargo build
--release -p modeler` produces a Janet-less binary, so `modeler_client`
reading `MODELER_PRELUDE` from the environment is now a live bug rather than
a future nicety: it answers "janet", sends Janet forms to an image with no
Janet reader, picks `.janet` scene paths that image refuses on purpose, and
uses the 30 s timeout instead of 180 s. The eval channel's `:scheme`/`:janet`
mode switch is the precedent for where the query belongs — protocol level,
needing no binding in either language. `%scheme-prelude?` cannot be the
probe: it exists only inside the Scheme image.**

- **Is the scm-only modeler a permanent leg or a pre-merge check?** This
  defines "green" for all of M5 and so cannot come after it. `pyray` is the
  cautionary case: a workspace member that is not a `default-member` went
  three days and 42 commits uncompiled with all five legs green.
- **`modeler_client` should ask the modeler its dialect** rather than read
  `MODELER_PRELUDE`. A Janet-less binary forces Scheme and the client cannot
  know; every M4 probe had to set the variable by hand.
- **The async scene-load worker, Scheme side.** The threading is already
  settled — `ray::api::scheme` says an `Interp` edits the scene of the thread
  it lives on — so the open question is the registry hand-back, not the
  thread.
- **`init.scm`.**

**M5a — the Scheme becomes the source (0.5).** Delete
`modeler/scripts/prelude/*.janet` and `unify.janet`; `scripts/scm/*.scm`
becomes hand-maintained; retire `janet_to_scheme.py`'s prelude mode and keep
its scene mode for the twins. The per-file shape is demonstrated twice
already: `02-scene.janet` and `07-camera.janet` are down to 105 and 109 lines
of shim from whole sections. Twenty more files, the same move.

☑ **RULED 2026-10-23 — Kevin: retire it.** M5a is done in ray (see "M5a as
built" below). The ruling as it was put:

**THE RULING, and it cannot be deferred past M5a: does a Janet modeler
prelude survive at all?** If it does not, `MODELER_PRELUDE=janet` stops being
a thing, the 110/110 Janet UI gate goes away, and `harness_touched.py`'s
both-dialects loop collapses to one. If it does, both preludes and the
translator are maintained forever, which is not a port. The recommendation is
the first; the reason it is a ruling and not a step is that it retires the
oracle every stage from M0 to M4 was measured against. **Kevin's.**

It also needs a replacement drift guard: `scm_twins.py` compares Janet's
round trip against Scheme's, and with no Janet prelude there is no left-hand
side.

**Corrected 2026-10-23: it does not.** `scm_twins.py`'s control is
`target/release/ray` (`scm_corpus.py:83`) — RAY's Janet prelude, which this
ruling does not touch. Also corrected: `test_agent_mcp` is not only a
timeout (38/50, 12 real failures, undiagnosed); `test_scene_load_camera` is
(9/9 at 420 s).

**M5a as built (2026-10-23).** What the plan did not count, and what the
0.5 became:

- **The Janet VM stays, as a SCENE image.** With the prelude gone the main
  Janet VM gets ray's verbs and ray's Janet prelude at boot, and
  `%janet-eval` is the Scheme side's door to it — a `.janet` scene's sync
  open, a `.janet` buffer's sends and the REPL's `:janet` dialect go there.
  Under `MODELER_PRELUDE=scheme` all three had been reaching the SCHEME
  reader: a `.janet` open answered `#f` with nothing loaded and nothing
  said, and the REPL's `:scheme` half was a door to a second Scheme VM that
  does not exist. The eval channel is unchanged — both its dialects reach
  the prelude image, which is what the harness's forms are written for
  (M5c's business).
- **The modeler manual had only a Janet generator** (help.janet). There is
  a Scheme `generate-manual` now; `api/modeler` is 521 verbs against 529,
  the gap being names that are not callable in the Scheme image, and its
  categories are the registry's.
- **31 cargo tests spoke Janet.** Ported, which surfaced the real defects:
  `(live)` turned live render off and `toggle-grid` could not hide the grid
  (an `#f` default cannot tell absent from false); an automatic mesh showed
  as "false" (`#f` reached Rust as `Bool(false)`, not `Nil`); hook
  arguments and option values came back inexact (`12.0`, `2.0`) because
  `num()` bypassed `number_val` and the ladder's clamp ran through ±inf;
  `tool-census` and a typed point called `peg/match`, which this image does
  not have; the REPL printed a bare variable as `#<<janet-array>>`.
- **`init.scm`** replaces `init.janet`, which is reported if found.
- **`janet_to_scheme.py` scene mode** reads its function set from a frozen
  snapshot of the retired prelude plus the live Scheme defines; all 17
  committed twins translate identically.

**M5b — the compat layer dissolves (1.5). RECOMMENDED NOT AS A CAMPAIGN.**
The mirror is Scheme written in Janet's vocabulary: ~1,870 sites —
`janet-string` 514, `janet-table` 305, `janet=` 273, `janet-length` 226,
`janet-array` 161, `each-of` 160, `janet-nil?` 119, `janet-index` 113.
Dissolving it is bigger than M0–M4 put together. It buys readability and it
eliminates the bare-global collision class outright (§2q's `fit`, and the
dormant `pair`) — but it risks re-breaking 114 green files for no behavioural
gain, and `00-janet-compat.scm` works. So: **dissolve a file's vocabulary
when already editing that file, never as a sweep.** If it is ever taken as a
stage it must follow M5a, or it is hand-editing generated files.

**M5c — the harness speaks Scheme natively (0.5).** 1,658 sites;
`harness_migrate.py` and `harness_hand_edits.py` retire. The migrator already
proves every rewrite, so this is largely committing its output and deleting
the translation step. **Worth doing only if M5a retires the Janet prelude** —
while both dialects run, the migrator is what keeps one test file honest in
two languages.

Order: **M5d → M5a → M5c, with M5b opportunistic.** ~1.3 of staged work
against M0–M4's 3.0, ending with one prelude in one language.

### The streaming-output dependency, and why no M5 stage is gated on it

Another agent is adding output streaming to scm concurrently (2026-10-22).
[port-plan.md](port-plan.md) §7 already settles the direction, checked that
day on both sides: scm captures a form's output into a string port and hands
it back with the answer (measured — 2.00 s of silence, then everything), and
so does the side it replaces. **Parity, not a regression**, and it does not
reach ray's long operations: a render broadcasts framebuffer strips to a
display thread, never text, so the 20 s terrain build was never telling a
script anything.

The caveat is about direction rather than blocking. The modeler is precisely
the **in-process host that wants the embed-API sink** rather than the
unsolicited socket message — that is [plan.md](plan.md) §13.3's own split —
and there is currently exactly ONE place hardening the output-with-the-answer
shape: `eval_to_triple` returning `[status text output]`, with `eval_channel`
prepending it. **M5 should not multiply consumers of that shape**, because
each becomes a migration site when the sink lands; and if the sink arrives
mid-M5, the modeler is its first natural caller.

The one place streaming would be VISIBLE rather than additive is the agent
pane, where a long `(ai …)` turn goes quiet. That is a user-facing win and it
sits outside this scope.
