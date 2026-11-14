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
(add-handler *text-handlers* 36 :vi-ex
  (lambda (s) (and (equal? s ":") (vi-normal?) (begin (ex-prompt) #t))))
