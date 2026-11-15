;## 11-diagnostics.scm — what the READER knows about the text being edited
;## (docs/editor-crate-plan.md, E3f): where it stops reading, and what it
;## defines. Both come from the text alone — scm's reader, and scm-text's
;## scan — so they answer for a buffer nothing has run.

; ── Where the buffer stops reading ─────────────────────────────────────────
; The editor marks it as you type (the widget asks the language, which asks
; scm's reader): text that will not read underlined in the error ink, with
; its line on the modeline; a form left open underlined faintly at its
; opening bracket — which is also every form while it is being typed, so
; that one is quiet. The reader stops at the first, so there is one.

(define (editor-goto eid line column)
  "Put EID's point at LINE (from 0) and COLUMN (characters), clearing the
  selection — as M-. lands on a name."
  (%editor-goto-line eid line)
  (let loop ((n column))
    (when (positive? n) (%editor-act eid "right") (loop (- n 1)))))

(define (goto-diagnostic)
  "Go to where the buffer stops reading, and say why — M-g n (and M-g p,
  there being one). Runs nothing: the reader is asked about the text as it
  stands, the same reader a send will meet.

  M-, comes back, as it does from M-.."
  (define eid (editor-pane-id))
  (match (%editor-diagnostics eid)
    (() (echo "the buffer reads"))
    (((line column message error?) . _)
      (push-definition-mark! eid)
      (editor-goto eid line column)
      (echo (number->string (+ line 1)) ":" (number->string (+ column 1)) "  "
            (if (= error? 1) "" "open: ") message))))

(bind-key "M-g n" (labelled "goto-diagnostic" (lambda () (if (editor-focused?) (goto-diagnostic) #f))))
(bind-key "M-g p" (labelled "goto-diagnostic" (lambda () (if (editor-focused?) (goto-diagnostic) #f))))

; ── Document symbols: what this buffer defines ─────────────────────────────
; Emacs's imenu (M-g i), from the same scan the M-. index uses: every
; define, define-record-type member, define-syntax … with its line, as
; minibuffer candidates. TAB narrows, RET jumps, M-, comes back.

(define (push-definition-mark! eid)
  "Remember where EID's point is, for M-, (10-xref's mark stack)."
  (match (%editor-cursor eid)
    ((line index) (set! *definition-marks*
                    (cons (list (%editor-active-buffer eid) line index) *definition-marks*)))
    (_ #f)))

(define (definition-label def)
  ; name:LINE — unique, and what the candidate list narrows on.
  (match def
    ((name form detail line column)
      (string-append name ":" (number->string (+ line 1))))))

(define (document-symbols)
  "Jump to a definition in this buffer — M-g i, Emacs's imenu. The
  minibuffer lists every name the buffer defines, as name:LINE in the
  buffer's order; typing narrows, RET jumps, M-, comes back.

  Read from the text as it stands, so a definition typed a moment ago and
  never run is there, and a half-typed buffer still answers."
  (define eid (editor-pane-id))
  (define defs (%text-definitions (%editor-text eid)))
  (define labels (map definition-label defs))
  (define (go def)
    (match def
      ((name form detail line column)
        (push-definition-mark! eid)
        (editor-goto eid line column)
        (echo name "  " (or detail form)))))
  (if (null? defs)
    (echo "this buffer defines nothing")
    (begin
      (read-minibuffer
        "Definition in this buffer: "
        (lambda (ans)
          (let ((ans (string-trim-both ans)))
            (cond ((string=? ans "") #f)
                  ((find (lambda (d) (string=? (definition-label d) ans)) defs) => go)
                  ((find (lambda (d) (string=? (car d) ans)) defs) => go)
                  (else (echo "no definition " ans " in this buffer")))))
        labels)
      ; The list is the point: show it before anything is typed.
      (comp-open labels #f))))

(bind-key "M-g i" (labelled "document-symbols" (lambda () (if (editor-focused?) (document-symbols) #f))))
