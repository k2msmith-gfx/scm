;## 14-remote.scm — a SOCKET backend: SLIME's model, over scm's own REPL
;## protocol (docs/editor-crate-plan.md, E5b). The terminal (13-term) is
;## inferior-lisp: a send rewrites the running program, and nothing
;## structured comes back. Here `scm --serve` is the SWANK half, and a send
;## answers as one in this image does — a value, what the form printed, an
;## error with the calls it was in — from ANOTHER process's image. A runaway
;## loop or a crash stays in that process. The client is edit's
;## scm_binding/remote.rs.

(define *remote* #f)              ; the current connection's id, or #f
(define *remote-kind* "scm")      ; which Scheme it is — a *schemes* name
(define *remote-timeout* 30)      ; seconds a send waits before interrupting
(define *remote-program* "scm")   ; what remote-start runs for scm, with --serve

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

(define (remote-result r)
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
            (cons :frames (let ((calls (%alist-ref r :backtrace '()))
                                (locals (%alist-ref r :locals '())))
                            (map remote-frame calls
                                 (if (= (length locals) (length calls)) locals (map (lambda (c) '()) calls)))))))
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
          (note-send-error! at "" message (map remote-frame (%alist-ref ev :backtrace '())))))
      ((resumed)
        (*transcript-append* "resumed" :dim)
        (echo "resumed"))
      (else #f))))

(define (remote-poll)
  "Deliver every answer an async send is owed: call each send's callback
with its RESULT, then show each event a server pushed (remote-event), then
run *wake-hooks*. The host calls this when a connection wakes it."
  (for-each (lambda (h) (h)) *wake-hooks*)
  (for-each
    (lambda (row)
      (match row
        ((conn rid r)
          (let ((k (hash-table-ref/default *remote-callbacks* (cons conn rid) #f)))
            (hash-table-delete! *remote-callbacks* (cons conn rid))
            (when k (k (remote-result r)))))))
    (%remote-poll))
  ; After the answers: a send's ok is shown before the pause it caused.
  (for-each (lambda (row) (remote-event (car row) (cadr row))) (%remote-events))
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
                  (remote-result (%remote-eval conn src #f *remote-timeout*)))))
        (cons :eval-async
              (lambda (src k)
                (guard (e (#t (k (failed-result e))))
                  (hash-table-set! *remote-callbacks* (cons conn (%remote-eval-async conn src #f)) k))))
        (cons :complete (lambda (prefix) (if (idle?) (guard (e (#t '())) (%remote-complete conn prefix)) '())))
        (cons :doc (lambda (name) (let ((d (describe name))) (and d (%alist-ref d :doc #f)))))
        (cons :signature (lambda (name) (let ((d (describe name))) (and d (%alist-ref d :signature #f)))))
        (cons :definition-sites (lambda (name) (definition-sites name)))))

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

(define (remote-attach)
  "Send this buffer's forms to the connected Scheme from now on — C-M-x,
  C-x C-e and C-c C-b evaluate there; completion and describe ask it, and
  the buffer takes that Scheme's language. remote-detach sends them back."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (if *remote*
      (begin (hash-table-set! *attached-backends* buf (remote-backend *remote* *remote-kind*))
             (match (assoc *remote-kind* *schemes*)
               ((name program args language) (%editor-set-language (editor-pane-id) language))
               (_ #f))
             (echo (%text (buffer-name buf)) " sends to " *remote-kind* " at " (%remote-address *remote*)))
      (echo "not connected — M-x remote-start, or remote-connect"))))

(define (remote-detach)
  "Send this buffer's forms to the host's backend again."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (hash-table-delete! *attached-backends* buf)
    (echo (%text (buffer-name buf)) " sends to this image")))

(define (remote-connect (addr "127.0.0.1:4007") (kind "scm"))
  "(remote-connect [\"HOST:PORT\" [KIND]])  ->  the connection

Attach to a REPL server already running — SLIME's slime-connect — and
point this buffer's sends at it. KIND names the Scheme it is (\"scm\", the
default, or \"guile\" for a serve.scm). `ray --tcp` serves scm on 4007 (the
default), `scm --serve HOST:PORT` anywhere, lux `--serve`. Its image is the
one a send changes, and the one completion and describe read.

It asks the server for its events: lux's errors in a frame, which come
after the send that caused them was answered, show in the transcript and
the error pane (C-h e) as a failed send does."
  (unless (%remote-refused)
    (when *remote* (%remote-close *remote*))
    (set! *remote* (%remote-connect addr))
    (%remote-subscribe *remote*)
    (set! *remote-kind* kind)
    (remote-attach)
    *remote*))

(define (buffer-scheme)
  ; The *schemes* name for this buffer's language: what remote-start offers
  ; first.
  (let ((lang (%buffer-language (editor-pane-id) (%editor-active-buffer (editor-pane-id)))))
    (or (any (lambda (s) (and (equal? (cadddr s) lang) (car s))) *schemes*) "scm")))

(define (remote-start (which #f))
  "Start a Scheme in a process of its own and point this buffer's sends at
it — SLIME's M-x slime. Asks WHICH — scm or guile (*schemes*), the buffer's
own first — and the buffer takes that Scheme's language. A loop that never
ends is interrupted after *remote-timeout* seconds; a crash takes the
server, not the editor. M-x remote-stop ends it; (remote-start \"guile\")
asks nothing."
  (define (go name)
    (match (assoc name *schemes*)
      ((name program args language)
        (when *remote* (%remote-close *remote*))
        (set! *remote* (%remote-start (program) 0 (args)))
        (set! *remote-kind* name)
        (remote-attach)
        (echo name " running at " (%remote-address *remote*) " — this buffer sends there")
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

(define (remote-stop)
  "Drop the connection, stopping the server if remote-start began it. The
  buffers that sent to it send where their language does again."
  (when *remote*
    (for-each (lambda (buf)
                (when (equal? (%alist-ref (hash-table-ref *attached-backends* buf) :conn #f) *remote*)
                  (hash-table-delete! *attached-backends* buf)))
              (hash-table-keys *attached-backends*))
    (%remote-close *remote*)
    (set! *remote* #f)
    (echo "disconnected")))

(define (remote-interrupt)
  "Stop what the buffer's other process is evaluating — a send that runs
  away. Its answer comes back marked (stopped); the process lives on."
  (let ((b (backend-of (%editor-active-buffer (editor-pane-id)))))
    (cond ((and b (%alist-ref b :interrupt #f)) => (lambda (interrupt) (interrupt)))   ; (evcxr's, 26-rust-repl)
          ((and b (%alist-ref b :conn #f)) => (lambda (conn) (%remote-interrupt conn) (echo "interrupted")))
          (*remote* (%remote-interrupt *remote*) (echo "interrupted"))
          (else (echo "nothing running elsewhere to interrupt")))))
