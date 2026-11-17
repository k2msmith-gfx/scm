# Rust sends from inside a Cargo project — plan, 2026-11-17

**PROPOSED, nothing built. To do later** (Kevin, 2026-11-17: *"lets write a
plan for this to do later"*).

Kevin: *"can evcxr for rust resolve `use` statements and read the
Cargo.toml files for a project?"*

Partly, and the gap is the editor's to close. A Rust buffer's sends go to
evcxr ([editor-crate-plan.md](editor-crate-plan.md), "Rust has a REPL").
That works for a file that stands alone. A file that is one module of a
crate begins `use crate::…` and `use serde::…`, and evcxr knows neither the
crate nor its dependencies — so the first send from most real Rust files
fails. This plan makes evcxr know the project the buffer is in.

## What evcxr does, measured

evcxr 0.21.1, in the `--ide-mode` the editor drives it in (2026-11-17).

| sent | result |
|---|---|
| `use std::collections::HashMap;` then a use of it | works; the import holds for later sends |
| `:dep probe_crate = { path = "/abs/probe_crate" }` | compiles the local crate (cargo's own "Compiling probe_crate…") and answers |
| then `use probe_crate::shapes::area;` and `area(2.0, 3.0)` | `6.0` |
| `use crate::nothing_here;` | `[E0432] unresolved import` — `crate` is evcxr's own scratch crate |
| `mod local;` | `[E0583] file not found for module` — it does not read sibling files |
| `:help` | `:dep` is the only way a dependency is added; nothing reads a `Cargo.toml` |

Also in `:help` and likely to matter: `:offline`, `:toolchain`,
`:build_env`, `:cache`, `:clear`.

**Not measured yet** (R0 below): a published crate by version (needs the
network); whether a path dependency is rebuilt when its source changes
between sends; a workspace; features; a dependency that is itself a path.

## The idea

When evcxr starts for a buffer whose file is inside a Cargo project, the
editor tells it about the project, in evcxr's own words:

1. **The project's crate, by path** — `:dep my_crate = { path = "/root" }` —
   so its public items are there to `use`.
2. **Each of its dependencies** — one `:dep` for every entry under
   `[dependencies]`, the spec copied, a relative `path` made absolute.
3. **`crate::` means the project** — a send's `use crate::x` (and
   `crate::x` in an expression) goes out as `use my_crate::x`, because the
   code runs in evcxr's crate, beside the project's, not inside it.

Then `C-M-x` on a function in `src/shapes.rs` that uses its neighbours
compiles, and `C-x C-e` on a call of a project function answers.

Everything is in the shell (`edit/scm/26-rust-repl.scm`) and one small
reader of `Cargo.toml` in the kernel. `src/pipe.rs` does not change.

## What it cannot do, by construction

- **A crate with no library target.** Cargo cannot depend on a binary. A
  project that is only `src/main.rs` gets its `[dependencies]` and nothing
  of its own; sends from it work only as far as they stand alone. The
  editor should say so once, not fail quietly.
- **Private items.** Sent code runs OUTSIDE the crate, so it reaches `pub`
  items only. `C-M-x` on a private `fn` still defines that function in
  evcxr (the text is sent), but a call from it to another private function
  of the crate does not resolve. This is the limit users will hit most,
  and it is evcxr's model, not a bug to fix.
- **`mod x;`, `#![…]`, `impl` blocks for the crate's own types** defined in
  a send are evcxr's crate's, not the project's.
- **It is still not `cargo run`.** Build scripts, `cfg` set by features of
  the top-level build, and the binary's `main` are Cargo's.

## Stages

| | what | exit criterion |
|---|---|---|
| **R0** | **Measure what the table above leaves out**, on a scratch project: a path dependency edited between sends (is it rebuilt, and how long does a send then take?); `:dep` of a published crate, online and with `:offline 1` from a warm cache; a workspace member (what path, and do `workspace = true` dependencies need the root's table?); `features = […]`, `default-features = false`, `optional`, `package = "…"` renames; a crate named with `-` (`use` writes `_`); two `:dep`s of the same crate; the edition. | a table here under "R0 as measured"; D1–D4 answered with evidence |
| **R1** | **Read the project.** A kernel function that finds the `Cargo.toml` above a file and answers what the shell needs — package name, whether it has a lib target (`[lib]`, or `src/lib.rs`), the `[dependencies]` table entry by entry as TOML text, the workspace root when there is one — and a prim over it. A small reader of the TOML this needs, written here: `edit` takes no crate for it unless the workspace already has one. | unit tests over real manifests: this repo's, ray's, a workspace member's, one with every dependency form R0 listed |
| **R2** | **Tell evcxr.** On `rust-repl-start` in a project: the `:dep` lines, sent as ordinary requests before the user's, their output shown dim in the transcript (the first compile is slow and should be seen to be happening). A manifest that changed since — its size and time — is told again on the next send. evcxr is per PROJECT: a buffer in another project gets its own (the terminal and the agent already key on the project). | against the real evcxr, in a scratch project with a lib and a path dependency: `C-x C-e` on a call of a project function answers; a send that `use`s a dependency compiles |
| **R3** | **`crate::` is the project.** A send's `crate::` paths rewritten to the package's name, off the lexer's tokens so a `crate` in a string or a comment is left alone; `self::` and `super::` rewritten to the module the FILE is (`src/a/b.rs` is `my_crate::a::b`), where that can be told from the path. What was rewritten is what the transcript shows was sent. | `C-M-x` on a function that begins with `use crate::shapes::area;` defines it; an error's report points at the user's text, not at a line the rewrite invented |
| **R4** | **Say what it cannot do.** A binary-only crate, a private item, an edition evcxr does not have: each has a recognisable error, and each gets one plain line under it saying why — "private: sent code runs beside the crate, not in it". `M-x rust-repl-project` shows what evcxr was told. | each case produces its line, once |

R1–R3 are the feature; R0 comes first because two of the decisions depend
on what it finds. Two to three hours for R1–R3 once R0 is in.

## Risks

- **The first send in a project is slow.** evcxr compiles the project and
  every dependency before it answers anything: seconds for a small crate,
  minutes for one with a heavy tree. It happens once per evcxr, and it must
  be visible — a transcript that sits silent for a minute reads as a hang.
- **A dependency that fails to build fails every send.** One bad `:dep` (a
  missing system library, a yanked version) and nothing compiles. The
  editor has to show cargo's error under the `:dep` that caused it and let
  the rest stand.
- **Stale project code.** If evcxr does not rebuild a path dependency whose
  source changed (R0), a send answers from the old code with no sign of
  it. Then a save of a project file must make the next send re-`:dep`.
- **Rewriting what the user sent.** R3 changes text. A wrong rewrite is
  worse than none; it stays to `crate`/`self`/`super` at the head of a
  path, and is off for a file outside `src/`.
- **Features and `cfg`.** The project's own features are not on unless the
  `:dep` says so; code behind `#[cfg(feature = "x")]` is simply absent, and
  the error will not say why.
- **The network.** `:dep` of a published crate downloads. Offline, from a
  cold cache, it fails; the message should say that and name `:offline`.

## Decisions for Kevin

- **D1 — automatic, or asked for?** Telling evcxr about the project on the
  first send is what makes sends just work; it also means the first send
  compiles the dependency tree unasked. *Recommend automatic, with the
  `:dep` lines shown as they go*, and `*rust-repl-project*` set to `#f` in
  `init.scm` to keep evcxr bare.
- **D2 — dev-dependencies too?** Sent code is often test-shaped and wants
  what the tests use. *Recommend yes*: `[dev-dependencies]` as well, since
  the cost is compile time for crates the project already builds.
- **D3 — which features of the project's own crate?** Its defaults, all of
  them, or a list. *Recommend the defaults*, with a variable to name
  others; R0 says whether evcxr's `:dep` takes `features`.
- **D4 — a workspace.** The member the file is in, or every member.
  *Recommend the member the file is in, and its path dependencies* (which
  are usually the sibling members it uses).
- **D5 — rewriting `crate::` (R3).** It is the part that edits what was
  sent. *Recommend doing it*, with the transcript showing the text as
  sent, because without it nearly every real file's first line fails.
