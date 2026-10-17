;;; test-scm-mode.el --- drive the real server from batch Emacs -*- lexical-binding: t -*-
(add-to-list 'load-path (file-name-directory (or load-file-name buffer-file-name)))
(require 'scm-mode)
(setq scm-program (or (getenv "SCM") "scm")
      scm-show-repl-on-connect nil)
(scm-start)
(let ((deadline (+ (float-time) 10)))
  (while (and (not (scm-connected-p)) (< (float-time) deadline))
    (accept-process-output nil 0.1))
  ;; ping's callback runs on the next read
  (accept-process-output scm--conn 0.3))
(unless (scm-connected-p) (error "did not connect"))
(cl-assert (equal (plist-get (scm--request-sync 'eval '(:code "(+ 1 2)")) :value) "3"))
(cl-assert (equal (plist-get (scm--request-sync 'eval '(:code "(display \"hi\\n\") (string-append \"a\" \"b\")")) :output) "hi\n"))
(cl-assert (eq (plist-get (scm--request-sync 'parse '(:code "(+ 1")) :framing) 'incomplete))
(cl-assert (member "vector-ref" (scm--candidates "vector-r")))
(let ((r (scm--request-sync 'describe '(:name "map"))))
  (cl-assert (plist-get r :bound))
  (cl-assert (equal (plist-get r :kind) "procedure")))
;; A signature reaches eldoc in the spelling a Scheme or Racket reader knows
;; (S21b): `:units units', not `&named units'. This is the protocol half of
;; that change -- what the editor shows is whatever `describe' returns.
(scm--request-sync 'eval '(:code "(define (area r #:units units) (* 3 r r))"))
(cl-assert (equal (plist-get (scm--request-sync 'describe '(:name "area")) :signature)
                  "(area r :units units)")
           nil "signature: %S" (scm--request-sync 'describe '(:name "area")))
;; The highlighting the README claims, asserted rather than claimed. Keywords
;; are scheme-mode's job and every shape scm can read must come out fontified
;; -- including `#:at' and the ones whose characters are not letters; the
;; brackets and the lambda-list markers are this mode's own rules.
(with-temp-buffer
  (insert "(define (add-box size #:at (at [0 1 0]) :tex-scale s &named :-x m) 1)")
  (scm-mode)
  (font-lock-ensure)
  (dolist (tok '("#:at" ":tex-scale" ":-x" "&named" "[" "]"))
    (goto-char (point-min))
    (search-forward tok)
    (let ((start (- (point) (length tok))))
      (dolist (i (number-sequence start (1- (point))))
        (cl-assert (get-text-property i 'face) nil
                   "%s: char %c at %d not fontified" tok (char-after i) i))))
  ;; And the two faces this mode owns are the ones it means.
  (goto-char (point-min))
  (cl-assert (eq (get-text-property (1- (search-forward "[")) 'face)
                 'font-lock-bracket-face))
  (goto-char (point-min))
  (cl-assert (eq (get-text-property (- (search-forward "&named") 6) 'face)
                 'font-lock-keyword-face))
  ;; The assertion that makes the six above mean something: an ordinary
  ;; parameter is NOT fontified, so "has a face" is a real distinction here
  ;; and not a property of every character in the buffer.
  (goto-char (point-min))
  (cl-assert (null (get-text-property (- (search-forward "size") 4) 'face))))
(let ((r (scm--request-sync 'eval '(:code "(define (f v) (vector-ref v 9)) (f (vector 1))"))))
  (cl-assert (eq (plist-get r :status) 'error))
  (cl-assert (string-match-p "out of range" (plist-get r :message)))
  (cl-assert (null (plist-get r :host-stopped)))
  (cl-assert (cl-some (lambda (f) (string-match-p "vector-ref v 9" f)) (plist-get r :backtrace))))
;; A string with every escape survives the round trip both ways.
(cl-assert (equal (plist-get (scm--request-sync 'eval (list :code (format "(string-length %s)" (scm--elisp-string "a\"b\\c\nd")))) :value) "7"))
;; The REPL buffer: a prompt, an entry, a result.
(scm--repl-ensure)
(with-current-buffer (scm--repl-buffer)
  (goto-char (point-max))
  (insert "(* 6 7)")
  (scm-repl-return)
  (let ((deadline (+ (float-time) 5)))
    (while (and (not (string-match-p "^42$" (buffer-string))) (< (float-time) deadline))
      (accept-process-output scm--conn 0.1)))
  (cl-assert (string-match-p "scm> (\\* 6 7)\n42\nscm> " (buffer-string)) nil "repl buffer: %S" (buffer-string)))
;; Interrupt over the control connection stops a runaway on the main one.
(let ((got nil))
  (scm--request 'eval '(:code "(let loop () (loop))") (lambda (r) (setq got r)))
  (accept-process-output scm--conn 0.3)
  (scm-interrupt)
  (let ((deadline (+ (float-time) 5)))
    (while (and (not got) (< (float-time) deadline)) (accept-process-output scm--conn 0.1)))
  (cl-assert (and got (plist-get got :host-stopped)) nil "interrupt: %S" got))
(cl-assert (equal (plist-get (scm--request-sync 'eval '(:code "(* 2 21)")) :value) "42"))
(scm-quit)
(message "scm-mode: all checks passed")
