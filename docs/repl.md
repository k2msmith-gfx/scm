# The REPL protocol

2026-10-12, stage S8. `scm --serve 127.0.0.1:7000`

Plan §8's premise is that the terminal prompt is **one client among several** —
the others are Emacs over TCP, the modeler's editor pane, SLIME sends and the
agent pane, all of which exist on the Janet side today. So the REPL is
`scm::Session` plus this protocol, and **`Session::handle` is the only door**.
`scm`'s prompt and `scm --serve` are both clients of it.

That is not a style preference. §8 says why: *"building the loop first and
bolting a socket on later is how the Janet side ended up with an eval channel
per consumer."* `--serve` exists in the first version of the REPL precisely so
the terminal path never becomes the only one that works.

## Framing is the reader

A message is **one datum**. A client reads until the text it holds is a
complete datum and then parses it — no length prefix, no newline escaping, and
a multi-line form needs no special case.

This is [`scm::parse_status`], the same incomplete-vs-invalid distinction S1
built for multi-line entry (§8 obligation 5), reused rather than reinvented.
It is also available *as an operation*, so an editor pane's `RET` and the
terminal prompt's continuation ask the same question over the same door and
cannot disagree:

```scheme
(:op parse :id 7 :code "(+ 1")     →  (:id 7 :status ok :framing incomplete)
```

## Requests

A request is a form of `:key value` pairs. `:op` and `:id` are always present;
`:id` is echoed back so a client may have several in flight. **An unknown key
is ignored**, so a newer client talking to an older server still works. `:op`
and `:name` accept a symbol *or* a string, because an Emacs client writes
`:op eval` and a Python one writes `:op "eval"`.

| op | arguments | answers |
|---|---|---|
| `eval` | `:code` | the value, printed, and whatever the form printed |
| `complete` | `:prefix` | matching global names, sorted |
| `describe` | `:name` | kind, signature, docstring, current value |
| `parse` | `:code` | `complete` / `incomplete` / `invalid` |
| `interrupt` | — | stops whatever is evaluating |
| `ping` | — | liveness and the version |

## Responses

```scheme
(:id 2 :status ok     :value "42" :output "out")
(:id 3 :status error  :message "car: not a pair: 5" :output "" :host-stopped #f)
(:id 5 :status ok     :bound #t :kind "procedure" :value "#<procedure area>"
                      :signature "(area r)" :doc "Circle area.")
(:id 6 :status ok     :candidates ("vector-ref"))
(:id 0 :status malformed :message "unknown op `frobnicate`")
```

Two details that are load-bearing rather than cosmetic:

**`:output` travels with the answer, including on failure.** A socket client
cannot see the server's stdout. And what a form printed *before* it broke is
usually the most useful part of the failure, so it is never dropped.

**`:host-stopped` separates the host saying stop from the script failing.** An
interrupt, a fuel exhaustion and a heap cap are the host's decision
(requirement 8) and none of them is catchable by the script; a client should
not offer "retry" for those the way it might for a typo. The message alone
would not tell a client which it was.

## `:interrupt` is out of band, and that is the whole design

The interpreter is single-threaded, so an evaluation holds it until it
finishes. **A server that queued an interrupt behind the evaluation it was
meant to stop would never deliver it.**

So `scm --serve` keeps the session on one thread and reads each connection on
its own. A reading thread that sees `:interrupt` sets the interrupt flag
*itself* — it is an `Arc<AtomicBool>`, which is `Send` — and never waits for
the session. Measured end to end: a runaway on one connection, stopped from a
second, with the session alive afterwards.

```
 -> (:op eval :id 10 :code "(let loop () (loop))")     [connection A]
 -> (:op interrupt :id 11)                             [connection B]
 <- (:id 11 :status ok :version "0.0.1")
 <- (:id 10 :status error :message "interrupted" :output "" :host-stopped #t)
 -> (:op eval :id 12 :code "(* 6 7)")
 <- (:id 12 :status ok :value "42" :output "")
```

## Introspection is values, not a comment convention

§8 obligation 6, and it has a named consumer: ray generates its API manual
from live docstrings, and the ray-steel work found Steel had **no runtime
docstrings at all**, so a REPL there could not answer "what is this".

A leading string in a procedure body is recorded as its docstring — R7RS makes
it an ordinary body expression with no effect, so this changes no semantics.
**Only when something follows it**, or `(define (greeting) "hello")` would
lose its value to the manual.

Reachable from Scheme as well as over the wire, because a script wants it too:

```scheme
(define (area r) "The area of a circle." (* 3.14159 r r))
(doc 'area)        ; => "The area of a circle."
(signature 'area)  ; => "(area r)"
(apropos "vector-f") ; => (vector-fill! vector-for-each vector-for-each*)
```

Completion filters out hygiene's renamed identifiers (`tmp.1274`): those are
names no source text can write, and offering one would be leaking the
implementation into the user's face.

## What is not here yet

- **Backtraces.** The exit criterion wants frames *and spans*; the explicit
  control stack (§5) makes the frames reachable, but nothing yet carries a
  source span into evaluation, so an error says what went wrong and not
  where. Next slice.
- **A break REPL** — stop on error with the frames live, inspect, resume or
  restart. §8's stretch goal, and the explicit stack is what makes it
  reachable at all.
- **Several interpreters over one socket.** One session per server today.
