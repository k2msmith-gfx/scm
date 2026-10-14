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

## Backtraces, and what a tail call costs you

`:eval` takes an optional `:name` — a file path, or whatever a client calls
the buffer — and a failure comes back with `:backtrace`, innermost first:

```scheme
(:id 2 :status error :message "vector-ref: index 7 out of range"
      :output "" :host-stopped #f
      :backtrace ("(vector-ref (vector 1 2) n)  at scene.scm:1:24"
                  "(+ 0 (vector-ref (vector 1 2) n))  at scene.scm:1:19"
                  "(* 2 (place 7))  at scene.scm:1:1"))
```

Three things about how that is built are worth knowing before reading one.

**A tail call leaves no frame.** §5's whole point is that a call in tail
position leaves the stack exactly as deep as it found it — so a chain of tail
calls fails with an **empty stack**, and a stack-only backtrace would say
nothing at all. Correct Scheme, useless diagnostic. The innermost call that
was *entered* is therefore tracked separately, one `Value` updated per call
with no allocation, so an all-tail-call failure still says where it broke:

```
vector-ref: index 99 out of range
  in (vector-ref v 99)  at bt.scm:2:3
```

That is one line where a stack-based trace would give none, and it is the
line that matters. It is also why a tail-recursive loop shows one frame
however many times it went round.

**A form built at run time has no location, and says nothing rather than
something wrong.** A `quasiquote` result or a macro expansion was never
written anywhere; pointing at the macro's definition as if the user had typed
it there would be worse than silence. Such a frame is still *named* — knowing
what failed is most of the value — just not placed.

**Locations cost program size, not execution size.** There is one entry per
compound form, recorded by the reader, and line and column are computed
**at read time** so no source text is retained — otherwise a long REPL
session would slowly leak every expression ever typed. The collector prunes
the entry for a form it frees, because a reused slot would otherwise report
where its *previous* occupant was written. There is a test that drives 400
evaluations and watches the table settle.

Deep recursion is truncated at 32 frames with a trailing `"…"`, so a
5,000-deep failure does not bury its own message.

## The break REPL (S8b)

An error nothing in the program will catch stops **with the frames live**,
when the client has installed a break handler (`Vm::set_break_handler`).
The explicit stack is what makes this cheap: the handler evaluates with
ordinary nested evaluation, which leaves the frames below untouched, and
decides `Abort` -- unwind as usual -- or `Return(v)` -- continue as if the
failed expression had produced `v`. In the terminal:

```
scm> (f 5)
break: vector-ref: index 5 out of range
  [0] (vector-ref (vector 1 2) x)  at <repl:1>:2:8
  [1] (* 2 (vector-ref (vector 1 2) x))  at <repl:1>:2:3
break[0]> ,locals
  x = 5
break[0]> (+ x 100)
105
break[0]> ,return 21
42
```

`,frames`, `,frame N`, `,locals`, `,return EXPR`, `,abort`; any other input
is evaluated in the selected frame's environment. An error in the handler
itself does not break again. A `guard`ed error never breaks: the handler is
only for what would otherwise reach the top. The socket client installs no
handler and gets today's unwind-and-report; an editor client can install
its own over the same `Vm` API (`live_frames`, `bindings_of`, `eval_in`).

## The terminal client (S8b)

Line editing and history (`~/.scm_history`) through rustyline, with the
session's own answers behind its hooks: `RET` inside an unfinished form
inserts a newline because the reader says *incomplete*, and tab completes
over the global names. Comma commands belong to the client, not the
language: `,load FILE`, `,time EXPR`, `,expand FORM`, `,expand1 FORM`,
`,bt`, `,tier`, `,help`, `,quit`. Results are pretty-printed to 80 columns
and multiple values print one per line. Without a terminal -- a pipe, a
test -- the client reads plain lines, so `echo '(+ 1 2)' | scm` still works.

`load`, `macroexpand` and `macroexpand-1` are procedures, not commands, so
every client has them.

## What is not here yet

- ~~**A break REPL**~~ — done, above.
- **Resume into the handler's choice of restart** beyond return-a-value: a
  retry, or a store-and-continue, are the next rungs.
  restart. §8's stretch goal, and now a good deal closer: the frames are
  already walkable and already named. What is missing is suspending at the
  error instead of unwinding past it.
- **Several interpreters over one socket.** One session per server today.
- **Completion inside a form** — candidates come from the global environment,
  so a local binding is not offered.
