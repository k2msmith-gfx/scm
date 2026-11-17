# Common Lisp in `edit`, through SBCL — plan, 2026-11-17

**PROPOSED, nothing built.** L0 (the measurements) is done and is below;
decisions D1–D6 at the end are Kevin's.

Kevin, 2026-11-17: *"what is the level of effort to support sbcl to the
same level as guile or scm?"* — and, after the Guile reader check landed:
*"then we can plan for sbcl."*

So: a `.lisp` buffer in `edit` whose sends go to an SBCL the editor starts,
with the same keys, transcript, REPL panel, completion, describe and read
marks a Guile buffer has. **About a day of work**, in the stages below.

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
turns out to give them for free, which is L6 — and anything SLIME has
beyond a REPL: the debugger with restarts, the inspector, who-calls.

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
| **L1** | **The server.** `edit/sbcl/serve.lisp`: a listener on the address it is given, a thread for each connection; `eval` (every form of the text, in order; the last one's values; what was printed; warnings collected and returned with the output), an error as its message with the calls it raised in, `complete`, `describe`, `parse`, `ping`, and `interrupt` from a second connection. The debugger never opens: `sb-ext:*invoke-debugger-hook*` answers an error instead. `*standard-input*` is empty, so a `(read-line)` ends rather than hangs. | the protocol's own conformance, driven from Rust as `remote.rs`'s tests drive scm and Guile: a value, output with a failure, a multi-form send, completion of a name just defined, describe with docstring and lambda list, an interrupt of `(loop)` that leaves the server up |
| **L2** | **The language.** `language/lisp.rs`: Scheme's scanner with Common Lisp's differences — `#\|…\|#`, `#\\Space`, `\|a symbol\|`, `#'f`, `#+x`/`#-x`, `:keyword` and `#:uninterned`, `pkg:name`; indentation by a table of body forms (`defun`, `let`, `when`, `flet`, `loop`…); the bracket marks Guile's has. Structural editing allowed for it. | a file of awkward cases highlights and indents as expected; a stray `)` and an open form are marked with no SBCL running |
| **L3** | **The shell.** `27-lisp.scm`: `sbcl-start`/`-stop`, `lisp-mode`, detection (`.lisp`, `.lsp`, `.cl`, `.asd`), the backend (the remote backend, with `:complete`, `:doc`, `:signature` asking in the buffer's package), the running-process hook so TAB asks SBCL, the buffer's package from its `in-package`, the REPL panel's prompt (`CL-USER> `, following the REPL's package) and its own "is this whole?" (brackets — scm's reader does not read `#'f`). | against a real SBCL: each send key, `=> 49`, `=> SQ` for a `defun` (which Common Lisp answers itself), an error's message, describe on a name and on an argument, TAB in the buffer and at the REPL offering a function just sent, a two-package file sending each form to its own package |
| **L4** | **Read marks.** SBCL's reader on the buffer's text, through `readcheck.rs` as built: `edit-serve:read-check` reads in a throwaway package with the buffer's use-list and `*read-eval*` nil, and answers where it stopped. | `#\\notachar` and `(foo:bar)` are marked on their line in SBCL's words; `#.(…)` is not; mending clears the mark; reading leaves no symbol behind in the user's package |
| **L5** | **`M-.`.** Ask SBCL where a name is defined (`sb-introspect`), falling back on the editor's own index of `defun`/`defmacro`/`defvar` forms for a file never loaded. | `M-.` on a call jumps to its `defun` in another file, loaded or not; `M-,` comes back |
| **L6** | **Warnings — past Guile's level.** What SBCL's compiler says of a form when it is sent (an undefined function, a wrong count of arguments, an unused variable) shown under the send, and marked in the buffer at the form. | sending a `defun` that calls a function that does not exist says so, and the form is marked until it is sent again clean |

L1–L4 are "the same level as Guile". L5 is what Guile gets from the
editor's Scheme index and Common Lisp does not. L6 is where SBCL gives more
than Guile does.

Sizes, from Guile's: the server about 350 lines (packages, lambda lists and
the debugger hook are what Guile's did not need); the language about 200;
the shell about 200; tests and this document's "as built" sections beside
them.

## Risks

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
- **D6 — the debugger.** SLIME's defining feature is the debugger with
  restarts; this plan answers an error with its message and its calls and
  goes no further, as for scm and Guile. *Recommend leaving it out* until
  the rest has been used: it is a design of its own (a break is a second
  REPL inside the first), and scm's own break REPL (repl.md, S8b) is the
  place to start that design from.
