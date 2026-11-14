;## 02-keymap.scm — key bindings, Emacs-shaped: global and pane-local maps,
;## prefix sequences, and the dispatch at the floor of the key chain
;## (docs/editor-crate-plan.md, E3a — moved from the modeler's 06-keymap,
;## whose default bindings stayed there).

; Which pane a KEY names, and back — the host's registry, reached through two
; FORWARD HOOKS so the keymap does not name it (the modeler's 05-panes binds
; them to *panes* and pane-key).
(define *pane-id* (lambda (key) #f))
(define *pane-key* (lambda (id) #f))

; The command echo, a forward hook too: the REPL section binds it to log each
; dispatched command when *echo-commands* is on.
(define *command-echo* #f)
; bound in the REPL section: logs each dispatched

; ── Keymap ──────────────────────────────────────────────────────────────────
; The Emacs half of keyboard input: the kernel normalizes every key press to
; a descriptor string and calls (on-key desc); these tables decide what runs.
; Descriptors: the bare key ("w", "space", "f5", "up") behind modifier
; prefixes in fixed order — C- control, M- option/alt, S- shift, s- command.
; So option-R is "M-r", shift-space "S-space". Escape is the kernel's hard
; quit and never reaches the keymap.
; Since M4 keymaps are pane-local first: the focused pane's map sees the key,
; then the global map. Global bindings live in *keymap*; per-pane maps are
; created on demand by (bind-key key f :pane-key).

(define *keymap* (make-hash-table equal?))

;Per-pane keymaps, keyed by pane id: the focused pane's map shadows the
;global *keymap*.
(define *pane-keymaps* (make-hash-table equal?))

(define (keymap-for pane)
  "The map bind-key writes into: global, or a pane's local map (created on
  demand)."
  (if pane
      (let ((id (*pane-id* pane)))
        (and id
             (or (hash-table-ref/default *pane-keymaps* id #f)
                 (let ((m (make-hash-table equal?)))
                   (hash-table-set! *pane-keymaps* id m)
                   m))))
      *keymap*))

(define (bind-key keys f (pane #f))
  "Bind a key descriptor (e.g. \"w\", \"M-r\", \"S-f\", \"f5\") — or a
  SEQUENCE like \"C-x C-f\" (space-separated; each prefix key becomes a
  nested keymap, Emacs-style) — to a zero-arg function OR a SYMBOL naming
  one. Prefer the symbol ('render): it is resolved to the CURRENT definition
  at press time, so redefining the verb updates the key automatically — a
  function value is frozen as bound (fine for lambdas). Globally, or in one
  pane's local keymap given its *panes* key: (bind-key \"x\" 'f :persp).
  Rebinding replaces; binding through a prefix replaces a plain binding on
  that prefix key. See describe-keys."
  (let ((m (keymap-for pane)))
    (if m
        ;; Every key but the last names a prefix map, made on the way down.
        (let loop ((target m) (parts (string-split keys " ")))
          (if (null? (cdr parts))
              (hash-table-set! target (car parts) f)
              (let ((sub (hash-table-ref/default target (car parts) #f)))
                (if (hash-table? sub)
                    (loop sub (cdr parts))
                    (let ((t (make-hash-table equal?)))
                      (hash-table-set! target (car parts) t)
                      (loop t (cdr parts)))))))
        (begin (display (format "unknown pane ~s — see describe-panes" pane)) (newline))))
  keys)

(define (unbind-key key (pane #f))
  "Remove a binding — global, or pane-local given a *panes* key."
  ;; DELETED, not set to #f: a key bound to nothing is not bound, and
  ;; describe-keys listed the #f ones as "key — " with nothing after it.
  (if pane
      (let* ((id (*pane-id* pane))
             (m (and id (hash-table-ref/default *pane-keymaps* id #f))))
        (when m (hash-table-delete! m key)))
      (hash-table-delete! *keymap* key))
  key)

(define (describe-map m prefix indent)
  "M's bindings as lines, sorted by key; a prefix map's spelled out under it."
  (append-map
    (lambda (key)
      (let ((v (hash-table-ref/default m key #f)))
        (if (hash-table? v)
            (describe-map v (string-append prefix key " ") indent)
            (list (string-append indent prefix key " — " (format "~a" v) "\n")))))
    (list-sort string<? (hash-table-keys m))))

(define (describe-keys)
  "The current key bindings, one 'key — function' line each (prefix
  sequences spelled out); pane-local bindings are listed under their pane."
  (apply string-append
    (append
      (describe-map *keymap* "" "")
      (append-map
        (lambda (id)
          (let ((m (hash-table-ref/default *pane-keymaps* id #f)))
            (if (zero? (hash-table-size m))
                '()
                (cons (string-append "pane " (%key-string (or (*pane-key* id) "")) ":\n")
                      (describe-map m "" "  ")))))
        (hash-table-keys *pane-keymaps*)))))

(define (chorded? key)
  "True for descriptors carrying a modifier prefix (C-/M-/S-/s-)."
  (or (string-prefix? "C-" key)
      (string-prefix? "M-" key)
      (string-prefix? "S-" key)
      (string-prefix? "s-" key)))

;An armed prefix key's sub-keymap (e.g. after C-x), consulted for
;exactly the next press; any miss disarms it.
(define *pending-keymap* #f)

(define (key-target b)
  "Resolve a stored binding to what to RUN. A SYMBOL binding is looked up
  in the live environment at press time — redefining a verb updates every
  key bound to it, no manual rebind (review item 7: bind-key used to freeze
  the function VALUE, so each redefinition needed a hand-written rebind or
  the key ran stale code). A function binding (a guarded lambda) runs as
  stored. nil when a symbol no longer resolves — the key reports unbound
  rather than crashing."
  (if (symbol? b) (%env-ref b #f) b))

; command's form when *echo-commands* is on

; Keymap dispatch — the FLOOR of the key chain (menus/tools claim above it
; via *key-handlers*). An armed prefix map is consulted first, then the
; focused pane's local map, then the global *keymap*. A binding that is
; itself a keymap (a table — what (bind-key "C-x C-f" f) creates) arms as
; the prefix for the next key. While a typing surface holds focus
; (%typing-pane), bare keys that miss the local map are swallowed — they
; belong to text entry — but chorded keys still reach the global map, so
; M-j and C-x C-f work mid-typing.
(add-handler
  *key-handlers*
  10
  :keymap
  (lambda (key)
    (define pending *pending-keymap*)
    (set! *pending-keymap* #f)
    (let* ((local (hash-table-ref/default *pane-keymaps* (%focused-pane) #f))
           (f (if pending
                  (hash-table-ref/default pending key #f)
                  (or (and local (hash-table-ref/default local key #f))
                      (and (or (not (%typing-pane)) (chorded? key))
                           (hash-table-ref/default *keymap* key #f))))))
      (cond ((hash-table? f) (set! *pending-keymap* f) f)
            (f
             (let ((target (key-target f)))
               (and target
                    (begin
                      (when *command-echo* (*command-echo* target))
                      (run-command target)))))
            (else #f)))))
