# S43: breaks kept, for a client over the wire

**Done 2026-11-22.** The REPL protocol section is `docs/repl.md`, "Breaks
kept"; edit's side is `docs/editor-crate-plan.md`, E3f.

## Why

The trial of lux against cl-wgpu (cl-wgpu `docs/trial/README.md`) found
SBCL's debugger the one real advantage of the Lisp edition. scm has had a
break REPL since S8b -- frames live, locals, evaluate in a frame, return a
value -- but only in the terminal and in edit's own process. Over a socket
an error unwound and reported, so lux, whose errors happen in a frame on
its render thread, showed a backtrace and nothing of what the failing code
could see.

## What

- `Interp::keep_breaks` (the VM's `keep_breaks`): a built-in break
  handler. It keeps the first break of each host entry -- the innermost
  evaluation's -- recording each call as the backtrace has them (32, then
  "…"), its locals printed (record printers and all, S42), and its
  environment rooted; then it aborts. The previous break is let go.
  `Session::new` installs it.
- `Interp::last_break` (and `current`, whether it was the latest entry's)
  and `Interp::eval_in_break` / `eval_in_break_capturing`.
- The protocol: a failed eval's answer carries `:locals`, one list a call
  of `:backtrace`; `(:op eval :frame K :code …)` is `Op::EvalIn`.
  `Outcome::Failed` gains `locals`.
- edit: `%remote-eval-in`; `14-remote` reads `:locals` from answers and
  from a host's `paused` event and keeps the connection (`*break-conn*`);
  `M-x eval-in-frame`; the error pane says when it can.
- lux sends `:locals` with `paused` (its own repository).

A client's evaluation in a frame, and a record printer failing while an
answer is printed, run with the handler set aside, so neither replaces
the break being looked at.

## Not done

**Continuing** (`,return`): the calls have unwound by the time a client
asks. That needs a break that waits, which a server on one thread -- lux
-- cannot do without freezing; a VM off the host's thread is the way, and
it is a larger change. Breakpoints and stepping need a VM hook, as before.

## Tests

- `scm/tests/repl.rs`: locals with the answer, aligned to the backtrace;
  evaluating in a frame, its output captured; the break outliving other
  sends; an error in a frame's evaluation not replacing it; a new error
  replacing it; no frame there; a caught error keeping nothing; a record
  local printed by its printer; and the wire round trip of `:locals` and
  `eval :frame`.
- `edit`, `a_remote_error_keeps_its_calls_locals_to_evaluate_in`: a remote
  send's frames reach the error pane with their locals, and
  `eval-in-frame` answers in the transcript.
- lux `tests/live.rs`, `a_paused_frames_calls_can_be_looked_into`.
