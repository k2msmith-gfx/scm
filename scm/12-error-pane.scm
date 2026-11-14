;## 12-error-pane.scm — SLIME's sldb, as a SNAPSHOT (docs/editor-crate-plan.md,
;## E3f): when a send raises and the sent code does not catch it, the calls
;## it was in and their locals, taken at the raise.
;##
;## A snapshot because a send runs on the UI thread: scm's break handler can
;## stop with the frames live (scm-dap waits in it), but nothing here can
;## wait, so the frames are printed and the send aborts. Evaluating in a
;## frame or continuing with a value needs a break that resumes later, or the
;## VM off the UI thread — scm's question. The snapshot is the backend's
;## (`:frames`, 07-backend); this file keeps the last one and shows it.

(define *last-error* #f)        ; (label src message frames) of the last failed send

; Open the pane on every failed send, as SLIME pops sldb — off, because a
; send that fails while a scene is being shaped is ordinary, and the pane
; takes the editor's place until q.
(define *error-pane-pops* #f)

(define (shell-frame? frame)
  ; The send's own machinery — the thunk, its parameterize, eval-string —
  ; is in 07-backend; the frames above it are the sent code's.
  (match frame
    ((call loc locals) (and (string? loc) (string-contains loc "07-backend.scm") #t))
    (_ #f)))

(define (note-send-error! label src message frames)
  "Keep a failed send's frames for the error pane (C-h e), and say they
  are there."
  (let ((frames (filter (lambda (f) (not (shell-frame? f))) frames)))
    (set! *last-error* (list label src message frames))
    (unless (null? frames)
      (if *error-pane-pops*
        (view-last-error)
        (*transcript-append*
          (string-append "   C-h e: the " (number->string (length frames))
                         (if (= 1 (length frames)) " call" " calls")
                         " it raised in, with their locals")
          :dim)))))

(define (md-code s)
  ; S as inline code — fenced wider when it holds a backtick itself.
  (if (string-contains s "`") (string-append "`` " s " ``") (string-append "`" s "`")))

(define (error-markdown e)
  "The error pane's page for E, a *last-error*."
  (match e
    ((label src message frames)
      (let ((out (open-output-string)))
        (define (emit . parts) (for-each (lambda (p) (display p out)) parts))
        (emit "# " message "\n\n")
        (emit "From " (md-code (string-append label ": " (send-shown src))) ". ")
        (emit "Innermost call first; each with the locals it could see. "
              "A snapshot: the send has already stopped.\n")
        (let loop ((fs frames) (i 1))
          (match fs
            (() #f)
            (((call loc locals) . rest)
              (emit "\n## " i " · " (md-code call) "\n\n")
              (when loc (emit "at " (md-code loc) "\n\n"))
              (if (null? locals)
                (emit "no locals\n")
                (for-each (lambda (l) (match l ((name value) (emit "- " (md-code name) " = " (md-code value) "\n"))))
                          locals))
              (loop rest (+ i 1)))))
        (get-output-string out)))))

(define (view-last-error)
  "Show the last failed send's calls and their locals — C-h e, SLIME's
  sldb as a snapshot: innermost call first, where each was written, and
  the locals it could see when the error was raised. q returns.

  Only for a send whose error the sent code did not catch itself, and only
  from a backend that sees frames (this image)."
  (cond ((not *last-error*) (echo "no send has failed"))
        ((null? (list-ref *last-error* 3))
          (echo "the last failed send left no frames: " (list-ref *last-error* 2)))
        (else (show-markdown-doc "*error*" (error-markdown *last-error*))
              (echo "the last failed send — q returns"))))

(bind-key "C-h e" 'view-last-error)
