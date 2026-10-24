# S14 — making T1 cheap per call: flat frames, loops, inline arithmetic

2026-10-24, aarch64 macOS. A plan, written before any code: where the code
and this disagree later, the code wins. Prompted by ray's
`examples/still-life/fruit-bowl.scm`, whose interpreted build is ~5x slower
than the same scene in Janet.

## What is slow, measured

ray-scm (T1, the default tier — `(%tier-stats)` says `t1`) against ray's
Janet 1.41, same machine, same programs:

| per operation | T1 | Janet | |
|---|---|---|---|
| empty `do` iteration | 76 ns | 3 ns | 25x |
| call of a 2-arg procedure | 166 ns | 12 ns | 14x |
| `hash2` (one `sin`, a `floor`) | 292 ns | 52 ns | 5.6x |
| `fbm`, four octaves of value noise | 8.5 µs | 1.16 µs | 7.3x |
| each `let` binding on top of a call | ~37 ns | ~0 | |

A sampling profile of the `fbm` loop (`sample`, 8 s) puts almost nothing in
the arithmetic itself (`sin`/`floor` ~3%):

| | share |
|---|---|
| `run_code` dispatch | ~28% |
| allocation + collection (`Engine::alloc`/`collect`, `malloc`/`free`, `slots`, `drop_in_place`) | ~27% |
| calls (`do_call`, `frame_at`, `enter_code`, the trampoline) | ~20% |
| arithmetic as a primitive call (`prim::mul`/`add`/`sub`, `want_number`) | ~12% |

T2 does not change this: wired into ray-scm for the measurement, `fbm` went
from 0.85 s to 0.76 s per 100k, and the fruit bowl's solver from 2.38 to
2.25 s. It inherits T1's frames and calls, and inlines fixnum arithmetic
only.

## Why, in the code

1. **Every call allocates an environment.** `do_call` builds a
   `Vec<(SymId, Value)>` of the arguments (pooled) and `Engine::env` puts an
   `Obj::Env` in the arena, for every call to every compiled closure. A
   `let` is `Op::PushEnv`: another `Obj::Env`. `vnoise`'s ten-binding `let*`
   is ten of them. `fbm` makes on the order of ninety per call, all garbage
   a moment later, and the collector is a full mark-sweep of a heap that in
   ray holds the whole prelude and the scene.
2. **Every local read walks the heap.** `Op::Local { depth, idx }` follows
   `depth` parent links through the arena (`frame_at`) and indexes a slot
   vector; each `let` deepens the chain the body reads through.
3. **A loop is a closure and a call per iteration.** A named `let` (and so
   `do`) is `MakeClosure` + a frame for the loop name + a `Call` each time
   round — the 76 ns.
4. **Every call leaves `run_code`.** A call into a compiled closure returns
   `Step::Run` to the trampoline in `Vm::run`, which pops the frame and
   calls `run_code` again; a return goes back out the same way (and asks
   `native_for` on each re-entry).
5. **`(+ a b)` is a global load and a call.** `Op::Global` for `+`, push,
   `do_call`, the plain-primitive path, then `prim::add`'s generic dispatch
   over the numeric tower, for every arithmetic operation.

## What makes it possible

- **Continuations are escape-only** (`docs/subset.md` §1). No continuation
  can resume a frame after its extent, so a frame's locals may live on the
  value stack: nothing will ever need them copied out.
- **The value stack is already a GC root, traced whole**, and `Value` is
  16 bytes, `Copy`, with flonums unboxed. A slot there costs nothing to
  allocate or collect.
- **The compiler already knows every scope statically** (`Cx::scopes`), and
  it can see every nested `lambda`, block and `Interp` it emits.

## Part 1 — frames that are not allocated

### 1a. Flat scopes

A scope — a procedure's parameters, a `let`'s variables, the slots of its
internal defines — is **captured** if anything emitted inside it can
outlive or escape the frame: a `MakeClosure` whose body refers to one of
its variables, an `Interp` (T0 is handed the environment), a `%try` or
`%parameterize` block (compiled as a separate `Code` run in a nested frame),
or a `define-syntax` (already declined). Everything else is **flat**.

- A **flat** scope's variables become slots of the frame on the value stack:
  new ops `Op::Arg(i)` / `Op::SetArg(i)` (and a `LocalDefSlot` pair for the
  checked define slots), addressed from the frame's `base`. A flat `let`
  allocates nothing: its inits are pushed and become the next slots; its
  end pops them.
- A **captured** scope is exactly today's `Obj::Env`, built the same way
  and read the same way, so T0 interop, `the-environment`-style uses, the
  leaf compiler's capture resolution (`scm-llvm/src/subset.rs`) and T2's
  pinned layouts see what they see now.
- A procedure whose own parameter scope is flat **takes its arguments where
  they already are**: `do_call` leaves them on the value stack and sets
  `base` instead of copying them into a `Vec` and an `Obj::Env`.
- Capture is decided **per scope**, conservatively: one captured variable
  keeps its whole scope on the heap. Mixed frames are a later refinement if
  the corpus asks for one.

### 1b. Loops as jumps

A named `let` whose name is used **only as the operator of tail calls in its
own body** — not passed, stored, captured or called non-tail — compiles to
a loop: its variables are flat slots, the body is a label, and `(loop a b)`
evaluates the new values, stores them and jumps. No closure, no frame, no
call. `do` expands to exactly this shape. Anything else (a loop name passed
to `map`, a non-tail recursive call) keeps today's closure.

### 1c. Calls that stay in `run_code`

A call from compiled code to a compiled closure with a plain lambda list
switches `code`/`pc`/`env`/`base` **inside** `run_code` and pushes the
caller's `Frame::Code`; `Return` pops back into the caller the same way. The
trampoline is left only for what needs it — T0, `apply`'s general cases,
the special primitives, catch/wind/parameter frames, errors. The safepoint
(interrupt, fuel, collection) keeps its 1024-tick cadence inside the loop,
and the frame-on-failure rule (`fail!`) is unchanged, so backtraces stay
frame-for-frame identical.

## Part 2 — arithmetic without a call

`(+ a b)`, `(- a b)`, `(* a b)`, `(/ a b)`, `(- a)` and the comparisons
`= < > <= >=` with two arguments compile to dedicated ops when the head
resolves to the **global** of that name (a local `+` is just a variable).
Each op:

- **checks the global still holds the original primitive** (one compare of
  the cell's `Value::Primitive` id) — so `(define (+ a b) …)` at the REPL
  still takes effect on the next execution, which is T2's guard and its
  `redefining_a_global_takes_effect_in_compiled_callers` test, kept;
- **adds two flonums, a fixnum and a flonum, or two fixnums with an
  overflow check** in place;
- **falls back to the primitive call** for anything else — overflow,
  exact division, a non-number — so every result and every error message
  is the primitive's, by construction.

Longer forms chain: `(+ a b c)` is two adds, left to right, which is the
primitive's order. `/` takes the fast path only when an operand is a
flonum (fixnum/fixnum is the primitive's decision).

## What must not change

- **Agreement**: T0, T1 and T2 agree on value, output and error message.
  The differential fuzzer is the instrument and must stay green throughout;
  every stage adds programs that stress its own shape (captured vs. flat
  `let`s, loops whose name escapes, redefined `+`).
- **Backtraces** frame for frame, and the **error text** of every refusal.
- **Escape continuations, `dynamic-wind`, `guard`, `parameterize`** across
  flat frames — they unwind by truncating the value stack, which now also
  drops flat locals.
- **The collector's roots**: flat slots are on the value stack, so they are
  traced; nothing new to root.
- **T2** declines any `Code` containing the new ops until it is taught them
  (it declines unknown ops today), so it keeps working, unaccelerated for
  those procedures, from stage one. Teaching it the ops is a later stage.
- **ray**: the scm-llvm leaf compiler resolves a texel's captured variables
  through `Obj::Env`. A variable a texel closure captures makes its scope
  captured, so it is still an `Obj::Env`. ray's `fruit-bowl.scm` texels
  (captured `ripe`, `yr`…) are the regression case.

## Stages, each measured and gated alone

| | change | exit criterion |
|---|---|---|
| S14a | capture analysis + flat `let` (no new frame ops for parameters yet) | gates green; `let*`-of-4 per-binding cost ~0; fuzzer extended with captured/flat `let` mixes |
| S14b | flat parameter frames (args stay on the value stack) | gates green; call cost measured; `fbm` measured |
| S14c | named `let`/`do` as jumps | empty `do` iteration measured (from 76 ns); loop-name-escapes cases in the fuzzer |
| S14d | calls inside `run_code` | call cost measured; backtrace tests unchanged |
| S14e | Part 2, guarded arithmetic ops | `fbm` and the fruit bowl measured; redefined-`+` test passes |
| S14f | teach T2 the new ops (optional) | T2 ≥ T1 on the 53 programs |

Each stage commits only on the full gate (CLAUDE.md "The gates": tests,
clippy at zero, conformance unchanged at 1112/71, fuzzer, benchmarks, boot,
scm-llvm, the Emacs protocol test, soak), and records its numbers here.
ray's side after each: `cargo update -p schemers -p scm-llvm`, ray's six
legs, and the fruit bowl timed and its render compared.

## What it should buy — an estimate, to be replaced by measurements

From the profile and the per-operation costs: allocation and collection
(~27%) mostly go with 1a/1b; `frame_at` (~5%) goes; the trampoline share of
calls (~20%) shrinks with 1c; the primitive dispatch for arithmetic (~12%,
plus its share of `do_call` and dispatch) goes with Part 2. A reasonable
expectation is **`fbm` from 8.5 µs to 2–3 µs** — within 2–3x of Janet rather
than 7x — and ray's fruit-bowl solver and geometry (3.9 s on a jit build)
by a similar factor. Dispatch itself (`run_code`'s match) is untouched by
this plan; if the measurements leave it as the largest share, that is the
next plan, not this one.

## Open questions for the owner

1. **Mixed frames** (heap only the captured variables of a scope) — worth it
   now, or wait for a corpus case that needs it?
2. **T2**: teach it the new ops in this plan (S14f), or leave T2 declining
   them until T2 is otherwise worth enabling in ray?
3. **ray's `make-texture` loop** (2.7 µs/texel of prelude Scheme, ~8% of the
   fruit bowl interpreted): move it into Rust in ray alongside this, or let
   S14 speed it up as ordinary Scheme?

## Results

### S14a + S14b, together (2026-10-24)

Built as one step, since a flat `let` and flat parameters share every
mechanism: `Kind::{Heap, Flat}` per compile-time scope, the `unit` driver
that recompiles a body with any flat scope something reached into moved to
the heap (a nested `lambda`, a `%try`/`%parameterize` block, an `Interp`
hand-off), and four ops — `Arg`, `SetArg`, `ArgDef`, `SetArgDef` — over
`vstack[base + i]`. A flat-parameter procedure's frame starts at its callee:
slot 0 the procedure, then the arguments where the call left them, then
defines and `let`s. Only a plain lambda list goes flat. `apply` (the door
`map`, `apply` and a host's `call_procedure` use) pushes the same layout. A
break's view builds an environment from the slots in scope at the frame's
pc (`Vm::frame_envs`), rooted until the next break; `a_break_handler_sees_
live_frames_and_can_return_a_value` covers it. T2 declines any unit with
flat slots (`native_for`, and `t2.rs` for completeness) until S14f.

| per operation (ray-scm, T1) | before | after |
|---|---|---|
| 2-arg call | 166 ns | 103 ns |
| call + `let*` of 4 | 341 ns | 187 ns (≈5 ns a binding, from ≈37) |
| `hash2` | 292 ns | 199 ns |
| `fbm` | 8.5 µs | 5.4 µs — `fbm`'s own loop still a closure (S14c) |

`a_flat_call_allocates_nothing`: 10,000 calls, 0 heap objects
(`Engine::allocations`, added for it — `obj_count` cannot say, a collection
between readings hides what was made). The fuzzer's arms 50–53 (an escaping
closure, a guard over the frame, shadowing lets, every door into a flat
procedure) and a deliberate break of the capture pass (keep the first pass
always: 882 mismatches in 1000) show it is watching. Gate: tests, clippy 0,
conformance 1112/71 unchanged, fuzz 0 mismatches over seeds 42/7/1234/99,
benchmarks, boot, scm-llvm.

Found on the way, fixed separately (bf4427c): the safepoint's step count was
per entry, so a host's many short calls never collected.

### S14c — named `let` as a loop (2026-10-24)

A named `let` whose name is only called, with the right count, in tail
position of the loop (or of an enclosing loop, so an inner loop can jump
to an outer one) compiles to slots and a back-edge: `JumpCall` names the
call form as the call in progress -- so a runaway stopped mid-loop still
reads `(loop)` innermost in its backtrace, which
`stopping_a_runaway_says_where_it_was` caught the first version dropping --
then jumps. Any other use of the name (a value, a non-tail call, the wrong
count, a `set!`), a closure or block capturing a loop variable, or a heap
`let` between the loop and the call marks the loop, and the unit
recompiles it as today's closure: the same driver, two scope ids opened in
both shapes so a recompile numbers them alike. Body defines get slots,
reset each pass.

| (ray-scm, T1) | after S14a+b | after S14c |
|---|---|---|
| empty `do` iteration | 71 ns | 33 ns |
| 2-arg call | 103 ns | 81 ns |
| call + `let*` of 4 | 187 ns | 145 ns |
| `fbm` | 5.4 µs | 5.3 µs (its loop runs 4 times; its cost is `vnoise`) |

`a_loop_allocates_nothing`: 200,000 iterations, 0 objects. Fuzzer arms 54
(nested loops, inner-to-outer jumps, a loop in non-tail position of
another) and 55 (non-tail recursion, the name escaping). Gate: tests,
clippy 0, conformance 1112/71, fuzz 0 mismatches over 4 seeds x 3000,
benchmarks, boot, scm-llvm.

### S14d — calls inside `run_code`: measured, and DROPPED (2026-10-24)

Built (a compiled callee's frame popped and run in place of `Step::Run`; a
`Return` straight into a compiled caller above a new `run_floor` the
trampoline kept), tests and fuzzer green — and worth nothing: minimum of six
alternating runs against S14c, same machine and load, a call 1% faster, a
`let*`-of-4 call 4%, `fbm` 1%. Leaving `run_code` was never the cost of a
call; `do_call`'s own work is (the arena lookup and match on the callee,
two `Rc` clones, the caller's `Frame::Code` pushed and later dropped, the
frame-stack bookkeeping). Not kept: a new invariant on the most delicate
path in the VM for 1% is a bad trade. If calls are the cost after S14e,
that list is where to look.
