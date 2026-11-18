;;;; serve.lisp — an SBCL REPL server speaking scm's REPL protocol
;;;; (docs/sbcl-plan.md, L1; the protocol is docs/repl.md).
;;;;
;;;;     sbcl --noinform --disable-debugger --load serve.lisp \
;;;;          --end-toplevel-options 127.0.0.1:PORT
;;;;
;;;; The editor's socket backend (edit's scm_binding/remote.rs) is a client
;;;; of `scm --serve`; answering the same requests here makes SBCL a backend
;;;; without a second client, as edit/guile/serve.scm does for Guile. A send
;;;; evaluates in THIS image and answers as an scm one does — the values
;;;; printed, what the forms printed, the compiler's warnings, or an error's
;;;; message and the calls it was raised in — and completion and describe
;;;; read this image.
;;;;
;;;; A request is one datum, `(:op eval :id 7 :code "…")`, its strings in
;;;; the wire's Scheme spelling (`\n`, `\t`): Common Lisp's reader would
;;;; read `\n` as `n`, so requests are read by a small reader of their own
;;;; here, which also interns nothing a client sends. Each connection is
;;;; served on its own thread, so `:interrupt` from a second connection
;;;; reaches an evaluation running on the first — the protocol's rule.
;;;;
;;;; An error does not unwind: it is a BREAK, served over the connection
;;;; (L7). The request is answered with the message, the calls and their
;;;; locals, and the restarts, numbered; the connection's next requests are
;;;; then evaluated INSIDE the error, a level deeper, until one takes a
;;;; restart — a number (`1`, or `1 (+ 2 3)` for a restart that wants a
;;;; value), `:abort` to give the innermost send up, `:top` all of them.
;;;; Whatever finishes answers the request last read, so a USE-VALUE's
;;;; request is answered with the original send's value. SBCL's own
;;;; debugger never opens: there is no terminal for it to read from.

(eval-when (:compile-toplevel :load-toplevel :execute)
  (require :sb-bsd-sockets)
  (require :sb-introspect))

(defpackage :edit-serve
  (:use :cl)
  (:export #:main #:in #:complete-in #:describe-in #:read-check #:definitions-in #:*break-on-error*))

(in-package :edit-serve)

;;; ---- the wire -----------------------------------------------------------

(defun wire-string (s)
  "S as the wire writes a string: Scheme's escapes, as scm's push_string."
  (with-output-to-string (out)
    (write-char #\" out)
    (loop for c across s
          do (case c
               (#\" (write-string "\\\"" out))
               (#\\ (write-string "\\\\" out))
               (#\Newline (write-string "\\n" out))
               (#\Return (write-string "\\r" out))
               (#\Tab (write-string "\\t" out))
               (t (write-char c out))))
    (write-char #\" out)))

(defun wire-list (strings)
  (format nil "(~{~a~^ ~})" (mapcar #'wire-string strings)))

(defun parse-request (text)
  "TEXT, one request, as an alist of (\"key\" . value): a value is a string,
an integer, T or NIL for #t/#f, or a symbol's name as a string. Reads only
what a request is made of, and interns nothing."
  (let ((i 0) (n (length text)))
    (labels ((peek () (and (< i n) (char text i)))
             (skip ()
               (loop while (and (< i n) (member (char text i) '(#\Space #\Tab #\Newline #\Return)))
                     do (incf i)))
             (fail (why) (error "malformed request: ~a" why))
             (str ()
               (incf i)
               (with-output-to-string (out)
                 (loop
                   (when (>= i n) (fail "a string never closed"))
                   (let ((c (char text i)))
                     (incf i)
                     (cond ((char= c #\") (return))
                           ((char= c #\\)
                            (when (>= i n) (fail "a string never closed"))
                            (let ((e (char text i)))
                              (incf i)
                              (write-char (case e (#\n #\Newline) (#\r #\Return) (#\t #\Tab) (t e)) out)))
                           (t (write-char c out)))))))
             (token ()
               (let ((start i))
                 (loop while (and (< i n) (not (member (char text i) '(#\Space #\Tab #\Newline #\Return #\( #\) #\"))))
                       do (incf i))
                 (subseq text start i)))
             (item ()
               (skip)
               (case (peek)
                 ((nil) (fail "it ended early"))
                 (#\" (str))
                 (#\( (incf i)
                  (let ((out '()))
                    (loop (skip)
                          (when (null (peek)) (fail "a list never closed"))
                          (when (eql (peek) #\)) (incf i) (return (nreverse out)))
                          (push (item) out))))
                 (#\) (fail "an unmatched )"))
                 (t (let ((tok (token)))
                      (cond ((string= tok "#t") t)
                            ((string= tok "#f") nil)
                            ((and (plusp (length tok)) (every #'digit-char-p tok)) (parse-integer tok))
                            (t tok)))))))
      (skip)
      (unless (eql (peek) #\() (fail "a request is a list"))
      (let ((items (item)) (alist '()))
        (loop while items
              do (let ((k (pop items)))
                   (unless (and (stringp k) (plusp (length k)) (char= (char k 0) #\:))
                     (fail "a request is :key value pairs"))
                   (push (cons (string-downcase (subseq k 1)) (pop items)) alist)))
        alist))))

(defun field (req key) (cdr (assoc key req :test #'string=)))

;;; ---- printing what the image says ---------------------------------------

(defun safely (thunk fallback)
  "THUNK's value, or FALLBACK if printing it fails: a broken PRINT-OBJECT
method must not cost the answer."
  (handler-case (funcall thunk)
    (error () fallback)))

(defun printed (v)
  (safely (lambda () (prin1-to-string v)) "#<unprintable>"))

(defun message-of (c)
  "C's report on one line: SBCL lays a report out over several (\"The
value~%  5~%is not of type~%  LIST\"), and an editor's echo line shows the
first. The `Stream: …` paragraph its reader errors end in is taken off."
  (let* ((text (safely (lambda () (princ-to-string c))
                       (format nil "~a (unprintable report)" (type-of c))))
         (cut (search (format nil "~%~%") text))
         (text (string-trim '(#\Space #\Newline) (if cut (subseq text 0 cut) text))))
    (with-output-to-string (out)
      (let ((space nil))
        (loop for ch across text
              do (if (member ch '(#\Space #\Newline #\Tab))
                     (setf space t)
                     (progn (when space (write-char #\Space out) (setf space nil))
                            (write-char ch out))))))))

(defun lowercase-name (s)
  "S as a reader would have to be given it, lower case: a name with lower case
letters of its own is written |Like This|."
  (let ((name (symbol-name s)))
    (if (some #'lower-case-p name)
        (format nil "|~a|" name)
        (string-downcase name))))

(defun lambda-list-text (name list)
  "\"(name a b &rest c)\": names lower case, and no package prefixes."
  (labels ((walk (x)
             (cond ((and (symbolp x) (keywordp x)) (format nil ":~a" (string-downcase (symbol-name x))))
                   ((symbolp x) (string-downcase (symbol-name x)))
                   ((consp x) (format nil "(~{~a~^ ~})" (mapcar #'walk (proper-part x))))
                   (t (printed x))))
           (proper-part (x)
             (loop for tail on x collect (car tail)
                   unless (listp (cdr tail)) collect '|.| and collect (cdr tail))))
    (format nil "(~a~{ ~a~})" name (mapcar #'walk (proper-part list)))))

;;; ---- the evaluating threads, for :interrupt -----------------------------

(defvar *lock* (sb-thread:make-mutex :name "edit-serve"))
(defvar *evaluating* '()
  "The threads evaluating a request right now — once for each request, so
a thread evaluating inside a break is here twice.")

(defun interrupt-all ()
  (sb-thread:with-mutex (*lock*)
    (dolist (th (remove-duplicates *evaluating*))
      (ignore-errors
       (sb-thread:interrupt-thread
        th #'interrupted)))))

(defvar *reading* nil
  "Whether this thread is waiting for its connection's next request — in a
break, between requests — which an interrupt leaves alone.")

(defun interrupted ()
  ;; A break with the evaluation where it was: CONTINUE goes on with it.
  (unless *reading*
    (with-simple-restart (continue "Go on with the evaluation.")
      (enter-break (make-condition 'simple-error :format-control "interrupted") t))))

;;; ---- eval ---------------------------------------------------------------

(defvar *frames-limit* 40)
(defvar *form* nil "The form being evaluated, for its frame.")

(defun server-frame-p (name)
  "Whether a frame named NAME is the server's or the signalling machinery's,
not the user's: the server's own functions and lambdas, and what stands
between a condition and its handler."
  (labels ((ours (x)
             (cond ((symbolp x) (eq (symbol-package x) (find-package :edit-serve)))
                   ((consp x) (or (ours (car x)) (ours (cdr x)))))))
    (or (ours name)
        (member name '(eval sb-int:simple-eval-in-lexenv sb-kernel::%signal signal
                       sb-debug::run-hook invoke-debugger sb-int:%break)))))

(defun frame-locals (frame)
  "FRAME's locals as ((name printed) …): those its code kept, which is
more of them under (debug 2)."
  (let ((fun (sb-di:frame-debug-fun frame))
        (at (ignore-errors (sb-di:frame-code-location frame))))
    (when (and at (sb-di:debug-var-info-available fun))
      (let ((*print-length* 5) (*print-level* 3) (*print-pretty* nil))
        (loop for v in (ignore-errors (sb-di:ambiguous-debug-vars fun ""))
              for name = (symbol-name (sb-di:debug-var-symbol v))
              when (and (plusp (length name))
                        (eq (ignore-errors (sb-di:debug-var-validity v at)) :valid))
                collect (list (string-downcase name)
                              (let ((text (safely (lambda () (prin1-to-string (sb-di:debug-var-value v frame)))
                                                  "#<unprintable>")))
                                (if (> (length text) 200) (concatenate 'string (subseq text 0 200) "…") text))))))))

(defun frames ()
  "The calls from the one that signalled out to the request, as ((text
locals) …) — the debugger's frames, cut where the server's own begin."
  (let ((out '()))
    (block walk
      (sb-debug:map-backtrace
       (lambda (frame)
         (let ((name (ignore-errors (sb-di:debug-fun-name (sb-di:frame-debug-fun frame)))))
           (when (member name '(eval-forms take-restart)) (return-from walk))
           (unless (server-frame-p name)
             (push (list (safely (lambda ()
                                   (let ((*print-length* 5) (*print-level* 3) (*print-pretty* nil))
                                     (if (equal name '(lambda ()))
                                         ;; A top-level form, which EVAL compiled into a
                                         ;; function with no name: show the form.
                                         (prin1-to-string *form*)
                                         (with-output-to-string (s) (sb-debug::print-frame-call frame s)))))
                                 "#<a frame>")
                         (safely (lambda () (frame-locals frame)) '()))
                   out)
             (when (>= (length out) *frames-limit*) (return-from walk)))))
       ;; From the frame that signalled, named outright: by default the
       ;; walk starts at the newest frame a trap interrupted, which inside
       ;; a `(car 5)` break is that one's, whatever signalled since.
       :from (or (ignore-errors (sb-debug::resolve-stack-top-hint)) :debugger-frame)))
    (nreverse out)))

;;; ---- the connection's state ---------------------------------------------
;;; Each bound per connection (serve-connection) or per break (enter-break).

(defvar *stream* nil "The connection's stream: a break answers on it.")
(defvar *owed* 0
  "The id of the request last read on this connection — the one the next
answer goes to. A restart that lets a send finish answers the restart's own
request with the send's value.")
(defvar *level* 0 "How many breaks deep the connection is.")
(defvar *restarts* '() "The innermost break's restarts, as numbered.")
(defvar *asking* nil "A restart of the innermost break waiting for its value's form.")
(defvar *top-abort* nil "The ABORT that gives up the outermost send: :top.")
(defvar *out* nil "Where the request being evaluated prints.")
(defvar *warnings* '()
  "The compiler's warnings for it, newest first, each (line . start): the
line as the output has it, and where the form it was said of begins in the
text it was sent in (*FORM-START*).")
(defvar *form-start* 0
  "Where the form being evaluated begins in the text eval-forms was given —
the user's own, inside `in` — as a character offset, past blank lines and
comments: a warning is marked at its form in the buffer (L6).")
(defvar *break-on-error* t
  "Whether an error is a break (the default) or answers and unwinds at once,
as L1 did.")

(defun reply (text)
  (write-string text *stream*)
  (terpri *stream*)
  (finish-output *stream*))

(defun shortest-name (pkg)
  (reduce (lambda (a b) (if (< (length b) (length a)) b a))
          (package-nicknames pkg) :initial-value (package-name pkg)))

(defun eval-reply (status &key value message host-stopped frames restarts asking)
  "An eval's answer, to the request owed one: what was printed and the
compiler's warnings since the last answer, the break level and the package
evaluation is in now, and the rest as STATUS has it."
  (let ((output (get-output-stream-string *out*))
        (warnings (reverse *warnings*)))
    (setf *warnings* '())
    (with-output-to-string (s)
      (format s "(:id ~a :status ~(~a~)" *owed* status)
      (if (eq status :ok)
          (format s " :value ~a" (wire-string value))
          (format s " :message ~a :host-stopped ~a :backtrace ~a :locals (~{~a~^ ~})"
                  (wire-string message) (if host-stopped "#t" "#f")
                  (wire-list (mapcar #'first frames))
                  (mapcar (lambda (f) (format nil "(~{~a~^ ~})" (mapcar #'wire-list (second f)))) frames)))
      (when restarts (format s " :restarts ~a" (wire-list restarts)))
      (when asking (format s " :asking ~a" (wire-string asking)))
      (format s " :output ~a :warnings ~a :warning-at (~{~a~^ ~}) :level ~a :package ~a)"
              (wire-string output) (wire-list (mapcar #'car warnings)) (mapcar #'cdr warnings)
              *level* (wire-string (shortest-name *package*))))))

;;; ---- errors and breaks --------------------------------------------------

(defun failed (message &key host-stopped (backtrace (frames)))
  "Give the request up, answering MESSAGE; never returns."
  (throw 'request (list :error message host-stopped backtrace)))

(defun fail-from-condition (c)
  (failed (message-of c)))

(defun restart-text (r)
  (format nil "[~a] ~a" (restart-name r) (message-of r)))

(defun break-restarts (c)
  "C's restarts, innermost first, as far as the one that gives up the
outermost send: those past it are the server's own."
  (let ((all (compute-restarts c)))
    (subseq all 0 (min (length all) (1+ (or (position *top-abort* all) (1- (length all))))))))

(defun enter-break (c &optional host-stopped)
  "Answer the request with C — its message, calls, locals and restarts —
and serve the connection's next requests from here, a level deeper, until
one takes a restart. Never returns."
  (unless *break-on-error*
    (failed (message-of c) :host-stopped host-stopped))
  (let* ((frames (frames))
         (restarts (break-restarts c))
         (*level* (1+ *level*))
         (*restarts* restarts)
         (*asking* nil))
    (reply (eval-reply :error :message (message-of c) :host-stopped host-stopped
                              :frames frames :restarts (mapcar #'restart-text restarts)))
    (serve-requests)))

(defun command-of (code)
  "What CODE asks of a break: (N), (N . \"value form\"), :abort or :top —
or NIL, for code to evaluate."
  (let* ((text (string-trim '(#\Space #\Tab #\Newline #\Return) code))
         (digits (position-if-not #'digit-char-p text)))
    (cond ((member text '(":abort" ":a" ":q") :test #'string-equal) :abort)
          ((string-equal text ":top") :top)
          ((and (plusp (length text)) (null digits)) (list (parse-integer text)))
          ((and digits (plusp digits) (member (char text digits) '(#\Space #\Tab)))
           (cons (parse-integer text :end digits) (subseq text digits))))))

(defun take-restart (r &optional value-text)
  "Invoke R; one that wants a value reads its form from VALUE-TEXT."
  (if value-text
      (let ((*query-io* (make-two-way-stream (make-string-input-stream value-text) (make-broadcast-stream))))
        (invoke-restart-interactively r))
      (invoke-restart r)))

(defun run (code)
  "A request's code, in a break perhaps: a command takes a restart, and
anything else evaluates inside the error. The values, or :asking."
  (let ((cmd (and (plusp *level*) (command-of code))))
    (flet ((our-abort () (or (find 'abort *restarts* :key #'restart-name) *top-abort*)))
      (cond ((eq cmd :abort) (invoke-restart (our-abort)))
            ((eq cmd :top) (invoke-restart *top-abort*))
            (*asking* (let ((r *asking*)) (setf *asking* nil) (take-restart r code)))
            (cmd
             (let ((r (nth (car cmd) *restarts*)))
               (cond ((null r) (failed (format nil "there is no restart ~a: 0 to ~a" (car cmd) (1- (length *restarts*)))
                                       :backtrace '()))
                     ((cdr cmd) (take-restart r (cdr cmd)))
                     ((sb-kernel::restart-interactive-function r)
                      ;; It wants a value: the next request is its form.
                      (setf *asking* r)
                      :asking)
                     (t (take-restart r)))))
            (t (eval-forms code))))))

(defun eval-forms (code)
  "Every form of CODE, in order, in *PACKAGE*; the last one's values."
  (let ((in (make-string-input-stream code))
        (values '()))
    (loop
      (let* ((*form-start* (skip-blank code (file-position in)))
             (form (handler-case (read in nil in)
                    ;; What the send's text is, not what it did: no calls.
                    (end-of-file () (failed "end of file: a form is not closed" :backtrace '()))
                    (reader-error (c) (failed (message-of c) :backtrace '())))))
        (when (eq form in) (return values))
        (setf values (let ((*form* form)) (multiple-value-list (eval form))))
        ;; The REPL's history variables, as SBCL's own REPL keeps them.
        (setf *** ** ** * * (first values)
              /// // // / / values
              +++ ++ ++ + + form)))))

(defvar *print-package* nil
  "The package a send's values print in: the one `in` evaluated it in, so
a buffer's symbols print as that buffer would write them.")

(defun values-text (values)
  (let ((*package* (or *print-package* *package*)))
    (if values
        (format nil "~{~a~^, ~}" (mapcar #'printed values))
        "; No values")))

(defun answer-eval (code)
  (let* ((*out* (make-string-output-stream))
         (*warnings* '())
         (no-input (make-string-input-stream ""))
         (io (make-two-way-stream no-input *out*))
         (level *level*)
         (result
           (catch 'request
             (restart-case
                 (let ((*standard-output* *out*) (*error-output* *out*) (*trace-output* *out*) (*print-package* nil)
                       (*standard-input* no-input) (*terminal-io* io) (*query-io* io) (*debug-io* io)
                       (*top-abort* (if (zerop level) (find-restart 'abort) *top-abort*))
                       ;; Where a backtrace starts (frames): an internal error
                       ;; (`(car 5)`'s trap) leaves its frame here for as long as
                       ;; its break lasts, and a request inside it has its own.
                       (sb-debug:*stack-top-hint* nil)
                       ;; Whatever would open the debugger is a break instead.
                       (sb-ext:*invoke-debugger-hook* (lambda (c hook) (declare (ignore hook)) (enter-break c)))
                       (*debugger-hook* (lambda (c hook) (declare (ignore hook)) (enter-break c))))
                   (sb-thread:with-mutex (*lock*) (push sb-thread:*current-thread* *evaluating*))
                   (unwind-protect
                        (handler-bind ((sb-kernel:redefinition-warning
                                         ;; Sending a defun again is how a file is
                                         ;; worked on here: "redefining F" says
                                         ;; nothing that was not meant.
                                         #'muffle-warning)
                                       (warning
                                         (lambda (w)
                                           ;; The compiler's verdict on what was sent, kept
                                           ;; with the output and said once.
                                           (let ((line (format nil "; ~a: ~a"
                                                               (if (typep w 'style-warning) "style-warning" "warning")
                                                               (message-of w))))
                                             (push (cons line *form-start*) *warnings*)
                                             (format *out* "~a~%" line))
                                           (muffle-warning w)))
                                       ;; Too deep to serve a break from: unwind.
                                       (storage-condition #'fail-from-condition)
                                       (error #'enter-break))
                          (let ((values (run code)))
                            (if (eq values :asking)
                                (list :asking (restart-text *asking*))
                                (list :ok (values-text values)))))
                     (sb-thread:with-mutex (*lock*)
                       (setf *evaluating* (remove sb-thread:*current-thread* *evaluating* :count 1)))))
               (abort ()
                 :report (lambda (s)
                           (if (zerop level)
                               (write-string "Give this send up." s)
                               (format s "Return to break level ~a." level)))
                 (list :ok "; Evaluation aborted"))))))
    (destructuring-bind (status text &optional host-stopped backtrace) result
      (case status
        (:ok (eval-reply :ok :value text))
        (:asking (eval-reply :ok :value (format nil "; ~a — its value's form next" text) :asking text))
        (t (eval-reply :error :message text :host-stopped host-stopped
                              :frames (mapcar (lambda (f) (if (consp f) f (list f '()))) backtrace)))))))

;;; ---- parse: is the text whole? ------------------------------------------

(defun scratch-package ()
  "A package to read in that interns nothing in the user's: the same
use-list as *PACKAGE*, so `car` still reads as CL's."
  (make-package (format nil "EDIT-SERVE-SCRATCH-~a" (random most-positive-fixnum))
                :use (package-use-list *package*)))

(defun reading-readtable ()
  "The current readtable, but `#.` reads its form and evaluates nothing."
  (let ((rt (copy-readtable)))
    (set-dispatch-macro-character #\# #\. (lambda (s c n) (declare (ignore c n)) (read s t nil t) nil) rt)
    rt))

(defun framing (code)
  "complete, incomplete or invalid — the protocol's `parse`. Whether the
brackets close is the question: a name in a package that does not exist yet
is complete text."
  (let* ((pkg (scratch-package))
         (*package* pkg)
         (*readtable* (reading-readtable))
         (*read-eval* nil))
    (unwind-protect
         (handler-case
             (let ((in (make-string-input-stream code)))
               (handler-bind ((sb-int:simple-reader-package-error
                                (lambda (c)
                                  (let ((r (find-restart 'continue c)))
                                    (when r (invoke-restart r))))))
                 (loop (when (eq (read in nil in) in) (return "complete")))))
           (end-of-file () "incomplete")
           (reader-error () "invalid")
           (error () "invalid"))
      (delete-package pkg))))

;;; ---- read-check: what SBCL's reader cannot read (L4) ---------------------

(defun line-and-column (text position)
  "POSITION, a character index into TEXT, as (line column), both from 0."
  (let ((line 0) (start 0))
    (loop for i below (min position (length text))
          when (char= (char text i) #\Newline)
            do (incf line) (setf start (1+ i)))
    (list line (- position start))))

(defun read-check (text)
  "Read TEXT, a buffer, to its end as SBCL would load it — each form in the
package of the `in-package` before it — and answer T, or (line column
message) where the reader stopped: just past what it could not read, in
SBCL's words. Nothing is evaluated (`#.` reads its form and runs nothing)
and nothing is left behind: forms are read in a throwaway package with the
real one's use-list, and every other package is locked meanwhile, so
`pkg::new-name` cannot intern there."
  (let* ((in (make-string-input-stream text))
         (scratch (scratch-package))
         (*package* scratch)
         (*readtable* (reading-readtable))
         (*read-eval* nil)
         (locked '()))
    (unwind-protect
         (progn
           (dolist (p (list-all-packages))
             (unless (or (eq p scratch) (sb-ext:package-locked-p p))
               (sb-ext:lock-package p)
               (push p locked)))
           (loop
             (let* ((start (file-position in))
                    (form (handler-case
                              (handler-case (read in nil in)
                                ;; A name the reader would intern in a real
                                ;; package: read the form again for its syntax
                                ;; alone.
                                (sb-ext:package-locked-error ()
                                  (file-position in start)
                                  (let ((*read-suppress* t)) (read in nil in) nil)))
                            ((or reader-error end-of-file) (c)
                              (return (append (line-and-column text (file-position in))
                                              (list (if (typep c 'end-of-file) "end of file" (message-of c)))))))))
               (when (eq form in) (return t))
               ;; (in-package :shapes): the rest reads as SHAPES would see it.
               (when (and (consp form) (symbolp (car form)) (string= (symbol-name (car form)) "IN-PACKAGE")
                          (consp (cdr form)) (typep (cadr form) '(or string symbol character)))
                 (let ((pkg (find-package (string (cadr form)))))
                   (when pkg
                     (let ((next (make-package (format nil "EDIT-SERVE-SCRATCH-~a" (random most-positive-fixnum))
                                               :use (package-use-list pkg))))
                       (delete-package *package*)
                       (setf scratch next *package* next))))))))
      (dolist (p locked) (sb-ext:unlock-package p))
      (when (package-name *package*) (delete-package *package*)))))

;;; ---- complete and describe ----------------------------------------------

(defun meaningful-p (s)
  (or (fboundp s) (boundp s) (find-class s nil) (special-operator-p s)))

(defun completions (prefix)
  "The names PREFIX begins, sorted: the symbols *PACKAGE* can see that name
something, or with `pkg:` the package's external ones (`pkg::`, all of its
own), and package names themselves as `pkg:`."
  (let ((names '())
        (colon (position #\: prefix)))
    (flet ((starts (name part) (and (>= (length name) (length part))
                                    (string-equal part name :end2 (length part)))))
      (cond
        ((eql colon 0) nil)                 ; keywords: too many to be useful
        (colon
         (let* ((pkg-name (subseq prefix 0 colon))
                (double (and (< (1+ colon) (length prefix)) (char= (char prefix (1+ colon)) #\:)))
                (part (subseq prefix (+ colon (if double 2 1))))
                (pkg (find-package (string-upcase pkg-name))))
           (when pkg
             (flet ((take (s)
                      (when (and (starts (symbol-name s) part) (meaningful-p s))
                        (push (format nil "~a~a~a" (string-downcase pkg-name) (if double "::" ":") (lowercase-name s))
                              names))))
               (if double
                   (do-symbols (s pkg) (when (eq (symbol-package s) pkg) (take s)))
                   (do-external-symbols (s pkg) (take s)))))))
        (t
         (do-symbols (s *package*)
           (when (and (starts (symbol-name s) prefix) (meaningful-p s))
             (push (lowercase-name s) names)))
         (dolist (p (list-all-packages))
           (dolist (n (cons (package-name p) (package-nicknames p)))
             (when (and (starts n prefix) (plusp (length prefix)))
               (push (format nil "~a:" (string-downcase n)) names)))))))
    (sort (remove-duplicates names :test #'string=) #'string<)))

(defun find-named (name)
  "The symbol NAME names, as the reader would take it, without interning:
`foo`, `pkg:foo`, `pkg::foo`, `|Mixed|`. NIL when there is none."
  (let* ((colon (position #\: name))
         (pkg (if (and colon (plusp colon)) (find-package (string-upcase (subseq name 0 colon))) *package*))
         (base (if colon (string-left-trim ":" (subseq name colon)) name))
         (key (if (and (> (length base) 1) (char= (char base 0) #\|) (char= (char base (1- (length base))) #\|))
                  (subseq base 1 (1- (length base)))
                  (string-upcase base))))
    (and pkg (plusp (length key)) (find-symbol key (if (eql colon 0) (find-package :keyword) pkg)))))

(defun describe-parts (s)
  "What describe says of symbol S: (kind value signature doc), the last two
NIL when there are none."
  (let* ((fn (and (fboundp s) (not (special-operator-p s)) (not (macro-function s)) (fdefinition s)))
         (kind (cond ((special-operator-p s) "special operator")
                     ((macro-function s) "macro")
                     ((typep fn 'generic-function) "generic function")
                     (fn "procedure")
                     ((boundp s) (if (constantp s) "constant" "variable"))
                     (t "class")))
         (lambda-list (and (fboundp s)
                           (ignore-errors (sb-introspect:function-lambda-list
                                           (or (macro-function s) (fdefinition s))))))
         (signature (and (fboundp s) (lambda-list-text (lowercase-name s) lambda-list)))
         (doc (or (and (fboundp s) (documentation s 'function))
                  (and (boundp s) (documentation s 'variable))
                  (and (find-class s nil) (documentation s 'type))))
         (value (cond ((fboundp s) (printed (if (macro-function s) (macro-function s) (fdefinition s))))
                      ((boundp s) (let ((*print-length* 20) (*print-level* 4)) (printed (symbol-value s))))
                      (t (printed (find-class s))))))
    (list kind value signature doc)))

(defun answer-describe (id name)
  (let ((s (find-named name)))
    (if (not (and s (meaningful-p s)))
        (format nil "(:id ~a :status ok :bound #f)" id)
        (destructuring-bind (kind value signature doc) (describe-parts s)
          (format nil "(:id ~a :status ok :bound #t :name ~a :kind ~a :value ~a~@[ :signature ~a~]~@[ :doc ~a~])"
                  id (wire-string (lowercase-name s)) (wire-string kind) (wire-string value)
                  (and signature (wire-string signature)) (and doc (wire-string doc)))))))

;;; ---- a buffer's package -------------------------------------------------
;;; A send from a buffer is read and evaluated in the package of the
;;; nearest `(in-package …)` above it, as SLIME's are — the editor finds the
;;; name in the text and asks through an ordinary `eval` of these. The REPL
;;; panel's lines are evaluated in the connection's own *PACKAGE*.

(defun package-named (name)
  "The package NAME (as `find-package` takes it), else CL-USER and a note
saying so."
  (let ((pkg (and name (plusp (length name)) (find-package name))))
    (if pkg
        (values pkg nil)
        (values (find-package :cl-user)
                (format nil "package ~a does not exist yet: read in CL-USER" name)))))

(defun in (package code)
  "Evaluate CODE's forms in PACKAGE: the last one's values, printed as that
package would write them."
  (multiple-value-bind (pkg note) (package-named package)
    (when note (format t "; ~a~%" note))
    (setf *print-package* pkg)
    (let ((*package* pkg))
      (values-list (eval-forms code)))))

(defun complete-in (package prefix)
  "The names PREFIX begins, as `complete` answers them, seen from PACKAGE."
  (let ((*package* (package-named package)))
    (completions prefix)))

(defun describe-in (package name)
  "(kind signature doc) of NAME seen from PACKAGE — \"\" where there is
none — or NIL when it names nothing."
  (let* ((*package* (package-named package))
         (s (find-named name)))
    (and s (meaningful-p s)
         (destructuring-bind (kind value signature doc) (describe-parts s)
           (declare (ignore value))
           (list kind (or signature "") (or doc ""))))))

;;; ---- M-.: where a name is defined (L5) ----------------------------------

(defparameter *definition-types*
  '(:function :generic-function :method :macro :compiler-macro :variable :constant
    :class :structure :condition :type :setf-expander :symbol-macro))

(defun skip-blank (text at)
  "The first position from AT in TEXT that is not white space or a
comment: where a top-level form begins whose reading began at AT."
  (let ((n (length text)))
    (loop
      (cond ((>= at n) (return n))
            ((member (char text at) '(#\Space #\Tab #\Newline #\Return #\Page)) (incf at))
            ((char= (char text at) #\;)
             (setf at (or (position #\Newline text :start at) n)))
            ((and (char= (char text at) #\#) (< (1+ at) n) (char= (char text (1+ at)) #\|))
             (setf at (let ((end (search "|#" text :start2 (+ at 2)))) (if end (+ end 2) n))))
            (t (return at))))))

(defun file-line-column (path offset form-number)
  "Where in the file at PATH a definition begins, as (line column): from
character OFFSET when SBCL kept one — where reading its top-level form
began, so before the blank lines and comments above it — else at the start
of top-level form FORM-NUMBER, found by reading with *READ-SUPPRESS*,
which interns nothing."
  (with-open-file (in path :external-format '(:utf-8 :replacement #\?))
    (let ((text (make-string (file-length in))))
      (setf text (subseq text 0 (read-sequence text in)))
      (let ((at (or offset
                    (and form-number
                         (let ((s (make-string-input-stream text)) (*read-suppress* t))
                           (handler-case
                               (loop for n from 0
                                     do (when (= n form-number) (return (file-position s)))
                                        (read s nil nil))
                             (error () nil)))))))
        (and at (line-and-column text (skip-blank text at)))))))

(defun definitions-in (package name)
  "Where SBCL knows NAME (seen from PACKAGE) to be defined in a file:
((path line column kind) …), 0-based. A definition evaluated from a string
— an editor send — has no file, and is not here: the editor's own index of
the project's files finds those."
  (let* ((*package* (package-named package))
         (s (find-named name))
         (out '()))
    (when s
      (dolist (type *definition-types*)
        (dolist (src (ignore-errors (sb-introspect:find-definition-sources-by-name s type)))
          (let* ((pathname (ignore-errors (sb-introspect:definition-source-pathname src)))
                 (file (and pathname (ignore-errors (probe-file (translate-logical-pathname pathname))))))
            (when file
              (let ((at (ignore-errors
                         (file-line-column file
                                           (sb-introspect:definition-source-character-offset src)
                                           (first (sb-introspect:definition-source-form-path src))))))
                (when at
                  (pushnew (list (namestring file) (first at) (second at) (string-downcase (symbol-name type)))
                           out :test (lambda (a b) (and (equal (first a) (first b)) (eql (second a) (second b))))))))))))
    (nreverse out)))

;;; ---- the requests -------------------------------------------------------

(defun version ()
  (format nil "sbcl ~a" (lisp-implementation-version)))

(defun answer (req)
  (let ((id (or (field req "id") 0))
        (op (field req "op")))
    (cond
      ((equal op "eval") (answer-eval (or (field req "code") "")))
      ((equal op "complete")
       (format nil "(:id ~a :status ok :candidates ~a)" id (wire-list (completions (or (field req "prefix") "")))))
      ((equal op "describe") (answer-describe id (or (field req "name") "")))
      ((equal op "parse")
       (format nil "(:id ~a :status ok :framing ~a)" id (framing (or (field req "code") ""))))
      ((equal op "interrupt") (interrupt-all) (format nil "(:id ~a :status ok :version ~a)" id (wire-string (version))))
      ((member op '("ping" "stream") :test #'equal)
       (format nil "(:id ~a :status ok :version ~a)" id (wire-string (version))))
      (t (format nil "(:id ~a :status malformed :message ~a)" id
                 (wire-string (format nil "unknown op `~a`" op)))))))

;;; ---- the connection -----------------------------------------------------

(defun read-datum (stream)
  "One request's text off STREAM — up to where its outer list closes,
skipping brackets inside strings — or NIL at the end."
  (let ((out (make-string-output-stream)) (depth 0) (started nil))
    (loop
      (let ((c (read-char stream nil nil)))
        (unless c (return nil))
        (when (or started (char= c #\())
          (write-char c out))
        (case c
          (#\( (setf started t) (incf depth))
          (#\) (when started
                 (decf depth)
                 (when (zerop depth) (return (get-output-stream-string out)))))
          (#\" (when started
                 (loop (let ((d (read-char stream nil nil)))
                         (unless d (return-from read-datum nil))
                         (write-char d out)
                         (cond ((char= d #\\) (let ((e (read-char stream nil nil)))
                                                (unless e (return-from read-datum nil))
                                                (write-char e out)))
                               ((char= d #\") (return))))))))))))

(defun serve-requests ()
  "Answer the connection's requests, one at a time, until it closes —
from the top, or from inside a break."
  (loop
    (let ((text (let ((*reading* t)) (handler-case (read-datum *stream*) (error () nil)))))
      (unless text (throw 'closed nil))
      (reply (handler-case (let ((req (parse-request text)))
                             (setf *owed* (or (field req "id") 0))
                             (answer req))
               ;; A bug here answers malformed rather than killing the
               ;; connection — under the request's own :id if it has one.
               (error (c)
                 (format nil "(:id ~a :status malformed :message ~a)"
                         (or (ignore-errors (field (parse-request text) "id")) 0)
                         (wire-string (message-of c)))))))))

(defun serve-connection (socket)
  (let ((*stream* (sb-bsd-sockets:socket-make-stream socket :input t :output t
                                                             :external-format :utf-8 :buffering :full))
        (*owed* 0)
        (*level* 0)
        (*package* (find-package :cl-user)))
    (unwind-protect
         ;; A connection that closes in a break leaves every level at once.
         (catch 'closed (serve-requests))
      (ignore-errors (close *stream*))
      (ignore-errors (sb-bsd-sockets:socket-close socket)))))

(defun main (&optional (addr (car (last sb-ext:*posix-argv*))))
  (let* ((colon (position #\: addr :from-end t))
         (host (subseq addr 0 colon))
         (port (parse-integer addr :start (1+ colon)))
         (listener (make-instance 'sb-bsd-sockets:inet-socket :type :stream :protocol :tcp)))
    (setf (sb-bsd-sockets:sockopt-reuse-address listener) t)
    (sb-bsd-sockets:socket-bind listener (sb-bsd-sockets:make-inet-address host) port)
    (sb-bsd-sockets:socket-listen listener 8)
    (loop
      (let ((conn (sb-bsd-sockets:socket-accept listener)))
        (sb-thread:make-thread (lambda () (serve-connection conn)) :name "edit-serve connection")))))

;; Serving is what loading this file is for -- but not when it is only
;; being compiled.
(unless (member :edit-serve-compile-only *features*)
  (main))
