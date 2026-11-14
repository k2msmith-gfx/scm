;## 13-term.scm — the terminal pane (docs/editor-crate-plan.md, E5): a
;## program in a pseudo-terminal — a REPL, a shell, a build — in a pane, and
;## the second kind of BACKEND: a buffer attached to a terminal sends its
;## forms there, comint's model, which is how a language with no in-process
;## image gets SLIME-style sends. The terminal itself is the widget's
;## (termpane.rs, alacritty's terminal library); this is what the keys do.

; What the HOST supplies, as FORWARD HOOKS:
;   *term-pane*    the pane a terminal shows in, or #f for none (the
;                  standalone: its lower pane; the modeler: a dock pane)
;   *term-shown*   make that pane visible (the modeler opens its dock)
;   *term-closed*  the terminal is gone (the modeler gives the dock back to
;                  its REPL)
;   *term-showing?* is pane ID showing its terminal now? (the standalone's
;                  panel shows the REPL in the same pane while its terminal
;                  runs on; the modeler's terminal has a pane of its own)
(define *term-pane* (lambda () (*pane-id* :terminal)))
(define *term-shown* (lambda (id) #f))
(define *term-closed* (lambda (id) #f))
(define *term-showing?* (lambda (id) #t))

; THE KEYS THE EDITOR KEEPS while a terminal has the keyboard (D4, Emacs
; vterm's choice): the C-x and C-c prefixes and M-x, so C-x o leaves, C-x b
; switches buffers and M-x runs a command. Everything else goes to the
; program — Escape too, so vi in a terminal works — and C-c C-c sends the
; program a literal C-c.
(define *term-kept-keys* '("C-x" "C-c" "M-x"))

(define (term-id)
  "The terminal pane's id when it shows a terminal, else #f."
  (let ((id (*term-pane*)))
    (and id (positive? (%term-open? id)) id)))

(define (terminal-typing?)
  (let ((t (%typing-pane)))
    (and t (positive? (%term-open? t)) (*term-showing?* t))))

(define (term-focus)
  "Give the terminal pane the keyboard."
  (let ((id (*term-pane*)))
    (when id
      (*term-shown* id)
      (%focus-pane id)
      (set! *focus-held* #t)
      (%hold-typing id))))

(define (term-open (program #f) . args)
  "(term-open [PROGRAM ARG …])  ->  #t

Run PROGRAM with ARGS in the terminal pane and give it the keyboard —
`(term-open \"scm\")` for a Scheme REPL in its own process, `(term-open
\"python3\")`, or, with no PROGRAM, your login shell. A program already
running there is stopped first. C-x o leaves the terminal; C-c C-c sends it
a C-c.

The login shell comes from the account database, not $SHELL: under `nix
develop` $SHELL is the build environment's bash, built without readline —
no Tab completion, no history (Kevin, 2026-11-14)."
  (let ((id (*term-pane*))
        (program (or program (%login-shell) (get-environment-variable "SHELL") "/bin/sh")))
    (if (not id)
      (begin (echo "no pane to run a terminal in") #f)
      (begin
        (%term-open id program args #f)
        (term-focus)
        (echo program " — C-x o leaves, C-c C-c sends C-c")
        #t))))

(define (term)
  "Prompt for a program and run it in the terminal pane — M-x term. The
  line is the program and its arguments; empty runs your login shell."
  (read-minibuffer
    "Run in terminal: "
    (lambda (line)
      (let ((words (filter (lambda (w) (not (string=? w ""))) (string-split (string-trim-both line) " "))))
        (if (null? words) (term-open) (apply term-open words))))))

(define (term-close)
  "Stop the terminal pane's program."
  (when-let ((id (term-id)))
    (%term-close id)
    (when (equal? (%typing-pane) id) (%release-typing))
    (*term-closed* id)
    (echo "terminal closed")))

(define (other-pane)
  "Move the keyboard between the editor and the terminal — C-x o."
  (cond ((terminal-typing?) (focus-scene-editor))
        ((term-id) (term-focus))
        (else (echo "no terminal — M-x term runs one"))))

; ── Keys and text go to the program ────────────────────────────────────────
; Above the keymap (10) and below the menus (20): an open popup still takes
; its keys, and a prefix the editor keeps finishes in the keymap.
(add-handler
  *key-handlers*
  15
  :terminal
  (lambda (key)
    (and (terminal-typing?)
         (not *pending-keymap*)
         (not (member key *term-kept-keys*))
         (positive? (%term-key (%typing-pane) key)))))

(add-handler
  *text-handlers*
  40
  :terminal
  (lambda (s)
    (and (terminal-typing?) (not *pending-keymap*)
         (begin (%term-send (%typing-pane) s) #t))))

; A click in the terminal gives it the keyboard; the wheel scrolls its
; history.
(add-handler
  *click-handlers*
  55
  :terminal-focus
  ; Not while a menu is up: the click is the menu's — an item over the
  ; terminal ran nothing, the click focusing the pane instead (Kevin,
  ; 2026-11-13).
  (lambda (x y . _)
    (let ((hit (%pane-at x y)))
      (and (not (menu-open?)) hit (equal? (cadr hit) "term")
           (begin (term-focus) #t)))))

(add-handler
  *wheel-handlers*
  35
  :terminal-scroll
  (lambda (d x y . _)
    (let ((hit (and x y (%pane-at x y))))
      (and hit (equal? (cadr hit) "term")
           (begin (%term-scroll (car hit) (* 3 (if (positive? d) 1 -1))) #t)))))

(bind-key "C-x o" 'other-pane)

; ── Copy and paste ─────────────────────────────────────────────────────────
; A drag selects (the host routes the mouse to the terminal's grid); Cmd-C
; copies the selection; Cmd-V pastes the clipboard into the program — as one
; bracketed paste when the program asked for that, so a shell takes the
; lines as one paste rather than running each as it arrives.

(define (term-copy)
  "Copy the terminal's selection to the clipboard — Cmd-C in the terminal,
  and Copy on its menu. Drag to select."
  (let* ((id (*term-pane*)) (s (and id (%term-selection id))))
    (if s
      (begin (%set-clipboard s) (%term-clear-selection id) (echo "copied " (string-length s) " chars") s)
      (begin (echo "nothing selected — drag to select") #f))))

(define (term-paste)
  "Paste the clipboard into the terminal's program — Cmd-V in the
  terminal, and Paste on its menu."
  (let ((id (*term-pane*)) (t (%clipboard)))
    (cond ((not (and id t (positive? (%term-open? id)))) (echo "nothing to paste into") #f)
          (else (%term-paste id t) t))))

(define (bind-term-keys)
  "(bind-term-keys)  ->  :terminal

The terminal pane's own keys: C-c C-c sends the program a C-c (C-c being
a prefix the editor keeps); Cmd-C copies the selection and Cmd-V pastes
the clipboard into the program. In the pane's local map, so the editor's
C-c C-c is untouched. Call it once the host's terminal pane exists."
  (bind-key "C-c C-c" (lambda () (%term-key (*term-pane*) "C-c")) :terminal)
  (bind-key "s-c" 'term-copy :terminal)
  (bind-key "s-v" 'term-paste :terminal)
  :terminal)

; ── A terminal as a BACKEND (E3e's interface) ──────────────────────────────
; A send writes the text and a newline to the program, which answers in its
; own pane: nothing structured comes back (inferior-lisp's model; E5b's
; socket backend is SLIME's). Completion and docs are not offered, so those
; commands fall back to this image's.

(define (terminal-backend id)
  "A backend that types each send into terminal pane ID."
  (list (cons :name "the terminal")
        (cons :eval
              (lambda (src)
                (if (positive? (%term-send id (string-append (string-trim-right src) "\r")))
                  (list (cons :status 'ok) (cons :value #f) (cons :printed "sent to the terminal")
                        (cons :output "") (cons :secs 0))
                  (list (cons :status 'error) (cons :message "the terminal has gone") (cons :output "")))))))

(define (term-attach)
  "Send this buffer's forms to the terminal's program from now on —
  C-M-x, C-x C-e, C-c C-b type them into it, as Emacs's inferior-lisp does.
  M-x term-detach sends them to this image again."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (if-let ((id (term-id)))
      (begin (hash-table-set! *attached-backends* buf (terminal-backend id))
             (echo (%text (buffer-name buf)) " sends to the terminal"))
      (echo "no terminal — M-x term runs one"))))

(define (term-detach)
  "Send this buffer's forms to the host's backend again."
  (let ((buf (%editor-active-buffer (editor-pane-id))))
    (hash-table-delete! *attached-backends* buf)
    (echo (%text (buffer-name buf)) " sends to this image")))
