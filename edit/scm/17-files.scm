;## 17-files.scm — the file tree: a column of the files under the git
;## checkout the editor is working in (Kevin, 2026-11-13). Folders expand and
;## collapse; RET or a click opens a file as C-x C-f does (open-path); what
;## git ignores is hidden unless asked for; the tree follows the buffer on
;## screen. The pane is a text pane in list layout; each row carries two
;## mark cells the host paints its icon over — a folder, braces for source,
;## a picture, a page — which the text pane draws, with the indent guides,
;## in any host (edit's textpane.rs, for a proportional list pane).

; What the HOST supplies, as FORWARD HOOKS:
;   *files-pane*    the pane the tree shows in, or #f (the standalone's left
;                   column; in the modeler, a column of the left dock beside
;                   the editor)
;   *files-shown*   show or hide that pane (#t / #f)
;   *files-shown?*  whether it shows
(define *files-pane* (lambda () (*pane-id* :files)))
(define *files-shown* (lambda (on) #f))
(define *files-shown?* (lambda () #f))

(define (files-mark n) (string (integer->char n) (integer->char n)))
(define folder-mark (files-mark #xE000))

; Which icon a file shows, by its extension: source as braces, images as a
; picture, the rest as a page (Kevin, 2026-11-14, after Zed's panel).
(define *files-code-extensions*
  '("scm" "ss" "sld" "guile" "janet" "lisp" "el" "rs" "py" "js" "ts" "c" "h"
    "cpp" "nix" "toml" "json" "sh" "glsl" "wgsl" "lua" "go"))
(define *files-image-extensions*
  '("png" "jpg" "jpeg" "gif" "webp" "svg" "exr" "hdr" "tif" "tiff" "bmp"
    "mp4" "mov" "m4v" "webm" "mkv" "avi"))

(define (files-extension p)
  (let ((b (files-base p)))
    (let loop ((k (- (string-length b) 1)))
      (cond ((<= k 0) "")
            ((char=? (string-ref b k) #\.) (string-downcase (substring b (+ k 1) (string-length b))))
            (else (loop (- k 1)))))))

(define (files-file-mark p)
  (let ((ext (files-extension p)))
    (files-mark (cond ((member ext *files-code-extensions*) #xE001)
                      ((member ext *files-image-extensions*) #xE002)
                      (else #xE003)))))

(define *files-root* #f)                          ; the directory at the top
(define *files-open* (make-hash-table equal?))    ; expanded folder → #t
(define *files-ignored* (make-hash-table equal?)) ; ignored path → #t
(define *files-show-ignored* #f)
(define *files-rows* '#())                        ; per row: (path dir? depth)
(define *files-sel* 0)

(define (files-path dir name)
  (if (string-suffix? "/" dir) (string-append dir name) (string-append dir "/" name)))

(define (files-dir? p) (eq? :directory (%alist-ref (or (%os-stat p) '()) :mode #f)))

(define (files-load-ignored!)
  ; What git ignores under the root, as absolute paths (a directory once).
  (set! *files-ignored* (make-hash-table equal?))
  (for-each (lambda (rel)
              (hash-table-set! *files-ignored*
                               (files-path *files-root* (if (string-suffix? "/" rel)
                                                          (substring rel 0 (- (string-length rel) 1))
                                                          rel))
                               #t))
            (%git-ignored *files-root*)))

(define (files-entries dir)
  "DIR's entries the tree shows: folders first, then files, each sorted;
  dotfiles and .git always hidden, what git ignores hidden unless shown."
  (let* ((names (or (%os-dir dir) '()))
         (shown (filter (lambda (n)
                          (and (not (string-prefix? "." n))
                               (or *files-show-ignored*
                                   (not (hash-table-ref/default *files-ignored* (files-path dir n) #f)))))
                        names))
         (paths (map (lambda (n) (files-path dir n)) shown))
         (dirs (list-sort string<? (filter files-dir? paths)))
         (files (list-sort string<? (filter (lambda (p) (not (files-dir? p))) paths))))
    (append (map (lambda (p) (list p #t)) dirs) (map (lambda (p) (list p #f)) files))))

(define (files-base p)
  (let ((i (let loop ((k (- (string-length p) 1)))
             (cond ((< k 0) #f) ((char=? (string-ref p k) #\/) k) (else (loop (- k 1)))))))
    (if i (substring p (+ i 1) (string-length p)) p)))

(define (files-row-text row)
  (match row
    ((path dir? depth)
      (string-append (make-string (* 2 depth) #\space)
                     (if dir? (if (hash-table-ref/default *files-open* path #f) "▾ " "▸ ") "  ")
                     (string-append (if dir? folder-mark (files-file-mark path)) " ")
                     (files-base path)))))

(define (files-collect)
  ; The rows: the root, then each shown entry, open folders' entries under
  ; them.
  (let walk ((dir *files-root*) (depth 1) (acc (list (list *files-root* #t 0))))
    (fold (lambda (e acc)
            (match e
              ((path dir?)
                (let ((acc (cons (list path dir? depth) acc)))
                  (if (and dir? (hash-table-ref/default *files-open* path #f))
                    (walk path (+ depth 1) acc)
                    acc)))))
          acc
          (files-entries dir))))

(define (files-render)
  "Lay the tree out in its pane, keeping the selection on the same path."
  (let ((id (*files-pane*))
        (was (files-selected)))
    (when (and id *files-root*)
      (set! *files-rows* (list->vector (reverse (files-collect))))
      (set! *files-sel*
        (or (and was (let loop ((i 0))
                       (cond ((= i (vector-length *files-rows*)) #f)
                             ((equal? (car (vector-ref *files-rows* i)) was) i)
                             (else (loop (+ i 1))))))
            (min *files-sel* (max 0 (- (vector-length *files-rows*) 1)))))
      (%pane-clear id)
      (let loop ((i 0))
        (when (< i (vector-length *files-rows*))
          (%pane-append id (if (= i *files-sel*) "accent" (if (zero? i) "dim" "normal"))
                        (files-row-text (vector-ref *files-rows* i)))
          (loop (+ i 1))))
      (%pane-reveal-row id *files-sel*))))

(define (files-selected)
  (and (< *files-sel* (vector-length *files-rows*)) (car (vector-ref *files-rows* *files-sel*))))

(define (files-select! i)
  (let ((id (*files-pane*)) (n (vector-length *files-rows*)))
    (when (and id (positive? n))
      (let ((old *files-sel*) (new (max 0 (min i (- n 1)))))
        (%pane-set-row id old (if (zero? old) "dim" "normal") (files-row-text (vector-ref *files-rows* old)))
        (set! *files-sel* new)
        (%pane-set-row id new "accent" (files-row-text (vector-ref *files-rows* new)))
        (%pane-reveal-row id new)))))

; ── Commands ───────────────────────────────────────────────────────────────

(define (files-open-root dir)
  "Root the tree at the git checkout DIR is in (DIR itself outside git),
  with the root open."
  (let ((root (or (%git-root dir) (real-file dir))))
    (set! *files-root* root)
    (set! *files-open* (make-hash-table equal?))
    (hash-table-set! *files-open* root #t)
    (set! *files-sel* 0)
    (files-load-ignored!)
    (files-render)
    root))

(define (files-refresh)
  "Read the tree again from disk — what changed since, and what git now
  ignores. The open folders stay open."
  (when *files-root*
    (files-load-ignored!)
    (files-render)
    (echo "files: " *files-root*)))

(define (files-expand)
  "Open the selected folder — right, or + in the tree."
  (match (and (positive? (vector-length *files-rows*)) (vector-ref *files-rows* *files-sel*))
    ((path #t depth) (hash-table-set! *files-open* path #t) (files-render))
    (_ #f)))

(define (files-collapse)
  "Close the selected folder, or, on a file or a closed folder, go up to
  the folder it is in — left, or - in the tree."
  (match (and (positive? (vector-length *files-rows*)) (vector-ref *files-rows* *files-sel*))
    ((path dir? depth)
      (if (and dir? (hash-table-ref/default *files-open* path #f) (positive? depth))
        (begin (hash-table-delete! *files-open* path) (files-render))
        ; Up to the parent row: the nearest row above with a smaller depth.
        (let loop ((i (- *files-sel* 1)))
          (cond ((< i 0) #f)
                ((< (caddr (vector-ref *files-rows* i)) depth) (files-select! i))
                (else (loop (- i 1)))))))
    (_ #f)))

(define (files-collapse-all)
  "Close every folder but the root."
  (set! *files-open* (make-hash-table equal?))
  (when *files-root* (hash-table-set! *files-open* *files-root* #t))
  (set! *files-sel* 0)
  (files-render))

(define (files-toggle-ignored)
  "Show or hide what git ignores."
  (set! *files-show-ignored* (not *files-show-ignored*))
  (files-render)
  (echo (if *files-show-ignored* "showing" "hiding") " what git ignores"))

(define (files-activate)
  "RET in the tree: open the selected file as C-x C-f does — the keyboard
  goes to the editor — or open or close the selected folder."
  (match (and (positive? (vector-length *files-rows*)) (vector-ref *files-rows* *files-sel*))
    ((path #t depth)
      (if (and (hash-table-ref/default *files-open* path #f) (positive? depth))
        (hash-table-delete! *files-open* path)
        (hash-table-set! *files-open* path #t))
      (files-render))
    ((path #f depth) (open-path path) (focus-scene-editor))
    (_ #f)))

(define (files-reveal path)
  "Select PATH in the tree, opening the folders above it — how the tree
  follows the buffer on screen. Nothing outside the root."
  (when (and *files-root* path)
    (let ((rp (real-file path)))
      (when (and rp (string-prefix? (string-append *files-root* "/") rp))
        ; Open each folder between the root and the file.
        (let loop ((i (+ (string-length *files-root*) 1)))
          (let ((slash (string-contains rp "/" i)))
            (when slash
              (hash-table-set! *files-open* (substring rp 0 slash) #t)
              (loop (+ slash 1)))))
        (files-render)
        (let find ((i 0))
          (when (< i (vector-length *files-rows*))
            (if (equal? (car (vector-ref *files-rows* i)) rp)
              (files-select! i)
              (find (+ i 1)))))))))

(define (files-focus)
  "Give the tree the keys: the FOCUS, not typing — a typing pane takes a
  bare key as text, and the tree's keys (n, p, g, +, -) are commands in its
  own keymap."
  (let ((id (*files-pane*)))
    (when id
      (*files-shown* #t)
      (when (%typing-pane) (%release-typing))
      (set! *focus-held* #f)
      (%focus-pane id))))

(define (toggle-files)
  "Show or hide the file tree — the strip's folder icon."
  (if (*files-shown?*)
    (begin (*files-shown* #f) (when (equal? (%typing-pane) (*files-pane*)) (focus-scene-editor)))
    (begin (*files-shown* #t) (unless *files-root* (files-open-root ".")) (files-render))))

; ── Keys, the mouse, the menu ──────────────────────────────────────────────

(define (files-next)
  "Select the next row of the tree — down, or n."
  (files-select! (+ *files-sel* 1)))

(define (files-previous)
  "Select the row above in the tree — up, or p."
  (files-select! (- *files-sel* 1)))

(define (bind-files-keys)
  "(bind-files-keys)  ->  :files

The tree's own keys: up/down (p/n) move, RET opens, right/left (+/-)
expand and collapse, g reads the disk again, i shows what git ignores, q
gives the keyboard back. Call it once the host's tree pane exists."
  (for-each (lambda (row) (bind-key (car row) (cadr row) :files))
            (list (list "down" files-next)
                  (list "n" files-next)
                  (list "up" files-previous)
                  (list "p" files-previous)
                  (list "return" files-activate)
                  (list "right" files-expand)
                  (list "+" files-expand)
                  (list "left" files-collapse)
                  (list "-" files-collapse)
                  (list "g" files-refresh)
                  (list "i" files-toggle-ignored)
                  (list "q" focus-scene-editor)))
  :files)

; A click on a row selects it; on a file, opens it; on a folder, opens or
; closes it.
(add-handler
  *click-handlers*
  58
  :files
  (lambda (x y . _)
    ; Not while a menu is up: the click is the menu's.
    (let ((id (*files-pane*)))
      (and id (not (menu-open?))
           (let ((hit (%pane-at x y)))
             (and hit (equal? (car hit) id)
                  (begin
                    (files-focus)
                    (let ((row (%pane-row-at id x y)))
                      (when row (files-select! row) (files-activate)))
                    #t)))))))

(add-handler
  *wheel-handlers*
  36
  :files-scroll
  (lambda (d x y . _)
    (let ((id (*files-pane*)) (hit (and x y (%pane-at x y))))
      (and id hit (equal? (car hit) id)
           (begin (%pane-scroll id (* 3 (if (positive? d) 1 -1))) #t)))))

; The tree's menu acts on the row under the press, as the outliner's does.
(set-pane-menu! :files
  (lambda (x y id shift)
    (let ((row (%pane-row-at id x y)))
      (when row (files-select! row)))
    :files))

(define-menu
  :files
  (list (list "open" files-activate "RET")
        (list "expand" files-expand "+")
        (list "collapse" files-collapse "-")
        (list "collapse all" files-collapse-all)
        (list "refresh" files-refresh "g")
        (list "show/hide ignored" files-toggle-ignored "i")
        (list "hide the tree" toggle-files)))
