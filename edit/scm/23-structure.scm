;## 23-structure.scm — editing Lisp by EXPRESSION rather than by character
;## (Kevin, 2026-11-16): Emacs's C-M- sexp commands, paredit's
;## slurp/barf/wrap/splice/raise, comment toggling, and the switch for
;## typing brackets in pairs. The work is the kernel's (src/sexp.rs, over
;## the scanner the editor highlights from); this file names the commands
;## and binds their keys, pane-local to the editor, so they work in every vi
;## mode as the other chords do.

(define (sexp-run op none)
  ; Run structural command OP on the editor; say NONE when there was
  ; nothing for it to do. A kill's text goes to the clipboard.
  (let ((r (%editor-sexp (editor-pane-id) op)))
    (cond ((string? r) (%set-clipboard r) r)
          ((eqv? r 0) (echo "not a Scheme or Guile buffer") #f)
          (r #t)
          (else (echo none) #f))))

; ── Moving ─────────────────────────────────────────────────────────────────
(define (forward-sexp)
  "Move past the next expression — a list, a string or an atom, with any
  quote before it — at the point's own level (C-M-f). With a selection, it
  grows."
  (sexp-run "forward" "no expression after the point in this list"))

(define (backward-sexp)
  "Move to the start of the previous expression (C-M-b)."
  (sexp-run "backward" "no expression before the point in this list"))

(define (backward-up-list)
  "Move out to the opening bracket of the list around the point (C-M-u)."
  (sexp-run "up" "not inside a list"))

(define (up-list)
  "Move out past the closing bracket of the list around the point (C-M-n)."
  (sexp-run "up-forward" "not inside a closed list"))

(define (down-list)
  "Move into the next list, just past its opening bracket (C-M-d)."
  (sexp-run "down" "no list ahead in this one"))

(define (mark-sexp)
  "Select the next expression; again, select one more (C-M-space)."
  (sexp-run "mark" "no expression after the point"))

; ── Deleting and swapping ──────────────────────────────────────────────────
(define (kill-sexp)
  "Delete from the point to the end of the next expression, onto the
  clipboard (C-M-k)."
  (sexp-run "kill-forward" "no expression after the point"))

(define (backward-kill-sexp)
  "Delete from the start of the previous expression to the point, onto the
  clipboard (C-M-backspace)."
  (sexp-run "kill-backward" "no expression before the point"))

(define (transpose-sexps)
  "Swap the expressions either side of the point (C-M-t)."
  (sexp-run "transpose" "needs an expression on each side of the point"))

; ── Reshaping (paredit's) ──────────────────────────────────────────────────
(define (slurp-forward)
  "The list around the point takes in the expression after it: (a |b) c
  becomes (a |b c)  (C-))."
  (sexp-run "slurp-forward" "nothing after this list to take in"))

(define (barf-forward)
  "The list around the point lets its last expression out: (a |b c) becomes
  (a |b) c  (C-})."
  (sexp-run "barf-forward" "nothing in this list to let out"))

(define (slurp-backward)
  "The list around the point takes in the expression before it: a (|b)
  becomes (a |b)  (C-()."
  (sexp-run "slurp-backward" "nothing before this list to take in"))

(define (barf-backward)
  "The list around the point lets its first expression out: (a |b c)
  becomes a (|b c)  (C-{)."
  (sexp-run "barf-backward" "nothing in this list to let out"))

(define (wrap-sexp)
  "Wrap the expression at the point in a list, the point just inside the
  new bracket (M-()."
  (sexp-run "wrap" "nothing to wrap"))

(define (splice-sexp)
  "Remove the brackets of the list around the point, keeping what was in
  it (M-s)."
  (sexp-run "splice" "not inside a list"))

(define (raise-sexp)
  "Replace the list around the point with the expression at the point:
  (if c |(do-it) #f) becomes (do-it)  (M-r)."
  (sexp-run "raise" "no expression at the point inside a list"))

(define (indent-defun)
  "Re-indent every line of the top-level form the point is in — or, with
  a selection, every selected line (in any language) — to the column the
  newline would give it (C-M-q). A line that starts inside a string keeps
  its own indentation."
  (sexp-run "indent-defun" "no form at the point"))

; ── Comments ───────────────────────────────────────────────────────────────
(define (comment-dwim)
  "Comment the selected lines — or the point's line — with ;; at their
  shared indent, or take the comment off when they all have one (M-;)."
  (sexp-run "comment-lines" "nothing to comment"))

(define (comment-sexp)
  "Comment the expression at the point out with #; — the reader skips it,
  brackets and all — or back in when it already is (C-M-;)."
  (sexp-run "comment-datum" "no expression at the point"))

; ── Pairing ────────────────────────────────────────────────────────────────
(define *paren-pairing* #t)

(define (set-paren-pairing (on #f))
  "Type brackets and quotes in pairs in a Scheme buffer, and keep them
  balanced: ( brings its ), a ) steps out past the list's own, backspace
  takes an empty () whole and steps over a full one, and \" steps over a
  string's closing quote. On by default. As M-x, with no ON, it toggles;
  (set-paren-pairing #f) in your init.scm starts with it off. Inside a
  string or a comment typing is plain; a paste is never changed."
  (let ((v (if (given? on) (and on #t) (not *paren-pairing*))))
    (set! *paren-pairing* v)
    (%editor-set-pairing (if v 1 0))
    (echo "paren pairing " (if v "on" "off"))))

; ── Keys ───────────────────────────────────────────────────────────────────
; Shifted punctuation is named by its key: C-) is C-S-0, M-( is M-S-9.
; Bound when the editor's pane-local map is (bind-editor-pane-keys, which
; runs once the pane exists — after this file loads).
(define (bind-structure-keys)
  (for-each (lambda (b) (bind-key (car b) (cadr b) :editor))
    '(("C-M-f" forward-sexp)
    ("C-M-b" backward-sexp)
    ("C-M-u" backward-up-list)
    ("C-M-n" up-list)
    ("C-M-d" down-list)
    ("C-M-space" mark-sexp)
    ("C-M-k" kill-sexp)
    ("C-M-backspace" backward-kill-sexp)
    ("C-M-t" transpose-sexps)
    ("C-S-0" slurp-forward)
    ("C-S-]" barf-forward)
    ("C-S-9" slurp-backward)
    ("C-S-[" barf-backward)
    ("M-S-9" wrap-sexp)
    ("M-s" splice-sexp)
    ("M-r" raise-sexp)
    ("M-;" comment-dwim)
    ("C-M-;" comment-sexp)
    ("C-M-q" indent-defun))))
