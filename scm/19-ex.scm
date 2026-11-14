;## 19-ex.scm — vi's ':' commands (Kevin, 2026-11-14: "needs to support ':'
;## to prefix commands when using vi mode, like :q!"). In vi's normal mode ':'
;## opens the minibuffer as ':' — not the widget's own command line, which
;## showed only in the modeline — and RET runs the line here. The commands
;## are the everyday ones; anything else runs as an M-x command, so
;## `:remote-start` works too.

; What the HOST does for the two commands that leave the buffer, as FORWARD
; HOOKS. The standalone quits the program and opens files with open-path;
; a host that binds neither says so.
(define *ex-quit* (lambda () (echo "nothing to quit here") #f))
(define *ex-open* (lambda (path) (echo "no :e here — C-x C-f opens a file") #f))

(define (ex-dirty-buffers)
  ; The open buffers with unsaved changes.
  (let ((eid (editor-pane-id)))
    (filter-map (lambda (row) (and (positive? (cadr row)) (car row)))
                (%editor-buffer-list eid))))

(define (ex-quit (force #f))
  ; :q — refused while a buffer has changes, unless forced (:q!).
  (let ((dirty (ex-dirty-buffers)))
    (if (and (pair? dirty) (not force))
      (echo "unsaved changes in " (string-join (map (lambda (b) (%text (buffer-name b))) dirty) ", ")
            " — :w saves, :wq saves and quits, :q! quits anyway")
      (*ex-quit*))))

(define (ex-write-all)
  ; :wa — every changed buffer that visits a file.
  (let ((n 0))
    (for-each (lambda (b)
                (when (buffer-path b)
                  (save-buffer-text! b)
                  (set! n (+ n 1))))
              (ex-dirty-buffers))
    (echo "wrote " n (if (= n 1) " buffer" " buffers"))))

(define (ex-cycle-buffer step)
  ; :bn / :bp — the next or previous open buffer, in the order they opened.
  (let* ((eid (editor-pane-id))
         (bufs (map car (%editor-buffer-list eid)))
         (here (%editor-active-buffer eid))
         (n (length bufs))
         (i (let loop ((k 0) (bs bufs)) (cond ((null? bs) 0) ((equal? (car bs) here) k) (else (loop (+ k 1) (cdr bs)))))))
    (if (> n 1)
      (visit-buffer (list-ref bufs (modulo (+ i step) n)))
      (echo "no other buffer"))))

(define (ex-revert)
  ; :e! — the file as it is on disk, the changes dropped.
  (let* ((eid (editor-pane-id)) (path (buffer-path (%editor-active-buffer eid))))
    (if path
      (begin (%editor-set-text eid (slurp path))
             (%editor-mark-saved eid)
             (echo "reverted " path))
      (echo "this buffer has no file to revert to"))))

; ── :help ───────────────────────────────────────────────────────────────────
; (Kevin, 2026-11-14: "a :help command that displays all the key mappings and
; the vi commands supported, in a .md file".) One markdown page in the
; reading view: vi's normal-mode keys (what modit 0.1.5, the widget's vi,
; parses), the search's and the replace's single keys, every ':' command,
; then the live keymap — C-h b's page, regenerated each time.

(define *ex-commands*
  ; (command what) — the ':' commands ex-run takes, as :help lists them.
  '((":w  :w FILE  :w!" "write the buffer; with FILE, save as")
    (":q  :qa" "quit — refused while a buffer has unsaved changes, naming them")
    (":q!  :qa!" "quit, dropping changes")
    (":wq  :x  :wq!" "write, then quit")
    (":wa" "write every changed buffer")
    (":wqa  :xa" "write every changed buffer, then quit")
    (":e FILE" "open a file")
    (":e!" "drop this buffer's changes: the file as it is on disk")
    (":bn  :bp" "next / previous buffer")
    (":bd" "close this buffer")
    (":ls" "list the buffers")
    (":b  :b NAME" "switch buffer (asks, or by name)")
    (":N  :$" "go to line N; the last line")
    (":s/old/new/" "replace every match on this line")
    (":%s/old/new/" "replace every match in the buffer (one undo step)")
    (":%s/old/new/c" "step through them, asking at each (the replace keys below)")
    (":noh" "clear the search's selection")
    (":help  :h" "this page")
    (":anything else" "runs as an M-x command — :remote-start is (remote-start)")))

(define *vi-normal-keys*
  ; (keys what) — the widget's vi, as modit parses it.
  '(("h j k l  SPC" "left, down, up, right")
    ("w b e  W B E  ge gE" "word motions (W B E: space-separated words)")
    ("0 ^ $" "line start, first non-blank, line end")
    ("gg  G  NG" "first line, last line, line N")
    ("H M L" "top, middle, bottom of the screen")
    ("- +" "previous / next line's first non-blank")
    ("f t F T  ; ," "to (or before) a character, forward or back; repeat, reverse")
    ("i a I A o O" "insert: before, after, line start, line end, new line below, above")
    ("v V" "visual, visual line")
    ("x X" "delete the character under, before")
    ("d c y  + motion" "delete, change, yank — dd cc yy for lines; D C Y to line end")
    ("s S  r R" "substitute character, line; replace one character, replace mode")
    ("p P" "put after, before")
    ("u  ." "undo; repeat the last change")
    ("~  > <  =" "swap case; indent, outdent; auto-indent (+ motion)")
    ("i( a( i[ i{ i\" …" "text objects, after an operator: d i ( deletes inside parens")
    ("\"x" "use register x for the next delete, yank or put")
    ("ZZ  ZQ" "write and quit (:x); quit without writing (:q!)")
    ("K" "describe the symbol under the cursor (this editor)")
    ("Escape" "back to normal mode")))

(define (vi-help-markdown)
  (define out (open-output-string))
  (define (emit . parts) (for-each (lambda (p) (display p out)) parts))
  ; Bullets for the key lists, as C-h b's page has them; the search's two
  ; styles side by side are a table (the reading view sets one in columns).
  (define (table rows)
    (for-each (lambda (r) (emit "- `" (car r) "` — " (cadr r) "\n")) rows)
    (emit "\n"))
  (emit "# Help\n\n")
  (emit "vi's keys and commands, the search and replace keys, then every key binding.\n")
  (emit "`q` closes this page; `C-c C-v` shows its markdown source.\n\n")
  (emit "## vi: normal mode\n\n")
  (emit "vi is on by default; `M-x set-editor-vi #f` turns it off (`#t` back on). Counts work\n")
  (emit "as in vi: `3w`, `2dd`. The Emacs chords keep working in every mode.\n\n")
  (table *vi-normal-keys*)
  (emit "## Search\n\n")
  (emit "The same search in both styles. The match follows each character as you type,\n")
  (emit "from where the search began, wrapping round the buffer once; a miss turns the prompt\n")
  (emit "to `Failing`. No capital in the search means case is ignored.\n\n")
  (emit "| | plain (Emacs) | vi normal mode |\n|---|---|---|\n")
  (emit "| search forward | `C-s` `Cmd-F` | `/` |\n")
  (emit "| search backward | `C-r` | `?` |\n")
  (emit "| next / previous match while typing | `C-s` / `C-r` | `C-s` / `C-r` |\n")
  (emit "| keep the match | `RET` | `RET` |\n")
  (emit "| give up, cursor back | `C-g` `Escape` | `C-g` `Escape` |\n")
  (emit "| the last search again | `C-s RET` `Cmd-G` | `/ RET` `n` |\n")
  (emit "| ... backward | `Cmd-Shift-G` | `N` |\n\n")
  (emit "After `RET` the match stays selected and these single keys act; the bottom line lists\n")
  (emit "them. Any other key, or a click, ends the search and does what it always does:\n\n")
  (table '(("n" "next match") ("p  (N in vi)" "previous match") ("r" "replace, one by one from here")
           ("a" "replace all (one undo step)") ("q  RET  Escape" "done")))
  (emit "## Replace\n\n")
  (emit "`M-%` (option-shift-5) or `Cmd-Alt-F`, `r` after a search, or `:%s/old/new/c`. At each match:\n\n")
  (table '(("y  SPC" "replace this one, go to the next") ("n  DEL" "skip this one")
           ("a  !" "replace this one and all the rest (one undo step)") ("." "replace this one and stop")
           ("q  RET  C-g  Escape" "quit") ("?" "these answers as a menu")))
  (emit "`M-x replace-all` replaces every match in the buffer at once.\n\n")
  (emit "## vi: `:` commands\n\n")
  (table *ex-commands*)
  (emit "---\n\n")
  (get-output-string out))

(define (ex-help)
  "vi's :help — vi's keys and ':' commands, the search and replace keys,
  and every key binding, as one markdown page (q closes it)."
  (show-markdown-doc "*help*" (string-append (vi-help-markdown) (keybindings-markdown)))
  (echo "help — q closes it, C-c C-v shows the source"))

(define (ex-run line)
  "Run one ':' command line (without the ':'):
    :w [file]  :w!       write (save-as with a file)
    :q  :qa              quit — refused while a buffer has unsaved changes
    :q!  :qa!            quit, dropping changes
    :wq  :x  :wq!        write, then quit
    :wa                  write every changed buffer
    :e FILE  :e!         open a file; drop this buffer's changes
    :bn  :bp  :bd  :ls  :b  next, previous, close, list, switch buffer
    :N  :$               go to line N; the last line
    :s/old/new/  :%s/old/new/[c]   substitute (18-search)
    :noh                 clear the search's selection
    :help  :h            every key and command, as a markdown page
  Anything else runs as an M-x command: `:remote-start` is (remote-start)."
  (let* ((line (string-trim-both line))
         (sp (string-contains line " "))
         (cmd (if sp (substring line 0 sp) line))
         (arg (and sp (let ((a (string-trim-both (substring line (+ sp 1) (string-length line)))))
                        (if (string=? a "") #f a))))
         (eid (editor-pane-id)))
    (cond
      ((string=? line "") #f)
      ((string->number line) => (lambda (n) (%editor-goto-line eid (max 0 (- n 1)))))
      ((string=? line "$") (%editor-goto-line eid 1000000000))
      ((member cmd '("w" "write" "w!")) (save-buffer arg))
      ((member cmd '("q" "quit" "qa" "qall" "quitall")) (ex-quit))
      ((member cmd '("q!" "quit!" "qa!" "qall!" "quitall!")) (ex-quit #t))
      ((member cmd '("wq" "x" "xit" "exit" "wq!" "x!"))
        (if (or arg (buffer-path (%editor-active-buffer eid)))
          (begin (save-buffer arg)
                 (ex-quit (string-suffix? "!" cmd)))
          (begin (save-buffer) (echo "name the file, then :q"))))
      ((member cmd '("wa" "wall")) (ex-write-all))
      ((member cmd '("wqa" "wqall" "xa" "xall")) (ex-write-all) (ex-quit))
      ((equal? cmd "e!") (ex-revert))
      ((member cmd '("e" "edit")) (if arg (*ex-open* arg) (echo ":e FILE opens a file; :e! reverts this one")))
      ((member cmd '("bn" "bnext")) (ex-cycle-buffer 1))
      ((member cmd '("bp" "bprevious" "bN")) (ex-cycle-buffer -1))
      ((member cmd '("bd" "bdelete")) (kill-buffer))
      ((member cmd '("ls" "buffers" "files")) (list-buffers))
      ((member cmd '("b" "buffer"))
        (if arg
          (let ((b (buffer-by-name arg))) (if b (visit-buffer b) (echo "no buffer " arg)))
          (switch-to-buffer)))
      ((member cmd '("help" "h")) (ex-help))
      ((member cmd '("noh" "nohlsearch"))
        (match (%editor-cursor eid) ((l i) (%editor-select eid l i i)) (_ #f))
        (set! *search-mode* #f))
      ((ex-substitute line) #t)
      (else
        ; An M-x command, as typed after M-x.
        (guard (e (#t (echo ":" line " — not a command here (" (if (string? e) e (*show-value* e)) ")")))
          (let ((r (eval-string (string-append "(" line ")"))))
            (when r (echo line " => " (*show-value* r)))))))))

(define (ex-prompt)
  "vi's ':' — a command line in the minibuffer (ex-run says what it takes)."
  (read-minibuffer ":" ex-run))

; In vi's normal mode ':' opens it, above the editor's own text handler
; (09-complete's, 30), which takes every key the editor is typed.
; ZZ and ZQ too: modit takes the Z and does nothing with the second key.
(define *vi-z* #f)   ; a Z was pressed, waiting for the second key
(add-handler *text-handlers* 36 :vi-ex
  (lambda (s)
    (and (vi-normal?)
         (cond (*vi-z*
                 (set! *vi-z* #f)
                 (cond ((equal? s "Z") (ex-run "x") #t)
                       ((equal? s "Q") (ex-run "q!") #t)
                       (else #f)))
               ((equal? s "Z") (set! *vi-z* #t) #t)
               ((equal? s ":") (ex-prompt) #t)
               (else #f)))))
