# `scm-lsp` — a Language Server

2026-10-22, stage S23. `scm-lsp [--addr 127.0.0.1:7000]`, stdio LSP.

Completion, hover, signature help, diagnostics, the document outline and
go-to-definition for `.scm` and `.sld` files. Zed is the named consumer
(`zed-scm`, Z3) and the reason it exists — a Zed extension cannot answer a
completion request itself, only name a program to launch — but nothing in
it is Zed-specific, and the modeller's editor pane is the next client.

## Names come from a live session, not from a parse of the file

This is the decision the stage turns on. Ask a running scm what it has and
the editor knows:

- the procedure defined at the prompt a second ago, written in no file;
- an accessor a macro generated — `point-x` from `define-record-type`;
- the docstring and signature **as the value carries them**, which is what
  `(doc …)` and `(signature …)` answer and what S8's `describe` already
  returns.

Parse the file instead and you know only what the text says, which for a
Lisp is the smaller half. It is the difference between completion and
SLIME's completion, and it is why `scm-lsp` is a **third client of the one
door**: the editor, the prompt and the language server, all against one
`scm --serve`. S22's `Request::to_wire` and `Response::parse` exist for
this, and the test that matters asserts exactly it — define something at a
prompt, then watch the editor complete it.

With no server listening it falls back to a session of its own, so a cold
buffer still completes over the prelude's 386 names; the `initialize`
handshake reports which it got, because a user with empty completion needs
to know whether the server was found. If a connected server goes away —
restarted — the next request falls back rather than failing silently for
the rest of the day.

**Nothing in it evaluates anything.** Only `complete`, `describe` and
`ping` are ever sent. An editor that evaluated a buffer to analyse it would
run a scene's side effects on every keystroke.

## Structure comes from a token scan, and that is not a shortcut

An editor's buffer is **unparseable most of the time**: every half-typed
form is an incomplete datum. A reader-based outline would blink out on the
keystroke after `(define (`, so `text.rs` lexes instead, and the outline of
a file being written still lists the definition being written. `imenu` has
always worked this way.

The lexer is one left-to-right pass that cannot fail. Three cases are
load-bearing rather than fussy, and each has a test:

- **`#\(` is a character, not a bracket.** Get it wrong and every form
  after it is unbalanced — the symptom being an outline that stops halfway
  down a file for no visible reason.
- **`#| … |#` nests.** A search for the first `|#` leaves the rest of the
  file inside a comment.
- **`'`, `` ` ``, `,` and `#;` prefix a datum without nesting it**, so they
  must not move the depth.

Only top-level definitions are listed: an outline should be the file's
shape, not every binding in it.

## Go-to-definition crosses the workspace

A jump whose answer can only be in the file you are already looking at is
the least useful kind, so the server indexes the whole worktree: every
`.scm` and `.sld` under the roots `initialize` names, scanned once with the
same token pass, with each name resolved to a **line and UTF-16 column at
index time** so the index holds no file contents. Measured on this
repository: 71 files, 295 definitions, 0.03 s.

`target`, `.git`, `node_modules` and anything dot-prefixed are skipped —
`target` alone can hold thousands of `.scm` fixtures, and a jump into a
build directory is worse than no jump. A file over 4 MB is generated or it
is data, not something to jump into.

Precedence is **this file, then everywhere else**: a name defined where you
are reading is the one you meant. Otherwise every site is returned, because
a name defined in two libraries is normal rather than an error, and the
editor can offer the choice. An open buffer indexes from its *text* rather
than from disk, so a definition typed a second ago is already a target; on
close it is re-read from disk, and if it cannot be read — deleted or
renamed while open — its entries are dropped rather than left as jumps to
nowhere.

`workspace/symbol` falls out of the same index for a few lines, and it is
how a reader finds a name they half-remember.

### What a record type binds, which nothing used to find

`(define-record-type point (make-point x y) point? (x point-x))` binds
*five* names. Only `point` is where a `define`'s name would be, so the
general case missed the constructor, the predicate and every accessor —
`point-x` was unreachable by go-to-definition **in the file that defines
it**, which is the kind of gap that looks like the feature not working at
all.

They are indexed individually and, in the outline, **nested under the
type**: they have to be findable, but six flat entries for one declaration
is not the shape of a file. `define-values` had the same shape of bug —
only the first name of `(define-values (q r) …)` was recorded.

### The bug underneath both

`token_at` took the first token whose range contained the offset
*inclusively*, so at the `h` of `(helper` the **opening paren**, which ends
at that same offset, won. Hover and go-to-definition therefore did nothing
for a symbol written straight after a delimiter — which in a Lisp is most
of them. Containing an offset now beats ending at it, and the
cursor-just-after-a-name case still resolves, because both are real and
they only collide at a boundary.

### What this still cannot do, and the shape of the better answer

It finds what a file *says*. A name defined at the REPL has no entry, and
neither has a prelude name — those are values, not lines.

`sb-introspect:find-definition-sources-by-name` is the better answer: the
*running session* saying where a binding came from, which resolves the
binding rather than matching a name and covers everything defined
interactively. Doing that here needs a location recorded at definition
time; scm's per-form location table is pruned by the collector, so it is a
small addition rather than a free one.

## Diagnostics come from the reader, because that is its question

`Reader::read_all` on the buffer, and the two failures are reported
differently — which is S1's distinction arriving in the editor's gutter:

| reader | LSP | why |
|---|---|---|
| `Invalid { span, msg }` | severity 1, **Error**, at the span | the text is wrong |
| `Incomplete` | severity 3, *Information*, at the end | "keep typing" is not an error |

Marking an unclosed form an error would put a red mark under the cursor of
every form being written. The reader already distinguishes the two; the
server only has to not throw that away.

## Offsets

Three coordinate systems meet here and none of them is the others:

- scm's `Span` indexes the **`Vec<char>`** its reader works on;
- LSP's `Position` is a 0-based line and **UTF-16 code units** from the
  line's start;
- `Content-Length` counts **bytes**.

So bytes appear only in the transport, and `Doc::position`/`Doc::offset`
convert char offsets to UTF-16 columns and back. A server that counted
characters would put the cursor in the wrong place on the first line
holding an emoji — the test uses one, with a `λ` beside it, since `λ` is
one unit and 🙂 is two.

## The dependency

`serde_json`, and it is in this crate rather than `scm-cli` deliberately:
the `scm` binary has a 10 ms boot budget with a test enforcing it (§7, S10)
and a `--batch` sweep pays it per scene, while an editor server lives for
hours. The embeddable `scm` crate keeps its no-dependencies rule either
way — this is tooling, like `rustyline` in `scm-cli`.

## What it does not do

- **Go-to-definition cannot reach a prelude name** or anything defined at
  the REPL. A prelude name is a value, not a line; hover is what tells you
  about those, and a wrong jump would be worse than none. It *does* cross
  the workspace — see above.
- **Completion is not scoped.** Candidates are the global environment plus
  the file's own top-level definitions, so a `let` binding is not offered.
  The same limit the REPL's completion has.
- **No rename, no references, no formatting.** Formatting in particular
  wants an opinion about Lisp indentation that this project has not formed.
- **Signature help reads `(signature …)`'s own text** and splits it on
  whitespace for the parameter list, rather than modelling the lambda list
  a second time. `&opt` and `:keyword` markers therefore appear as
  parameters, which is honest about what the signature says.
