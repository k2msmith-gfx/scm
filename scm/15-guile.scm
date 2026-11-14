;## 15-guile.scm — GNU Guile, the editor's second language
;## (docs/editor-crate-plan.md, E6). A Guile buffer highlights and indents
;## as Scheme (edit's language/guile.rs), and its sends go to a Guile the
;## editor starts — not to this image, which is scm's. That Guile runs
;## edit/guile/serve.scm, which speaks scm's REPL protocol, so the socket
;## backend (14-remote) is its backend unchanged: C-M-x answers `=> value`,
;## an error its message, and completion and describe read the live Guile.

(define *guile-program* "guile")   ; what guile-start runs
(define *guile* #f)                ; the connection to it, once started

(define (guile-args)
  ; serve.scm, uncompiled (no cache to go stale), on the address.
  (list "--no-auto-compile" "-s" (%edit-path "guile/serve.scm") "{addr}"))

; remote-start can start one too (14-remote's *schemes*).
(define-scheme! "guile" (lambda () *guile-program*) guile-args "guile")

(define (guile-start)
  "Start a Guile for the Guile buffers' sends, or answer the one running —
M-x guile-start. The first send from a Guile buffer starts it; this is for
starting it before then."
  (unless (and *guile* (%remote-address *guile*))
    (set! *guile* (%remote-start *guile-program* 0 (guile-args)))
    (echo "guile running at " (%remote-address *guile*)))
  *guile*)

(define (guile-stop)
  "Stop the Guile the editor started."
  (when *guile*
    (%remote-close *guile*)
    (set! *guile* #f)
    (echo "guile stopped")))

; A Guile buffer's backend: the Guile, started on its first send. When
; Guile cannot start, the send FAILS, saying why — it must not fall through
; to this image, where Guile code would run as scm's.
(hash-table-set! *language-backends* "guile"
  (lambda ()
    (guard (e (#t (let ((why (if (string? e) e (*show-value* e))))
                    (list (cons :name "guile (not running)")
                          (cons :eval (lambda (src)
                                        (list (cons :status 'error)
                                              (cons :message (string-append "guile: " why))
                                              (cons :output ""))))))))
      (remote-backend (guile-start) "guile"))))

(define (guile-mode)
  "Treat this buffer as GNU Guile — M-x guile-mode: Guile's keywords
highlight, scm's read marks go, and its sends evaluate in a Guile the
editor starts. A file that says it is Guile (a #!…guile line, a
define-module or use-modules form, a .guile name) is set so when opened."
  (%editor-set-language (editor-pane-id) "guile")
  (echo "guile mode — sends go to Guile"))

(define (scheme-mode)
  "Treat this buffer as scm's Scheme again — M-x scheme-mode."
  (%editor-set-language (editor-pane-id) "scheme")
  (echo "scheme mode — sends go to this image"))

(define (guile-text? path text)
  "Does the file at PATH, holding TEXT, say it is Guile?"
  (or (and path (string-suffix? ".guile" path))
      (and (string-prefix? "#!" text)
           (let ((nl (or (string-contains text "\n") (string-length text))))
             (string-contains (substring text 0 nl) "guile")))
      (string-contains text "(define-module ")
      (string-contains text "(use-modules ")))

(define (detect-language! eid path)
  "Set the active buffer of editor EID to the language its file says it is."
  (when (guile-text? path (%editor-text eid))
    (%editor-set-language eid "guile")))
