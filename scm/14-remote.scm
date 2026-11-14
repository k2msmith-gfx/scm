;## 14-remote.scm — a SOCKET backend: SLIME's model, over scm's own REPL
;## protocol (docs/editor-crate-plan.md, E5b). The terminal (13-term) is
;## inferior-lisp: a send rewrites the running program, and nothing
;## structured comes back. Here `scm --serve` is the SWANK half, and a send
;## answers as one in this image does — a value, what the form printed, an
;## error with the calls it was in — from ANOTHER process's image. A runaway
;## loop or a crash stays in that process. The client is edit's
;## scm_binding/remote.rs.

(define *remote* #f)              ; the current connection's id, or #f
(define *remote-timeout* 30)      ; seconds a send waits before interrupting
(define *remote-program* "scm")   ; what remote-start runs with --serve

(define (remote-backend conn (label "scm"))
  "A backend that asks the image at connection CONN — scm's, or anything
that speaks its protocol (Guile's serve.scm, 15-guile): sends evaluate
there, completion and docs answer from its live bindings, and M-. stays
with this editor's index of the files. LABEL names it."
  (define (describe name) (guard (e (#t #f)) (%remote-describe conn name)))
  (list (cons :name (string-append label " at " (or (%remote-address conn) "?")))
        (cons :eval
              (lambda (src)
                (guard (e (#t (list (cons :status 'error)
                                    (cons :message (if (string? e) e (*show-value* e)))
                                    (cons :output ""))))
                  (let ((r (%remote-eval conn src #f *remote-timeout*)))
                    (if (eq? (%alist-ref r :status #f) 'ok)
                      ; :value is the printed value — the live one is in
                      ; the other process; truthy, so the transcript shows it.
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
                            (cons :frames (map remote-frame (%alist-ref r :backtrace '())))))))))
        (cons :complete (lambda (prefix) (guard (e (#t '())) (%remote-complete conn prefix))))
        (cons :doc (lambda (name) (let ((d (describe name))) (and d (%alist-ref d :doc #f)))))
        (cons :signature (lambda (name) (let ((d (describe name))) (and d (%alist-ref d :signature #f)))))
        (cons :definition-sites (lambda (name) (definition-sites name)))))

(define (remote-frame line)
  ; "(form)  at src:line:col" as an error-pane frame: (call location locals).
  ; A wire carries the calls, not the stack, so there are no locals.
  (let ((at (string-contains line "  at ")))
    (if at
      (list (substring line 0 at) (substring line (+ at 5) (string-length line)) '())
      (list line #f '()))))

(define (remote-attach)
  "Send this buffer's forms to the connected scm image from now on —
  C-M-x, C-x C-e and C-c C-b evaluate there; completion and describe ask
  it. remote-detach sends them to this image again."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (if *remote*
      (begin (hash-table-set! *attached-backends* buf (remote-backend *remote*))
             (echo (%text (buffer-name buf)) " sends to scm at " (%remote-address *remote*)))
      (echo "not connected — M-x remote-start, or remote-connect"))))

(define (remote-detach)
  "Send this buffer's forms to the host's backend again."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (hash-table-delete! *attached-backends* buf)
    (echo (%text (buffer-name buf)) " sends to this image")))

(define (remote-connect (addr "127.0.0.1:4007"))
  "(remote-connect [\"HOST:PORT\"])  ->  the connection

Attach to an scm REPL server already running — SLIME's slime-connect —
and point this buffer's sends at it. `ray --tcp` serves one on 4007 (the
default), `scm --serve HOST:PORT` anywhere. Its image is the one a send
changes, and the one completion and describe read."
  (when *remote* (%remote-close *remote*))
  (set! *remote* (%remote-connect addr))
  (remote-attach)
  *remote*)

(define (remote-start)
  "Start a fresh `scm --serve` on a free port and point this buffer's sends
  at it — SLIME's M-x slime, the image in its own process. A loop that never
  ends is interrupted after *remote-timeout* seconds; a crash takes the
  server, not the editor. M-x remote-stop ends it."
  (when *remote* (%remote-close *remote*))
  (set! *remote* (%remote-start *remote-program* 0))
  (remote-attach)
  (echo "scm running at " (%remote-address *remote*) " — this buffer sends there")
  *remote*)

(define (remote-stop)
  "Drop the connection, stopping the server if remote-start began it."
  (when *remote*
    (%remote-close *remote*)
    (set! *remote* #f)
    (echo "disconnected")))

(define (remote-interrupt)
  "Stop what the connected image is evaluating."
  (when *remote* (%remote-interrupt *remote*)))
