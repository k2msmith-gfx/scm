# Common Lisp in `edit`, through SBCL — plan, 2026-11-17

**IN PROGRESS.** L0 (the measurements), L1 (the server), L2 (the
language), L3 (the shell) and L7 (the debugger) are done; each is below. A
`.lisp` file sends to SBCL, and an error is a break at the REPL panel. L4–L6
are next. Kevin, 2026-11-17: *"I would probably switch to edit if it
supported all of the L features"* — so all of L1–L7, in D6's order (L1–L3,
L7, L4–L6), with each decision as recommended.

Kevin, 2026-11-17: *"what is the level of effort to support sbcl to the
same level as guile or scm?"* — and, after the Guile reader check landed:
*"then we can plan for sbcl."*

So: a `.lisp` buffer in `edit` whose sends go to an SBCL the editor starts,
with the same keys, transcript, REPL panel, completion, describe and read
marks a Guile buffer has — **about a day of work** — and then SBCL's own
debugger in the REPL panel, half a day more. The stages are below.

**This is not [cl-plan.md](cl-plan.md).** That document proposes a Common
Lisp *implementation* in Rust. This one uses SBCL as it is, as an external
process the editor talks to — the way it uses Guile. Nothing here depends
on that plan or constrains it; a CL built there could later answer the same
protocol and be a second backend for the same buffers.

## What "the same level as Guile" is

What a Guile buffer has today, each with where it comes from:

| | Guile gets it from | size |
|---|---|---|
| sends (`C-x C-e`, `C-M-x`, `C-c C-r`, `C-c C-b`), values and errors in the transcript | `edit/guile/serve.scm`: a server in Guile speaking scm's REPL protocol ([repl.md](repl.md)) | 223 lines |
| the REPL panel, in Guile's prompt | the shell, following the buffer's backend | — |
| completion and describe from the live image | the protocol's `complete` and `describe` | in `serve.scm` |
| interrupt of a runaway | a second connection (`:interrupt` is out of band) | in `serve.scm` |
| highlighting, indentation, structural editing | `language/guile.rs`, over Scheme's scanner | 75 lines |
| read marks | brackets from the editor; the rest from Guile's own reader (2026-11-17) | `readcheck.rs`, 15-guile |
| start, stop, mode, detection | `15-guile.scm` | ~130 lines |

**Not at Guile's level, and so not in this plan's baseline:** scm's
warnings (a name nothing binds, a wrong count of arguments) — though SBCL
turns out to give them for free, which is L6 — and SLIME's inspector and
who-calls. **The debugger IS in the plan** (L7): the first draft left it
out, and Kevin asked why — *"it's built into the sbcl repl"* — which is
right, and the answer is below.

## L0 as measured (2026-11-17)

SBCL 2.6.4 on this machine, by a probe script. Each line is something the
stages below lean on.

| question | answer |
|---|---|
| threads, sockets, introspection | `:sb-thread` is a feature; `sb-bsd-sockets` and `sb-introspect` load with `require`. No library from outside SBCL is needed. |
| start-up | 0.33 s with `~/.sbclrc` (which here loads Quicklisp), 0.02 s with `--no-userinit`. |
| eval, values, output | `(multiple-value-list (eval form))` with `*standard-output*` rebound gives values and what was printed, separately; `(values 1 2)` prints as `1, 2`. |
| an error | a condition, caught by `handler-case`; `princ` of it is SBCL's message (`The value 5 is not of type LIST…`), what was printed before it is kept; `sb-debug:print-backtrace` inside a `handler-bind` gives the calls. |
| completion | `apropos-list` and the package's symbols; symbols are upper case inside and are shown lower case. |
| describe | `(documentation 'sq 'function)` → the docstring; `sb-introspect:function-lambda-list` → `(X)`, and for `mapcar`, `(FUNCTION LIST &REST MORE-LISTS)`. |
| where a function is defined | `sb-introspect:find-definition-source` → the file. (For `M-.`, L5.) |
| interrupt | `sb-thread:interrupt-thread` with a function that signals: a thread in `(loop)` stopped and reported `interrupted`. |
| the reader's verdict, with a place | `reader-error` and `end-of-file` are distinct conditions, and `file-position` of the string stream is where it stopped. Seen: an unmatched `)`, `Package FOO does not exist`, `Symbol "X" not found in the COMMON-LISP package`, `unrecognized character name`, `dot context error`, `illegal sharp macro character`; `end-of-file` for an open list, an open string and an open `#|`. `#+`/`#-`, `\|a b\|`, `#p""`, `#'`, `#(`, `#:x`, `1/2` read. |
| **reading interns** | Reading `(zzprobe-new-symbol 1)` in `CL-USER` left that symbol in `CL-USER`. Read in a temporary package with the same use-list, it did not — and `car` still read. So the read check reads in a throwaway package. |
| **`#.` and the read check** | With `*read-eval*` nil — which a check that must not RUN anything needs — `#.(+ 1 2)` is a reader error ("can't read #. while \*READ-EVAL\* is NIL"). That one message is not the user's mistake and is not marked. |
| **warnings as data** | Compiling `(lambda (x) (undefined-fn-zz x) (car 1 2))` signalled two `warning`s that a handler collected: `The function CAR is called with two arguments, but wants exactly one.` and `undefined function: UNDEFINED-FN-ZZ`. SBCL's `eval` compiles, so a send of a `defun` can report these with no extra pass. |

### The debugger, measured

SBCL's own debugger is a loop on the terminal: it prints the restarts and
reads a choice from standard input. A server has no terminal — started as
this one is, the debugger would wait for a line that never comes and the
send would hang. That is the only reason it does not work "for free", and
it is not a reason to go without: everything the debugger offers is
available to the program that hit the error, **before** the stack unwinds.
Probed (2026-11-17), with the next requests standing in for typed lines:

| case | what happened |
|---|---|
| `(* 2 unbound-thing-zz)`; then evaluate `(+ 1 2)` in the break; then restart `USE-VALUE` with 42 | the restarts listed (`CONTINUE`, `USE-VALUE`, `STORE-VALUE`, ours, SBCL's); `=> 3` while the error stood open; then the ORIGINAL send finished, answering 84 |
| `(car 5)`; then abort | the send answers as given up; the server is at top level |
| `(cerror "Go on anyway." …)`; then `CONTINUE` | the send went on and answered its value |
| `(break "look here")` — which calls the debugger without signalling | caught by `sb-ext:*invoke-debugger-hook*`; `CONTINUE` returned from it |
| an error INSIDE the break | a second level, with its own restarts; aborting it and then the first unwound both |
| the frames, and a local | `sb-di` walked the stack to a function compiled with `(debug 3)` and read its variables: `((DOUBLED 42) (SECRET 21))` |

So the debugger is a **break loop inside the request**: a `handler-bind`
(and the debugger hook, for `break`) that, instead of unwinding, answers
the send with the error, its restarts and its frames, and then serves the
connection's next requests from inside the error — an expression
evaluates there, a number invokes that restart, and the send that broke
either finishes with a value or is given up. It is the built-in debugger's
behaviour, with the REPL panel as its terminal.

## The shape

    edit/sbcl/serve.lisp        the server: scm's REPL protocol, in Common Lisp
    edit/src/language/lisp.rs   highlighting, indentation, bracket marks
    edit/scm/27-lisp.scm        start/stop, the backend, the mode, detection
    scm_binding/readcheck.rs    (as built for Guile) the reader's marks

**The protocol does not change.** `scm --serve`, the Emacs mode and Guile's
`serve.scm` are untouched, and the Rust client (`scm_binding/remote.rs`)
needs nothing new. What Common Lisp needs beyond the six requests —
evaluating in a buffer's PACKAGE, completing and describing there, reading
without evaluating — is asked for as an ordinary `eval` of a call into the
server's own package, the way the Guile read check is an `eval` of a form
that only reads:

    (:op eval :id 7 :code "(edit-serve:in \"MY-PKG\" \"(defun sq (x) (* x x))\")")
    (:op eval :id 8 :code "(edit-serve:read-check \"…buffer text…\" \"MY-PKG\")")

The answer is a value like any other, which the shell reads.

**The buffer's package is the buffer's**, as in SLIME: a send from a file
evaluates in the package of the nearest `(in-package …)` above the point,
found by the shell from the text. The REPL panel keeps its own, changed by
typing `(in-package …)` at it. A package that does not exist yet falls
back to `CL-USER` and the answer says so.

**The language is `lisp`, the backend is SBCL.** Nothing in the language
file is SBCL's; everything in `serve.lisp` may be. A second implementation
would be a second server script, not a second language.

## Stages

Each leaves the suite green; the tests that need SBCL skip without it, as
Guile's do without Guile.

| | what | exit criterion |
|---|---|---|
| **L0** ✔ | Measure SBCL (above). | this table |
| **L1** ✔ | **The server.** `edit/sbcl/serve.lisp`: a listener on the address it is given, a thread for each connection; `eval` (every form of the text, in order; the last one's values; what was printed; warnings collected and returned with the output), an error as its message with the calls it raised in, `complete`, `describe`, `parse`, `ping`, and `interrupt` from a second connection. The debugger never opens: `sb-ext:*invoke-debugger-hook*` answers an error instead. `*standard-input*` is empty, so a `(read-line)` ends rather than hangs. | the protocol's own conformance, driven from Rust as `remote.rs`'s tests drive scm and Guile: a value, output with a failure, a multi-form send, completion of a name just defined, describe with docstring and lambda list, an interrupt of `(loop)` that leaves the server up |
| **L2** ✔ | **The language.** `language/lisp.rs`: Scheme's scanner with Common Lisp's differences — `#\|…\|#`, `#\\Space`, `\|a symbol\|`, `#'f`, `#+x`/`#-x`, `:keyword` and `#:uninterned`, `pkg:name`; indentation by a table of body forms (`defun`, `let`, `when`, `flet`, `loop`…); the bracket marks Guile's has. Structural editing allowed for it. | a file of awkward cases highlights and indents as expected; a stray `)` and an open form are marked with no SBCL running |
| **L3** ✔ | **The shell.** `27-lisp.scm`: `sbcl-start`/`-stop`, `lisp-mode`, detection (`.lisp`, `.lsp`, `.cl`, `.asd`), the backend (the remote backend, with `:complete`, `:doc`, `:signature` asking in the buffer's package), the running-process hook so TAB asks SBCL, the buffer's package from its `in-package`, the REPL panel's prompt (`CL-USER> `, following the REPL's package) and its own "is this whole?" (brackets — scm's reader does not read `#'f`). | against a real SBCL: each send key, `=> 49`, `=> SQ` for a `defun` (which Common Lisp answers itself), an error's message, describe on a name and on an argument, TAB in the buffer and at the REPL offering a function just sent, a two-package file sending each form to its own package |
| **L4** | **Read marks.** SBCL's reader on the buffer's text, through `readcheck.rs` as built: `edit-serve:read-check` reads in a throwaway package with the buffer's use-list and `*read-eval*` nil, and answers where it stopped. | `#\\notachar` and `(foo:bar)` are marked on their line in SBCL's words; `#.(…)` is not; mending clears the mark; reading leaves no symbol behind in the user's package |
| **L5** | **`M-.`.** Ask SBCL where a name is defined (`sb-introspect`), falling back on the editor's own index of `defun`/`defmacro`/`defvar` forms for a file never loaded. | `M-.` on a call jumps to its `defun` in another file, loaded or not; `M-,` comes back |
| **L6** | **Warnings — past Guile's level.** What SBCL's compiler says of a form when it is sent (an undefined function, a wrong count of arguments, an unused variable) shown under the send, and marked in the buffer at the form. | sending a `defun` that calls a function that does not exist says so, and the form is marked until it is sent again clean |

| **L7** ✔ | **The debugger.** An error does not unwind: the send is answered with the message, the restarts numbered and the frames, and the connection is in a BREAK — the REPL panel's prompt says so (`CL-USER[1]> `, SBCL's own `0]` counting levels). At it: a number takes that restart (one that wants a value asks for it), an expression evaluates inside the error, `:abort` gives the send up, `:top` gives up every level. The error pane (`C-h e`) shows the frames, with the locals of code compiled with `(debug 2)` or more. A send from a buffer while in a break evaluates in the break too. | the probe's six cases through the editor: `USE-VALUE` finishes the original send with its value; `CONTINUE` after a `cerror`; `(break)`; an error inside a break and out of both; a frame's locals in the error pane; and Escape/interrupt of a runaway lands in a break rather than losing the work |

L1–L4 are "the same level as Guile". L5 is what Guile gets from the
editor's Scheme index and Common Lisp does not. L6 and L7 are where SBCL
gives more than Guile does — and L7 is the part of Common Lisp that people
who use it would miss first.

Sizes, from Guile's: the server about 350 lines (packages, lambda lists and
the debugger hook are what Guile's did not need); the language about 200;
the shell about 200; tests and this document's "as built" sections beside
them.

## L1 as built (2026-11-17)

`edit/sbcl/serve.lisp`, 460 lines, loaded as

    sbcl --noinform --disable-debugger --load serve.lisp --end-toplevel-options 127.0.0.1:PORT

and listening in 0.1 s without an init file. It answers every request the
protocol has, as `edit/guile/serve.scm` does, a thread per connection.

- **Requests are read by a reader of the server's own**, not Common
  Lisp's: the wire spells strings as Scheme does (`\n`, `\t`), which CL's
  reader would take as `n` and `t`. It reads only the plist a request is,
  and interns nothing a client sends.
- **`eval`** reads the text's forms one at a time in the connection's
  `*package*` and evaluates each; the answer is the last one's values,
  printed and joined with `, ` (`; No values` for none), and everything
  printed to standard, error or trace output. `*`, `**`, `/`, `+` and the
  rest are kept as SBCL's REPL keeps them. `(in-package …)` lasts for the
  connection (the REPL panel's), not for a buffer's next send — the
  buffer's own package is L3's.
- **The compiler's warnings** come back in the output, one line each
  (`; warning: The function CAR is called with two arguments…`,
  `; style-warning: undefined function: …`), and as a `:warnings` list,
  which the client ignores today and L6 will read.
- **An error** answers with its report (the reader's `Stream: …` paragraph
  taken off) and the calls it was raised in, from the one that signalled
  outwards, with the server's frames, `EVAL`'s and the signalling
  machinery's taken out: `(INNER 5)`, `(OUTER 4)`. A tail call leaves no
  frame — SBCL's, not the server's. A top-level form `eval` compiled into
  an anonymous function shows as the form, so an interrupted `(loop)` is
  `(LOOP)`. A send whose text does not read answers what is wrong with the
  text and no calls.
- **The debugger never opens.** `sb-ext:*invoke-debugger-hook*` and
  `*debugger-hook*` answer instead, so `(break "look here")` comes back
  as a failure with its message. Standard input is an empty stream, so
  `(read-line)` ends with an end-of-file error instead of waiting for a
  terminal. (L7 replaced the first with a break served over the
  connection: below.)
- **`interrupt`**, from a second connection, interrupts every thread
  evaluating a request: `host-stopped`, message `interrupted`, and the
  calls where it was. The server stays up.
- **`complete`** offers the symbols the package can see that name
  something (a function, macro, variable, class or special operator),
  lower case; `pkg:` completes a package's external symbols and `pkg::`
  all of its own; package names complete as `pkg:`. A name with lower case
  letters of its own comes back `|Like This|`. Keywords are not offered.
- **`describe`** takes `foo`, `pkg:foo` or `|Mixed|` without interning:
  kind (`procedure`, `generic function`, `macro`, `special operator`,
  `variable`, `constant`, `class`), the value, the docstring, and the
  signature from `sb-introspect`, lower case and without package prefixes:
  `(mapcar function list &rest more-lists)`.
- **`parse`** reads in a scratch package with the same use-list, with
  `#.` reading its form and evaluating nothing; a name in a package that
  does not exist yet is complete text, not invalid.
- `ping` and `stream` answer `sbcl 2.6.4`.

Tests (`remote.rs`, `sbcl_tests`, skipped without `sbcl`):
`sbcl_answers_the_protocol` (every case above, through `remote.rs`'s own
`start` and `ask`) and `an_interrupt_stops_sbcl_and_it_stays_up`.
`compile-file` of the server is free of warnings.

## L2 as built (2026-11-17)

`edit/src/language/lisp.rs`, the language `lisp` ("Lisp" on the modeline).

- **Its own tokens.** scm-text reads Scheme, and probing it on Common Lisp
  found three places where that moves brackets: `|a (b|` split at the
  space (and opened a list), `[ ] { }` taken as brackets when CL reads them
  as letters, and `#'f` read as a stray `#` and a quote. So `lisp::tokens`
  splits the text as CL's reader does — `|…|` and `\x` escapes in symbols,
  nesting `#|…|#`, `#\Space`, and `#'`, `#.`, `#+`, `#-`, `#p`, `#c`,
  `#2A`, `#S`, `#1=` and a vector's `#` as PREFIXES of the datum after
  them — into scm-text's own `Token`s. Everything downstream takes them
  as it takes Scheme's.
- **Shared with Scheme, not copied.** `scheme.rs`'s placement, clean line
  starts, scan and incremental re-scan became `Tokens::of` (with a
  `Dialect`), `scan_tokens` and `rescan_with`; Scheme and Lisp both call
  them, and only an atom's colour asks the dialect. The re-scan is checked
  against a full scan over random edits for Lisp as it is for Scheme.
- **Colours**: `:key`, `#:sym` and `&key` as keywords; `t`, `nil`, numbers
  (`1/2`, `2.5d0`, `#x1F`, not `1+`) and characters as literals; a feature
  expression (`#+sbcl`) as a keyword; the name after `#'` as a call; a
  form's head as a call, except a quoted list's or a vector's or an
  array's.
- **Indentation is Emacs's Common Lisp's**: a body form (`defun`, `let`,
  `when`, `handler-case`, anything `def…`, `with-…` or `do-…`) indents its
  body two in; a call's arguments line up under the first when the first
  is on the head's line, and under the head when not; `if` lines up its
  arguments, as CL style has it; a local function in `flet`, `labels` or
  `macrolet` indents its body two in from its `(f`; data (`'(…)`, `#(…)`)
  lines up under its first element.
- **Bracket marks** need no SBCL: Guile's check, now `guile::bracket_marks`
  and shared. A stray `)` is an error where it stands; a form left open is
  marked quietly at its outermost `(`.
- **Structural editing** (the `C-M-` keys, paredit's, pairing, vi's `%`)
  is on for Lisp, over Lisp's tokens (`Sexps::with_tokens`): `|a (b|`,
  `#'g` and `#(1 2)` each move as one expression. `[` and `{` are not
  paired in Lisp, being letters there. Rename stays Scheme's — its scope
  analysis is Scheme's binding forms.
- The agent's replies colour a ```` ```lisp ````, ```` ```common-lisp ````
  or ```` ```cl ```` block as a Lisp buffer is coloured.

Not yet: nothing opens a `.lisp` file as Lisp, and nothing sends — that is
L3's.

## L3 as built (2026-11-17)

`edit/scm/27-lisp.scm`, four prims over the language (`scm_binding/lisp.rs`),
and three entry points in the server.

- **A `.lisp`, `.lsp`, `.cl` or `.asd` file opens as Lisp**; `M-x lisp-mode`
  sets any buffer so. Its sends go to an SBCL the editor starts on the
  first one (`M-x sbcl-start`, `sbcl-stop`, `sbcl-restart`;
  `(remote-start "sbcl")` too), running `edit/sbcl/serve.lisp` — compiled
  in, like Guile's server, for a binary whose tree is gone. With no SBCL a
  send FAILS, saying so; it never falls through to scm's image.
- **D1: `~/.sbclrc` is loaded**, so Quicklisp is there: the server listens
  in 0.22 s with this machine's (0.1 s without). `(set! *sbcl-init* #f)`
  in `init.scm` starts it bare; the tests do.
- **What a send reads** is Lisp's (`define-language-forms!`): C-M-x the
  top-level form at the point, else the nearest before it; C-x C-e the
  expression before the point — both off Lisp's tokens, so `|a (b|` is one
  symbol and `#+sbcl (f)` one form. C-c C-r the selection; C-c C-b the
  buffer.
- **D2, packages, as SLIME has them.** A send from a buffer is read and
  evaluated in the package of the last `(in-package …)` above the point
  (`%lisp-package`, off the tokens): the shell wraps it as
  `(edit-serve:in "SHAPES" "…")` (08-sends' new `*language-send-wrap*`),
  and its values print as that package writes them — `(1 G)`, not
  `(1 SHAPES::G)`. A whole buffer or file starts in CL-USER and its own
  `in-package` forms take effect as they run. A package that does not exist
  yet falls back to CL-USER and the output says so. A `defun` answers
  `=> SQ` by itself.
- **SBCL's reports come on one line** (the server folds them): `error: The
  value 5 is not of type LIST when binding LIST`, where the echo line had
  shown only "The value".
- **Completion and describe ask in the buffer's package** (`complete-in`,
  `describe-in`, through an ordinary eval) — C-c C-d d on a name or on an
  argument of its call shows the docstring and echoes `(sq x)`. TAB in the
  buffer and at the REPL offers SBCL's names once it runs. Before then a
  Lisp buffer offers its own words — hyphenated names whole (`buffer-words`
  takes a Lisp's word characters now) — and no signatures: scm's names
  mean nothing in Common Lisp.
- **The REPL panel** evaluates in the REPL's own package, `CL-USER> ` to
  start; `(in-package :shapes)` typed there makes it `SHAPES> ` (the
  backend's new `:prompt`). A line whose brackets are still open waits for
  the rest (`%lisp-complete?`, Lisp's tokens: scm's reader does not read
  `#'f`).
- `M-x remote-interrupt` stops a runaway send, from a second connection.

Tests (`remote.rs`, `sbcl_tests`): `a_lisp_file_is_lisps_and_never_runs_as_scm`
(always) and `a_lisp_buffer_sends_to_sbcl` (with `sbcl`): every send key in a
two-package file, an error, describe both ways, TAB, the REPL's prompt
following its package, a line continued. `language::lisp`'s
`what_a_send_reads` and `whole_text` hold the cutting.

## L7 as built (2026-11-17)

The server (`serve.lisp`, about 130 lines more), `27-lisp.scm`'s prompt and
restart list, and three keys the Rust client passes through.

- **An error is a break**, in the server, without SBCL's debugger: the
  handler answers the request — the message, the calls, each call's locals
  (`:locals`, beside `:backtrace`), the restarts (`:restarts`, innermost
  first), the level (`:level`) — and then serves the connection's next
  requests from inside the handler, a level deeper. Whatever finally
  finishes answers the request read LAST (`*owed*`), so a USE-VALUE's line
  is answered with the value of the send it broke. `cerror`, `break`,
  `invoke-debugger` and `signal`ed errors all come the same way. Stack
  exhaustion still unwinds: there is no room to serve a break from.
- **At a break** a line that is a number takes that restart; one that
  wants a value (`USE-VALUE`, `STORE-VALUE` — SBCL marks them with an
  interactive function) answers `:asking` and reads the next line as the
  value's form, or takes it on the same line, `1 (* 6 7)`. `:abort` (`:a`,
  `:q`) invokes the innermost `ABORT`, `:top` the outermost. Each send,
  and each line inside a break, has an `ABORT` of its own ("Give this send
  up.", "Return to break level 1."), so SBCL's list reads as its own REPL's
  does. Anything else evaluates inside the error, in its dynamic
  environment — and a send from a buffer while in a break does too, as the
  plan said, since it is the same connection.
- **Every eval answer says the package** the connection reads in
  (`:package`, its shortest name), and the shell's prompt is
  `PACKAGE> `, `PACKAGE[N]> ` N breaks down, or `USE-VALUE value> ` while a
  restart waits. L3's separate "which package now?" request after an
  `in-package` is gone.
- **The restarts are listed** under the error in the transcript, numbered,
  with a line saying what a number, `:abort` and anything else do; the
  echo line says the send is in a break. `C-h e` shows each call with its
  locals — whatever SBCL kept, which `(debug 2)` makes more of (`x = 4`).
  The error pane's "the send has already stopped" became "a snapshot,
  taken when it raised": an SBCL send is still live, at the REPL.
- **An interrupt is a break too** (`M-x remote-interrupt`; the ask
  timeout's interrupt): `CONTINUE` ("Go on with the evaluation.") resumes
  the runaway where it was, and anything evaluated first sees its work so
  far. A thread waiting for its next request inside a break is not
  interrupted.
- **Backtraces inside a break.** SBCL's backtrace starts by default at the
  newest frame a trap interrupted — inside a `(car 5)` break, that one,
  whatever signalled since — so every later error showed `(CAR 5)`. Each
  request binds `*stack-top-hint*` to nil and the walk starts at the hint
  resolved, which is the signalling call's frame.
- **The client** (`remote.rs`) keeps each answer's text beside the parsed
  `Response` and reads the keys scm's protocol does not have from it
  (`extra_keys`: `level`, `restarts`, `locals`, `asking`, `package`), so scm's
  `Response` and its tests are untouched. `remote-result` (14-remote) passes
  them on; Guile and scm send none.
- `edit-serve:*break-on-error*`, nil, gives L1's behaviour back: an error
  answers and unwinds.

Not done: a sync question the editor asks itself (completion's
`complete-in` through `%remote-eval`, 5 s) that times out would leave the
connection in a break nobody listed — the next send's answer says the
level, so the prompt catches up, but no restarts are shown for it. Stepping
and the inspector stay out (D6).

Tests (`remote.rs`, `sbcl_tests`): `a_lisp_error_is_a_break` — the six
cases through the editor: a call's locals in the error pane, evaluating
inside a break and `:abort`, USE-VALUE asking for its value and finishing
the original send (`=> 43`), CONTINUE after `cerror`, out of `(break)`, two
breaks deep and out of both, and an interrupted `(loop (incf *n*))` looked
at (`*n*` > 0), resumed, interrupted again and given up with `:top`.
`a_lisp_buffer_sends_to_sbcl` now finds itself two breaks down after its
two errors, and leaves with `:top`.

## Risks

- **A break holds the connection.** While an error stands open the
  connection is inside it, and so is whatever the buffer sends next — which
  is what a debugger is, and is a surprise if the prompt does not say so
  plainly. The prompt, the modeline and the echo all have to.
- **A break left open forever.** Nothing times it out; `:abort`, `:top`, a
  restart or `sbcl-stop` end it. The agent's `eval` tool must never be left
  in one: its call gives the send up if it breaks, and says what the
  restarts were.
- **An interrupt is not safe at every point.** `interrupt-thread` runs a
  function wherever the thread happens to be, and unwinding from there can
  leave a lock held or a structure half-updated. SLIME lives with the same.
  The server interrupts only the thread that is evaluating a request.
- **The user's `.sbclrc` runs.** With it (D1) the REPL has Quicklisp and
  whatever else the user loads, and anything it prints or breaks is the
  server's start-up. The server is given no terminal, so an init file that
  asks a question gets end-of-file.
- **Output from other threads is lost.** `*standard-output*` is rebound for
  the thread evaluating the request; a thread the user's code starts
  prints to the process's own, which nothing reads.
- **The read check sees the image as it is.** A file's `(foo:bar)` is
  marked until package `FOO` exists — which is right before the file is
  loaded and noise if the user reads it that way. It clears on the first
  check after the `defpackage` is sent.
- **The reader is programmable.** A file that installs reader macros reads
  correctly only after that part of it has been sent. Same remark.
- **Symbol case.** Names are shown lower case and looked up case-insensitively
  for an ordinary symbol; a `|Mixed Case|` one must survive the round trip.
- **`defined-name` and friends are Scheme's.** The shell has places that
  assume `(define …)`; Common Lisp mostly does not need them (a `defun`
  answers its own name), but each such place has to be checked, not
  assumed.

## Decisions for Kevin

- **D1 — does the server load `~/.sbclrc`?** With it, `(ql:quickload …)`
  works at the REPL as it does in a terminal, for 0.3 s at start here.
  Without it the server is the same on every machine and starts in 0.02 s.
  *Recommend loading it* — an SBCL without the user's Quicklisp is not the
  SBCL they use — with `*sbcl-args*` there to say otherwise.
- **D2 — packages in the first cut?** Sending every form to `CL-USER` is
  simpler and wrong for any file that begins `(in-package …)`.
  *Recommend doing it in L3*, since the server needs the package-taking
  entry points anyway for completion and the read check.
- **D3 — what is the language called?** `lisp` (the mode `lisp-mode`, the
  modeline "Lisp"), with the commands named for the program (`sbcl-start`).
  *Recommended.* The alternative, `commonlisp`, is more exact and longer.
- **D4 — which files are Common Lisp?** `.lisp`, `.lsp`, `.cl`, `.asd`.
  *Recommended.* A `.scm` is never Common Lisp; `M-x lisp-mode` is there
  for anything else.
- **D5 — how far past Guile?** L5 (`M-.`) and L6 (compiler warnings) are
  each a few hours and each is something a Lisp programmer will reach for
  on the first day. *Recommend both*, after L1–L4 are in use.
- **D6 — the debugger: IN (Kevin, 2026-11-17).** The first draft
  recommended leaving it out, as a design of its own. The probe says it is
  half a day, not a design problem: SBCL hands the program everything its
  debugger shows, and a break is the connection's next requests served
  from inside the error. What is still a choice is the ORDER: L7 can come
  straight after L3, before the read marks and `M-.`, if the debugger is
  what you want to use first. *Recommend L1–L3, then L7, then L4–L6.*
  Two things stay out: stepping (SBCL's stepper is its own protocol), and
  SLIME's inspector.
