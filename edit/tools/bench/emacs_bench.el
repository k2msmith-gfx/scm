;;; emacs_bench.el --- edit's benchmark, as Emacs runs it  -*- lexical-binding: t -*-
;; Each key runs as typed (execute-kbd-macro: the command loop, evil and
;; every hook), then a forced redisplay -- the terminal write -- and the
;; pair is timed. BENCH-OUT names the results file; BENCH-EVIL says whether
;; the keys are vi's (Doom) or Emacs's own (emacs -Q).
(defvar bench-out "emacs.txt")
(defvar bench-evil t)

(defun bench--write (name times)
  (let ((line (concat name (mapconcat (lambda (x) (format " %.6f" x)) times "") "\n")))
    (write-region line nil bench-out t 'silent)))

(defun bench--run (name keys)
  (let (times)
    (dolist (k keys)
      (let ((t0 (float-time)))
        (execute-kbd-macro k)
        (redisplay t)
        (push (- (float-time) t0) times)))
    (bench--write name (nreverse times))))

(defun bench--chars ()
  (let (out)
    (dotimes (i 600)
      (push (cond ((= (% i 60) 59) (kbd "RET"))
                  ((= (% i 7) 6) " ")
                  (t (string (+ ?a (% i 26)))))
            out))
    (nreverse out)))

(defun bench-go ()
  (when (file-exists-p bench-out) (delete-file bench-out))
  (goto-char (point-min))
  (redisplay t)
  (if bench-evil
      (progn
        (bench--run "scroll-j" (make-list 1000 "j"))
        (bench--run "page-down" (make-list 100 [next]))
        (execute-kbd-macro "3000Go")
        (redisplay t)
        (bench--run "insert" (bench--chars))
        (execute-kbd-macro [escape])
        (execute-kbd-macro "gg")
        (redisplay t)
        (bench--run "search" (make-list 200 (kbd "/define RET"))))
    (bench--run "scroll-j" (make-list 1000 (kbd "C-n")))
    (bench--run "page-down" (make-list 100 [next]))
    (goto-char (point-min))
    (forward-line 2999)
    (end-of-line)
    (newline)
    (redisplay t)
    (bench--run "insert" (bench--chars))
    (goto-char (point-min))
    (redisplay t)
    (bench--run "search" (make-list 200 (kbd "C-s define RET"))))
  (write-region (format "lines %d columns %d cursor %d:%d major %s\n"
                        (window-body-height) (window-body-width)
                        (line-number-at-pos) (current-column) major-mode)
                nil bench-out t 'silent)
  (write-region (point-min) (point-max) (concat bench-out ".text") nil 'silent)
  (set-buffer-modified-p nil)
  (kill-emacs 0))

;; After startup has settled: the dashboard gone, the file shown, any
;; idle-time work done.
(run-with-timer 4 nil
                (lambda ()
                  (switch-to-buffer (get-file-buffer bench-file))
                  (delete-other-windows)
                  (bench-go)))
