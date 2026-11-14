;## 01-handlers.scm — the input chains every key, click and wheel goes
;## through, and `run-command`, which every command a key or a menu reaches
;## runs under (docs/editor-crate-plan.md, E3a — moved from the modeler's
;## 00-decls). The kernel calls the `on-*` hooks below; a feature claims an
;## event by adding a handler, in the editor or in the host.

; The echo area, as a FORWARD HOOK: `run-command` reports through it, and the
; minibuffer that draws it loads later (the modeler's 08-editor binds it).
(define *echo* (lambda (_) #f))
; minibuffer echo for sections loading before 08

(define (run-command f)
  "Run F — a command reached from a key, a menu or a pie — so that an error it
  RAISES lands in the echo area. Verbs raise their reason (\"extrude: no faces
  selected\") so scripts fail loudly; a key press has no REPL to print it, and
  the kernel's hook runner would only log it to stderr."
  (guard (e (#t (*echo* (if (string? e) e (format "~a" e))) #f)) (f)))

; ── Hooks, not wraps (review item 8) ─────────────────────────────────────────
; Input events used to be layered by save-and-wrap: each feature redefined
; on-click, stashing the previous definition and delegating to it — six
; layers deep, and one missed stash silently severed every layer below.
; Now each event is ONE dispatcher over an ordered handler list: a feature
; adds itself with a priority; the first handler to return truthy CLAIMS the
; event; nil passes it down. Priorities <= 0 are POST handlers — they run
; after the claim scan, ALWAYS (work that must follow whoever handled the
; event, e.g. the gizmo re-sync after a click resolves).

; A handler list is a plain LIST of (prio name f) entries, held in a global,
; highest priority first — so a test or the REPL reads it with `map` and
; `filter` like any list. `add-handler` is a MACRO because it rebinds that
; global: every caller names the list's variable (*click-handlers* …), and a
; list cannot grow in place the way the Janet array it replaces did (M5b).
;
; (add-handler LIST PRIO NAME F) registers F as LIST's handler NAME at
; priority PRIO (higher runs first; re-adding NAME replaces it —
; live-redefinition friendly). A handler returning truthy claims the event;
; #f passes it on. PRIO <= 0 = a post handler: runs after the claim scan
; regardless of who claimed. Answers NAME.
(define (%with-handler entries prio name f)
  ;; Appended, then a STABLE sort: among equal priorities the newest runs
  ;; last, which is the order the array version kept.
  (list-sort (lambda (a b) (> (car a) (car b)))
             (append (filter (lambda (r) (not (equal? (cadr r) name))) entries)
                     (list (list prio name f)))))

(define-syntax add-handler
  (syntax-rules ()
    ((_ handlers prio name f)
     (begin (set! handlers (%with-handler handlers prio name f)) name))))

(define (run-handlers list . args)
  "Drive LIST for one event: claim scan in priority order, then the post
  handlers (prio <= 0). Returns the claiming handler's value, or nil."
  (let loop ((hs list) (claimed #f))
    (cond ((null? hs) claimed)
          ((positive? (car (car hs)))
           (loop (cdr hs) (or claimed (apply (caddr (car hs)) args))))
          (else (apply (caddr (car hs)) args) (loop (cdr hs) claimed)))))

(define (run-post-handlers list . args)
  "Drive only LIST's POST handlers (prio <= 0) — the ones `run-handlers`
  runs \"regardless of who claimed\".

  For an event a RUST TOOL claimed (T4): the claim scan is its, and never
  reaches here, but the chrome those post handlers maintain is owed either
  way — the gizmo's own sync after a click, the options strip's sweep, the
  edge loop's double-click timer. A claimed click that skipped them would
  leave the gizmo pointing at the last selection. This is why a claimed
  DRAG can skip Janet entirely and a claimed click cannot: there are no
  post handlers on *drag-handlers*, and three on *click-handlers*."
  (for-each (lambda (r) (unless (positive? (car r)) (apply (caddr r) args))) list)
  #f)

(define *cursor-handlers* '())
; (x y)               hover / focus-follows-mouse
(define *key-handlers* '())
; (key)               chords + keymap dispatch
(define *click-handlers* '())
; (x y shift)         left click
(define *wheel-handlers* '())
; (d x y)             scroll
(define *text-handlers* '())
; (s)                 printable text while typing
(define *drag-handlers* '())
; (button dx dy alt shift)
(define *release-handlers* '())
; (x y)               left button up
(define *escape-handlers* '())
; ()                  Escape, after typing focus

(define (on-cursor-move x y)
  "Kernel hook: un-dragged mouse travel."
  (run-handlers *cursor-handlers* x y))
(define (on-key key)
  "Kernel hook: a key press, as a normalized descriptor."
  (run-handlers *key-handlers* key))

(define *click-ctrl* #f)

(define (click-ctrl?)
  "Was CTRL down for the click being handled? Read inside a *click-handlers*
  entry, where it is this event's modifier.

  A var rather than a fourth argument on purpose: `run-handlers` splices its
  args into EVERY handler, so widening the click hook would mean widening the
  arity of all nine of them — in six files — for the one gesture that reads
  it (shift-ctrl-drag in a 3D pane, the region render). Set immediately before
  the handlers run and read inside them, so it is this click's modifier and
  nothing else's."
  *click-ctrl*)

(define (on-click x y (shift #f) (ctrl #f))
  "Kernel hook: a plain left click. SHIFT adds to a selection; CTRL is read
  through (click-ctrl?) by the one handler that wants it. Both are optional,
  so a scripted (on-click x y) still means a plain click."
  (set! *click-ctrl* (if ctrl #t #f))
  (run-handlers *click-handlers* x y shift))
(define (on-wheel d (x #f) (y #f) (alt #f) (shift #f))
  "Kernel hook: a scroll-wheel delta, with the cursor and the modifiers.
  ALT means dolly, everywhere; bare, the wheel belongs to whatever tool is
  engaged (see the :tool-wheel handler)."
  (run-handlers *wheel-handlers* d x y alt shift))
(define (on-text s)
  "Kernel hook: printable text while a typing surface holds focus."
  (run-handlers *text-handlers* s))
(define (on-mouse-drag button dx dy alt shift)
  "Kernel hook: a mouse drag (button dx dy alt shift)."
  (run-handlers *drag-handlers* button dx dy alt shift))
(define (on-release x y)
  "Kernel hook: the left button came up."
  (run-handlers *release-handlers* x y))

; The POST-ONLY halves, for an event a Rust tool claimed (T4) — see
; (doc run-post-handlers). Each takes what its hook takes.
(define (on-click-post x y (shift #f) (ctrl #f))
  (set! *click-ctrl* (if ctrl #t #f))
  (run-post-handlers *click-handlers* x y shift))
(define (on-release-post x y) (run-post-handlers *release-handlers* x y))
(define (on-escape-post) (run-post-handlers *escape-handlers*))
(define (on-key-post key) (run-post-handlers *key-handlers* key))
(define (on-escape)
  "Kernel hook: Escape, after typing focus; truthy = consumed (else the
  kernel continues its own chain: popup, preview, quit)."
  (run-handlers *escape-handlers*))
