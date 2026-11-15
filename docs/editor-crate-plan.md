# The editor as a crate — plan, 2026-11-13

**Moved 2026-11-14 from ray (`docs/editor-crate-plan.md`) with the crate
itself: `edit/` is a member of THIS workspace now (D2, below, decided).**
The text below was written in ray, so a bare `edit/…` path means this
repo's `edit/`, and `modeler/…`, `src/…`, `scripts/…` and `assets/…` are
ray's. ray's modeler takes `edit` from this repo by git, pinned in its
Cargo.lock as it pins scm; ray's `.cargo/config.toml` redirect points it
at a sibling checkout for work on both at once.

Kevin, 2026-11-13: *"I really like our editor and I would like to use it as a
standalone program with scm (and potentially other languages). If I make
improvements to the editor I would like both ray modeler and the standalone
version of the editor to benefit."* And: *"the standalone editor also needs the
ability to pop up a terminal window so the repl can run as well — that should
be included in the crate."*

So: one crate, two hosts. The modeler keeps its editor and links it; a new
standalone binary links the same crate. Nothing here removes the editor from
the modeler, which is what
[modeler-editor-split-research.md](modeler-editor-split-research.md) argued
against (2026-09-12). That research asked whether the editor should LEAVE; this
plan shares it.

Working name: **`edit`** (decision D1). Measured at `bbf23195`.

**Status, 2026-11-13: recommendations ACCEPTED (Kevin), build DEFERRED.**
D2–D5 go as recommended below; D1 keeps `edit` as the working name. E1 waits
until the Fallingwater work running on the other machine has landed — E1–E3
move files the whole modeler touches, and a big move under someone else's
open branch is a merge nobody wants.

**Status, 2026-11-14: E1–E6 and D5 built and pushed**; what Kevin asked for
in daily use since, and what is still open, is at the end of the stages
("The standalone in use", "The modeler, kept level with it", "Still open").

**Status, 2026-11-13 (later): E1 STARTED** — the Fallingwater work landed at
`5fc06189`. E1 goes in three commits: **E1a** the move and the `Host`,
**E1b** the `Language` trait over the lexer as it is, **E1c** that lexer
replaced by `scm-text`'s scan, checked against the old one across the
corpus. E1a is done (below, "E1a as built").

## What "the editor" is

The text widget is the small part. What makes the editor worth keeping is
the Emacs-shaped SHELL around it, and a standalone editor without that shell
would be a worse cosmic-text demo:

| layer | where it is now | size |
| --- | --- | --- |
| the widget: buffer, cursor, vi, undo, highlighting, markdown view | `modeler/src/editorpane.rs` | 3,330 |
| transcripts, input line, popup overlay (menus, completion) | `modeler/src/textpane.rs` | 1,417 |
| glyphs, metrics, blending | `modeler/src/text.rs` | 673 |
| the CPU framebuffer it all draws into | `modeler/src/framebuffer.rs` | — |
| the `%editor-*` prims | `modeler/src/script/editor.rs` (73), `text.rs`, `xref.rs` | — |
| keymaps, prefix keys, handler chains | `00-decls.scm`, `06-keymap.scm` | — |
| minibuffer, REPL instance, completion, editor keys | `08-editor.scm` | 1,366 |
| popup menus, the marking menu | `09-menus.scm` | — |
| buffers, save/undo, SLIME sends, completion, eldoc, `M-.`, `C-x b` | `unify.scm` 5008–7116 | ~2,100 |

The Rust widget is already nearly free. `editorpane.rs` reaches the modeler
through four `panes::` calls (`mark_dirty_ui`, `rect`, `ui_scale`, `typing`),
`theme::get`, and the font helpers in `text.rs` — that is the whole surface.
The Scheme half is the work: its editor sections are interleaved with curve
tools and scene loading in `unify.scm`, and it leans on shell vocabulary
(`bind-key`, `add-handler`, `read-minibuffer`, `echo`, `show-menu-items`)
defined in four other files.

## The shape

```
edit/                       the crate (a workspace member)
  src/widget/               editorpane, textpane, text, framebuffer
  src/language.rs           trait Language: lex, indent, pair matching
  src/language/scheme.rs    the one there is today
  src/term/                 the terminal pane (E5)
  src/backend.rs            where a send goes: in-process VM, or a terminal
  src/host.rs               trait Host: dirty, rect, ui scale, theme colour
  src/scm_binding.rs        the %editor-* / %term-* prims — the ONE file that names the VM
  scm/                      the shell: keymaps, handlers, minibuffer, menus, buffers, editor
  src/bin/edit.rs           the standalone program (E4)
modeler/                    links edit; loads edit/scm/ before its own prelude
```

**Scheme is the extension language; the edited languages are pluggable.**
That is Emacs's split (Elisp extends it, it edits everything), and it keeps
the shell written once. A language brings a `Language` (how its text lexes and
indents) and a `Backend` (where a send goes). The modeler's layering rule
holds inside the crate: only `scm_binding.rs` names the VM, so a second
extension language would be a second binding over the same widget, not a
rewrite.

**No install beyond the binary** (Kevin, 2026-11-13: *"One thing I want to
avoid is any complicated install with an lsp."*). Everything Scheme gets —
the scan, definitions, diagnostics, the error pane — is LINKED, so nothing
runs beside `edit` and nothing is configured. No feature requires a language
server; another language needs only its own REPL on the `PATH`, which anyone
using the language already has. `edit` never installs or bundles a server.

## Where scm's editor tools fit

`~/devel/scm` has three, built for zed-scm, which Kevin does not use. They
are plain stdio servers, so they stay for any editor that wants them (Emacs's
eglot and dape included); `edit` is built on their LOGIC, not their
protocols. A language server exists to reach a session in another process,
and `edit` is inside the session — the reason `scm-text` was split out of
`scm-lsp` on 2026-11-02 for the modeler.

| | what it is | in `edit` |
| --- | --- | --- |
| `scm-text` (1,147 lines) | token scan, symbol at point, call around point, the cross-file definition index | already linked (`M-.`, `modeler/src/script/xref.rs`); becomes the Scheme `Language`'s engine (E1) |
| `scm-lsp` (687) | completion, hover, signature help, definition, document/workspace symbols, diagnostics from scm's own reader | its diagnostics and document symbols, split out as library calls the way `scm-text` was (E3f); `scm-lsp` stays a thin protocol layer over them |
| `scm-dap` (504) | the break REPL over DAP: stops on an UNCAUGHT error with frames live — locals, evaluate in a frame, abort or continue with a value; no breakpoints or stepping (the VM has no hook) | the same abilities in-process, through `set_break_handler`, as an error pane (E3f) — not over DAP |

**One Scheme scanner, not three.** There are three today: `scm-text`'s, the
lexer in `editorpane.rs` (highlighting, indent), and the SLIME form scanner
in `unify.scm` (`editor-scan-spans`, ~90 lines of Scheme). A fix to how
Scheme is read should reach the modeler, `edit` and `scm-lsp` at once.

## Stages

Each stage leaves both legs green and the modeler unchanged to a user.

**E1 — the widget crate.** Move `editorpane.rs`, `textpane.rs`, `text.rs`,
`framebuffer.rs` into `edit`. Replace the `panes::`/`theme::` calls with a
`Host` the modeler implements. Pull the lexer, `target_indent` and paren
matching behind `Language`, with Scheme the one implementation — built on
`scm-text`'s scan rather than `editorpane.rs`'s own lexer. Their unit
tests (`lexer_classifies_scheme_tokens`, `indent_follows_the_innermost_open_paren`,
…) move with them. Check: `cargo test`, both legs; the UI harness, because it
is the only thing that drives the editor with real keys.

**E1a as built.** `edit/` is a workspace member and a default one, so a
bare `cargo test` runs it. The four files are in `edit/src/widget/`, and the
modeler re-exports them at its root (`pub(crate) use edit::{editorpane, …}`),
so every `crate::text::…` in the modeler reads as before. What they asked of
the modeler is `edit::host::Host` — two dirty flags, a pane's rect, the UI
scale, the typing pane, the window size, the strip text size (the minibuffer
draws at the status label's) and the colours — answered by `panes.rs`'s
`EditHost`, which is installed whenever the registry takes a pane, so the
modeler's tests have it without running `main`. The host is stateless (it
answers from `panes`' own thread-locals), so one is installed per PROCESS.
The colours are `edit::theme::Theme`, the eighteen slots the widget reads,
filled from the modeler's table on every draw; a test holds the two
defaults equal. `raster::draw_line_2d` moved into `framebuffer.rs`, beside
`glass_rect`. Four test-only accessors in `textpane.rs` lost their
`#[cfg(test)]`: a dependency is built without it when the modeler's tests run.

**E1b as built.** `edit::language`: the highlight classes (`Tok`), a
buffer's `Scan` (each line's spans and every code-context bracket), and the
`Language` trait — `scan`, `indent`, `enclosing_head`. Bracket pairing
(`find_partner`) stays generic beside it, since it only walks the scan's
brackets. `language/scheme.rs` is the lexer exactly as it was, with its
prose and tests; each `Doc` carries a `lang` (Scheme for every buffer until
E6), and the widget calls it for highlighting, show-paren, Enter/TAB/vi
indent and eldoc's head.

**E1c as built.** The Scheme `Language` is `scm-text`'s token scan; the
editor's own lexer is gone. That took one change in scm, `Doc::tokens()`
made public (scm `dccb184`), and so ray's scm pin moved `f70d5fd` →
`dccb184` — which carries scm S30–S34 (superinstructions, the generational
collector) and `116513e` with it, because `schemers`, `scm-llvm` and
`scm-text` are one git source and Cargo pins them together (Kevin,
2026-11-13). The editor adds only the highlight CLASS to the tokens
(keyword, literal, a form's head) and walks `scm-text`'s char offsets back
to the (line, byte) the widget counts. **Checked against the old lexer
before it went:** every line of the tree's 388 Scheme files — 209,799
lines, 1,066,321 spans — gave the same spans and the same brackets.
**Cost:** a full scan of `unify.scm` (8,588 lines) is 1.84 ms against the
old lexer's 0.73 ms, per draw; `scm-text`'s own `Doc::new` is 0.9 ms of it.
A 55-line scene is 0.02 ms either way. **And a draw no longer scans
unless the text changed** (Kevin, 2026-11-13): the editor pane redraws on
every repaint of the window, a camera drag's frames included, and until
E1c every one of them re-lexed the whole buffer. `relex` keeps the last
scan beside a copy of the lines it was made from (`LexCache`), re-scans
only when a line differs, and skips the per-line pass too when the
show-paren pair, theme and font family are what it last applied —
`a_draw_scans_only_when_the_text_changed` holds it, and fails with the
cache disabled. Of the three scanners, two are now
one; the third, the SLIME form scanner in `unify.scm`, goes in E3e.

**E2 — the prims.** `script/editor.rs`, the editor half of `script/text.rs`,
and `script/xref.rs` (with its `scm-text` dependency) move into
`edit::scm_binding`, installed by the modeler's session as it is today.
`modeler/src/layering.rs` learns that the binding file now lives in `edit`.

**E2 as built.** `edit::scm_binding` (`scm_binding.rs` and
`scm_binding/{editor,text,xref}.rs`, moved with `git mv`): 61 prims (42,
15, 4) — the
editor's, the text panes' and the font knobs, and the cross-file index —
with their bodies and docstrings unchanged, declared through a `prim!` cut
down from the modeler's to the five argument kinds and six answers they
use. `install(interp, installed)` defines them and names each one, which
the modeler passes to its own `note_installed`, so 99-scheme-only wraps
them as before. Two things did not come: ray's number rule, copied (a
whole number answers exact) rather than linking the raytracer for one
comparison; and the modeler's PARKED host — its prims park the
interpreter because some call back into the prelude, and these never do.
`%xref-prelude-dirs` stayed in the modeler (`script/xref_dirs.rs`): where
the modeler's preludes live is the modeler's to say. `edit/src/layering.rs`
now allows the binding and nothing else; the modeler's own rule needed no
change, since it only scans `modeler/src`.
`the_prims_answer_through_a_bare_interpreter` installs the binding into a
bare `scm::Interp` — no modeler, no prelude, what E4 will boot — and calls
the editor, a text pane and the index through Scheme.

**E3 — the shell, as shared Scheme, in five slices.** The riskiest stage: a
move of prose-heavy prelude code, the thing `scripts/dev/check_moved_prose.py`
exists for. Each slice lands into `edit/scm/` and passes the gate before the
next starts, in the order each needs the last. What stays in the modeler
connects through HOOKS the shared files call; the `*outliner-refresh*`/`*echo*`
forward hooks in `00-decls` are the pattern. Every slice moves verbs between
files, so `generated_modeler_manual_is_up_to_date` turns red each time:
regenerate `api/modeler` and read the diff.

- **E3a — the core.** The handler chains and `run-command` (`00-decls`:
  `add-handler`, `*key-handlers*`, `*click-handlers*`, …) and the keymaps
  (`06-keymap`: `bind-key`, prefix maps, pane-local maps). Nothing else
  moves until these have.
- **E3a as built.** `edit/scm/` holds three files, which
  `edit::scm_binding::SHELL` names in load order: `00-base` (`%env-ref`,
  `%key-string`, out of `00-helpers`, and the SRFI-125 import the keymaps
  need as they load), `01-handlers` (`run-command`, the chains and their
  lists, the kernel's `on-*` hooks and their post halves, the `*echo*`
  hook — out of `00-decls`) and `02-keymap` (all of `06-keymap` but the
  modeler's default bindings and render verbs, and the `*command-echo*`
  hook). The keymap named the modeler's `*panes*`; it asks two new forward
  hooks, `*pane-id*` and `*pane-key*`, which `05-panes` binds and
  `*hook-placeholders*` lists. The modeler loads the shell AFTER its
  vocabulary baseline — so `api-catalog` still lists `bind-key` and the rest
  as the modeler's — and before `00-data`; `%xref-prelude-dirs` includes
  `edit/scm`, so `M-.` still finds them. Docstrings moved byte for byte:
  the regenerated manual is unchanged (607 entries), and
  `check_moved_prose.py` is clean. What the shell takes from the image
  beneath it — `list-sort`, `string-split`, `string-prefix?`, `filter`,
  `append-map`, `%focused-pane`, `%typing-pane` — is listed in `00-base` and
  held by `the_shell_runs_keys_on_a_bare_interpreter`, which boots the shell
  with only those and runs a prefix sequence, the typing-pane rule and an
  error to the echo through `on-key`. E4 must supply that list.
- **E3b — menus.** Its own slice because the modeler uses menus everywhere
  (`define-menu` or `show-menu-items` in six prelude files), so they must be
  shared before anything that opens one. `09-menus.scm` (356 lines) is
  nearly generic already: menus as data, the linear popup, the marking-menu
  gesture, their handler-chain entries, and the `%show-menu` / `%overlay-*` /
  `%show-pie` prims. Two changes: **`on-menu-press` stops hardcoding** which
  menu each pane opens (a 3D pick → `:object`, the editor, the REPL, an
  outliner row) and asks a registry the host fills —
  `(set-pane-menu! ROLE (lambda (x y) MENU-NAME-OR-#f))`, the 3D and
  outliner entries staying in the modeler; and **`toolbox-menu` stays** in
  the modeler.
- **E3b as built.** `edit/scm/03-menus.scm`: menus as data, the linear
  popup, the marking-menu gesture and their three handler entries, out of
  `09-menus`; the seven prims they draw through (`%show-menu`,
  `%overlay-hit`, `%overlay-select`, `%show-pie`, `%pie-mark`,
  `%clear-completions`, `%set-menu-alpha`) moved to
  `edit/src/scm_binding/menu.rs` — each is one call into `textpane`.
  `%set-pie-delay` stayed: the hold timer is the host's event loop.
  `on-menu-press` no longer names a pane: it asks two hooks, `*pane-role*`
  and `*menu-allowed?*`, and a registry `set-pane-menu!` fills by role; the
  modeler's `09-menus` registers its five (the 3D pick-and-select for
  `:viewport` and `:preview`, the editor, the REPL, the agent, the
  outliner) with the comments that explained each, and keeps
  `toolbox-menu`. The menus' small kit (`%text`, `%make-state`,
  `%state-ref`, `%list-ref/default`, `%pi`) moved to `00-base`; `%as-list`
  is in both — the shell's handles vectors, and the modeler's, which
  replaces it as `00-helpers` loads, unwraps its Janet data's arrays too.
  The manual changed in one docstring, `toolbox-menu`'s, which had said the
  popup was in its own file. The bare-interpreter test now opens a menu by
  keys and by a right-press through the registry.
- **E3c — the minibuffer.** `read-minibuffer`, its completion sources, `echo`,
  the input editor (`08-editor`). The REPL and agent instances stay.
- **E3c as built**, in two commits. E3c-1: the clipboard prims (with
  `arboard`) and `%set-completions` moved into `edit`, and the kit echo and
  M-x are written in (`%as-keyword`, `%alist-ref`, `%number-text`,
  `%describe-part`, `eval-string`) into `00-base`. E3c-2:
  `edit/scm/04-minibuffer.scm` — the shared line editor's state, its
  completion (symbols, commands, segment-wise files), the editor commands,
  `echo`, `read-minibuffer` and `collapse-path`, `M-x`, the `:line-editor`
  text handler, `on-typing-released`, the editor keys, and the text-size
  and font verbs, with `*focus-held*` / `*prompt-open*` out of `05-panes`.
  Where the code named the modeler it now asks: `mini-id` is
  `(*pane-id* :minibuffer)`; a closed prompt calls `*prompt-closed*` (the
  modeler's `%refocus-3d`); M-x shows a value through `*show-value*` (the
  modeler's `readable`); symbol completion adds `*keyword-candidates*` (the
  modeler's docstring harvest) to scm's `(apropos)`. The REPL's clause in
  `edit-quit` became an `:on-quit` the REPL passes to `edit-start`
  (`repl-quit`), and the per-pane key loop became `(bind-editor-keys
  PANE-KEY PASTE)`, which the modeler calls for its three surfaces once
  `05-panes` has made them. `*echo*` came off `*hook-placeholders*`: the
  shell binds it before `00-decls` records the placeholders, so the boot
  audit read the bound value as never bound — the audit caught that. The
  REPL and agent instances, pane roles, command echo, the click floor and
  `C-x C-f` stayed. The bare-interpreter test runs a prompt to its
  callback.
- **E3d — buffers and editing.** `unify.scm` 5008–5551 and 7116–7123: buffers,
  save, undo, `C-x b` / `C-x k` / `C-x C-b`. `*scene-file*` and "an emptied
  scene is no longer a file" (5103) stay, behind a hook.
- **E3d as built.** `edit/scm/05-buffers.scm`: the buffer registry
  (naming, uniquify, the lookups, `visit-buffer`, `real-file` /
  `same-file?`, `buffer-for-file`), the editor pane's typing, mouse, wheel
  and keys (now `(bind-editor-pane-keys)`, called by the modeler once the
  pane exists), paste/copy, quitting a view-only buffer, `save-buffer` and
  vi's `:w`, `set-editor-vi`, `C-x C-s`, and the buffer verbs
  `switch-to-buffer` / `kill-buffer` / `list-buffers` on `C-x b` / `C-x k` /
  `C-x C-b`. What a buffer means to the SCENE stayed in `unify.scm` behind
  eight hooks it binds: `*buffers-changed*` (scene tags), `*show-status*`,
  `*show-editor*` / `*hide-editor*` (the left dock), `*writable-path?*`
  (only scene files save), `*after-save*` (the scene root follows a
  save-as; a scene file saved marks the scene changed or reruns it),
  `*buffer-killed*` (the root's buffer takes the scene with it) and
  `*save-as-initial*`. `find-file` / `load-file` stayed — they choose by file
  type (`.scm` scene, `.gltf` model, `.janet`) — and so did the scene loads,
  `clear-scene` and the dock. Docstrings moved byte for byte (several still
  speak of the scene, which is true in the modeler); the manual is
  unchanged. The bare-interpreter test now names, saves, switches and
  kills buffers.
- **E3h — help** (Kevin, 2026-11-13: *"I want to also preserve the help
  screens for keymaps etc. in the editor"*). After E3d, because the two
  markdown pages open in a view-only editor buffer. Into the shell:
  `show-keys` (`?`, `f1` — today in `13-outliner`, with `keys-into`,
  `fn-label`, `%key-column`), the key browser `view-keybindings` (`C-h b`)
  and `describe-key` (`C-h k`) with their handler, `key-for-command` and
  the menu key hints, `key-label`, `key-base`, `doc-summary`, and
  `show-markdown-doc` (all `unify.scm` today). Two hooks for what is the
  host's: extra `show-keys` sections (the modeler's mouse gestures) and the
  page's title and its name for bare-key bindings ("Modeler key bindings",
  "Viewport"). `(help NAME)` / `(apropos …)` / `(doc …)` rest on scm's own
  docstrings and work in either host. `describe-panes` and `C-h r` (render
  settings) stay: they describe the modeler's panes and renderer.
- **E3h as built.** `edit/scm/06-help.scm`: `show-keys` (`?`, `f1`) with
  `fn-label`, `keys-into` and `%key-column` (from `13-outliner`), and from
  `unify.scm` `key-for-command` and the menu key hints, `key-label`,
  `key-base`, `viewport-key?`, `%doc-of`, `doc-summary`, `describe-key`
  (`C-h k`) with its page and key handler, `keybindings-markdown`,
  `show-markdown-doc` and `view-keybindings` (`C-h b`). Five hooks the
  modeler binds: `*help-open*` / `*help-append*` (show-keys prints in the
  REPL, opening the bottom dock), `*show-keys-extra*` (its mouse gestures),
  `*keybindings-words*` (the page's title and what it says of chords and
  bare keys — "Modeler key bindings", "Viewport") and `*editor-shown?*`.
  **The modeler's C-h b page is byte-identical**: the original
  `keybindings-markdown`, loaded beside the new one in a live modeler,
  produced the same 16,243 characters. `00-base` imports `match` and
  `and-let*` (SRFI 204, 2) itself rather than counting on ray's prelude, and
  its comment now lists everything a standalone host must supply — the
  bare-interpreter test found `match`, `string-pad-right`, `string-contains`
  missing from the earlier list, one test run each, before a scan of ray's
  prelude finished the list (seven procedures). That test now runs
  `show-keys`, `C-h k` on a sequence (described, not run) and `C-h b`.
- **E3e — SLIME, over an interface.** Below.

### The interface between the editor and Scheme

The SLIME layer (`unify.scm` 5551–6438) is mostly generic already, but the
modeler is inside the same functions:

| generic | the modeler's, inside them |
| --- | --- |
| the form scanner: `editor-scan-spans`, `editor-form-before`, `editor-topform-at` | after every send: `%sync-retained`, `*outliner-refresh*`, `%poke-settle` |
| `C-x C-e`, `C-M-x`, `C-c C-r`, `C-c C-b`, `C-c C-l` and what text each reads | **form ownership**: `eval-owning`, `send-marking-forms`, `%editor-own` — running a form again replaces what it built |
| output capture, errors to the transcript and the echo area, `=> value`, `defined-name` | `adopt-scene-file!`, and `C-c M-b` clearing the scene |
| completion, eldoc, `C-c C-d d`, `M-.` / `M-,` | `editor-scheme-buffer?` — sends only in a `.scm` buffer |

Everything in the right column happens AROUND an evaluation, never inside
one. So the interface is two small things:

**A backend — what the editor asks a language.** A record of procedures:

- `(eval src)` → the value, or an error, and the output captured either way;
- `(complete prefix)` → candidates;
- `(signature name)`, `(doc name)`;
- `(definition-sites name)` → `((path line column) …)`.

The in-process scm session answers all five. A terminal backend (E5) answers
`eval` only — it writes the form and a newline to the PTY and returns no
value — and completion falls back to the buffer's own words. A missing
procedure means "not offered", and the command that needs it says so.

**Host hooks — what the host does around a send.**

- `around-send (lambda (run start end) …)` — the modeler wraps `run` in a sync
  window and gives what it made to the form at `start` (ownership); the
  standalone calls `run`.
- `after-send` — the modeler syncs the scene and refreshes the outliner; the
  standalone does nothing.
- `buffer-backend buf` — which backend a buffer sends to, or `#f` for none
  (replaces `editor-scheme-buffer?`).
- `on-run-buffer buf` — the modeler adopts the scene file; the standalone
  does nothing.

With those four in place the SLIME code does not mention the scene. E3e
installs the interface first, with the modeler's hooks doing exactly what the
code does now, and only then moves the sends, completion, eldoc and `M-.`
into `edit/scm/`.

**E3e as built, so far.** E3e-1 installs the interface with the code still
in `unify.scm`: `edit/scm/07-backend.scm` defines a backend as an alist of
procedures (`:eval` answering a RESULT — status, value, printed, output,
seconds, or an error message — and `:complete`, `:doc`; `:signature` and
`:definition-sites` arrive with the code that asks them), the in-process
backend that is this image, and three hooks: `*buffer-backend*` (the
modeler: this image for a Scheme buffer, else none — the test
`editor-scheme-buffer?` made), `*after-send*` (sync the retained scene,
refresh the outliner, nudge live render) and `*transcript-append*` (the
REPL; show-keys prints through it too, so E3h's `*help-append*` folded
into it). `editor-send-local` now asks the buffer's backend and prints the
result in the order it always did. Ownership (`eval-owning`,
`send-marking-forms`) and `adopt-scene-file!` are still the modeler's
code; they become `*around-send*` / `*on-run-buffer*` as the sends move
(E3e-2).

E3e-2 moves the sends: `edit/scm/08-sends.scm` takes the form scanner
(`editor-scan-spans`, `editor-form-before`, `editor-topform-at`),
`editor-point`, `editor-send-local` / `editor-send` / `defined-name`,
`eval-owning`, `eval-last-sexp`, `eval-defun`, `eval-region` and their keys
(`C-M-x` joins `bind-editor-pane-keys`). Owning is a hook, `*around-send*`
(run start end): the modeler's takes what the text made last time, opens
ray's scoped sync window, and gives the text what it made — the old
`eval-owning` body, unchanged. The shell has plain `eval-buffer` (C-c C-b)
and `eval-file` (C-c C-l) for a standalone host; the modeler's
`unify.scm` defines both again, running them INTO THE SCENE, and replaces
them — so `*on-run-buffer*` was not needed: running a buffer is the
modeler's whole command, not a step in the shell's. `send-marking-forms`,
`C-c M-b`, `C-c C-k`, `copy-region-to-repl` and the editor's marking menu
stayed. `describe-val`, dead since E3e-1, went. The bare-interpreter test
now sends a form with `C-M-x` (and found `string-trim-right` missing from
00-base's list).

E3e-3 moves completion, eldoc and describe: `edit/scm/09-complete.scm`
(the editor's completion popup and its keys — now
`(bind-editor-completion-keys)`, called after `bind-editor-pane-keys` —
`signature-of`, eldoc, `editor-describe`, `editor-back`, and C-c C-d d).
The in-process backend answers `:signature`. **A bug the interface fixed:**
in a Scheme buffer — every scene buffer — describe (C-c C-d d, vi's K)
asked `%scm-describe`, the door to a second VM that J5c retired and
`00-decls` stubs to answer nothing, so it said "no doc for 2" on
`(add-sphere :at [1 2|`; the harness only checked that it survived. It
asks the buffer's backend now, through `(describe-name NAME)`, which M-.
also uses for a bound name with no source (its "its doc is in the REPL"
had printed nothing for the same reason). `editor-help-at-point` (it
renders `view-help`, the modeler's manual) and the editor marking menu
stayed.

E3e-4 moves M-.: `edit/scm/10-xref.scm` takes `definition-sites`,
`find-definitions`, `goto-site`, `jump-to-definition` and the M-. / M-, /
C-M-. commands (M-. and M-, join `bind-editor-pane-keys`); the in-process
backend answers `:definition-sites`, so it offers all five. Which files the
index holds stays the modeler's (`xref-reindex`: the scene's files and the
preludes), and a jump opens its file through `*visit-file*` — the shell's
`visit-file-text`, or the modeler's `visit-scene-text`, which also says
what the file is to the scene. The messages still speak of "this scene's
files", true in the modeler; a standalone host's wording is an E4 question.
**E3e is done**: every send, completion, eldoc, describe and M-. asks a
backend, and a second backend (E5, E5b) is a new alist, not a new editor.

**E3f — what the reader and the break handler know.** After the SLIME
slice, and in both hosts:

- **Diagnostics as you type**: unbalanced or unreadable text marked from scm's
  own reader, split out of `scm-lsp`.
- **Document symbols**: the buffer's definitions as a popup to jump through.
- **The error pane**, SLIME's sldb: when a send raises and nothing catches
  it, the frames and their locals instead of one `error: msg` line. **A
  constraint to settle first:** `scm-dap` can wait inside the break handler
  because its VM has a thread to itself; the modeler evaluates sends on the UI
  thread, and winit has no nested event loop to wait in. So: a SNAPSHOT first
  (frames and locals captured, then abort), and "evaluate in this frame" /
  "continue with a value" only once scm has an answer — the VM off the UI
  thread, or a break that can resume later. That is a scm-side design
  question, not an `edit` one.

**E3f as built.** All three, in `edit/scm/11-diagnostics.scm` and
`12-error-pane.scm`, and none needed a change to scm: `scm::Reader`,
`scm_text::Doc::definitions` and the break handler were already library
calls.

- **The marks** are the widget's: `Language::diagnose` runs with the scan,
  so only when the text changes. The Scheme language asks a reader the
  binding installs (`language::scheme::set_reader`, because `language/` may
  not name the VM). Reading costs about twice the scan: 3 ms for
  `unify.scm`'s 337 KB and 17 ms for a 1.1 MB data file, so no debounce.
  Text that will not read is underlined in the error ink, and the modeline
  says which line. The outermost form left open is underlined faintly at its
  opening bracket, because that is also what every form looks like while it
  is being typed. The reader stops at the first error, so there is one
  mark. Only buffers that read as Scheme are marked: a scratch buffer or a
  `.scm`/`.ss`/`.sld`/`.sls` file, not a `.janet` session or a `.json`.
  `M-g n` / `M-g p` go there and say why; `M-,` comes back.
- **Document symbols** (`M-g i`, Emacs's imenu): the buffer's definitions
  as `name:LINE` minibuffer candidates, from the same scan as the `M-.`
  index.
- **The error pane** is a SNAPSHOT, as planned. `in-process-eval` calls the
  send's thunk through `%call-with-break`, a nested evaluation, so the
  editor's own `guard` is below its base and the send's uncaught error
  reaches the break handler (`scm_binding/breaks.rs`). The handler prints
  each frame's call, location and locals, and aborts. The result's
  `:frames` carry the snapshot. The condition reaches the shell as the same
  object a `guard` would have caught (`%break-condition`), so the
  transcript's `error:` line is unchanged. An error the sent code catches
  itself never breaks. The transcript says `C-h e` shows the calls;
  `view-last-error` opens them as a markdown page, `*error*`.
  `*error-pane-pops*` (off) opens it on every failure, as SLIME's sldb does.
  It is off because a send that fails while a scene is being shaped is
  ordinary, and the pane takes the editor's place until `q`. Tail calls
  leave no frame. A form SENT from a buffer is read from a string, so its
  frames say `<input>:LINE:COL`, counted from the start of the send, not
  the buffer; code loaded from a file carries the file's path. Naming the
  send's source needs `eval-string` to take a name, a follow-up.
  `test_reader_and_error_pane.py` drives all three in the modeler.

**E4 — the standalone program.** `edit` as a binary: a winit window with the
modeler's CPU framebuffer, a fixed layout (editor, transcript, minibuffer), an
in-process scm session that loads `edit/scm/` and `~/.config/edit/init.scm`,
`C-x C-f`/`C-x C-s`, and the SLIME sends evaluated in its own session. Its own
small layout rather than `panes.rs`, which is tangled with the 3D panes;
extract a shared layout only if the two start duplicating each other.

**E4 as built.** `cargo run -p edit -- FILE…` opens it; `edit --screenshot
OUT.png FILE` draws one frame with no window. It has four parts:

- `src/bin/edit.rs` is the winit window. It presents through softbuffer and
  routes keys, the mouse and the wheel the modeler's way: a typing pane
  gets printable keys as text, chords go as descriptors, the right button
  drives the marking-menu gesture, and Escape climbs the same ladder.
- `src/keys.rs` names a key press for the keymap. It moved from the
  modeler's `main.rs`, so both hosts bind the same descriptors.
- `src/app.rs` holds the three panes, the focus, the dirty flag and the
  draw, with no winit and no VM in it.
- `scm_binding/standalone.rs` is the session. It answers 00-base.scm's
  kernel prims over `app`'s panes and the file system, and boots this
  order: the editor's prims, `scm/host/prelude.scm`, the shell,
  `scm/host/standalone.scm` (the hooks bound to the three panes, a
  `find-file` for text, markdown and images, `C-x C-c`), then the user's
  `~/.config/edit/init.scm` (an error there is reported in the transcript,
  not fatal) and the files named on the command line.

**The prelude question is settled as COPIES.** `host/prelude.scm` holds
ray's eleven helpers byte for byte, rather than a slice of ray's prelude,
because `edit` must build and run without ray.
`the_host_prelude_is_rays_verbatim` fails when ray's text changes and the
copy has not. The bare-interpreter test now loads the same file instead of
inlining its own versions. `the_standalone_session_types_sends_saves_and_draws`
drives the program without a window: it boots, types, sends, saves, opens a
prompt and lets go of it, draws, and quits.

Not yet at first: menus. The editor's `:editor` sends menu and its
`*editor-menu*` routing are the modeler's (`unify.scm`), so a right-press
armed nothing. Kevin noticed at once (2026-11-13), and asked for a strip
icon to toggle the terminal, as the modeler has for its docks. Both landed
after E5:
- **Menus.** `host/standalone.scm` names its panes' roles (`*pane-role*`)
  and defines four menus:
  - `:editor`: the modeler's sends, then describe, copy and paste;
  - `:help`: back and toggle source, in a markdown view;
  - `:terminal`;
  - `:transcript`.

  A quick right-click posts a menu as a list; holding or dragging gives the
  marking menu.
- **The terminal icon.** It sits at the right end of the minibuffer strip
  (drawn in `app.rs`, as the modeler draws its panel toggles), lit while
  the terminal shows. A click runs `toggle-terminal`, which swaps the lower
  pane between the terminal and the transcript and leaves the program
  running; the first show starts `$SHELL`.
- **A test fix.** `app::install_host` does nothing under `cargo test`. The
  host is process-wide, and installing the standalone's from one test took
  the test host from under the widget's tests on other threads.

Kevin's second round: the icon should show or hide the WHOLE panel, as
the modeler's does, with a shell running in it by default, and a scene
loaded should bring up a REPL. He then asked for two mutually exclusive
icons, one for the terminal and one for the REPL, and that menus show
every item's key. What that became:
- **The bottom panel** shows a terminal or the REPL, or is hidden and the
  editor takes the height (`app::Panel`, `%show-panel`). The strip has two
  icons, λ for the REPL and a window for the terminal. Clicking the dim one
  shows its content; clicking the lit one hides the panel. A terminal's
  program runs on while hidden.
- **The REPL** is the transcript with a prompt. It evaluates where the
  sends go (this image, or the scm that remote-start attached), so what
  `C-M-x` defined, it can see. RET submits a form scm's reader calls
  complete (`%read-status`) and continues one it does not. `C-c C-z` shows
  it.
- **Startup** shows the REPL when a Scheme file was named on the command
  line, and a terminal running `$SHELL` otherwise.
- **Running a file** (`C-c C-b`, `C-c C-l`) is loading it, so the shell's
  new `*before-run-file*` hook shows the panel its answers land in: the
  REPL, or the terminal for a buffer attached to one. A hook rather than a
  wrapped `eval-buffer`, because a wrapper is an anonymous procedure and
  the menu could not find its key.
- **Key routing.** `*term-showing?*` (13-term) keeps a terminal that runs
  on behind the REPL from claiming the REPL's keys.
- **Every menu key shows.** The shell now labels the focus-guarded keys it
  binds itself: the sends, `C-x C-s`, `C-c C-d d`. Their labels were in
  the modeler's `unify.scm`, so the standalone's menus showed no key for
  those items. The modeler keeps only its own keys' labels.

The messages that speak of "this scene's files" are still the modeler's
wording.

**E5 — the terminal.** A pane that runs a program in a pseudo-terminal: a
REPL (`scm`, `python3`, `ghci`), a shell, a build. In the crate, so the
modeler gets it too.

- A PTY spawns the child and carries its bytes (`portable-pty`, wezterm's).
- A VT state machine keeps the screen grid and scrollback
  (`alacritty_terminal` — the emulator core alone, no window). Versions to
  check when E5 starts.
- The grid draws through the widget's monospace path into the framebuffer:
  one glyph per cell, colours from the cell, the cursor a block.
- A reader thread posts output and marks the pane dirty, as the render worker
  does; a resize sends the new size to the PTY.
- Keys go to the child encoded as the terminal expects (arrows as escape
  sequences, `C-c` as `0x03`), except the few the shell keeps to leave the
  pane — which ones is D4.
- Verbs: `(term-open "scm")`, `(term-send id text)`, `(term-close id)`,
  toggled like `M-o`.

**This is also the second backend.** `C-c C-e` in a buffer whose backend is
a terminal writes the form and a newline to the PTY — comint's model, and
how another language gets SLIME-style sends without an in-process VM. The
in-process scm session stays the default for Scheme: it is the one that
completes, finds definitions and describes against live state.

**E5-1 as built: the terminal, in `edit` and the standalone.** One crate,
not two. `alacritty_terminal` 0.26 opens the PTY itself (`tty`) and runs
the reader thread that parses the child's output into its grid
(`event_loop`), so `portable-pty` was not needed.
- **The widget** is `widget/termpane.rs`. It holds one terminal per pane
  id. It draws the grid through the mono path, with the program's colours
  over an xterm palette, inverse video, underlines, and a block cursor
  while typing. It tells the child its size (SIGWINCH) when a draw finds
  the pane resized. It encodes keys the way xterm does, honouring the
  program's cursor mode. The reader thread wakes the host through
  `termpane::set_waker` (winit's proxy in the standalone), because a
  pane's dirty flag belongs to the UI thread.
- **The shell** is `scm/13-term.scm`:
  - `term-open` / `M-x term` run a program; `$SHELL` when none is named.
  - `term-close` stops it.
  - `C-x o` moves the keyboard between the editor and the terminal.
  - A key handler at 15, above the keymap and below the menus, sends
    everything to the program except the keys the editor keeps.
- **D4 is vterm's choice.** The editor keeps the `C-x` and `C-c`
  prefixes and `M-x`. `C-c C-c` sends a literal `C-c`, from the terminal
  pane's own keymap (`bind-term-keys`), so the modeler's `C-c C-c` (eval
  defun) is untouched. Escape goes to the program.
- **The second backend.** `term-attach` points the active buffer's sends at
  the terminal: `(terminal-backend id)` types the form and a return, and
  nothing structured comes back, as in inferior-lisp. Every send now asks
  `backend-of`, which honours an attachment before the host's
  `*buffer-backend*`.
- **The standalone** shows the terminal in its lower pane while one runs,
  and the transcript otherwise.
- **Tests.** A real `/bin/sh` runs in a test: its output lands on the
  screen, `tput cols` agrees with the drawn size, and `C-d` ends it. The
  standalone session test opens a shell, types into it, leaves with
  `C-x o`, and sends an attached buffer to it.

**E5-2 as built: the terminal in the modeler.**
- **The pane.** A `term` pane kind (`PaneKind::Term`) that the frame
  draws through `edit::termpane`. The `:terminal` pane shares the BOTTOM
  dock with the REPL, as the editor shares the left one with the
  outliner: `*term-shown*` swaps it into the slot, and `*term-closed*`
  or `focus-repl` gives the slot back.
- **Wake and Escape.** The reader thread wakes the window with
  `Wake::Term`. Escape's kernel rung sends a terminal's program Escape
  before anything else on the ladder.
- **The test.** `test_terminal_pane.py` checks these in a real modeler:
  - the dock swap;
  - typing through the kernel's injected-key path;
  - `C-x o`;
  - an attached buffer's `C-M-x` reaching `sh`;
  - closing.

**E5b — a socket backend: SLIME's model, over scm's own protocol** (Kevin,
2026-11-13: *"is it the same as when running sbcl under emacs using slime?
Can we edit the running image?"*). The terminal backend is `inferior-lisp`:
a send rewrites the running subprocess's definitions, but nothing structured
comes back. scm already has the SWANK half — `scm --serve HOST:PORT`, the
protocol in `~/devel/scm/docs/repl.md`, which `ray --tcp` serves on 4007:

| op | what the editor gets | SLIME's |
| --- | --- | --- |
| `eval` | the value, printed, and what the form printed | `C-x C-e` → `=> 42` |
| `complete` | the live image's matching names | completion |
| `describe` | kind, signature, docstring, current value | eldoc, `C-c C-d d` |
| `interrupt` | stops a runaway evaluation | `C-c C-c` |
| `parse` | complete / incomplete / invalid | RET or continue |

So a backend over a socket answers four of the five procedures of the
interface (E3e) — `eval`, `complete`, `signature`, `doc` — against ANOTHER
process's image, and `definition-sites` stays local (`scm-text`'s index).
The editor starts `scm --serve` on a free port and connects, as SLIME starts
SBCL with SWANK, or it attaches to a server already running — `ray --tcp`
today, the modeler if it serves the protocol — and edits that program's
live image. A runaway loop or a crash stays in the server; `interrupt`
stops the first.

What SLIME has and this will not: SLDB's restarts (scm has no resumable
conditions — on an uncaught error `scm-dap` shows frames and locals, then
aborts or continues with a value; no stepping), the inspector, and who-calls
cross-reference. Small, because the server and the protocol exist; it
needs E3e's interface first. **Which backend a Scheme buffer sends to by
default** — the in-process session (completion against the editor's own
image) or a socket (isolation from the code under test) — is a decision for
E4/E5b, not before.

**E5b as built.** `scm_binding/remote.rs` is the client, and
`scm/14-remote.scm` the commands. The wire is scm's own `Request::to_wire` /
`Response::parse`, framed as `scm --connect` frames it (read until one
complete datum), so this client cannot come to disagree with the server.
- **Connecting.**
  - `remote-start` runs `scm --serve` on a free port (`*remote-program*`)
    and points the buffer's sends at it.
  - `remote-connect "host:port"` attaches to a server already running,
    `ray --tcp`'s 4007 by default.
  - `remote-stop` drops the connection, and stops the server if
    `remote-start` began it.
  - `remote-attach` / `remote-detach` point a buffer at the connection, or
    back at the host's backend.
- **The backend.** `remote-backend` answers `:eval` with the printed value,
  what the form printed, or the error with its backtrace as error-pane
  frames (calls, no locals: a wire carries values, not a stack).
  `:complete`, `:doc` and `:signature` ask the remote image.
  `:definition-sites` stays this editor's index.
- **Waiting.** A send waits, as an in-process one does, for
  `*remote-timeout*` seconds (30). Past that it sends `:interrupt` on a
  SECOND connection, because the server is single-threaded and an interrupt
  queued behind the form would never arrive. The answer comes back
  host-stopped, marked "(stopped)".
- **Checked.** Against the real `scm --serve`: `(+ 40 2)` answered
  `=> 42`, a `(let loop () (loop))` was interrupted at a 1 s timeout, and
  the same image answered the next send. The committed test runs a protocol
  server built from `scm::Session` (no binary needed), sends a definition
  and a call from a buffer, checks that the definition landed in THAT image
  and not this one, and describes and fails across the wire.
- **Not yet, at first:** sends were synchronous, so while one ran the window
  waited, as it does for an in-process send. Async sends (below) followed. Which backend a Scheme buffer
  uses by default stays the in-process image; a socket is chosen per buffer.

**E6 — a second language.** One `Language` and a terminal backend, end to
end: Python is the obvious one. This is where the trait boundaries get
tested; until a second language exists they are guesses. **No language
server**: highlighting and indent are compiled in, sends go to `python3` in
the terminal pane, and completion falls back to the buffer's words. An LSP
client (pyright, rust-analyzer) is OUT of the plan by default; if it is ever
wanted, it is opt-in — used only when a server is already on the `PATH`,
with no configuration — and nothing works differently without it.

**E6 as built: GNU Guile** (Kevin, 2026-11-13: "for the second language
let's use guile scheme" — not Python). A second Scheme sounds like a weak
test of the boundaries, and it is the right one. The TEXT is nearly the
same, so what has to vary is exactly what the boundaries are for: which
reader judges the text, and which image a send goes to.
- **Language per buffer.** `Language::name`, `language::by_name`, and
  `editorpane::set_language` / `buffer_language` (`%editor-set-language`,
  `%buffer-language`). `language/guile.rs` is Scheme's scan, indentation
  and call-at-point, with `#:keyword` coloured as a keyword and NO read
  marks. Scheme's marks are scm's reader's verdict, and scm rejects Guile
  syntax (`#:key`) that Guile reads fine.
- **A language names its backend.** `backend-of` asks
  `*language-backends*` by the buffer's language before the host's
  `*buffer-backend*`, so a Guile buffer in the modeler sends to Guile, not
  into the scene. If Guile cannot start, the send FAILS and says why. It
  never falls through to scm's image.
- **No second client.** `edit/guile/serve.scm` is a Guile program speaking
  scm's REPL protocol:
  - eval (compiled, so procedures keep their parameter names), with
    captured output and Guile's error message;
  - complete (`apropos-internal`);
  - describe (procedure-arguments, docstrings);
  - parse;
  - interrupt from a second connection (`system-async-mark` on the
    evaluating thread);
  - ping.

  E5b's socket client is Guile's backend, unchanged: `%remote-start` now
  takes the program's arguments (`{addr}` for the address).
- **Commands** (`scm/15-guile.scm`): `guile-start`, `guile-stop`,
  `guile-mode`, `scheme-mode`. A file is Guile when it says so (a
  `#!…guile` line, `define-module` / `use-modules`, a `.guile` name),
  checked on open (`detect-language!`, from `visit-file-text` and the
  modeler's `visit-scene-text`). The first send starts the Guile.
- **Getting Guile.** It comes from ray's flake dev shell (Kevin: not a
  global install). Tests that need it skip without it, saying so, and the
  oidn run, which is inside the shell, runs them. Always tested: a Guile
  file opens as Guile, and a send whose Guile cannot start fails rather
  than running as scm.
- **Two server bugs the probe found.** Guile 3 has no `keyword->string`.
  And an answer to a request that failed went out as `:id 0`, which left
  the client waiting out its 5 s timeout for its own id.
- **Not yet.** Read marks from GUILE's reader (the `parse` op is there; the
  widget asks synchronously and per keystroke, so it needs the language to
  ask an image, which is a design step). `M-.` into Guile's own modules.

**After E6: which Scheme a standalone buffer runs in** (Kevin, 2026-11-13).
- **`remote-start` asks which Scheme** — `scm` or `guile`, from
  `*schemes*` (`define-scheme!`: program, arguments, the language a buffer
  sending to it takes), the buffer's own offered first — and the buffer
  takes that Scheme's language. `remote-stop` lets go of the buffers it
  served.
- **A Scheme FILE runs in a separate scm by default.** The standalone's
  `*buffer-backend*` sends a `.scm`/`.ss`/`.sld`/`.sls` file to one scm
  started on the first send (`*scm-files*`), so the program being written
  cannot freeze, crash or rename the editor. What configures the EDITOR
  stays in its image, as Emacs's `*scratch*` does: the scratch buffer,
  `~/.config/edit/init.scm` and the shell's own `edit/scm/` files.
  `run-in-editor-image` points a file there by hand. A Scheme that cannot
  start fails the send, naming why (`unavailable-backend`), and never
  falls through. The REPL's prompt says where it is: `scm λ > `,
  `scheme@(guile-user)> `, or `scheme λ > ` for the editor's image. The
  Schemes the editor started stop when it exits (`remote::close_all`).
- **What a separate image costs:** the error pane's locals — a wire carries
  the calls, not the frames' bindings. `scm --serve` sending locals with
  its backtrace would close that (a change in scm's server).

**D5 as built: the modeler's REPL and terminal, consistent with `edit`**
(Kevin, 2026-11-13).
- **The bottom dock** holds the REPL OR the terminal, never both. The strip
  has an icon for each, as `edit`'s does: λ for the REPL, the terminal
  window for the terminal. `toggle-repl` and `toggle-terminal` swap their
  own into the dock, or close the dock when their own is showing. The
  terminal's first show starts `$SHELL`, which runs on while hidden.
- **No remote Scheme for the scene.** The modeler's image IS the scene, so
  a send to another process would answer and change nothing.
  `remote-start` and `remote-connect` ask the new host hook
  `*remote-allowed?*`, which the modeler answers with its reason. A Guile
  file still goes to Guile, which is never a scene, and the terminal still
  runs programs.
- **The REPL commands are shared.** `,help ,describe ,apropos ,time ,expand
  ,expand1 ,load ,bt` live in `scm/16-repl-commands.scm`, taking the REPL's
  backend and its eval-print. The modeler's REPL evaluates in the scene,
  always, under `scheme λ >`. The standalone's follows the active buffer,
  having no scene to anchor to.
- **A REPL error keeps its calls.** The modeler's REPL evaluates through
  `%call-with-break`, so a line's uncaught error keeps the calls it raised
  in for `C-h e`, as a send does. The error line reads as before.
- `test_terminal_pane.py` (15 checks) holds the swap, the close, the
  refusal, `,help` at the REPL and the REPL's frames.

**Async sends (2026-11-13).** A send to another process no longer holds the
window: SLIME's model.
- **A reader thread per connection** (`remote.rs`). It parses answers as
  they arrive and matches each to its request by `:id`. A waiting request
  (complete, describe, the REPL commands' synchronous eval) gets its answer
  straight away; an async one's answer is queued and the host woken.
  Answers can now arrive back to back, so the reader splits off one datum
  at a time (`first_datum_len`): handing `parse_status` two lost the
  second.
- **The waker is shared.** `edit::wake` serves the terminal and the
  connections, and both hosts call `(remote-poll)` when woken.
- **The backend gains `:eval-async (src k)`.** `editor-send-local` uses it
  when it is there: it echoes "running in …", returns at once, and
  `send-answer` prints the RESULT when it comes. The standalone REPL does
  the same, and `,time` waits for its answer (`eval-print`'s THEN).
- **Nothing queues behind a running send.** While one runs, completion and
  describe answer nothing at once rather than wait behind it, since the
  server does one thing at a time.
- **No timeout on async sends,** since the window does not wait. A runaway
  is stopped with `M-x remote-interrupt`, which now asks the active
  buffer's own process.
- **Measured** (`a_slow_send_does_not_wait`): the send returned in 1 ms
  while the form ran 150 ms, and describe answered in 33 µs during it.

**The file tree (2026-11-13).** Kevin asked for one in `edit`, rooted at the
git checkout, with what git ignores hidden and a menu to expand, collapse
and collapse all.
- **The layout** (`app.rs`). The tree is a left column (pane `FILES`)
  beside the editor and panel, with its own draggable edge (`Divider`:
  `Panel` or `Files`, 8–50% of the width). The strip gains a folder icon
  (`Icon`: `Files`, `Repl`, `Terminal`) that shows and hides it.
- **The tree** (`scm/17-files.scm`). Text-pane rows in list layout
  (`%pane-top-anchor`). Folders come first, then files; dotfiles never
  show, and what git ignores shows only on request (`i`). RET or a click
  opens a file (`open-path`) or opens and closes a folder. `files-reveal`
  follows the buffer on screen, through `*buffers-changed*`, opening the
  folders above it. `g` reads the disk again.
- **Its keys and menu.** In its own keymap, because the tree takes the
  FOCUS, not typing, so a bare key is a command: up/down or p/n, right/left
  or +/-, RET, g, i, q. The menu offers open, expand, collapse, collapse
  all, refresh, show/hide ignored and hide the tree. A right-press selects
  the row under it first.
- **Folder icons.** The mono face has no folder glyph, so a folder row
  carries two private-use placeholder cells (`app::FOLDER_MARK`) and the app
  paints a folder over them, at the rows the pane records for list layout
  too (`laid_rows`).
- **New prims.** `%git-root` and `%git-ignored` (ray's prim, answered by a
  standalone without ray), `%pane-row-at`, `%show-files`.
- `the_file_tree_shows_the_checkout_and_opens_files` holds it against a
  temporary git checkout.

**The standalone in use (2026-11-13/14).** What Kevin asked for while using
`edit` every day, in the order it landed. Each is a commit of its own, named
after his words.
- **The tree, as Zed's panel looks** (`28b3535e` `6ea71ad0` `a960dfa7`).
  Names in IBM Plex Sans, 1.08 of the text size, while the indent and the
  icon cells keep the mono grid (`%pane-proportional`). A row carries two
  mark cells for its icon: a folder, braces for source, a picture, a page
  (`FOLDER_MARK` … `FILE_MARK`, chosen by extension in 17-files). A faint
  guide runs down from each open folder. The text pane draws all of this
  for any proportional list pane (`textpane::draw_tree_marks`), so every
  host's tree has it.
- **Copy and paste in the REPL and the terminal** (`4ed852f2`). A drag
  selects: alacritty's `Selection` in the terminal, the transcript's in the
  REPL. Cmd-C copies, Cmd-V pastes: into a shell as one bracketed paste,
  into the REPL with its lines kept and nothing run until RET.
- **Search and replace** (`9afb0fcd` … `dff08fde`, `scm/18-search.scm`).
  - C-s / Cmd-F / C-r, and vi's / and ?, search as you type. The match
    follows each character from where the search began, a miss says
    `Failing`, C-s / C-r at the prompt step on, and C-g or Escape goes back.
  - RET keeps the match and turns on single keys, which the accent prompt
    lists: n next, p (N in vi) previous, r replace one by one, a replace
    all, q done. Any other key ends the search and acts, as isearch does.
  - M-% / Cmd-Alt-F step through matches asking y / n / a (all the rest,
    one undo step) / . / q, with ? for the same answers as a menu.
    `replace-all` does the whole buffer at once.
  - Plain text within a line, smart case. The editor widget has `find`,
    `select_range` and `replace_all` (a range that may wrap round the end);
    the prims are `%editor-find`, `%editor-select` and `%editor-replace-all`.
  - The minibuffer gained two callbacks: `:on-change`, for the incremental
    search, and `:on-cancel`. A `:plain` prompt source makes @ and TAB
    plain text at a search prompt.
- **vi's command line** (`63991b8c` `7924f35e`, `scm/19-ex.scm`). In normal
  mode, `:` opens the minibuffer, and `ex-run` takes the everyday commands:
  - :w, :q (refused over unsaved buffers, naming them), :q!, :wq / :x,
    :wa, :e FILE, :e!, :bn / :bp / :bd / :ls / :b, :N and :$;
  - :s and :%s, where flag c steps through;
  - :noh and :help.
  Anything else runs as an M-x command. ZZ and ZQ are done here, because
  modit takes the Z and drops the second key. `:help` opens one markdown
  page: vi's normal-mode keys as modit 0.1.5 parses them, the search in
  both styles side by side, the replace, the `:` commands, then the live
  keymap. Two host hooks, `*ex-quit*` and `*ex-open*`, cover what leaves
  the buffer.
- **Markdown tables** (`89d21edf`). The reading view sets them out in
  aligned columns: the mono face makes padding align, a dim │ separates
  columns, a ─┼─ rule sits under the header, and alignments are honoured.
  Before this, a table ran together into one paragraph.
- **The terminal's shell** (`8a70b94f`). It is the LOGIN shell, from the
  account database (`%login-shell`: dscl, getent, /etc/passwd), not
  `$SHELL`. Under `nix develop`, `$SHELL` is the build environment's bash,
  which has no readline, so Tab completed nothing.
- **Menus** (`96e98700`). Menu text is near-white (#f5f7fa, its key
  #c3c8d0) in both programs' themes and presets, in Plex Sans, the desktop
  menu face.

**The modeler, kept level with it (2026-11-14).**
- **Search, :commands and :help.** These are shell code, so the modeler's
  editor has them as they landed. Find… and Replace… close its marking
  menu, after the sends, so no sector moves (`f5d819d1`).
- **The file tree** (`6ae71a46`). A status-strip folder toggle shows
  17-files' tree as a column at the left of the LEFT dock, in front of the
  editor, which keeps its width; the column has its own divider. Showing
  it brings the editor up.
  - The modeler gained an `open-path`: `find-file`'s dispatch, plus plain
    text for any other file.
  - Its `*buffers-changed*` also reveals the buffer in the tree, and the
    tree has a `:files` role.
  - To share the tree, three things moved out of the standalone: the icon
    drawing (into the text pane), `%git-root` (into edit's prims), and the
    `:files` menu routing (into 17-files, with a `*files-shown?*` hook).
  - `the_file_tree_shows_beside_the_editor` holds it.

**E6b as built: Rust, piece 1** (Kevin, 2026-11-14: "do piece 1, the Rust
Language, first"). This is the first language whose text is not Scheme, and
it adds nothing that runs beside the editor.
- **`language/rust.rs`** is a hand-written lexer, as Scheme's is. It reads
  `//` and nesting `/* */` comments, strings (raw, byte and C strings too), a
  char literal as distinct from a lifetime, numbers (`1..2` stays a range),
  raw identifiers and attributes. A bracket inside any of those is not a
  bracket.
  - Highlighting uses the theme's slots: keywords and lifetimes, `true`/
    `false`, and a function's name where it is called or defined (a
    macro's with its `!`).
  - Indentation is rustfmt's block style. A line goes four in from the
    innermost open bracket's line, back out for a closer, and four more for
    an expression carried on (a chain, a `let`'s right side). A chain off
    `})` stays level with it, and a let-else's `else` stays with its `let`.
    Inside a string the editor never adds spaces.
  - The call around the point is the name before the innermost `(` (`foo`,
    `Vec::new`, `println!`, a method). It looks out through tuples, but
    never out of a `{`.
  - No read marks: whether Rust is right is the compiler's verdict.
- **Measured** on every `.rs` file in ray (252) and scm (120), with
  `rust_corpus` (ignored; `RUST_CORPUS=DIR`). No file's brackets come out of
  place. TAB leaves 99.5% / 99.6% of the rustfmt-formatted lines where they
  are. Most of the rest are files rustfmt does not format, or
  `matches!(…, … if …)` guards.
- **The shell, `20-rust.scm`:**
  - A `.rs` file opens as Rust. `detect-language!` moved to 07-backend as a
    list of detectors that each language adds to; Guile's is one of them.
  - Rust's backend refuses a send rather than letting it fall through to
    this image, where it would read as Scheme.
  - Rust is one of the `*non-scheme-languages*`. In those, completion
    offers the buffer's own words and eldoc stays quiet, because the
    image's names mean nothing in them.
  - `M-x rust-mode` sets a buffer to Rust by hand.
- **Not yet** (both done in E7): completion's token was still Scheme's,
  and nothing was underlined.

**E7 as built: rust-analyzer, when it is on the PATH** (Kevin, 2026-11-14:
"lets do next phase" — which reverses E6's "no language server" default,
on E6's own terms: opt-in by presence, nothing configured, nothing different
without it).
- **`lsp.rs`** is a generic LSP client over a child's stdin/stdout. It never
  names the VM, and its prims are in `scm_binding/lsp.rs`. A language
  registers a program and a root marker:
  `(%lsp-register "rust" "rust-analyzer" '("Cargo.toml"))`. The root is
  the nearest marker, then further up while a `Cargo.toml` declares a
  `[workspace]`, so all of ray's crates share one server.
- **Nothing on the UI thread waits on startup.**
  - `initialize` goes at once. What follows is queued until the reader
    thread has the answer, which then sends `initialized` and the queue in
    order.
  - All writes go through a writer thread, so a server slow to drain its
    pipe cannot freeze the window.
  - Positions are bytes (`positionEncodings: utf-8`, which rust-analyzer
    accepts). A UTF-16-only server is converted line by line.
- **Diagnostics**: the server pushes them, the reader stores them and wakes
  the host, and the widget draws them. The widget's hook is
  `editorpane::Watcher`: it is told each file buffer's text when it changes
  (`didOpen`, then whole-text `didChange`) and asked what is wrong. Errors
  draw in the error ink, warnings quietly, hints not at all. `M-g n` goes to
  one and says why. A save sends `didSave`, so `cargo check`'s errors come
  back too (`*save-hooks*`, 05-buffers).
- **The questions** ask the server about the point, waiting at most 1.5 s
  and cancelling past it:
  - TAB completes from the server, falling back to the buffer's words while
    it loads. A token in Rust is letters, digits and `_`, so `xs.it`
    completes `it`.
  - `M-.` jumps (`:definition-sites`).
  - `C-c C-d d` describes from hover. The answer is kept only when there is
    one: a cached "nothing" from a server still loading hid every later
    answer until this was found in testing.
  - Eldoc's argument hint is asked without waiting. It arrives on a wake,
    and `*wake-hooks*` (14-remote's `remote-poll`) echoes it.
- **Commands**: `rust-analyzer-status` (starting, indexing with the
  server's own progress, ready, or not on the PATH), `rust-analyzer-stop`,
  `rust-analyzer-start`.
- **Tests**:
  - a fake server on a pair of pipes (queueing, the order after
    `initialize`, diagnostics, a server's own request answered);
  - a program not on the PATH serving nothing;
  - the watcher hook in the widget;
  - two `#[ignore]`d runs against the real rust-analyzer, one of them
    through the standalone shell exactly as a user reaches it. Completion
    answers about 0.3–0.5 s after opening a small crate.
- **Not yet:** `didClose` when a buffer is killed (the server keeps the
  file); rename, code actions and formatting; completion ranked by the
  server's own order (the shell re-ranks by match); a mode-line indicator
  of the server's state.

**Still open.**
- **In scm** (each is a change in the scm repo):
  - a sent form locates as `<input>:L:C`, because `eval-string` takes no
    source name;
  - a remote error carries its calls but not their locals;
  - `scm --serve` answers one of two coalesced requests (worked around
    here: one request in flight per connection);
  - a break cannot resume later.
- **Here:**
  - Guile buffers have no reader marks;
  - modit has no Ctrl keys, so vi's C-d / C-u / C-r do nothing (C-r is the
    backward search);
  - search is plain text within a line, with no regex and no matches
    across lines.
- **Decisions:**
  - D1, the name (`edit` still);
  - D2, the move to the scm repo once the crate settles;
  - D3, a second OS window.

## What it does to the modeler

**Smaller by about a seventh; simpler at the seams; one boundary added.**
Measured at `78a7ae8f`:

| | the modeler now | moves to `edit` | after |
| --- | --- | --- | --- |
| Rust | 50,259 lines | ~6,600 (`editorpane` 3,330, `textpane` 1,417, `text` 673, `framebuffer` 298, editor/text/xref prims ~900) | ~43,600 (−13%) |
| Scheme prelude | 23,990 lines | ~3,900 (`00-decls`, `06-keymap`, `09-menus`, most of `08-editor`, ~2,100 of `unify.scm`) | ~20,000 (−16%) |

The code mostly MOVES; the one real deletion is the scanner (three to one).
The crate adds the `Host` trait, the hooks, the menu registry and the terminal.

Simpler:

- `unify.scm` (8,587 lines) stops interleaving the editor between the curve
  tools and scene loading.
- What a send does to the scene stops being buried: `editor-send-local` calls
  `%sync-retained`, `*outliner-refresh*` and `%poke-settle` from inside the
  eval today; afterwards it is four named hooks in one place.
- `on-menu-press` stops being a `cond` over every kind of pane; a feature
  registers its own menu.
- The editor's tests run without booting the modeler, and the standalone is a
  second host that catches modeler assumptions leaking in.

The cost: load order becomes a rule (`edit/scm/` loads first, so the modeler
can hook the editor but not the other way round — `00-decls`'s `*echo*`
forward hook already works this way), and an editor change has two hosts to
make sense in.

### Which bugs cross the boundary

Only the ones at the seam (Kevin, 2026-11-13: *"it's only a boundary to cross
when debugging if it is an editor bug right?"*):

- **A modeling bug** — geometry, tools, the outliner's contents, rendering —
  never touches `edit`.
- **An editor bug** — cursor, vi, undo, highlighting — is entirely in `edit`,
  and reproduces in the standalone, which is easier than in the modeler.
- **A seam bug** crosses: a send that misbehaves in the scene, a modeler
  menu, focus between a 3D pane and a text pane, a key that should reach a
  modeler command.

The qualifier: "the editor" includes the SHELL, and keymaps, handler chains,
menus and the minibuffer carry every key and right-press in the modeler, not
only the editor's. So an INPUT bug can cross even when it has nothing to do
with text. The outliner's Delete key (`980b16a0`) was one: key dispatch and
focus (shared, after E3a) plus the outliner's click handler (stays). The fix
would land on the modeler side; the diagnosis would read the shared dispatch.

## Decisions for Kevin

- **D1 — the name.** `edit` is a placeholder.
- **D2 — where it lives. DECIDED 2026-11-14 (Kevin): the scm repo.** It
  started in ray's workspace, as recommended here:
  One gate covers both hosts, the UI harness keeps exercising the editor, and
  an improvement reaches the modeler on the next build. The scm repo (beside
  `scm-lsp` and `scm-text`) is the tidier home for an scm editor, but ray
  pins scm by git commit, so every editor change would mean push scm, then
  `cargo update` here — the friction that hurts most while the editor is
  still moving. Move it once it settles.
- **D3 — "pop up": a pane or a window.** Recommended: a pane first (one
  framebuffer, one event loop, docks like the outliner), a separate OS window
  later if wanted. winit can open a second window, but every pane, key route
  and redraw would then be per-window.
- **D4 — which keys the terminal keeps.** Emacs's vterm keeps a short list
  (`C-x`, `M-x`, the pane switch) and sends everything else; `C-c C-c` sends a
  literal `C-c`. Proposed: the same.
- **D5 — the REPL pane's future.** The modeler's REPL is an in-process
  transcript, and it can stay one. A terminal running `scm` is a different
  thing: another process, another image. Both are useful; neither replaces
  the other.
