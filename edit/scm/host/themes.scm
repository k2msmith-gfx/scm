;## host/themes.scm — the standalone editor's colours: set-color, the named
;## themes, and M-x load-theme. Loaded after host/standalone.scm; the
;## bundled themes (edit/scm/themes/*.scm) after it, then the user's own
;## (~/.config/edit/themes/*.scm), then the theme last chosen with M-x
;## load-theme — all before ~/.config/edit/init.scm, so init has the last
;## word. The modeler has its own set-color over its own table (its
;## 04-theme.scm); the slot names are the same, so a theme file serves both.

; ── Slots ──────────────────────────────────────────────────────────────────
(define (set-color slot value)
  "Set a colour slot to a 0xRRGGBB value; #f for a name that is not a slot.
  Slots: :editor-bg :panel-bg :border-focused :status-bg :status-ink
  :text-ink :text-dim :text-accent :text-error :text-warning :popup-bg
  :menu-ink :menu-key-ink, and the syntax inks :code-comment :code-string
  :code-keyword :code-number :code-call :code-paren. :panel-bg is the REPL's
  and the file tree's background, :editor-bg the buffer's; :text-accent is
  the cursor block and a menu's selected row. Example: (set-color
  :code-string #x50fa7b)."
  (%set-color slot value))

(define (get-color slot)
  "A colour slot's current value as a number, or #f for a name that is not
  a slot."
  (%get-color slot))

; ── Named themes ───────────────────────────────────────────────────────────
; A theme is a name, a line saying what it is, and a slot → 0xRRGGBB table.
; Applying one starts from :one-dark, the shipped colours, so a theme that
; names only a few slots is those changes to the default.
(define *themes* '())             ; ((name doc . palette) …), newest first
(define *theme* :one-dark)        ; the one applied last
(define *theme-base* :one-dark)

(define (define-theme name doc palette)
  "Register a colour theme NAME (a keyword): DOC, one line saying what it
  is, and PALETTE, an alist of slot → 0xRRGGBB (see set-color). Defining a
  name again replaces it. A file in ~/.config/edit/themes/ that calls this
  adds a theme M-x load-theme offers. Returns NAME."
  (set! *themes* (cons (cons name (cons doc palette))
                       (remove (lambda (e) (eq? (car e) name)) *themes*)))
  name)

(define (theme-names)
  "Every registered theme's name, sorted."
  (list-sort (lambda (a b) (string<? (keyword->string a) (keyword->string b)))
             (map car *themes*)))

(define (apply-theme name)
  ; Paint NAME's palette over the base theme's; #f when there is no NAME.
  (define (paint n)
    (when-let ((e (assq n *themes*)))
      (for-each (lambda (slot) (set-color (car slot) (cdr slot))) (cddr e))))
  (and (assq name *themes*)
       (begin (paint *theme-base*)
              (paint name)
              (set! *theme* name)
              name)))

(define *theme-file* "~/.config/edit/theme.scm")

(define (save-theme-choice name)
  ; Remember NAME as the theme edit opens in.
  (guard (e (#t (echo "load-theme: could not save " *theme-file* ": " (*show-value* e))))
    (spit (expand-home *theme-file*)
          (string-append
            ";; Written by M-x load-theme: the theme edit opens in. Your init.scm\n"
            ";; runs after this file, so a (load-theme …) there wins.\n"
            "(load-theme " (*show-value* name) ")\n"))))

(define (load-theme (name #f))
  "(load-theme [NAME])  ->  the theme's name

  Apply the colour theme NAME (:one-dark, :doom-one, :dracula, :nano-light,
  :nano-dark, or one of yours from ~/.config/edit/themes/). As M-x, with no
  NAME, it prompts — TAB completes over (theme-names) — and REMEMBERS the
  choice in ~/.config/edit/theme.scm, so the next edit opens in it. Called
  with a NAME it applies without saving."
  (define (go n save?)
    (if (apply-theme n)
        (begin (when save? (save-theme-choice n))
               (echo "theme " (keyword->string n) " — " (cadr (assq n *themes*)))
               n)
        (begin (echo "no theme " (*show-value* n) " — M-x load-theme lists them")
               #f)))
  (if (given? name)
      (go (if (string? name) (string->keyword name) name) #f)
      (read-minibuffer "Theme: "
                       (lambda (s)
                         (let ((t (string-trim-both s)))
                           (unless (equal? t "")
                             (go (string->keyword (if (string-prefix? ":" t) (substring t 1 (string-length t)) t)) #t))))
                       (map keyword->string (theme-names)))))

(define (themes-boot)
  ; After the bundled themes: the user's own, then the saved choice. A
  ; broken file says so in the transcript and the rest still load.
  (define dir (expand-home "~/.config/edit/themes"))
  (define (try path)
    (guard (e (#t (*transcript-append* (string-append "theme " path ": " (if (string? e) e (*show-value* e))) :error)))
      (load path)))
  (for-each (lambda (n) (when (string-suffix? ".scm" n) (try (string-append dir "/" n))))
            (list-sort string<? (or (guard (_ (#t #f)) (%os-dir dir)) '())))
  (let ((saved (expand-home *theme-file*)))
    (when (file-exists? saved) (try saved))))
