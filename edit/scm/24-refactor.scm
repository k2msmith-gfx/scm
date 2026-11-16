;## 24-refactor.scm — macroexpand at the point, rename, and saving every
;## changed buffer (Kevin, 2026-11-16). The kernel finds the form and the
;## places (src/sexp.rs, over scm-text's scope pass); the expansion is
;## asked of the buffer's own backend — this image, the file's scm, ray or
;## Guile — the one C-x C-e sends to.

; ── Macroexpand ────────────────────────────────────────────────────────────
(define (expand-at-point proc)
  ; Ask the buffer's backend for (PROC 'FORM) of the form at the point and
  ; print the answer, laid out, in the REPL.
  (define eid (editor-pane-id))
  (define form (%editor-sexp eid "form-text"))
  (define backend (backend-of (%editor-active-buffer eid)))
  (define (show r)
    (if (eq? (%alist-ref r :status #f) 'ok)
        (let ((printed (%alist-ref r :printed "")))
          (*before-run-file*)
          (*transcript-append* (string-append ";; " proc ": " (send-shown form)) :accent)
          (for-each (lambda (line) (*transcript-append* line))
                    (string-split (%pretty printed 72) "\n"))
          (echo (if (equal? (%pretty printed 1000) (%pretty form 1000))
                    "no macro to expand here — the form is its own expansion"
                    (string-append proc " — in the REPL"))))
        (echo proc ": " (%alist-ref r :message ""))))
  (cond ((eqv? form 0) (echo "not a Scheme or Guile buffer"))
        ((not form) (echo "no form at the point"))
        ((not backend) (echo "this buffer sends nowhere"))
        (else
          ; Guile's macroexpand answers Tree-IL, and expands everything at
          ; once (it has no macroexpand-1): turned back into Scheme to show.
          (let* ((guile? (string-prefix? "guile" (%alist-ref backend :name "")))
                 (src (if guile?
                          (string-append "((@ (language tree-il) tree-il->scheme) (macroexpand '" form "))")
                          (string-append "(" proc " '" form ")"))))
            (when (and guile? (equal? proc "macroexpand-1"))
              (set! proc "macroexpand (Guile expands all at once)"))
            (cond ((backend-op backend :eval-async) => (lambda (ask) (ask src show)))
                  (else (show ((backend-op backend :eval) src))))))))

(define (macroexpand-1-at-point)
  "Expand the macro call at the point ONCE — the list the point is on, or
  the one around it — in the buffer's own image, and print the expansion,
  laid out, in the REPL (C-c C-m, SLIME's key)."
  (expand-at-point "macroexpand-1"))

(define (macroexpand-at-point)
  "Expand the macro call at the point until its head is no longer a macro
  (C-c M-m). The forms inside are left as written, as Lisp's macroexpand
  leaves them."
  (expand-at-point "macroexpand"))

; ── Rename ─────────────────────────────────────────────────────────────────
(define (valid-name? s)
  (and (not (string=? s ""))
       (not (any (lambda (c) (or (char-whitespace? c) (memv c '(#\( #\) #\[ #\] #\{ #\} #\" #\; #\' #\` #\,))))
                 (string->list s)))))

(define (rename-symbol)
  "Rename the name at the point (C-c r). A LOCAL — a parameter, a let's
  name — changes only inside the form that binds it, and not where an inner
  binding shadows it. A TOP-LEVEL name changes at its definitions and every
  free use: in this buffer, and in every file of the checkout M-? finds it
  in — each opened as a buffer, the rename one undo step there, and left
  UNSAVED for you to look over (C-x s saves them all). Quoted symbols and
  strings are never touched."
  (define eid (editor-pane-id))
  (match (%editor-rename-info eid)
    ((name global n)
      (read-minibuffer
        (string-append "Rename " name " (" (number->string n) (if (= n 1) " place" " places")
                       (if (positive? global) ", top-level" ", local") ") to: ")
        (lambda (new)
          (let ((new (string-trim-both new)))
            (cond ((or (string=? new "") (string=? new name)) (echo "rename: nothing to do"))
                  ((not (valid-name? new)) (echo "rename: " new " is not a name"))
                  ((positive? global) (rename-top-level name new))
                  (else
                    (let ((k (%editor-rename eid new)))
                      (echo "renamed " name " → " new " in " (number->string k) " places"))))))
        :plain name))
    (_ (echo "no name at the point to rename"))))

(define (rename-top-level name new)
  ; This buffer, then — after a yes — each other file the index finds NAME
  ; in, opened and renamed in its own buffer, unsaved.
  (define eid (editor-pane-id))
  (define here (%editor-active-buffer eid))
  (define here-file (real-file (buffer-path here)))
  (define others
    (delete-duplicates
      (filter (lambda (f) (and f (not (equal? f here-file))))
              (map (lambda (site) (real-file (car site)))
                   (append (%xref-references name) (%xref-sites name))))))
  (define (finish k others-renamed)
    (echo "renamed " name " → " new ": " (number->string k) " here"
          (if (null? others-renamed) ""
              (string-append ", " (number->string (apply + (map cdr others-renamed))) " in "
                             (number->string (length others-renamed)) " other files — unsaved, C-x s saves them"))))
  (define (rename-others)
    (let ((done (filter-map (lambda (f)
                              (*visit-file* f)
                              (let ((k (%editor-rename-global eid name new)))
                                (and (positive? k) (cons f k))))
                            others)))
      (visit-buffer here)
      done))
  (if (null? others)
      (finish (%editor-rename eid new) '())
      (read-minibuffer
        (string-append name " is also used in " (number->string (length others))
                       (if (= 1 (length others)) " other file" " other files") " — rename there too? (y or n) ")
        (lambda (answer)
          (let ((k (%editor-rename eid new)))
            (finish k (if (string-prefix? "y" (string-downcase (string-trim-both answer))) (rename-others) '()))))
        :plain)))

; ── Saving ─────────────────────────────────────────────────────────────────
(define (save-some-buffers)
  "Save every buffer that visits a file and has unsaved changes (C-x s) —
  after a rename that reached other files, say. Says which."
  (define saved
    (filter-map (lambda (row)
                  (match row
                    ((buf dirty)
                      (and (positive? dirty) (buffer-path buf) (*writable-path?* (buffer-path buf))
                           (begin (save-buffer-text! buf) (buffer-name buf))))
                    (_ #f)))
                (%editor-buffer-list (editor-pane-id))))
  (echo (if (null? saved) "no changed buffers to save"
            (string-append "saved " (string-join saved ", ")))))

; C-c and C-x are prefixes, so these bind globally with a focus guard, as
; the sends do (08-sends): a pane-local C-c map would shadow the global one.
(bind-key "C-c C-m" (labelled "macroexpand-1-at-point" (lambda () (if (editor-focused?) (macroexpand-1-at-point) #f))))
(bind-key "C-c M-m" (labelled "macroexpand-at-point" (lambda () (if (editor-focused?) (macroexpand-at-point) #f))))
(bind-key "C-c r" (labelled "rename-symbol" (lambda () (if (editor-focused?) (rename-symbol) #f))))
(bind-key "C-x s" (labelled "save-some-buffers" (lambda () (if (editor-focused?) (save-some-buffers) #f))))
