;## 04-minibuffer.scm — the one line editor every typing surface shares,
;## its completion, the minibuffer and the echo area, M-x, and the text
;## panes' size and font (docs/editor-crate-plan.md, E3c — moved from the
;## modeler's 08-editor, which keeps the REPL and the agent instances, the
;## pane roles and the click floor).

; ── Input editor + REPL console + minibuffer (M4.5) ───────────────────────
; ONE line editor serves every typing surface — the REPL pane, the
; minibuffer, later the agent panel (the plan's shared-editor decision).
; The kernel mirrors editor state for drawing (%pane-set-input), routes
; printable keys here (on-text), and draws the completion overlay
; (%set-completions); everything below is live-redefinable policy.
; NOTE: line editing is by CHARACTER (Scheme strings), and so is the pane's
; cursor arithmetic; the Janet strings this was written for were bytes.

(define *edit*
  ; The active editor, or #f: a %make-state of :pane id :prompt str :text
  ; str :cursor n :on-submit fn :history (a procedure answering the list)
  ; :hist idx-or-#f :source :files-or-#f :comp completion-state-or-#f
  #f)

(define (%edit k) (hash-table-ref/default *edit* k #f))
(define (%edit! k v) (hash-table-set! *edit* k v))
(define (%editing-pane? id) (and *edit* (equal? (%edit :pane) id)))

(define (mini-id) (*pane-id* :minibuffer))

;True while a typing surface (REPL/agent input, minibuffer) holds focus.
;Hovering a 3D viewport releases it (the mouse rules); over docks and
;text panes it holds until Escape/M-j/click dismisses.
(define *focus-held* #f)

;True while a minibuffer PROMPT — a question with a callback waiting on the
;answer — holds typing. The mouse rule below does not apply to it: a tool
;that ends a click sequence by asking for a number (calibrate-reference,
;camera-match-tool) leaves the pointer over the 3D pane it was clicking in,
;and the first twitch of the hand was closing the prompt before a digit
;could be typed (Kevin, 2026-10-04: 'the focus briefly goes to the
;minibuffer then changes to the persp view'). A prompt is answered or
;escaped, never hovered away. Set by edit-start, cleared by mini-dismiss and
;on-typing-released (below).
(define *prompt-open* #f)

; What a closed minibuffer PROMPT hands focus back to, as a FORWARD HOOK: the
; modeler's 08-editor binds it to the last 3D pane (%refocus-3d).
(define *prompt-closed* (lambda () #f))

; How M-x shows a value it echoes, as a FORWARD HOOK: `write`'s spelling
; unless the host has its own (the modeler's is `readable`).
(define *show-value* (lambda (x) (let ((p (open-output-string))) (write x p) (get-output-string p))))

(define (echo . parts)
  "Show a message in the minibuffer's echo area (idle duty: latest line)."
  (%pane-append (mini-id) "normal" (apply string-append (map %describe-part parts))))
; The forward hook (01-handlers): code loading before this file (03-selection's
; display-lines defect echo) reach the minibuffer through *echo*.
(set! *echo* (lambda (msg) (echo msg)))

; ── Completion: TAB symbols + @ files ──────────────────────────────────────

(define (rank-candidates tok cands)
  "Case-insensitive substring filter over cands, best matches first
  (earlier match, then shorter candidate). Empty tok keeps everything
  (string/find rejects an empty pattern)."
  (if (equal? tok "")
    (list-copy (%as-list cands))
    (let* ((ltok (string-downcase tok))
           (scored
             (append-map
               (lambda (c)
                 (let ((i (string-contains (string-downcase c) ltok)))
                   (if i (list (list i (string-length c) c)) '())))
               (%as-list cands))))
      ;; Earlier match first, then the shorter candidate; stable otherwise.
      (map caddr
           (list-sort (lambda (a b) (or (< (car a) (car b))
                                        (and (= (car a) (car b)) (< (cadr a) (cadr b)))))
                      scored)))))

; File completion is SEGMENT-WISE, the zsh/Emacs way: candidates are one
; directory's entries (never a capped tree walk), the first TAB extends
; to the longest common prefix, a unique directory continues inside, a
; unique file accepts. Fuzzy substring ranking stays for the flat
; namespaces (symbols, M-x) where it belongs.

(define (expand-home p)
  (if (string-prefix? "~/" p)
    (string-append (or (get-environment-variable "HOME") "~") (substring p 1 (string-length p)))
    p))

(define (split-path-token tok)
  "\"assets/mo\" -> (\"assets/\" \"mo\") — the dir part keeps its slash."
  (let ((i (let loop ((i (string-length tok)))
             (if (and (positive? i) (not (char=? (string-ref tok (- i 1)) #\/)))
                 (loop (- i 1))
                 i))))
    (list (substring tok 0 i) (substring tok i (string-length tok)))))

(define (file-entries dirpart stem)
  "One directory's entries: names only, directories wearing a trailing
  slash, dotfiles hidden unless the stem asks for them."
  (define fs (expand-home (if (equal? dirpart "") "." dirpart)))
  (append-map
    (lambda (n)
      (if (and (string-prefix? "." n) (not (string-prefix? "." stem)))
          '()
          (let* ((full (string-append fs (if (string-suffix? "/" fs) "" "/") n))
                 (st (%os-stat full)))
            (list (if (eq? :directory (%alist-ref st :mode #f)) (string-append n "/") n)))))
    (list-sort string<? (or (guard (_ (#t #f)) (%os-dir fs)) '()))))

; `keyword-candidates` — the API's :keyword arguments, harvested from the
; docstrings — is the modeler's 99-scheme-only's: the Janet version pulled
; them out with a PEG, and this image has none. symbol-candidates below asks
; it through *keyword-candidates*.

; THE HOST'S KEYWORDS, as a forward hook: the modeler harvests its API's
; :keyword arguments from the docstrings (99-scheme-only's
; keyword-candidates) and binds this to answer them.
(define *keyword-candidates* (lambda () '()))

; Every name the image has is scm's `(apropos)` with no argument.
(define (symbol-candidates)
  (append (map symbol->string (apropos)) (*keyword-candidates*)))

(define (command-candidates)
  "Callable, non-internal bindings — what M-x completes over."
  (append-map
    (lambda (sym)
      (let ((name (symbol->string sym)))
        (if (and (not (string-prefix? "%" name)) (procedure? (%env-ref sym #f)))
            (list name)
            '())))
    (apropos)))

;; THE COMPLETION STATE is a %make-state on *edit* :comp — :all :files :at
;; :cands :sel :start :dir.
(define (%comp k) (hash-table-ref/default (%edit :comp) k #f))
(define (%comp! k v) (hash-table-set! (%edit :comp) k v))

(define (comp-close)
  (when (and *edit* (%edit :comp))
    (%edit! :comp #f)
    (%clear-completions)))

(define (comp-show)
  (define cands (%comp :cands))
  (define sel (%comp :sel))
  ; An 8-row window that keeps the selection in view.
  (define lo (max 0 (min (- sel 3) (- (length cands) 8))))
  (define rows (take (drop cands lo) (- (min (length cands) (+ lo 8)) lo)))
  (%set-completions (%edit :pane) (string-join rows "\n") (- sel lo)))

(define *files-lcp* (lambda () #f))
; bound below (needs comp-accept)

(define (comp-token)
  (define tok-all (substring (%edit :text) (%comp :start) (%edit :cursor)))
  (if (%comp :at) (substring tok-all 1 (string-length tok-all)) tok-all))

(define (comp-refresh)
  (when (and *edit* (%edit :comp))
    (let* ((tok (comp-token))
           (matches
             (if (%comp :files)
                 (let* ((parts (split-path-token tok))
                        (dirp (car parts))
                        (stem (cadr parts))
                        (lstem (string-downcase stem)))
                   (%comp! :dir dirp)
                   (filter (lambda (e) (string-prefix? lstem (string-downcase e)))
                           (file-entries dirp stem)))
                 (rank-candidates tok (%comp :all)))))
      (%comp! :cands matches)
      (%comp! :sel 0)
      (if (null? matches) (comp-close) (comp-show)))))

(define (token-start)
  "Scan back from the cursor to the token's start (past word chars)."
  (define t (%edit :text))
  (let loop ((i (%edit :cursor)))
    (if (and (positive? i)
             (not (memv (string-ref t (- i 1)) '(#\space #\( #\) #\[ #\] #\{ #\} #\"))))
        (loop (- i 1))
        i)))

(define (comp-open source at)
  "Open completion at the cursor's token. source :files (segment-wise,
  zsh-style), :commands, :buffers, :definitions, :symbols, or a list of
  candidate strings; at true when triggered by @ (the
  @ is part of the token and the accepted path replaces it, quoted)."
  (define all
    (case source
      ((:files) #f)
      ; dynamic: comp-refresh lists the token's directory
      ((:commands) (command-candidates))
      ; :buffers — open editor buffers by name (S5). buffer-names lives
      ; in unify.scm (loaded later); resolve it at call time.
      ((:buffers) (let ((f (%env-ref 'buffer-names #f))) (if f (%as-list (f)) '())))
      ; :definitions — every name the definition index holds (unify.scm's
      ; find-definitions, resolved at call time like buffer-names).
      ((:definitions)
        (let ((f (%env-ref 'find-definitions #f)))
          (if f (delete-duplicates (map car (f "" 100000))) '())))
      ; A LIST is the candidates themselves — a prompt over choices it
      ; computed (document-symbols' definitions).
      (else (if (list? source) source (symbol-candidates)))))
  (%edit! :comp
    (%make-state :all all
                 :files (eq? source :files)
                 :at at
                 :cands '()
                 :sel 0
                 :start (if at (- (%edit :cursor) 1) (token-start))))
  (comp-refresh)
  (when (eq? source :files) (*files-lcp*)))

(define (comp-move d)
  (when (and *edit* (%edit :comp))
    (let ((n (length (%comp :cands))))
      (when (positive? n)
        (%comp! :sel (modulo (+ (%comp :sel) d n) n))
        (comp-show)))))

(define (edit-set text cursor)
  (%edit! :text text)
  (%edit! :cursor (max 0 (min cursor (string-length text))))
  (%pane-set-input (%edit :pane) (%edit :prompt) (%edit :text) (%edit :cursor)))

(define (%edit-splice ins)
  "The editor's text with the token under completion replaced by INS, and
  the cursor after it."
  (let ((t (%edit :text)))
    (edit-set (string-append (substring t 0 (%comp :start))
                             ins
                             (substring t (%edit :cursor) (string-length t)))
              (+ (%comp :start) (string-length ins)))))

(define (comp-replace-token tok)
  "Rewrite the token under completion (LCP extension, dir descent)."
  (%edit-splice (if (%comp :at) (string-append "@" tok) tok)))

(define (comp-accept)
  (when (and *edit* (%edit :comp))
    (let ((cand (%list-ref/default (%comp :cands) (%comp :sel) #f)))
      (cond ((not cand) (comp-close))
        ; Files: a directory DESCENDS (token becomes dir/, completion
        ; continues inside — zsh); a file finalizes (quoted for @).
        ((%comp :files)
          (let ((full (string-append (or (%comp :dir) "") cand)))
            (if (string-suffix? "/" cand)
              (begin (comp-replace-token full) (comp-refresh))
              (begin
                (%edit-splice (if (%comp :at) (string-append "\"" full "\"") full))
                (comp-close)))))
        ; Flat sources replace the token verbatim (quoted for @).
        (else
          (%edit-splice (if (%comp :at) (string-append "\"" cand "\"") cand))
          (comp-close))))))

(define (files-lcp)
  "zsh's first-TAB for paths: extend the stem to the matches' longest
  common prefix; a unique directory descends, a unique file accepts.
  True when it changed anything (else the caller menu-cycles)."
  (and *edit* (%edit :comp) (%comp :files)
       (let ((cands (%comp :cands)))
         (and (pair? cands)
              (let* ((parts (split-path-token (comp-token)))
                     (dirp (car parts))
                     (stem (cadr parts))
                     (head (car cands))
                     (lhead (string-downcase head))
                     ;; the longest common prefix, case-insensitively
                     (n (fold (lambda (c n)
                                (let ((lc (string-downcase c)))
                                  (let loop ((k 0))
                                    (if (and (< k n) (< k (string-length lc))
                                             (char=? (string-ref lhead k) (string-ref lc k)))
                                        (loop (+ k 1))
                                        k))))
                              (string-length head)
                              cands)))
                (cond ((= 1 (length cands)) (comp-accept) #t)
                      ((> n (string-length stem))
                       (comp-replace-token (string-append dirp (substring head 0 n)))
                       (comp-refresh)
                       #t)
                      (else #f)))))))
(set! *files-lcp* files-lcp)

; ── The editor itself ──────────────────────────────────────────────────────

(define (edit-sync)
  (when *edit*
    (%pane-set-input (%edit :pane) (%edit :prompt) (%edit :text) (%edit :cursor))))

(define (edit-start pane prompt on-submit &named history source initial on-quit)
  ;; HISTORY is a procedure answering the current list (or a list, kept as
  ;; it is): the REPL's grows on every eval, and a list handed over once
  ;; would be the one from the moment the editor opened. ON-QUIT is asked
  ;; first by C-g (edit-quit) and answers whether it handled the press: the
  ;; REPL's abandons a pending multi-line form.
  (%clear-completions)
  (let ((init (or initial "")))
    (set! *edit*
      (%make-state :pane pane
                   :prompt prompt
                   :text init
                   :cursor (string-length init)
                   :on-submit on-submit
                   :history (or history '())
                   :hist #f
                   :source source
                   :on-quit on-quit))
    (set! *focus-held* #t)
    (set! *prompt-open* (equal? pane (mini-id)))
    (%hold-typing pane)
    (edit-sync)))

(define (%edit-history)
  (let ((h (%edit :history))) (%as-list (if (procedure? h) (h) h))))

(define (edit-insert s)
  (%edit! :hist #f)
  (let ((t (%edit :text)) (c (%edit :cursor)))
    (edit-set (string-append (substring t 0 c) s (substring t c (string-length t)))
              (+ c (string-length s)))))

(define (edit-backspace)
  (define c (%edit :cursor))
  (when (positive? c)
    (let ((t (%edit :text)))
      (edit-set (string-append (substring t 0 (- c 1)) (substring t c (string-length t))) (- c 1))
      (if (and (%edit :comp) (< (%edit :cursor) (%comp :start)))
        (comp-close)
        (comp-refresh)))))

(define (edit-delete)
  (define t (%edit :text))
  (define c (%edit :cursor))
  (when (< c (string-length t))
    (edit-set (string-append (substring t 0 c) (substring t (+ c 1) (string-length t))) c)
    (comp-refresh)))

(define (edit-kill-to-end)
  "Kill to end of line — the killed text lands on the clipboard (Emacs)."
  (let ((t (%edit :text)) (c (%edit :cursor)))
    (%set-clipboard (substring t c (string-length t)))
    (edit-set (substring t 0 c) c)))

(define (edit-hist-show idx)
  (%edit! :hist idx)
  (let ((src (if idx (list-ref (%edit-history) idx) "")))
    (edit-set src (string-length src))))

(define (edit-up)
  (if (%edit :comp)
    (comp-move -1)
    (let ((n (length (%edit-history))))
      (when (positive? n) (edit-hist-show (max 0 (- (or (%edit :hist) n) 1)))))))

(define (edit-down)
  (if (%edit :comp)
    (comp-move 1)
    (let ((h (%edit :hist)))
      (when h
        (if (< (+ h 1) (length (%edit-history)))
          (edit-hist-show (+ h 1))
          (edit-hist-show #f))))))

(define (in-string?)
  "Whether the cursor sits inside an unclosed double-quoted string —
  where TAB should complete FILE PATHS, not symbols:
  (import-model \"assets/<TAB>."
  (define t (%edit :text))
  (let loop ((i 0) (n 0))
    (if (< i (%edit :cursor))
        (loop (+ i 1)
              (if (and (char=? (string-ref t i) #\")
                       (or (zero? i) (not (char=? (string-ref t (- i 1)) #\\))))
                  (+ n 1)
                  n))
        (odd? n))))

(define (edit-tab)
  (if (%edit :comp)
    ; zsh: TAB extends/descends while it can, then menu-cycles.
    (if (%comp :files) (if (files-lcp) #f (comp-move 1)) (comp-move 1))
    (comp-open (if (in-string?) :files (or (%edit :source) :symbols)) #f)))

(define (edit-return)
  (if (%edit :comp)
    (comp-accept)
    (let ((f (%edit :on-submit))) (when f (f (%edit :text))))))

(define (edit-paste)
  "Insert the system clipboard at the cursor, newlines becoming spaces —
  the input is one line. The REPL binds repl-paste instead, which keeps the
  lines; this is the minibuffer's paste, where a path never wants them."
  (let ((t (%clipboard)))
    (when t
      (edit-insert (string-map (lambda (c) (if (memv c '(#\newline #\tab)) #\space c)) t)))))

(define (edit-copy-line)
  "Copy the whole input line to the system clipboard (no region concept
  yet in the single-line editor)."
  (%set-clipboard (%edit :text))
  (echo "line copied"))

(define (edit-cut-line)
  "Cut the whole input line to the system clipboard."
  (%set-clipboard (%edit :text))
  (edit-set "" 0)
  (echo "line cut"))

(define (mini-dismiss)
  "Close an open minibuffer PROMPT and give the keys back to the viewport —
  the tail of submitting one. ESCAPE is the key that ABANDONS one (Kevin,
  2026-09-28: \"I'd rather use ESC to abort the prompt and/or go up a level\");
  the kernel handles that press itself, releasing typing focus, and
  on-typing-released does this side of it. C-g clears the line, as it does in
  the REPL."
  (%pane-set-input (mini-id) "" "" 0)
  (comp-close)
  (%release-typing)
  (set! *focus-held* #f)
  (set! *prompt-open* #f)
  (set! *edit* #f)
  (*prompt-closed*))

(define (edit-quit)
  (cond ((%edit :comp) (comp-close))
    ; The editor's own C-g first — the REPL's abandons a pending form (see
    ; edit-start's ON-QUIT) — then the line.
    ((let ((q (%edit :on-quit))) (and q (q))))
    (else (edit-set "" 0))))

; ── The minibuffer instance ────────────────────────────────────────────────

(define (collapse-path text)
  "Emacs's double-slash rule, for file prompts that PRE-FILL a directory.

  Typing an absolute path into a prompt already holding one produces
  `/pre/filled//typed/path` — a path that exists nowhere. Emacs reads the
  second root as \"start over from here\" and discards everything before it,
  so you can always just type an absolute path regardless of what the
  prompt was pre-filled with. Same for a `~/` typed mid-string.

  Returns TEXT unchanged when neither appears, so a normal relative or
  completed path is untouched."
  (define (last-index-of needle)
    (let loop ((from 0) (last #f))
      (let ((i (string-contains text needle from)))
        (if i (loop (+ i 1) i) last))))
  (define tilde (last-index-of "~/"))
  (define slashes (last-index-of "//"))
  (cond ; A later `~/` wins over an earlier `//` and vice versa — whichever
    ; root the user typed LAST is the one they meant.
    ((and tilde (or (not slashes) (> tilde slashes)))
      (string-append (or (get-environment-variable "HOME") "~")
                     (substring text (+ tilde 1) (string-length text))))
    (slashes (substring text (+ slashes 1) (string-length text)))
    (else text)))

(define (read-minibuffer prompt callback (source #f) (initial #f))
  "Prompt in the minibuffer with the shared input editor; `callback` gets
  the submitted string after the prompt closes. Optional completion
  source: :files (TAB completes project paths) or :symbols. `initial`
  pre-fills the input (cursor at end) — e.g. the last path, so a repeat
  find-file starts where the last one left off. Escape cancels.

  ‼ a :files prompt runs its answer through (collapse-path), so typing an
    absolute path over a pre-filled directory means what it says."
  (edit-start
    (mini-id)
    prompt
    (lambda (text)
      (mini-dismiss)
      ; transient prompt: keys go back to the viewport
      (callback (if (eq? source :files) (collapse-path text) text)))
    :source
    source
    :initial
    initial))

(define (execute-command)
  "M-x: prompt for any command by name (TAB fuzzy-completes over every
  callable in the live environment — including ones defined seconds ago)
  and run it. Arguments may follow the name, evaluated as Scheme — the
  line is the command form without its outer parens: `render :wire`,
  `set-text-size 16`, `load-model \"a.gltf\"`. The result echoes."
  (read-minibuffer
    "M-x "
    (lambda (line)
      (set! line (string-trim-both line))
      (unless (equal? line "")
        (guard (e (#t (echo "M-x " line " error: " (if (string? e) e (*show-value* e)))))
          (let ((r (eval-string (string-append "(" line ")"))))
            (echo line (if r (string-append " => " (*show-value* r)) ""))))))
    :commands))

; ── Kernel hooks ───────────────────────────────────────────────────────────

(define (on-typing-released)
  "Kernel hook: Escape released typing focus — close the overlay and any
  minibuffer prompt."
  (comp-close)
  (when (%editing-pane? (mini-id))
    (%pane-set-input (mini-id) "" "" 0)
    (set! *edit* #f)
    (*prompt-closed*))
  (set! *focus-held* #f)
  (set! *prompt-open* #f))

; Line-editor text floor (REPL / minibuffer): @ opens the file picker
; (Claude Code's convention); other keys refresh an open completion.
(add-handler
  *text-handlers*
  10
  :line-editor
  (lambda (s)
    (if (and *edit* (equal? (%typing-pane) (%edit :pane)))
      (begin
        (edit-insert s)
        (cond ((equal? s "@") (comp-open :files #t))
          ((%edit :comp) (comp-refresh))
          (else #f))
        #t)
      #f)))

; ── Editor keys (pane-local on every typing surface) ───────────────────────
; Guarded: a pane-local map fires whenever its pane is focused, but the
; editor functions only make sense while that pane holds typing focus.

(define (editing?) (and *edit* (equal? (%typing-pane) (%edit :pane))))

(define *editor-key-labels*
  ; The (editing?) guard wraps editor fns anonymously; show-keys reads
  ; the real names from here. A hash table, key -> the function or a label.
  (make-hash-table equal?))

(define (bind-editor-keys pane-key (paste #f))
  "(bind-editor-keys PANE-KEY &opt PASTE)  ->  PANE-KEY

The shared input editor's keys on one typing surface's pane-local map —
return, the motions, TAB completion, C-k, C-g, history on up/down — each
guarded so it acts only while that pane holds typing. PASTE is the paste
command (C-y, s-v, C-v), edit-paste when not given: the REPL passes one
that keeps newlines. Call it once the pane exists."
  (let ((paste (or paste edit-paste)))
    (for-each
      (lambda (binding)
        (let ((k (car binding)) (f (cadr binding)))
          (hash-table-set! *editor-key-labels* k f)
          (bind-key k (lambda () (if (editing?) (f) #f)) pane-key)))
      (list
        (list "return" edit-return)
        (list "backspace" edit-backspace)
        (list "delete" edit-delete)
        (list "tab" edit-tab)
        (list "S-tab" (lambda () (comp-move -1)))
        (list
          "left"
          (lambda () (comp-close) (edit-set (%edit :text) (- (%edit :cursor) 1))))
        (list
          "right"
          (lambda () (comp-close) (edit-set (%edit :text) (+ (%edit :cursor) 1))))
        (list "home" (lambda () (edit-set (%edit :text) 0)))
        (list "end" (lambda () (edit-set (%edit :text) (string-length (%edit :text)))))
        (list "C-a" (lambda () (edit-set (%edit :text) 0)))
        (list "C-e" (lambda () (edit-set (%edit :text) (string-length (%edit :text)))))
        (list "C-k" edit-kill-to-end)
        (list "C-y" paste)
        (list "s-v" paste)
        ; C-v pastes here too (user call 2026-09-14). Emacs reserves it
        ; for page-down, but these two panes are one line and one
        ; transcript — nothing to page — and the editor pane, where
        ; paging and vi's visual-block both matter, keeps it free.
        (list "C-v" paste)
        (list "s-c" edit-copy-line)
        (list "s-x" edit-cut-line)
        (list "C-g" edit-quit)
        (list "up" edit-up)
        (list "down" edit-down)))
    pane-key))

(define (%bind-labelled pane-key bindings)
  "Bind each (key f label) in BINDINGS on PANE-KEY's map, and file its label."
  (for-each
    (lambda (b)
      (hash-table-set! *editor-key-labels* (car b) (caddr b))
      (bind-key (car b) (cadr b) pane-key))
    bindings))

(define (set-text-size px)
  "Set the text panes' font size in pixels (clamped 8..32). Returns the
  size in effect; (set-text-size 0) just reads it. Default 14."
  (%set-text-size px))
(define (text-size+)
  "Bump the text panes' font one pixel (C-+)."
  (set-text-size (+ (set-text-size 0) 1)))
(define (text-size-)
  "Shrink the text panes' font one pixel (C--)."
  (set-text-size (- (set-text-size 0) 1)))
(define (set-text-font path (index #f))
  "Load a monospace face for the text panes from a local font file
  (.ttf/.otf/.ttc; optional face index inside a collection; ~/ expands).
  System fonts work without bundling:
  (set-text-font \"/System/Library/Fonts/Monaco.ttf\"). Returns \"ok\" or
  the error; the current face survives a bad load. Default = bundled
  JetBrains Mono. Persist a choice in ~/.config/modeler/init.scm."
  (%set-text-font (%text path) (or index 0)))
(define (set-text-rendering (mode #f))
  "How glyphs are hinted and placed, for every text surface — the knob the
  Zed comparison came out of (docs/modeler-text-rendering-research.md).
  MODE is :smooth (the DEFAULT — no hinting, quarter-pixel placement, and a
  coverage curve that thickens strokes: Zed's geometry plus a stand-in for
  the luminance-dependent dilation macOS applies and we don't), :zed (the
  same without the thickening), :crisp (hinted and whole-pixel everywhere —
  uniformly sharp, spacing jitters up to half a pixel), :auto (:smooth on a
  HiDPI screen, :crisp below scale 1.5, re-decided every frame so moving the
  window between displays follows along — for a low-DPI screen where :smooth
  disappoints) or :current (what shipped before 2026-09-14 — hinted, with the
  editor alone on quarter-pixel bins). Takes effect on the next frame. Returns the
  mode now in effect; (set-text-rendering) just reads it. Persist a taste in
  ~/.config/modeler/init.scm."
  (%as-keyword (%set-text-rendering (if mode (%text mode) ""))))
(define (text-rendering)
  "The glyph rendering mode actually drawing right now — like
  (set-text-rendering) but with :auto resolved against this display's scale,
  so it answers :smooth or :crisp rather than :auto. The one to ask when the
  text looks different than you expected on a second monitor."
  (%as-keyword (%text-rendering)))
(define (set-label-font path (index #f))
  "Load the proportional UI face from a local font file — menus (both the
  linear popup and the radial pie), completion rows, the status strip and the
  viewport HUDs. Same contract as `set-text-font` (.ttf/.otf/.ttc; optional
  face index; ~/ expands; returns \"ok\" or the error, and the current face
  survives a bad load). Default = bundled IBM Plex Sans, the face Zed draws
  its own popups in. Persist a choice in ~/.config/modeler/init.scm."
  (%set-label-font (%text path) (or index 0)))

; Global bindings for the shell's commands — bound after they exist.
(bind-key "M-x" 'execute-command)
(bind-key "C-=" 'text-size+)
(bind-key "C-S-=" 'text-size+)
(bind-key "C--" 'text-size-)
