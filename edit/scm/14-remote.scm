;## 14-remote.scm — a SOCKET backend: SLIME's model, over scm's own REPL
;## protocol (docs/editor-crate-plan.md, E5b). The terminal (13-term) is
;## inferior-lisp: a send rewrites the running program, and nothing
;## structured comes back. Here `scm --serve` is the SWANK half, and a send
;## answers as one in this image does — a value, what the form printed, an
;## error with the calls it was in — from ANOTHER process's image. A runaway
;## loop or a crash stays in that process. The client is edit's
;## scm_binding/remote.rs.

; THE BACKEND (Kevin, 2026-11-23: "if you can have multiple backends
; started, I say no"): ONE Scheme in a process of its own at a time,
; started by remote-start -- or scm-start, lux-start, guile-start --
; or attached by remote-connect. While it runs every Scheme file sends to
; it (the host's *buffer-backend*); starting another, or a Lisp file's
; SBCL (27-lisp), stops it.
(define *remote* #f)              ; the current connection's id, or #f
(define *remote-kind* "scm")      ; which Scheme it is — a *schemes* name
(define *remote-timeout* 30)      ; seconds a send waits before interrupting
(define *remote-program* "scm")   ; what remote-start runs for scm, with --serve
(define *lux-program* "lux")      ; what remote-start runs for lux: lux FILE --serve ADDR

; THE SCHEMES remote-start can start (Kevin, 2026-11-13: "you should be able
; to choose which scheme you want to start"): each a name, the program and
; its arguments (thunks, so a changed *…-program* is seen; "{addr}" stands
; for the address, and no arguments means scm's `--serve {addr}`), and the
; LANGUAGE a buffer sending to it is set to. 15-guile adds Guile; a third is
; one more define-scheme!.
(define *schemes* '())

; Whether this HOST lets a buffer send to another process at all, as a
; FORWARD HOOK answering #t, or a string saying why not. The standalone
; does; the modeler does not — its image IS the scene, so a send to another
; Scheme would answer `=> …` and change nothing on screen (Kevin,
; 2026-11-13). A language's own backend (a Guile buffer's Guile) is not
; this: a Guile file is never a scene.
(define *remote-allowed?* (lambda () #t))

(define (%remote-refused)
  ; #f when remote is allowed, else echo why and answer #t.
  (let ((ok (*remote-allowed?*)))
    (and (not (eq? ok #t))
         (begin (echo (if (string? ok) ok "no remote Scheme in this host")) #t))))

(define (define-scheme! name program args language)
  "Make NAME a Scheme remote-start can start: PROGRAM and ARGS, thunks
answering the program and its arguments, and LANGUAGE, the editor language
a buffer sending to it is set to."
  (set! *schemes* (append (filter (lambda (s) (not (equal? (car s) name))) *schemes*)
                          (list (list name program args language))))
  name)

(define-scheme! "scm" (lambda () *remote-program*) (lambda () '()) "scheme")

; lux (k2msmith-gfx/lux): live-coded 3D in scm. It runs the buffer's FILE,
; as saved, in a window of its own -- a buffer with no file, nothing, the
; background until something is sent to draw -- serves this protocol
; between frames, and pushes its pauses (remote-event). An error loading
; the file pauses it too, rather than ending it, so the fix can be sent
; (Kevin, 2026-11-22: "i should be able to start lux from within edit and
; connect to it"). *lux-program* is lux's tools/lux, linked onto PATH: it
; runs lux with the libraries lux's dev shell supplies.
(define (lux-args)
  (let ((path (buffer-path (%editor-active-buffer (editor-pane-id)))))
    (if (and (string? path) (file-exists? path))
      (list path "--serve" "{addr}")
      '("--serve" "{addr}"))))

(define-scheme! "lux" (lambda () *lux-program*) lux-args "scheme")

(define (remote-frames calls locals)
  ; The wire's calls, each with its locals when the server sent them (scm's
  ; S43 and SBCL's both send one list a call), as error-pane frames.
  (map remote-frame calls (if (= (length locals) (length calls)) locals (map (lambda (c) '()) calls))))

(define (remote-result r (conn #f))
  ; The wire's answer (%remote-eval / %remote-poll) as a backend RESULT.
  ; :value is the printed value — the live one is in the other process —
  ; and truthy, so the transcript shows it. What only some servers say is
  ; kept as it came: SBCL's break (27-lisp) — its level, its restarts, a
  ; restart waiting for a value, the package the next line reads in — and
  ; its compiler's warnings, with where each one's form began.
  (define said
    (filter (lambda (kv) (memq (car kv) '(:level :restarts :asking :package :warnings :warning-at))) r))
  (append
    (if (eq? (%alist-ref r :status #f) 'ok)
      (list (cons :status 'ok)
            (cons :value (%alist-ref r :value ""))
            (cons :printed (%alist-ref r :value ""))
            (cons :output (%alist-ref r :output ""))
            (cons :secs 0))
      (list (cons :status 'error)
            (cons :message (string-append
                             (%alist-ref r :message "")
                             (if (%alist-ref r :host-stopped #f) "  (stopped)" "")))
            (cons :output (%alist-ref r :output ""))
            (cons :frames (begin
                            ; A scm server's calls with their locals can be
                            ; evaluated in (eval-in-frame); SBCL's are in
                            ; its own break, at the REPL.
                            (when (and conn (pair? (%alist-ref r :locals '())) (not (assq :restarts r)))
                              (set! *break-conn* conn))
                            (remote-frames (%alist-ref r :backtrace '()) (%alist-ref r :locals '()))))))
    said))

(define (failed-result e)
  (list (cons :status 'error)
        (cons :message (if (string? e) e (*show-value* e)))
        (cons :output "")))

; ── Async sends (SLIME's model) ────────────────────────────────────────────
; A send to another process does not wait: :eval-async returns at once, and
; the answer arrives later — the connection's reader thread queues it and
; wakes the host, whose loop calls (remote-poll), which hands each answer to
; the callback its send left here. The window stays live while a form runs.
(define *remote-callbacks* (make-hash-table equal?))   ; (conn . request-id) → k

; What else runs when the host is woken (20-rust: a language server's
; argument hint, which arrives as a socket's answer does).
(define *wake-hooks* '())

; ── Events (scm's S40) ─────────────────────────────────────────────────────
; What a server says without being asked. remote-connect asks for them; lux
; sends (:event paused :message … :backtrace (…)) when an error in a frame
; pauses it -- after the send that caused it was answered ok, so without
; this the editor never hears -- and (:event resumed) when it draws again.
; A paused program shows as a failed send does: the transcript, the echo,
; and the error pane (C-h e) with its calls.

(define (remote-event conn ev)
  "Show EV, an event connection CONN's server pushed: an alist with :event
and the event's own keys."
  (let ((at (string-append "the program at " (or (%remote-address conn) "?"))))
    (case (%alist-ref ev :event #f)
      ((paused)
        (let ((message (%alist-ref ev :message "")))
          (*transcript-append* (string-append "paused: " message) :error)
          (echo "paused: " message)
          (when (pair? (%alist-ref ev :locals '())) (set! *break-conn* conn))
          (note-send-error! at "" message (remote-frames (%alist-ref ev :backtrace '()) (%alist-ref ev :locals '())))))
      ((resumed)
        (*transcript-append* "resumed" :dim)
        (echo "resumed"))
      (else #f))))

(define (remote-poll)
  "Run *wake-hooks*; deliver every answer an async send is owed, calling
each send's callback with its RESULT; show each event a server pushed
(remote-event); and say on the modeline where the buffer on screen runs.
The host calls this when it is woken."
  (for-each (lambda (h) (h)) *wake-hooks*)
  (for-each
    (lambda (row)
      (match row
        ((conn rid r)
          (let ((k (hash-table-ref/default *remote-callbacks* (cons conn rid) #f)))
            (hash-table-delete! *remote-callbacks* (cons conn rid))
            (when k (k (remote-result r conn)))))))
    (%remote-poll))
  ; After the answers: a send's ok is shown before the pause it caused.
  (for-each (lambda (row) (remote-event (car row) (cadr row))) (%remote-events))
  (status-refresh)
  #f)

(define (remote-backend conn (label "scm"))
  "A backend that asks the image at connection CONN — scm's, or anything
that speaks its protocol (Guile's serve.scm, 15-guile): sends evaluate
there, completion and docs answer from its live bindings, and M-. stays
with this editor's index of the files. LABEL names it.

A send is ASYNC (:eval-async): it returns at once and its answer arrives
through remote-poll, so the window stays live. While a send is running,
completion and describe answer nothing rather than queue behind it — the
server does one thing at a time. :eval still waits, for the commands that
need a value now."
  (define (idle?) (zero? (%remote-running conn)))
  (define (describe name) (and (idle?) (guard (e (#t #f)) (%remote-describe conn name))))
  (list (cons :name (string-append label " at " (or (%remote-address conn) "?")))
        (cons :conn conn)
        (cons :eval
              (lambda (src)
                (guard (e (#t (failed-result e)))
                  (remote-result (%remote-eval conn src #f *remote-timeout*) conn))))
        (cons :eval-async
              (lambda (src k)
                (guard (e (#t (k (failed-result e))))
                  (hash-table-set! *remote-callbacks* (cons conn (%remote-eval-async conn src #f)) k))))
        (cons :complete (lambda (prefix) (if (idle?) (guard (e (#t '())) (%remote-complete conn prefix)) '())))
        (cons :doc (lambda (name) (let ((d (describe name))) (and d (%alist-ref d :doc #f)))))
        (cons :signature (lambda (name) (let ((d (describe name))) (and d (%alist-ref d :signature #f)))))
        (cons :definition-sites (lambda (name) (definition-sites name)))))

(define (on-first-send label start (also '()))
  "A backend for a program that is not running yet: START, a thunk that
starts it and answers its backend, runs on the first SEND, and not before
-- a menu, the REPL's prompt, completion and describe all ask a buffer for
its backend, and asking must not start a program. Until then this offers
only the sends, and ALSO: what answers without the program (SBCL's REPL
prompt, 27-lisp: a Lisp file's SBCL is the one program that starts by
itself)."
  (append
    (list (cons :name label)
          (cons :eval (lambda (src) ((backend-op (start) :eval) src)))
          (cons :eval-async
                (lambda (src k)
                  (let* ((b (start)) (async (backend-op b :eval-async)))
                    (if async (async src k) (k ((backend-op b :eval) src)))))))
    also))

; WHERE A BUFFER RUNS, on its modeline: "lux at 127.0.0.1:4321" -- the
; other process its sends go to -- and nothing for this image's own
; (Kevin, 2026-11-23: "there should also be a status message when starting
; or connecting to a language ... in the edit status bar"). Refreshed after
; every key, click, release and wake, for the buffer on screen.
(define (buffer-runs-in buf)
  (let ((b (guard (e (#t #f)) (running-image-backend buf))))
    (if b (%alist-ref b :name "") "")))

(define (status-refresh . _)
  (guard (e (#t #f))
    (let ((buf (%editor-active-buffer (editor-pane-id))))
      (%buffer-set-status (editor-pane-id) buf (buffer-runs-in buf))))
  #f)

(add-handler *key-handlers* -40 :runs-in status-refresh)
(add-handler *click-handlers* -40 :runs-in status-refresh)
(add-handler *release-handlers* -40 :runs-in status-refresh)

(define (unavailable-backend label e)
  "A backend for a Scheme that could not start: every send FAILS, saying
why, rather than falling through to another image, where the code would
run somewhere it was not written for. E is what the start raised."
  (let ((why (if (string? e) e (*show-value* e))))
    (list (cons :name (string-append label " (not running)"))
          (cons :eval (lambda (src)
                        (list (cons :status 'error)
                              (cons :message (string-append label ": " why))
                              (cons :output "")))))))

(define (remote-frame line (locals '()))
  ; "(form)  at src:line:col" as an error-pane frame: (call location locals).
  ; scm's wire carries the calls, not their locals; SBCL's carries both
  ; (:locals, ((name printed) …) a call).
  (let ((at (string-contains line "  at ")))
    (if at
      (list (substring line 0 at) (substring line (+ at 5) (string-length line)) locals)
      (list line #f locals))))

(define (eval-in-frame (frame #f) (src #f))
  "Evaluate an expression in one of the calls the last error was raised in
-- the error pane's (C-h e), numbered as it numbers them -- with that
call's locals in scope, as they were when it raised, in the program it was
raised in; the answer goes to the transcript. A scm server keeps the calls
of its last error (scm S43): lux's paused frame, or a send that failed.
Asks which call and what; (eval-in-frame 1 \"x\") asks nothing."
  (define (go n text)
    (let ((r (guard (e (#t (failed-result e)))
               (remote-result (%remote-eval-in *break-conn* text (- n 1) *remote-timeout*)))))
      (*transcript-append* (string-append "in call " (number->string n) ": " text) :dim)
      (if (eq? (%alist-ref r :status #f) 'ok)
        (begin
          (let ((out (%alist-ref r :output "")))
            (unless (string=? out "") (*transcript-append* out)))
          (*transcript-append* (%alist-ref r :printed ""))
          (echo (%alist-ref r :printed "")))
        (begin
          (*transcript-append* (%alist-ref r :message "") :error)
          (echo (%alist-ref r :message ""))))))
  (define (ask-what n)
    (if src
      (go n src)
      (read-minibuffer (string-append "Evaluate in call " (number->string n) ": ")
                       (lambda (s) (go n s)))))
  (cond
    ((not *break-conn*) (echo "no error kept to evaluate in -- only a scm server keeps one"))
    (frame (ask-what frame))
    (else
      (read-minibuffer "Evaluate in which call (C-h e numbers them): "
                       (lambda (s)
                         (let ((n (string->number (string-trim-both s))))
                           (if (and n (exact-integer? n) (>= n 1))
                             (ask-what n)
                             (echo "a call's number, 1 the innermost"))))
                       :plain
                       "1"))
      #f))

(define (remote-running)
  "The backend's connection while it runs, else #f."
  (and *remote* (%remote-address *remote*) *remote*))

(define (stop-backends)
  ; The one-backend rule: before another starts, the one running stops --
  ; this one, and a Lisp file's SBCL (27-lisp).
  (when *remote*
    (%remote-close *remote*)
    (set! *remote* #f))
  (guard (e (#t #f))
    (when (sbcl-running?) (sbcl-stop))))

(define (remote-took! kind)
  ; The backend is KIND now: the buffer on screen takes its language (a
  ; Guile file Guile's), and the modeline and the echo say where it runs.
  (set! *remote-kind* kind)
  (match (assoc kind *schemes*)
    ((name program args language) (%editor-set-language (editor-pane-id) language))
    (_ #f))
  (status-refresh)
  (echo kind " running at " (%remote-address *remote*) " — the Scheme files send there; M-x remote-stop stops it"))

(define (remote-connect (addr "127.0.0.1:4007") (kind "scm"))
  "(remote-connect [\"HOST:PORT\" [KIND]])  ->  the connection

Attach to a REPL server already running — SLIME's slime-connect — as THE
backend, which every Scheme file sends to until M-x remote-stop; the one
running stops. KIND names the Scheme it is (\"scm\", the default, or
\"guile\" for a serve.scm). `ray --tcp` serves scm on 4007 (the default),
`scm --serve HOST:PORT` anywhere, lux `--serve`. Its image is the one a
send changes, and the one completion and describe read.

It asks the server for its events: lux's errors in a frame, which come
after the send that caused them was answered, show in the transcript and
the error pane (C-h e) as a failed send does."
  (unless (%remote-refused)
    (echo "connecting to " addr "…")
    (stop-backends)
    (set! *remote* (%remote-connect addr))
    (%remote-subscribe *remote*)
    (remote-took! kind)
    *remote*))

(define (buffer-scheme)
  ; The *schemes* name for this buffer's language: what remote-start offers
  ; first.
  (let ((lang (%buffer-language (editor-pane-id) (%editor-active-buffer (editor-pane-id)))))
    (or (any (lambda (s) (and (equal? (cadddr s) lang) (car s))) *schemes*) "scm")))

(define (remote-start (which #f))
  "Start a Scheme in a process of its own as THE backend — SLIME's M-x
slime — which every Scheme file sends to until M-x remote-stop; the one
running stops first. Asks WHICH — scm, lux or guile (*schemes*), the
buffer's own first — and the buffer on screen takes that Scheme's
language. It asks the server for its events, as remote-connect does: lux's
pauses. A loop that never ends is interrupted after *remote-timeout*
seconds; a crash takes the server, not the editor. (remote-start
\"guile\") asks nothing; scm-start, lux-start and guile-start are each one."
  (define (go name)
    (match (assoc name *schemes*)
      ((name program args language)
        (stop-backends)
        (set! *remote* (%remote-start (program) 0 (args)))
        (%remote-subscribe *remote*)
        (remote-took! name)
        *remote*)
      (_ (echo "no Scheme called " name " — " (string-join (map car *schemes*) ", ")) #f)))
  (cond
    ((%remote-refused) #f)
    (which (go which))
    (else
      (read-minibuffer "Start which Scheme: "
                       (lambda (s) (go (string-trim-both s)))
                       (map car *schemes*)
                       (buffer-scheme))
      #f)))

(define (scm-start)
  "Start scm in a process of its own — `scm --serve` — as the backend every
Scheme file sends to (remote-start \"scm\"). M-x remote-stop stops it."
  (remote-start "scm"))

(define (lux-start)
  "Run this buffer's file, as saved, in lux — a window of its own; with no
file, nothing, until something is sent to draw — as the backend every
Scheme file sends to: C-c C-c redefines in the running program. An error
in the file pauses lux rather than ending it; send the fix. M-x
remote-stop closes it. (remote-start \"lux\")."
  (remote-start "lux"))

(define (remote-stop)
  "Stop the backend — the Scheme remote-start began, or drop the one
remote-connect attached. The Scheme files send to the editor's image again."
  (if (not *remote*)
    (echo "no Scheme running in a process of its own")
    (begin
      (%remote-close *remote*)
      (set! *remote* #f)
      (status-refresh)
      (echo *remote-kind* " stopped — the Scheme files send to the editor's image"))))

(define (remote-interrupt)
  "Stop what the buffer's other process is evaluating — a send that runs
  away. Its answer comes back marked (stopped); the process lives on."
  (let ((b (backend-of (%editor-active-buffer (editor-pane-id)))))
    (cond ((and b (%alist-ref b :interrupt #f)) => (lambda (interrupt) (interrupt)))   ; (evcxr's, 26-rust-repl)
          ((and b (%alist-ref b :conn #f)) => (lambda (conn) (%remote-interrupt conn) (echo "interrupted")))
          (*remote* (%remote-interrupt *remote*) (echo "interrupted"))
          (else (echo "nothing running elsewhere to interrupt")))))
