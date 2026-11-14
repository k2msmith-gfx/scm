;## 18-search.scm — search and replace in the editor (Kevin, 2026-11-14:
;## "a basic search and replace where you can step through and replace one
;## by one or all"). Emacs's shape: C-s / C-r search forward and back from
;## the cursor, wrapping once; M-% steps through the matches asking at each
;## one, as query-replace does; replace-all does the lot in one undo step.
;## The text side is edit's editorpane (find, select_range, replace_all);
;## matching is plain text within a line, ignoring case unless the search
;## has a capital in it.

(define *search-last* #f)       ; the last search string
(define *replace-last* #f)      ; the last replacement

(define (search-fold? s)
  ; Smart case: a search with no capitals ignores case.
  (if (string=? s (string-downcase s)) 1 0))

(define (utf8-length s) (bytevector-length (string->utf8 s)))

(define (search-from forward)
  ; Where a search starts: just past the cursor going forward (so a repeat
  ; moves on from the match it selected), the cursor itself going back.
  (match (%editor-cursor (editor-pane-id))
    ((line index) (list line index))
    (_ (list 0 0))))

(define (search-step needle forward)
  "Find NEEDLE from the cursor — FORWARD, or back — select it and say so;
  #f when the buffer has none."
  (set! *search-last* needle)
  (match (search-from forward)
    ((line index)
      (let ((m (%editor-find (editor-pane-id) needle line
                             (if forward index (max 0 (- index (utf8-length needle))))
                             (if forward 1 0) (search-fold? needle))))
        (match m
          ((l s e wrapped)
            (%editor-select (editor-pane-id) l s e)
            (echo (if (positive? wrapped) "wrapped: " "") needle "  (line " (+ l 1) ")")
            #t)
          (_ (echo "not found: " needle) #f))))))

(define (with-default prompt last)
  ; "Search (beta): " — the last answer, which an empty RET takes. Shown
  ; rather than filled in, so typing starts a new one (Emacs's way).
  (if last (string-append prompt " (" last "): ") (string-append prompt ": ")))

(define (or-last s last) (if (and (string=? s "") last) last s))

(define (search-prompt forward)
  (read-minibuffer (with-default (if forward "Search" "Search backward") *search-last*)
                   (lambda (s)
                     (let ((s (or-last s *search-last*)))
                       (unless (string=? s "") (search-step s forward))))))

(define (search-forward)
  "Search forward from the cursor — C-s, Cmd-F — wrapping round once. An
  empty answer searches for the last thing again, so C-s RET is \"next\";
  Cmd-G finds the next one without asking."
  (search-prompt #t))

(define (search-backward)
  "Search backward from the cursor — C-r. Cmd-Shift-G repeats it."
  (search-prompt #f))

(define (search-next)
  "The next match of the last search — Cmd-G."
  (if *search-last* (search-step *search-last* #t) (search-forward)))

(define (search-previous)
  "The previous match of the last search — Cmd-Shift-G."
  (if *search-last* (search-step *search-last* #f) (search-backward)))

; ── Query replace: step through, one by one or all ─────────────────────────
; While one runs, its keys come first (the handlers below):
;   y SPC   replace this one, go to the next
;   n DEL   skip this one
;   !       replace this one and every one after it
;   .       replace this one and stop
;   q RET   quit (C-g and Escape too)

(define *query* #f)   ; while running: a hash of :from :to :fold :count :match :start :wrapped

(define (query-help)
  (echo "replace “" (%state-ref *query* :from) "” with “" (%state-ref *query* :to)
        "”?  y replace · n skip · ! all · . this and stop · q quit"
        (let ((n (%state-ref *query* :count))) (if (positive? n) (string-append "  (" (number->string n) " done)") ""))))

(define (query-done)
  (let ((n (%state-ref *query* :count)))
    (set! *query* #f)
    (echo "replaced " n (if (= n 1) " match" " matches"))))

(define (query-find from)
  ; The next match at or after FROM (line byte), wrapping round once and
  ; stopping where the run started: #f when there is none left.
  (match from
    ((line index)
      (match (%editor-find (editor-pane-id) (%state-ref *query* :from) line index 1 (%state-ref *query* :fold))
        ((l s e wrapped)
          (when (positive? wrapped) (hash-table-set! *query* :wrapped #t))
          (let ((past-start (and (%state-ref *query* :wrapped)
                                 (match (%state-ref *query* :start)
                                   ((sl si) (or (> l sl) (and (= l sl) (>= s si))))))))
            (and (not past-start) (list l s e))))
        (_ #f)))))

(define (query-show from)
  ; Select the next match and ask, or finish.
  (match (query-find from)
    ((l s e)
      (hash-table-set! *query* :match (list l s e))
      (%editor-select (editor-pane-id) l s e)
      (query-help))
    (_ (query-done))))

(define (query-replace-this)
  ; Replace the current match: where the search goes on from.
  (match (%state-ref *query* :match)
    ((l s e)
      (let* ((to (%state-ref *query* :to))
             (delta (- (utf8-length to) (- e s))))
        (%editor-replace (editor-pane-id) l s l e to)
        (hash-table-set! *query* :count (+ 1 (%state-ref *query* :count)))
        ; A replacement before the start on its line moves the start.
        (match (%state-ref *query* :start)
          ((sl si) (when (and (= l sl) (< s si))
                     (hash-table-set! *query* :start (list sl (+ si delta))))))
        (list l (+ s (utf8-length to)))))))

(define (query-answer what)
  (match (%state-ref *query* :match)
    ((l s e)
      (case what
        ((:yes) (query-show (query-replace-this)))
        ((:no) (query-show (list l e)))
        ((:this-and-stop) (query-replace-this) (query-done))
        ((:all)
          ; This one and the rest at once, as ONE undo step: from here to
          ; where the run started — round the end of the buffer if it
          ; started mid-way and has not wrapped yet.
          (let* ((start (%state-ref *query* :start))
                 (end (if (%state-ref *query* :wrapped) start
                        (match start ((0 0) (list 1000000000 0)) (_ start))))
                 (n (%editor-replace-all (editor-pane-id) (%state-ref *query* :from) (%state-ref *query* :to)
                                         (%state-ref *query* :fold) l s (car end) (cadr end))))
            (hash-table-set! *query* :count (+ n (%state-ref *query* :count)))
            (query-done)))
        ((:quit) (query-done))))
    (_ (query-done))))

(define (query-replace-run from to)
  (set! *search-last* from)
  (set! *replace-last* to)
  (let ((here (search-from #t)))
    (set! *query* (make-hash-table equal?))
    (hash-table-set! *query* :from from)
    (hash-table-set! *query* :to to)
    (hash-table-set! *query* :fold (search-fold? from))
    (hash-table-set! *query* :count 0)
    (hash-table-set! *query* :start here)
    (hash-table-set! *query* :wrapped #f)
    (query-show here)))

(define (query-replace)
  "Replace a string, one match at a time — M-% (option-shift-5), Cmd-Alt-F. Asks what to
  replace and with what, then selects each match from the cursor on
  (wrapping round once) and asks: y or SPC replaces it, n or DEL skips it,
  ! replaces it and all the rest, . replaces it and stops, q quits (RET,
  C-g and Escape too)."
  (read-minibuffer (with-default "Replace" *search-last*)
                   (lambda (from)
                     (let ((from (or-last from *search-last*)))
                       (unless (string=? from "")
                         (read-minibuffer (with-default (string-append "Replace “" from "” with") *replace-last*)
                                          (lambda (to) (query-replace-run from (or-last to *replace-last*)))))))))

(define (replace-all)
  "Replace every match in the buffer at once, as one undo step — asks what
  and with what. M-% then ! does the same from the cursor, looking first."
  (read-minibuffer (with-default "Replace all" *search-last*)
                   (lambda (from)
                     (let ((from (or-last from *search-last*)))
                       (unless (string=? from "")
                         (read-minibuffer (with-default (string-append "Replace every “" from "” with") *replace-last*)
                                          (lambda (to)
                                            (let ((to (or-last to *replace-last*)))
                                              (set! *search-last* from)
                                              (set! *replace-last* to)
                                              (let ((n (%editor-replace-all (editor-pane-id) from to (search-fold? from) 0 0 1000000000 0)))
                                                (echo "replaced " n (if (= n 1) " match" " matches")))))))))))

; The query's keys come before everything but an open menu: a key the query
; does not know is held (the prompt shows again) rather than run, so a stray
; key never edits the buffer mid-run. Printable keys arrive as TEXT while the
; editor holds typing, so both chains are watched.
(define (query-key k)
  (cond ((member k '("y" "space" " ")) (query-answer :yes))
        ((member k '("n" "backspace" "delete")) (query-answer :no))
        ((equal? k "!") (query-answer :all))
        ((equal? k ".") (query-answer :this-and-stop))
        ((member k '("q" "return" "C-g" "escape")) (query-answer :quit))
        (else (query-help)))
  #t)

(add-handler *key-handlers* 18 :query-replace (lambda (key) (and *query* (query-key key))))
(add-handler *text-handlers* 50 :query-replace (lambda (s) (and *query* (query-key s))))
(add-handler *escape-handlers* 60 :query-replace (lambda () (and *query* (query-answer :quit) #t)))

; ── Keys and menu ──────────────────────────────────────────────────────────
; Global, acting only while the editor has the keys (as M-g n does), so a
; host's own C-s elsewhere is untouched.
(define (when-editor f) (lambda () (if (editor-focused?) (f) #f)))
; C-s asks with the last search filled in, so C-s RET is "again".
(bind-key "C-s" (when-editor search-forward))
(bind-key "C-r" (when-editor search-backward))
(bind-key "s-f" (when-editor search-forward))
(bind-key "s-g" (when-editor search-next))
(bind-key "S-s-g" (when-editor search-previous))
; M-% is option-shift-5: the descriptor names the shift-free key.
(bind-key "M-S-5" (when-editor query-replace))
(bind-key "M-s-f" (when-editor query-replace))
