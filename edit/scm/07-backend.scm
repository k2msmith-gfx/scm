;## 07-backend.scm — where a send goes: the BACKEND a buffer's language
;## answers through, and the transcript the answers land in
;## (docs/editor-crate-plan.md, E3e — "The interface between the editor and
;## Scheme"). The SLIME sends, completion, eldoc and M-. ask a backend rather
;## than calling the image themselves, so a second kind — a terminal (E5), a
;## socket to another scm (E5b) — is a second backend, not a second editor.

; Where a send's lines, values and errors go — and show-keys' listing — as a
; FORWARD HOOK: the modeler binds it to its REPL transcript.
(define *transcript-append* (lambda (text (style #f)) (display text) (newline)))

; A BACKEND is an alist of procedures, each one a question the editor asks
; a language; a missing one means "not offered", and the command that needs
; it says so:
;   :name              what the echo calls it ("this image")
;   :eval (src)        → a RESULT, an alist —
;                        ((:status . ok) (:value . v) (:printed . "…")
;                         (:output . "…") (:secs . n))
;                        or ((:status . error) (:message . "…") (:output . "…")
;                            (:frames . ((call location ((name printed) …)) …)))
;                        :frames, innermost first, where the backend can
;                        see them (in-process; the error pane shows them)
;                        :value is the live value where the backend has one
;                        (in-process); :printed is what to show either way
;   :complete (prefix) → candidate names
;   :doc (name)        → a docstring, or #f
;   :signature (name)  → a signature line, or #f
;   :definition-sites (name) → ((path line column form) …)
(define (backend-op backend key)
  "BACKEND's procedure for KEY, or #f when it offers none."
  (and backend (%alist-ref backend key #f)))

(define (in-process-eval src)
  "A send evaluated in THIS image — SRC's forms in order, at top level — as
a backend RESULT: what the forms printed is captured, and a form that
raises answers an error result rather than unwinding the editor, with the
frames it raised in (:frames — the error pane's, E3f)."
  ;; Through %call-with-break, so an error the sent code does not catch
  ;; itself is taken at the raise, frames and locals live (the snapshot is
  ;; edit's scm_binding/breaks.rs). The value stays in this closure: a
  ;; value crossing back through the prim would be copied.
  (let ((out (open-output-string))
        (val #f)
        (t0 (current-jiffy)))
    (set! %break-condition #f)
    (if (%call-with-break
          (lambda ()
            (set! val (parameterize ((current-output-port out)) (eval-string src)))
            #t))
      (list (cons :status 'ok)
            (cons :value val)
            (cons :printed (*show-value* val))
            (cons :output (get-output-string out))
            (cons :secs (/ (- (current-jiffy) t0) (jiffies-per-second))))
      (let ((e %break-condition)
            (brk (%last-break)))
        (set! %break-condition #f)
        (list (cons :status 'error)
              (cons :message (cond ((not e) (if brk (car brk) "error"))
                                   ((string? e) e)
                                   (else (*show-value* e))))
              (cons :output (get-output-string out))
              (cons :frames (if brk (cadr brk) '())))))))

(define (in-process-backend)
  "The backend that is this image: sends evaluate here, completion and docs
answer from its live bindings."
  (list (cons :name "this image")
        (cons :eval in-process-eval)
        (cons :complete
              (lambda (prefix)
                (filter (lambda (n) (string-prefix? prefix n))
                        (map symbol->string (apropos)))))
        (cons :doc %doc-of)
        (cons :signature (lambda (name) (signature-of name)))
        (cons :definition-sites (lambda (name) (definition-sites name)))))

; Which backend a BUFFER sends to, or #f for none — the host's choice, as a
; FORWARD HOOK (the modeler: this image, for a Scheme buffer). And what the
; host does after a send has run (the modeler syncs the scene, refreshes the
; outliner, nudges live render).
(define *buffer-backend* (lambda (buf) (in-process-backend)))

; A buffer may be pointed at ANOTHER backend than the host's choice — a
; terminal (13-term's term-attach). Every send, completion and describe asks
; backend-of, which honours that first.
(define *attached-backends* (make-hash-table equal?))

; A LANGUAGE may name its own backend (E6): a Guile buffer's sends go to a
; Guile, whatever the host would send a Scheme buffer to. Name → a thunk
; answering the backend (it may start one), consulted by the buffer's
; language (%buffer-language).
(define *language-backends* (make-hash-table equal?))

; Which language a file opened as text is: (language . test) pairs, asked
; in order, TEST taking the path and the text. A language adds its own
; (15-guile, 20-rust); a file none claims stays scm's Scheme.
(define *language-detectors* '())

(define (add-language-detector! language test)
  "Set a file opened as text to LANGUAGE when (TEST path text) is true."
  (set! *language-detectors* (append *language-detectors* (list (cons language test)))))

; Which DIALECT of Scheme a Scheme file is written for — ray's scenes, whose
; verbs only ray's image knows: (dialect . test) pairs as above, asked only
; of a file that reads as Scheme. The modeline gives it in brackets,
; "Scheme (ray)" (21-ray).
(define *dialect-detectors* '())

(define (add-dialect-detector! dialect test)
  "Mark a Scheme file opened as text as written for DIALECT when (TEST path
text) is true; the modeline says \"Scheme (DIALECT)\"."
  (set! *dialect-detectors* (append *dialect-detectors* (list (cons dialect test)))))

; Run with each buffer a file opens into, once its language is known: the
; warnings (22-warnings) judge it from then on.
(define *buffer-opened-hooks* '())

(define (detect-language! eid path)
  "Set the active buffer of editor EID to the language its file says it is,
and a Scheme file's dialect; then *buffer-opened-hooks*."
  (let ((text (%text (%editor-text eid)))
        (buf (%editor-active-buffer eid)))
    (when-let ((hit (find (lambda (d) ((cdr d) path text)) *language-detectors*)))
      (%editor-set-language eid (car hit)))
    (%buffer-set-dialect
      eid buf
      (or (and (equal? (%buffer-language eid buf) "scheme")
               (and-let* ((hit (find (lambda (d) ((cdr d) path text)) *dialect-detectors*))) (car hit)))
          ""))
    (for-each (lambda (h) (guard (e (#t #f)) (h buf))) *buffer-opened-hooks*)))

; Languages that are not Scheme at all (20-rust): this image's names, docs
; and signatures mean nothing in them, so completion offers the buffer's own
; words and eldoc stays quiet.
(define *non-scheme-languages* '())

(define (non-scheme-buffer? buf)
  "Is BUF in a language whose text is not Scheme?"
  (and (member (%buffer-language (*pane-id* :editor) buf) *non-scheme-languages*) #t))

(define (non-scheme-backend buf)
  "The backend of BUF's language when that language is not Scheme (Rust's,
20-rust), else #f. Completion, eldoc and M-. ask it; for a Scheme buffer
they answer from this image, and asking would start the Scheme a send
goes to."
  (and (non-scheme-buffer? buf)
       (let ((f (hash-table-ref/default *language-backends*
                                        (%buffer-language (*pane-id* :editor) buf) #f)))
         (and f (f)))))

(define (backend-of buf)
  "BUF's backend: the one it was attached to, else its language's, else
the host's choice."
  (or (hash-table-ref/default *attached-backends* buf #f)
      (let ((f (hash-table-ref/default *language-backends*
                                       (%buffer-language (*pane-id* :editor) buf) #f)))
        (and f (f)))
      (*buffer-backend* buf)))
; Which IMAGE a buffer's Scheme means, as a FORWARD HOOK: 'self (this
; image), a remote connection, or #f for none — found without starting
; anything. The warnings judge against it (22-warnings), and completion
; and eldoc answer from it (09-complete). The standalone says
; (host/standalone.scm): a ray file's ray, once it runs; a Scheme file's
; scm, once it runs, else this image, which has everything plain scm has.
; In the modeler every Scheme file runs here.
(define *buffer-image* (lambda (buf) (and (buffer-path buf) 'self)))

; A language whose buffers send to a process of the language's own (Guile,
; 15-guile) says here whether that process is RUNNING: language -> a thunk
; answering its backend, or #f when it is not — asked without starting it.
(define *language-running* (make-hash-table equal?))

(define (running-image-backend buf)
  "The backend of the other process BUF's Scheme runs in, when that is
running — a Scheme file's scm, a ray file's ray, a Guile buffer's Guile —
else #f, and this image answers. Completion and eldoc ask it, so a name
sent there completes and shows its signature; nothing is started to ask."
  (let ((running (hash-table-ref/default *language-running*
                                         (%buffer-language (*pane-id* :editor) buf) #f)))
    (if running
        ; (Unless the buffer was attached somewhere else by hand.)
        (if (hash-table-ref/default *attached-backends* buf #f) (backend-of buf) (running))
        (and-let* ((img (*buffer-image* buf))
                   ((not (eq? img 'self))))
          (backend-of buf)))))
(define *after-send* (lambda () #f))
