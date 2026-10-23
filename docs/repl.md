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
(apropos "vector-f") ; => (numvector-fill! vector-fill! vector-for-each)
(apropos "circle")   ; => (area)  -- found by its docstring
```

`apropos` lists every name containing the text first, then every other
name whose docstring contains it, ignoring case. The second half is what
finds a concept when its name is not the word you know: a host that
documents `revolve` as "a vase, a dome" is reachable from `"vase"`. With no
argument it is every global name.

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

## Output as it happens (S25)

`:output` travels with the answer, which means a form that prints and then
runs for two seconds says **nothing** for two seconds. Measured before:

    first bytes after 2.00s; complete after 2.00s
    (:id 1 :status ok :value "42" :output "FIRSTSECOND")

and after:

    +0.00s  (:id 2 :status output :output "FIRST")
    +2.00s  (:id 2 :status output :output "SECOND")
    +2.00s  (:id 2 :status ok :value "42" :output "FIRSTSECOND")

Note the third line. **`:output` still arrives with the answer, including
on failure** — streaming adds a channel rather than moving one, because
every client written before this depends on that and because what a form
printed before it broke is usually the most useful part.

### It is opt-in, and that is not timidity

    (:op stream :id 1 :on #t)   →  (:id 1 :status ok :version "0.0.1")

A client that does not ask sees exactly the bytes it saw before. The reason
is concrete: `scm-mode.el` dispatches a response by `:id` and **removes the
callback when it fires**, so an unsolicited message carrying an in-flight
`:id` would consume the callback and the real answer would arrive to
nobody. Opting in is what makes the addition safe for a client nobody
recompiled — and the Emacs batch gate passes unchanged, which is the proof
rather than the claim.

A chunk has its **own status**, `output`, so it cannot be read as an answer
whose value happens to be nothing; and it carries the `:id` of the eval
that produced it, so a client with several requests in flight knows whose
output it is reading.

**Per connection, not per session.** Two clients of one session may
disagree about wanting it, and a sink installed per session would stream
one client's output down another's socket.

### The mechanism, and where it lives

The door did not change. `Session::handle` is still one request in, one
response out — what changed is that a host can ask to be *told* as output
is produced:

```rust
interp.set_output_sink(Some(Box::new(|text| …)));
```

It is a side channel the host wires up, exactly like ray's own
`set_diag_sink`, and `scm --serve` is its first user: for a request from a
connection that asked, it installs a sink that writes framed chunks down
that connection, calls `handle`, and removes it.

Two details that are load-bearing:

- **Only the port the host installed streams.** A script's own
  `(open-output-string)` does not reach the sink, or every internal
  `with-output-to-string` would appear in the editor's transcript and a
  host could not tell which writes were meant for it.
- **A sink is not re-entrant.** One that evaluates sees no sink installed,
  which is the only answer that terminates.

### What it fixed that was not a new feature

`scm-dap` stops *inside* a failing evaluation, and the capture returned its
string only when that evaluation ended — so the Debug Console held nothing
of what the program had printed at exactly the moment a user was choosing
between `,return` and abort. That is now delivered at the break, and it was
the strongest argument for the stage: a hole in something shipped rather
than a feature nobody had.

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

## The Rust client, and `--connect` (S22)

`scm --connect 127.0.0.1:7000` is the terminal prompt with its session on
the other end of a socket. Everything S8b gave the prompt — line editing,
history, multi-line entry, tab completion, the comma commands — works
against a server somewhere else, and **the state is the server's**: two
prompts are two processes and one environment.

The point is not remote work. It is that an editor and a prompt can be
clients of **one** session, which is what a SLIME-style workflow actually
needs: eval a form from the buffer, then type at a prompt that can see what
it defined. Zed is the named consumer (`zed-scm`, Z2) because its extension
API cannot host a REPL pane, so the prompt has to be a terminal process —
but nothing about it is Zed-specific.

### The protocol is symmetric now

S8 formatted requests on the client and parsed them on the server, and did
the reverse for responses. Only two of those four existed in Rust:

| | request | response |
|---|---|---|
| format | **`Request::to_wire`** (S22) | `Response::to_wire` (S8) |
| read | `Request::parse` (S8) | **`Response::parse`** (S22) |

Until the two in bold, a Rust client had to format the text itself — which
is the shape §8 warns about. Two clients formatting the same request
separately are two clients that can come to disagree, and the protocol test
can only see one of them. The tests are therefore **round trips**: every
`Op` and every `Outcome` through `to_wire` and back, including the payload
that breaks a naive formatter (a quote, a backslash, a newline and
unbalanced parens inside a string), plus a pass over what the session
really answered, so a field one half learns and the other does not fails
even if nobody updated the list.

**`:name` is now on a `describe` response.** Without it a described value
could not be read back into the value it came from, because the name was
the one field the wire did not carry. Adding a key is the protocol's own
way of evolving — "an unknown key is ignored, so a newer client talking to
an older server still works" — and the Emacs batch test, which drives the
real server, passes unchanged.

### `C-c` from a client is a second connection

A remote prompt's VM is in another process, so the interrupt flag the
signal handler used to set is the wrong one. It sends `:op interrupt` down
the **control connection** instead — the same pair `scm-mode.el` opens, for
the same reason: the interpreter is single-threaded, so an interrupt on the
main connection would queue behind the form it is meant to stop.

A signal handler cannot touch a `TcpStream`, so it writes the request with
`write(2)`, which is async-signal-safe, to a descriptor stored once at
connect time. The text is a constant because a handler must not allocate.
Measured end to end in `scm-cli/tests/connect.rs`: a runaway on the main
connection, a real `SIGINT` to a real client process, the answer coming
back on the main connection with the frame it was in, and `(* 6 7)` → 42
on the same session afterwards.

### What a remote prompt does not get

- **The break REPL.** A break handler runs *inside* the failing evaluation
  with the frames live, and a wire carries values, not a stack. A remote
  client gets today's unwind-and-report, exactly as the socket client does.
- **A script's stdin.** The server owns its own.
- `C-d` leaves the prompt and **leaves the server running**, which is the
  one difference from `scm` that a user has to be told, so the banner says
  it.

One other asymmetry is deliberate: the line editor decides *incomplete* with
the in-process `parse_status` rather than asking `:op parse` per keystroke.
Same reader, same binary in practice; a round trip per character would not
buy anything a differing version could not also break.

## What is not here yet

- ~~**A break REPL**~~ — done, above.
- **Resume into the handler's choice of restart** beyond return-a-value: a
  retry, or a store-and-continue, are the next rungs.
  restart. §8's stretch goal, and now a good deal closer: the frames are
  already walkable and already named. What is missing is suspending at the
  error instead of unwinding past it.
- ~~**An editor client.**~~ `emacs/scm-mode.el` (2026-10-13): a SLIME-style
  mode over this protocol, with a REPL buffer whose `RET` asks `:op parse`,
  completion from `:op complete`, eldoc from `:op describe`, and interrupt on
  a second connection. Its batch test drives the real server.
- ~~**A Rust client of the protocol.**~~ `scm --connect` (S22), above.
- ~~**Streaming output.**~~ Done, S25 — above.
- **Several interpreters over one socket.** One session per server today.
- **Completion inside a form** — candidates come from the global environment,
  so a local binding is not offered.
