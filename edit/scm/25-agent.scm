;## 25-agent.scm — the agent pane (docs/agent-panel-plan.md, A2): a
;## conversation with Claude Code, on the login the `claude` CLI already
;## holds. The REPL's shape with a different prompt — one input line, a
;## transcript above — and what you type is a request in English. The
;## process, its stream and its login are the kernel's (src/agent.rs); this
;## is what the pane says and what its keys do. It came from the modeler's
;## 08-editor, which keeps only what is the modeler's to say.

; What the HOST supplies, as FORWARD HOOKS:
;   (*pane-id* :agent)   the pane the conversation shows in
;   *agent-shown*        make that pane visible (a dock opens)
;   *agent-profile*      the agent this host wants, as an alist — where it
;                        runs, what it may do, what its prompt adds. The
;                        default is the CODE profile, below; a host builds
;                        on it with agent-profile-extend.
;   *agent-prepare*      what you typed -> the message sent. Here: what
;                        the editor shows, then the request
;                        (agent-with-editor-context). The modeler puts its
;                        scene and matching examples in front as well.
;   *agent-row*          (kind a b) -> (style text), or #f for the row
;                        below: how this host shows a step
;   *agent-session-ref* / *agent-session-set!*
;                        the conversation's id, by the directory it runs
;                        in. Kept in memory here; a host with a session
;                        store keeps it across restarts.
;   *agent-turn-ended*   (status) once a turn is over: "done", "error" or
;                        "cancelled"
(define *agent-shown* (lambda (id) #f))
(define *agent-prepare* (lambda (text) (agent-with-editor-context text)))
(define *agent-row* (lambda (kind a b) #f))
(define *agent-turn-ended* (lambda (status) #f))

(define (agent-id) (*pane-id* :agent))

; ── The profile ────────────────────────────────────────────────────────────
; An alist of what src/agent.rs's Profile reads: :cwd :model :max-turns
; :setting-sources :system-prompt :append-system-prompt :mcp-config
; :strict-mcp :allowed-tools :denied-tools :permission-mode
; :permission-prompt-tool :tool-prefix :stream-log :exclude-dynamic :bin,
; and :arg, which may repeat. A key that is absent is the CLI's own default.

(define agent-editor-note
  "You are running inside `edit`, a text editor, in a side panel: the user reads your replies in a narrow transcript beside their source, so keep prose short and put code in fenced blocks. A fenced code block in your reply is drawn as a block the user can copy, or put into their buffer, with one click: put an example they may want to use in one, name its language on the fence, and keep it to code that stands on its own. Files you change on disk are the files open in the editor. Each request opens with an <editor> block the editor wrote, not the user: the file they are looking at, where their cursor is, the line it is on, what they have selected, and the other files open. \"This\", \"here\", \"this function\" mean that place. You also have the editor's own tools (mcp__edit__*): buffers, buffer_read, buffer_edit, buffer_write, buffer_save, buffer_close, diagnostics, open_file, eval, and — to drive the editor itself — editor_commands, editor_command and editor_eval. When the user asks for something of the EDITOR (a colour theme, the REPL, the terminal, a file opened, a buffer saved or closed) do it with these rather than explaining how: open_file opens a file in a buffer, buffer_save and buffer_close save and close one, and editor_command runs a UI command such as load-theme or repl (editor_commands lists them). A file marked as having unsaved edits is NEWER in the editor than on disk: Read would show you a stale copy and Edit would be refused by the editor, so read it with buffer_read and change it with buffer_edit, which edits the buffer in place where the user watches and can undo. For a file with no unsaved edits either way works, and the editor reloads it as soon as you change it on disk. After a change, diagnostics tells you what the editor now finds wrong.")

; ── What the editor shows, for the agent ───────────────────────────────────
; The agent reads files itself; what it cannot see is the USER — which file
; is in front of them, where the cursor is, what is selected, what else is
; open and which of it is not saved. That rides in front of every request,
; and stays small: names and a selection, never a file's contents. In the
; REQUEST, not the system prompt, which must not change from turn to turn
; if it is to stay cached.
(define *agent-selection-limit* 4000)

(define (agent-editor-context)
  "(agent-editor-context)  ->  a string, or #f with no editor pane

What the editor shows, as the agent is told it: the buffer in front, its
language, the cursor's line and column and that line's text, the selection
when there is one, and the other buffers open — each marked when it has
edits that are not saved."
  (let ((eid (editor-pane-id)))
    (and eid
         (let* ((buf (%editor-active-buffer eid))
                (dirty (lambda (b)
                         (any (lambda (row) (and (equal? (car row) b) (positive? (cadr row))))
                              (%editor-buffer-list eid))))
                (label (lambda (b)
                         (string-append (or (buffer-path b) (%text (buffer-name b)))
                                        (if (dirty b) " (unsaved edits)" ""))))
                (cursor (%editor-cursor eid))
                (lines (string-split (%editor-text eid) "\n"))
                (line (%list-ref/default lines (car cursor) ""))
                (language (%buffer-language eid buf))
                (selection (%editor-act eid "copy"))
                (others (filter (lambda (b) (not (equal? b buf))) (%buffer-mru eid))))
           (string-append
             "<editor>\n"
             "file: " (label buf)
             (if (and (string? language) (not (equal? language ""))) (string-append " — " language) "")
             "\n"
             "cursor: line " (number->string (+ 1 (car cursor)))
             ", column " (number->string (+ 1 (cadr cursor)))
             " of " (number->string (length lines)) " lines\n"
             ; (Its indentation kept: where in the line the cursor is matters.)
             "that line: " (if (> (string-length line) 300) (substring line 0 300) line) "\n"
             (if (and (string? selection) (not (equal? selection "")))
                 (string-append "selection:\n" (%agent-cut selection *agent-selection-limit*) "\n")
                 "")
             (if (null? others)
                 ""
                 (string-append "also open: "
                                (apply string-append
                                       (cdr (apply append (map (lambda (b) (list ", " (label b))) others))))
                                "\n"))
             "</editor>")))))

(define (agent-with-editor-context text)
  "TEXT as a request, with what the editor shows in front of it."
  (let ((context (guard (e (#t #f)) (agent-editor-context))))
    (if context (string-append context "\n\n" text) text)))

(define (agent-project-dir)
  "Where the agent runs: the git checkout the buffer's file is in, else
  that file's directory, else the editor's own."
  (let* ((eid (editor-pane-id))
         (path (and eid (buffer-path (%editor-active-buffer eid))))
         ; The last "/" of the path, if it has one past the root.
         (slash (and (string? path)
                     (let loop ((i (- (string-length path) 1)))
                       (cond ((< i 1) #f)
                             ((char=? (string-ref path i) #\/) i)
                             (else (loop (- i 1)))))))
         (dir (if slash (substring path 0 slash) ".")))
    (or (%git-root dir) (%os-realpath dir))))

(define (agent-code-profile)
  "The CODE profile: a programming assistant in the project. It runs where
  the buffer's file lives, so the project's CLAUDE.md is read. It reads the
  project and edits its files unasked (`acceptEdits`), and runs commands
  that only look (git status, ls). For anything else — running a program,
  reading or writing a file OUTSIDE the project — it ASKS in the pane and
  waits for your answer (measured 2026-11-17). Rules you have already
  allowed in Claude Code's own settings still hold."
  (list (cons :cwd (agent-project-dir))
        (cons :permission-mode "acceptEdits")
        (cons :permission-prompt-tool "stdio")
        (cons :append-system-prompt agent-editor-note)))

(define *agent-profile* agent-code-profile)

; The model /model chose, over the profile's; #f is the profile's own.
(define *agent-model* #f)

(define (agent-profile-now)
  "The profile the next request goes out under: the host's, the model
  /model chose, and the editor's own tools (define-agent-tool) — offered
  with whatever servers the host's profile names, and allowed unasked
  unless a tool was defined to ask."
  (let* ((p (*agent-profile*))
         (p (if *agent-model* (agent-profile-extend p :model *agent-model*) p)))
    (if (or (null? *agent-tools*) (%alist-ref p :tools #f))
        p
        (let ((mine (%agent-join (map (lambda (t) (string-append "mcp__edit__" (car t)))
                                      (filter (lambda (t) (not (list-ref t 4))) *agent-tools*))
                                 ","))
              (theirs (%alist-ref p :allowed-tools #f)))
          (agent-profile-extend p
            :tools (agent-tools-json)
            :allowed-tools (if theirs (string-append theirs "," mine) mine))))))

; ── The editor's tools, for the agent (A6) ─────────────────────────────────
; What the agent's own file tools cannot reach: a buffer as the user sees
; it, unsaved edits and all; an edit made IN the buffer, which one `u`
; takes back; what the reader and the language server say is wrong; the
; image the buffer's forms are sent to. They are served to the CLI on the
; conversation's own channel (src/agent.rs), so a call is an event here and
; its handler is Scheme, on the thread that owns the panes.
(define *agent-tools* '())   ; each (name description params handler ask?)

(define (define-agent-tool name description params handler (ask #f))
  "(define-agent-tool NAME DESCRIPTION PARAMS HANDLER &opt ASK)  ->  NAME

Offer the agent a tool. PARAMS is a list of (name type description
required?) — type \"string\", \"integer\" or \"boolean\". HANDLER is called
with the call's arguments as an alist of (name . text) and a procedure
REPLY: call (reply TEXT) with what the tool says — now, or later when the
answer is asynchronous — or (reply TEXT #t) when it failed. An error HANDLER
raises is the tool's failure. With ASK, the agent must have your leave for
each call, as it must to run a program. A tool defined again replaces the
one of that name; a process already running keeps the tools it started
with until /new."
  (set! *agent-tools*
    (append (filter (lambda (t) (not (equal? (car t) name))) *agent-tools*)
            (list (list name description params handler ask))))
  name)

(define (%agent-join strings between)
  (if (null? strings)
      ""
      (apply string-append (cdr (apply append (map (lambda (s) (list between s)) strings))))))

(define (agent-tools-json)
  ; The tools as the kernel hands them to the CLI: MCP's tool list.
  (define (param p)
    (string-append (%json-string (car p)) ":{\"type\":" (%json-string (cadr p))
                   ",\"description\":" (%json-string (caddr p)) "}"))
  (define (tool t)
    (let ((params (caddr t)))
      (string-append
        "{\"name\":" (%json-string (car t))
        ",\"description\":" (%json-string (cadr t))
        ",\"inputSchema\":{\"type\":\"object\",\"properties\":{" (%agent-join (map param params) ",") "}"
        ",\"required\":["
        (%agent-join (map (lambda (p) (%json-string (car p))) (filter (lambda (p) (%list-ref/default p 3 #f)) params)) ",")
        "]}}")))
  (string-append "[" (%agent-join (map tool *agent-tools*) ",") "]"))

(define (run-agent-tool name args answer)
  "(run-agent-tool NAME ARGS ANSWER)

Run the editor tool NAME as the agent would, with ARGS an alist of (name .
text), and hand what it says to ANSWER as (ANSWER text failed?) — once,
however the handler ends. How a call from the agent is run, and a way to
try a tool by hand."
  (let* ((tool (find (lambda (t) (equal? (car t) name)) *agent-tools*))
         (answered #f)
         (reply (lambda (text (failed #f))
                  (unless answered
                    (set! answered #t)
                    (answer (%text text) (if failed #t #f))))))
    ; An argument the tool does not take, or one it needs and was not given,
    ; is answered with what it DOES take: a call made on a guess is
    ; corrected by its own error.
    (define (takes)
      (let ((params (caddr tool)))
        (if (null? params)
            "it takes no arguments"
            (string-append "it takes: "
                           (%agent-join (map (lambda (p)
                                               (string-append (car p) " (" (cadr p)
                                                              (if (%list-ref/default p 3 #f) ", required)" ")")))
                                             params)
                                        ", ")))))
    (cond ((not tool) (reply (string-append "the editor has no tool named " name) #t))
          ((find (lambda (kv) (not (assoc (car kv) (caddr tool)))) args)
            => (lambda (kv) (reply (string-append name " has no argument " (car kv) " — " (takes)) #t)))
          ((find (lambda (p) (and (%list-ref/default p 3 #f) (not (assoc (car p) args)))) (caddr tool))
            => (lambda (p) (reply (string-append name " needs " (car p) " — " (takes)) #t)))
          (else
            (guard (e (#t (reply (%agent-error-text e) #t)))
              ((list-ref tool 3) args reply))))))

(define (agent-call id name args)
  ; A call from the agent: its answer goes back under the call's id.
  (run-agent-tool name args (lambda (text failed) (%agent-tool-result id text failed))))

(define (%agent-arg args name (default #f))
  (let ((kv (assoc name args)))
    (if (and kv (not (equal? (cdr kv) ""))) (cdr kv) default)))

(define (%agent-buffer args)
  ; The buffer a call names — by its name or its file — or the one on
  ; screen; an error that says which are open when it names none of them.
  (let ((eid (editor-pane-id)) (which (%agent-arg args "buffer")))
    (cond ((not which) (%editor-active-buffer eid))
          ((buffer-by-name which))
          ((buffer-for-file which))
          ((and *agent-turn-dir* (buffer-for-file (string-append *agent-turn-dir* "/" which))))
          (else (raise (string-append "no open buffer " which " — open: " (%agent-join (buffer-names) ", ")))))))

(define (%agent-in-buffer buf thunk)
  ; THUNK with BUF as the editor's buffer, and the user's own back after —
  ; whatever THUNK does.
  (let* ((eid (editor-pane-id)) (cur (%editor-active-buffer eid)))
    (if (equal? cur buf)
        (thunk)
        (begin
          (%editor-buffer-switch eid buf)
          (let ((r (guard (e (#t (%editor-buffer-switch eid cur) (raise e))) (thunk))))
            (%editor-buffer-switch eid cur)
            r)))))

(define (%agent-buffer-label buf)
  (or (buffer-path buf) (%text (buffer-name buf))))

(define-agent-tool "buffers"
  "List the buffers open in the user's editor: each one's name, file, language, whether it has unsaved edits, and which is on screen with the cursor's line. Call this to see what the user has open."
  '()
  (lambda (args reply)
    (let* ((eid (editor-pane-id)) (cur (%editor-active-buffer eid)))
      (reply
        (%agent-join
          (map (lambda (row)
                 (let ((b (car row)))
                   (string-append
                     (%text (buffer-name b))
                     (if (buffer-path b) (string-append " — " (buffer-path b)) " — no file")
                     " — " (%text (%buffer-language eid b))
                     (if (positive? (cadr row)) " — UNSAVED EDITS" "")
                     (if (equal? b cur)
                         (string-append " — ON SCREEN, cursor on line " (number->string (+ 1 (car (%editor-cursor eid)))))
                         ""))))
               (%editor-buffer-list eid))
          "\n")))))

(define-agent-tool "buffer_read"
  "Read an open editor buffer AS THE USER SEES IT, unsaved edits included — unlike Read, which reads the file on disk. Lines come back numbered as `N<tab>text`. Use it for any file the <editor> block marks as having unsaved edits, and for a buffer that has no file."
  '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen.")
    ("from" "integer" "First line to return, from 1. Omit for the start.")
    ("to" "integer" "Last line to return. Omit for the end."))
  (lambda (args reply)
    (let* ((buf (%agent-buffer args))
           (lines (string-split (%agent-in-buffer buf (lambda () (%editor-text (editor-pane-id)))) "\n"))
           (n (length lines))
           (from (max 1 (or (string->number (%agent-arg args "from" "1")) 1)))
           (to (min n (or (string->number (%agent-arg args "to" (number->string n))) n))))
      (reply
        (let loop ((i 1) (ls lines) (acc '()))
          (cond ((or (null? ls) (> i to)) (%agent-join (reverse acc) "\n"))
                ((< i from) (loop (+ i 1) (cdr ls) acc))
                (else (loop (+ i 1) (cdr ls) (cons (string-append (number->string i) "\t" (car ls)) acc)))))))))

(define-agent-tool "buffer_edit"
  "Edit an open editor buffer IN PLACE: replace the one occurrence of `old` with `new`. The change appears in the editor at once, unsaved, and the user can undo it with one undo — prefer this to Edit for a file open in the editor, and it is the ONLY way to change a buffer with unsaved edits or no file. `old` must occur exactly once (it does not include the line numbers buffer_read shows); include enough surrounding text to make it so."
  '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen.")
    ("old" "string" "The exact text to replace, occurring once in the buffer." #t)
    ("new" "string" "The text to put in its place." #t))
  (lambda (args reply)
    (let* ((buf (%agent-buffer args))
           (found (%agent-in-buffer buf
                    (lambda () (%editor-edit (editor-pane-id) (%agent-arg args "old" "") (or (%agent-arg args "new") ""))))))
      (cond ((eqv? found 1)
              (agent-append (string-append "✎ edited " (%text (buffer-name buf)) " — unsaved; undo takes it back") :dim)
              (reply (string-append "Edited " (%agent-buffer-label buf) " in the editor. It is unsaved; the user saves it.")))
            ((eqv? found 0) (reply "`old` does not occur in that buffer. Read it again with buffer_read: it may have changed." #t))
            (else (reply (string-append "`old` occurs " (number->string found) " times; include more of the surrounding text so that it occurs once.") #t))))))

(define-agent-tool "buffer_write"
  "Replace the WHOLE text of an open editor buffer. The change appears in the editor at once, unsaved, and one undo takes it back. For a small change use buffer_edit."
  '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen.")
    ("text" "string" "The buffer's new text, all of it." #t))
  (lambda (args reply)
    (let ((buf (%agent-buffer args)))
      (%agent-in-buffer buf (lambda () (%editor-rewrite (editor-pane-id) (or (%agent-arg args "text") ""))))
      (agent-append (string-append "✎ rewrote " (%text (buffer-name buf)) " — unsaved; undo takes it back") :dim)
      (reply (string-append "Rewrote " (%agent-buffer-label buf) " in the editor. It is unsaved; the user saves it.")))))

(define-agent-tool "diagnostics"
  "What the editor says is wrong with an open buffer right now: what its reader cannot read, a form left open, and the language server's errors and warnings — for the text as it stands, unsaved edits included."
  '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen."))
  (lambda (args reply)
    (let* ((buf (%agent-buffer args))
           (found (%agent-in-buffer buf (lambda () (%editor-diagnostics (editor-pane-id))))))
      (reply
        (if (null? found)
            "No diagnostics: the editor finds nothing wrong with it."
            (%agent-join
              (map (lambda (d)
                     (string-append
                       "line " (number->string (+ 1 (car d))) ", column " (number->string (+ 1 (cadr d))) ": "
                       (let ((sev (%list-ref/default d 3 1)))
                         (cond ((eqv? sev 1) "error: ") ((eqv? sev 2) "warning: ") (else "left open: ")))
                       (%text (caddr d))))
                   found)
              "\n"))))))

(define-agent-tool "open_file"
  "Show a file to the user: open it in the editor (or switch to its buffer) and put the cursor on a line. Use it to point at the place you are talking about."
  '(("path" "string" "The file, absolute or relative to the project." #t)
    ("line" "integer" "The line to put the cursor on, from 1."))
  (lambda (args reply)
    (let* ((given (%agent-arg args "path" ""))
           (path (if (or (string-prefix? "/" given) (not *agent-turn-dir*)) given (string-append *agent-turn-dir* "/" given)))
           (line (string->number (%agent-arg args "line" ""))))
      (if (not (file-exists? path))
          (reply (string-append "no such file: " path) #t)
          (begin
            (visit-file-text path)
            (when line (%editor-goto-line (editor-pane-id) (max 0 (- line 1))))
            (reply (string-append "Showing " path (if line (string-append " at line " (number->string line)) ""))))))))

(define-agent-tool "buffer_save"
  "Save an open editor buffer to its file, as the user's C-x C-s does. Use it after buffer_edit when the user asked for the change to be saved."
  '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen."))
  (lambda (args reply)
    (let* ((eid (editor-pane-id)) (buf (%agent-buffer args)) (path (buffer-path buf)))
      (cond ((not path) (reply "that buffer has no file: tell the user, who can save it under a name (C-x C-s)" #t))
            (else
              (if (equal? buf (%editor-active-buffer eid)) (save-buffer) (save-buffer-text! buf))
              (hash-table-set! *agent-file-marks* path (%agent-file-mark path))
              (agent-append (string-append "✓ saved " (%text (buffer-name buf))) :dim)
              (reply (string-append "Saved " path)))))))

(define-agent-tool "buffer_close"
  "Close an open editor buffer. One with unsaved edits is NOT closed unless you buffer_save it first, or pass discard=true because the user said to throw the edits away."
  '(("buffer" "string" "The buffer's name or its file's path. Omit for the buffer on screen.")
    ("discard" "boolean" "true to close it even with unsaved edits, losing them."))
  (lambda (args reply)
    (let* ((eid (editor-pane-id))
           (buf (%agent-buffer args))
           (cur (%editor-active-buffer eid))
           (name (%text (buffer-name buf)))
           (dirty (any (lambda (row) (and (equal? (car row) buf) (positive? (cadr row)))) (%editor-buffer-list eid))))
      (if (and dirty (not (equal? (%agent-arg args "discard" "false") "true")))
          (reply (string-append name " has unsaved edits: buffer_save it first, or ask the user whether to discard them") #t)
          (begin
            (%editor-buffer-switch eid buf)
            ; (kill-buffer asks before it drops unsaved edits; these were
            ; just declared discarded.)
            (when dirty (%editor-mark-saved eid))
            (kill-buffer)
            (unless (equal? cur buf) (visit-buffer cur))
            (reply (string-append "Closed " name)))))))

; THE EDITOR'S COMMANDS (Kevin, 2026-11-17: "loading a new theme, open a
; repl …"). The editor is Scheme, and every command a key or M-x runs is a
; procedure, so the agent drives the UI by calling them. Two doors: the
; commands NAMED HERE — ones that only change what the editor shows — are
; called unasked, with literal arguments; anything else is Scheme evaluated
; in the editor's own image, which can do whatever the editor can, and so
; asks. A host adds its own to the list.
(define *agent-ui-commands*
  '("focus-scene-editor" "focus-agent" "sync-buffers-with-disk" "show-keys"
    "text-size+" "text-size-" "set-text-size" "toggle-markdown-view"))

(define (%agent-contains? text part)
  (let ((n (string-length text)) (m (string-length part)))
    (let loop ((i 0))
      (cond ((> (+ i m) n) #f)
            ((string=? (substring text i (+ i m)) part) #t)
            (else (loop (+ i 1)))))))

(define (%agent-command-line name)
  ; "name — the first line of its docstring", for a list of commands.
  (let* ((doc (%doc-of name))
         (line (if doc
                   (let ((ls (filter (lambda (l) (not (equal? (string-trim-both l) "")))
                                     (string-split doc "\n"))))
                     ; A docstring may open with its own signature line.
                     (string-trim-both (if (and (pair? ls) (pair? (cdr ls)) (string-prefix? "(" (string-trim-both (car ls))))
                                           (string-append (car ls) " — " (cadr ls))
                                           (if (pair? ls) (car ls) ""))))
                   "")))
    (string-append name (if (equal? line "") "" (string-append " — " line)))))

(define-agent-tool "editor_commands"
  "List the editor's own commands — what its keys and M-x run: themes, panels, the REPL, the terminal, buffers, search. With no query: the ones editor_command may call unasked. With a query: every command whose name contains it, marked when it is one of those."
  '(("query" "string" "Part of a command's name, e.g. theme, repl, buffer."))
  (lambda (args reply)
    (let* ((query (%agent-arg args "query"))
           (free (filter (lambda (n) (procedure? (%env-ref (string->symbol n) #f))) *agent-ui-commands*))
           (names (if query
                      (filter (lambda (n) (%agent-contains? n query)) (list-sort string<? (command-candidates)))
                      free)))
      (reply
        (if (null? names)
            (string-append "no command has " query " in its name")
            (%agent-join
              (map (lambda (n)
                     (string-append (if (and query (member n free)) "[unasked] " "") (%agent-command-line n)))
                   names)
              "\n"))))))

(define (%agent-literals text)
  ; The data in TEXT when every one is a literal — a string, a number, a
  ; boolean, a keyword, or a quoted symbol — else #f: arguments that are
  ; values, with nothing in them to evaluate.
  (let ((port (open-input-string text)))
    (let loop ((acc '()))
      (let ((d (read port)))
        (cond ((eof-object? d) (reverse acc))
              ((or (string? d) (number? d) (boolean? d) (keyword? d)) (loop (cons d acc)))
              ((and (pair? d) (eq? (car d) 'quote) (pair? (cdr d)) (symbol? (cadr d))) (loop (cons (cadr d) acc)))
              (else #f))))))

(define-agent-tool "editor_command"
  "Run one of the editor's commands that only change what the editor shows — the ones editor_commands lists with no query: a colour theme, the REPL or terminal panel, the file tree, the text size. `args` is the command's arguments as Scheme literals, e.g. `:dracula`, `16`, `\"notes.md\"`. For any other command, or anything that needs evaluating, use editor_eval."
  '(("name" "string" "The command's name, e.g. load-theme." #t)
    ("args" "string" "Its arguments, as Scheme literals separated by spaces. Omit for none."))
  (lambda (args reply)
    (let* ((name (%agent-arg args "name" ""))
           (f (%env-ref (string->symbol name) #f))
           (given (guard (e (#t #f)) (%agent-literals (%agent-arg args "args" "")))))
      (cond ((not (member name *agent-ui-commands*))
              (reply (string-append name " is not one of the commands editor_command may call — editor_commands lists them; editor_eval runs anything, with the user's leave") #t))
            ((not (procedure? f)) (reply (string-append "this editor has no command " name) #t))
            ((not given) (reply "args must be Scheme literals: strings, numbers, #t/#f, :keywords or 'symbols" #t))
            (else
              (let ((r (apply f given)))
                (agent-append (string-append "⌘ " name (if (null? given) "" (string-append " " (%agent-arg args "args" "")))) :dim)
                (reply (string-append "Ran (" name (if (null? given) "" (string-append " " (%agent-arg args "args" ""))) ")"
                                      (if (or (not r) (eq? r #t)) "" (string-append " => " (*show-value* r)))))))))))

(define-agent-tool "editor_eval"
  "Evaluate Scheme in the EDITOR's own image — where its commands, keymaps and settings live — and return what it printed and its value. This can do anything the editor can: bind a key, define a command, call any command with computed arguments. The user is asked before each call. (To evaluate code in the user's PROGRAM, use eval.)"
  '(("code" "string" "Scheme forms, evaluated in order; the last one's value is returned." #t))
  (lambda (args reply)
    (let* ((r (in-process-eval (%agent-arg args "code" "")))
           (out (string-trim-right (%alist-ref r :output ""))))
      (if (eq? (%alist-ref r :status #f) 'ok)
          (reply (string-append out (if (equal? out "") "" "\n") "=> " (%text (%alist-ref r :printed ""))))
          (reply (string-append out (if (equal? out "") "" "\n") "error: " (%text (%alist-ref r :message ""))) #t))))
  #t)

(define-agent-tool "eval"
  "Evaluate code in the running image the buffer's forms are sent to — the program the user is working on (a Scheme REPL process, ray, Guile), where their definitions are live — and return what it printed and its value. Use it to try an expression or check a definition. The user is asked before each call."
  '(("code" "string" "The code to evaluate, in the buffer's language." #t)
    ("buffer" "string" "Whose image: the buffer's name or its file's path. Omit for the buffer on screen."))
  (lambda (args reply)
    (let* ((buf (%agent-buffer args))
           (backend (backend-of buf))
           (code (%agent-arg args "code" ""))
           (answer (lambda (r)
                     (let ((out (string-trim-right (%alist-ref r :output ""))))
                       (if (eq? (%alist-ref r :status #f) 'ok)
                           (reply (string-append out (if (equal? out "") "" "\n") "=> " (%text (%alist-ref r :printed ""))))
                           (reply (string-append out (if (equal? out "") "" "\n") "error: " (%text (%alist-ref r :message ""))) #t))))))
      (cond ((not backend) (reply "that buffer's language has nowhere to send code from this editor" #t))
            ((backend-op backend :eval-async) => (lambda (ask) (ask code answer)))
            (else (answer ((backend-op backend :eval) code))))))
  #t)

(define (agent-profile-extend base . settings)
  "(agent-profile-extend BASE KEY VALUE …)  ->  a profile

BASE with each KEY set to VALUE: a later setting replaces an earlier one,
#f removes it, and :arg adds an argument rather than replacing the last.
How a host makes its agent out of the code profile:

  (set! *agent-profile*
    (lambda () (agent-profile-extend (agent-code-profile)
                 :mcp-config my-servers :model \"sonnet\")))"
  (let loop ((p base) (s settings))
    (if (or (null? s) (null? (cdr s)))
        p
        (let ((k (car s)) (v (cadr s)))
          (loop (cond ((eq? k :arg) (append p (list (cons k v))))
                      (else (let ((rest (filter (lambda (kv) (not (eq? (car kv) k))) p)))
                              (if v (append rest (list (cons k v))) rest))))
                (cddr s))))))

(define (%agent-flat profile)
  ; The profile as the kernel takes it: ("cwd" "/proj" "model" "opus" …).
  (apply append
         (map (lambda (kv)
                (let ((v (cdr kv)))
                  (cond ((not v) '())
                        ((eq? v #t) (list (%text (car kv)) "1"))
                        (else (list (%text (car kv)) (%text v))))))
              profile)))

(define *agent-sessions* (make-hash-table equal?))   ; directory -> session id
(define *agent-session-ref* (lambda (dir) (hash-table-ref/default *agent-sessions* dir #f)))
(define *agent-session-set!* (lambda (dir id) (hash-table-set! *agent-sessions* dir id)))

; ── The agent edits files; the buffers follow (A4) ─────────────────────────
; It works on the files on disk, with its own tools and with whatever it
; runs (a formatter rewrites what it likes). A buffer with NO edits of its
; own is that file as it was, so when the file changes the buffer is
; reloaded and says so. A buffer WITH unsaved edits is never touched: the
; pane says, in red, that the two have parted, and how to choose.
;
; A changed file is found by its size and time (%os-stat), noted when a
; request goes out — reading every open file on every step would be the
; cost of the feature — and a file the agent's own edit tools NAMED is
; compared by its text whatever the clock says.
(define *agent-edit-tools* '("Edit" "Write" "MultiEdit" "NotebookEdit"))
(define *agent-touched* '())                       ; files its edit tools named, this turn
(define *agent-warned* '())                        ; and those already warned of
(define *agent-file-marks* (make-hash-table equal?))   ; path -> (size modified)

(define (%agent-file-mark path)
  (let ((stat (%env-ref '%os-stat #f)))
    (and stat
         (let ((st (stat path)))
           (and st (list (%alist-ref st :size #f) (%alist-ref st :modified #f)))))))

(define (%agent-file-buffers)
  ; (buffer path dirty?) for each buffer on a file that is text and exists.
  (let ((eid (editor-pane-id)))
    (if (not eid)
        '()
        (filter-map
          (lambda (row)
            (let ((path (buffer-path (car row))))
              (and path (file-exists? path) (*writable-path?* path)
                   (list (car row) path (positive? (cadr row))))))
          (%editor-buffer-list eid)))))

(define (agent-note-files)
  "Note every open file as it is on disk now: what a later
  sync-buffers-with-disk measures change from."
  (for-each (lambda (b) (hash-table-set! *agent-file-marks* (cadr b) (%agent-file-mark (cadr b))))
            (%agent-file-buffers)))

(define (%agent-reload buf path)
  ; BUF's text from PATH, when they differ; the cursor stays on its line.
  ; #t when it was reloaded. (As a LOAD: the buffer's undo does not reach
  ; back across it. Edits made in the buffer, undoable, are A6's.)
  (let* ((eid (editor-pane-id))
         (cur (%editor-active-buffer eid))
         (there (equal? cur buf)))
    (unless there (%editor-buffer-switch eid buf))
    (let* ((disk (slurp path))
           (changed (and disk
                         (not (positive? (%editor-markdown? eid)))
                         (not (equal? disk (%editor-text eid))))))
      (when changed
        (let ((line (car (%editor-cursor eid))))
          (%editor-set-text eid disk)
          (%editor-mark-saved eid)
          (%editor-goto-line eid line)))
      (unless there (%editor-buffer-switch eid cur))
      (if changed #t #f))))

(define (sync-buffers-with-disk)
  "(sync-buffers-with-disk)  ->  the buffers reloaded

Bring the open buffers up to their files — M-x sync-buffers-with-disk, and
what the agent pane does after each of the agent's edits and at the end of
every turn. A buffer with no unsaved edits whose file has changed is
reloaded, the cursor kept on its line. A buffer WITH unsaved edits is left
exactly as it is; when the agent changed that file, the pane says so —
`:e!` takes the file's version, C-x C-s keeps yours."
  (let ((reloaded '()))
    (for-each
      (lambda (b)
        (let* ((buf (car b)) (path (cadr b)) (dirty (caddr b))
               (touched (any (lambda (t) (same-file? t path)) *agent-touched*))
               (mark (%agent-file-mark path))
               (moved (or touched (not mark)
                          (not (equal? mark (hash-table-ref/default *agent-file-marks* path mark))))))
          (hash-table-set! *agent-file-marks* path mark)
          (cond ((not moved) #f)
                ((not dirty)
                  (when (%agent-reload buf path)
                    (set! reloaded (cons buf reloaded))
                    (agent-append (string-append "↻ reloaded " (%text (buffer-name buf))) :dim)))
                ((and touched (not (member path *agent-warned*)))
                  (set! *agent-warned* (cons path *agent-warned*))
                  (agent-append
                    (string-append "✗ " (%text (buffer-name buf))
                                   " has unsaved edits, and the agent changed its file — the buffer is NOT reloaded."
                                   " :e! takes the file's version; C-x C-s keeps yours.")
                    :error)))))
      (%agent-file-buffers))
    (unless (null? reloaded) (*buffers-changed*))
    (reverse reloaded)))

; ── The transcript ─────────────────────────────────────────────────────────

(define agent-prompt "agent> ")
(define *agent-history* '())

(define (agent-append text (style #f))
  "Append TEXT to the agent's transcript, a row per line. Styles: :normal
  :dim :accent :error :code (source, token-coloured)."
  (let ((id (agent-id)))
    (when id (%pane-append id (%text (or style :normal)) (%describe-part text)))))

; The row the reply will REPLACE: while a request is out the transcript ends
; with a dim "thinking…" (Kevin, 2026-09-21: "some indication that the agent
; is working"). The first thing back lands on that row.
(define *agent-pending* #f)
(define *agent-steps* 0)
; The directory the turn in flight runs in: its session id is filed there.
(define *agent-turn-dir* #f)

; LEAVE. A call the agent may not make unasked arrives as a question and the
; turn waits; the next line typed is the answer. *agent-asks* is the
; questions still open, oldest first, each (id tool arg); *agent-allowed*
; the tools "a" allowed for the rest of this conversation.
(define *agent-asks* '())
(define *agent-allowed* '())

; A PLACE, ONCE GRANTED, IS NOT ASKED FOR AGAIN (Kevin, 2026-11-17). Leave
; given for a file is leave for its directory and everything under it: to
; write there (which is also to read there) when a write was allowed, to
; read when a read was. Grants belong to the PROJECT the agent runs in, not
; to one conversation — C-l does not forget them, /revoke does — and a
; host with somewhere to keep them keeps them across restarts.
;   *agent-grants-ref* / *agent-grants-set!*   the grants of a project, a
;                        list of (directory . "write" | "read")
; A command allowed with y is not asked for again either, word for word,
; for the rest of the conversation.
(define *agent-write-tools* '("Write" "Edit" "MultiEdit" "NotebookEdit"))
(define *agent-read-tools* '("Read" "Glob" "Grep"))
(define *agent-grant-table* (make-hash-table equal?))
(define *agent-grants-ref* (lambda (project) (hash-table-ref/default *agent-grant-table* project '())))
(define *agent-grants-set!* (lambda (project grants) (hash-table-set! *agent-grant-table* project grants)))
(define *agent-commands-allowed* '())

(define (%agent-project)
  (or *agent-turn-dir* (%alist-ref (*agent-profile*) :cwd "")))

(define (%agent-dirname path)
  ; The directory PATH is in; PATH itself when it is one.
  (if (guard (e (#t #f)) (files-dir? path))
      path
      (let loop ((i (- (string-length path) 1)))
        (cond ((< i 1) "/")
              ((char=? (string-ref path i) #\/) (substring path 0 i))
              (else (loop (- i 1)))))))

(define (%agent-file-ask? tool arg)
  ; Is this a question about a FILE, by a path we can place?
  (and (or (member tool *agent-write-tools*) (member tool *agent-read-tools*))
       (string-prefix? "/" arg)))

(define (agent-granted? tool path)
  "Has leave already been given for TOOL at PATH — a grant on its directory
  or one above it, to write, or to read when TOOL only reads?"
  (any (lambda (g)
         (and (or (equal? path (car g)) (string-prefix? (string-append (car g) "/") path))
              (or (equal? (cdr g) "write") (member tool *agent-read-tools*))))
       (*agent-grants-ref* (%agent-project))))

(define (agent-grant! tool path)
  ; Leave for TOOL at PATH becomes leave for its directory.
  (let* ((dir (%agent-dirname path))
         (kind (if (member tool *agent-write-tools*) "write" "read"))
         (project (%agent-project))
         (others (filter (lambda (g) (not (equal? (car g) dir))) (*agent-grants-ref* project)))
         (had (assoc dir (*agent-grants-ref* project))))
    ; A read granted where a write already was does not take the write away.
    (*agent-grants-set!* project
      (append others (list (cons dir (if (and had (equal? (cdr had) "write")) "write" kind)))))
    dir))

(define (%agent-ask-shown tool arg)
  (string-append tool (if (equal? arg "") "" (string-append " " (%agent-cut (%agent-first-line arg) 300)))))

; AUTOMATIC (Kevin, 2026-11-17: "still noisy as far as asking for 'y'
; approvals"). /auto and every question is answered yes as it arrives — a
; program run, a file anywhere — each still drawn as a ✓ row, so what it did
; is on the page. It is a grant like the others: the project's, kept across
; restarts, forgotten by /auto off and by /revoke. The prompt says so for
; as long as it holds, because an agent that runs anything unasked should
; never be a surprise.
(define (agent-auto?)
  (if (assoc "*" (*agent-grants-ref* (%agent-project))) #t #f))

(define (agent-set-auto! on)
  (let* ((project (%agent-project))
         (others (filter (lambda (g) (not (equal? (car g) "*"))) (*agent-grants-ref* project))))
    (*agent-grants-set!* project (if on (cons (cons "*" "auto") others) others))
    (agent-prompt-refresh)
    on))

(define (agent-prompt-now)
  (if (agent-auto?) "agent (auto)> " agent-prompt))

(define (agent-prompt-refresh)
  ; The prompt on screen, when the agent's input is open, as it should read.
  (when (and (agent-id) (%editing-pane? (agent-id)))
    (%edit! :prompt (agent-prompt-now))
    (edit-sync)))

(define (agent-asked id tool arg why)
  (cond ((or (agent-auto?)
             (member tool *agent-allowed*)
             (and (%agent-file-ask? tool arg) (agent-granted? tool arg))
             (member (cons tool arg) *agent-commands-allowed*))
          (%agent-answer id #t "")
          (agent-show (string-append "✓ " (%agent-ask-shown tool arg)) :dim))
        (else
          (set! *agent-asks* (append *agent-asks* (list (list id tool arg))))
          (agent-show (string-append "? " (%agent-ask-shown tool arg)) :accent)
          (agent-append
            (if (%agent-file-ask? tool arg)
                (string-append "  y allow, here and from now on in " (%agent-dirname arg)
                               " · once · n deny · /auto stops asking · or say what to do instead")
                (string-append "  y allow · n deny · a allow every " tool
                               " call in this conversation · /auto stops asking · or say what to do instead"))
            :dim)
          (echo "agent: asks leave — " (%agent-ask-shown tool arg)))))

(define (agent-answer text)
  "Answer the agent's oldest open question with TEXT, as typed at agent>:
  y or yes allows the call, and REMEMBERS it — for a file, its directory
  and everything under it, from now on in this project; for a command,
  that command for the rest of the conversation. once allows this call and
  remembers nothing. a or always allows every later call of that tool in
  this conversation. n or no refuses; anything else refuses and is what the
  agent is told — \"use cargo check instead\"."
  (let* ((ask (car *agent-asks*))
         (id (car ask)) (tool (cadr ask)) (arg (caddr ask))
         (word (string-downcase (string-trim-both text)))
         (always (member word '("a" "always")))
         (once (member word '("once" "1")))
         (yes (member word '("y" "yes")))
         (allow (or always once yes))
         (no (member word '("n" "no")))
         (place (%agent-file-ask? tool arg)))
    (set! *agent-asks* (cdr *agent-asks*))
    (when always (set! *agent-allowed* (cons tool *agent-allowed*)))
    (cond ((not (%agent-answer id (if allow #t #f) (if (or allow no) "" text)))
            (agent-append "that question is no longer open" :dim))
          (always (agent-append (string-append "→ allowed, and every " tool " call from here on") :dim))
          ((and yes place)
            (agent-append (string-append "→ allowed, and not asked again in " (agent-grant! tool arg)
                                         (if (member tool *agent-write-tools*) "" " (to read)")
                                         " — /grants lists, /revoke forgets")
                          :dim))
          (yes
            (set! *agent-commands-allowed* (cons (cons tool arg) *agent-commands-allowed*))
            (agent-append "→ allowed, and not asked again for that in this conversation" :dim))
          (allow (agent-append "→ allowed, this once" :dim))
          (else (agent-append "→ refused" :dim)))
    (unless (null? *agent-asks*)
      (agent-append (string-append "? " (%agent-ask-shown (cadr (car *agent-asks*)) (caddr (car *agent-asks*)))) :accent))))

(define (agent-take-pending text style)
  ; Put TEXT where the pending row is, its first line on that row and the
  ; rest after it; #f when no row was pending.
  (let ((pending *agent-pending*) (id (agent-id)))
    (set! *agent-pending* #f)
    (and pending id
         (let ((lines (string-split text "\n")))
           (%pane-set-row id pending (%text style) (car lines))
           (let loop ((ls (cdr lines)) (at (+ pending 1)))
             (unless (null? ls)
               (%pane-insert-row id at (%text style) (car ls))
               (loop (cdr ls) (+ at 1))))
           #t))))

(define (agent-show text style)
  (unless (agent-take-pending text style) (agent-append text style)))

; A REPLY AS IT IS WRITTEN. The kernel sends a message's text in pieces
; ("delta") and then whole ("text"); the pieces are drawn where they will
; end up, so the first words are on screen when the model has said them
; rather than when it has finished. #f, or #(first-row rows text).
(define *agent-stream* #f)

; CODE IN A REPLY (Kevin, 2026-11-17: "formatted differently than other
; text … and a little glyph … that when you click on it, it gets copied").
; A fenced block — ```lang … ``` — is drawn as a block: a header row naming
; its language and offering what can be done with it, then its lines on the
; editor's own ground. The header's words are clicked:
;   copy     the block to the clipboard
;   insert   into the buffer on screen, at the cursor
;   append   to the end of that buffer, after a blank line
; A row's style is where the block is remembered: a click reads the rows
; under the header back, so nothing has to be kept beside the transcript.
(define *agent-mark* (string (integer->char #xE010)))   ; the widget's ACTION_MARK
(define *agent-block-actions* '("copy" "insert" "append"))
(define *agent-lisps* '("scheme" "scm" "lisp" "elisp" "emacs-lisp" "clojure" "racket" "janet" "guile"))

(define (%agent-block-head language)
  (string-append
    (if (equal? language "") "code" language)
    (apply string-append
           (map (lambda (action) (string-append "  " *agent-mark* action *agent-mark*)) *agent-block-actions*))))

(define (%agent-fence line)
  ; The language a fence line opens with ("" for none), or #f: not a fence.
  (let ((t (string-trim-both line)))
    (and (string-prefix? "```" t)
         (string-trim-both (substring t 3 (string-length t))))))

(define (agent-reply-rows text)
  "TEXT, a reply as the model wrote it, as the rows it is drawn in: a list
  of (style . text), one for each of its lines."
  (let loop ((ls (string-split text "\n")) (in #f) (acc '()))
    (if (null? ls)
        (reverse acc)
        (let ((fence (%agent-fence (car ls))))
          (cond ((and fence (not in))
                  (loop (cdr ls)
                        (if (member (string-downcase fence) *agent-lisps*) "block-code" "block")
                        (cons (cons "block-head" (%agent-block-head fence)) acc)))
                ; The closing fence: a blank row, which sets the block off.
                (fence (loop (cdr ls) #f (cons (cons "normal" "") acc)))
                (in (loop (cdr ls) in (cons (cons in (car ls)) acc)))
                (else (loop (cdr ls) #f (cons (cons "normal" (car ls)) acc))))))))

(define (agent-stream-draw text)
  ; Draw TEXT over the rows the reply has so far: only its last row can have
  ; changed, and any line after that is new.
  (let* ((id (agent-id))
         (st (or *agent-stream*
                 (let ((at (or *agent-pending*
                               (begin (agent-append "" :normal) (- (%pane-row-count id) 1)))))
                   (set! *agent-pending* #f)
                   (vector at 1 ""))))
         (first (vector-ref st 0))
         (had (vector-ref st 1))
         (rows (agent-reply-rows text)))
    (let loop ((i 0) (rs rows))
      (unless (null? rs)
        (cond ((< i (- had 1)) #f)
              ((< i had) (%pane-set-row id (+ first i) (car (car rs)) (cdr (car rs))))
              (else (%pane-insert-row id (+ first i) (car (car rs)) (cdr (car rs)))))
        (loop (+ i 1) (cdr rs))))
    (set! *agent-stream* (vector first (max had (length rows)) text))))

(define (agent-show-reply text)
  ; A whole reply at once — one that did not arrive in pieces.
  (when (agent-id)
    (set! *agent-stream* #f)
    (agent-stream-draw text)
    (set! *agent-stream* #f)))

(define (agent-block-text row)
  "The code of the block whose header is transcript row ROW."
  (let ((id (agent-id)))
    (let loop ((i (+ row 1)) (acc '()))
      (if (member (%pane-row-style id i) '("block" "block-code"))
          (loop (+ i 1) (cons (%pane-row-text id i) acc))
          (%agent-join (reverse acc) "\n")))))

(define (%agent-head-action head col)
  ; The action word of header text HEAD that column COL is on, or #f.
  (let loop ((i 0) (inside #f) (start 0))
    (cond ((>= i (string-length head)) #f)
          ((string=? (substring head i (+ i 1)) *agent-mark*)
            (if (and inside (>= col start) (< col i))
                (substring head start i)
                (loop (+ i 1) (not inside) (+ i 1))))
          (else (loop (+ i 1) inside start)))))

(define (agent-block-act action code)
  "Do ACTION — \"copy\", \"insert\" or \"append\" — with CODE, a block of a
  reply: to the clipboard, into the buffer on screen at the cursor, or onto
  its end after a blank line. An insertion is one undo step."
  (let* ((eid (editor-pane-id))
         (n (length (string-split code "\n")))
         (lines (lambda () (string-append (number->string n) (if (= n 1) " line" " lines"))))
         (name (lambda () (%text (buffer-name (%editor-active-buffer eid))))))
    (cond
      ((equal? action "copy")
        (%set-clipboard code)
        (echo "copied " (lines) " — paste with Cmd-V / C-y"))
      ((not eid) (echo "there is no editor buffer to put it in"))
      ((equal? action "insert")
        (let ((at (%editor-cursor eid)))
          (%editor-replace eid (car at) (cadr at) (car at) (cadr at) (string-append code "\n"))
          (echo "inserted " (lines) " into " (name) " at line " (+ 1 (car at)) " — undo takes it back")))
      ((equal? action "append")
        (let* ((text (%editor-text eid))
               (all (string-split text "\n"))
               (last (- (length all) 1))
               (end (bytevector-length (string->utf8 (list-ref all last))))
               ; After a blank line, whatever the buffer ends with.
               (lead (cond ((equal? text "") "")
                           ((string-suffix? "\n\n" text) "")
                           ((string-suffix? "\n" text) "\n")
                           (else "\n\n"))))
          (%editor-replace eid last end last end (string-append lead code "\n"))
          (echo "appended " (lines) " to " (name) " — undo takes it back"))))
    action))

; A click on a block header's word does it. Above the click that gives the
; pane the keyboard (50), which a click anywhere else in the pane still is.
(add-handler
  *click-handlers*
  60
  :agent-block
  (lambda (x y . _)
    (let* ((id (agent-id))
           (hit (and id (not (menu-open?)) (%pane-at x y)))
           (at (and hit (equal? (car hit) id) (%pane-line-at id x y))))
      (and at
           (equal? (%pane-row-style id (car at)) "block-head")
           (let ((action (%agent-head-action (%pane-row-text id (car at)) (cadr at))))
             (and action (begin (agent-block-act action (agent-block-text (car at))) #t)))))))

(define (agent-drop-pending)
  (let ((pending *agent-pending*) (id (agent-id)))
    (set! *agent-pending* #f)
    (when (and pending id) (%pane-remove-row id pending))))

(define (%agent-cut text n)
  (let ((t (string-trim-both text)))
    (if (> (string-length t) n) (string-append (substring t 0 n) " …") t)))

(define (%agent-first-line text)
  (car (string-split (string-trim-both text) "\n")))

; The tool whose result comes next, and the tools whose results are not
; worth a row: a file read back is the file, and an edit's "updated" is said
; better by the buffer reloading.
(define *agent-last-tool* #f)
(define *agent-quiet-results*
  '("Read" "Edit" "Write" "MultiEdit" "NotebookEdit" "ToolSearch"
    "buffers" "buffer_read" "buffer_edit" "buffer_write" "buffer_save" "buffer_close"
    "open_file" "editor_commands" "editor_command"))

(define (%agent-in-project path)
  ; PATH as the project sees it: without the turn's directory in front.
  (let ((dir (and *agent-turn-dir* (string-append *agent-turn-dir* "/"))))
    (cond ((and dir (string-prefix? dir path)) (substring path (string-length dir) (string-length path)))
          ; (macOS: /var is /private/var, and the two spell one directory.)
          ((and dir (string-prefix? (string-append "/private" dir) path))
            (substring path (+ 8 (string-length dir)) (string-length path)))
          (else path))))

(define (%agent-head text lines width)
  ; The first LINES lines of TEXT, each cut to WIDTH, and a mark when there
  ; was more.
  (let* ((all (string-split (string-trim-both text) "\n"))
         (kept (let loop ((ls all) (n lines) (acc '()))
                 (if (or (null? ls) (zero? n)) (reverse acc)
                     (loop (cdr ls) (- n 1)
                           (cons (if (> (string-length (car ls)) width)
                                     (string-append (substring (car ls) 0 width) " …")
                                     (car ls))
                                 acc))))))
    (string-append
      (apply string-append (cdr (apply append (map (lambda (l) (list "\n" l)) kept))))
      (if (> (length all) lines) "\n…" ""))))

(define (agent-default-row kind a b)
  ; How a step reads when the host has nothing to say about it: (style
  ; text), or (#f) for no row at all.
  (cond ((equal? kind "text") (list :normal a))
        ; (Its own row, "⌘ load-theme :dracula", says more.)
        ((and (equal? kind "tool") (equal? a "editor_command")) (list #f))
        ((equal? kind "tool")
          (list :dim (string-append "→ " a (if (equal? b "") ""
                                               (string-append " " (%agent-cut (%agent-first-line (%agent-in-project b)) 160))))))
        ((and (equal? kind "result") (member *agent-last-tool* *agent-quiet-results*)) (list #f))
        ((equal? kind "result") (list :dim (%agent-head a 4 200)))
        ; A refusal is not a tool result like the others: the agent was
        ; asked for something and could not do it.
        ((equal? kind "denied") (list :error (string-append "✗ " a " was not allowed — " b)))
        ; The footer is a measurement, not the model talking.
        ((equal? kind "metrics") (list :dim (string-append "· " a)))
        ((equal? kind "note") (list :dim a))
        (else (list :normal a))))

(define (agent-turn-over status)
  ; Whatever the turn left on disk — its last edit, a command's output —
  ; the buffers have before the user looks.
  (guard (e (#t #f)) (sync-buffers-with-disk))
  (set! *agent-touched* '())
  (set! *agent-warned* '())
  (set! *agent-last-tool* #f)
  (set! *agent-steps* 0)
  (set! *agent-asks* '())
  (set! *agent-turn-dir* #f)
  (*agent-turn-ended* status))

(define (agent-keep-session id)
  (when (and id *agent-turn-dir*) (*agent-session-set!* *agent-turn-dir* id)))

(define (%agent-pairs flat)
  ; (k v k v …) as an alist.
  (if (or (null? flat) (null? (cdr flat)))
      '()
      (cons (cons (car flat) (cadr flat)) (%agent-pairs (cddr flat)))))

(define (agent-event row)
  "One event of a turn, from the kernel's queue (%agent-poll), drawn into
  the transcript as it arrives — so a long job reads as work in progress
  rather than silence."
  (let ((kind (car row))
        (a (if (pair? (cdr row)) (cadr row) ""))
        (b (if (and (pair? (cdr row)) (pair? (cddr row))) (caddr row) "")))
    (unless (member kind '("delta" "text")) (set! *agent-stream* #f))
    (cond
      ; ("ask" id tool arg why)
      ; ("call" id name key value …)
      ((equal? kind "call") (agent-call a b (%agent-pairs (cdddr row))))
      ((equal? kind "ask")
        (set! *agent-last-tool* b)
        (agent-asked a b (%list-ref/default row 3 "") (%list-ref/default row 4 "")))
      ((equal? kind "delta")
        (when (agent-id)
          (agent-stream-draw (string-append (if *agent-stream* (vector-ref *agent-stream* 2) "") a))))
      ; The whole message its pieces added up to: drawn over them.
      ((and (equal? kind "text") *agent-stream*)
        (agent-stream-draw a)
        (set! *agent-stream* #f)
        (set! *agent-steps* (+ *agent-steps* 1)))
      ((equal? kind "done")
        (agent-keep-session b)
        (if (equal? a "") (agent-drop-pending) (agent-show-reply a))
        (echo "agent: finished")
        (agent-turn-over "done"))
      ((equal? kind "error")
        ; A turn that hit its cap still names its conversation: keep it.
        (agent-keep-session b)
        (agent-show (string-append "[agent] " a) :error)
        (echo "agent: " (%agent-first-line a))
        (agent-turn-over "error"))
      ((equal? kind "cancelled")
        ; What its tools already did stays done: the interrupt stops the
        ; work, it does not undo it. Nor the conversation: its id is kept.
        (agent-keep-session b)
        (agent-show "— interrupted —" :dim)
        (echo "agent: interrupted")
        (agent-turn-over "cancelled"))
      ((equal? kind "auth") (agent-append a :normal))
      ((equal? kind "auth-done") (unless (equal? a "") (agent-append a :dim)))
      ((equal? kind "auth-error") (agent-append (string-append "[agent] " a) :error))
      (else
        ; An edit tool names its file BEFORE it writes; its result is when
        ; the file has changed.
        (when (and (equal? kind "tool") (member a *agent-edit-tools*) (not (equal? b "")))
          (set! *agent-touched* (cons b *agent-touched*)))
        (when (and (equal? kind "result") (pair? *agent-touched*))
          (guard (e (#t #f)) (sync-buffers-with-disk)))
        (set! *agent-steps* (+ *agent-steps* 1))
        (let ((shown (or (*agent-row* kind a b) (agent-default-row kind a b))))
          (cond ((not (car shown)) #f)
                ; The model's own words: prose, and code blocks as blocks.
                ((and (equal? kind "text") (eq? (car shown) :normal)) (agent-show-reply (cadr shown)))
                (else (agent-show (cadr shown) (car shown)))))
        (when (equal? kind "tool") (set! *agent-last-tool* a))
        (echo "agent: step " *agent-steps* " — "
              (cond ((equal? kind "tool") (string-append a " " (%agent-cut (%agent-first-line b) 60)))
                    ((equal? kind "metrics") a)
                    (else "thinking…")))))
    #f))

(define (agent-poll)
  "Draw every event the agent has queued. The host's wake calls it, through
  *wake-hooks*, on the thread that owns the panes."
  (for-each agent-event (%agent-poll)))

(set! *wake-hooks* (cons agent-poll *wake-hooks*))

; ── What you type ──────────────────────────────────────────────────────────

(define (%agent-error-text e)
  ; What a prim raised, without the prim's own name in front: the pane is
  ; read by someone who typed English, not `%agent-send`.
  (let* ((t (if (string? e) e (*show-value* e)))
         (cut (and (string-prefix? "%agent-" t)
                   (let loop ((i 0))
                     (cond ((>= (+ i 1) (string-length t)) #f)
                           ((and (char=? (string-ref t i) #\:) (char=? (string-ref t (+ i 1)) #\space)) (+ i 2))
                           (else (loop (+ i 1))))))))
    (if cut (substring t cut (string-length t)) t)))

(define (agent-ask text)
  "Send TEXT to the agent as a request, as if typed at agent> — the
  conversation continues where it was. The answer streams into the pane."
  (let* ((profile (agent-profile-now))
         (dir (%alist-ref profile :cwd ""))
         (session (*agent-session-ref* dir)))
    (guard (e (#t #f)) (agent-note-files))
    (agent-append "thinking…" :dim)
    (set! *agent-pending* (- (%pane-row-count (agent-id)) 1))
    (set! *agent-turn-dir* dir)
    (guard (e (#t (agent-show (string-append "[agent] " (%agent-error-text e)) :error)
                  (set! *agent-turn-dir* #f)
                  #f))
      (%agent-send (*agent-prepare* text)
                   (%agent-flat (agent-profile-extend profile :session session)))
      (echo "agent: thinking…")
      #t)))

; Slash commands: the account, driven from the pane. /login runs `claude
; auth login` — the browser opens itself; when the signed-in page shows a
; code, paste it HERE (the next line goes to the login, not the agent).
(define (agent-command cmd)
  (define (auth action)
    (guard (e (#t (agent-append (string-append "[agent] " (%agent-error-text e)) :error)))
      (%agent-auth action (%agent-flat (*agent-profile*)))))
  (define words (filter (lambda (w) (not (equal? w ""))) (string-split cmd " ")))
  (cond
    ; /model NAME — a faster or a stronger model from the next request on,
    ; in the same conversation (it costs one restart of the process).
    ((equal? (car words) "/model")
      (if (null? (cdr words))
          (agent-append (string-append "model: " (or *agent-model* (%alist-ref (*agent-profile*) :model #f) "opus")
                                       " — /model opus | sonnet | haiku | default")
                        :dim)
          (begin
            (set! *agent-model* (if (equal? (cadr words) "default") #f (cadr words)))
            (agent-append (string-append "model: " (or *agent-model* "the default") " from the next request") :dim))))
    ((equal? cmd "/login")
      (agent-append "opening the browser to sign in — if the page shows a code, paste it here" :dim)
      (auth "login"))
    ((equal? cmd "/logout") (auth "logout"))
    ((equal? cmd "/status") (auth "status"))
    ((equal? cmd "/new") (new-conversation))
    ; /usage — what the plan has left and what this conversation has used.
    ((equal? cmd "/usage") (for-each (lambda (line) (agent-append line :dim)) (%agent-usage)))
    ; /auto [on|off] — answer every question yes, or go back to asking.
    ((equal? (car words) "/auto")
      (let ((on (cond ((null? (cdr words)) (not (agent-auto?)))
                      (else (not (member (cadr words) '("off" "no" "0")))))))
        (agent-set-auto! on)
        ; Questions already waiting are answered by the same rule.
        (when on
          (for-each (lambda (ask)
                      (%agent-answer (car ask) #t "")
                      (agent-append (string-append "✓ " (%agent-ask-shown (cadr ask) (caddr ask))) :dim))
                    *agent-asks*)
          (set! *agent-asks* '()))
        (agent-append
          (if on
              "automatic: the agent runs programs and touches files anywhere WITHOUT asking, in this project, until /auto off. Each is still shown, as a ✓ row."
              "asking again: the agent asks before it runs a program or leaves the project")
          (if on :accent :dim))))
    ((equal? cmd "/grants")
      (let ((grants (*agent-grants-ref* (%agent-project))))
        (if (null? grants)
            (agent-append "no place outside the project has been granted — the agent asks, and y remembers" :dim)
            (for-each (lambda (g)
                        (agent-append (if (equal? (car g) "*")
                                          "  automatic: every question is answered yes (/auto off)"
                                          (string-append "  " (cdr g) "  " (car g)))
                                      :dim))
                      grants))))
    ((equal? cmd "/revoke")
      (*agent-grants-set!* (%agent-project) '())
      (agent-prompt-refresh)
      (set! *agent-commands-allowed* '())
      (set! *agent-allowed* '())
      (agent-append "forgotten: the agent asks again before it leaves the project or runs a program" :dim))
    ((equal? cmd "/help")
      (agent-append "commands: /login /logout /status /usage /new /model /auto /grants /revoke — anything else is a request to the agent. Escape interrupts it." :dim))
    (else
      (agent-append (string-append "unknown command " cmd " — /login /logout /status /usage /new /model /auto /grants /revoke /help") :error))))

(define (agent-submit text)
  (%edit! :hist #f)
  (edit-set "" 0)
  (let ((trimmed (string-trim-both text)))
    (unless (equal? trimmed "")
      (set! *agent-history* (append *agent-history* (list text)))
      (agent-append (string-append "you> " text) :accent)
      (cond ((string-prefix? "/" trimmed) (agent-command trimmed))
            ; A /login is waiting on its code: this line is the paste.
            ((%agent-auth-pending?)
              (if (%agent-auth-input trimmed)
                (agent-append "→ sent to the login prompt" :dim)
                (agent-append "the login is no longer waiting — /status to check" :error)))
            ; The agent is waiting on a question: this line is the answer.
            ((pair? *agent-asks*) (agent-answer trimmed))
            ((%agent-busy?)
              (agent-append "still answering — Escape interrupts it" :dim))
            (else (agent-ask text)))))
  #f)

(define (agent-typing?)
  "Does the agent's input have the keyboard? What scopes Escape's interrupt
  to this pane, so Escape everywhere else means what it meant."
  (let ((id (agent-id))) (and id (equal? (%typing-pane) id))))

(define (agent-interrupt)
  "(agent-interrupt)  ->  #t when a turn was stopped

Stop the agent's turn — Escape in its pane, and Interrupt on its menu. It
is asked to stop and answers at once; what its tools already did stays
done, and the conversation is still there to go on with."
  (if (%agent-cancel) #t (begin (echo "the agent is not working on anything") #f)))

(define (focus-agent)
  "Show the agent pane and give its input the keyboard."
  (let ((id (agent-id)))
    (cond ((not id) (echo "this editor has no agent pane"))
          (else
            (*agent-shown* id)
            (%focus-pane id)
            (if (%editing-pane? id)
              (begin (set! *focus-held* #t) (%hold-typing id) (edit-sync))
              (edit-start id (agent-prompt-now) agent-submit :history (lambda () *agent-history*)))))))

(define (agent-paste)
  "Paste the clipboard into the agent's input — Cmd-V / C-y there. The
  input is one line of English, so newlines become spaces."
  (let ((t (%clipboard)))
    (and t
         (begin
           (unless (%editing-pane? (agent-id)) (focus-agent))
           (edit-paste)
           t))))

(define (new-conversation)
  "Clear the agent's transcript AND forget the conversation behind it —
  C-l in the agent pane, /new, and the first item on its menu. The next
  request starts fresh; the old conversation is not resumed again."
  (%agent-stop)
  ; (A turn that was being answered has just been closed as cancelled: its
  ; event is the old conversation's, not this one's first row.)
  (%agent-poll)
  (set! *agent-turn-dir* #f)
  (set! *agent-steps* 0)
  (when (agent-id) (%pane-clear (agent-id)))
  (set! *agent-pending* #f)
  (set! *agent-stream* #f)
  (set! *agent-asks* '())
  (set! *agent-allowed* '())
  (set! *agent-commands-allowed* '())
  (*agent-session-set!* (%alist-ref (*agent-profile*) :cwd "") #f)
  (agent-append "— new conversation —" :dim))

(define (bind-agent-keys)
  "(bind-agent-keys)  ->  :agent

The agent pane's own keys: the input editor's (return, the motions,
history), C-l for a new conversation, and page up and down through the
transcript. Call it once the host's agent pane exists."
  (bind-editor-keys :agent)
  (%bind-labelled :agent
    (list
      (list "C-l" new-conversation "new-conversation")
      (list "pageup" (lambda () (%pane-scroll (agent-id) 8)) "scroll-back")
      (list "pagedown" (lambda () (%pane-scroll (agent-id) -8)) "scroll-forward")))
  :agent)

(define-menu
  :agent
  (list (list "New conversation" new-conversation "C-l")
        (list "Interrupt" agent-interrupt "Esc")
        (list "Paste" agent-paste "s-v")))
