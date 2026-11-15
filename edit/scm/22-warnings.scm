;## 22-warnings.scm — warnings before you run (docs/editor-crate-plan.md,
;## E8; Kevin, 2026-11-15): in a Scheme buffer, a name nothing binds and a
;## call with the wrong count of arguments, underlined in the warning ink as
;## you type. M-g n goes to them; the modeline counts them;
;## M-x set-editor-warnings turns them off.
;##
;## The judging is Rust's (scm_binding/warn.rs, over scm-text's scope pass);
;## the POLICY is here: which image a buffer is judged against — the one its
;## sends go to — and telling the judge what that image binds, again after
;## every send, since a send is what changes it.

(define *editor-warnings* #t)

; Which image a buffer is judged against, as a FORWARD HOOK: 'self (this
; image), a remote connection, or #f for none — no warnings, rather than
; warnings against an image the code will not run in. The standalone says
; (host/standalone.scm): a ray file's ray, once it runs; a Scheme file's
; scm, once it runs, else this image, which has everything plain scm has.
; In the modeler every Scheme file runs here.
(define *buffer-image* (lambda (buf) (and (buffer-path buf) 'self)))

(define (image-key img)
  (if (eq? img 'self) "self" (string-append "conn:" (number->string img))))

; What an image binds, as Scheme text it evaluates: its names, its syntax,
; and every procedure's call shape, flat (name sig name sig …). Plain
; Scheme, so another process (scm's, ray's) can run it too.
(define *image-facts-form*
  "(let loop ((ns (apropos)) (names '()) (sigs '()))
     (if (null? ns)
       (list names (map symbol->string (%syntax-names)) sigs)
       (let* ((n (symbol->string (car ns))) (s (signature (car ns))))
         (loop (cdr ns) (cons n names) (if s (cons n (cons s sigs)) sigs)))))")

(define (describe-image! img)
  "Tell the judge what IMG binds. Another process is asked only when it is
  idle, and one that cannot answer (an older scm) leaves its buffers
  unjudged."
  (define facts
    (if (eq? img 'self)
      (eval (read (open-input-string *image-facts-form*)) (interaction-environment))
      (and (zero? (%remote-running img))
           (guard (e (#t #f))
             (let ((r (%remote-eval img *image-facts-form* #f 10)))
               (and (eq? (%alist-ref r :status #f) 'ok)
                    (read (open-input-string (%alist-ref r :value "#f")))))))))
  (match facts
    ((names syntax sigs) (%warn-facts (image-key img) names syntax sigs) #t)
    (_ #f)))

(define (warnings-attach! buf (fresh #f))
  "Judge BUF against the image its sends go to, describing that image first
  when it has not been — or, FRESH, when a send may have changed it."
  (define path (buffer-path buf))
  (when path
    (let ((img (and *editor-warnings*
                    (equal? (%buffer-language (editor-pane-id) buf) "scheme")
                    (*buffer-image* buf))))
      (if (and img (or (and (not fresh) (positive? (%warn-facts? (image-key img))))
                       (describe-image! img)))
        (%warn-buffer path (image-key img))
        (%warn-buffer path #f)))))

(define (warnings-after-send!)
  ; A send changed what its image binds: describe it again, and judge the
  ; buffer on screen against it — its image may only now be running (a ray
  ; file's first send starts its ray).
  (when *editor-warnings*
    (guard (e (#t #f))
      (warnings-attach! (%editor-active-buffer (editor-pane-id)) #t))))

(set! *buffer-opened-hooks* (cons warnings-attach! *buffer-opened-hooks*))

(define (set-editor-warnings on)
  "Underline what is likely wrong before it runs — a name nothing binds, a
  call with the wrong count of arguments — or not: (set-editor-warnings #f)
  turns them off, #t back on (M-x set-editor-warnings)."
  (set! *editor-warnings* (and on #t))
  (%warn-enable (if on 1 0))
  (when on (warnings-after-send!))
  (echo "warnings " (if on "on" "off")))
