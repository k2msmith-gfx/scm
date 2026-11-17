;## host/standalone.scm — the standalone editor's side of the shell's hooks
;## (docs/editor-crate-plan.md, E4), loaded after the shell and before the
;## user's ~/.config/edit/init.scm. Three panes, fixed: the editor, the
;## bottom PANEL under it — a terminal or the REPL, or hidden — and the
;## minibuffer (src/app.rs). What the modeler's 05-panes, 08-editor and
;## unify do for the shell, for these.

; ── The panes, by key ──────────────────────────────────────────────────────
(set! *pane-id*
  (lambda (key)
    (case key
      ((:editor) 1)
      ; The bottom panel: the REPL's transcript, or the terminal (E5).
      ((:transcript :repl :terminal) 2)
      ((:minibuffer) 3)
      ((:files) 4)
      ((:agent) 5)
      (else #f))))
(set! *pane-key*
  (lambda (id)
    (case id
      ((1) :editor)
      ((2) :transcript)
      ((3) :minibuffer)
      ((4) :files)
      ((5) :agent)
      (else #f))))

; ── The file tree: the left column (17-files) ──────────────────────────────
; Rooted at the git checkout the editor was started in, laid out as a list,
; and following the buffer on screen.
(%pane-top-anchor 4 1)
; Names in IBM Plex Sans, as Zed's project panel sets them (it ships the
; face as Zed Plex Sans); the marks and folder cells stay on the mono grid.
(%pane-proportional 4 1)
(set! *files-shown* (lambda (on) (%show-files on)))
(set! *files-shown?* (lambda () (%files-shown?)))
(set! *buffers-changed*
  (lambda ()
    (when (%files-shown?)
      (files-reveal (buffer-path (%editor-active-buffer (editor-pane-id)))))))

; A prompt that closes gives the keyboard back to the editor.
(set! *prompt-closed* (lambda () (focus-scene-editor)))

; ── Where a send's answers go: the transcript ─────────────────────────────
(set! *transcript-append*
  (lambda (text (style #f))
    (%pane-append 2 (%text (or style :normal)) (%describe-part text))))

; WHERE A SEND GOES (Kevin, 2026-11-13: a loaded .scm file should run in an
; scm process of its own). A Scheme FILE is the program being written, so it
; runs in a separate scm — started on the first send, one for every such
; file — where a runaway loop is interrupted, a crash takes only that
; process, and its names never collide with the editor's. What configures
; the EDITOR stays in the editor's own image, as Emacs's *scratch* does:
; the scratch buffer, ~/.config/edit/init.scm, and the shell's own files.
; M-x run-in-editor-image points one file there too; anything not Scheme
; sends nowhere. (A Guile file's language sends it to Guile first, 15-guile.)
(define *scm-files* #f)           ; the separate scm for Scheme files, once started

(define (scheme-path? p)
  (any (lambda (ext) (string-suffix? ext p)) '(".scm" ".ss" ".sld" ".sls")))

(define (editor-own-file? p)
  "Is P one of the files that configure this editor — the user's init, or
the shell's own (edit/scm/)?"
  (let ((rp (real-file p)))
    (or (equal? rp (real-file (expand-home "~/.config/edit/init.scm")))
        (string-prefix? (string-append (real-file (%edit-path "scm")) "/") rp))))

(define (scm-files-backend)
  ; The separate scm, started on first use; a send FAILS if it cannot start.
  (guard (e (#t (unavailable-backend "scm" e)))
    (unless (and *scm-files* (%remote-address *scm-files*))
      (set! *scm-files* (%remote-start *remote-program* 0 '()))
      (echo "scm running at " (%remote-address *scm-files*) " for the Scheme files"))
    (remote-backend *scm-files* "scm")))

; A RAY file (21-ray: the modeline says "Scheme (ray)") runs in ray, whose
; image has the verbs it calls — `ray --serve`, the protocol scm's speaks
; (Kevin, 2026-11-15). One ray for each checkout, run from its root so a
; scene's (load "examples/…") finds what it does from a terminal there, and
; that checkout's own build (target/release/ray) before the PATH's; a file
; outside any checkout gets one run from its own directory. Each send names
; its file first, (%including "/abs/file.scm"), so (include "frame.scm")
; resolves beside the file as `ray FILE` would. M-x remote-connect attaches
; a buffer to a `ray --tcp` already running instead — ray-view's.
(define *ray-program* #f)       ; #f: the checkout's build, else "ray"
(define *ray-servers* (make-hash-table equal?))   ; directory → connection

(define (ray-program dir)
  (or *ray-program*
      (let ((built (string-append dir "/target/release/ray")))
        (if (file-exists? built) built "ray"))))

(define (ray-file-backend path)
  ; The ray for PATH's checkout (or directory), started on first use; a send
  ; FAILS if it cannot start, saying why.
  (define dir (ray-dir path))
  (guard (e (#t (unavailable-backend "ray" e)))
    (let ((conn (hash-table-ref/default *ray-servers* dir #f)))
      (unless (and conn (%remote-address conn))
        (set! conn (%remote-start (ray-program dir) 0 '("--serve" "{addr}") dir))
        (hash-table-set! *ray-servers* dir conn)
        (echo "ray running at " (%remote-address conn) " for " dir))
      (ray-sends-from path (remote-backend conn "ray")))))

(define (ray-sends-from path backend)
  ; BACKEND with each send prefixed by (%including PATH), on the send's own
  ; first line so a reported line number stays the buffer's.
  (define (from src) (string-append "(%including " (*show-value* path) ") " src))
  (map (lambda (op)
         (match op
           ((:eval . f) (cons :eval (lambda (src) (f (from src)))))
           ((:eval-async . f) (cons :eval-async (lambda (src k) (f (from src) k))))
           (_ op)))
       backend))

(define (ray-dir path)
  ; Where PATH's ray runs: its checkout's root, else its own directory.
  (or (ray-checkout-root path) (path-parent path) "."))

(define (ray-restart)
  "Stop the ray this buffer's sends go to — M-x ray-restart — so the next
send starts a fresh one: no definitions, no scene, nothing left from before.
Every ray file of the same checkout shares it, so they all start over."
  (let* ((buf (%editor-active-buffer (editor-pane-id)))
         (path (buffer-path buf)))
    (if (not (and path (ray-buffer? buf)))
      (echo "not a ray file — the modeline says Scheme (ray) of one")
      (let* ((dir (ray-dir path))
             (conn (hash-table-ref/default *ray-servers* dir #f)))
        (if (not conn)
          (echo "no ray running for " dir " — the next send starts one")
          (begin
            (%remote-close conn)
            (hash-table-delete! *ray-servers* dir)
            (echo "ray for " dir " stopped — the next send starts a fresh one")))))))

(define (ray-buffer? buf)
  (equal? (%buffer-dialect (editor-pane-id) buf) "ray"))

; Which image the WARNINGS judge a buffer against (22-warnings) — where its
; sends go, without starting anything to find out: a ray file's ray and a
; Scheme file's scm once they run. Before its scm runs a Scheme file is
; judged against this image, which binds all that plain scm does and more,
; so it can miss a warning but not invent one; a ray file before its ray
; runs is not judged, since this image has none of ray's verbs.
(set! *buffer-image*
  (lambda (buf)
    (let ((p (buffer-path buf))
          (running (lambda (c) (and c (%remote-address c) c))))
      (cond ((not p) #f)
            ((hash-table-ref/default *attached-backends* buf #f)
              => (lambda (b) (or (%alist-ref b :conn #f) 'self)))
            ((or (string-prefix? "*" p) (editor-own-file? p)) 'self)
            ((ray-buffer? buf) (running (hash-table-ref/default *ray-servers* (ray-dir p) #f)))
            ((scheme-path? p) (or (running *scm-files*) 'self))
            (else #f)))))

(set! *buffer-backend*
  (lambda (buf)
    (let ((p (buffer-path buf)))
      (cond ((or (not p) (string-prefix? "*" p) (editor-own-file? p)) (in-process-backend))
            ((ray-buffer? buf) (ray-file-backend p))
            ((scheme-path? p) (scm-files-backend))
            (else #f)))))

(define (run-in-editor-image)
  "Send this buffer's forms to the EDITOR's own image — where the editor's
  commands and keymaps live — rather than to the separate scm a Scheme file
  goes to. For code that extends the editor. remote-detach undoes it."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (hash-table-set! *attached-backends* buf (in-process-backend))
    (echo (%text (buffer-name buf)) " sends to the editor's image")))

; ── Files ──────────────────────────────────────────────────────────────────
(define *last-open-path* #f)

(define (open-path p)
  "Open the file at P in a buffer, as C-x C-f does: markdown in the reading
  view, a .png/.jpg as a picture, anything else as text. Runs nothing. The
  files named on the command line open this way too."
  (let ((eid (editor-pane-id))
        (low (string-downcase p)))
    (define (viewer open)
      (%editor-buffer-new eid)
      (open eid p)
      (register-buffer (%editor-active-buffer eid) p)
      (visit-buffer (%editor-active-buffer eid)))
    (cond ((string-suffix? ".md" low) (viewer %editor-open-markdown))
          ((any (lambda (e) (string-suffix? e low)) '(".png" ".jpg" ".jpeg")) (viewer %editor-open-image))
          ((video-path? p) (viewer %editor-open-video))
          (else (visit-file-text p)))))

(define (find-file)
  "Open a file in a buffer — C-x C-f. Runs nothing. Markdown opens in the
  reading view and a .png/.jpg as a picture; anything else is text. TAB
  completes the path."
  (read-minibuffer
    "Find file: "
    (lambda (p)
      (set! *last-open-path* p)
      (open-path p))
    :files
    *last-open-path*))

; ── Keys ───────────────────────────────────────────────────────────────────
(bind-key "C-x C-f" 'find-file)
(bind-editor-pane-keys)
(bind-editor-completion-keys)
(bind-editor-keys :minibuffer)
(bind-editor-keys :repl)
(bind-term-keys)
(bind-files-keys)
; The lower pane is the REPL OR the terminal, one keymap between them: Cmd-C
; and Cmd-V act on whichever shows.
(bind-key "s-c" (labelled "repl-copy" (lambda () (if (equal? (%panel) :terminal) (term-copy) (repl-copy)))) :repl)
(bind-key "s-v" (labelled "repl-paste" (lambda () (if (equal? (%panel) :terminal) (term-paste) (repl-paste)))) :repl)
(bind-key "C-x C-c" (labelled "quit" (lambda () (%quit))))
; vi's :q and :e (19-ex).
(set! *ex-quit* (lambda () (%quit)))
(set! *ex-open* (lambda (path) (open-path (collapse-path path))))

; ── The bottom panel: a terminal or the REPL ───────────────────────────────
; Kevin, 2026-11-13: two icons at the strip's right end (src/app.rs), one
; for each, mutually exclusive — the panel shows one or the other, and the
; lit icon clicked again hides the whole panel, as the modeler's dock
; toggles do. The terminal runs your login shell; the REPL is a prompt over the
; image this editor's sends go to, so what C-M-x defined, it can see.

(define (show-panel kind)
  "Show the bottom panel with KIND in it — :terminal or :repl — and give
  it the keyboard. The terminal's first show starts your login shell."
  (case kind
    ((:terminal) (%show-panel :terminal) (if (term-id) (term-focus) (term-open)))
    ((:repl) (%show-panel :repl) (repl-focus))))

(define (toggle-panel kind)
  "The strip icon for KIND (:terminal or :repl): show the panel with KIND,
  or, when it already shows KIND, hide the panel. A terminal's program runs
  on while it is hidden."
  (if (equal? (%panel) kind)
    (begin (%show-panel #f) (focus-scene-editor))
    (show-panel kind)))

(define (toggle-terminal)
  "Show or hide the terminal in the bottom panel — the strip's terminal icon."
  (toggle-panel :terminal))

(define (toggle-repl)
  "Show or hide the REPL in the bottom panel — the strip's λ icon."
  (toggle-panel :repl))

(define (repl)
  "Show the REPL and give it the keyboard — C-c C-z, SLIME's switch to the
  REPL."
  (show-panel :repl))
(bind-key "C-c C-z" 'repl)

; A terminal that opens shows; one that closes leaves the REPL.
(set! *term-shown* (lambda (id) (%show-panel :terminal)))
(set! *term-closed* (lambda (id) (%show-panel :repl)))
(set! *term-showing?* (lambda (id) (equal? (%panel) :terminal)))
; What prints to the transcript — a send's answer, show-keys, describe —
; is the REPL's, so it shows the REPL when it was hidden. A terminal on
; show stays: a C-M-x's value is on the echo line too.
(set! *help-open* (lambda () (unless (%panel) (%show-panel :repl))))

; ── The REPL: the transcript with a prompt ─────────────────────────────────
; It evaluates where the ACTIVE BUFFER's sends do — this image, the scm
; image remote-start / remote-connect attached (14-remote), or a Guile
; buffer's Guile (15-guile) — so the REPL and the buffer always agree, and
; its prompt says which: scm's `scheme λ > `, or Guile's own
; `scheme@(guile-user)> `. RET submits a form the reader says is complete
; and continues one that is not; up and down walk the history.
(define *repl-history* '())
(define *repl-pending* "")

(define (repl-backend)
  ; Where the REPL's forms go: the active buffer's backend, else this image
  ; — never a terminal one, which is a program with its own prompt.
  (let ((b (backend-of (%editor-active-buffer (editor-pane-id)))))
    (if (and b (not (equal? (%alist-ref b :name #f) "the terminal"))) b (in-process-backend))))

(define (repl-guile?)
  (string-prefix? "guile" (%alist-ref (repl-backend) :name "")))

(define (repl-prompt)
  ; The prompt says where the REPL is: Guile's own (its language and
  ; module), the separate scm's `scm λ > `, or the editor's image's
  ; `scheme λ > ` — the λ kept for both scm images (Kevin, 2026-11-13). A
  ; continuation lines up under the form, as the prompt's width of spaces.
  (let ((p (cond ((repl-guile?) "scheme@(guile-user)> ")
                 ((string-prefix? "scm at " (%alist-ref (repl-backend) :name "")) "scm λ > ")
                 (else "scheme λ > "))))
    (if (string=? *repl-pending* "") p (make-string (string-length p) #\space))))

; Copy and paste at the REPL: Cmd-C copies a mouse selection in the
; transcript (drag to select), else the input line; Cmd-V pastes keeping the
; lines — all but the last wait as pending source, nothing runs until RET.
(define (repl-copy)
  "Copy the REPL's selection — or, with none, its input line — to the
  clipboard: Cmd-C at the REPL, and Copy on its menu."
  (let ((s (%pane-selection 2)))
    (cond (s (%set-clipboard s) (%pane-clear-selection 2) (echo "copied " (string-length s) " chars") s)
          ((%editing-pane? 2) (edit-copy-line) (%edit :text))
          (else (echo "nothing selected — drag to select") #f))))

(define (repl-paste)
  "Paste the clipboard at the REPL prompt, its lines intact: all but the
  last wait as pending source (the continuation prompt marks them) and the
  last sits editable — nothing is evaluated until RET. Cmd-V at the REPL,
  and Paste on its menu."
  (let ((t (%clipboard)))
    (and t
         (begin
           (unless (%editing-pane? 2) (repl-focus))
           (let ((lines (string-split t "\n")))
             (for-each
               (lambda (l)
                 (*transcript-append* (string-append (repl-prompt) l) :accent)
                 (set! *repl-pending* (string-append *repl-pending* l "\n")))
               (reverse (cdr (reverse lines))))
             (%edit! :prompt (repl-prompt))
             (edit-insert (car (last-pair lines))))
           t))))

(define (repl-focus)
  "Give the REPL's prompt the keyboard."
  (%focus-pane 2)
  (set! *focus-held* #t)
  (edit-start 2 (repl-prompt) repl-submit :history (lambda () *repl-history*)))

(define (repl-print src r)
  ; What a REPL line printed, then its value or its error.
  (let ((printed (string-trim-right (%alist-ref r :output ""))))
    (unless (string=? printed "") (*transcript-append* printed))
    (if (eq? (%alist-ref r :status #f) 'ok)
      (let ((v (%alist-ref r :printed "")))
        (unless (or (string=? v "") (string=? v "#<unspecified>"))
          (*transcript-append* (string-append "=> " v) :dim)))
      (begin
        (*transcript-append* (string-append "error: " (%alist-ref r :message "")) :error)
        (note-send-error! "repl" src (%alist-ref r :message "") (%alist-ref r :frames '()))))))

(define (repl-eval-print src (then #f))
  "Evaluate SRC where the REPL does and print what it printed, then its
  value or its error; THEN, when given, runs after. In another process the
  line answers LATER (the backend's :eval-async), so the prompt is back at
  once and the window stays live while it runs."
  (let ((b (repl-backend)))
    (cond ((backend-op b :eval-async)
            => (lambda (eval-async)
                 (eval-async src (lambda (r) (repl-print src r) (when then (then))))))
          (else (repl-print src ((backend-op b :eval) src))
                (when then (then))))))

(define (repl-submit line)
  (let ((src (string-append *repl-pending* line)))
    (*transcript-append* (string-append (repl-prompt) line) :accent)
    (cond
      ; A comma line is a REPL COMMAND: Guile's own run in Guile, these in
      ; scm (below) — the same names at either prompt.
      ((and (string=? *repl-pending* "") (repl-command-line? line) (not (repl-guile?)))
        (set! *repl-history* (cons line *repl-history*))
        (repl-command line (repl-backend) repl-eval-print))
      ((eq? (%read-status src) 'incomplete)
        (set! *repl-pending* (string-append src "\n")))
      (else
        (set! *repl-pending* "")
        (set! *repl-history* (cons src *repl-history*))
        (repl-eval-print src)))
    (repl-focus)))

; A click in the REPL's pane gives it the keyboard.
(add-handler
  *click-handlers*
  50
  :repl-focus
  ; Not while a menu is up: the click is the menu's.
  (lambda (x y . _)
    (let ((hit (%pane-at x y)))
      (and (not (menu-open?)) hit (equal? (car hit) 2) (equal? (cadr hit) "text")
           (begin (repl-focus) #t)))))

; Running a FILE is loading it, and the panel shows where its answers land
; (Kevin: "if you load a scene, then it should load a repl"): the REPL, or
; the terminal for a buffer attached to one. A one-form send leaves the panel
; as it is; its value is on the echo line.
(set! *before-run-file*
  (lambda ()
    (let ((b (backend-of (%editor-active-buffer (editor-pane-id)))))
      (%show-panel (if (and b (equal? (%alist-ref b :name #f) "the terminal")) :terminal :repl)))))

(define (other-pane)
  "Move the keyboard round the panes that show — C-x o: the editor, the
  bottom panel, the agent, and the editor again."
  (let ((at (%typing-pane)))
    (cond ((equal? at 5) (focus-scene-editor))
          ((and (equal? at 2) (%agent-shown?)) (focus-agent))
          ((equal? at 2) (focus-scene-editor))
          ((%panel) (show-panel (%panel)))
          ((%agent-shown?) (focus-agent))
          (else (echo "nothing else is showing — C-c C-z shows the REPL, M-a the agent")))))

; ── The agent: a column on the right (25-agent) ────────────────────────────
; Kevin, 2026-11-16: Claude Code, on the subscription login, as a
; programming assistant in the editor. Hidden until asked for — its icon at
; the strip's right end, or M-a — and nothing is started until the first
; request. Its conversation goes on while the column is hidden.
(define (agent-greeting)
  (agent-append "Claude, in this project. Ask in English; RET sends." :dim)
  (agent-append "It reads and edits the project's files, and asks here before it runs a program or touches a file outside it." :dim)
  (agent-append "Escape interrupts · C-l or /new starts over · /model, /status, /login · /help" :dim))

(set! *agent-shown*
  (lambda (id)
    (%show-agent #t)
    (when (zero? (%pane-row-count id)) (agent-greeting))))

(define (toggle-agent)
  "Go to the agent — show its column and give its prompt the keyboard — or,
  from its prompt, hide it and go back to the editor: M-a, and the robot
  icon at the strip's right end. A turn it is answering goes on while it is
  hidden."
  (if (and (%agent-shown?) (agent-typing?))
    (begin (%show-agent #f) (focus-scene-editor))
    (focus-agent)))
(bind-key "M-a" 'toggle-agent)

(define (agent-icon-click)
  ; The strip's icon is a plain toggle, as the others are: lit, it hides.
  (if (%agent-shown?)
    (begin (%show-agent #f) (focus-scene-editor))
    (focus-agent)))
(bind-agent-keys)

(define (agent-copy)
  "Copy the agent pane's selection — or, with none, its input line — to
  the clipboard: Cmd-C there, and Copy on its menu. Drag to select."
  (let ((s (%pane-selection 5)))
    (cond (s (%set-clipboard s) (%pane-clear-selection 5) (echo "copied " (string-length s) " chars") s)
          ((%editing-pane? 5) (edit-copy-line) (%edit :text))
          (else (echo "nothing selected — drag to select") #f))))
(bind-key "s-c" 'agent-copy :agent)
(bind-key "s-v" 'agent-paste :agent)

; A click in the agent's column gives its prompt the keyboard.
(add-handler
  *click-handlers*
  50
  :agent-focus
  (lambda (x y . _)
    (let ((hit (%pane-at x y)))
      (and (not (menu-open?)) hit (equal? (car hit) 5)
           (begin (focus-agent) #t)))))

; THE CONVERSATION SURVIVES A RESTART: its id is filed by the directory it
; runs in, in ~/.config/edit/agent-sessions.scm, and the next request there
; resumes it. C-l forgets it.
; What the standalone lets the agent call unasked (25-agent's
; editor_command): its themes and its panels. load-theme with a name applies
; it for this run; save-theme-choice makes it the one edit opens in.
(set! *agent-ui-commands*
  (append *agent-ui-commands*
          '("load-theme" "save-theme-choice" "theme-names"
            "repl" "toggle-repl" "toggle-terminal" "toggle-files" "toggle-agent" "other-pane")))

; LEAVE GIVEN FOR A PLACE IS KEPT, by project, in
; ~/.config/edit/agent-grants.scm — a file to read, and to edit.
(define *agent-grants-file* "~/.config/edit/agent-grants.scm")
(define *agent-grants-read* #f)

(define (agent-grants-load)
  (unless *agent-grants-read*
    (set! *agent-grants-read* #t)
    (guard (e (#t #f))
      (let ((path (expand-home *agent-grants-file*)))
        (when (file-exists? path)
          (for-each (lambda (kv) (when (and (pair? kv) (string? (car kv)) (list? (cdr kv)))
                                   (hash-table-set! *agent-grant-table* (car kv) (cdr kv))))
                    (call-with-input-file path read)))))))

(set! *agent-grants-ref*
  (lambda (project) (agent-grants-load) (hash-table-ref/default *agent-grant-table* project '())))
(set! *agent-grants-set!*
  (lambda (project grants)
    (agent-grants-load)
    (hash-table-set! *agent-grant-table* project grants)
    (guard (e (#t (echo "agent: could not save " *agent-grants-file* ": " (*show-value* e))))
      (spit (expand-home *agent-grants-file*)
            (string-append
              ";; Where the agent may read and write outside each project without asking (edit, 25-agent).\n"
              ";; ((project (directory . \"write\" | \"read\") …) …) — /grants lists, /revoke forgets.\n"
              (let ((out (open-output-string)))
                (write (filter (lambda (kv) (pair? (cdr kv)))
                               (map (lambda (project) (cons project (hash-table-ref/default *agent-grant-table* project '())))
                                    (hash-table-keys *agent-grant-table*)))
                       out)
                (get-output-string out))
              "\n")))))

(define *agent-session-file* "~/.config/edit/agent-sessions.scm")
(define *agent-sessions-read* #f)

(define (agent-sessions-load)
  (unless *agent-sessions-read*
    (set! *agent-sessions-read* #t)
    (guard (e (#t #f))
      (let ((path (expand-home *agent-session-file*)))
        (when (file-exists? path)
          (for-each (lambda (kv) (when (and (pair? kv) (string? (car kv)) (string? (cdr kv)))
                                   (hash-table-set! *agent-sessions* (car kv) (cdr kv))))
                    (call-with-input-file path read)))))))

(define (agent-sessions-save)
  (guard (e (#t (echo "agent: could not save " *agent-session-file* ": " (*show-value* e))))
    (spit (expand-home *agent-session-file*)
          (string-append
            ";; The agent's conversations, by the directory each runs in (edit, 25-agent).\n"
            (let ((out (open-output-string)))
              (write (filter cdr (map (lambda (dir) (cons dir (hash-table-ref/default *agent-sessions* dir #f)))
                                      (hash-table-keys *agent-sessions*)))
                     out)
              (get-output-string out))
            "\n"))))

(set! *agent-session-ref*
  (lambda (dir) (agent-sessions-load) (hash-table-ref/default *agent-sessions* dir #f)))
(set! *agent-session-set!*
  (lambda (dir id)
    (agent-sessions-load)
    (unless (equal? id (hash-table-ref/default *agent-sessions* dir #f))
      (hash-table-set! *agent-sessions* dir id)
      (agent-sessions-save))))

(define (edit-open-files files)
  "The file tree at startup: rooted at the git checkout the first FILE is
  in — or the one the editor was started in — showing that file."
  (let ((root (files-open-root (if (pair? files) (let ((d (files-dir-of (car files)))) d) "."))))
    ; M-. and M-? search the checkout the tree shows — every Scheme file in
    ; it, not only the ones opened (E8): references over the files opened
    ; so far would be a fraction of them. The index skips target/ and .git.
    ; ONLY a checkout: outside git the root is the directory itself, and
    ; `edit` started in ~ walked the whole home directory before the window
    ; opened (2026-11-15). There the index is the files opened and saved.
    (when (%git-root root)
      (%xref-index (list root))))
  (when (pair? files) (files-reveal (car files))))

(define (files-dir-of p)
  ; The directory P names, or the one it is in.
  (let ((rp (or (real-file p) p)))
    (if (files-dir? rp) rp
        (let loop ((k (- (string-length rp) 1)))
          (cond ((< k 0) ".") ((char=? (string-ref rp k) #\/) (if (zero? k) "/" (substring rp 0 k))) (else (loop (- k 1))))))))

; The panel starts HIDDEN (Kevin, 2026-11-13: neither the REPL nor a
; terminal at startup): the editor has the height, and no shell starts until
; asked for. It shows when wanted — its icons, C-c C-z, running a file,
; a help screen that prints into the transcript.

; ── Menus: a right-press opens its pane's ──────────────────────────────────
; A quick click posts the menu as a list; a press held or dragged is the
; marking menu (the shell's 03-menus). The editor's is the modeler's: the
; sends around the compass, then describe, copy and paste.
(set! *pane-role*
  (lambda (id kind)
    (case id
      ((1) :editor)
      ((2) (if (equal? kind "term") :terminal :transcript))
      ((4) :files)
      ((5) :agent)
      (else #f))))

(define (toggle-markdown-view)
  "Flip a markdown buffer between its reading and source views — C-c C-v."
  (let ((r (%editor-toggle-markdown (editor-pane-id))))
    (cond ((eqv? r 1) (echo "markdown: reading view"))
          ((eqv? r 0) (echo "markdown: source view"))
          (else (echo "not a markdown buffer")))))
(bind-key "C-c C-v" (lambda () (if (editor-focused?) (toggle-markdown-view) #f)))
(hash-table-set! *editor-key-labels* "C-c C-v" "toggle-markdown-view")

(define-menu
  :editor
  (list (list "send defun" eval-defun)
        (list "send sexp" eval-last-sexp)
        (list "send buffer" eval-buffer)
        (list "send region" eval-region)
        (list "describe" editor-describe)
        (list "Copy" scene-editor-copy "s-c")
        (list "Paste" scene-editor-paste "s-v")
        (list "Find…" search-forward "C-s")
        (list "Replace…" query-replace "M-%")))
(define-menu
  :help
  (list (list "back" editor-back) (list "toggle source" toggle-markdown-view)))
(define-menu
  :terminal
  (list (list "back to the editor" focus-scene-editor "C-x o")
        (list "Copy" term-copy "s-c")
        (list "Paste" term-paste "s-v")
        (list "send C-c" (lambda () (%term-key 2 "C-c")) "C-c C-c")
        (list "REPL" repl)
        (list "hide" toggle-terminal)
        (list "stop the program" term-close)))
(define-menu
  :transcript
  (list (list "back to the editor" focus-scene-editor "C-x o")
        (list "Copy" repl-copy "s-c")
        (list "Paste" repl-paste "s-v")
        (list "terminal" toggle-terminal)
        (list "hide" toggle-repl)
        (list "clear" (lambda () (%pane-clear 2)))))

(set-pane-menu! :editor
  (lambda (x y id shift)
    (%focus-pane id)
    (cond ((markdown-buffer?) :help)
          ((video-buffer?) :video)
          ((backend-of (%editor-active-buffer id)) :editor)
          (else #f))))
(define-menu
  :agent
  (list (list "New conversation" new-conversation "C-l")
        (list "Interrupt" agent-interrupt "Esc")
        (list "Copy" agent-copy "s-c")
        (list "Paste" agent-paste "s-v")
        (list "back to the editor" focus-scene-editor "C-x o")
        (list "hide" toggle-agent "M-a")))
(set-pane-menu! :agent (lambda (x y id shift) :agent))
(set-pane-menu! :terminal (lambda (x y id shift) :terminal))
(set-pane-menu! :transcript (lambda (x y id shift) :transcript))

; ── The mouse's floor: a transcript scrolls ────────────────────────────────
(add-handler
  *wheel-handlers*
  10
  :transcript-scroll
  (lambda (d x y (alt #f) (shift #f))
    (let ((hit (and x y (%pane-at x y))))
      (and hit (equal? (cadr hit) "text")
           (begin (%pane-scroll (car hit) (* 3 (if (positive? d) 1 -1))) #t)))))

(define (edit-greeting)
  "What the transcript says when the editor opens."
  (*transcript-append* "edit — C-x C-f opens a file, C-x C-s saves. A Scheme file runs in an scm of its own;" :dim)
  (*transcript-append* "*scratch* and your init.scm run in the editor's image." :dim)
  (*transcript-append* "C-M-x / C-x C-e / C-c C-b send, C-h b lists every key, C-x C-c quits." :dim)
  (*transcript-append* "This is the REPL (C-c C-z). The icons at the bottom right swap it for a terminal, or hide" :dim)
  (*transcript-append* "the panel; C-x o moves the keyboard. Right-click a pane for its menu, hold or drag for the pie." :dim)
  (*transcript-append* "M-a, or the robot icon, opens the agent: Claude, as a programming assistant in this project." :dim))
