# `scm-dap` — the break REPL, with a user interface

2026-10-22, stage S24. `scm-dap`, stdio Debug Adapter Protocol.

S8b's break REPL already does the hard part: an error nothing will catch
stops **with the frames live**, and the handler may read a frame's
bindings, evaluate in its environment, then abort or continue with a value
in place of the expression that failed. All of that has only ever had a
terminal prompt. This is the same thing over DAP, so an editor can show the
stack and the locals — which `scm-mode.el` does not do either.

## What it is not, said plainly

**There are no breakpoints and no stepping.** The VM has no hook for
either: `set_break_handler` fires on an uncaught error and that is the
whole of it. So:

- the capabilities advertise neither;
- `setBreakpoints` answers `verified: false` with a message, so a client
  marks the breakpoint unmet instead of waiting for a stop that cannot
  come;
- `next` / `stepIn` / `stepOut` / `pause` are **refused** with "scm cannot
  step: the break is an error, not a pause".

A debugger that accepted a breakpoint and silently ignored it would be
worse than one that admits it has none.

**It cannot attach**, and that is structural rather than unfinished: a
break handler runs inside the failing evaluation, so the adapter has to be
the process that owns the `Vm`. This is the one place the editor tooling
does *not* go through `--serve` — `scm-lsp` is a client of a running
session, and this is not, because a socket carries values and not a live
stack.

## A tail call leaves no frame, and a debugger makes that visible

§5's whole point, arriving where a user will meet it. This program stops
with **one** frame:

```scheme
(define (inner n) (car n))
(define (outer k) (inner k))
(outer 5)
```

Every call in it is in tail position, so the stack is exactly as deep as it
started and `outer`'s `k` **cannot be inspected — its frame no longer
exists.** The one frame shown is the innermost call that was *entered*,
which the VM tracks separately precisely so an all-tail-call failure still
says where it broke.

Make one call non-tail and the frames appear:

```scheme
(define (outer k) (* 1 (inner k)))     ; now `k` is inspectable
```

This is correct Scheme and a thin stack, not a bug, and it is recorded as a
test (`a_tail_call_leaves_no_frame_to_select`) because the first version of
the test beside it assumed the frame would be there. A user will assume the
same thing.

## `continue` unwinds; `,return` resumes

DAP assumes a debugger that can resume. Here the error has already
happened, so there is nothing to resume *to* — the choice the terminal
prompt offers is abort or return-a-value, and the adapter offers the same
two under DAP's vocabulary:

| action | what happens |
|---|---|
| `continue` | `BreakAction::Abort` — unwind as if there had been no handler |
| `,return EXPR` in the Debug Console | `BreakAction::Return(v)` — the failed expression produces `v` and the program **carries on** |
| `,abort` in the Debug Console | the same as `continue` |

The comma spellings are the terminal break REPL's own, so one vocabulary
serves both rather than a second one being invented for the same two
actions. The test for `,return` asserts the program's *later* output: a
`,return 21` into `(display (* 2 (half)))` must print 42, which only
happens if execution really continued.

## Threads, and a deadlock worth not repeating

The `Vm` is `Rc`-based and stays on one thread, so stdin is read on a
second thread that only ever forwards. Requests land in **one** queue with
two readers — the adapter loop, and the break handler while it is stopped.
They never run at the same time, because the handler runs only while the
adapter's thread is inside the VM, so the mutex costs nothing and there is
exactly one place a request can be taken from.

The first version of this file gave the handler its own pair of channels.
That deadlocks: nothing drains a reply channel while the thread that would
drain it is blocked inside the very evaluation the handler is describing.

## Output

The program's output cannot go to stdout, because stdout is the protocol,
so it is captured (`eval_capturing_named`) and sent as `output` events.

**This used to be the adapter's worst flaw, and fixing it was the strongest
argument for S25.** The capture returns its string when the evaluation
*ends*, and the break handler fires **inside** that evaluation — so at a
break the Debug Console held nothing of what the program had printed.
Measured then, on a program that displays twice and then calls `(car '())`:

    => output events delivered by the time we are STOPPED: 0
    => output arriving only AFTER continue: "IMPORTANT CLUE\nstep 2 done\n…"

The clue arrived after the user had already chosen between `,return` and
abort — which is exactly the decision the clue was for.

**Fixed by S25**, which is a sink rather than anything rearranged here: the
adapter installs `Interp::set_output_sink` for the run and forwards each
piece as an `output` event the moment it is written, so the console has the
program's narration *at* the break.
`the_console_has_the_programs_output_at_the_break` is the test, and it
asserts on the state at the `stopped` event rather than after continuing —
which is the distinction the old behaviour passed every other test by
blurring.

The capture is still taken as well, because `:output` arriving with the
answer is the protocol's guarantee and a future client may want it whole.

## Locations

`FrameView::loc` is pre-formatted as `file:line:column`, so it is split
from the right — a path may contain colons, a line and a column may not. A
frame whose location does not parse, or a form built at run time that never
had one, is **named but not placed**: pointing at a line that is a guess
would be worse than leaving it blank, which is the rule the backtrace
already follows.
