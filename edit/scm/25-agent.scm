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
  "You are running inside `edit`, a text editor, in a side panel: the user reads your replies in a narrow transcript beside their source, so keep prose short and put code in fenced blocks. Files you change on disk are the files open in the editor. Each request opens with an <editor> block the editor wrote, not the user: the file they are looking at, where their cursor is, the line it is on, what they have selected, and the other files open. \"This\", \"here\", \"this function\" mean that place. A file marked as having unsaved edits is NEWER in the editor than on disk: what you read from disk may be stale, and the editor will not take your change to it, so say so and ask them to save (C-x C-s) before you edit it. A file with no unsaved edits is reloaded in the editor as soon as you change it.")

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
  "The profile the next request goes out under: the host's, and the model
  /model chose."
  (if *agent-model*
      (agent-profile-extend (*agent-profile*) :model *agent-model*)
      (*agent-profile*)))

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

(define (%agent-ask-shown tool arg)
  (string-append tool (if (equal? arg "") "" (string-append " " (%agent-cut (%agent-first-line arg) 300)))))

(define (agent-asked id tool arg why)
  (cond ((member tool *agent-allowed*)
          (%agent-answer id #t "")
          (agent-show (string-append "✓ " (%agent-ask-shown tool arg)) :dim))
        (else
          (set! *agent-asks* (append *agent-asks* (list (list id tool arg))))
          (agent-show (string-append "? " (%agent-ask-shown tool arg)) :accent)
          (agent-append (string-append "  y allow · n deny · a allow every " tool
                                       " call in this conversation · or say what to do instead")
                        :dim)
          (echo "agent: asks leave — " (%agent-ask-shown tool arg)))))

(define (agent-answer text)
  "Answer the agent's oldest open question with TEXT, as typed at agent>:
  y or yes allows the call; a or always allows it and every later call of
  that tool in this conversation; n or no refuses it; anything else refuses
  it and is what the agent is told — \"use cargo check instead\"."
  (let* ((ask (car *agent-asks*))
         (id (car ask)) (tool (cadr ask))
         (word (string-downcase (string-trim-both text)))
         (always (member word '("a" "always")))
         (allow (or always (member word '("y" "yes"))))
         (no (member word '("n" "no"))))
    (set! *agent-asks* (cdr *agent-asks*))
    (when always (set! *agent-allowed* (cons tool *agent-allowed*)))
    (cond ((not (%agent-answer id (if allow #t #f) (if (or allow no) "" text)))
            (agent-append "that question is no longer open" :dim))
          (always (agent-append (string-append "→ allowed, and every " tool " call from here on") :dim))
          (allow (agent-append "→ allowed" :dim))
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
         (lines (string-split text "\n")))
    (let loop ((i 0) (ls lines))
      (unless (null? ls)
        (cond ((< i (- had 1)) #f)
              ((< i had) (%pane-set-row id (+ first i) "normal" (car ls)))
              (else (%pane-insert-row id (+ first i) "normal" (car ls))))
        (loop (+ i 1) (cdr ls))))
    (set! *agent-stream* (vector first (max had (length lines)) text))))

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
(define *agent-quiet-results* '("Read" "Edit" "Write" "MultiEdit" "NotebookEdit"))

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
        (if (equal? a "") (agent-drop-pending) (agent-show a :normal))
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
          (when (car shown) (agent-show (cadr shown) (car shown))))
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
    ((equal? cmd "/help")
      (agent-append "commands: /login /logout /status /new /model — anything else is a request to the agent. Escape interrupts it." :dim))
    (else
      (agent-append (string-append "unknown command " cmd " — /login /logout /status /new /model /help") :error))))

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
              (edit-start id agent-prompt agent-submit :history (lambda () *agent-history*)))))))

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
