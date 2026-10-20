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

**M2 — the prelude moves (1.0).** 892 definitions across 23 files, and the
division B5 asked for is already there: 22 numbered prelude files in boot
order, plus `unify.janet` last. A file lands when the harness is green with
it, so the unit of work and the unit of proof are the same thing.

A first draft of this document said the layer was one 6,538-line file and
that M2 would have to divide it first. That was `unify.janet` alone — the
bridge — and it missed `modeler/scripts/prelude/`, which is three times the
size and already split. Corrected the same day, before anything was built
on it.

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
