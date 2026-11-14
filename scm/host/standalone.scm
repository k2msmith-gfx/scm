;## host/standalone.scm — the standalone editor's side of the shell's hooks
;## (docs/editor-crate-plan.md, E4), loaded after the shell and before the
;## user's ~/.config/edit/init.scm. Three panes, fixed: the editor, a
;## transcript under it, the minibuffer at the bottom (src/app.rs). What the
;## modeler's 05-panes, 08-editor and unify do for the shell, for these.

; ── The panes, by key ──────────────────────────────────────────────────────
(set! *pane-id*
  (lambda (key)
    (case key
      ((:editor) 1)
      ((:transcript :repl) 2)
      ((:minibuffer) 3)
      (else #f))))
(set! *pane-key*
  (lambda (id)
    (case id
      ((1) :editor)
      ((2) :transcript)
      ((3) :minibuffer)
      (else #f))))

; ── Where a send's answers go: the transcript ─────────────────────────────
(set! *transcript-append*
  (lambda (text (style #f))
    (%pane-append 2 (%text (or style :normal)) (%describe-part text))))

; A send goes to this image for a Scheme buffer — a scratch, or a file named
; as Scheme — and nowhere for anything else (the modeler's rule, by name).
(set! *buffer-backend*
  (lambda (buf)
    (let ((p (buffer-path buf)))
      (and (or (not p)
               (string-prefix? "*" p)
               (any (lambda (ext) (string-suffix? ext p)) '(".scm" ".ss" ".sld" ".sls")))
           (in-process-backend)))))

; ── Files ──────────────────────────────────────────────────────────────────
(define *last-open-path* #f)

(define (find-file)
  "Open a file in a buffer — C-x C-f. Runs nothing. Markdown opens in the
  reading view and a .png/.jpg as a picture; anything else is text. TAB
  completes the path."
  (read-minibuffer
    "Find file: "
    (lambda (p)
      (set! *last-open-path* p)
      (let ((eid (editor-pane-id))
            (low (string-downcase p)))
        (define (viewer open)
          (%editor-buffer-new eid)
          (open eid p)
          (register-buffer (%editor-active-buffer eid) p)
          (visit-buffer (%editor-active-buffer eid)))
        (cond ((string-suffix? ".md" low) (viewer %editor-open-markdown))
              ((any (lambda (e) (string-suffix? e low)) '(".png" ".jpg" ".jpeg")) (viewer %editor-open-image))
              (else (visit-file-text p)))))
    :files
    *last-open-path*))

; ── Keys ───────────────────────────────────────────────────────────────────
(bind-key "C-x C-f" 'find-file)
(bind-editor-pane-keys)
(bind-editor-completion-keys)
(bind-editor-keys :minibuffer)
(bind-key "C-x C-c" (lambda () (%quit)))

; ── The mouse's floor: a transcript scrolls ────────────────────────────────
(add-handler
  *wheel-handlers*
  10
  :transcript-scroll
  (lambda (d x y (alt #f) (shift #f))
    (let ((hit (and x y (%pane-at x y))))
      (and hit (equal? (cadr hit) "text")
           (begin (%pane-scroll (car hit) (* 3 (if (positive? d) 1 -1))) #t)))))

(define (edit-greeting)
  "What the transcript says when the editor opens."
  (*transcript-append* "edit — Scheme in this image. C-x C-f opens a file, C-x C-s saves," :dim)
  (*transcript-append* "C-M-x / C-x C-e / C-c C-b send, C-h b lists every key, C-x C-c quits." :dim))
