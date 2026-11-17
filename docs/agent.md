# The agent in `edit`

`edit` has Claude in a pane: a programming assistant that runs on your
Claude subscription, sees what you are looking at, reads and edits your
buffers and files, runs programs with your leave, and can drive the editor
itself. This is how to use it and how to extend it.

It is Claude Code underneath — the `claude` command-line program, started
by the editor and spoken to over a pipe — so it has Claude Code's tools,
reads the project's `CLAUDE.md`, and honours the permissions you have
already set there. Nothing in the editor holds a credential.

How it came to be built this way, with the measurements behind each
choice, is [agent-panel-plan.md](agent-panel-plan.md). This file is the
manual.

## What you need

- **Claude Code**, installed and on the `PATH`: the `claude` program.
  `EDIT_CLAUDE_BIN` names one elsewhere.
- **A Claude subscription** you are logged into. `/login` in the pane does
  it if you are not.

Nothing else. Without `claude` the pane says so and the rest of the editor
is unaffected.

## Starting

    cargo run --release -p edit -- some/file.rs

Then **`M-a`**, or the robot icon at the right end of the status strip. The
agent's column opens on the right with its prompt, `agent> `. Type a
request in English and press Return.

| key | where | does |
|---|---|---|
| `M-a` | anywhere | go to the agent; from its own prompt, hide it |
| robot icon | status strip | show or hide the column |
| `Return` | `agent>` | send the request |
| `Escape` | `agent>`, while it works | interrupt the turn; the keyboard stays |
| `Escape` | `agent>`, idle | back to the editor |
| `C-l` | agent pane | new conversation |
| `PageUp` / `PageDown` | agent pane | scroll the transcript |
| `Up` / `Down` | `agent>` | earlier requests |
| `Cmd-C` / `Cmd-V` | agent pane | copy the selection (drag to select) / paste |
| `C-x o` | anywhere | round the panes that are showing |
| right-click | agent pane | its menu; on a code block, the block's |

The column's left edge drags to resize it. Nothing is started until the
first request, and a turn goes on while the column is hidden.

## Logging in

The agent uses the login `claude` already has. If there is none:

    /login

Your browser opens. If the page shows a code, paste it at `agent>` — the
next line you type goes to the login, not to the agent. `/status` says who
is logged in and on what plan; `/logout` logs out.

**It is your subscription, not an API key.** `/login` asks for the
subscription login outright, and the editor removes `ANTHROPIC_API_KEY`,
`ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_BASE_URL` from the agent's
environment, so a key left in your shell for something else cannot route a
turn to API billing. `/status` says so loudly if the login is ever an API
one.

## What it knows without being told

Every request carries a short note the editor writes, not you:

- the file in front of you, its language, and whether it has unsaved edits;
- the cursor's line and column, and that line's text;
- your selection, when there is one;
- the other buffers open, and which have unsaved edits.

So "what does this do", "fix this function" and "the line my cursor is on"
mean the place you are looking at. The note is names and a selection,
never a file's contents: the agent reads files itself, and a small note
keeps each turn fast.

It runs in the project the file belongs to — the git checkout, else the
file's directory — so the project's `CLAUDE.md` is read and relative paths
mean what they mean to you.

## What it can do

It has Claude Code's own tools (read, edit and write files; search; run
commands; fetch web pages) and the editor's:

| tool | does | asks first |
|---|---|---|
| `buffers` | what is open: name, file, language, unsaved?, which is on screen, the cursor's line | no |
| `buffer_read` | a buffer as you see it, unsaved edits included | no |
| `buffer_edit` | replace one piece of text in a buffer, in place | no |
| `buffer_write` | replace a buffer's whole text | no |
| `buffer_save` | save a buffer to its file | no |
| `buffer_close` | close a buffer — not over unsaved edits unless told to discard them | no |
| `diagnostics` | what the editor finds wrong with a buffer's text right now | no |
| `open_file` | show a file in the editor, at a line | no |
| `editor_commands` | list the editor's commands, or search them by name | no |
| `editor_command` | run a command that only changes what the editor shows | no |
| `editor_eval` | evaluate Scheme in the editor's own image | **yes** |
| `eval` | evaluate code where the buffer's sends go — its scm, ray, Guile or evcxr | **yes** |

**Edits to a buffer are yours to keep or undo.** `buffer_edit` and
`buffer_write` change the buffer in place, unsaved, as one undo step each:
one undo takes an edit back, and nothing is written to disk until you save
(or ask the agent to).

**Edits to a file reach the buffer.** When the agent changes a file on
disk — with its own tools, or by running something that rewrites it — a
buffer on that file with no unsaved edits is reloaded, the cursor kept on
its line, and the transcript says `↻ reloaded NAME`. A buffer **with**
unsaved edits is never overwritten: the pane says, in red, that the file
changed underneath it, and how to choose (`:e!` takes the file's version,
`C-x C-s` keeps yours). `M-x sync-buffers-with-disk` does the same reload by
hand.

**Driving the editor.** Ask for the thing and it is done: "switch to the
dracula theme", "open the REPL", "open notes.md", "save this and close it".
`editor_command` runs the commands that only change what the editor shows —
a theme (`load-theme`; `save-theme-choice` makes it the default), the REPL,
terminal, file-tree and agent panels, the text size — with literal
arguments and without asking. Anything else is `editor_eval`, which can do
whatever the editor can (bind a key, define a command) and so asks.

## Permission

The agent reads and edits files **in the project** without asking, and
runs commands that only look (`git status`, `ls`). For anything else — a
program run, a file read or written **outside** the project, the two tools
that evaluate — it asks in the pane and waits:

    ? Bash cargo test -p edit
      y allow · n deny · a allow every Bash call in this conversation · /auto stops asking · or say what to do instead

The next line you type is the answer:

| answer | means |
|---|---|
| `y` | allow, and remember (see below) |
| `once` | allow this call; remember nothing |
| `a` | allow this and every later call of that tool, this conversation |
| `n` | refuse |
| anything else | refuse, and the agent is told what you typed — "use cargo check instead" |

**What `y` remembers.** For a file, its **directory** and everything under
it, from then on in this project — to write (which is also to read) when a
write was allowed, to read when a read was. For a command, that exact
command for the rest of the conversation. The question's own line says
which.

**Automatic mode.** `/auto` answers every question yes as it arrives.
Each action is still shown, as a `✓` row, and the prompt reads
`agent (auto)> ` for as long as it holds. It is per project and kept
across restarts; `/auto off` goes back to asking. In this mode the agent
runs any command and writes anywhere your account can, unasked.

`/grants` lists what has been granted in this project; `/revoke` forgets
all of it, automatic mode included. Permissions you have already allowed in
Claude Code's own settings apply here too.

## Code in a reply

A fenced block in a reply is drawn as a block: a header naming its
language, then its lines on the editor's own background. Rust, Scheme and
Guile are coloured as the editor colours them; other Lisps get simpler
colours; anything else is plain.

**Right-click a block** — its header or any line — for its menu: a click
gives the list, a hold or a drag the marking menu.

| item | does |
|---|---|
| Copy | the block to the clipboard |
| Insert at cursor | into the buffer on screen, at the cursor |
| Append to buffer | onto the end of that buffer, after a blank line |

An insertion is one undo step.

## Slash commands

| command | does |
|---|---|
| `/help` | list these |
| `/login`, `/logout`, `/status` | the account |
| `/usage` | the plan's limits (percent used, when each resets) and what this conversation has used |
| `/new` | a new conversation (also `C-l`) |
| `/model NAME` | `opus`, `sonnet`, `haiku` or a full id, from the next request; `/model` alone says which; `/model default` goes back |
| `/auto`, `/auto off` | stop asking permission; ask again |
| `/grants` | what has been granted in this project |
| `/revoke` | forget every grant, automatic mode included |

Anything else is a request to the agent.

## The transcript

| row | is |
|---|---|
| `you> …` | what you asked |
| plain text | the agent's words, drawn as they are written |
| `→ Edit src/main.rs` | a tool it called, with the argument worth showing |
| dim text under a tool | what the tool answered, its first lines |
| `↻ reloaded NAME` | a buffer brought up to its changed file |
| `✎ edited NAME` | an edit made in a buffer |
| `⌘ load-theme :dracula` | an editor command it ran |
| `? …` | a question, waiting for your answer |
| `✓ …` | something allowed without asking (a grant, or automatic mode) |
| `✗ …` (red) | something refused, or a buffer the agent's change could not reach |
| `· 5s (4s api) · 3 turns · …` | the turn's meter: time, model turns, tool calls, context, output tokens, cache use, cost at API prices |
| `— interrupted —` | you pressed Escape |

Files it has read back, and an edit's own "updated", get no row: the
reload says it.

## Conversations

A conversation belongs to the project it runs in and **survives a
restart**: the next request there picks it up. `C-l` or `/new` starts
over. Interrupting keeps the conversation; so does changing the model.

`/usage` shows the context the conversation has grown to against the
model's window. A long one is compacted by Claude Code itself, and the
turn's meter says `COMPACTED` when that happens.

## Files it keeps

| file | holds |
|---|---|
| `~/.config/edit/agent-sessions.scm` | each project's conversation id |
| `~/.config/edit/agent-grants.scm` | each project's grants, and whether automatic mode is on |

Both are readable Scheme and safe to edit or delete. The conversations
themselves are Claude Code's, under `~/.claude/projects/`.

## Settings

Environment, read when a conversation's process starts:

| variable | does |
|---|---|
| `EDIT_CLAUDE_BIN` | the `claude` to run, instead of the `PATH`'s |
| `EDIT_AI_MODEL` | the model when neither `/model` nor the profile names one (default `opus`) |
| `EDIT_AI_NO_STREAM_LOG` | do not keep the raw stream, where a profile asked for it |

In `~/.config/edit/init.scm`:

| variable | is |
|---|---|
| `*agent-model*` | the model, as `/model` sets it; `#f` for the profile's |
| `*agent-profile*` | a procedure answering the agent's profile (below) |
| `*agent-ui-commands*` | the commands `editor_command` may run unasked |
| `*agent-selection-limit*` | how much of a selection rides with a request (4000 characters) |
| `*agent-session-file*`, `*agent-grants-file*` | where the two files above are |

## Extending it

All of this is Scheme in `edit/scm/25-agent.scm`, loaded like the rest of
the editor's shell, so your `init.scm` can change any of it.

### A tool of your own

    (define-agent-tool "word_count"
      "Count the words in an open editor buffer."
      '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen."))
      (lambda (args reply)
        (let* ((buf (%agent-buffer args))
               (text (%agent-in-buffer buf (lambda () (%editor-text (editor-pane-id))))))
          (reply (number->string (length (filter (lambda (w) (not (equal? w "")))
                                                 (string-split text " "))))))))

The parameters are `(name type description required?)`, with a type of
`"string"`, `"integer"` or `"boolean"`. The handler gets the arguments as
an alist of `(name . text)` and a `reply` procedure: call `(reply text)`
now, or later when the answer comes from somewhere else, or
`(reply text #t)` for a failure. An error the handler raises is the tool's
failure. A fifth argument of `#t` makes the tool ask leave before each
call. A call with an argument the tool does not take is answered with the
ones it does.

A tool defined after a conversation has started is offered from the next
`/new`.

`(run-agent-tool NAME ARGS ANSWER)` runs a tool as the agent would — a way
to try one by hand.

### An unasked editor command

    (set! *agent-ui-commands* (cons "my-command" *agent-ui-commands*))

`editor_command` then runs it without asking, with literal arguments only
(strings, numbers, booleans, `:keywords`, `'symbols`).

### The profile

What kind of agent this is — where it runs, what it may do, what its
prompt adds — is an alist, the **profile**. The default is the code
profile, `(agent-code-profile)`; build on it:

    (set! *agent-profile*
      (lambda ()
        (agent-profile-extend (agent-code-profile)
          :model "sonnet"
          :append-system-prompt "This project's tests run with `just test`.")))

| key | is |
|---|---|
| `:cwd` | where it runs (default: the buffer's project) |
| `:model` | a model alias or id |
| `:permission-mode` | Claude Code's mode (default `acceptEdits`) |
| `:permission-prompt-tool` | who is asked (default `stdio`: the pane) |
| `:allowed-tools`, `:denied-tools` | tools that run unasked; tools removed outright |
| `:append-system-prompt` | text added to Claude Code's own prompt |
| `:system-prompt` | text that **replaces** it |
| `:mcp-config` | other MCP servers, as the JSON `claude --mcp-config` takes |
| `:strict-mcp` | only those servers |
| `:setting-sources` | which of Claude Code's settings load; `""` for none |
| `:max-turns` | a cap on model turns in one request |
| `:tool-prefix` | a prefix stripped from tool names in the transcript |
| `:stream-log` | a directory to keep each process's raw stream in |
| `:bin` | the `claude` to run |
| `:arg` | an extra argument to `claude`; may repeat |

`agent-profile-extend` replaces a key, removes it when given `#f`, and
adds to `:arg` rather than replacing it. A profile key the editor does not
know is an error, not a setting silently dropped. A change to the profile
that the running process cannot take (its directory, model, tools, prompt)
restarts it, resumed into the same conversation.

### Hooks

Each is a variable holding a procedure; `set!` it.

| hook | called | to |
|---|---|---|
| `*agent-prepare*` | `(text)` → the message sent | put more in front of a request (the default adds the editor note) |
| `*agent-row*` | `(kind a b)` → `(style text)` or `#f` | show a step your own way |
| `*agent-turn-ended*` | `(status)`: `"done"`, `"error"`, `"cancelled"` | act when a turn is over |
| `*agent-shown*` | `(pane-id)` | make the pane visible (the host's) |
| `*agent-session-ref*`, `*agent-session-set!*` | by directory | keep conversation ids somewhere else |
| `*agent-grants-ref*`, `*agent-grants-set!*` | by project | keep grants somewhere else |

Useful procedures: `(agent-ask TEXT)` sends a request as if typed;
`(agent-interrupt)`; `(new-conversation)`; `(focus-agent)`;
`(agent-editor-context)`, the note as a string; `(agent-granted? TOOL PATH)`.

## How it works

- **One process per conversation.** The editor starts
  `claude -p --input-format stream-json --output-format stream-json`, and
  each request is a line on its standard input. Keeping the process alive
  is what makes a turn start at once: a process per turn spent about a
  second starting before the model was asked.
- **Escape is a request, not a kill.** The process is asked to stop and
  stays up with the conversation. One that has not answered in three
  seconds is killed, and the next request resumes the conversation in a new
  process.
- **Replies stream.** The model's text arrives in pieces and is drawn on
  the rows the finished message lands on.
- **Permission and the editor's tools use the same pipe.** Claude Code asks
  its host — the editor — whether a call may go ahead, and for the editor's
  tools, on the conversation's own channel. There is no second process, no
  socket and no port, and a tool's handler is Scheme on the thread that
  owns the panes.
- **The prompt stays cached.** What is fixed for a conversation is in the
  system prompt; what changes per request — the editor note — is in the
  request. The meter's `cache … read / … new` is where a prefix that has
  stopped caching shows.

| code | is |
|---|---|
| `edit/src/agent.rs` | the bridge: the process, the stream, interrupts, permission questions, the tool server, login, usage |
| `edit/src/scm_binding/agent.rs` | the `%agent-*` prims |
| `edit/scm/25-agent.scm` | the pane: everything in this manual |
| `edit/scm/host/standalone.scm` | where the standalone puts it: the column, `M-a`, the files it keeps |
| `edit/src/widget/textpane.rs` | the transcript widget, code blocks included |

**Tests** run against a stand-in `claude`, a shell script that speaks the
stream, so the suite spends nothing. Five are `#[ignore]`d because they
spend real turns on your login; run one by name to check the real thing:

    cargo test -p edit --lib the_real_claude_holds -- --ignored --nocapture
    cargo test -p edit --lib the_real_claude_fixes -- --ignored --nocapture
    cargo test -p edit --lib the_real_claude_works_in -- --ignored --nocapture
    cargo test -p edit --lib the_real_claude_drives -- --ignored --nocapture
    cargo test -p edit --lib the_real_claude_shows_rust -- --ignored --nocapture

## Limits, and when something is wrong

- **"no `claude` on PATH".** Install Claude Code, or set `EDIT_CLAUDE_BIN`.
- **"Not logged in".** `/login`.
- **It answers from a stale file.** A buffer with unsaved edits is newer
  than its file. The agent is told, and should read it with `buffer_read`;
  saving the buffer settles it.
- **Your buffer did not take the agent's change.** It had unsaved edits.
  The red row says how to choose.
- **A reload cost the buffer its undo.** A reload from disk is a load: undo
  does not reach back across it. Edits the agent makes with `buffer_edit`
  do undo.
- **Insert and Append go to the buffer on screen**, whichever it is.
- **A failed `buffer_edit` shows only as the tool called twice.** The
  stream does not mark which results are errors; the agent retries.
- **Context.** The editor's tools are loaded with every conversation, with
  the tools of any connectors on your Claude account: roughly 20,000 tokens
  of cached context before the first word.
- **The modeler.** ray's modeler still has its own, older agent pane;
  replacing it with this one is planned and not done.
