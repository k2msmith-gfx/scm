;## 16-repl-commands.scm — REPL commands at scm's prompt, in every REPL
;## (docs/editor-crate-plan.md, D5): the standalone's and the modeler's.
;##
;## Guile's REPL has ,help ,describe ,apropos ,time ,expand …; scm's terminal
;## client has its own handful. At the editor's REPLs scm takes GUILE'S
;## NAMES, so the same words work at both prompts (Kevin, 2026-11-13), each
;## turned into a form scm answers — apropos, doc, macroexpand and load are
;## procedures in every scm image — and sent where the REPL sends, this image
;## or a remote-start scm alike. A REPL hands over its BACKEND (for
;## describe) and its EVAL-PRINT (a procedure that evaluates source and
;## prints the answer, as its own Return does); what the commands print goes
;## through *transcript-append*.
(define *repl-commands*
  ; (name abbreviation argument what)
  '(("help" "h" "" "this list")
    ("describe" "d" "NAME" "NAME's signature and docstring")
    ("apropos" "a" "TEXT" "every name with TEXT in its name or docstring")
    ("time" "t" "EXPR" "evaluate EXPR and say how long it took")
    ("expand" "exp" "FORM" "FORM with every macro expanded")
    ("expand1" "exp1" "FORM" "FORM with its outermost macro expanded once")
    ("load" "l" "FILE" "evaluate FILE's forms")
    ("backtrace" "bt" "" "the calls the last error raised in (C-h e)")))

(define (repl-command-line? line)
  (let ((t (string-trim-both line)))
    (and (> (string-length t) 1) (char=? (string-ref t 0) #\,))))

(define (%written x) (let ((p (open-output-string))) (write x p) (get-output-string p)))

(define (repl-command line backend eval-print)
  "Run a comma LINE at an scm REPL: BACKEND is where the REPL evaluates,
  EVAL-PRINT what evaluates source there and prints the answer — given an
  optional THEN, run once the answer has printed (it may come later, from
  another process)."
  (let* ((body (string-trim-both (substring (string-trim-both line) 1 (string-length (string-trim-both line)))))
         (sp (string-contains body " "))
         (word (if sp (substring body 0 sp) body))
         (arg (if sp (string-trim-both (substring body sp (string-length body))) ""))
         (cmd (find (lambda (c) (or (string=? word (car c)) (string=? word (cadr c)))) *repl-commands*)))
    (define (needs what)
      (when (string=? arg "") (*transcript-append* (string-append ",ERROR: " (car cmd) " needs " what) :error)))
    (if (not cmd)
      (*transcript-append* (string-append "unknown command ," word " — ,help lists them") :error)
      (case (string->symbol (car cmd))
        ((help)
          (*transcript-append* "REPL commands — Guile's names, at scm's prompt:" :accent)
          (for-each (lambda (c)
                      (*transcript-append*
                        (string-append " ," (string-pad-right (string-append (car c) " " (caddr c)) 18)
                                       " [," (cadr c) "]  " (cadddr c))
                        :dim))
                    *repl-commands*))
        ((describe)
          (needs "a NAME")
          (unless (string=? arg "")
            (let ((sig ((or (backend-op backend :signature) signature-of) arg))
                  (doc ((or (backend-op backend :doc) %doc-of) arg)))
              (if (not (or sig doc))
                (*transcript-append* (string-append "no doc for " arg) :dim)
                (begin
                  (when sig (*transcript-append* sig :accent))
                  (when doc (for-each (lambda (l) (*transcript-append* l :dim)) (string-split doc "\n"))))))))
        ((apropos) (needs "TEXT") (unless (string=? arg "") (eval-print (string-append "(apropos " (%written arg) ")"))))
        ((time)
          (needs "an EXPR")
          (unless (string=? arg "")
            (let ((t0 (%os-clock)))
              (eval-print arg
                (lambda ()
                  (let ((secs (- (%os-clock) t0)))
                    (*transcript-append* (string-append ";; " (number->string (/ (round (* secs 100000)) 100000.0)) " s") :dim)))))))
        ((expand) (needs "a FORM") (unless (string=? arg "") (eval-print (string-append "(macroexpand '" arg ")"))))
        ((expand1) (needs "a FORM") (unless (string=? arg "") (eval-print (string-append "(macroexpand-1 '" arg ")"))))
        ((load) (needs "a FILE") (unless (string=? arg "") (eval-print (string-append "(load " (%written (expand-home arg)) ")"))))
        ((backtrace)
          (match *last-error*
            ((label src message frames)
              (*transcript-append* (string-append "error: " message) :error)
              (for-each (lambda (f) (match f ((call loc locals)
                                              (*transcript-append* (string-append "  in " call (if loc (string-append "  at " loc) "")) :dim))))
                        frames))
            (_ (*transcript-append* "no error yet" :dim))))))))
