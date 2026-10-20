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
