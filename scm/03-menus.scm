;## 03-menus.scm — menus as data, the linear popup and the marking-menu
;## gesture over them (docs/editor-crate-plan.md, E3b — moved from the
;## modeler's 09-menus, which keeps the toolbox's menu and the entries that
;## say what each of its panes opens).

; ── Context menus (M5): menus as DATA, the widget is a renderer ────────────
; (define-menu :name (list (list label fn) ...)) stores plain data; the kernel
; draws the rows (the popup overlay, point-anchored) and hit-tests them.
; This linear popup is the first renderer; the radial marking-menu gesture
; layer can arrive later over the SAME data (the M5 decision). Right-click
; opens :viewport in any 3D pane; hover or arrows move, click or return
; runs, Escape/C-g/click-away closes. All live-redefinable.

;Menu definitions by name: a hash table, name -> a list of (label
;zero-arg-fn) items.
(define *menus* (make-hash-table equal?))

(define *menu* #f)
; the open menu: a %make-state of :items and :sel, or #f

(define *pie*
  ; The in-flight gesture: a %make-state of :items ((label fn) ...),
  ; :at (x y), :sel idx-or-#f, :shown, :opaque and :t — or #f.
  #f)

(define (%item-label it) (car (%as-list it)))
(define (%item-action it) (cadr (%as-list it)))
;The key that runs an item's action, for the label — or #f. unify.scm sets
;it once the keymap's labels exist; a menu drawn before then shows none.
(define *menu-key-hint* (lambda (action) #f))

(define (title-case s)
  "S with the first letter of each word capitalised — a word starts the
  string or follows a space or a /. The rest is left as written, so REPL
  and C-c stay as they are. How a command menu's labels are drawn (Kevin,
  2026-11-14: \"first letter capitalized of each word\")."
  (let loop ((cs (string->list s)) (start #t) (acc '()))
    (if (null? cs)
      (list->string (reverse acc))
      (let ((c (car cs)))
        (loop (cdr cs)
              (and (memv c '(#\space #\/)) #t)
              (cons (if (and start (char-alphabetic? c)) (char-upcase c) c) acc))))))

(define (%item-text it (title #f))
  "IT's label as drawn: the label, then a TAB and the key that runs the same
  command — a third element of the item when it has one, else looked up in
  the live keymap, so a rebinding shows in the menu. The kernel draws the key
  in its own column and ink (theme slot :menu-key-ink). TITLE draws the label
  in title case (a command menu's; a list of buffers or objects keeps its
  names as they are)."
  (let* ((parts (%as-list it))
         (label (let ((l (%text (%item-label it)))) (if title (title-case l) l)))
         (key (if (> (length parts) 2) (caddr parts) (*menu-key-hint* (cadr parts)))))
    (if key (string-append label "\t" key) label)))

(define (%item-labels items (title #f))
  "ITEMS' labels as the one newline-joined string the kernel draws, each
  with its key beside it (`%item-text`); TITLE in title case, as every
  named menu (define-menu) is drawn — the data lists show-menu-items posts
  are not."
  (string-join (map (lambda (it) (%item-text it title)) (%as-list items)) "\n"))

;Drag distance (px) below which no sector is armed — releasing inside
;it either posts the linear menu (quick click) or cancels (after hold).
(define *pie-dead-zone* 16)

(define (define-menu name items)
  "Define (or replace) a named menu from (label fn) pairs:
  (define-menu :viewport (list (list \"Frame\" frame!) ...)). Menus are plain
  data — edit them live, and the AI can too. Each label is drawn with the key
  bound to its command; a third element, (label fn \"key\"), names the key
  for a command reached through another procedure."
  (hash-table-set! *menus* name items)
  name)

(define (menu-close) (set! *menu* #f) (%clear-completions))

(define (on-menu-cancel)
  "Kernel hook: Escape closed the open popup (list or pie)."
  (set! *menu* #f)
  (set! *pie* #f))

(define (menu-open?)
  "True while a linear popup menu is up (not the radial pie). Outer on-click
  handlers (e.g. the editor pane's, which otherwise eats clicks to place the
  cursor) check this and defer to the menu layer so a click runs its item."
  (if *menu* #t #f))

(define (menu-run idx)
  (define item (%list-ref/default (%as-list (%state-ref *menu* :items)) idx #f))
  (menu-close)
  (and item
       (begin
         (when *command-echo* (*command-echo* (%item-action item)))
         (run-command (%item-action item)))))

(define (show-menu name x y)
  "Open a named menu at window point (x, y)."
  (let ((items (hash-table-ref/default *menus* name #f)))
    (if items
        (begin
          (set! *menu* (%make-state :items items :sel 0))
          (%show-menu (%item-labels items #t) 0 x y))
        (begin (display (format "unknown menu ~s — see *menus*" name)) (newline) #f))))

(define (show-menu-items items x y (sel 0) (opaque #f) (title #f))
  "Open a popup of [label fn] items built right now, at window point (x, y) —
  where (show-menu) opens one that was DEFINED. For a menu whose items are the
  situation rather than the vocabulary: the objects under an ambiguous click,
  say. Same widget, same dismissal, no entry in *menus*. SEL is the row
  highlighted first; OPAQUE draws it solid, for a popup over text. TITLE
  draws the labels in title case, as a named menu's are: for a menu of
  commands, not of names (buffers, objects), which keep their spelling."
  (if (pair? (%as-list items))
    (begin
      (set! *menu* (%make-state :items items :sel sel))
      (%show-menu (%item-labels items title) sel x y (if opaque 1 0)))
    #f))

; ── The marking-menu gesture (the optional radial layer, over the SAME
; menu data). Kurtenbach's trick: selection logic runs from the moment of
; the press; the pie only DRAWS after the kernel's ~200ms hold timer —
; novices pause and read, experts flick blind, one code path. A quick
; right-click (no drag, no hold) falls back to the linear popup.

(define (pie-index dx dy n)
  "The ring sector for a drag offset, 0 = up, clockwise — or nil inside
  the dead zone."
  (if (> (sqrt (+ (* dx dx) (* dy dy))) *pie-dead-zone*)
    (let* ((ang (atan dx (- dy)))
           (step (/ (* 2 %pi) n)))
      ;; EXACT: the sector indexes a list, and `round` of a float is a float.
      (modulo (+ (exact (round (/ ang step))) n) n))
    #f))

(define (pie-draw)
  (let ((at (%state-ref *pie* :at)))
    (%show-pie
      (%item-labels (%state-ref *pie* :items) #t)
      (or (%state-ref *pie* :sel) -1)
      (car at)
      (cadr at)
      (if (%state-ref *pie* :opaque) 1 0))))

(define (menu-opaque? x y)
  "Should a menu opened at (x y) be SOLID rather than glass?

  Over a 3D pane the glass is the point: the model stays visible under the
  labels, which is what lets you aim a flick at what you can see. Over a text
  pane there is nothing to see through to and the text underneath fights every
  label, so those menus are solid (Kevin, 2026-09-28: the agent, REPL and dock
  menus). Stated as \"not 3D\" rather than as a list of panes, so a menu on a
  pane that has none yet — the outliner — is right the day it gets one."
  (let ((hit (%pane-at x y)))
    (if hit (not (equal? (cadr hit) "3d")) #t)))

(define (set-menu-alpha (a #f))
  "(set-menu-alpha &opt a)  ->  the opacity in effect

How see-through the MENUS are — the linear popup and the marking menu's
label boxes, both renderers of the same data.

  a   0 invisible · 1 solid · default 0.38, the toolbox's own glass

  → the value in effect; (set-menu-alpha) with no argument just reads it.

  · the SELECTED row is drawn nearer solid whatever this is: a highlight you
    can see through stops saying \"this one\", which is all it is for.
  · COMPLETION popups are not affected. They share the renderer but not the
    job — they sit over source you are reading letter by letter.
  · a preference, so it belongs in ~/.config/modeler/init.scm."
  (%set-menu-alpha (or a -1)))

(define (set-pie-delay ms)
  "The marking menu's draw delay in ms (default 200 — Kurtenbach's
  press-and-wait, the delay that trains the expert flick). 0 shows the
  pie instantly at the press."
  (%set-pie-delay ms))

;Which menu a right-press opens, by the pressed pane's ROLE: an alist of
;(role . f), f taking (x y id shift) and answering a *menus* name or #f. The
;host fills it with `set-pane-menu!`; a role it never names opens nothing.
(define *pane-menus* '())

(define (set-pane-menu! role f)
  "(set-pane-menu! ROLE F)  ->  ROLE

What a right-press on a pane of ROLE opens: F is called with (x y id shift)
at the press and answers a menu's name in *menus*, or #f for none. It may act
first — focus the pane, select what is under the press — as a 3D pane's
selects the object it opens a menu for. Replaces ROLE's entry."
  (set! *pane-menus* (cons (cons role f) (filter (lambda (e) (not (equal? (car e) role))) *pane-menus*)))
  role)

; The two things a press asks the host, as FORWARD HOOKS: the pressed pane's
; ROLE from its id and %pane-at kind, and whether that role may open a menu
; at all. The modeler's 09-menus binds them to its pane-role and pane-allows?.
(define *pane-role* (lambda (id kind) #f))
(define *menu-allowed?* (lambda (role) #t))

(define (on-menu-press x y (shift #f))
  "Kernel hook: right button down (no chord). Anchors the marking-menu
  gesture over the menu the pressed pane's role names (set-pane-menu!), or
  does nothing when it names none or the role may not menu. SHIFT goes to
  the role's entry, which may read it (a 3D pane's opens :viewport from
  anywhere with it)."
  (let ((hit (%pane-at x y)))
    (when hit
      (let* ((id (car hit))
             (role (*pane-role* id (cadr hit)))
             (entry (and (*menu-allowed?* role) (assoc role *pane-menus*)))
             (menu-name (and entry ((cdr entry) x y id shift))))
        (if (and menu-name (hash-table-ref/default *menus* menu-name #f))
          (begin
            (set! *pie*
              (%make-state
                :items (hash-table-ref/default *menus* menu-name #f)
                :at (list x y)
                :sel #f
                :shown #f
                :opaque (menu-opaque? x y)
                :t (%os-clock)))
            *pie*)
          #f)))))

(define (on-menu-hold)
  "Kernel hook: the ~200ms hold timer — draw the pie the flick never needs."
  (if *pie* (begin (hash-table-set! *pie* :shown #t) (pie-draw)) #f))

(define (on-menu-drag x y)
  "Kernel hook: the cursor moved with the right button down — arm the
  sector under the drag angle (drawn or not)."
  (if *pie*
    (let* ((at (%state-ref *pie* :at))
           (idx (pie-index (- x (car at)) (- y (cadr at))
                           (length (%as-list (%state-ref *pie* :items))))))
      (unless (equal? idx (%state-ref *pie* :sel))
        (hash-table-set! *pie* :sel idx)
        (when (%state-ref *pie* :shown) (pie-draw)))
      ; The ink trail follows the stroke whenever the pie is up (pie-draw
      ; resets it, so re-mark after).
      (if (%state-ref *pie* :shown) (%pie-mark x y) #f))
    #f))

(define (on-menu-release x y)
  "Kernel hook: right button up — commit the armed sector; a quick click
  in the dead zone posts the linear menu instead; a held release in the
  dead zone cancels."
  (if *pie*
    (let ((p *pie*))
      (set! *pie* #f)
      (%clear-completions)
      (cond
        ((%state-ref p :sel)
          (let ((item (%list-ref/default (%as-list (%state-ref p :items)) (%state-ref p :sel) #f)))
            (when item
              (when *command-echo* (*command-echo* (%item-action item)))
              (run-command (%item-action item)))))
        ; A dead-zone release with the pie never shown = a quick click:
        ; post the linear menu (browse mode). Once the pie is up — however
        ; it got there, hold or instant mode — the pie IS the browse UI and
        ; a dead-zone release cancels.
        ((not (%state-ref p :shown))
          (let ((at (%state-ref p :at)))
            (set! *menu* (%make-state :items (%state-ref p :items) :sel 0))
            (%show-menu
              (%item-labels (%state-ref p :items) #t)
              0
              (car at)
              (cadr at)
              (if (%state-ref p :opaque) 1 0))))
        (else #f)))
    #f))

; Decorate the input hooks: an open menu absorbs clicks, hover moves its
; highlight, and navigation keys drive it — everything else passes through
; to the base behavior defined above.
; An open linear popup claims clicks (run the hit row / dismiss), hover
; (selection tracking) and keys (arrows/return/anything-else-dismisses).
(add-handler
  *click-handlers*
  20
  :menu
  (lambda (x y shift)
    (if *menu*
      (let ((row (%overlay-hit x y))) (if row (menu-run row) (menu-close)) #t)
      #f)))

(add-handler
  *cursor-handlers*
  20
  :menu
  (lambda (x y)
    (if *menu*
      (begin
        (let ((row (%overlay-hit x y)))
          (when (and row (not (equal? row (%state-ref *menu* :sel))))
            (hash-table-set! *menu* :sel row)
            (%overlay-select row)))
        #t)
      #f)))

(add-handler
  *key-handlers*
  20
  :menu
  (lambda (key)
    (if *menu*
      (begin
        (let ((sel (%state-ref *menu* :sel)))
          (cond
            ((equal? key "up")
              (hash-table-set! *menu* :sel (max 0 (- sel 1)))
              (%overlay-select (%state-ref *menu* :sel)))
            ((equal? key "down")
              (hash-table-set! *menu* :sel
                (min (- (length (%as-list (%state-ref *menu* :items))) 1) (+ sel 1)))
              (%overlay-select (%state-ref *menu* :sel)))
            ((equal? key "return") (menu-run sel))
            (else (menu-close))))
        ; any other key dismisses (C-g, a stray letter, ...)
        #t)
      #f)))
