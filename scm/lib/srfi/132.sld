;;; (srfi 132) — Sort Libraries.
;;;
;;; Merge sort throughout, so every sort here is stable and the
;;; "stable" names are the same procedures: SRFI 132 permits an
;;; implementation whose plain sorts happen to be stable, and promising
;;; less than is delivered would only invite someone to work around it.
;;;
;;; The `!` procedures sort in place for vectors, as the SRFI requires, and
;;; are allowed to reuse structure for lists -- where they return the
;;; sorted list and the argument is not to be used again.
;;;
;;; Every vector procedure takes the SRFI's optional `start` and `end`.
(define-library (srfi 132)
  (export list-sorted? vector-sorted?
          list-sort list-stable-sort vector-sort vector-stable-sort
          list-sort! list-stable-sort! vector-sort! vector-stable-sort!
          list-merge list-merge! vector-merge vector-merge!
          list-delete-neighbor-dups list-delete-neighbor-dups!
          vector-delete-neighbor-dups vector-delete-neighbor-dups!
          vector-select! vector-separate! vector-find-median
          vector-find-median!)
  (import (scheme base))
  (begin

    (define (range v start end)
      (values (if start start 0)
              (if end end (vector-length v))))

    ;; ── predicates ────────────────────────────────────────────────────
    (define (list-sorted? < lst)
      (let loop ((l lst))
        (cond ((null? l) #t)
              ((null? (cdr l)) #t)
              ((< (cadr l) (car l)) #f)
              (else (loop (cdr l))))))

    (define (vector-sorted? < v . opt)
      (let* ((start (if (pair? opt) (car opt) 0))
             (end (if (and (pair? opt) (pair? (cdr opt)))
                      (cadr opt)
                      (vector-length v))))
        (let loop ((i (+ start 1)))
          (cond ((>= i end) #t)
                ((< (vector-ref v i) (vector-ref v (- i 1))) #f)
                (else (loop (+ i 1)))))))

    ;; ── merging ───────────────────────────────────────────────────────
    ;;
    ;; Ties go to the left list, which is what makes the sort stable.
    (define (list-merge < a b)
      (let loop ((a a) (b b) (acc '()))
        (cond ((null? a) (append-reverse acc b))
              ((null? b) (append-reverse acc a))
              ((< (car b) (car a)) (loop a (cdr b) (cons (car b) acc)))
              (else (loop (cdr a) b (cons (car a) acc))))))
    (define list-merge! list-merge)

    (define (append-reverse rev tail)
      (let loop ((l rev) (acc tail))
        (if (null? l) acc (loop (cdr l) (cons (car l) acc)))))

    ;; ── the sorts ─────────────────────────────────────────────────────
    (define (list-sort < lst)
      (define (halve l)
        ;; Tortoise and hare, so the split needs no length.
        (let loop ((slow l) (fast (cdr l)) (acc '()))
          (if (or (null? fast) (null? (cdr fast)))
              (values (reverse (cons (car slow) acc)) (cdr slow))
              (loop (cdr slow) (cddr fast) (cons (car slow) acc)))))
      (let sort ((l lst))
        (cond ((null? l) '())
              ((null? (cdr l)) l)
              (else
               (let-values (((left right) (halve l)))
                 (list-merge < (sort left) (sort right)))))))

    (define list-stable-sort list-sort)
    (define list-sort! list-sort)
    (define list-stable-sort! list-sort)

    (define (vector-sort! < v . opt)
      (let-values (((start end) (range v (and (pair? opt) (car opt))
                                      (and (pair? opt) (pair? (cdr opt)) (cadr opt)))))
        ;; Sort the slice as a list and write it back: one allocation of
        ;; the slice rather than a scratch vector per level.
        (let ((sorted (list-sort < (vector->list v start end))))
          (let loop ((i start) (l sorted))
            (if (null? l)
                v
                (begin (vector-set! v i (car l)) (loop (+ i 1) (cdr l))))))))
    (define vector-stable-sort! vector-sort!)

    (define (vector-sort < v . opt)
      (let-values (((start end) (range v (and (pair? opt) (car opt))
                                      (and (pair? opt) (pair? (cdr opt)) (cadr opt)))))
        (list->vector (list-sort < (vector->list v start end)))))
    (define vector-stable-sort vector-sort)

    (define (vector-merge < a b . opt)
      (let* ((astart (if (pair? opt) (car opt) 0))
             (opt (if (pair? opt) (cdr opt) '()))
             (aend (if (pair? opt) (car opt) (vector-length a)))
             (opt (if (pair? opt) (cdr opt) '()))
             (bstart (if (pair? opt) (car opt) 0))
             (opt (if (pair? opt) (cdr opt) '()))
             (bend (if (pair? opt) (car opt) (vector-length b))))
        (list->vector (list-merge < (vector->list a astart aend)
                                  (vector->list b bstart bend)))))

    (define (vector-merge! < to a b . opt)
      (let* ((at (if (pair? opt) (car opt) 0))
             (merged (vector-merge < a b)))
        (let loop ((i 0))
          (if (>= i (vector-length merged))
              to
              (begin (vector-set! to (+ at i) (vector-ref merged i))
                     (loop (+ i 1)))))))

    ;; ── deleting neighbouring duplicates ──────────────────────────────
    (define (list-delete-neighbor-dups = lst)
      (if (null? lst)
          '()
          (let loop ((l (cdr lst)) (prev (car lst)) (acc (list (car lst))))
            (cond ((null? l) (reverse acc))
                  ((= prev (car l)) (loop (cdr l) prev acc))
                  (else (loop (cdr l) (car l) (cons (car l) acc)))))))
    (define list-delete-neighbor-dups! list-delete-neighbor-dups)

    (define (vector-delete-neighbor-dups = v . opt)
      (let-values (((start end) (range v (and (pair? opt) (car opt))
                                      (and (pair? opt) (pair? (cdr opt)) (cadr opt)))))
        (list->vector
         (list-delete-neighbor-dups = (vector->list v start end)))))

    ;; In place: the kept elements move down, and the count comes back.
    (define (vector-delete-neighbor-dups! = v . opt)
      (let-values (((start end) (range v (and (pair? opt) (car opt))
                                      (and (pair? opt) (pair? (cdr opt)) (cadr opt)))))
        (if (>= start end)
            start
            (let loop ((i (+ start 1)) (out start))
              (cond ((>= i end) (+ out 1))
                    ((= (vector-ref v out) (vector-ref v i)) (loop (+ i 1) out))
                    (else
                     (vector-set! v (+ out 1) (vector-ref v i))
                     (loop (+ i 1) (+ out 1))))))))

    ;; ── selection ─────────────────────────────────────────────────────
    (define (vector-select! < v k . opt)
      (let-values (((start end) (range v (and (pair? opt) (car opt))
                                      (and (pair? opt) (pair? (cdr opt)) (cadr opt)))))
        (list-ref (list-sort < (vector->list v start end)) k)))

    ;; Partition the slice so that the `k` smallest come first; the SRFI
    ;; does not say the two halves are each sorted, but sorting the whole
    ;; slice satisfies it and is what the merge sort already gives.
    (define (vector-separate! < v k . opt)
      (apply vector-sort! < v opt)
      v)

    (define (vector-find-median < v knil . opt)
      (let* ((mean (if (pair? opt)
                       (car opt)
                       (lambda (a b) (/ (+ a b) 2))))
             (sorted (list-sort < (vector->list v)))
             (n (length sorted)))
        (cond ((= n 0) knil)
              ((odd? n) (list-ref sorted (quotient n 2)))
              (else (mean (list-ref sorted (- (quotient n 2) 1))
                          (list-ref sorted (quotient n 2)))))))

    (define (vector-find-median! < v knil . opt)
      (let ((m (apply vector-find-median < v knil opt)))
        (vector-sort! < v)
        m))))
