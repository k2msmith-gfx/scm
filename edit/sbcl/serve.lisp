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
;;;; The debugger never opens. An error, a `break` or anything else that
;;;; would enter it answers the request instead, with the message and the
;;;; calls; a server has no terminal for the debugger to read a choice from,
;;;; and waiting on one would hang the send. (L7 replaces this with a break
;;;; served over the connection.)

(eval-when (:compile-toplevel :load-toplevel :execute)
  (require :sb-bsd-sockets)
  (require :sb-introspect))

(defpackage :edit-serve
  (:use :cl)
  (:export #:main #:in #:complete-in #:describe-in))

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
  "The threads evaluating a request right now.")

(defun interrupt-all ()
  (sb-thread:with-mutex (*lock*)
    (dolist (th *evaluating*)
      (ignore-errors
       (sb-thread:interrupt-thread
        th #'interrupted)))))

(defun interrupted ()
  (failed "interrupted" :host-stopped t))

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

(defun frames ()
  "The calls from the one that signalled out to the send, as text — the
debugger's frames, cut where the server's own begin."
  (let ((out '()))
    (block walk
      (sb-debug:map-backtrace
       (lambda (frame)
         (let ((name (ignore-errors (sb-di:debug-fun-name (sb-di:frame-debug-fun frame)))))
           (when (eq name 'eval-forms) (return-from walk))
           (unless (server-frame-p name)
             (push (safely (lambda ()
                             (let ((*print-length* 5) (*print-level* 3) (*print-pretty* nil))
                               (if (equal name '(lambda ()))
                                   ;; A top-level form, which EVAL compiled into a
                                   ;; function with no name: show the form.
                                   (prin1-to-string *form*)
                                   (with-output-to-string (s) (sb-debug::print-frame-call frame s)))))
                           "#<a frame>")
                   out)
             (when (>= (length out) *frames-limit*) (return-from walk)))))))
    (nreverse out)))

(defun failed (message &key host-stopped (backtrace (frames)))
  "Give the request up, answering MESSAGE; never returns."
  (throw 'request (list :error message host-stopped backtrace)))

(defun fail-from-condition (c)
  (failed (message-of c)))

(defun eval-forms (code)
  "Every form of CODE, in order, in *PACKAGE*; the last one's values."
  (let ((in (make-string-input-stream code))
        (values '()))
    (loop
      (let ((form (handler-case (read in nil in)
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

(defun answer-eval (id code)
  (let* ((out (make-string-output-stream))
         (no-input (make-string-input-stream ""))
         (io (make-two-way-stream no-input out))
         (warnings '())
         (result
           (catch 'request
             (let ((*standard-output* out) (*error-output* out) (*trace-output* out) (*print-package* nil)
                   (*standard-input* no-input) (*terminal-io* io) (*query-io* io) (*debug-io* io)
                   ;; Whatever would open the debugger answers instead.
                   (sb-ext:*invoke-debugger-hook* (lambda (c hook) (declare (ignore hook)) (fail-from-condition c)))
                   (*debugger-hook* (lambda (c hook) (declare (ignore hook)) (fail-from-condition c))))
               (sb-thread:with-mutex (*lock*) (push sb-thread:*current-thread* *evaluating*))
               (unwind-protect
                    (handler-bind ((warning
                                     (lambda (w)
                                       ;; The compiler's verdict on what was sent, kept
                                       ;; with the output and said once.
                                       (let ((line (format nil "; ~a: ~a"
                                                           (if (typep w 'style-warning) "style-warning" "warning")
                                                           (message-of w))))
                                         (push line warnings)
                                         (format out "~a~%" line))
                                       (muffle-warning w)))
                                   ((or error storage-condition)
                                     #'fail-from-condition))
                      (list :ok (values-text (eval-forms code))))
                 (sb-thread:with-mutex (*lock*)
                   (setf *evaluating* (remove sb-thread:*current-thread* *evaluating*))))))))
    (let ((output (get-output-stream-string out)))
      (destructuring-bind (status text &optional host-stopped backtrace) result
        (if (eq status :ok)
            (format nil "(:id ~a :status ok :value ~a :output ~a :warnings ~a)"
                    id (wire-string text) (wire-string output) (wire-list (reverse warnings)))
            (format nil "(:id ~a :status error :message ~a :output ~a :host-stopped ~a :backtrace ~a :warnings ~a)"
                    id (wire-string text) (wire-string output)
                    (if host-stopped "#t" "#f") (wire-list backtrace) (wire-list (reverse warnings))))))))

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

;;; ---- the requests -------------------------------------------------------

(defun version ()
  (format nil "sbcl ~a" (lisp-implementation-version)))

(defun answer (req)
  (let ((id (or (field req "id") 0))
        (op (field req "op")))
    (cond
      ((equal op "eval") (answer-eval id (or (field req "code") "")))
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

(defun serve-connection (socket)
  (let ((stream (sb-bsd-sockets:socket-make-stream socket :input t :output t
                                                          :external-format :utf-8 :buffering :full))
        (*package* (find-package :cl-user)))
    (unwind-protect
         (loop
           (let ((text (handler-case (read-datum stream) (error () nil))))
             (unless text (return))
             (let ((reply (handler-case (answer (parse-request text))
                            ;; A bug here answers malformed rather than killing the
                            ;; connection — under the request's own :id if it has one.
                            (error (c)
                              (format nil "(:id ~a :status malformed :message ~a)"
                                      (or (ignore-errors (field (parse-request text) "id")) 0)
                                      (wire-string (message-of c)))))))
               (write-string reply stream)
               (terpri stream)
               (finish-output stream))))
      (ignore-errors (close stream))
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
