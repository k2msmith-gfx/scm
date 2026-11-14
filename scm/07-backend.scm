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
;                        or ((:status . error) (:message . "…") (:output . "…"))
;                        :value is the live value where the backend has one
;                        (in-process); :printed is what to show either way
;   :complete (prefix) → candidate names
;   :doc (name)        → a docstring, or #f
;   :signature (name)  → a signature line, or #f       (E3e-3)
;   :definition-sites (name) → ((path line column form) …)   (E3e-4)
(define (backend-op backend key)
  "BACKEND's procedure for KEY, or #f when it offers none."
  (and backend (%alist-ref backend key #f)))

(define (in-process-eval src)
  "A send evaluated in THIS image — SRC's forms in order, at top level — as
a backend RESULT: what the forms printed is captured, and a form that
raises answers an error result rather than unwinding the editor."
  (let ((out (open-output-string)))
    (guard
      (e (#t (list (cons :status 'error)
                   (cons :message (if (string? e) e (*show-value* e)))
                   (cons :output (get-output-string out)))))
      (let* ((t0 (current-jiffy))
             (val (parameterize ((current-output-port out)) (eval-string src)))
             (secs (/ (- (current-jiffy) t0) (jiffies-per-second))))
        (list (cons :status 'ok)
              (cons :value val)
              (cons :printed (*show-value* val))
              (cons :output (get-output-string out))
              (cons :secs secs))))))

(define (in-process-backend)
  "The backend that is this image: sends evaluate here, completion and docs
answer from its live bindings."
  (list (cons :name "this image")
        (cons :eval in-process-eval)
        (cons :complete
              (lambda (prefix)
                (filter (lambda (n) (string-prefix? prefix n))
                        (map symbol->string (apropos)))))
        (cons :doc %doc-of)))

; Which backend a BUFFER sends to, or #f for none — the host's choice, as a
; FORWARD HOOK (the modeler: this image, for a Scheme buffer). And what the
; host does after a send has run (the modeler syncs the scene, refreshes the
; outliner, nudges live render).
(define *buffer-backend* (lambda (buf) (in-process-backend)))
(define *after-send* (lambda () #f))
