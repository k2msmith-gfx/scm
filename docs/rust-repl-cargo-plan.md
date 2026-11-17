# Rust sends from inside a Cargo project — plan, 2026-11-17

**PARKED 2026-11-17, after R0** (Kevin: *"somehow I think this is going to
get very complicated and I don't know how much I will use it"*). R0, the
measurements, is done and below; R1–R4 and the decisions are not started.
By hand it already works: in a Rust buffer's REPL panel,
`:dep my-crate = { path = "/abs/project" }` makes the project's `pub`
items usable, and cargo builds its tree, resolves its `workspace = true`
entries and rebuilds it when a source file changes. Take this up again
only if that gets typed often — and then the one-command form first
(`M-x rust-repl-project`: the package name and that one `:dep`, about an
hour), not the whole plan.

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

The rest is measured below, under "R0 as measured".

## R0 as measured (2026-11-17)

evcxr 0.21.1 on an aarch64 Mac with rustc 1.93 — the newest evcxr wants
rustc 1.95 and would not install, which is `rust-repl-missing`'s case; an
older one by `--version` did. Driven as `src/pipe.rs` drives it
(`--ide-mode --disable-readline`, U+2028 newlines, the U+0091/U+0092 end
marks), each send timed, against scratch projects: a hyphenated library
crate of edition 2024 with features, a workspace whose member inherits by
`workspace = true`, a binary-only crate, and two real ones — scm's
`schemers` (no dependencies) and `edit` (a heavy tree).

| question | measured |
|---|---|
| a path dependency edited between sends | **rebuilt by itself, on the next send**: `stamp()` answered `"v1"`, `"v2"`, `"v3"` as the file changed, with no new `:dep`. A send costs the same 0.37 s for the small crate; for `schemers` 0.58 s against 0.45 s. Cargo's own fingerprinting does it. |
| the first send's cost | the `:dep` compiles the crate: 2.5 s for the scratch crate, **3.3 s for `schemers`, 30 s for `edit`** (cosmic-text, winit and the rest), cold. |
| a second evcxr (a restart) | compiles everything again: 3.3 s for `schemers`. With `:cache 2000` (MiB; per process, not remembered) `edit`'s second start took 15 s against 30 s — half, not all. The cache is `~/Library/Caches/evcxr` (410 MB after these runs). |
| a published crate, online | `:dep nanorand = "0.7"`, not in the cargo cache: downloaded and compiled, 2.8 s. |
| offline (`:offline 1`), cold | fails at once, cleanly: `no matching package named nanorand found`, and cargo's reminder that offline mode was on. |
| offline, warm | works: `:dep itoa = "1"` compiled from the cache, 0.8 s. |
| **which version** | **evcxr resolves fresh and ignores the project's `Cargo.lock`**: `itoa = "1"` came out 1.0.18 offline and 1.0.19 online, while the projects here lock 1.0.15/1.0.17. A project's code runs against the newest compatible versions, not the ones it was built and tested with. |
| a failed `:dep` | **not kept**: `:show_deps` does not list it and later sends compile. One bad dependency does not poison the rest. |
| a workspace member | `:dep ws-app = { path = "…/ws/app" }` works; **cargo resolves the member's own `workspace = true` entries itself** (its `probe-crate` inherited from the root with `features = ["extra"]` on top). |
| …its dependencies | not visible to sends: `ws_core::…` fails until `ws-core` gets its own `:dep`. Only direct dependencies can be named, as in Rust. |
| `{ workspace = true }` copied into a `:dep` | **fails**: `failed to find a workspace root` — evcxr's scratch crate is in no workspace. The editor must expand it from the root's `[workspace.dependencies]`, its `path` relative to the ROOT, its `features` added to the root's. |
| the workspace root | a virtual manifest cannot be a dependency (`found a virtual manifest`). |
| `default-features = false` | honoured: `loud()`, behind a default feature, is gone. |
| `features = [...]` | honoured. |
| **two `:dep`s of one crate** | **the second replaces the first** (`:show_deps` lists one). Features do not accumulate: a crate's one `:dep` must carry everything. |
| `package = "…"` | honoured: `:dep shapes = { package = "probe-crate", … }` is used as `shapes::`. |
| `optional = true` | accepted, then **unusable** (`unresolved … crate probe_crate`): nothing turns the optional dependency on. The key must be dropped. |
| a hyphen in the name | the `:dep` key must be the package name, hyphen and all (`probe_crate = …` fails, "perhaps you meant probe-crate"); `use` writes `probe_crate`. |
| the edition | **evcxr compiles sends as edition 2024**, and has no command to change it: a let chain works and `gen` is a reserved word. A dependency keeps its own edition (an edition 2024 crate and a 2021 one both load). |
| a binary-only crate | refused, plainly: `Dependency binonly is missing a lib target`. |
| a private item | `[E0603] function private_helper is private`, with the note pointing at it; a public function that calls it works. |
| `:clear` | **drops the `:dep`s too** (`:show_deps` empty, `use probe_crate` fails after it), though its help says it keeps the cache. |
| output | a `:dep` answers with cargo's `Compiling …` lines, and a send after an edit has them before its value; the value is still the last line. |
| history | evcxr writes every line it is sent to `~/Library/Application Support/evcxr/history.txt`, `--disable-readline` or not. |

What this changes in the stages below:

- **R2 does not re-`:dep` on a save.** The "stale project code" risk is
  gone: cargo rebuilds a path dependency whose source changed, at the next
  send. R2 re-tells evcxr only when a MANIFEST changes, and after
  `rust-repl-restart` or a `:clear`.
- **R2 sends one `:dep` per crate**, with every feature in it, since a
  second replaces the first; drops `optional`; keeps `package`; makes a
  relative `path` absolute against the manifest it came from; and expands
  `workspace = true` from the root's table.
- **R2 turns on `:cache`** before the first `:dep`: half the first compile
  of a heavy project, every restart after the first.
- **R1 also reads `Cargo.lock`**, for D6 below.
- **R3 has nothing to do about editions** in evcxr's own crate: it is 2024
  whatever the project's is. A 2021 file that uses `gen` as a name fails to
  send; R4 can say why.

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
| **R0** ✔ | **Measure what the table above leaves out**, on a scratch project: a path dependency edited between sends (is it rebuilt, and how long does a send then take?); `:dep` of a published crate, online and with `:offline 1` from a warm cache; a workspace member (what path, and do `workspace = true` dependencies need the root's table?); `features = […]`, `default-features = false`, `optional`, `package = "…"` renames; a crate named with `-` (`use` writes `_`); two `:dep`s of the same crate; the edition. | a table here under "R0 as measured"; D1–D4 answered with evidence |
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
- **Stale project code** — measured away (R0): cargo rebuilds an edited
  path dependency at the next send.
- **Not the project's versions.** evcxr ignores `Cargo.lock` and takes the
  newest compatible version of each dependency, so a send can run against a
  version the project never built with, and a crate that builds under
  `cargo` can fail to under evcxr. D6.
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
  compiles the dependency tree unasked. *R0: 3 s for a crate with no
  dependencies (`schemers`), 30 s cold and 15 s cached for a heavy tree
  (`edit`).* *Recommend automatic, with cargo's `Compiling …` lines shown
  dim as they go* — at 30 s a silent transcript would read as a hang — and
  `*rust-repl-project*` set to `#f` in `init.scm` to keep evcxr bare.
- **D2 — dev-dependencies too?** Sent code is often test-shaped and wants
  what the tests use. *Recommend yes*: `[dev-dependencies]` as well, since
  the cost is compile time for crates the project already builds. R0 found
  nothing for or against: they are `:dep`s like the rest.
- **D3 — which features of the project's own crate?** Its defaults, all of
  them, or a list. *R0: `:dep` honours `features` and `default-features`,
  and a crate's second `:dep` replaces its first, so the choice is made
  once, in one line.* *Recommend the defaults*, with
  `*rust-repl-features*` to name others (a change re-sends that crate's
  one `:dep`).
- **D4 — a workspace.** The member the file is in, or every member. *R0: a
  member works as a path dependency and cargo resolves its own
  `workspace = true` entries; its dependencies are not nameable until each
  has its own `:dep`, and the editor must expand a `workspace = true` there
  from the root's table; the root itself cannot be one.* *Recommend the
  member the file is in, and its direct dependencies* — the sibling
  members it uses among them — which is exactly what that member's code
  can name.
- **D5 — rewriting `crate::` (R3).** It is the part that edits what was
  sent. *Recommend doing it*, with the transcript showing the text as
  sent, because without it nearly every real file's first line fails.
- **D6 — the project's versions (new, from R0).** evcxr ignores
  `Cargo.lock`. Either each registry dependency's `:dep` is pinned to the
  locked version (`itoa = "=1.0.15"`), so a send runs what `cargo run`
  would, or the manifest's spec goes as written and evcxr takes the newest.
  *Recommend pinning*: the point is to run the project's code as it is
  built. It needs R1 to read `Cargo.lock` (one more small reader) and costs
  a download when the locked version is not in the cargo cache — offline,
  that `:dep` fails with cargo's message, and the rest stand.
