# The agent panel in `edit` — plan, 2026-11-16

**Status, 2026-11-17: A0 measured; A1–A4 and A6 built** (below, "A1 and
A2 as built", "A3 as built", "A4 as built", "A6 as built"): the standalone
has the agent, its buffers follow the agent's edits, and the agent reads
and edits the buffers themselves. **A5, the modeler, WAITS** (Kevin,
2026-11-17: *"I'd like to try the agent pane here first before the modeler
integration"*). Kevin accepted
the recommendations on D1–D7 on 2026-11-16 and then changed two of them,
both recorded where they stand: **D1 is one process per CONVERSATION**, and
**D3 is ask in the pane**, for programs and for files outside the project.

Kevin, 2026-11-16: *"I would like to create an agent panel (similar to the
modeler agent panel) that allows me to log into my claude subscription
account and use it as a programming assistant in edit."* And: *"this agent
panel should also replace the one in the modeler too, so it should be part
of the edit/scm crate."*

So: one agent panel, in `edit/`, two hosts — the shape
[editor-crate-plan.md](editor-crate-plan.md) already gave the text widget,
the minibuffer and the terminal pane. The standalone gets a programming
assistant; the modeler drops its own bridge and pane code and keeps only
what is the modeler's to say: its tools, its prompt, its fence.

A bare path here is this repo; a `ray/` prefix is the other.

## What exists, measured

Read at ray `origin/main` `6cb58083` and scm `0c314b8`.

| piece | where today | lines | generic? |
|---|---|---|---|
| spawn `claude -p`, read stream-json, cancel, session id, `claude auth`, metrics footer | `ray/modeler/src/ai.rs` | 1,368 | nearly all |
| the `%ai-*` prims (12) | `ray/modeler/src/script.rs` | ~130 | yes |
| `Wake::Ai` → `on-ai-progress` / `on-ai-reply` | `ray/modeler/src/main.rs` | ~60 | replaced |
| Escape interrupts a turn | `ray/modeler/src/main.rs` kernel branch | ~12 | hook stays, body moves |
| pane policy: `agent-submit`, slash commands, pending row, progress rows | `ray/modeler/scripts/scm/08-editor.scm` | ~160 | yes |
| prompt, catalogue, example retrieval, `:agent` menu | `ray/modeler/scripts/scm/14-ai.scm` | 622 | **no** — the modeler's |
| MCP tools over the eval port | `ray/modeler/tools/modeler_mcp.py` | 730 | **no** — the modeler's |
| transcript + input line (`Style::Code` and input wrap were built for this pane) | `edit/src/widget/textpane.rs` | 1,575 | already here |
| the input editor (`edit-start`, history, paste) | `edit/scm/04-minibuffer.scm` | 723 | already here |

How the modeler's bridge works, because the plan keeps it:

- **One process per turn.** `claude -p --output-format stream-json --verbose`,
  the prompt on stdin (then EOF), `--resume <session-id>` for every turn
  after the first. Not a long-lived session.
- **Login is the CLI's own.** Nothing here holds a credential.
  `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_BASE_URL` are
  removed from the child's environment so a stray key cannot move a
  subscription user onto metered billing. `/login`, `/logout` and `/status`
  in the pane run `claude auth …`; while a login waits, the next line typed
  goes to its stdin (the pasted code).
- **`--bare` is wrong** — it skips credential reads, so a logged-in user is
  told to log in. The modeler uses `--setting-sources ""` and a fixed neutral
  cwd instead.
- **A big system prompt goes by file** (`--system-prompt-file`): Linux caps
  one argv string at 128 KiB.
- **stderr is drained on its own thread**, or a 64 KB pipe fills and the
  child blocks.
- **Cancel kills the child**, and a killed turn is its own outcome
  (`Cancelled`), not an error. Tools already run stay run.
- **An allow-list is not a fence.** With nobody to prompt, built-in tools
  simply run; only `--disallowedTools` denies.

What `edit` offers a subprocess-backed pane:

- `crate::wake::wake()` — a reader thread fills shared state and wakes the
  loop; the host then calls `(remote-poll)`, which runs `*wake-hooks*`
  (`edit/scm/14-remote.scm:78`). `20-rust.scm` already hooks an LSP hint
  this way.
- `lsp.rs` is the model for the Rust half: a child over pipes, a reader
  thread, `serde_json` (already a dependency of `edit`), and
  `Server::over(from, to)` so tests run against a fake.
- The layering rule: only `scm_binding*` may name the VM.

What `edit` lacks:

- **The standalone has no place for it.** `app.rs` hard-codes four pane ids,
  a `Panel { Terminal, Repl }` and three icons. No right dock.
- **No session store** (the modeler has `00-session.scm`; the standalone
  keeps only `~/.config/edit/theme.scm`).
- **No auto-revert.** `:e!` reverts by hand. An agent that edits files on
  disk makes this matter the first day.

Checked against `claude` 2.1.303 on this machine: `--input-format`,
`--include-partial-messages`, `--permission-mode`, `--permission-prompt-tool`,
`--session-id`, `--append-system-prompt[-file]`, `--add-dir` and
`claude auth login|logout|status` all exist. Their behaviour under `-p` is
A0's to measure, not this document's to assert.

## The shape

    edit/src/agent.rs            the bridge: no VM names, serde_json, std threads
    edit/src/scm_binding/agent.rs  the %agent-* prims
    edit/scm/25-agent.scm        the pane: submit, slash commands, transcript rows
    edit/scm/host/standalone.scm the standalone's profile and its dock
    ray/modeler/scripts/scm/14-ai.scm   the modeler's profile: prompt, tools, fence

**The bridge (`agent.rs`)** is `ai.rs` with the modeler taken out. One
`Request` in, a queue of `Event`s out:

    Request { prompt, profile }            // profile: see below
    Event::Text(s) | Tool{name, arg} | Result(s) | Metrics(footer)
        | Done{summary, session} | Error(s) | Cancelled | Auth(line)

The reader thread pushes events onto a `Mutex<VecDeque<Event>>` and calls
`wake()`. No `EventLoopProxy`, so it does not know which host it is in.
`agent::busy()` and `agent::cancel()` are plain Rust, because both hosts'
Escape paths are kernel-side (the modeler's lesson: a handler on the Scheme
chain needed two presses and stole Escape from the live render).

**The profile** is the whole host interface — an alist the host's Scheme
sets, read at each turn:

| key | standalone default | modeler |
|---|---|---|
| `:cwd` | the buffer's git root (`%git-root`), else its directory | `/tmp/modeler-ai` |
| `:setting-sources` | the CLI's default (so `CLAUDE.md` loads) | `""` |
| `:system-prompt-file` / `:append-system-prompt` | append a short "you are in `edit`" note | its catalogue, by file |
| `:mcp-config`, `:strict-mcp` | none | `modeler_mcp.py` on `MODELER_PORT` |
| `:allowed-tools`, `:denied-tools` | none | the 19 `mcp__modeler__*`; `AGENT_DENIED_TOOLS` |
| `:permission-mode`, `:permission-prompt-tool` | `acceptEdits`, `stdio` (it asks in the pane) | unset |
| `:model`, `:max-turns` | `opus`, unset | `opus`, 120 |
| `:prepare` | identity | `(ai prompt)`'s retrieval: prompt → user message |
| `:tool-label` | name + first argument | strips `mcp__modeler__`, `eval` rows as `:code` |
| `:session-get` / `:session-set` | a file under `~/.config/edit/` keyed by cwd | its session store, `:ai-session` |

The modeler column is the modeler's agent AS IT IS TODAY: a fenced maker
with a neutral cwd, no `CLAUDE.md` and no file or shell tools. That is why
it is a profile and not two code paths.

**But today's modeler profile is not where it should end** (Kevin,
2026-11-16: in `edit` the agent is *"primarily code assisting"*, in the
modeler *"it would be doing that but also driving ray/model api to make
things"*). So the modeler's agent is the standalone's PLUS the scene, not a
different agent:

    code profile  = project cwd + CLAUDE.md + file tools        (edit's default)
    make profile  = code profile + the modeler's MCP tools + its catalogue, appended

A profile is therefore a BASE the host adds to, not a table the host fills
in from nothing: `(agent-profile-extend base :mcp-config … :append-system-prompt-file …)`.
Sharing is what makes this cheap — a code-assist improvement made for
`edit` (auto-revert, `@` mentions, permission prompts, editor tools) is in
the modeler the day it lands, where two bridges would have to be kept level
by hand. What it costs is D7.

**The pane (`25-agent.scm`)** is the ~160 lines from `08-editor.scm`, moved:
`agent-submit`, the slash commands, the pending "thinking…" row, the
progress rows, the footer, `new-conversation`, the `:agent` menu. Events
arrive through one `*wake-hooks*` entry that drains `(%agent-poll)`. The
host says where the pane is (`*pane-id*` `:agent`) as it does for every
other pane.

## Stages

Each leaves both hosts green. Sizes are guesses until A0.

| | what | exit criterion |
|---|---|---|
| **A0** | **Measure the CLI.** A scratch script against the real `claude`: does `--resume` work across cwd-keyed sessions the way `ai.rs` assumes in 2.1.x; what `--permission-mode acceptEdits` / `plan` do under `-p`; what `--permission-prompt-tool` asks of an MCP tool; what `--include-partial-messages` emits; whether one `--input-format stream-json` process can take turn after turn and be interrupted without being killed. | a table in this file under "A0 as measured"; D1 and D3 answered with evidence |
| **A1** ✔ | **The bridge.** `edit/src/agent.rs` ported from `ai.rs`: spawn, stderr drain, stream parse, metrics de-duplicated by message id, cancel, stale-session retry, `claude auth`. `Bridge::over(reader)` for tests. The Messages-API backend and the one-shot mode are NOT ported (D5). | unit tests over canned `.jsonl` (text, tool use, tool result with an image, compaction, error result, a stream cut mid-line); a `sleep` child killed by `cancel()` reports `Cancelled`; no real turn spent |
| **A2** ✔ | **Prims and pane.** `scm_binding/agent.rs` (`%agent-send`, `-poll`, `-cancel`, `-busy?`, `-session`, `-session-set`, `-reset`, `-auth`, `-auth-pending`, `-auth-input`); `25-agent.scm` in `SHELL` and `EMBEDDED`; the profile. | a bare-interpreter test drives `agent-submit` against a fake `claude` (a shell script that prints canned events, named by `EDIT_CLAUDE_BIN`) and asserts the transcript rows; `/status` against a fake `auth` |
| **A3** ✔ | **The standalone hosts it.** A fifth pane id and a right dock in `app.rs` (D2), an icon, `M-a`, `%show-dock`; Escape in `Session::escape` cancels a busy turn when the agent holds typing; session id stored per project root. | `cargo run -p edit -- FILE`, `M-a`, `/login`, a real question about the open file answered, restart, the conversation resumes. Kevin's use is the gate |
| **A4** ✔ | **It edits, and the editor notices.** (The permission posture is built: see "as built".) Auto-revert of an unmodified buffer whose file changed on disk, and a loud refusal to revert a modified one; the turn's changed files listed in the transcript. | the agent fixes a planted bug in an open buffer; the buffer shows the fix without `:e!`; a buffer with unsaved edits is not overwritten |
| **A5** | **The modeler takes it.** `14-ai.scm` sets the profile; `ai.rs`, the `%ai-*` prims, `Wake::Ai` and the pane half of `08-editor.scm` go; `MODELER_AI_*` variables keep working through the profile; Escape's kernel branch calls `edit::agent`. | ray's `test_agent_pane.py`, `test_agent_mcp.py`, `test_agent_during_render.py` (19/19) and `test_ai_catalog_modes.py` pass unchanged; `ai_census.py` and `ai_ab.py` still read the stream log; one real turn resumes a pre-existing `:ai-session` |
| **A6** ✔ | **Editor tools for the agent** (D4). An MCP server the binary itself provides (`edit --mcp`, stdio, reaching the running editor as `14-remote` reaches `scm --serve`): open buffers and unsaved text, selection, cursor, diagnostics, open-file-at-line, eval in the attached backend; and the WRITE side the modeler's agent already has for the scene editor (`editor_write`, `editor_replace`, `editor_send`): replace a region or a whole buffer in place, each agent edit ONE undo step. | the agent answers "what is wrong with the form at point" from an UNSAVED buffer, fixes it in the buffer, and one `u` takes the fix back |
| **A7** | **Reading comfort.** Replies are markdown: fenced code coloured through `Language`, `@` file mentions (the picker is already bound, `04-minibuffer.scm:546`), token streaming if A0 says partial messages are worth it, send-region-to-agent. | by eye |

A1–A3 are the programming assistant in its first usable form. A5 is the
replacement and can go any time after A2; it is ordered after A4 only so
the modeler changes once the interface has stopped moving.

## A0 as measured (2026-11-16)

`claude` 2.1.303, macOS, the subscription login, `--model haiku`, in a
scratch git repo holding a one-line `CLAUDE.md`. Fifteen real turns, each
run once — enough to say what a flag does, not how often. Every run had
the API-key variables removed; `init` reported `apiKeySource: none`.

| question | answer |
|---|---|
| does the stream still look as `ai.rs` reads it? | Yes: `assistant` (text, `tool_use`), `user` (`tool_result`), `result` with `session_id`, `num_turns`, `duration_ms`, `duration_api_ms`, `total_cost_usd`. New since: `system` subtypes `hook_started`, `hook_response`, `thinking_tokens`, `permission_denied`; a `rate_limit_event`; `thinking` content blocks. `ai.rs`'s `_ => {}` arms already skip them. |
| is `CLAUDE.md` read from the cwd? | Yes, with default setting sources. The user's HOOKS run too (`hook_started` in the stream). |
| `--resume` in the same cwd | Works. |
| `--resume` from ANOTHER cwd | **Works** — the conversation came back, and the model noted the first directory's `CLAUDE.md` was no longer loaded. `ai.rs`'s "sessions are keyed by cwd" did not hold here. Storing the id per cwd is still right, for the second half of that sentence. |
| default permission mode | `Write` and a redirecting `Bash` are both REFUSED, as a `permission_denied` event and an error `tool_result`; `result.permission_denials` lists them. The turn still ends `success`. |
| `--permission-mode acceptEdits` | `Write` runs. **`Bash` is not denied by it**: `uname -s` ran, `echo x > file` inside the cwd ran, `python3 -c …` was refused. Read-only and in-tree-writing commands pass; the rest need approval. `--setting-sources ""` changed nothing, so this is the CLI's rule and not Kevin's settings. |
| `acceptEdits` + `--disallowedTools Bash` | `Write` runs; the model reports it has no Bash tool at all. **This is D3(b).** |
| `acceptEdits` + `--allowedTools "Bash(echo:*)"` | both ran. |
| `--permission-mode plan` | no edit, no shell; it wrote a plan file under `~/.claude/plans/` and stopped. |
| `--permission-prompt-tool mcp__perm__ask` | The CLI calls the tool with `{tool_name, input, tool_use_id}` and reads back JSON text: `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`. A deny reaches the model as the tool's error text. It was asked for `Write` and for `python3`; **it was NOT asked for `echo forbidden`** — commands the CLI holds to be read-only never reach the prompt. An answer delayed **75 s** was still honoured, so a person can take their time. |
| `--include-partial-messages` | `stream_event` lines carrying the API's own events; 27 `text_delta`s for two sentences. The whole `assistant` event still follows. |
| one process, many turns (`--input-format stream-json`) | **Works.** A `{"type":"user","message":{…}}` line per turn; each turn ends in its own `result`; the process waits for the next line; EOF on stdin exits 0. The second turn's `init` came 0.1 s after the line was written. |
| interrupt without a kill | **Works.** `{"type":"control_request","request_id":…,"request":{"subtype":"interrupt"}}` on stdin is answered at once by a `control_response`, then a `result` with `subtype: error_during_execution`, `terminal_reason: aborted_streaming`. The process stayed up, and the next turn knew both the earlier turns and what it had been interrupted doing. NOT measured: an interrupt while a tool is running. |

What this decides:

- **D3 is buildable as recommended.** (b) is `acceptEdits` plus
  `--disallowedTools Bash`; `acceptEdits` alone is not "shell denied". (d)
  is `--permission-prompt-tool`, with the caveat that read-only commands
  bypass it.
- **D1: per turn for A1, as decided — and the other IS worth a stage.** The
  stream is the same either way, so `agent.rs` reads one format; what
  differs is who owns stdin and when the child ends. A long-lived session
  buys an interrupt that keeps the conversation's process, and no start-up
  per turn. It becomes **A3b**, after the standalone is in use, and A1
  keeps the reader free of any assumption that EOF ends a turn (a `result`
  line does).
- **Show denials.** `permission_denied` is a new row kind for the
  transcript: a refused write the user never hears of looks like an agent
  that ignored them.

## A1 and A2 as built (2026-11-17)

`edit/src/agent.rs` (the bridge), `edit/src/scm_binding/agent.rs` (ten
`%agent-*` prims), `edit/scm/25-agent.scm` (the pane). Thirteen tests, one
of them `#[ignore]`d because it spends three real turns
(`the_real_claude_holds_a_conversation_and_asks_leave`; passed 2026-11-17).
The rest run against a stand-in `claude`, a shell script that speaks the
stream.

**Three things are not as the stages above first said**, each because
Kevin asked for it and a measurement backed it.

**1. One process per conversation, not per turn** (Kevin: *"make
communication as efficient as possible to minimize latency but keep good
context"*). Four trivial turns each way, Haiku:

| | before the model is asked | first token | turn over |
|---|---|---|---|
| a process per turn, `--resume` | 0.6–1.3 s | 1.8–2.6 s | 2.6–3.2 s, exit included |
| one process, turns 2–4 | 0.01 s | 1.2 s | 1.2–1.3 s |

So: `--input-format stream-json`, a request per stdin line, a turn ended by
its `result` line. Escape is a `control_request` interrupt, which the CLI
answers at once — measured mid-reply and DURING a running tool (the tool's
process was gone, the CLI stayed up, the next turn knew what had happened).
A process that has not answered within three seconds is killed, and the
turn still closes as cancelled. `--resume` is for what a live process
cannot carry: a restart of the editor, a process that died, a setting fixed
at start-up that changed — the profile's arguments ARE the process's
identity, so `/model sonnet` is one restart into the same conversation.
The modeler gains more than these numbers: its turn also restarts a Python
tool server and reloads a long session.

**2. Replies are drawn as they are written** (`--include-partial-messages`):
a `delta` event per piece, drawn onto the rows the whole message then
lands on. The first words are on screen when the model has said them.

**3. The agent asks leave, in the pane** (Kevin: *"access to files and run
programs (with permission)"*, *"files outside the immediate working
directory"*). `--permission-prompt-tool stdio` turns each call that needs
leave into a `can_use_tool` request on the process's own stdout, answered
on its stdin — **no tool server**, which is why D3(d) no longer waits for
A6. Under `acceptEdits` plus `stdio`, measured:

| the agent does | asked? |
|---|---|
| reads a file in the project | no |
| writes a file in the project | no |
| runs `git status` (the CLI holds it read-only) | no |
| runs `cargo --version`, `python3 -c …` | **yes** |
| reads a file outside the project | **yes** |
| writes a file outside the project | **yes** |

In the pane the question is a row and the next line typed is the answer:
`y`, `n`, `a` (this and every later call of that tool, this conversation),
or anything else — which refuses and is what the agent is told ("use cargo
check instead"). A refusal waited 3 s in the measurement and an MCP one
75 s in A0; a person can take their time. Rules already allowed in Claude
Code's own settings still hold, because the setting sources are the CLI's
default in this profile.

**Login is the subscription's, said outright.** `/login` runs `claude auth
login --claudeai` (the CLI's other mode, `--console`, is API billing);
`/status` names the account and plan, and says so loudly if the login is
ever not `claude.ai`; the API-key variables never reach the child.

**Other things settled in the building:**

- **The agent's state is per UI thread** (a thread-local, shared with its
  reader threads by `Arc`), not per process. The first version kept a
  process-wide queue, and every interpreter that loaded the shell drained
  it — one test's events were drawn into another's pane, and the suite hung
  one run in three.
- **The session id is the host's, not the kernel's.** It rides out on
  `Done`, `Error` AND `Cancelled` (`ai.rs` threw the conversation away on a
  turn that hit its cap), is filed by the directory the turn ran in, and
  comes back in the profile. So there are no `%agent-session` prims.
- **A profile crosses as a flat list** — `("cwd" "/proj" "model" "opus")` —
  and an unknown key is an error, not a setting dropped.
- **Context stays cheap by construction**: the system prompt is fixed when
  the process starts, so the prefix caches; what changes per request goes
  in the request (`*agent-prepare*`), and the footer's `cache … read / …
  new` is where a prefix that has stopped caching shows.

## A3 as built (2026-11-17)

`cargo run --release -p edit -- FILE`, then **M-a** or the robot icon at
the right end of the status strip.

- **A fifth pane, a column on the right** (`app.rs`: `AGENT = 5`, 30% of
  the width, its left edge draggable 12–60%), hidden until asked for, so no
  `claude` starts and the editor keeps the width. `Icon::Agent` is the
  fourth strip icon — a robot's head, as the modeler's dock button is — lit
  while the column shows.
- **M-a goes to the agent from anywhere, and from its own prompt hides it**;
  the icon is a plain toggle. `C-x o` goes round the panes that show. A
  click in the column takes the keyboard; a drag selects and Cmd-C copies.
- **Escape at its prompt interrupts a turn and keeps the keyboard there**,
  in `Session::escape` — the kernel, not the Scheme chain, which that
  function returns before reaching. Idle, Escape goes back to the editor.
- **The conversation survives a restart**: its id is filed by directory in
  `~/.config/edit/agent-sessions.scm` and the next request there resumes
  it. `C-l` and `/new` forget it.
- **Every request says what the editor shows** (Kevin: *"the agent should
  have access to buffers, what the user is typing and where"*): an
  `<editor>` block in front of the request — the file in front, its
  language, the cursor's line and column and that line's text, the
  selection, the other buffers open, and which have edits not saved. It is
  the shell's (`agent-editor-context`, 25-agent), so the modeler gets it
  too, and it rides in the REQUEST so the cached system prompt does not
  change. It is names and a selection, never a file's contents: the agent
  reads files itself. **What it still cannot do is read a buffer's UNSAVED
  text or write into a buffer** — it is told when a file is newer in the
  editor than on disk and asks for a save. That is A6's, and with the
  process's own control channel proven for permissions, A6 should try
  `--mcp-config` with an SDK-style server on the same channel before a
  second process.

Tested by `the_agent_has_a_column_and_knows_what_the_editor_shows`
(`standalone.rs`), against the stand-in; `EDIT_AGENT_SHOT=out.png` on that
test keeps a frame to look at. **Not yet done by a person:** a real
conversation from the window — that is the stage's exit criterion and it
is Kevin's.

## A4 as built (2026-11-17)

The agent works on the files on disk; the buffers follow
(`sync-buffers-with-disk`, 25-agent — the shell's, so the modeler's too).

- **A buffer with no edits of its own is reloaded** when its file changes,
  the cursor kept on its line, and the pane says `↻ reloaded NAME`: after
  each result of one of the agent's edit tools, and at the end of every
  turn — which is what catches a file a COMMAND changed (a formatter).
  The buffer on screen and the ones that are not.
- **A buffer with unsaved edits is never touched.** When the agent changed
  its file the pane says so, in red, once: `:e!` takes the file's version,
  `C-x C-s` keeps yours. The agent is told beforehand, too — the `<editor>`
  block marks the file, and its prompt says the editor will not take a
  change to it.
- **A changed file is found by its size and time** (`%os-stat`), noted
  when a request goes out, so a step does not read every open file; a file
  the agent's own edit tools named is compared by its text whatever the
  clock says. Without `%os-stat` (a host that has none) only those are
  checked.
- **A reload is a load**: the buffer's undo does not reach back across it.
  Edits made in the buffer as undo steps are A6's.
- **The transcript got quieter in the doing**: a path under the project is
  shown from the project, a file read back and an edit's "updated" get no
  row (the reload says it), and any other result is its first four lines.

Tests: `a_buffer_follows_the_agents_edit_unless_it_has_edits_of_its_own`
against the stand-in, and the stage's exit against the REAL `claude`,
`the_real_claude_fixes_a_bug_in_the_open_buffer` (`#[ignore]`, one turn;
passed 2026-11-17 in 5 s): a wrong line under the cursor, "the line my
cursor is on has a bug", and `width * height` on screen with no `:e!`.

**Not done:** the plan's `/mode` command. With leave asked in the pane
(D3) there is no mode to switch between yet; it returns if a read-only
"plan" posture is wanted.

## A6 as built (2026-11-17)

**D4 is decided by a measurement, and it is neither option that was put.**
Not the modeler's Python script and not `edit --mcp`: an MCP server of
type `sdk` is one the CLI reaches by asking ITS HOST, on the conversation's
own channel — `mcp_message` control requests out, the answers back on
stdin. So the editor's tools need no second process, no socket and no
port, and a tool's handler is Scheme on the thread that owns the panes.
`src/agent.rs` answers the handshake and the tool list itself; a call
becomes an event, `(define-agent-tool …)`'s handler runs, and its answer
goes back under the call's id. A handler may answer LATER (it is handed a
`reply`), which is how `eval` waits on another process.

The tools (25-agent — the shell's, so the modeler's agent has them the day
it takes the pane):

| tool | what it does | asks leave? |
|---|---|---|
| `buffers` | what is open: name, file, language, unsaved?, which is on screen and where the cursor is | no |
| `buffer_read` | a buffer AS THE USER SEES IT, unsaved edits included, lines numbered; a range | no |
| `buffer_edit` | replace the one occurrence of `old` with `new` IN the buffer — there at once, unsaved, one undo | no |
| `buffer_write` | the whole text, likewise one undo | no |
| `diagnostics` | what the reader and the language server find wrong, for the text as it stands | no |
| `open_file` | show a file at a line | no |
| `eval` | evaluate code in the image the buffer's forms go to (scm, ray, Guile) | **yes** |

A tool the profile does not list as allowed asks through the same question
the shell does, so "asks leave" is one flag on the definition. A host adds
its own with `define-agent-tool`, and may still name whole servers in
`:mcp-config`; the two are merged.

Two prims came with it, because `%editor-set-text` is a LOAD and forgets
the undo: `%editor-edit` (the one place, one undo step) and
`%editor-rewrite`.

Tests: `the_hosts_tools_are_served_on_the_control_channel` (the protocol),
`the_agent_reads_and_edits_buffers_through_the_editors_tools` (each tool,
and a call from the agent end to end), and the stage's exit against the
REAL `claude`, `the_real_claude_works_in_an_unsaved_buffer` (`#[ignore]`,
two turns; passed 2026-11-17, 4 s and 5 s): a bug that exists ONLY in the
buffer — the file on disk is right — is named from `buffer_read`, fixed
with `buffer_edit`, the file never written, and one undo takes the fix
back.

**Seen and left:** a `buffer_edit` that fails (its `old` not found) shows
only as a second `→ buffer_edit` row — the stream does not say which
results are errors. And in one of three runs the model spent a step on the
CLI's own `ToolSearch` before its first editor tool; the tools are few
enough that this should be measured before the modeler adds its nineteen.

## In use: what Kevin asked for next (2026-11-17)

*"Beautiful, love it! I need a few features though…"*

**1. Leave given for a place is not asked for again.** `y` to a file
outside the project grants its DIRECTORY and everything under it — to
write (and so to read) when a write was allowed, to read when a read was —
for the project, not the conversation: kept in
`~/.config/edit/agent-grants.scm`, by project, across restarts, and not
forgotten by `C-l`. `once` allows the call and remembers nothing; `/grants`
lists; `/revoke` forgets. A command allowed with `y` is not asked for again,
word for word, for the rest of the conversation; `a` is still every call of
that tool. The question's own line says which of these `y` will be.
(`leave_given_for_a_place_is_remembered`.) The grant is the pane's, answered
before the question is shown; the CLI still asks, and is told yes.

**2. The editor's own commands, for the agent.** Five more tools:

| tool | what it does | asks leave? |
|---|---|---|
| `buffer_save` | a buffer to its file, as C-x C-s | no |
| `buffer_close` | close one; not over unsaved edits unless told they are discarded | no |
| `editor_commands` | list the editor's commands, or search them by name | no |
| `editor_command` | run one that only changes what the editor SHOWS, with literal arguments | no |
| `editor_eval` | Scheme in the editor's own image — anything the editor can do | **yes** |

Which commands are the unasked ones is a list, `*agent-ui-commands*`, that
the host adds to: the standalone's are its themes (`load-theme`,
`save-theme-choice`), its panels (`repl`, `toggle-terminal`,
`toggle-files`, `toggle-agent`) and the text size. `editor_command` takes
only literals — a string, a number, a keyword — so nothing is evaluated on
that door; everything else goes through `editor_eval` and is asked for.

Against the REAL `claude` (`the_real_claude_drives_the_editor`,
`#[ignore]`; passed 2026-11-17): "switch the editor to the dracula theme
and open the REPL" — 4 s, two calls; "open notes.txt, change red to blue in
its buffer, save it and close that buffer" — 7 s, five calls, the file
changed and the buffer gone.

**3. Automatic** (Kevin, later the same day: *"still noisy as far as asking
for 'y' approvals — can we put in an automatic mode?"*). `/auto` answers
every question yes as it arrives — a program run, a file anywhere, the two
tools that evaluate — and one already waiting. Each is still drawn, as a
`✓` row, so what the agent did is on the page. It is a grant like the
others: the project's, kept across restarts, ended by `/auto off` or
`/revoke`; and the prompt reads `agent (auto)>` for as long as it holds,
because an agent that runs anything unasked should never be a surprise.
Every question's own line now names `/auto` as the way to stop them.

**4. `/usage`.** What the plan has left — each limit's window, how much
of it is used and when it resets, and loudly when a limit is hit or
overage is in use — from the `rate_limit_event` the CLI sends with every
reply; and what this conversation has used: turns, tool calls, output
tokens, the context against its window, the model, and what it would have
cost at API list prices (a subscription is not billed that). Nothing is
fetched: it is what the stream already said, so before the first reply the
plan line says it has not been reported yet. **A bug it turned up:** the
CLI's `total_cost_usd` is the PROCESS's running total, so with one process
per conversation every turn's footer had been showing the whole
conversation's dollars; a turn's footer now shows its own.

**5. Code in a reply is a block** (Kevin: *"formatted differently than
other text … and a little glyph … that when you click on it, it gets
copied … AND/OR … copies it directly into the edit buffer at the point or
after the last function"*). A fenced block is drawn as one: a header row
naming its language, then its lines on the editor's own ground, set in
from the pane's — Lisp token-coloured, anything else in plain ink. The
header's words are clicked: **copy** (the clipboard), **insert** (the
buffer on screen, at the cursor) and **append** (its end, after a blank
line); an insertion is one undo step. It is a block while it streams — a
fence half written is already a header.

In the widget (`textpane.rs`): three row styles, `block`, `block-code` and
`block-head`, a band of ground under a row that has one, and
`%pane-line-at`, which a transcript had no answer to before (the row under
a point was the file tree's question only). The block is remembered
nowhere but the rows: a click reads the styled rows under its header back.
(`code_in_a_reply_is_a_block_that_copies_and_inserts_at_a_click`, with
real clicks at where the words were drawn.) **Not done:** colouring by the
block's language beyond Lisp — the editor has a `Language` for Rust and
could lend it.

**What that run found first, and fixed:** the CLI DEFERS MCP tools behind
its own `ToolSearch`, and the model called `editor_command` without having
seen its schema — with `command:` for a parameter named `name` — and was
refused twice. The process is now started with `ENABLE_TOOL_SEARCH=false`
whenever the host has tools (the schemas ride in the cached prefix: about
22k tokens more context here, with the account's own connectors in it,
and no search turn), and a call with an argument the tool does not take is
answered with the arguments it DOES take.

## Risks

- **A5 is a swap under a tested surface.** The modeler's agent has four
  Python suites and two harnesses (`ai_census.py`, `ai_ab.py`) that read
  its stream log and its footer. The footer text and the stream-log path
  are interface; keep them byte-for-byte or change the readers in the same
  commit.
- **Two repos, one change.** A5 lands as: push scm, `cargo update -p edit`
  in ray, then the ray commit. ray's `.cargo/config.toml` redirect is the
  loop for working on both.
- **The modeler's wake path.** The standalone calls `(remote-poll)` on
  every wake. A5 must confirm the modeler does too, or agent events queue
  and never draw.
- **A programming assistant has a shell.** The modeler's agent is fenced
  and its blast radius is a scene. The standalone's is the working tree.
  That is D3, and it should be decided before A4, not discovered in it.
- **Sessions are keyed by cwd.** A profile whose `:cwd` moves (a buffer in
  another repo) cannot resume the old id. Store the id per cwd; never use
  `--continue`, which would pick up a terminal Claude Code session in the
  same directory.
- **Fetched and file content is a prompt-injection surface** in a long
  agent loop. The fence and the permission mode are the mitigation; the
  transcript showing every tool call is the audit.

## Decisions for Kevin

- **D1 — DECIDED 2026-11-17: one process per conversation** (see "as
  built"). What was recommended first: per turn is what
  the modeler has run since September: cancel is a kill, a crash loses one
  turn, `--resume` carries the rest. A long-lived `--input-format
  stream-json` process would start turns faster and could be interrupted
  without losing the process, but nothing here has measured it.
  *Recommend per turn for A1, and let A0 say whether the other is worth a
  later stage.*
- **D2 — where the pane sits in the standalone.** A right dock, as in the
  modeler (56 columns there), or a third occupant of the bottom panel
  beside the REPL and the terminal. *Recommend the right dock:* a
  conversation is tall and narrow, and the REPL should stay visible while
  the agent works. It costs `app.rs` a fifth rect and a fourth icon.
- **D3 — DECIDED 2026-11-17: it asks in the pane** — (d), with in-project
  edits unasked — see "as built". The options as first put: (a) read-only until
  told otherwise (`plan`-like); (b) edits files freely, shell denied;
  (c) edits freely and runs an allow-listed shell (`cargo`, `git` read
  verbs); (d) asks in the pane before each write or command, through
  `--permission-prompt-tool` — which needs A6's server first.
  *Recommend (b) for A4 with a `/mode` slash command to change it, and (d)
  as the place to end up.*
- **D4 — DECIDED 2026-11-17 by measurement: on the conversation's own
  channel** (see "A6 as built"). As first put: The modeler's are a
  Python script over a TCP port. The editor plan says "no install beyond
  the binary". *Recommend `edit --mcp`:* the same binary, stdio, no
  Python. The modeler keeps `modeler_mcp.py` either way; the profile takes
  any MCP config.
- **D5 — the Messages-API backend and one-shot mode.** `ai.rs` carries
  both (`MODELER_AI_BACKEND=api`, `MODELER_AI_AGENT=0`). Neither is the
  subscription path and neither is what the pane is for now.
  *Recommend retiring both at A5* rather than porting them; say if either
  is still used.
- **D7 — the modeler's agent: one combined profile, or two it switches
  between?** Combined is the simplest thing that matches the sentence
  above, and it has three costs, all measured before: (1) the catalogue
  rides on every turn, including "rename this function" (it outgrew
  128 KiB in September); (2) with file tools and the examples directory in
  reach the agent copies an existing model instead of building one — the
  ASK 21 runs had to forbid it; (3) the fence exists because an unfenced
  maker reached for `perl` and `sed` instead of the verbs. Two profiles
  (`/code` and `/make`, one conversation each, the pane remembering which)
  keep the maker fenced and the catalogue out of code turns, at the price
  of the agent not being able to write a verb and use it in one turn.
  *Recommend: A5 ports today's fenced maker unchanged, so the swap is
  testable against the existing suites; then add `/code` beside it as a
  second profile; then try the combined one as a third and let use decide
  which is the default.* The bridge is the same for all three.
- **D6 — environment variable names.** *Recommend `EDIT_AI_MODEL`,
  `EDIT_AI_MAX_TURNS`, `EDIT_CLAUDE_BIN`, `EDIT_AI_DEBUG`,
  `EDIT_AI_NO_STREAM_LOG` in the crate,* with the modeler's profile
  reading its `MODELER_AI_*` names so nothing a harness sets stops working.
