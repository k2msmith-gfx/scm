;## 05-buffers.scm — the editor's buffers: the registry (names, paths, the
;## MRU), visiting, saving, the editor pane's typing, keys and mouse, and the
;## buffer verbs C-x b / C-x k / C-x C-b (docs/editor-crate-plan.md, E3d —
;## moved from the modeler's unify.scm, which keeps what a buffer means to
;## the SCENE: *scene-file*, scene tags, reload-on-save, loading, find-file).

; What the HOST does around a buffer, as FORWARD HOOKS — the modeler's
; unify.scm binds each:
;   *buffers-changed*   a buffer was named, visited or closed (scene tags)
;   *show-status*       a buffer's path or name for the status strip
;   *show-editor*       reveal the editor (the left dock)
;   *hide-editor*       hide it again (a view-only popup that opened it quit)
;   *writable-path?*    may C-x C-s write a buffer visiting PATH (or none)?
;   *after-save*        (path own): after a write; true when it said so itself
;   *buffer-killed*     (path): after a close; a note for the message, or #f
;   *save-as-initial*   the directory a save-as prompt starts in
(define *buffers-changed* (lambda () #f))
(define *show-status* (lambda (text) #f))
(define *show-editor* (lambda () #f))
(define *hide-editor* (lambda () #f))
(define *writable-path?* (lambda (path) (not (or (image-path? path) (video-path? path)))))
(define *after-save* (lambda (path own) #f))
(define *buffer-killed* (lambda (path) #f))
(define *save-as-initial* (lambda () #f))

(define (editor-pane-id) (*pane-id* :editor))

; ── S5: multiple editor buffers + C-x b switching ───────────────────────────
; The Rust pane holds a stack of documents (each with its own text, cursor,
; undo, dirty flag). `*scene-file*` stays an ALIAS for the ACTIVE buffer's
; path, so open-scene / save-scene / (render) keep reading it unchanged.
; Item 1c (docs/modeler-architecture-review.md): the registry — path, display
; name, MRU order, auto-opened flag — lives ON the kernel Doc (%buffer-*
; prims), the same store that owns the text, so it cannot drift from the real
; buffer list (the old parallel *buffers* table needed a prune-mru janitor).
; The prelude keeps the POLICY: uniquified naming, find-file, C-x b defaults.

(define (buffer-basename path)
  (if (or (not path) (string=? path ""))
    "*scratch*"
    (last (string-split path "/"))))

(define (buffer-path buf)
  "The file BUF visits, or nil for a scratch/generated buffer."
  (%buffer-path (editor-pane-id) buf))

(define (buffer-name buf)
  "BUF's display name (the modeline / C-x b name)."
  (%buffer-name (editor-pane-id) buf))

(define (name-taken? name except)
  (define eid (editor-pane-id))
  (any (lambda (row)
         (let ((b (car row)))
           (and (not (equal? b except)) (equal? (%buffer-name eid b) name))))
       (%editor-buffer-list eid)))

(define (uniquify base except)
  ; Emacs uniquify, lite: foo.scm, foo.scm<2>, foo.scm<3>…
  (define (nth-name n) (string-append base "<" (number->string n) ">"))
  (if (not (name-taken? base except))
    base
    (let loop ((n 2))
      (if (name-taken? (nth-name n) except) (loop (+ n 1)) (nth-name n)))))

(define (register-buffer buf path)
  "Point BUF at PATH and give it a uniquified display name — on the Doc."
  (define eid (editor-pane-id))
  (%buffer-set-path eid buf path)
  (%buffer-set-name eid buf (uniquify (buffer-basename path) buf))
  (*buffers-changed*))

(define (buffer-names)
  "Open buffers' display names, most-recently-visited first — the C-x b
  completion source and (list-buffers) order."
  (define eid (editor-pane-id))
  (filter-map (lambda (b) (%buffer-name eid b)) (%buffer-mru eid)))

(define (buffer-by-name name) (%buffer-by-name (editor-pane-id) name))

(define (buffer-for-path path) (if path (%buffer-by-path (editor-pane-id) path) #f))

(define (image-path? path)
  "A view-only raster the editor can blit (.png/.jpg/.jpeg). image::open
  sniffs the format, so the suffix only gates the find-file dispatch."
  (and
    (string? path)
    (or
      (string-suffix? ".png" path)
      (string-suffix? ".jpg" path)
      (string-suffix? ".jpeg" path))))

(define (video-path? path)
  "A movie the editor can play (.mp4/.mov/.m4v/.webm/.mkv/.avi) — by its
  suffix, as image-path? goes. ffmpeg decodes it, so any codec it reads plays."
  (and (string? path)
       (let ((low (string-downcase path)))
         (any (lambda (e) (string-suffix? e low)) '(".mp4" ".mov" ".m4v" ".webm" ".mkv" ".avi")))))

(define (video-buffer?)
  "True when the active editor buffer is a movie."
  (and (%video-control (editor-pane-id) "state") #t))

(define (video-do what)
  ; A movie control by name, in the active buffer; says so when it is not one.
  (or (%video-control (editor-pane-id) what) (begin (echo "not a movie") #f)))

(define (video-play-pause)
  "Play or pause the movie — a click on it, or space. A movie opened from
  the file tree or C-x C-f starts playing; a drag across it scrubs, the
  pane's width spanning the whole clip."
  (video-do "toggle"))
(define (video-restart)
  "Play the movie from its first frame — 0, or Home."
  (video-do "restart"))
(define (video-step)
  "Pause, and show the next frame — l, or the right arrow."
  (video-do "step"))
(define (video-step-back)
  "Pause, and show the frame before — h, or the left arrow."
  (video-do "back"))
(define (video-toggle-loop)
  "Loop the movie, or play it once and stop at its end (the modeline says
  'once'). Loops by default."
  (match (video-do "loop")
    ((_ _ _ looping) (echo (if (= looping 1) "movie: loops" "movie: plays once")))
    (_ #f)))

; A movie buffer's right-press menu (hosts route to it when video-buffer?).
(define-menu
  :video
  (list (list "play / pause" video-play-pause "SPC")
        (list "restart" video-restart "0")
        (list "step forward" video-step "l")
        (list "step back" video-step-back "h")
        (list "loop on / off" video-toggle-loop)))

(define (visit-buffer buf)
  "Make BUF the on-screen buffer: switch the kernel (which bumps its MRU) and
  show its name. The SCENE does not move: *scene-file* names the file the
  loaded scene came from, and only a load changes it — C-o, or M-x
  reload-scene (docs/modeler-scene-buffers-plan.md). C-x C-s writes BUF's own file."
  (%editor-buffer-switch (editor-pane-id) buf)
  (*show-status* (or (buffer-path buf) (buffer-name buf)))
  (*buffers-changed*)
  buf)

(define (real-file path) (and path (or (%os-realpath path) path)))

(define (same-file? a b) (equal? (real-file a) (real-file b)))

(define (buffer-for-file p)
  ; The buffer visiting P's FILE, however it is spelled. buffer-for-path
  ; matches the string, and a definition site is a real path while the
  ; buffer may have been opened through a symlink (/var → /private/var on
  ; macOS): the jump then opened the file a second time, as name<2>.
  (or (buffer-for-path p)
      (let ((f (real-file p)))
        (find (lambda (b) (equal? f (real-file (buffer-path b))))
              (map car (%editor-buffer-list (editor-pane-id)))))))

; Wheel over the editor pane scrolls the FILE (its kind is "editor" — the
; "text" branch in 08-editor keeps scrolling transcripts). Wheel-up moves
; toward the top of the file, matching every text view.
; Wheel over the editor pane scrolls the FILE.
(add-handler
  *wheel-handlers*
  30
  :editor-scroll
  (lambda (d x y (alt #f) (shift #f))
    (define hit (if (and x y) (%pane-at x y) #f))   ; (id kind …)
    (if (and hit (equal? (cadr hit) "editor"))
      (begin (%editor-scroll (car hit) (* 3 (if (positive? d) -1 1))) #t)
      #f)))

; ── S2: editing, save, undo (docs/modeler-scene-editor-plan.md) ─────────────
; The kernel owns the buffer and the raw actions (%editor-act, %editor-undo/
; redo); everything below is policy — which keys mean what, where the
; clipboard lives, when the file writes. Typing focus follows the REPL's
; model: click the pane to type, Escape (or hovering a 3D view) releases.

(define (editor-typing?) (equal? (%typing-pane) (editor-pane-id)))

; What runs after a buffer is written to PATH (20-rust: its language
; server checks the file).
(define *save-hooks* '())
; The file may say another language now (07-backend). The hooks run with
; the written buffer active.
(set! *save-hooks* (cons (lambda (path) (redetect-language! (editor-pane-id))) *save-hooks*))

(define (save-buffer (path #f))
  "Write this buffer to its own file — C-x C-s. Runs nothing, unless
  reload-on-save is on and the file is one of the loaded scene's: then the
  scene reruns from its root, from disk.

  Also vi's :w [file]. This saves
  the text as you wrote it; (save-scene) stays the serialize-the-retained-store
  path. Prompts for a path when the buffer visits none. A PATH is a save-as:
  the buffer visits it from then on, and when the buffer was the scene's own
  (or there was no scene file) it becomes the new *scene-file*.

  Saving one of the loaded scene's files does not reload the scene unless
  *reload-on-save* is on; the modeline says the scene has changed, and C-c C-k
  runs the saved file on top of it (M-x reload-scene rebuilds it)."
  (define eid (editor-pane-id))
  (define buf (%editor-active-buffer eid))
  (define own (buffer-path buf))
  ; What the HOST does after a write (*after-save*: the modeler moves the
  ; scene's root, and reloads it or marks it changed) answers true when it
  ; has said so itself; otherwise "wrote PATH" is the whole of it.
  (define (write-to path)
    (spit path (%editor-text eid))
    (%editor-mark-saved eid)
    (unless (and own (same-file? own path)) (register-buffer buf path))
    ; The index is Scheme's (M-., M-?): a Rust file read as Scheme put its
    ; words among a Scheme name's uses (2026-11-15, the demo film).
    (unless (non-scheme-buffer? buf)
      (%xref-update (real-file path) (%text (%editor-text eid))))
    (for-each (lambda (h) (h path)) *save-hooks*)
    (*show-status* path)
    (unless (*after-save* path own) (echo "wrote " path)))
  (cond ; A buffer the host will not write (an image, a Janet file) has no
    ; editable text — never write (empty) text over it.
    ((not (*writable-path?* own))
      (echo "not a text buffer — nothing to save"))
    (path (write-to path))
    ; :w <file> — save-as to an explicit path
    (own (write-to own))
    (else (read-minibuffer "Write scene text to: " write-to :files (*save-as-initial*)))))

(define (editor-ex-command ex)
  "Dispatch a vi ':' ex-command submitted from the editor command line — EX
  is the \":<text>\" string %editor-act hands back on Enter. 19-ex's ex-run
  does the work (and says what it takes); ':' in vi's normal mode goes there
  through the minibuffer, so this is the widget's own line's way in."
  (ex-run (substring ex 1 (string-length ex))))

(define (focus-scene-editor)
  "Give the editor pane typing focus (held until Escape or a 3D hover)."
  (%focus-pane (editor-pane-id))
  (set! *focus-held* #t)
  (%hold-typing (editor-pane-id)))

(define (scene-editor-paste)
  "(scene-editor-paste)  ->  what was pasted, or nil

  Paste the system clipboard into the editor pane at point — C-y / Cmd-V
  there, and Paste on its marking menu. A direct insert in any vi mode (vi's
  own registers ride y/p), replacing a selection if there is one; a large
  paste is one undo step. Takes typing focus first when the menu was opened
  over a pane that had none, so the text lands where you can see it."
  (let ((t (%clipboard)))
    (and t
         (begin
           (unless (editor-typing?) (focus-scene-editor))
           (%editor-act (editor-pane-id) "paste" t)
           t))))

(define (scene-editor-copy)
  "(scene-editor-copy)  ->  what went to the clipboard, or nil

  Copy the editor pane's SELECTION to the system clipboard — Cmd-C there,
  and Copy on its marking menu. Nothing selected, nothing copied: the editor
  has a real selection (vi's visual mode, or a drag), so unlike the REPL's
  Copy there is no \"the line you are typing\" to fall back to."
  (let ((t (%editor-act (editor-pane-id) "copy")))
    (if t
      (begin (%set-clipboard t) (echo "copied " (string-length t) " chars") t)
      (begin (echo "nothing selected to copy") #f))))

; A view-only popup (help, keybindings, any markdown/reading-view buffer)
; swallows every keystroke in the kernel, so it can't be edited. 'q' in one
; quits it — like a `q`-to-quit help window: close the buffer, and if showing
; it is what auto-opened the editor dock (see show-markdown-doc), close the
; dock too so the workspace returns to how it was.
(define (editor-view-only?) (positive? (%editor-markdown? (editor-pane-id))))

(define (editor-quit-view)
  (define eid (editor-pane-id))
  (define buf (%editor-active-buffer eid))
  (define auto (positive? (%buffer-auto-opened? eid buf)))
  (when (%editor-buffer-close eid buf)
    ; false only when it's the last buffer
    (let ((now (%editor-active-buffer eid)))
      ; self-switch bumps NOW to MRU front
      (%editor-buffer-switch eid now)
      (*show-status* (or (buffer-path now) (buffer-name now)))
      (*buffers-changed*)))
  ; close the dock this popup opened
  (when auto (*hide-editor*)))

; Printable keys while the editor holds typing focus insert; everything
; else chains to the shared single-line editor's on-text. (The q-in-a-popup
; quit is added to the completion-aware on-text further down — the last
; redefinition is the one the kernel actually calls.)
; Editor typing inserts into the buffer. (The completion layer above
; handles editor typing first — this is its fallback should it ever pass.)
(add-handler
  *text-handlers*
  20
  :editor-insert
  (lambda (s) (if (editor-typing?) (begin (%editor-act (editor-pane-id) "insert" s) #t) #f)))

; Mouse: click focuses + places the cursor; double/triple click select
; word/line (detected here — the kernel sends plain clicks); dragging
; extends the selection. Deltas accumulate from the press point because
; on-mouse-drag speaks dx/dy.
(define *editor-click* (%make-state :t 0 :x 0 :y 0 :n 0))
(define *editor-drag* #f)

; A click in the editor pane focuses it and places the cursor; double/
; triple click select word/line (detected here — the kernel sends plain
; clicks). An open popup menu wins over cursor placement (the guard passes
; the click down to the menu handler so it runs its item / dismisses).
(add-handler
  *click-handlers*
  60
  :editor-cursor
  (lambda (x y shift)
    (define hit (%pane-at x y))                  ; (id kind …)
    (if (and (not (menu-open?)) hit (equal? (cadr hit) "editor"))
      (let* ((now (%os-clock))
             (c *editor-click*)
             (near (and (< (abs (- x (%state-ref c :x))) 6) (< (abs (- y (%state-ref c :y))) 6)))
             (n (if (and near (< (- now (%state-ref c :t)) 0.45)) (+ 1 (%state-ref c :n)) 1)))
        (focus-scene-editor)
        (set! *editor-click* (%make-state :t now :x x :y y :n n))
        (%editor-act
          (editor-pane-id)
          (case n ((2) "double-click") ((3) "triple-click") (else "click"))
          #f
          x
          y)
        (set! *editor-drag* (%make-state :x x :y y))
        #t)
      #f)))

; Dragging from an editor click extends the selection (deltas accumulate
; from the press point because the kernel speaks dx/dy).
(add-handler
  *drag-handlers*
  30
  :editor-select
  (lambda (button dx dy alt shift)
    (if (and *editor-drag* (equal? button :left) (not alt))
      (let ()
        (define d *editor-drag*)
        (hash-table-set! d :x (+ (%state-ref d :x) dx))
        (hash-table-set! d :y (+ (%state-ref d :y) dy))
        (%editor-act (editor-pane-id) "drag" #f (%state-ref d :x) (%state-ref d :y))
        #t)
      #f)))

(add-handler
  *release-handlers*
  20
  :editor-select-end
  (lambda (x y)
    ; The press ends: a movie takes it as a click (play/pause) or the end
    ; of a scrub; text has nothing to do.
    (if *editor-drag*
      (begin (set! *editor-drag* #f) (%editor-act (editor-pane-id) "release" #f x y) #t)
      #f)))

; Editor keys (pane-local; guarded on typing focus like the REPL's editing?
; guard, so a focused-but-released pane leaves bare keys to the global map).
(define (eact kind)
  (lambda () (if (editor-typing?) (%editor-act (editor-pane-id) kind) #f)))

(define (%when-typing f)
  "F, run only while the editor holds typing focus — the guard every editor
  key carries."
  (lambda () (if (editor-typing?) (f) #f)))

(define (bind-editor-pane-keys)
  "(bind-editor-pane-keys)  ->  :editor

The editor pane's keys on its pane-local map — newline, indent, the
motions, kill and yank, copy/cut/paste, undo and redo — each guarded so it
acts only while the editor holds typing. Call it once the pane exists."
  (for-each
    (lambda (row)                                  ; (key command label)
      (let ((k (car row)) (f (cadr row)) (label (caddr row)))
        (bind-key k (%name-binding label f) :editor)))
    (list
      (list "return" (eact "enter") "editor-newline")
      (list "backspace" (eact "backspace") "editor-backspace")
      (list "delete" (eact "delete") "editor-delete")
      (list "tab" (eact "indent") "editor-indent-line")
      (list "left" (eact "left") "editor-left")
      (list "right" (eact "right") "editor-right")
      (list "up" (eact "up") "editor-up")
      (list "down" (eact "down") "editor-down")
      (list "home" (eact "home") "editor-home")
      (list "end" (eact "end") "editor-end")
      (list "C-a" (eact "home") "editor-line-start")
      (list "C-e" (eact "end") "editor-line-end")
      (list "pageup" (eact "page-up") "editor-page-up")
      (list "pagedown" (eact "page-down") "editor-page-down")
      ; M-< and M-> as Emacs spells them (the kernel names a shifted key by
      ; its unshifted character): M-. and M-, are find-definition and back.
      (list "M-S-," (eact "buffer-start") "editor-buffer-start")
      (list "M-S-." (eact "buffer-end") "editor-buffer-end")
      (list "C-g" (eact "escape") "editor-deselect")
      (list
        "C-k"
        (%when-typing
          (lambda ()
            (let ((killed (%editor-act (editor-pane-id) "kill-eol")))
              (when killed (%set-clipboard killed)))))
        "editor-kill-line")
      (list
        "C-y"
        (%when-typing
          (lambda () (let ((t (%clipboard))) (when t (%editor-act (editor-pane-id) "paste" t)))))
        "editor-yank")
      (list
        "s-v"
        (%when-typing
          (lambda () (let ((t (%clipboard))) (when t (%editor-act (editor-pane-id) "paste" t)))))
        "editor-paste")
      (list
        "s-c"
        (%when-typing
          (lambda ()
            (let ((t (%editor-act (editor-pane-id) "copy")))
              (when t (%set-clipboard t) (echo "copied")))))
        "editor-copy")
      (list
        "s-x"
        (%when-typing
          (lambda ()
            (let ((t (%editor-act (editor-pane-id) "cut")))
              (when t (%set-clipboard t) (echo "cut")))))
        "editor-cut")
      (list
        "s-z"
        (%when-typing
          (lambda () (if (positive? (%editor-undo (editor-pane-id))) #f (echo "nothing to undo"))))
        "editor-undo")
      (list
        "S-s-z"
        (%when-typing
          (lambda () (if (positive? (%editor-redo (editor-pane-id))) #f (echo "nothing to redo"))))
        "editor-redo")
      (list
        "C-/"
        (%when-typing
          (lambda () (if (positive? (%editor-undo (editor-pane-id))) #f (echo "nothing to undo"))))
        "editor-undo")))
  ; C-M-x is one chord — safe pane-local (08-sends binds the C-x/C-c sends
  ; globally, with a focus guard).
  (bind-key "C-M-x" 'eval-defun :editor)
  ; M-. and M-, (10-xref): Emacs's xref keys, pane-local.
  (bind-key "M-." 'find-definition-at-point :editor)
  (bind-key "M-," 'go-back-from-definition :editor)
  ; The structural keys, C-M-f and the rest (23-structure, loaded later
  ; but bound by the time this runs).
  (bind-structure-keys)
  :editor)

(define (set-editor-vi on)
  "Toggle VI editing in the scene-editor pane: modal NORMAL/INSERT/VISUAL
  with motions, operators, and text objects (modit — the COSMIC vi engine).
  ON by default. Escape returns to normal mode (a second Escape releases
  the pane); the mode badge sits bottom-left; the Emacs chords (C-x C-e
  sends, C-x C-s save, cmd-Z undo) keep working in every mode. To opt out
  at boot put (set-editor-vi #f) in ~/.config/modeler/init.scm, which loads
  after the whole prelude."
  (%editor-set-vi (if on 1 0))
  (echo "editor vi mode " (if on "on" "off")))

;; LINE NUMBERS down the editor's left side (Kevin, 2026-11-16): on by
;; default; the modeline's right end gives the cursor's LINE:COLUMN either way.
(define *line-numbers* :absolute)

(define (set-line-numbers (mode #f))
  "The editor's line numbers. MODE #t or :absolute numbers each line;
  :relative counts from the cursor's line, as vi's relativenumber — what a
  5j or d3k counts — with the cursor's line keeping its own number; #f
  hides them. As M-x, with no MODE, it turns them off, or back on as they
  were. On by default; put (set-line-numbers :relative) or
  (set-line-numbers #f) in your init.scm to start otherwise."
  (define m
    (cond ((not (given? mode)) (if (eq? *line-numbers* #f) :absolute #f))
          ((eq? mode #t) :absolute)
          (else mode)))
  (case m
    ((:absolute :relative #f)
     (set! *line-numbers* m)
     (%editor-set-line-numbers (case m ((:absolute) 1) ((:relative) 2) (else 0)))
     (echo "line numbers " (if m (keyword->string m) "off")))
    (else (echo "set-line-numbers: :absolute, :relative or #f, not " (*show-value* m)))))

; C-x C-s: save the editor buffer when the editor is focused (global — a
; pane-local C-x prefix would shadow C-x C-f/C-x i while typing there).
(define (editor-focused?) (equal? (%focused-pane) (editor-pane-id)))
(bind-key "C-x C-s" (lambda () (if (editor-focused?) (save-buffer) #f)))
(hash-table-set! *editor-key-labels* "C-x C-s" "save-buffer")   ; the lambda names nothing

; ── The buffer verbs (C-x b / C-x k / C-x C-b) ─────────────────────────────

(define (save-buffer-text! buf)
  ; Write BUF's text to its file without leaving the buffer on screen.
  (define eid (editor-pane-id))
  (define cur (%editor-active-buffer eid))
  (%editor-buffer-switch eid buf)
  (spit (buffer-path buf) (%editor-text eid))
  (%editor-mark-saved eid)
  (for-each (lambda (h) (h (buffer-path buf))) *save-hooks*)
  (%editor-buffer-switch eid cur))

(define (switch-to-buffer)
  "Show another open buffer — C-x b. Runs nothing; the scene stays as it is.

  By name: TAB completes;
  empty input picks the most-recently-used OTHER buffer (Emacs's default).
  An unknown name opens a fresh empty buffer under it. Each buffer keeps its
  own text, cursor, undo, and dirty flag across switches. Switching never
  changes the scene: C-c C-b runs the buffer you are in on top of it."
  (*show-editor*)
  (let* ((names (buffer-names))
         ; names[0] is the current buffer (MRU-first)
         (deflt (%list-ref/default names 1 #f)))
    (read-minibuffer
      (string-append "Switch to buffer" (if deflt (string-append " (default " deflt ")") "") ": ")
      (lambda (input)
        (let* ((name (string-trim-both input))
               (target (if (string=? name "") deflt name)))
          (cond ((not target) (echo "no other buffer"))
                ((buffer-by-name target) (visit-buffer (buffer-by-name target)))
                (else
                  (let* ((eid (editor-pane-id))
                         (buf (%editor-buffer-new eid)))
                    (%buffer-set-name eid buf (uniquify target buf))
                    (%editor-set-text eid "")
                    (%editor-mark-saved eid)
                    (visit-buffer buf))))))
      :buffers)))

(define (fresh-scratch)
  "A new empty *scratch*, made active — the floor C-x k lands on when it
  kills the last buffer. Same sequence as (scene-untitled!), spelled out here
  because that one is a no-op when the current buffer is already unnamed, and
  killing *scratch* has to produce a NEW one rather than keep the old."
  (define eid (editor-pane-id))
  (define buf (%editor-buffer-new eid))
  (register-buffer buf #f)
  ; names it *scratch* (uniquified if taken)
  (%editor-set-text eid "")
  (%editor-mark-saved eid)
  (visit-buffer buf)
  buf)

(define (kill-buffer)
  "Close this buffer — C-x k. Runs nothing; closing the buffer of the
  scene's root file also clears the scene.

  With unsaved changes, confirms first. Killing the buffer of the scene's ROOT clears the scene, since it
  owned it; killing any other buffer — one of the scene's parts included,
  whose file is still on disk — leaves the scene alone. Killing the LAST
  buffer leaves a fresh empty *scratch*, the way Emacs always keeps one."
  (define eid (editor-pane-id))
  (define buf (%editor-active-buffer eid))
  (define (do-kill)
    ; The floor. %editor-buffer-close refuses to remove the only buffer, so
    ; make the replacement FIRST and the close is never that case. Before
    ; this, C-x k on the last buffer did nothing at all — silently, and while
    ; its own docstring claimed it left the buffer empty. And because
    ; find-file REUSES an untouched *scratch* (so the first C-o consumes it),
    ; "the last buffer" is normally a file buffer: there was no way back to a
    ; blank slate once you had opened anything.
    ;
    ; What closing it means to the HOST, asked after the close: the
    ; modeler clears the scene when BUF was its root's (*buffer-killed*),
    ; and answers what to add to the message.
    (define path (buffer-path buf))
    (when (= 1 (length (%editor-buffer-list eid)))
      (fresh-scratch))
    (if (%editor-buffer-close eid buf)
      (let* ((now (%editor-active-buffer eid))
             (note (*buffer-killed* path)))
        (visit-buffer now)
        (unless (buffer-path now)
          ; The floor buffer was named while the one being killed still
          ; existed, so uniquify gave it *scratch*<2>. Take the plain name
          ; back now that the old one is gone.
          (register-buffer now #f))
        (echo "killed buffer; now on " (buffer-name now) (or note "")))
      (echo "can't kill the last buffer")))
  (if (positive? (%editor-dirty? eid))
    (read-minibuffer
      (string-append "Buffer " (%text (buffer-name buf)) " modified; kill anyway? (y/n) ")
      (lambda (ans)
        (if (member (string-downcase (string-trim-both ans)) '("y" "yes"))
          (do-kill)
          (echo "kill cancelled"))))
    (do-kill)))

(define (list-buffers)
  "Pick an open buffer from a list over the minibuffer — C-x C-b: the active
  one in [brackets], unsaved ones starred, most recent first. Up/down move,
  Return (or a click) switches, anything else closes. The highlight starts on
  the buffer before this one, so C-x C-b Return goes back, as C-x b Return
  does. Runs nothing."
  (define eid (editor-pane-id))
  (define active (%editor-active-buffer eid))
  (define dirty (%editor-buffer-list eid))      ; ((buf dirty) ...)
  (define bufs (%buffer-mru eid))
  (define items
    (map
      (lambda (b)
        (let* ((nm (%text (%buffer-name eid b)))
               (row (assoc b dirty))
               (star (if (and row (positive? (cadr row))) "*" "")))
          (list (if (equal? b active) (string-append "[" nm star "]") (string-append " " nm star " "))
                (lambda () (visit-buffer b)))))
      bufs))
  ; Over the minibuffer: the popup flips above a point it would run off the
  ; window from, so it rises out of the echo area.
  (match (%pane-rect (mini-id))
    ((x y . _) (show-menu-items items x y (if (pair? (cdr bufs)) 1 0) #t))
    (_ (echo "no minibuffer to list the buffers over"))))

(bind-key "C-x b" 'switch-to-buffer)
(bind-key "C-x k" 'kill-buffer)
(bind-key "C-x C-b" 'list-buffers)
