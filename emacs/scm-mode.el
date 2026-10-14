;;; scm-mode.el --- scm Scheme: editing, a REPL, and a SLIME-style connection -*- lexical-binding: t -*-

;; Copyright (C) 2026 Kevin Smith
;; Author: Kevin Smith
;; Version: 0.1
;; Package-Requires: ((emacs "28.1"))
;; Keywords: languages, lisp, scheme

;;; Commentary:

;; A client of scm's REPL protocol (docs/repl.md in the scm repository).
;; `scm-mode' edits .scm files on top of the built-in `scheme-mode', starts
;; `scm --serve' the first time a file opens, connects over TCP, and gives
;; the subset of SLIME anyone uses:
;;
;;   C-x C-e   eval the sexp before point         C-c C-c   eval the top-level form
;;   C-c C-r   eval the region                    C-c C-k   eval the whole buffer
;;   C-c C-z   go to the REPL                     C-c C-d   describe the symbol at point
;;   C-c C-b   interrupt a runaway                C-c C-q   stop the server
;;   TAB / completion-at-point in both the editor and the REPL
;;
;; The REPL buffer, *scm*, is this package's own: RET sends a form when the
;; server says it is complete and inserts a newline when it says more is
;; coming -- the same parse the terminal client asks for, over the same
;; door. M-p / M-n walk the history.
;;
;; Nothing here parses Scheme. Framing and completion are the server's.

;;; Code:

(require 'scheme)
(require 'cl-lib)
(require 'subr-x)

(defgroup scm nil "The scm Scheme and its REPL." :group 'languages :prefix "scm-")

(defcustom scm-program "scm"
  "The scm executable. A path, or a name on `exec-path'."
  :type 'string)

(defcustom scm-connect-automatically t
  "Start and connect to an scm server when the first .scm buffer opens."
  :type 'boolean)

(defcustom scm-show-repl-on-connect t
  "Display the REPL buffer in a window when the connection comes up."
  :type 'boolean)

(defcustom scm-request-timeout 10
  "Seconds a synchronous request waits before giving up."
  :type 'number)

;;;; The connection

(defvar scm--server nil "The `scm --serve' subprocess.")
(defvar scm--conn nil "The network connection requests go over.")
(defvar scm--control nil "A second connection, for `:interrupt' -- it is out of band by design.")
(defvar scm--pending "" "Bytes received and not yet a whole response.")
(defvar scm--next-id 0)
(defvar scm--callbacks (make-hash-table) "Request id -> function of the response plist.")
(defvar scm--version nil)
(defvar scm--on-connect nil "Functions to call once connected.")

(defun scm-connected-p ()
  "Whether a live connection exists."
  (and scm--conn (process-live-p scm--conn)))

(defun scm--log (fmt &rest args)
  (with-current-buffer (get-buffer-create " *scm-log*")
    (goto-char (point-max))
    (insert (apply #'format fmt args) "\n")))

(defun scm-start ()
  "Start an scm server and connect to it. Reconnects if one is running."
  (interactive)
  (scm-quit)
  (let ((exe (or (executable-find scm-program) scm-program)))
    (unless (file-executable-p exe)
      (user-error "scm: %s is not executable; set `scm-program'" scm-program))
    (setq scm--pending "")
    (clrhash scm--callbacks)
    (setq scm--server
          (make-process :name "scm-server"
                        :buffer " *scm-server*"
                        :command (list exe "--serve" "127.0.0.1:0")
                        :noquery t
                        :filter #'scm--server-filter
                        :sentinel (lambda (_p event)
                                    (scm--log "server: %s" (string-trim event)))))
    (message "scm: starting %s" exe)))

(defun scm--server-filter (proc string)
  "Learn the port from the server's one line of stdout, then connect."
  (scm--log "server> %s" (string-trim string))
  (when (string-match "scm serving on \\([^:]+\\):\\([0-9]+\\)" string)
    (let ((host (match-string 1 string))
          (port (string-to-number (match-string 2 string))))
      (set-process-filter proc nil)
      (scm--connect host port))))

(defun scm--connect (host port)
  (setq scm--conn (open-network-stream "scm" nil host port :nowait nil)
        scm--control (open-network-stream "scm-control" nil host port :nowait nil))
  (set-process-query-on-exit-flag scm--conn nil)
  (set-process-query-on-exit-flag scm--control nil)
  (set-process-coding-system scm--conn 'utf-8-unix 'utf-8-unix)
  (set-process-coding-system scm--control 'utf-8-unix 'utf-8-unix)
  (set-process-filter scm--conn #'scm--filter)
  (set-process-filter scm--control #'scm--filter)
  (scm--request 'ping nil
                (lambda (r)
                  (setq scm--version (plist-get r :version))
                  (message "scm %s connected on %s:%d" scm--version host port)
                  (scm--repl-ensure)
                  (when scm-show-repl-on-connect
                    (display-buffer (scm--repl-buffer)))
                  (run-hooks 'scm--on-connect)
                  (setq scm--on-connect nil))))

(defun scm-quit ()
  "Stop the server and drop the connection."
  (interactive)
  (dolist (p (list scm--conn scm--control scm--server))
    (when (and p (process-live-p p))
      (delete-process p)))
  (setq scm--conn nil scm--control nil scm--server nil))

(defun scm-ensure-connected (&optional then)
  "Connect if not connected; run THEN once connected."
  (if (scm-connected-p)
      (when then (funcall then))
    (when then (push then scm--on-connect))
    (unless (and scm--server (process-live-p scm--server))
      (scm-start))))

;;;; Requests and responses

(defun scm--elisp-string (s)
  "S as a string literal scm's reader accepts: `prin1' escapes \" and \;
newlines may stay raw."
  (prin1-to-string s))

(defun scm--request (op args callback)
  "Send OP with ARGS, a plist of keywords and strings or numbers.
CALLBACK gets the response plist."
  (unless (scm-connected-p)
    (user-error "scm: not connected (M-x scm-start)"))
  (scm--send scm--conn op args callback))

(defun scm--send (conn op args callback)
  (let* ((id (cl-incf scm--next-id))
         (text (with-output-to-string
                 (princ (format "(:op %s :id %d" op id))
                 (cl-loop for (k v) on args by #'cddr
                          do (princ (format " %s %s" k
                                            (if (stringp v) (scm--elisp-string v) v))))
                 (princ ")\n"))))
    (when callback (puthash id callback scm--callbacks))
    (scm--log "-> %s" (string-trim text))
    (process-send-string conn text)
    id))

(defun scm--request-sync (op args)
  "Send OP and wait for its response plist, or nil on timeout."
  (let ((result nil) (done nil))
    (scm--request op args (lambda (r) (setq result r done t)))
    (let ((deadline (+ (float-time) scm-request-timeout)))
      (while (and (not done) (< (float-time) deadline) (scm-connected-p))
        (accept-process-output scm--conn 0.05)))
    result))

(defun scm--filter (_proc string)
  "Responses are one per line: the server escapes newlines inside strings."
  (setq scm--pending (concat scm--pending string))
  (while (string-match "\n" scm--pending)
    (let ((line (substring scm--pending 0 (match-beginning 0))))
      (setq scm--pending (substring scm--pending (match-end 0)))
      (unless (string-empty-p (string-trim line))
        (scm--dispatch (scm--parse-response line))))))

(defun scm--parse-response (line)
  "The response LINE as an Elisp plist.
Scheme's #t and #f outside strings become t and nil."
  (scm--log "<- %s" line)
  (let ((out (make-string 0 ?x)) (i 0) (n (length line)) (in-string nil))
    (while (< i n)
      (let ((c (aref line i)))
        (cond
         (in-string
          (setq out (concat out (string c)))
          (cond ((eq c ?\\) (cl-incf i) (when (< i n) (setq out (concat out (string (aref line i))))))
                ((eq c ?\") (setq in-string nil))))
         ((eq c ?\") (setq in-string t) (setq out (concat out "\"")))
         ((and (eq c ?#) (< (1+ i) n) (memq (aref line (1+ i)) '(?t ?f)))
          (setq out (concat out (if (eq (aref line (1+ i)) ?t) "t" "nil")))
          (cl-incf i))
         (t (setq out (concat out (string c))))))
      (cl-incf i))
    (condition-case err
        (car (read-from-string out))
      (error (scm--log "unparseable response: %s (%s)" line err) nil))))

(defun scm--dispatch (r)
  (when r
    (let* ((id (plist-get r :id))
           (cb (gethash id scm--callbacks)))
      (when cb
        (remhash id scm--callbacks)
        (funcall cb r)))))

;;;; Evaluation

(defun scm--defined-name (code)
  "The name CODE defines, if it is a `define' form."
  (when (string-match "\\`[ \t\n]*(define\\(?:-record-type\\)?[ \t\n]+(?[ \t\n]*\\([^][ \t\n()]+\\)" code)
    (match-string 1 code)))

(defun scm--show-result (r &optional echo code)
  "Print response R in the REPL; with ECHO, also in the echo area.
CODE is what was evaluated, so a definition can be echoed by name as
SLIME does, instead of as the unspecified value it returns."
  (let ((output (or (plist-get r :output) ""))
        (status (plist-get r :status)))
    ;; Unquoted wire values read as symbols: `ok', `error', `incomplete'.
    (pcase status
      ('ok (scm--repl-insert-result output (plist-get r :value))
       (when echo
         (let ((value (plist-get r :value)))
           (cond ((not (equal value "#<unspecified>")) (message "%s" value))
                 ((and code (scm--defined-name code)) (message "%s" (scm--defined-name code)))
                 (t (message "; No value"))))))
      ('error
       (scm--repl-insert-error output (plist-get r :message) (plist-get r :backtrace))
       (when echo (message "scm: %s" (plist-get r :message))))
      (_ (scm--repl-insert-error "" (format "%s" r) nil)))))

(defun scm-eval-string (code &optional source echo)
  "Evaluate CODE at top level, named SOURCE in backtraces."
  (scm-ensure-connected
   (lambda ()
     (scm--request 'eval (append (list :code code) (when source (list :source source)))
                   (lambda (r) (scm--show-result r echo code))))))

(defun scm-eval-last-sexp ()
  "Evaluate the sexp before point; the value goes to the echo area and the REPL."
  (interactive)
  (scm-eval-string (buffer-substring-no-properties (save-excursion (backward-sexp) (point)) (point))
                   (scm--source-name) t))

(defun scm-eval-defun ()
  "Evaluate the top-level form around point."
  (interactive)
  (save-excursion
    (end-of-defun)
    (let ((end (point)))
      (beginning-of-defun)
      (scm-eval-string (buffer-substring-no-properties (point) end) (scm--source-name) t))))

(defun scm-eval-region (start end)
  "Evaluate the region."
  (interactive "r")
  (scm-eval-string (buffer-substring-no-properties start end) (scm--source-name) t))

(defun scm-eval-buffer ()
  "Evaluate the whole buffer, as `load' would."
  (interactive)
  (scm-eval-string (buffer-substring-no-properties (point-min) (point-max)) (scm--source-name) t))

(defun scm--source-name ()
  (or (and buffer-file-name (file-name-nondirectory buffer-file-name)) (buffer-name)))

(defun scm-interrupt ()
  "Stop whatever the server is evaluating, over the control connection."
  (interactive)
  (if (and scm--control (process-live-p scm--control))
      (progn (scm--send scm--control 'interrupt nil nil) (message "scm: interrupt sent"))
    (user-error "scm: not connected")))

;;;; Introspection and completion

(defun scm--symbol-at-point ()
  (let ((s (thing-at-point 'symbol t))) (and s (substring-no-properties s))))

(defun scm-describe-symbol (name)
  "Describe NAME: kind, signature, docstring, current value."
  (interactive (list (let ((d (scm--symbol-at-point)))
                       (read-string (if d (format "Describe (%s): " d) "Describe: ") nil nil d))))
  (let ((r (scm--request-sync 'describe (list :name name))))
    (cond ((null r) (message "scm: no answer"))
          ((not (plist-get r :bound)) (message "%s is unbound" name))
          (t (with-current-buffer (get-buffer-create "*scm-doc*")
               (let ((inhibit-read-only t))
                 (erase-buffer)
                 (insert (or (plist-get r :signature) name) "\n")
                 (insert (plist-get r :kind) "\n\n")
                 (when (plist-get r :doc) (insert (plist-get r :doc) "\n\n"))
                 (insert "value: " (or (plist-get r :value) "") "\n")
                 (goto-char (point-min))
                 (special-mode))
               (display-buffer (current-buffer)))))))

(defun scm--candidates (prefix)
  (when (scm-connected-p)
    (plist-get (scm--request-sync 'complete (list :prefix prefix)) :candidates)))

(defun scm-complete-at-point ()
  "A `completion-at-point-functions' entry: global names from the server."
  (let ((bounds (bounds-of-thing-at-point 'symbol)))
    (when (and bounds (scm-connected-p))
      (list (car bounds) (cdr bounds)
            (completion-table-dynamic #'scm--candidates)
            :exclusive 'no))))

(defvar scm--eldoc-cache (make-hash-table :test 'equal))

(defun scm-eldoc-function (callback &rest _)
  "Signature of the symbol at point, cached; asks the server once per name."
  (when-let ((name (scm--symbol-at-point)))
    (let ((hit (gethash name scm--eldoc-cache 'miss)))
      (cond ((not (eq hit 'miss)) hit)
            ((scm-connected-p)
             (scm--request 'describe (list :name name)
                           (lambda (r)
                             (let ((sig (and (plist-get r :bound) (plist-get r :signature))))
                               (puthash name sig scm--eldoc-cache)
                               (when sig (funcall callback sig :thing name)))))
             nil)))))

;;;; The editing mode

(defvar scm-mode-map
  (let ((m (make-sparse-keymap)))
    (define-key m (kbd "C-x C-e") #'scm-eval-last-sexp)
    (define-key m (kbd "C-c C-c") #'scm-eval-defun)
    (define-key m (kbd "C-c C-r") #'scm-eval-region)
    (define-key m (kbd "C-c C-k") #'scm-eval-buffer)
    (define-key m (kbd "C-c C-z") #'scm-switch-to-repl)
    (define-key m (kbd "C-c C-d") #'scm-describe-symbol)
    (define-key m (kbd "C-c C-b") #'scm-interrupt)
    (define-key m (kbd "C-c C-q") #'scm-quit)
    (define-key m (kbd "C-M-x") #'scm-eval-defun)
    m))

;;;###autoload
(define-derived-mode scm-mode scheme-mode "scm"
  "Scheme as scm speaks it, connected to a running scm."
  (add-hook 'completion-at-point-functions #'scm-complete-at-point nil t)
  (add-hook 'eldoc-documentation-functions #'scm-eldoc-function nil t)
  (font-lock-add-keywords
   nil '(("\\[" . 'font-lock-bracket-face) ("\\]" . 'font-lock-bracket-face)
         ("\\_<&\\(opt\\|named\\|keys\\)\\_>" . 'font-lock-keyword-face)
         ("\\_<:[^ \t\n()\\[\\]]+" . 'font-lock-constant-face)))
  (when scm-connect-automatically
    (scm-ensure-connected)))

(defun scm--no-geiser ()
  "Keep Geiser out of scm buffers.
Doom's scheme module hooks `geiser-mode' onto `scheme-mode', which
`scm-mode' derives from; the parent's hooks run first, so this undoes it."
  (when (bound-and-true-p geiser-mode)
    (geiser-mode -1)))

(add-hook 'scm-mode-hook #'scm--no-geiser 90)

;;;###autoload
(add-to-list 'auto-mode-alist '("\\.scm\\'" . scm-mode))

;;;; The REPL buffer

(defvar scm--repl-prompt "scm> ")
(defvar-local scm--repl-input-start nil "Marker: where the current input begins.")
(defvar scm--repl-history nil)
(defvar scm--repl-history-index nil)

(defun scm--repl-buffer () (get-buffer-create "*scm*"))

(defvar scm-repl-mode-map
  (let ((m (make-sparse-keymap)))
    (define-key m (kbd "RET") #'scm-repl-return)
    (define-key m (kbd "M-p") #'scm-repl-previous-input)
    (define-key m (kbd "M-n") #'scm-repl-next-input)
    (define-key m (kbd "C-c C-b") #'scm-interrupt)
    (define-key m (kbd "C-c C-d") #'scm-describe-symbol)
    (define-key m (kbd "C-c C-q") #'scm-quit)
    (define-key m (kbd "TAB") #'completion-at-point)
    m))

(define-derived-mode scm-repl-mode scheme-mode "scm-repl"
  "The scm REPL: a buffer of prompts over the protocol."
  (setq-local scm--repl-input-start (make-marker))
  (add-hook 'completion-at-point-functions #'scm-complete-at-point nil t)
  (setq-local comment-start ";")
  (setq-local indent-tabs-mode nil)
  ;; The indenter parses from the "beginning of defun": here that is the
  ;; start of the current input, so earlier output never confuses it and
  ;; columns stay real, prompt included.
  (setq-local beginning-of-defun-function
              (lambda (&optional _arg)
                (goto-char (marker-position scm--repl-input-start))
                t)))

(defun scm--repl-ensure ()
  (with-current-buffer (scm--repl-buffer)
    (unless (derived-mode-p 'scm-repl-mode)
      (scm-repl-mode)
      (insert (format ";; scm %s — C-c C-b interrupts, M-p history, TAB completes\n" (or scm--version "")))
      (scm--repl-prompt))))

(defun scm--repl-prompt ()
  (let ((inhibit-read-only t))
    (goto-char (point-max))
    (unless (bolp) (insert "\n"))
    (let ((start (point)))
      (insert scm--repl-prompt)
      (add-text-properties start (point) '(read-only t rear-nonsticky t front-sticky (read-only) face font-lock-keyword-face)))
    (set-marker scm--repl-input-start (point))))

(defun scm--repl-insert (text &optional face)
  "Insert TEXT before the prompt line of the REPL."
  (with-current-buffer (scm--repl-buffer)
    (let ((inhibit-read-only t)
          (at-end (= (point) (point-max))))
      (save-excursion
        (goto-char (marker-position scm--repl-input-start))
        (beginning-of-line)
        (let ((start (point)))
          (insert text)
          (when face (add-text-properties start (point) (list 'face face)))))
      (when at-end (goto-char (point-max))))))

(defun scm--repl-insert-result (output value)
  (scm--repl-insert (concat output (unless (or (string-empty-p output) (string-suffix-p "\n" output)) "\n")
                            (if (equal value "#<unspecified>") "" (concat value "\n")))))

(defun scm--repl-insert-error (output message backtrace)
  (scm--repl-insert (concat output (unless (or (string-empty-p output) (string-suffix-p "\n" output)) "\n")))
  (scm--repl-insert (concat message "\n" (mapconcat (lambda (f) (concat "  in " f "\n")) backtrace ""))
                    'font-lock-warning-face))

(defun scm--repl-input ()
  (buffer-substring-no-properties (marker-position scm--repl-input-start) (point-max)))

(defun scm-repl-return ()
  "Send the input if the server says it is a complete form; else a newline."
  (interactive)
  (unless (scm-connected-p) (user-error "scm: not connected (M-x scm-start)"))
  (let ((input (scm--repl-input)))
    (if (string-blank-p input)
        (scm--repl-prompt)
      (let ((framing (plist-get (scm--request-sync 'parse (list :code input)) :framing)))
        (if (eq framing 'incomplete)
            (scm--repl-newline-and-indent)
          (let ((inhibit-read-only t))
            (goto-char (point-max))
            (insert "\n")
            (add-text-properties (marker-position scm--repl-input-start) (point-max) '(read-only t rear-nonsticky t)))
          (push input scm--repl-history)
          (setq scm--repl-history-index nil)
          (scm--repl-prompt)
          (scm--request 'eval (list :code input :source "<repl>")
                        (lambda (r) (scm--show-result r))))))))

(defun scm--repl-newline-and-indent ()
  "A newline inside an unfinished form, indented as `scheme-mode' would.
Only the current input is parsed: earlier output in the buffer is not
Scheme and must not influence the indentation."
  (goto-char (point-max))
  (insert "\n")
  (lisp-indent-line))

(defun scm--repl-replace-input (text)
  (let ((inhibit-read-only t))
    (delete-region (marker-position scm--repl-input-start) (point-max))
    (goto-char (point-max))
    (insert text)))

(defun scm-repl-previous-input ()
  "Walk back through the REPL history."
  (interactive)
  (when scm--repl-history
    (setq scm--repl-history-index (min (1- (length scm--repl-history)) (1+ (or scm--repl-history-index -1))))
    (scm--repl-replace-input (nth scm--repl-history-index scm--repl-history))))

(defun scm-repl-next-input ()
  "Walk forward through the REPL history."
  (interactive)
  (when scm--repl-history-index
    (if (= scm--repl-history-index 0)
        (progn (setq scm--repl-history-index nil) (scm--repl-replace-input ""))
      (cl-decf scm--repl-history-index)
      (scm--repl-replace-input (nth scm--repl-history-index scm--repl-history)))))

(defun scm-switch-to-repl ()
  "Show the REPL and put point at its prompt."
  (interactive)
  (scm-ensure-connected)
  (scm--repl-ensure)
  (pop-to-buffer (scm--repl-buffer))
  (goto-char (point-max)))

;;;###autoload
(defun scm-repl ()
  "Start scm, or show its REPL."
  (interactive)
  (scm-switch-to-repl))

(provide 'scm-mode)
;;; scm-mode.el ends here
