;## 09-complete.scm — completion at the cursor, eldoc in the echo area,
;## and describe (C-c C-d d, vi's K) in the editor pane
;## (docs/editor-crate-plan.md, E3e-3 — moved from the modeler's unify.scm).

; ── Completion + signature help (user request 2026-09-08) ───────────────────
; The REPL's candidate pool (symbol-candidates: every live binding + the
; :keyword args harvested from docstrings), shown in a popup AT THE CURSOR.
; TAB completes mid-symbol while characters insert (vi insert mode / vi
; off) and re-indents otherwise; while the popup is up TAB/S-TAB/arrows
; cycle, return accepts, C-g/Escape close. The echo area shows the
; enclosing form's signature as you type (eldoc); K in vi normal mode —
; vi's doc-lookup key — or C-c C-d d prints the full docstring (the aligned
; parameter block) to the REPL transcript.

(define *editor-comp* #f)
; a %make-state of :all (the pool) :cands (ranked) :sel :start (an offset)

(define (editor-insertish?)
  "Typed characters INSERT here (vi insert/replace mode, or vi off)."
  (define m (%text (%editor-vi-mode (editor-pane-id))))
  (or (equal? m "") (equal? m "INSERT") (equal? m "REPLACE")))

(define (editor-text+point)
  (define text (%text (%editor-text (editor-pane-id))))
  (list text (editor-point text)))

(define editor-delims (list " " "\t" "\n" "(" ")" "[" "]" "{" "}" "\"" "`" "#"))

(define (editor-token-start text point)
  "Scan back from POINT over symbol chars to the token's start."
  (let loop ((i point))
    (if (and (positive? i) (not (member (substring text (- i 1) i) editor-delims)))
      (loop (- i 1))
      i)))

(define (editor-comp-cancel)
  "Close the editor's completion popup (also the kernel's Escape hook)."
  (set! *editor-comp* #f)
  (%clear-completions))

(define (editor-comp-show)
  (define comp *editor-comp*)
  (define cands (%state-ref comp :cands))
  (define sel (%state-ref comp :sel))
  ; An 8-row window that keeps the selection in view (the REPL's rule).
  (define lo (max 0 (min (- sel 3) (- (length cands) 8))))
  (define rows (take (drop cands lo) (- (min (length cands) (+ lo 8)) lo)))
  (%editor-completions (editor-pane-id) (string-join rows "\n") (- sel lo)))

(define (editor-comp-refresh)
  (let ((comp *editor-comp*))
    (when comp
      (let* ((tp (editor-text+point))
             (text (car tp))
             (point (cadr tp)))
        (if (< point (%state-ref comp :start))
          (editor-comp-cancel)
          (let* ((tok (substring text (%state-ref comp :start) point))
                 (matches (%as-list (rank-candidates tok (%state-ref comp :all)))))
            (hash-table-set! comp :cands matches)
            (hash-table-set! comp :sel 0)
            (if (null? matches) (editor-comp-cancel) (editor-comp-show))))))))

(define (editor-candidates)
  "The completion pool: this image's bindings and the API's :keywords
  (symbol-candidates), whatever buffer the editor is on. The caller ranks
  it against what has been typed.

  It asked `%scm-complete` for a Scheme buffer while that was a second VM's
  door (modeler-port-plan M0); from inside the one image the door leads
  nowhere (00-decls stubs it to answer nothing), and this pool is what it
  would have answered."
  (symbol-candidates))

(define (editor-complete)
  "Open symbol completion at the cursor's token — TAB mid-symbol."
  (let* ((tp (editor-text+point))
         (text (car tp))
         (point (cadr tp)))
    (set! *editor-comp*
      (%make-state
        :all (editor-candidates)
        :cands '()
        :sel 0
        :start (editor-token-start text point)))
    (editor-comp-refresh)))

(define (offset->pos text off)
  "A byte offset into TEXT as (line index-in-line)."
  (let loop ((i 0) (line 0) (start 0))
    (cond ((>= i off) (list line (- off start)))
          ((char=? (string-ref text i) #\newline) (loop (+ i 1) (+ line 1) (+ i 1)))
          (else (loop (+ i 1) line start)))))

(define (editor-comp-accept)
  (let* ((comp *editor-comp*)
         (cand (and comp (%list-ref/default (%state-ref comp :cands) (%state-ref comp :sel) #f))))
    (when comp
      (if (not cand)
        (editor-comp-cancel)
        (let* ((tp (editor-text+point))
               (text (car tp))
               (point (cadr tp))
               (p0 (offset->pos text (%state-ref comp :start)))
               (p1 (offset->pos text point)))
          (%editor-replace (editor-pane-id) (car p0) (cadr p0) (car p1) (cadr p1) cand)
          (editor-comp-cancel))))))

(define (editor-comp-move d)
  (let ((comp *editor-comp*))
    (when comp
      (let ((n (length (%state-ref comp :cands))))
        (if (positive? n)
          (begin (hash-table-set! comp :sel (modulo (+ (%state-ref comp :sel) d n) n)) (editor-comp-show))
          #f)))))

; ── eldoc: the enclosing form's signature in the echo area ──────────────────

(define *editor-eldoc* #f)

(define (signature-of name)
  "NAME's signature line, or #f: the first line of the docstring's SECOND
  paragraph when that opens with `(`, else its first line. Janet prefixed
  every docstring with an auto arity line ((name & args) + blank), so the
  authored signature sat second — the promotion help.janet made; scm's
  `doc` has no prefix, so an aligned docstring's signature is its first
  line, which the fallback answers."
  (let ((d (%doc-of name)))
    (and d
      (let* ((first-line (lambda (s) (let ((i (string-contains s "\n"))) (if i (substring s 0 i) s))))
             (para (string-contains d "\n\n"))
             (body (if para (substring d (+ para 2) (string-length d)) d))
             (line (first-line body)))
        (if (string-prefix? "(" line) line (first-line d))))))

(define (editor-eldoc)
  "Echo the enclosing form's signature when its head changed under typing."
  (define head (%editor-head (editor-pane-id)))
  (unless (equal? head *editor-eldoc*)
    (set! *editor-eldoc* head)
    (when head
      (let ((sig (signature-of head))) (when sig (echo sig))))))

(define (editor-token-around text point)
  "The whole symbol token spanning POINT."
  (define n (string-length text))
  (let loop ((e point))
    (if (and (< e n) (not (member (substring text e (+ e 1)) editor-delims)))
      (loop (+ e 1))
      (substring text (editor-token-start text point) e))))

(define (describe-name name)
  "(describe-name NAME)  ->  #t when it found a doc

NAME's full docstring into the transcript under a `── (doc NAME) ──` rule,
its signature echoed — as the active buffer's backend answers them (this
image's, for a .scm buffer). Says so, and answers #f, when there is none."
  (define backend (*buffer-backend* (%editor-active-buffer (editor-pane-id))))
  (define doc-of (or (backend-op backend :doc) %doc-of))
  (define sig-of (or (backend-op backend :signature) signature-of))
  (if (not (sig-of name))
    (begin (echo "no doc for " name) #f)
    (begin
      (*help-open*)
      (*transcript-append* (string-append "── (doc " name ") ──────────────────") :accent)
      (for-each (lambda (line) (*transcript-append* line :dim)) (string-split (doc-of name) "\n"))
      (echo (sig-of name))
      #t)))

(define (editor-describe)
  "Full docstring — signature + the aligned parameter block — for the
  symbol at point (else the enclosing form's head) into the REPL
  transcript, signature echoed. K in vi normal mode, C-c C-d d anywhere.
  The answer comes from the buffer's backend (this image, for a .scm
  buffer)."
  (define tp (editor-text+point))
  (define at-point (editor-token-around (car tp) (cadr tp)))
  (define (head-name) (%text (or (%editor-head (editor-pane-id)) "")))
  (define backend (*buffer-backend* (%editor-active-buffer (editor-pane-id))))
  (define sig-of (or (backend-op backend :signature) signature-of))
  ; A token with no doc (an argument, a number) defers to the form's head:
  ; point on `2` in (add-sphere :at [1 2|) describes add-sphere. (A Scheme
  ; buffer asked a second VM's `%scm-describe` until J5c retired that VM;
  ; from then on it answered nothing, and C-c C-d d said "no doc for 2".)
  (let ((name (if (or (string=? at-point "") (not (sig-of at-point))) (head-name) at-point)))
    (if (string=? name "")
      (echo "nothing at point to describe")
      (describe-name name))))

(define (editor-back)
  "Switch the editor back to the previously-visited buffer — the markdown
  viewer's marking-menu 'back' (returns from a *help*/*keybindings* doc to
  wherever you were). Same target as C-x b's default (2nd-MRU)."
  (let ((prev (%list-ref/default (%buffer-mru (editor-pane-id)) 1 #f)))
    (if prev (visit-buffer prev) (echo "no previous buffer"))))

; Comp-aware key rebinds (bind-key replaces the S2 bindings) + the vi K
; doc-lookup and eldoc refresh on the typing path.
; Completion-aware editor typing (claims BEFORE the plain :editor-insert
; layer): armed prefix keys route to the keymap, 'q' quits a view-only
; popup, vi-NORMAL K describes, and inserts refresh completion + eldoc.
(add-handler
  *text-handlers*
  30
  :editor-completion
  (lambda (s)
    (cond ; A prefix key (C-x …) is armed: the kernel still delivers the next
      ; bare key as TEXT while a pane is typing, so without this it would
      ; self-insert and the sequence would never reach its binding — that's
      ; why C-x b failed but chorded C-x C-f worked. Route it through the
      ; keymap to finish the sequence (on-key consumes the armed prefix).
      (*pending-keymap* (on-key s) #t)
      ((editor-typing?)
        (cond ; 'q' in a view-only popup (help, keybindings, any .md reading
          ; view) quits it — the buffer can't be edited, so q is unambiguous.
          ((and (editor-view-only?) (equal? s "q")) (editor-quit-view))
          ((and (equal? s "K") (equal? (%text (%editor-vi-mode (editor-pane-id))) "NORMAL"))
            (editor-describe))
          (else
            (%editor-act (editor-pane-id) "insert" s)
            (when *editor-comp* (editor-comp-refresh))
            (editor-eldoc)))
        #t)
      (else #f))))

; Any click closes an open editor completion popup (a click moves the
; cursor) — pure side effect, never claims; the click continues down.
(add-handler
  *click-handlers*
  70
  :editor-comp-cancel
  (lambda (x y shift) (if *editor-comp* (editor-comp-cancel) #f) #f))

(define (bind-editor-completion-keys)
  "(bind-editor-completion-keys)  ->  :editor

The editor pane's completion-aware keys — return accepts, TAB completes or
indents, S-TAB / up / down cycle, C-g closes, backspace narrows — over the
plain ones (bind-editor-pane-keys), which they replace. Call it after that,
once the pane exists."
  (for-each
    (lambda (row)                                  ; (key command label)
      (let ((k (car row)) (f (cadr row)) (label (caddr row)))
        (hash-table-set! *editor-key-labels* k label)
        (bind-key k f :editor)))
    (list
      (list
        "return"
        (%when-typing
          (lambda ()
            (if *editor-comp*
              (editor-comp-accept)
              ; A vi ':' ex-command comes back as ":<text>"; a plain
              ; newline returns nil.
              (let ((ex (%editor-act (editor-pane-id) "enter")))
                (when ex (editor-ex-command ex))))))
        "editor-newline/comp-accept")
      (list
        "tab"
        (%when-typing
          (lambda ()
            (cond (*editor-comp* (editor-comp-move 1))
              ; Mid-symbol while typing inserts: complete. Elsewhere
              ; (line starts, vi normal): re-indent.
              ((and (editor-insertish?)
                    (let ((tp (editor-text+point))) (< (editor-token-start (car tp) (cadr tp)) (cadr tp))))
                (editor-complete))
              (else (%editor-act (editor-pane-id) "indent")))))
        "editor-complete/indent")
      (list "S-tab" (%when-typing (lambda () (editor-comp-move -1))) "editor-comp-previous")
      (list
        "up"
        (%when-typing
          (lambda () (if *editor-comp* (editor-comp-move -1) (%editor-act (editor-pane-id) "up"))))
        "editor-up/comp")
      (list
        "down"
        (%when-typing
          (lambda () (if *editor-comp* (editor-comp-move 1) (%editor-act (editor-pane-id) "down"))))
        "editor-down/comp")
      (list
        "C-g"
        (%when-typing
          (lambda () (if *editor-comp* (editor-comp-cancel) (%editor-act (editor-pane-id) "escape"))))
        "editor-comp-cancel/deselect")
      (list
        "backspace"
        (%when-typing
          (lambda ()
            (%editor-act (editor-pane-id) "backspace")
            (when *editor-comp* (editor-comp-refresh))
            (editor-eldoc)))
        "editor-backspace")))
  :editor)

; SLIME's documentation prefix: C-c C-d d (or C-c C-d C-d) describes.
(bind-key "C-c C-d d" (lambda () (if (editor-focused?) (editor-describe) #f)))
(bind-key "C-c C-d C-d" (lambda () (if (editor-focused?) (editor-describe) #f)))

