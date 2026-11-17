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
    (guard (e (#t (unavailable-backend "guile" e)))
      (remote-backend (guile-start) "guile"))))

; Completion, the argument hint and the REPL's TAB ask the Guile that is
; running, and start none to ask (07-backend's running-image-backend): a
; Guile buffer's names are Guile's, not this image's (found 2026-11-17 —
; they had been scm's, or the separate scm's for a Guile FILE).
(hash-table-set! *language-running* "guile"
  (lambda () (and *guile* (%remote-address *guile*) (remote-backend *guile* "guile"))))

; ── Guile's reader reads Guile's buffers ───────────────────────────────────
; (Kevin, 2026-11-17.) scm's reader marks a Scheme buffer as you type; it
; cannot mark a Guile one — it rejects `#:key` — so the kernel marks only
; the brackets there (language/guile.rs). What Guile itself cannot read —
; `#\notachar`, a bad `#` form, a string never closed — is asked of the
; GUILE THAT IS RUNNING, with its reader as it is now, read-hash-extend and
; all: the kernel keeps each Guile file's newest text (%read-pending), this
; puts it to Guile when Guile is idle, and the answer comes back a mark
; (%read-result). One question at a time; none is started to ask; a text
; that changed while it was being read is asked about again.
(define *guile-read-check* #t)   ; #f: brackets only
(define %guile-reading #f)       ; a question is out

; Read TEXT to its end, in Guile: #t, or (line column message) where its
; reader stopped. Sent with each question, so it needs nothing of the
; serve.scm the Guile was started with.
(define %guile-read-form
  "((lambda (text)
      (let ((p (open-input-string text)))
        (catch #t
          (lambda () (let loop () (if (eof-object? (read p)) #t (loop))))
          (lambda (key . args)
            (list (port-line p) (port-column p)
                  (catch #t
                    (lambda () (apply simple-format #f (cadr args) (caddr args)))
                    (lambda _ (symbol->string key))))))))
    ")

(define (%guile-read-message text)
  ; Guile's own words, without the "#<unknown port>:3:9: " it puts in front.
  (let ((at (string-contains text ": ")))
    (if (and at (string-prefix? "#<" text)) (substring text (+ at 2) (string-length text)) text)))

(define (guile-read-poll)
  ; Put the next changed Guile buffer to Guile, when Guile can take it.
  (when (and *guile-read-check* (not %guile-reading)
             *guile* (%remote-address *guile*) (zero? (%remote-running *guile*)))
    (and-let* ((next (%read-pending "guile")))
      (let ((path (car next)) (version (cadr next)) (text (caddr next)))
        (guard (e (#t (set! %guile-reading #f)))
          (set! %guile-reading #t)
          (hash-table-set! *remote-callbacks*
            (cons *guile* (%remote-eval-async *guile* (string-append %guile-read-form (*show-value* text) ")") #f))
            (lambda (r)
              (set! %guile-reading #f)
              (let ((said (and (eq? (%alist-ref r :status #f) 'ok)
                               (guard (e (#t #f)) (read (open-input-string (%alist-ref r :printed "#t")))))))
                (if (and (pair? said) (= (length said) 3))
                    (%read-result path version (car said) (cadr said) (%guile-read-message (caddr said)))
                    (%read-result path version #f #f "")))
              ; The next one waiting, or this one again if it has changed.
              (guile-read-poll))))))))

(%read-watch "guile")
(set! *wake-hooks* (cons guile-read-poll *wake-hooks*))

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

(add-language-detector! "guile" guile-text?)
