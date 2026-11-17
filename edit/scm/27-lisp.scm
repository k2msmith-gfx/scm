;## 27-lisp.scm — Common Lisp, through SBCL (docs/sbcl-plan.md, L3). A Lisp
;## buffer highlights, indents and edits by expression as Common Lisp
;## (edit's language/lisp.rs), and its sends go to an SBCL the editor starts
;## — not to this image, which is scm's. That SBCL runs edit/sbcl/serve.lisp,
;## which speaks scm's REPL protocol, so the socket backend (14-remote) is
;## its backend: C-M-x answers `=> F`, an error its message and calls, and
;## completion and describe read the live SBCL.
;##
;## PACKAGES are the buffer's, as SLIME's are: a send is read and evaluated
;## in the package of the nearest (in-package …) above it — found in the
;## text here (%lisp-package), and handed to the server's edit-serve:in. The
;## REPL panel's lines are evaluated in the REPL's own package, which an
;## (in-package …) typed there changes, and its prompt says which.

(define *sbcl-program* "sbcl")   ; what sbcl-start runs
(define *sbcl* #f)               ; the connection to it, once started
(define *sbcl-init* #t)          ; load ~/.sbclrc (Quicklisp and the rest)? (D1)
(define *sbcl-repl-package* "CL-USER")

(define (sbcl-args)
  ; serve.lisp, loaded as source, on the address. --disable-debugger: an
  ; init file that breaks exits rather than waiting at a debugger prompt
  ; nobody can see.
  (append (list "--noinform" "--disable-debugger")
          (if *sbcl-init* '() (list "--no-userinit"))
          (list "--load" (%edit-path "sbcl/serve.lisp") "--end-toplevel-options" "{addr}")))

; remote-start can start one too (14-remote's *schemes*).
(define-scheme! "sbcl" (lambda () *sbcl-program*) sbcl-args "lisp")

(define (sbcl-running?) (and *sbcl* (%remote-address *sbcl*) #t))

(define (sbcl-start)
  "Start an SBCL for the Lisp buffers' sends, or answer the one running —
M-x sbcl-start. The first send from a Lisp buffer starts it; this is for
starting it before then."
  (unless (sbcl-running?)
    (set! *sbcl* (%remote-start *sbcl-program* 0 (sbcl-args)))
    (set! *sbcl-repl-package* "CL-USER")
    (echo "sbcl running at " (%remote-address *sbcl*)))
  *sbcl*)

(define (sbcl-stop)
  "Stop the SBCL the editor started."
  (when *sbcl*
    (%remote-close *sbcl*)
    (set! *sbcl* #f)
    (echo "sbcl stopped")))

(define (sbcl-restart)
  "Stop the SBCL and start a fresh one — everything sent to it is gone."
  (sbcl-stop)
  (sbcl-start))

; ── The buffer's package ───────────────────────────────────────────────────
(define (lisp-buffer-package)
  ; The package a send from the point is read in: the nearest
  ; (in-package …) above it, else CL-USER.
  (let ((at (%editor-cursor (editor-pane-id))))
    (or (%lisp-package (%editor-text (editor-pane-id)) (car at) (cadr at)) "COMMON-LISP-USER")))

(define (lisp-string s)
  ; S as a Common Lisp string literal: only \ and " are escaped there.
  (string-append "\""
                 (apply string-append
                        (map (lambda (c) (case c ((#\\) "\\\\") ((#\") "\\\"") (else (string c))))
                             (string->list s)))
                 "\""))

(define (lisp-call fn . args)
  ; "(edit-serve:FN \"arg\" …)" — a call into the server's own package.
  (string-append "(edit-serve:" fn
                 (apply string-append (map (lambda (a) (string-append " " (lisp-string a))) args))
                 ")"))

; A send from a buffer runs in that buffer's package; a whole buffer or a
; file starts where its text starts, in CL-USER, until its own in-package.
(hash-table-set! *language-send-wrap* "lisp"
  (lambda (run label)
    (lisp-call "in" (if (member label '("buffer" "file")) "COMMON-LISP-USER" (lisp-buffer-package)) run)))

(define (lisp-ask conn call)
  ; CALL's value, READ — the server's helpers answer lists of strings —
  ; or #f when SBCL is busy, will not say, or says something unreadable.
  (and (zero? (%remote-running conn))
       (guard (e (#t #f))
         (let ((r (%remote-eval conn call #f 5)))
           (and (eq? (%alist-ref r :status #f) 'ok)
                (read (open-input-string (%alist-ref r :value "NIL"))))))))

(define (lisp-said x)
  ; A Lisp answer as the shell takes it: NIL and "" are nothing.
  (and x (not (eq? x 'NIL)) (not (equal? x "")) x))

(define (refresh-repl-package! conn)
  ; Ask which package the REPL is in now, and redraw its prompt.
  (hash-table-set! *remote-callbacks*
    (cons conn (%remote-eval-async conn "(package-name *package*)" #f))
    (lambda (r)
      (when (eq? (%alist-ref r :status #f) 'ok)
        (let ((name (guard (e (#t #f)) (read (open-input-string (%alist-ref r :printed ""))))))
          (when (string? name)
            (set! *sbcl-repl-package* name)
            ; (The standalone's REPL panel; a host without one has no
            ; prompt to redraw.)
            (guard (e (#t #f))
              (when (%editing-pane? 2) (%edit! :prompt (repl-prompt))))))))))

(define (lisp-backend conn)
  "A Lisp buffer's backend at the SBCL on CONN: the socket backend, with
completion and describe asked in the buffer's package, the REPL's prompt
its package's, and a REPL line whole when its brackets close."
  (let ((base (remote-backend conn "sbcl")))
    (append
      (list (cons :eval-async
                  (lambda (src k)
                    ((backend-op base :eval-async)
                     src
                     (lambda (r)
                       (k r)
                       ; A REPL line may have changed the REPL's package.
                       (when (and (string-contains src "in-package")
                                  (not (string-prefix? "(edit-serve:in " src)))
                         (refresh-repl-package! conn))))))
            (cons :complete
                  (lambda (prefix)
                    (let ((names (lisp-ask conn (lisp-call "complete-in" (lisp-buffer-package) prefix))))
                      (if (list? names) (filter string? names) '()))))
            (cons :doc
                  (lambda (name)
                    (let ((d (lisp-ask conn (lisp-call "describe-in" (lisp-buffer-package) name))))
                      (and (pair? d) (lisp-said (caddr d))))))
            (cons :signature
                  (lambda (name)
                    (let ((d (lisp-ask conn (lisp-call "describe-in" (lisp-buffer-package) name))))
                      (and (pair? d) (lisp-said (cadr d))))))
            (cons :prompt (lambda () (string-append *sbcl-repl-package* "> ")))
            (cons :whole? %lisp-complete?))
      (filter (lambda (op) (not (memq (car op) '(:eval-async :complete :doc :signature)))) base))))

; A Lisp buffer's backend: the SBCL, started on its first send. When SBCL
; cannot start, the send FAILS, saying why — it must not fall through to
; this image, where Common Lisp would be read as Scheme.
(hash-table-set! *language-backends* "lisp"
  (lambda ()
    (guard (e (#t (unavailable-backend "sbcl" e)))
      (lisp-backend (sbcl-start)))))

; Completion, the argument hint and the REPL's TAB ask the SBCL that is
; running, and start none to ask. With none running a Lisp buffer offers
; its own words and no signatures: this image's names are scm's, and mean
; nothing in Common Lisp.
(hash-table-set! *language-running* "lisp"
  (lambda ()
    (if (sbcl-running?)
        (lisp-backend *sbcl*)
        (list (cons :name "sbcl (not running)")
              (cons :complete (lambda (prefix) (buffer-words (%editor-text (editor-pane-id)) "-*+!?<>=/%&:")))
              (cons :doc (lambda (name) #f))
              (cons :signature (lambda (name) #f))))))

; What a send reads from a Lisp buffer: the top-level form at the point and
; the expression before it, off Common Lisp's own tokens — `|a (b|` is one
; symbol, and `#+sbcl (f)` one form.
(define-language-forms! "lisp"
  (lambda (text)
    (let ((at (%editor-cursor (editor-pane-id))))
      (%lisp-form-at text (car at) (cadr at))))
  (lambda (text)
    (let ((at (%editor-cursor (editor-pane-id))))
      (%lisp-form-before text (car at) (cadr at)))))

(define (lisp-mode)
  "Treat this buffer as Common Lisp — M-x lisp-mode: Common Lisp's
highlighting and indentation, and sends evaluate in an SBCL the editor
starts. A .lisp, .lsp, .cl or .asd file is set so when opened."
  (%editor-set-language (editor-pane-id) "lisp")
  (echo "lisp mode — sends go to SBCL"))

(define (lisp-text? path text)
  "Is the file at PATH Common Lisp, by its name?"
  (and path (any (lambda (ext) (string-suffix? ext path)) '(".lisp" ".lsp" ".cl" ".asd"))))

(add-language-detector! "lisp" lisp-text?)
