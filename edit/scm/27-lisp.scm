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
;##
;## AN ERROR IS A BREAK (L7), as at SBCL's own prompt: the send answers its
;## message, and the restarts are listed under it; the connection is then
;## inside the error, a level deeper (the prompt says `CL-USER[1]> `), and
;## what the REPL — or a buffer — sends evaluates there. A restart's number
;## takes it (one that wants a value asks for the value's form next, or
;## takes it on the line: `1 42`), `:abort` gives the innermost send up and
;## `:top` all of them. C-h e shows the calls with their locals.

(define *sbcl-program* "sbcl")   ; what sbcl-start runs
(define *sbcl* #f)               ; the connection to it, once started
(define *sbcl-init* #t)          ; load ~/.sbclrc (Quicklisp and the rest)? (D1)
(define *sbcl-repl-package* "CL-USER")
(define *sbcl-level* 0)          ; how many breaks deep the connection is
(define *sbcl-asking* #f)        ; the restart waiting for its value's form

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
    (set! *sbcl-level* 0)
    (set! *sbcl-asking* #f)
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
    ; Which file's forms these are, for the warnings said of them (L6).
    (let ((path (buffer-path (%editor-active-buffer (editor-pane-id)))))
      (when path (set! *lisp-sends* (append *lisp-sends* (list (cons path run))))))
    (lisp-call "in" (if (member label '("buffer" "file")) "COMMON-LISP-USER" (lisp-buffer-package)) run)))

; ── The compiler's warnings, marked at their forms (L6) ────────────────────
; What SBCL's compiler says of a form as it is sent — an undefined
; function, a wrong count of arguments, an unused variable — comes back
; with the answer (the transcript shows it under the send), each warning
; with where its form began in the text sent. %lisp-warned marks the
; buffer at those forms' heads, in the warning ink, and every form sent
; loses its old marks: a mark stays until its form is sent again clean,
; following its defun as the text around it moves. M-g n goes to them.
(define *lisp-sends* '())        ; ((path . run) …): sends not yet answered, oldest first

(define (lisp-warned! sent r)
  (%lisp-warned (car sent) (cdr sent) (%alist-ref r :warning-at '()) (%alist-ref r :warnings '())))

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

(define (sbcl-prompt)
  ; CL-USER> at the top, CL-USER[2]> two breaks down, and the restart's
  ; name while it waits for a value: `USE-VALUE value> `.
  (cond (*sbcl-asking*
          (let ((close (string-contains *sbcl-asking* "]")))
            (string-append (if close (substring *sbcl-asking* 1 close) "restart") " value> ")))
        ((zero? *sbcl-level*) (string-append *sbcl-repl-package* "> "))
        (else (string-append *sbcl-repl-package* "[" (number->string *sbcl-level*) "]> "))))

(define (sbcl-heard! r)
  ; An eval's answer says where the connection is now: its break level,
  ; the restart waiting for a value, the package the REPL reads in. A break
  ; it entered lists its restarts, numbered, under the error.
  (set! *sbcl-level* (%alist-ref r :level 0))
  (set! *sbcl-asking* (%alist-ref r :asking #f))
  (set! *sbcl-repl-package* (%alist-ref r :package *sbcl-repl-package*))
  (let ((restarts (%alist-ref r :restarts '())))
    (unless (null? restarts)
      (let loop ((rs restarts) (i 0))
        (unless (null? rs)
          (*transcript-append* (string-append "  " (number->string i) ": " (car rs)) :dim)
          (loop (cdr rs) (+ i 1))))
      (*transcript-append*
        (string-append "  break " (number->string *sbcl-level*)
                       ": a restart's number takes it, :abort gives the send up, anything else evaluates in the error")
        :dim)
      (echo "error: " (%alist-ref r :message "") "   — break " (number->string *sbcl-level*)
            ": a restart's number at the REPL (C-c C-z), or :abort")))
  ; (The standalone's REPL panel; a host without one has no prompt to
  ; redraw.)
  (guard (e (#t #f))
    (when (%editing-pane? 2) (%edit! :prompt (repl-prompt)))))

(define (lisp-backend conn)
  "A Lisp buffer's backend at the SBCL on CONN: the socket backend, with
completion and describe asked in the buffer's package, the REPL's prompt
its package's and its break level, a break's restarts listed under its
error, and a REPL line whole when its brackets close."
  (let ((base (remote-backend conn "sbcl")))
    (append
      (list (cons :eval-async
                  (lambda (src k)
                    ; A buffer's send (wrapped above): its entry, and any older
                    ; one that never reached here (a send that failed first).
                    (let ((sent (and (string-prefix? "(edit-serve:in " src)
                                     (let loop ()
                                       (match *lisp-sends*
                                         (() #f)
                                         ((s . rest)
                                           (set! *lisp-sends* rest)
                                           (if (string-contains src (lisp-string (cdr s))) s (loop))))))))
                      ((backend-op base :eval-async)
                       src
                       (lambda (r) (k r) (sbcl-heard! r) (when sent (lisp-warned! sent r)))))))
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
            (cons :prompt sbcl-prompt)
            (cons :whole? %lisp-complete?))
      (filter (lambda (op) (not (memq (car op) '(:eval-async :complete :doc :signature)))) base))))

; A Lisp buffer's backend: the SBCL, started on its first send. When SBCL
; cannot start, the send FAILS, saying why — it must not fall through to
; this image, where Common Lisp would be read as Scheme.
(hash-table-set! *language-backends* "lisp"
  (lambda ()
    (define (started)
      (guard (e (#t (unavailable-backend "sbcl" e)))
        (lisp-backend (sbcl-start))))
    (if (sbcl-running?) (started) (on-first-send "sbcl" started))))

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

; ── SBCL's reader reads Lisp buffers (L4) ──────────────────────────────────
; The kernel marks a Lisp buffer's brackets itself (language/lisp.rs); what
; else SBCL's reader cannot read — `#\notachar`, a package that does not
; exist, a string never closed — is asked of the SBCL that is running, as
; 15-guile asks Guile: the kernel keeps each Lisp file's newest text
; (%read-pending), this puts it to edit-serve:read-check when SBCL is idle,
; and the answer comes back a mark (%read-result). It reads each form in the
; package of the in-package before it, evaluates nothing, and leaves no
; symbol behind. None is started to ask; one question at a time; none while
; a restart waits for its value, which would take the question for it.
(define *sbcl-read-check* #t)    ; #f: brackets only
(define %sbcl-reading #f)        ; a question is out

(define (sbcl-read-poll)
  ; Put the next changed Lisp buffer to SBCL, when SBCL can take it.
  (when (and *sbcl-read-check* (not %sbcl-reading) (not *sbcl-asking*)
             (sbcl-running?) (zero? (%remote-running *sbcl*)))
    (and-let* ((next (%read-pending "lisp")))
      (let ((path (car next)) (version (cadr next)) (text (caddr next)))
        (guard (e (#t (set! %sbcl-reading #f)))
          (set! %sbcl-reading #t)
          (hash-table-set! *remote-callbacks*
            (cons *sbcl* (%remote-eval-async *sbcl* (lisp-call "read-check" text) #f))
            (lambda (r)
              (set! %sbcl-reading #f)
              (let ((said (and (eq? (%alist-ref r :status #f) 'ok)
                               (guard (e (#t #f)) (read (open-input-string (%alist-ref r :value "T")))))))
                (if (and (pair? said) (= (length said) 3))
                    (%read-result path version (car said) (cadr said) (caddr said))
                    (%read-result path version #f #f "")))
              ; The next one waiting, or this one again if it has changed.
              (sbcl-read-poll))))))))

(%read-watch "lisp")
(set! *wake-hooks* (cons sbcl-read-poll *wake-hooks*))

;; ── M-. (L5) ──────────────────────────────────────────────────────────────
;; Where a name is defined, from two places: the project's Lisp files as
;; they stand (%lisp-definitions — every `def…` form naming it, under the
;; nearest directory above with a .asd or .git; the buffer on screen as it
;; is, saved or not), which knows code SBCL was SENT, since SBCL keeps no
;; file for a form evaluated from a string; and, when SBCL is running and
;; idle, SBCL itself (sb-introspect), which knows what was LOADED from a
;; file — a Quicklisp library, SBCL's own source. The project's come first;
;; one SBCL names on the same line is not said twice. M-, comes back.
(define (lisp-definition-sites name)
  (let* ((eid (editor-pane-id))
         (path (real-file (buffer-path (%editor-active-buffer eid))))
         (here (if path
                   (%lisp-definitions path name (list (list path (%text (%editor-text eid)))))
                   '()))
         (loaded (if (sbcl-running?)
                     (let ((s (lisp-ask *sbcl* (lisp-call "definitions-in" (lisp-buffer-package) name))))
                       (if (list? s) (filter (lambda (site) (and (list? site) (= (length site) 4))) s) '()))
                     '())))
    (append here
            (filter (lambda (site)
                      (not (any (lambda (h) (and (equal? (car h) (car site)) (= (cadr h) (cadr site)))) here)))
                    loaded))))

(hash-table-set! *language-definitions* "lisp" lisp-definition-sites)

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
