;## 06-help.scm — the help screens: every key binding as a list (show-keys,
;## ? / f1) and as a document (view-keybindings, C-h b), what one key runs
;## (describe-key, C-h k), the key beside a menu item, and the view-only
;## markdown buffer the documents open in (docs/editor-crate-plan.md, E3h —
;## Kevin, 2026-11-13: "I want to also preserve the help screens for keymaps
;## etc. in the editor"). Moved from the modeler's 13-outliner and unify.scm.

; What the HOST supplies, as FORWARD HOOKS — the modeler binds each (unify):
;   *help-open*          reveal where show-keys prints (the REPL's dock); the
;                        lines themselves go through *transcript-append*
;                        (07-backend), as a send's do
;   *show-keys-extra*    sections that are not bindings: ((heading (k what) …) …)
;   *keybindings-words*  the C-h b page's title, and what it says about chords
;                        and bare keys: an alist of :title :chords
;                        :bare-heading :bare
;   *editor-shown?*      is the editor on screen (so quitting a popup that
;                        opened it may hide it again)
(define *help-open* (lambda () #f))
(define *show-keys-extra* (lambda () '()))
(define *keybindings-words* (lambda () '()))
(define *editor-shown?* (lambda () #t))

(define (fn-label f)
  "A binding's display name: a symbol's name, the procedure's name inside
  #<procedure ...>, or the raw printed value for an anonymous one."
  (cond ((symbol? f) (symbol->string f))
        ((string? f) f)
        ((hash-table-ref/default *binding-labels* f #f) => fn-label)
        (else
         (let ((st (format "~a" f)))
           (cond ((string-prefix? "#<procedure " st) (substring st 12 (- (string-length st) 1)))
                 ((string-prefix? "#<primitive " st) (substring st 12 (- (string-length st) 1)))
                 (else st))))))

(define (keys-into m prefix)
  "M's bindings as (key label) pairs, sorted; a prefix map's spelled out."
  (append-map
    (lambda (k)
      (let ((v (hash-table-ref/default m k #f)))
        (if (hash-table? v)
            (keys-into v (string-append prefix k " "))
            (list (list (string-append prefix k) (fn-label v))))))
    (list-sort string<? (hash-table-keys m))))

(define (%key-column k) (string-pad-right k (max 10 (string-length k))))

(define (show-keys)
  "List every key binding in the REPL transcript, aligned and grouped —
  bound to ? (and f1). Pane-local maps (the REPL's editing keys) are
  grouped under their pane."
  (*help-open*)
  (*transcript-append* "── key bindings ──────────────────────────" :accent)
  (let ()
    (for-each (lambda (b) (*transcript-append* (string-append "  " (%key-column (car b)) " " (cadr b))))
              (keys-into *keymap* ""))
    (hash-table-walk *pane-keymaps*
      (lambda (id m)
        (unless (zero? (hash-table-size m))
          (*transcript-append* (string-append "── pane " (%text (*pane-key* id)) " ──") :accent)
          (for-each
            (lambda (b)
              (let* ((k (car b))
                     (label (key-label k (cadr b))))
                (*transcript-append* (string-append "  " (%key-column k) " " label) :dim)))
            (keys-into m "")))))
    ; What the HOST adds that is not a binding — the modeler's mouse
    ; gestures (*show-keys-extra*): (heading (key what) …) sections.
    (for-each
      (lambda (sec)
        (*transcript-append* (car sec) :accent)
        (for-each
          (lambda (g) (*transcript-append* (string-append "  " (%key-column (car g)) " " (cadr g)) :dim))
          (cdr sec)))
      (*show-keys-extra*))
    (*transcript-append* "(bind-key \"key\" fn) rebinds — live." :dim)
    #f))

(bind-key "S-/" 'show-keys)
; the ? key
(bind-key "f1" 'show-keys)

; ── Key-binding browser (markdown) ──────────────────────────────────────────
; (view-keybindings) / C-h b renders the LIVE keymap as a markdown document and
; opens it in a view-only buffer — the reading view (default) shows it pretty,
; C-c C-v flips to source. Reuses the outliner's keys-into/fn-label and
; the shell's *editor-key-labels* (all one shared env, loaded before this).

(define (%doc-of name)
  "NAME's docstring (a string naming a binding), or #f — unbound, or bound
  with no doc."
  (let ((d (guard (e (#t #f)) (doc (string->symbol name)))))
    (and (string? d) d)))

(define (key-for-command name)
  "The shortest key bound to the command called NAME — in the global
  keymap or a pane's — or #f. A focus-guarded key counts by its label
  (*editor-key-labels*), the name C-h b shows beside it; a key bound to the
  command itself counts by its own name too, since the label table is keyed
  by key alone and one pane's label for C-l would hide another pane's
  clear-repl."
  (define (keys-of m)
    (filter-map (lambda (row)
                  (match row
                    ((k raw) (and (or (equal? raw name) (equal? (key-label k raw) name)) k))))
                (keys-into m "")))
  (let ((keys (append (keys-of *keymap*)
                      (append-map (lambda (id) (keys-of (hash-table-ref *pane-keymaps* id)))
                                  (hash-table-keys *pane-keymaps*)))))
    (and (pair? keys)
         (fold (lambda (k best) (if (< (string-length k) (string-length best)) k best))
               (car keys) (cdr keys)))))

;An unnamed procedure prints as #<procedure>, which every other unnamed
;one matches; such an item names its key itself or shows none.
(set! *menu-key-hint*
  (lambda (action)
    (let ((name (fn-label action)))
      (and (not (string-prefix? "#<" name)) (key-for-command name)))))

(define (key-label k name)
  "A binding's display name: NAME, the fn-label keys-into found, when it
  names something; for an anonymous one, a registered *editor-key-labels*
  override for key K (string or fn). Keyed by key alone, the override is
  for global keys: a pane's key names itself through `labelled`, so the
  minibuffer's label for C-a no longer names the editor's."
  (let ((lf (and (string-prefix? "#<" name) (hash-table-ref/default *editor-key-labels* k #f))))
    (cond ((not lf) name)
          ((string? lf) lf)
          (else (fn-label lf)))))

(define (key-base k)
  "Strip a descriptor's modifier prefixes and return (mods base) — so
  \"C-S-=\" gives ((\"C-\" \"S-\") \"=\") and \"w\" gives (() \"w\"). Prefixes are the
  fixed set the kernel emits (06-keymap): C- M- S- s-."
  (let loop ((rest k) (mods '()))
    (let ((m (and (> (string-length rest) 2)
                  (find (lambda (m) (string-prefix? m rest)) '("C-" "M-" "S-" "s-")))))
      (if m
        (loop (substring rest (string-length m) (string-length rest)) (cons m mods))
        (list (reverse mods) rest)))))

(define (viewport-key? k)
  "Whether a GLOBAL binding can only fire with a 3D pane focused.

  Not a taste call — it follows from dispatch. The kernel routes printable
  keys to (on-text) whenever a typing surface holds focus, so they only
  reach the keymap when none does, i.e. in the viewport. A chord (C-/M-/s-)
  reaches the keymap either way and really is global. Shift alone doesn't
  change that: S-b is \"B\", still printable, still swallowed while typing.

  Multi-key sequences (\"C-x C-f\") are chords by construction."
  (and
    (not (string-contains k " "))
    (let* ((kb (key-base k)) (mods (car kb)) (base (cadr kb)))
      (and
        (not (any (lambda (m) (member m '("C-" "M-" "s-"))) mods))
        (or (= 1 (string-length base)) (equal? base "space"))))))

(define (doc-summary name)
  "NAME's docstring summary for a one-line listing, or #f: its first
  paragraph on one line — the commands write it as what they do and what
  they run — or its first sentence when that paragraph is long."
  (and-let* ((d (%doc-of name)))
    (let* ((para (let ((i (string-contains d "\n\n"))) (if i (substring d 0 i) d)))
           (flat (string-join (map string-trim-both (string-split para "\n")) " ")))
      (if (<= (string-length flat) 220)
        flat
        (let ((i (string-contains flat ". ")))
          (if (and i (< i 220)) (substring flat 0 (+ i 1)) (string-append (substring flat 0 217) "...")))))))

(define *describe-key* #f)   ; after C-h k: (keys-so-far prefix-map-or-#f)

(define (describe-key)
  "Show what a key does — C-h k, then the key (a sequence such as C-x C-f
  too). Runs nothing: the next key is described, not run, and the page shows
  the command it runs and that command's whole docstring.

  C-g cancels. Bare keys (w, d, 1) reach it from the viewport; while you
  are typing in a pane they insert text, so describe them from the
  viewport, or read C-h b."
  (set! *describe-key* (list "" #f))
  (echo "describe key: press a key (C-g cancels)"))

(define (describe-key-page keys b)
  ; The C-h k page for KEYS, bound to B.
  (let* ((name (key-label keys (fn-label b)))
         (d (%doc-of name)))
    (string-append
      "# `" keys "`\n\nRuns **`" name "`**.\n\n"
      (if d
        (string-append "```\n" d "\n```\n")
        "It has no docstring.\n"))))

; Ahead of the keymap (10), menus and tools: while a C-h k is waiting, the
; next key is DESCRIBED, never run. A prefix (C-x) waits for the rest.
(add-handler
  *key-handlers*
  90
  :describe-key
  (lambda (key)
    (match *describe-key*
      ((so-far prefix)
        (let* ((keys (if (string=? so-far "") key (string-append so-far " " key)))
               (local (hash-table-ref/default *pane-keymaps* (%focused-pane) #f))
               (b (if prefix
                    (hash-table-ref/default prefix key #f)
                    (or (and local (hash-table-ref/default local key #f))
                        (hash-table-ref/default *keymap* key #f)))))
          (cond ((equal? key "C-g") (set! *describe-key* #f) (echo "describe key: cancelled"))
                ((hash-table? b) (set! *describe-key* (list keys b)) (echo "describe key: " keys " …"))
                (b (set! *describe-key* #f)
                   (show-markdown-doc "*describe-key*" (describe-key-page keys b))
                   (echo keys " runs " (key-label keys (fn-label b))))
                (else (set! *describe-key* #f) (echo keys " is not bound")))
          #t))
      (_ #f))))

(define (keybindings-markdown)
  "The whole keymap (global + viewport + per-pane) as a markdown document."
  (define out (open-output-string))
  (define (emit . parts) (for-each (lambda (p) (display (%describe-part p) out)) parts))
  (define (rows acc)
    (for-each
      (lambda (row)
        (let* ((name (key-label (car row) (cadr row)))
               (what (doc-summary name)))
          ; An unnamed procedure prints #<procedure>, and the reading view
          ; takes <procedure> for a tag and shows a bare # (Kevin,
          ; 2026-11-14): say so in words instead.
          (if (string-prefix? "#<" name)
            (emit "- `" (car row) "` — *an unnamed procedure*\n")
            (emit "- `" (car row) "` — **" name "**" (if what (string-append " — " what) "") "\n"))))
      acc))
  ; keys-into ANSWERS the (key label) pairs since M5b (13-outliner).
  (define acc (keys-into *keymap* ""))
  (define words (*keybindings-words*))
  (emit "# " (%alist-ref words :title "Key bindings") "\n\n")
  (emit "Live keymap — regenerated each call. `(bind-key \"key\" fn)` rebinds.\n")
  (emit "Each line says what the command does and what it RUNS; `C-h k` then a key\n")
  (emit "shows the command's whole docstring.\n\n")
  (emit "## Global\n\n")
  (emit (%alist-ref words :chords "Chords — these reach the keymap whatever holds focus.") "\n\n")
  (rows (filter (lambda (row) (not (viewport-key? (car row)))) acc))
  (emit "\n## " (%alist-ref words :bare-heading "Bare keys") "\n\n")
  (emit (%alist-ref words :bare "Bare keys — they only reach the keymap when NO typing surface has\nfocus. Type in a pane and the same keys insert text instead.") "\n\n")
  (rows (filter (lambda (row) (viewport-key? (car row))) acc))
  (for-each
    (lambda (id)
      (let ((m (hash-table-ref *pane-keymaps* id)))
        (unless (null? (hash-table-keys m))
          (emit "\n## " (%text (*pane-key* id)) " pane\n\n")
          (rows (keys-into m "")))))
    (hash-table-keys *pane-keymaps*))
  (get-output-string out))

(define (show-markdown-doc name md)
  "Open MD (a markdown string) in a reused, view-only editor buffer called
  NAME, flipped into the reading view. Shared by the keybinding browser and
  (help). Re-invoking with the same NAME regenerates in place."
  (define was-open (*editor-shown?*))
  (*show-editor*)
  (let ((eid (editor-pane-id))
        (buf (buffer-for-path name)))
    (if buf
      (begin (visit-buffer buf) (%editor-open-markdown-text eid md))
      (let ((cur (%editor-active-buffer eid)))
        (unless (and (not (buffer-path cur)) (not (positive? (%editor-dirty? eid))))
          (%editor-buffer-new eid))
        (%editor-open-markdown-text eid md)
        (register-buffer (%editor-active-buffer eid) name)
        (visit-buffer (%editor-active-buffer eid))))
    ; If showing this popup is what opened the dock, remember it on the buffer
    ; so 'q' (editor-quit-view) can close the dock again when quitting.
    (unless was-open (%buffer-set-auto-opened eid (%editor-active-buffer eid) 1))
    ; A fresh markdown buffer opens in source view; flip once into reading view.
    (%editor-toggle-markdown eid)
    eid))

(define (view-keybindings)
  "Show every key binding as a markdown document in the editor (view-only;
  opens in the reading view, C-c C-v flips to source). Regenerated live — C-h b."
  (show-markdown-doc "*keybindings*" (keybindings-markdown))
  (echo "key bindings — C-c C-v toggles source view"))
(bind-key "C-h b" 'view-keybindings)
(bind-key "C-h k" 'describe-key)
