;## 10-xref.scm — where a name is defined, and M-. / M-, / C-M-. to go
;## there and back (docs/editor-crate-plan.md, E3e-4 — moved from the
;## modeler's unify.scm, which keeps which files the index holds:
;## xref-reindex, rebuilt from the scene's files and the preludes).

;; ── Where a name is defined (docs/modeler-scene-buffers-plan.md, B3) ──────
;; The index (xref.rs, over scm-text) holds the loaded scene's files and the
;; preludes' sources — not the project: in a tree of two hundred example
;; scenes a name like `slab` is defined many times, and the right one is the
;; one this scene includes. Rebuilt on every load (note-scene-files!), a file
;; re-indexed when it is saved, and the buffer on screen re-indexed from its
;; unsaved text before a lookup, since an open buffer is the truth about
;; itself. A name defined at the REPL, or a Rust verb, has no site.

; How a jump opens a file, as a FORWARD HOOK: as TEXT, in its own buffer,
; nothing run — the modeler's visit-scene-text also says what the file is to
; the loaded scene.
(define (visit-file-text p)
  "Open P as text in a buffer — its own, if it has one, else a new one
(reusing a clean, unnamed scratch). Runs nothing."
  (*show-editor*)
  (if-let ((buf (buffer-for-file p)))
    (visit-buffer buf)
    (let* ((eid (editor-pane-id))
           (cur (%editor-active-buffer eid)))
      (unless (and (not (buffer-path cur)) (not (positive? (%editor-dirty? eid))))
        (%editor-buffer-new eid))
      (register-buffer (%editor-active-buffer eid) p)
      (%editor-set-text eid (if (file-exists? p) (slurp p) ""))
      (%editor-mark-saved eid)
      (visit-buffer (%editor-active-buffer eid)))))
(define *visit-file* visit-file-text)

(define (xref-note-buffer!)
  ; The buffer on screen, from its text as it stands.
  (define eid (editor-pane-id))
  (when-let ((path (buffer-path (%editor-active-buffer eid))))
    (when (string-suffix? ".scm" path)
      (%xref-update (real-file path) (%text (%editor-text eid))))))

(define (definition-sites name)
  "Every place NAME (a symbol or a string) is defined, as a list of
  (path line column form) — the line and column 0-based, the column in
  characters, FORM the defining word (\"define\", \"define-syntax\", …).
  Searched: the loaded scene's files (see (scene-files)) and ray's and the
  modeler's Scheme preludes, when the source tree is present; the buffer on
  screen counts as it stands, saved or not. Empty for a name defined at the
  REPL or built into ray in Rust — (doc NAME) answers those.

Example: (definition-sites 'add-roof)  ; → ((\"/…/levels.scm\" 41 9 \"define\"))"
  (xref-note-buffer!)
  (%xref-sites (if (symbol? name) (symbol->string name) name)))

(define (find-definitions text (limit 50))
  "Names defined in the same files (definition-sites) searches whose name
  contains TEXT, caselessly, shortest first, at most LIMIT: a list of
  (name path line column form). An empty TEXT lists them all, to LIMIT.

Example: (find-definitions \"stair\")"
  (xref-note-buffer!)
  (%xref-matching text limit))

;; ── M-. and M-, (B4) ───────────────────────────────────────────────────────
;; Emacs's xref keys over (definition-sites). A jump VISITS the file — as
;; text, never as a scene: a definition in another scene's file must not
;; replace the one you are looking at, which is what B2 was for. Each jump
;; pushes where it left from, and M-, walks back.

(define *definition-marks* '())   ; ((buf line index) …), newest first

(define (goto-site site)
  ; Visit SITE's file as text and put point on the name.
  (match site
    ((path line column . _)
      (*visit-file* path)
      (let ((eid (editor-pane-id)))
        (%editor-goto-line eid line)
        (let loop ((n column))
          (when (positive? n) (%editor-act eid "right") (loop (- n 1)))))
      (echo (buffer-basename path) ":" (number->string (+ line 1))))
    (_ (echo "no such definition"))))

(define (jump-to-definition name)
  "Visit where NAME is defined, remembering where you were for M-,. One
  site jumps; several ask which, by number; none shows NAME's doc when it
  is bound — a Rust verb or a REPL definition has no file — and says so."
  (define eid (editor-pane-id))
  (define (go site)
    (match (%editor-cursor eid)
      ((line index) (set! *definition-marks*
                      (cons (list (%editor-active-buffer eid) line index) *definition-marks*)))
      (_ #f))
    (goto-site site))
  (match (definition-sites name)
    (()
      (if (%env-ref (string->symbol name) #f)
        (begin
          (describe-name name)
          (echo name " has no source in this scene's files or the preludes — "
                "built into ray, or defined at the REPL; its doc is in the REPL"))
        (echo "no definition of " name " in this scene's files or the preludes")))
    ((site) (go site))
    (sites
      (read-minibuffer
        (string-append
          (number->string (length sites)) " definitions of " name " — "
          (string-join
            (let number ((ss sites) (i 1))
              (match ss
                (() '())
                (((path line . _) . rest)
                  (cons (string-append (number->string i) ") " (buffer-basename path) ":"
                                       (number->string (+ line 1)))
                        (number rest (+ i 1))))))
            "  ")
          "  which? ")
        (lambda (ans)
          (let ((i (string->number (string-trim-both ans))))
            (if (and i (exact-integer? i) (<= 1 i (length sites)))
              (go (list-ref sites (- i 1)))
              (echo "no definition " ans " — pick 1 to " (number->string (length sites))))))))))

(define (find-definition-at-point)
  "Jump to the definition of the symbol under the cursor — M-.. Runs
  nothing: the file it is in opens as text. M-, comes back.

  Emacs's xref-find-definitions. Searches the loaded scene's files and the preludes
  (see (definition-sites)), the buffer on screen as it stands; the file opens
  as TEXT, so a definition in another scene's file never loads that scene."
  (define tp (editor-text+point))
  (define name (editor-token-around (car tp) (cadr tp)))
  (if (string=? name "")
    (echo "no symbol at point — C-M-. asks for a name")
    (jump-to-definition name)))

(define (find-definition)
  "Prompt for a name and jump to its definition — C-M-.. Runs nothing: the
  file it is in opens as text. M-, comes back.

  TAB completes over every name defined in the loaded scene's files and the
  preludes; the spot Emacs gives xref-find-apropos."
  (read-minibuffer
    "Find definition: "
    (lambda (n)
      (let ((name (string-trim-both n)))
        (unless (string=? name "") (jump-to-definition name))))
    :definitions))

(define (go-back-from-definition)
  "Go back to where the last M-. or C-M-. jumped from — M-,. Runs nothing.

  Repeats walk
  further back; a buffer killed since is skipped with a message."
  (define eid (editor-pane-id))
  (match *definition-marks*
    (() (echo "no earlier position — M-. leaves one"))
    (((buf line index) . rest)
      (set! *definition-marks* rest)
      (if (assoc buf (%editor-buffer-list eid))
        (begin
          (visit-buffer buf)
          (%editor-goto-line eid line)
          (let loop ((n index))
            (when (positive? n) (%editor-act eid "right") (loop (- n 1))))
          (echo "back to " (%text (buffer-name buf)) ":" (number->string (+ line 1))))
        (echo "that buffer was killed — M-, again goes further back")))))

(bind-key "C-M-." 'find-definition)
