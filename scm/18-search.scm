;## 18-search.scm — search and replace in the editor (Kevin, 2026-11-14:
;## "a basic search and replace where you can step through and replace one
;## by one or all"). Emacs's shape: C-s / C-r search forward and back from
;## the cursor, wrapping once; M-% steps through the matches asking at each
;## one, as query-replace does; replace-all does the lot in one undo step.
;## The text side is edit's editorpane (find, select_range, replace_all);
;## matching is plain text within a line, ignoring case unless the search
;## has a capital in it. After a search finds a match the keys are single
;## letters, as in the replace: n next, p previous, r replace one by one,
;## a replace all, q done; any
;## other key or a click ends that and does what it always does (Kevin,
;## 2026-11-14: "show the single key commands", not chords). In vi's normal
;## mode the same search is / and ?, n and N repeat it, and :s substitutes.

(define *search-last* #f)       ; the last search string
(define *replace-last* #f)      ; the last replacement
(define *search-mode* #f)       ; the match on screen (line start end) while
                                ; the search's single keys are live, else #f

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
            (set! *search-mode* (list l s e))
            (search-help (positive? wrapped))
            #t)
          (_ (set! *search-mode* #f) (echo "not found: " needle) #f))))))

(define (search-help (wrapped #f))
  ; The search's prompt, as the replace's is: in the accent, every key.
  (match *search-mode*
    ((l s e)
      (%pane-append (mini-id) "accent"
        (string-append (if wrapped "Wrapped: " "") "“" *search-last* "” line " (number->string (+ l 1))
                       "   [n] next   " (if (vi-normal?) "[N]" "[p]") " previous   [r] replace   [a] replace all   [q] done")))
    (_ #f)))

(define (search-done)
  "Leave the search's single keys; the match stays selected."
  (set! *search-mode* #f)
  (echo ""))

(define (search-replace)
  ; r: replace what was searched for, asking only what with, starting at
  ; the match on screen.
  (match *search-mode*
    ((l s e)
      (set! *search-mode* #f)
      (read-plain (with-default (string-append "Replace “" *search-last* "” with") *replace-last*)
                       (lambda (to)
                         (%editor-select (editor-pane-id) l s s)
                         (query-replace-run *search-last* (or-last to *replace-last*)))))
    (_ #f)))

(define (search-replace-all)
  ; a: replace every match of what was searched for, in the whole buffer,
  ; asking only what with — one undo step.
  (when *search-mode*
    (set! *search-mode* #f)
    (let ((from *search-last*))
      (read-plain (with-default (string-append "Replace every “" from "” with") *replace-last*)
                       (lambda (to)
                         (let ((to (or-last to *replace-last*)))
                           (set! *replace-last* to)
                           (let ((n (%editor-replace-all (editor-pane-id) from to (search-fold? from) 0 0 1000000000 0)))
                             (echo "replaced " n (if (= n 1) " match" " matches")))))))))

(define (search-key k)
  ; A key while the search's single keys are live: #f for one it does not
  ; know, which ends the mode and goes on to do what it always does.
  (cond ((member k '("n" "C-s" "s-g")) (search-step *search-last* #t) #t)
        ((member k '("p" "N" "C-r" "S-s-g")) (search-step *search-last* #f) #t)
        ((equal? k "r") (search-replace) #t)
        ((equal? k "a") (search-replace-all) #t)
        ((member k '("q" "return" "C-g" "escape")) (search-done) #t)
        (else (set! *search-mode* #f) #f)))

(define (with-default prompt last)
  ; "Search (beta): " — the last answer, which an empty RET takes. Shown
  ; rather than filled in, so typing starts a new one (Emacs's way).
  (if last (string-append prompt " (" last "): ") (string-append prompt ": ")))

(define (or-last s last) (if (and (string=? s "") last) last s))

(define (read-plain prompt callback (on-change #f) (on-cancel #f))
  ; A prompt for TEXT — what to search for, what to replace with: no TAB
  ; completion, and @ is a character, not the file picker (:plain).
  (read-minibuffer prompt callback :plain #f on-change on-cancel))

; ── Incremental: the match follows the typing ──────────────────────────────
; (Kevin, 2026-11-14: "as you type a character the cursor goes to that
; substring, then after you hit return, you get the single key options".)
; Each change searches again from where the search began; C-s / C-r at the
; prompt move on to the next or previous match; RET keeps the match and
; turns on the single keys; C-g or Escape puts the cursor back.

(define *isearch* #f)   ; while the prompt is up: a hash of :origin :forward
                        ; :match (line start end, or #f) :prompt

(define (isearch-show text)
  ; Find TEXT from the origin and select it, the prompt saying when it
  ; fails.
  (let* ((eid (editor-pane-id))
         (origin (%state-ref *isearch* :origin))
         (forward (%state-ref *isearch* :forward))
         (base (%state-ref *isearch* :prompt)))
    (define (prompt p) (%edit! :prompt p) (edit-sync))
    (match origin
      ((ol oi)
        (if (string=? text "")
          (begin (%editor-select eid ol oi oi)
                 (hash-table-set! *isearch* :match #f)
                 (prompt base))
          (match (%editor-find eid text ol oi (if forward 1 0) (search-fold? text))
            ((l s e wrapped)
              (%editor-select eid l s e)
              (hash-table-set! *isearch* :match (list l s e))
              (prompt (if (positive? wrapped) (string-append "Wrapped " base) base)))
            (_ (hash-table-set! *isearch* :match #f)
               (prompt (string-append "Failing " base)))))))))

(define (isearch-advance forward)
  ; C-s / C-r at the prompt: on to the next match (or back), from the one
  ; on screen; with nothing typed yet, the last search fills the line.
  (let ((text (%edit :text)))
    (if (and (string=? text "") *search-last*)
      (edit-set *search-last* (string-length *search-last*))
      (match (%state-ref *isearch* :match)
        ((l s e)
          (hash-table-set! *isearch* :forward forward)
          (hash-table-set! *isearch* :origin (if forward (list l e) (list l s)))
          (isearch-show text))
        (_ #f)))))

(define (search-prompt forward (vi #f))
  (let* ((eid (editor-pane-id))
         (origin (match (%editor-cursor eid) ((l i) (list l i)) (_ (list 0 0))))
         (base (if vi
                 (string-append (if forward "/" "?") (if *search-last* (string-append " (" *search-last* ") ") ""))
                 (with-default (if forward "Search" "Search backward") *search-last*))))
    (set! *search-mode* #f)
    (set! *isearch* (make-hash-table equal?))
    (hash-table-set! *isearch* :origin origin)
    (hash-table-set! *isearch* :forward forward)
    (hash-table-set! *isearch* :match #f)
    (hash-table-set! *isearch* :prompt base)
    (read-plain
      base
      (lambda (typed)
        ; RET: the match on screen stays, and the single keys come on.
        ; Nothing typed means the last search again, from here.
        (let ((m (%state-ref *isearch* :match))
              (fwd (%state-ref *isearch* :forward)))
          (set! *isearch* #f)
          (cond
            ((and (string=? typed "") *search-last*) (search-step *search-last* fwd))
            ((string=? typed "") #f)
            (m (set! *search-last* typed)
               (set! *search-mode* m)
               (search-help))
            (else (set! *search-last* typed) (echo "not found: " typed)))))
      (lambda (text) (when *isearch* (isearch-show text)))
      ; C-g / Escape: back where the search began.
      (lambda ()
        (when *isearch*
          (match (%state-ref *isearch* :origin)
            ((l i) (%editor-select eid l i i))
            (_ #f))
          (set! *isearch* #f))))))

(define (search-forward)
  "Search forward from the cursor as you type — C-s, Cmd-F, vi's / — the
  match following each character, wrapping round once; C-s again at the
  prompt goes on to the next. RET keeps the match; an empty answer
  searches for the last thing again. C-g or Escape goes back. On a match: n next, p
  previous, r replace one by one, a replace all, q done; any other key
  ends the search and acts."
  (search-prompt #t))

(define (search-backward)
  "Search backward from the cursor — C-r. The same keys on a match."
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
;   a !     replace this one and every one after it
;   .       replace this one and stop
;   q RET   quit (C-g and Escape too)
;   ?       the same as a menu at the match, to read or click

(define *query* #f)   ; while running: a hash of :from :to :fold :count :match :start :wrapped

(define (query-help)
  ; The prompt, in the accent so it reads as a question waiting for a key,
  ; not a message: every key it takes, and ? for the menu (Kevin,
  ; 2026-11-14: "it needs a prompt that explains the keys").
  (%pane-append (mini-id) "accent"
    (string-append
      "Replace “" (%state-ref *query* :from) "” with “" (%state-ref *query* :to) "”?   "
      "[y] replace   [n] skip   [a] all the rest   [.] replace and stop   [q] quit   [?] menu"
      (let ((n (%state-ref *query* :count)))
        (if (positive? n) (string-append "    " (number->string n) " replaced") "")))))

(define (query-menu)
  ; The answers as a popup under the match, each with its key: the long
  ; form of the prompt, to read or click.
  (match (%editor-cursor-xy (editor-pane-id))
    ((x y)
      (show-menu-items
        (list (list "Replace this one" (lambda () (query-answer :yes)) "y")
              (list "Skip this one" (lambda () (query-answer :no)) "n")
              (list "Replace all the rest" (lambda () (query-answer :all)) "a")
              (list "Replace this one and stop" (lambda () (query-answer :this-and-stop)) ".")
              (list "Quit" (lambda () (query-answer :quit)) "q"))
        x y 0 #t))
    (_ (query-help))))

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
  a (or !) replaces it and all the rest, . replaces it and stops, q quits (RET,
  C-g and Escape too)."
  (read-plain (with-default "Replace" *search-last*)
                   (lambda (from)
                     (let ((from (or-last from *search-last*)))
                       (unless (string=? from "")
                         (read-plain (with-default (string-append "Replace “" from "” with") *replace-last*)
                                          (lambda (to) (query-replace-run from (or-last to *replace-last*)))))))))

(define (replace-all)
  "Replace every match in the buffer at once, as one undo step — asks what
  and with what. M-% then ! does the same from the cursor, looking first."
  (read-plain (with-default "Replace all" *search-last*)
                   (lambda (from)
                     (let ((from (or-last from *search-last*)))
                       (unless (string=? from "")
                         (read-plain (with-default (string-append "Replace every “" from "” with") *replace-last*)
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
  (when (menu-open?) (menu-close))
  (cond ((member k '("y" "space" " ")) (query-answer :yes))
        ((member k '("n" "backspace" "delete")) (query-answer :no))
        ((member k '("a" "!")) (query-answer :all))
        ((equal? k ".") (query-answer :this-and-stop))
        ((member k '("q" "return" "C-g" "escape")) (query-answer :quit))
        ((member k '("?" "S-/")) (query-menu))
        (else (query-help)))
  #t)

;; Below the menu's own keys (20), so with the ? menu up, up/down/RET work
;; it; a letter still answers straight away.
(add-handler *key-handlers* 18 :query-replace (lambda (key) (and *query* (query-key key))))
(add-handler *text-handlers* 50 :query-replace (lambda (s) (and *query* (query-key s))))
(add-handler *escape-handlers* 60 :query-replace (lambda () (and *query* (query-answer :quit) #t)))
(add-handler *key-handlers* 17 :search (lambda (key) (and *search-mode* (search-key key))))
(add-handler *text-handlers* 49 :search (lambda (s) (and *search-mode* (search-key s))))
; A click anywhere ends the search's keys (a POST handler: it runs whoever
; claims the click), so a later n types an n.
(add-handler *click-handlers* -5 :search (lambda _ (set! *search-mode* #f) #f))

(define (search-escape)
  ; Escape: quit a replace, else leave the search's keys; #f when neither ran.
  (cond (*query* (query-answer :quit) #t)
        (*search-mode* (search-done) #t)
        (else #f)))

; ── vi: / ? n N and :s ──────────────────────────────────────────────────────
; In vi's normal mode, / and ? open this search rather than the widget's own
; (which shows nothing but the modeline), and n / N repeat it — the same
; prompt, the same single keys on a match.

(define (vi-normal?)
  (and (editor-typing?) (equal? (%editor-vi-mode (editor-pane-id)) "NORMAL")))

; Above the editor's own text handler (09-complete's, 30), which takes every
; key the editor is typed.
(add-handler *text-handlers* 35 :vi-search
  (lambda (s)
    (and (vi-normal?)
         (cond ((equal? s "/") (search-prompt #t #t) #t)
               ((equal? s "?") (search-prompt #f #t) #t)
               ((and (equal? s "n") *search-last*) (search-step *search-last* #t) #t)
               ((and (equal? s "N") *search-last*) (search-step *search-last* #f) #t)
               (else #f)))))

(define (ex-substitute line)
  "vi's :s — `s/OLD/NEW/` on this line, `%s/OLD/NEW/` the whole buffer;
  flag g is how it always works here (every match in the range, not the
  first per line), and flag c steps through them asking, as M-% does. Any
  character after the s may stand for the /. #f when LINE is not a :s."
  (let* ((whole (string-prefix? "%" line))
         (rest (if whole (substring line 1 (string-length line)) line)))
    (and (>= (string-length rest) 2)
         (char=? (string-ref rest 0) #\s)
         (let* ((delim (string (string-ref rest 1)))
                (parts (string-split (substring rest 2 (string-length rest)) delim))
                (from (if (pair? parts) (car parts) ""))
                (to (if (and (pair? parts) (pair? (cdr parts))) (cadr parts) ""))
                (flags (if (and (pair? parts) (pair? (cdr parts)) (pair? (cddr parts))) (caddr parts) "")))
           (cond
             ((string=? from "") (echo ":s needs something to replace — :%s/old/new/") #t)
             (else
               (set! *search-last* from)
               (set! *replace-last* to)
               (match (%editor-cursor (editor-pane-id))
                 ((cl ci)
                   (cond
                     ((string-contains flags "c")
                       ; Step through, from the range's start.
                       (%editor-goto-line (editor-pane-id) (if whole 0 cl))
                       (query-replace-run from to))
                     (else
                       (let ((n (%editor-replace-all (editor-pane-id) from to (search-fold? from)
                                                     (if whole 0 cl) 0
                                                     (if whole 1000000000 (+ cl 1)) 0)))
                         (echo "replaced " n (if (= n 1) " match" " matches")
                               (if whole "" " on this line"))))))
                 (_ #f))
               #t))))))

; ── Keys and menu ──────────────────────────────────────────────────────────
; Global, acting only while the editor has the keys (as M-g n does), so a
; host's own C-s elsewhere is untouched.
(define (when-editor f) (lambda () (if (editor-focused?) (f) #f)))
; C-s with nothing typed and RET is "the last search again".
; At the search's own prompt C-s / C-r step to the next or previous match.
(bind-key "C-s" (lambda () (cond (*isearch* (isearch-advance #t)) ((editor-focused?) (search-forward)) (else #f))))
(bind-key "C-r" (lambda () (cond (*isearch* (isearch-advance #f)) ((editor-focused?) (search-backward)) (else #f))))
(bind-key "s-f" (when-editor search-forward))
(bind-key "s-g" (when-editor search-next))
(bind-key "S-s-g" (when-editor search-previous))
; M-% is option-shift-5: the descriptor names the shift-free key.
(bind-key "M-S-5" (when-editor query-replace))
(bind-key "M-s-f" (when-editor query-replace))
