;## 08-sends.scm — the SLIME sends: what text C-x C-e, C-M-x and C-c C-r
;## read, the send itself through the buffer's backend (07-backend), and the
;## value and errors it answers (docs/editor-crate-plan.md, E3e-2 — moved
;## from the modeler's unify.scm, which keeps what a send does to the SCENE:
;## which shapes a form owns, and running a buffer or a file as the scene).

; ── S3: SLIME-style sends (docs/modeler-scene-editor-plan.md) ───────────────
; The Emacs keys, in-pane: C-x C-e sends the sexp before point, C-M-x the
; enclosing top-level form (C-c C-c too), C-c C-r the selection, C-c C-b
; the whole buffer, on top of the scene — the same wire shapes
; emacs/janet-live-mode.el sent over TCP in the Janet era, minus the TCP. Results land in
; the REPL transcript (the conversation log); errors also flash the echo
; area. No tree-sitter: a ~90-line scanner handles a lisp.

; THE SCANNER READS SCHEME. It took a dialect flag from M0 until J5c,
; because Janet and Scheme disagree about the two bytes a scanner cares most
; about (`#` opens a Janet comment and a Scheme vector); with one language
; left there is one reading.
(define scheme-prefixes (list #\' #\` #\, #\#))
; ' ` , #

(define (prefix-adjust src s)
  "Extend a span's start left over reader-prefix bytes so a form sends WITH
  its prefix: `#(1 2 3)` or `'(a b)`."
  (define prefixes scheme-prefixes)
  (let loop ((i s))
    (if (and (positive? i) (memv (string-ref src (- i 1)) prefixes))
      (loop (- i 1))
      i)))

(define (editor-scan-spans src)
  "Scan SRC once; return (spans tops) where spans = every COMPLETED form
  span (start end) in completion order (delimiter groups, strings, bare
  atoms — any depth) and tops = the top-level spans. Comments (`;` to end of
  line, `#| … |#`) are skipped; reader prefixes ride with their form."
  (define n (string-length src))
  (define (at i) (string-ref src i))
  (define (at? i c) (and (< i n) (char=? (string-ref src i) c)))
  (define spans '())                             ; newest first
  (define tops '())
  (define stack '())                             ; open-delimiter starts, innermost first
  (define atom-start #f)
  (define (push-span s e)
    (set! spans (cons (list s e) spans))
    (when (null? stack) (set! tops (cons (list s e) tops))))
  (define (end-atom end)
    (when atom-start
      (push-span atom-start end)
      (set! atom-start #f)))
  (let loop ((i 0))
    (when (< i n)
      (let ((c (at i)))
        (cond
          ; line comment — to EOL.
          ((char=? c #\;)
            (end-atom i)
            (loop (let eol ((i i)) (if (and (< i n) (not (char=? (at i) #\newline))) (eol (+ i 1)) i))))
          ; Scheme's block comment, #| … |#, which nests.
          ((and (char=? c #\#) (at? (+ i 1) #\|))
            (end-atom i)
            (loop
              (let block ((i i) (depth 0))
                (cond ((>= i n) i)
                      ((and (char=? (at i) #\#) (at? (+ i 1) #\|)) (block (+ i 2) (+ depth 1)))
                      ((and (char=? (at i) #\|) (at? (+ i 1) #\#))
                        (if (= depth 1) (+ i 2) (block (+ i 2) (- depth 1))))
                      (else (block (+ i 1) depth))))))
          ; string with escapes
          ((char=? c #\")
            (end-atom i)
            (let* ((s (prefix-adjust src i))
                   (close (let str ((i (+ i 1)))
                            (cond ((>= i n) i)
                                  ((char=? (at i) #\") i)
                                  ((char=? (at i) #\\) (str (+ i 2)))
                                  (else (str (+ i 1))))))
                   (e (if (< close n) (+ close 1) close)))
              (push-span s e)
              (loop e)))
          ; open delimiter
          ((memv c '(#\( #\[ #\{))
            (end-atom i)
            (set! stack (cons (prefix-adjust src i) stack))
            (loop (+ i 1)))
          ; close delimiter (a stray closer just moves on)
          ((memv c '(#\) #\] #\}))
            (end-atom i)
            (when (pair? stack)
              (let ((s (car stack)))
                (set! stack (cdr stack))
                (push-span s (+ i 1))))
            (loop (+ i 1)))
          ; whitespace ends an atom
          ((memv c '(#\space #\tab #\newline #\return))
            (end-atom i)
            (loop (+ i 1)))
          (else
            (unless atom-start (set! atom-start i))
            (loop (+ i 1)))))))
  (end-atom n)
  (list (reverse spans) (reverse tops)))

(define (editor-form-before src pos)
  "The span (start end) of the sexp ending at or nearest before POS —
  Emacs's eval-last-sexp target. Nil when nothing precedes."
  (let loop ((spans (car (editor-scan-spans src))) (best #f))
    (if (null? spans)
      best
      (let ((s (car (car spans))) (e (cadr (car spans))))
        (loop (cdr spans)
              (if (and (<= e pos)
                       (or (not best)
                           (> e (cadr best))
                           ; outermost wins a tie
                           (and (= e (cadr best)) (< s (car best)))))
                (list s e)
                best))))))

(define (editor-topform-at src pos)
  "The top-level form span containing POS, else the nearest one before it —
  Emacs's eval-defun target. Nil in an empty buffer."
  (let loop ((tops (cadr (editor-scan-spans src))) (best #f))
    (if (null? tops)
      best
      (let ((s (car (car tops))) (e (cadr (car tops))))
        (loop (cdr tops)
              (if (or (and (<= s pos) (<= pos e))                      ; containing
                      (and (<= e pos) (or (not best) (> e (cadr best)))))  ; else nearest preceding
                (list s e)
                best))))))

(define (editor-point text)
  "The cursor as a byte offset into the buffer text."
  (let* ((cur (%editor-cursor (editor-pane-id)))   ; (line index-in-line)
         (line (car cur))
         (idx (cadr cur))
         (lines (string-split text "\n"))
         (off (let loop ((ls lines) (i 0) (off 0))
                (if (or (null? ls) (>= i line))
                  off
                  (loop (cdr ls) (+ i 1) (+ off 1 (string-length (car ls))))))))
    (min (+ off idx) (string-length text))))

(define (markdown-buffer?)
  "True when the active editor buffer is a view-only markdown buffer (help,
  keybindings, or a .md file) — asks the kernel, so it tracks the real state."
  (positive? (%editor-markdown? (editor-pane-id))))

(define (send-shown src)
  "SRC as a send's transcript line shows it: one line, and at most 64 chars."
  (string-map (lambda (c) (if (char=? c #\newline) #\space c))
              (if (> (string-length src) 64) (string-append (substring src 0 61) "...") src)))

(define *send-failed* #f)   ; did the last send raise? (eval-owning reads it)

(define (editor-send-local src label (run src))
  "Eval SRC in THIS IMAGE's REPL env: output and value echo into the REPL
  transcript, errors also flash the echo area, and the retained scene syncs
  so the viewport reflects what was sent.

  Named for the IMAGE: there is one, the Scheme interpreter, and this is its
  door. While a second VM existed, `editor-send-scheme` forwarded to it
  through `%scm-eval`, which in a one-image build answered \"this modeler
  was built without the scm feature\" — true and useless: the forms were
  never evaluated, nine checks in `scene_editor` read as broken sends, and
  the scene they were sent to was fine."
  (*transcript-append* (string-append ";; " label ": " (send-shown src)) :accent)
  ;; Through the buffer's BACKEND (edit/scm/07-backend.scm): this image's
  ;; answers a result — what the forms PRINTED, then the value or the error —
  ;; and what the forms printed goes to the transcript above the value, kept
  ;; when one of them raises — the REPL's own rule (08-editor).
  (let* ((backend (backend-of (%editor-active-buffer (editor-pane-id))))
         (r ((backend-op backend :eval) run))
         (printed (string-trim-right (%alist-ref r :output ""))))
    (define (failed msg)
      (set! *send-failed* #t)
      (*transcript-append* (string-append "error: " msg) :error)
      (echo "error: " msg))
    (unless (string=? printed "") (*transcript-append* printed))
    (if (not (eq? (%alist-ref r :status #f) 'ok))
      (begin
        (failed (%alist-ref r :message ""))
        ;; The frames it raised in, for the error pane (12-error-pane).
        (note-send-error! label src (%alist-ref r :message "") (%alist-ref r :frames '())))
      (guard (e (#t (failed (if (string? e) e (readable e)))))
        (set! *send-failed* #f)
        (*after-send*)
        (let ((val (%alist-ref r :value #f))
              (secs (%alist-ref r :secs 0)))
          (when val (*transcript-append* (string-append "=> " (%alist-ref r :printed "")) :dim))
          ; Echo the value, Emacs eval-last-sexp style — a (defn ...) send prints
          ; its function name, so redefining confirms itself on the echo line and
          ; not only in the (possibly closed) REPL transcript.
          ; The window waited while the forms ran — a render-to-file in them
          ; traces right here — so a long run says how long, or the pause is
          ; a mystery.
          (echo label " => " (or (and (eq? val (if #f #f)) (defined-name src)) (%alist-ref r :printed ""))
                (if (>= secs 1)
                  (string-append "   (ran " (number->string (/ (round (* 10 secs)) 10.0)) " s)")
                  ""))
          val)))))

(define (defined-name src)
  "The name SRC defines when it is one definition — `(define (f …) …)`,
`(define f …)`, `(defn f …)` — else #f. A Scheme define's value is
unspecified, where Janet's defn answered the function; this is what lets a
send of one still echo `sexp => probe-fn`, so a redefinition confirms
itself on the echo line."
  (let* ((t (string-trim-both src))
         (head (find (lambda (h) (string-prefix? h t)) '("(define (" "(define " "(defn " "(def "))))
    (and head
         (let loop ((i (string-length head)))
           (if (or (= i (string-length t)) (memv (string-ref t i) '(#\space #\( #\) #\newline #\tab)))
               (and (> i (string-length head)) (substring t (string-length head) i))
               (loop (+ i 1)))))))

(define (editor-send src label (run src))
  "SLIME send funnel: eval SRC in this image — for a .scm buffer or an
  unnamed scratch — and echo a notice for anything else. Gates all four send commands (C-x C-e / C-M-x /
  C-c C-r / C-c C-b). RUN, when given, is what is evaluated in SRC's place
  (the buffer send marks each top-level form); SRC is what the transcript
  shows."
  (cond ((backend-of (%editor-active-buffer (editor-pane-id))) (editor-send-local src label run))
    (else (echo "sends work on .scm buffers only"))))

; ── What a form owns (docs/modeler-scene-buffers-plan.md, E1) ──────────────
; An unnamed shape gets a fresh key every time its form runs, so C-M-x on
; (add-sphere …) used to leave the last run's sphere behind, and a later
; C-c C-b kept those copies too (a C-M-x is REPL-born). Now each stretch of
; text that runs keeps, at its start, the keys it made — an anchor in the
; editor pane that moves as you type — and running it again replaces them:
; ray's scoped sync window sweeps exactly those it did not make again.

; What a send OWNS is the host's: *around-send* wraps the send of the text
; from START to END (RUN does the send and answers its value). The modeler's
; takes what the text made last time, opens a scoped sync window around RUN,
; and gives the text what it made this time.
(define *around-send* (lambda (run start end) (run)))

(define (eval-owning src start label)
  "Evaluate SRC, the buffer's text from character START, so that what it
  builds belongs to it: what the same text built last time is replaced, not
  added to. A failed run sweeps nothing and keeps both."
  ;; What OWNING means is the host's (*around-send*): the modeler gives what
  ;; the run made to the text at START, replacing what it made last time.
  (if (not (backend-of (%editor-active-buffer (editor-pane-id))))
    (editor-send src label)
    (*around-send* (lambda () (editor-send src label)) start (+ start (string-length src)))))

(define (eval-last-sexp)
  "Run the expression that ends just before the cursor — C-x C-e. Runs that
  expression, which may be inside a larger form, and echoes its value.

  For seeing a value — a computed point, a length — without building
  anything. On a whole (add-…) form it builds, and replaces what that form
  built last time, as C-M-x does."
  (define text (%editor-text (editor-pane-id)))
  (let ((span (editor-form-before text (editor-point text))))
    (if span
      (eval-owning (substring text (car span) (cadr span)) (car span) "sexp")
      (echo "no expression before point"))))

(define (eval-defun)
  "Run the top-level form around the cursor — C-M-x. Runs that one form, as
  it stands, in the live session; the scene file does not change.

  What it builds REPLACES what the same form built last time — a shape with
  no :name included — so edit a form and press C-M-x as often as you like
  (docs/modeler-scene-buffers-plan.md, E1). A form that raises replaces
  nothing. After changing a procedure, run the forms that CALL it again:
  shapes already built ran the old definition."
  (define text (%editor-text (editor-pane-id)))
  (let ((span (editor-topform-at text (editor-point text))))
    (if span
      (eval-owning (substring text (car span) (cadr span)) (car span) "defun")
      (echo "no top-level form at point"))))

(define (eval-region)
  "Run the selected text — C-c C-r. Runs the selection, as it stands; what
  it builds replaces what the same text built last time, as with C-M-x."
  (let ((sel (%editor-act (editor-pane-id) "copy")))
    (if sel
      (let ((at (selection-start (%editor-text (editor-pane-id)) sel)))
        (if at (eval-owning sel at "region") (editor-send sel "region")))
      (echo "no selection"))))

(define (selection-start text sel)
  ; Where SEL begins in TEXT: point is at one end of a selection, so it is
  ; the text just before point or just after it; #f if neither (then the
  ; region runs without owning what it makes).
  (let* ((p (editor-point text))
         (n (string-length sel))
         (at? (lambda (s) (and (>= s 0) (<= (+ s n) (string-length text))
                               (string=? (substring text s (+ s n)) sel)))))
    (cond ((at? (- p n)) (- p n))
          ((at? p) p)
          (else #f))))

;; A host may run a whole buffer or a file its own way — the modeler runs
;; them INTO THE SCENE, every top-level form owning what it made, and its
;; unify.scm defines both again, replacing these.

(define (eval-buffer)
  "Run this buffer's text — C-c C-b. Runs the text as it stands, unsaved
  edits included, through the buffer's backend; what it builds belongs to
  it, as with C-M-x."
  (eval-owning (%editor-text (editor-pane-id)) 0 "buffer"))

(define (eval-file)
  "Run a file you name — C-c C-l. Runs the file from disk, through the
  active buffer's backend, and opens no buffer for it."
  (read-minibuffer
    "Run file: "
    (lambda (p)
      (if (file-exists? p)
        (editor-send-local (slurp p) "file")
        (echo "no such file: " p)))
    :files))

; The SLIME keys. C-x/C-c sequences bind GLOBALLY with a focus guard: a
; pane-local prefix table would shadow the global C-x C-f / C-h b while the
; editor is focused (C-M-x, one chord, is pane-local — bind-editor-pane-keys).
(bind-key "C-c C-c" (lambda () (if (editor-focused?) (eval-defun) #f)))
(bind-key "C-x C-e" (lambda () (if (editor-focused?) (eval-last-sexp) #f)))
(bind-key "C-c C-r" (lambda () (if (editor-focused?) (eval-region) #f)))
(bind-key "C-c C-b" (lambda () (if (editor-focused?) (eval-buffer) #f)))
(bind-key "C-c C-l" 'eval-file)
