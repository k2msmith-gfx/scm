;;; (srfi 1) — List Library.
;;;
;;; Written against the SRFI 1 specification rather than copied from its
;;; reference implementation, so every procedure here is one this project
;;; can test and change. The linear-update variants (`append!`, `map!`,
;;; `delete!` and the rest) are the non-destructive versions: the SRFI
;;; permits an implementation to reuse the argument's cells or not, and
;;; "not" is the version that cannot surprise a caller who kept a
;;; reference.
;;;
;;; Three exports deliberately differ from scm's prelude, because the SRFI
;;; defines them differently and an explicit import should win:
;;;
;;;   (reduce f ridentity lst)  calls (f elem acc); the prelude's calls
;;;                             (f acc elem) and takes an init
;;;   (fold-left …)             is not SRFI 1's and is left alone
;;;   (delete x lst [=])        the prelude has no delete
;;;
;;; `map`, `for-each`, `member`, `assoc` and `filter` are supersets of the
;;; R7RS ones (unequal list lengths, an optional equality predicate), so
;;; importing them replaces the globals harmlessly.
(define-library (srfi 1)
  (export
    ;; constructors
    xcons cons* make-list list-tabulate list-copy circular-list iota
    ;; predicates
    proper-list? circular-list? dotted-list? null-list? not-pair? list=
    ;; selectors
    first second third fourth fifth sixth seventh eighth ninth tenth
    car+cdr take drop take! take-right drop-right drop-right!
    split-at split-at! last last-pair
    ;; length, append, reverse, zip, count
    length+ concatenate concatenate! append! reverse!
    append-reverse append-reverse! zip unzip1 unzip2 unzip3 unzip4 unzip5
    count
    ;; fold, unfold, map
    fold fold-right pair-fold pair-fold-right reduce reduce-right
    unfold unfold-right map for-each append-map append-map! map!
    map-in-order pair-for-each filter-map
    ;; filtering and partitioning
    filter partition remove filter! partition! remove!
    ;; searching
    member find find-tail any every list-index
    take-while take-while! drop-while span span! break break!
    ;; deletion
    delete delete! delete-duplicates delete-duplicates!
    ;; association lists
    assoc alist-cons alist-copy alist-delete alist-delete!
    ;; set operations on lists
    lset<= lset= lset-adjoin lset-union lset-union! lset-intersection
    lset-intersection! lset-difference lset-difference! lset-xor lset-xor!
    lset-diff+intersection lset-diff+intersection!)
  (import (scheme base))
  (begin

    ;; ── constructors ──────────────────────────────────────────────────
    (define (xcons d a) (cons a d))

    (define (cons* first . rest)
      (if (null? rest)
          first
          (let loop ((x first) (rest rest))
            (if (null? rest)
                x
                (cons x (loop (car rest) (cdr rest)))))))

    (define (list-tabulate n proc)
      (let loop ((i (- n 1)) (acc '()))
        (if (< i 0) acc (loop (- i 1) (cons (proc i) acc)))))

    (define (circular-list elt . rest)
      (let ((all (cons elt rest)))
        (set-cdr! (last-pair all) all)
        all))

    ;; (iota count [start step])
    (define (iota count . opt)
      (let ((start (if (pair? opt) (car opt) 0))
            (step (if (and (pair? opt) (pair? (cdr opt))) (cadr opt) 1)))
        (let loop ((i (- count 1)) (acc '()))
          (if (< i 0)
              acc
              (loop (- i 1) (cons (+ start (* i step)) acc))))))

    ;; ── predicates ────────────────────────────────────────────────────
    ;;
    ;; One walk answers all three: tortoise and hare, so a circular list
    ;; is reported rather than hung on.
    (define (list-shape x)
      (let loop ((slow x) (fast x))
        (cond ((null? fast) 'proper)
              ((not (pair? fast)) 'dotted)
              ((null? (cdr fast)) 'proper)
              ((not (pair? (cdr fast))) 'dotted)
              (else
               (let ((slow (cdr slow)) (fast (cddr fast)))
                 (if (eq? slow fast) 'circular (loop slow fast)))))))

    (define (proper-list? x) (eq? 'proper (list-shape x)))
    (define (circular-list? x) (eq? 'circular (list-shape x)))
    (define (dotted-list? x)
      (if (pair? x) (eq? 'dotted (list-shape x)) (not (null? x))))
    (define (not-pair? x) (not (pair? x)))

    (define (null-list? l)
      (cond ((null? l) #t)
            ((pair? l) #f)
            (else (error "null-list?: not a list" l))))

    (define (list= elt= . lists)
      (define (two? a b)
        (let loop ((a a) (b b))
          (cond ((null? a) (null? b))
                ((null? b) #f)
                ((elt= (car a) (car b)) (loop (cdr a) (cdr b)))
                (else #f))))
      (let loop ((ls lists))
        (cond ((null? ls) #t)
              ((null? (cdr ls)) #t)
              ((two? (car ls) (cadr ls)) (loop (cdr ls)))
              (else #f))))

    ;; ── selectors ─────────────────────────────────────────────────────
    (define (first l) (car l))
    (define (second l) (cadr l))
    (define (third l) (caddr l))
    (define (fourth l) (cadddr l))
    (define (fifth l) (list-ref l 4))
    (define (sixth l) (list-ref l 5))
    (define (seventh l) (list-ref l 6))
    (define (eighth l) (list-ref l 7))
    (define (ninth l) (list-ref l 8))
    (define (tenth l) (list-ref l 9))

    (define (car+cdr p) (values (car p) (cdr p)))

    (define (take l k)
      (let loop ((l l) (k k) (acc '()))
        (if (<= k 0)
            (reverse acc)
            (loop (cdr l) (- k 1) (cons (car l) acc)))))
    (define take! take)

    (define (drop l k)
      (let loop ((l l) (k k))
        (if (<= k 0) l (loop (cdr l) (- k 1)))))

    (define (take-right l k)
      (let loop ((lag l) (lead (drop l k)))
        (if (pair? lead) (loop (cdr lag) (cdr lead)) lag)))

    (define (drop-right l k)
      (let loop ((lag l) (lead (drop l k)) (acc '()))
        (if (pair? lead)
            (loop (cdr lag) (cdr lead) (cons (car lag) acc))
            (reverse acc))))
    (define drop-right! drop-right)

    (define (split-at l k) (values (take l k) (drop l k)))
    (define split-at! split-at)

    (define (last l) (car (last-pair l)))
    (define (last-pair l)
      (let loop ((l l))
        (if (pair? (cdr l)) (loop (cdr l)) l)))

    ;; ── length, append, reverse, zip, count ───────────────────────────
    (define (length+ l)
      (if (circular-list? l) #f (length l)))

    (define (concatenate lists) (apply append lists))
    (define concatenate! concatenate)
    (define (append! . lists) (apply append lists))
    (define (reverse! l) (reverse l))

    (define (append-reverse rev-head tail)
      (let loop ((l rev-head) (acc tail))
        (if (null? l) acc (loop (cdr l) (cons (car l) acc)))))
    (define append-reverse! append-reverse)

    (define (zip l . more) (apply map list l more))

    (define (unzip1 l) (map car l))
    (define (unzip2 l) (values (map car l) (map cadr l)))
    (define (unzip3 l) (values (map car l) (map cadr l) (map caddr l)))
    (define (unzip4 l)
      (values (map car l) (map cadr l) (map caddr l) (map cadddr l)))
    (define (unzip5 l)
      (values (map car l) (map cadr l) (map caddr l) (map cadddr l)
              (map (lambda (x) (list-ref x 4)) l)))

    (define (count pred l . more)
      (if (null? more)
          (let loop ((l l) (n 0))
            (cond ((null? l) n)
                  ((pred (car l)) (loop (cdr l) (+ n 1)))
                  (else (loop (cdr l) n))))
          (let loop ((ls (cons l more)) (n 0))
            (if (any-empty? ls)
                n
                (loop (map cdr ls)
                      (if (apply pred (map car ls)) (+ n 1) n))))))

    ;; ── the multi-list helpers the folds and maps share ───────────────
    (define (any-empty? ls)
      (let loop ((ls ls))
        (cond ((null? ls) #f)
              ((null? (car ls)) #t)
              (else (loop (cdr ls))))))

    ;; ── fold, unfold, map ─────────────────────────────────────────────
    (define (fold kons knil l . more)
      (if (null? more)
          (let loop ((l l) (acc knil))
            (if (null? l) acc (loop (cdr l) (kons (car l) acc))))
          (let loop ((ls (cons l more)) (acc knil))
            (if (any-empty? ls)
                acc
                (loop (map cdr ls)
                      (apply kons (append (map car ls) (list acc))))))))

    (define (fold-right kons knil l . more)
      (if (null? more)
          (let loop ((l l))
            (if (null? l) knil (kons (car l) (loop (cdr l)))))
          (let loop ((ls (cons l more)))
            (if (any-empty? ls)
                knil
                (apply kons (append (map car ls) (list (loop (map cdr ls)))))))))

    (define (pair-fold kons knil l . more)
      (if (null? more)
          (let loop ((l l) (acc knil))
            (if (null? l)
                acc
                (let ((tail (cdr l))) (loop tail (kons l acc)))))
          (let loop ((ls (cons l more)) (acc knil))
            (if (any-empty? ls)
                acc
                (let ((tails (map cdr ls)))
                  (loop tails (apply kons (append ls (list acc)))))))))

    (define (pair-fold-right kons knil l . more)
      (if (null? more)
          (let loop ((l l))
            (if (null? l) knil (kons l (loop (cdr l)))))
          (let loop ((ls (cons l more)))
            (if (any-empty? ls)
                knil
                (apply kons (append ls (list (loop (map cdr ls)))))))))

    ;; SRFI 1's reduce: no init, and `f` takes (elem acc).
    (define (reduce f ridentity l)
      (if (null? l) ridentity (fold f (car l) (cdr l))))

    (define (reduce-right f ridentity l)
      (if (null? l)
          ridentity
          (let loop ((head (car l)) (rest (cdr l)))
            (if (null? rest) head (f head (loop (car rest) (cdr rest)))))))

    ;; (unfold p f g seed [tail-gen])
    (define (unfold p f g seed . opt)
      (let ((tail-gen (if (pair? opt) (car opt) (lambda (x) '()))))
        (let loop ((seed seed) (acc '()))
          (if (p seed)
              (append-reverse acc (tail-gen seed))
              (loop (g seed) (cons (f seed) acc))))))

    (define (unfold-right p f g seed . opt)
      (let loop ((seed seed) (acc (if (pair? opt) (car opt) '())))
        (if (p seed) acc (loop (g seed) (cons (f seed) acc)))))

    ;; SRFI 1's `map` stops at the shortest list; R7RS's requires equal
    ;; lengths, so this is a superset and replacing the global is safe.
    (define (map f l . more)
      (if (null? more)
          (let loop ((l l) (acc '()))
            (if (null? l) (reverse acc) (loop (cdr l) (cons (f (car l)) acc))))
          (let loop ((ls (cons l more)) (acc '()))
            (if (any-empty? ls)
                (reverse acc)
                (loop (map-cdrs ls) (cons (apply f (map-cars ls)) acc))))))

    ;; Spelled out rather than using `map`, which is being defined.
    (define (map-cars ls)
      (let loop ((ls ls) (acc '()))
        (if (null? ls) (reverse acc) (loop (cdr ls) (cons (caar ls) acc)))))
    (define (map-cdrs ls)
      (let loop ((ls ls) (acc '()))
        (if (null? ls) (reverse acc) (loop (cdr ls) (cons (cdar ls) acc)))))

    (define map-in-order map)
    (define map! map)

    (define (for-each f l . more)
      (if (null? more)
          (let loop ((l l))
            (if (null? l)
                #t
                (begin (f (car l)) (loop (cdr l)))))
          (let loop ((ls (cons l more)))
            (if (any-empty? ls)
                #t
                (begin (apply f (map-cars ls)) (loop (map-cdrs ls)))))))

    (define (append-map f l . more)
      (apply append (apply map f l more)))
    (define append-map! append-map)

    (define (pair-for-each f l . more)
      (if (null? more)
          (let loop ((l l))
            (if (null? l)
                #t
                (let ((tail (cdr l))) (f l) (loop tail))))
          (let loop ((ls (cons l more)))
            (if (any-empty? ls)
                #t
                (let ((tails (map-cdrs ls))) (apply f ls) (loop tails))))))

    (define (filter-map f l . more)
      (let loop ((xs (apply map f l more)) (acc '()))
        (cond ((null? xs) (reverse acc))
              ((car xs) (loop (cdr xs) (cons (car xs) acc)))
              (else (loop (cdr xs) acc)))))

    ;; ── filtering and partitioning ────────────────────────────────────
    (define (filter pred l)
      (let loop ((l l) (acc '()))
        (cond ((null? l) (reverse acc))
              ((pred (car l)) (loop (cdr l) (cons (car l) acc)))
              (else (loop (cdr l) acc)))))
    (define filter! filter)

    (define (remove pred l) (filter (lambda (x) (not (pred x))) l))
    (define remove! remove)

    (define (partition pred l)
      (let loop ((l l) (in '()) (out '()))
        (cond ((null? l) (values (reverse in) (reverse out)))
              ((pred (car l)) (loop (cdr l) (cons (car l) in) out))
              (else (loop (cdr l) in (cons (car l) out))))))
    (define partition! partition)

    ;; ── searching ─────────────────────────────────────────────────────
    (define (member x l . opt)
      (let ((= (if (pair? opt) (car opt) equal?)))
        (let loop ((l l))
          (cond ((null? l) #f)
                ((= x (car l)) l)
                (else (loop (cdr l)))))))

    (define (find pred l)
      (let ((tail (find-tail pred l)))
        (and tail (car tail))))

    (define (find-tail pred l)
      (let loop ((l l))
        (cond ((null? l) #f)
              ((pred (car l)) l)
              (else (loop (cdr l))))))

    (define (any pred l . more)
      (if (null? more)
          (let loop ((l l))
            (cond ((null? l) #f)
                  ((null? (cdr l)) (pred (car l)))
                  ((pred (car l)))
                  (else (loop (cdr l)))))
          (let loop ((ls (cons l more)))
            (cond ((any-empty? ls) #f)
                  ((any-empty? (map-cdrs ls)) (apply pred (map-cars ls)))
                  ((apply pred (map-cars ls)))
                  (else (loop (map-cdrs ls)))))))

    (define (every pred l . more)
      (if (null? more)
          (let loop ((l l))
            (cond ((null? l) #t)
                  ((null? (cdr l)) (pred (car l)))
                  ((pred (car l)) (loop (cdr l)))
                  (else #f)))
          (let loop ((ls (cons l more)))
            (cond ((any-empty? ls) #t)
                  ((any-empty? (map-cdrs ls)) (apply pred (map-cars ls)))
                  ((apply pred (map-cars ls)) (loop (map-cdrs ls)))
                  (else #f)))))

    (define (list-index pred l . more)
      (if (null? more)
          (let loop ((l l) (i 0))
            (cond ((null? l) #f)
                  ((pred (car l)) i)
                  (else (loop (cdr l) (+ i 1)))))
          (let loop ((ls (cons l more)) (i 0))
            (cond ((any-empty? ls) #f)
                  ((apply pred (map-cars ls)) i)
                  (else (loop (map-cdrs ls) (+ i 1)))))))

    (define (take-while pred l)
      (let loop ((l l) (acc '()))
        (if (or (null? l) (not (pred (car l))))
            (reverse acc)
            (loop (cdr l) (cons (car l) acc)))))
    (define take-while! take-while)

    (define (drop-while pred l)
      (let loop ((l l))
        (if (or (null? l) (not (pred (car l)))) l (loop (cdr l)))))

    (define (span pred l)
      (values (take-while pred l) (drop-while pred l)))
    (define span! span)

    (define (break pred l)
      (span (lambda (x) (not (pred x))) l))
    (define break! break)

    ;; ── deletion ──────────────────────────────────────────────────────
    (define (delete x l . opt)
      (let ((= (if (pair? opt) (car opt) equal?)))
        (filter (lambda (y) (not (= x y))) l)))
    (define delete! delete)

    (define (delete-duplicates l . opt)
      (let ((= (if (pair? opt) (car opt) equal?)))
        (let loop ((l l) (acc '()))
          (cond ((null? l) (reverse acc))
                ((member (car l) acc =) (loop (cdr l) acc))
                (else (loop (cdr l) (cons (car l) acc)))))))
    (define delete-duplicates! delete-duplicates)

    ;; ── association lists ─────────────────────────────────────────────
    (define (assoc key alist . opt)
      (let ((= (if (pair? opt) (car opt) equal?)))
        (let loop ((alist alist))
          (cond ((null? alist) #f)
                ((= key (caar alist)) (car alist))
                (else (loop (cdr alist)))))))

    (define (alist-cons key datum alist) (cons (cons key datum) alist))

    (define (alist-copy alist)
      (map (lambda (p) (cons (car p) (cdr p))) alist))

    (define (alist-delete key alist . opt)
      (let ((= (if (pair? opt) (car opt) equal?)))
        (filter (lambda (p) (not (= key (car p)))) alist)))
    (define alist-delete! alist-delete)

    ;; ── set operations on lists ───────────────────────────────────────
    (define (lset<= = . lists)
      (define (subset? a b)
        (every (lambda (x) (member x b =)) a))
      (let loop ((ls lists))
        (cond ((null? ls) #t)
              ((null? (cdr ls)) #t)
              ((subset? (car ls) (cadr ls)) (loop (cdr ls)))
              (else #f))))

    (define (lset= = . lists)
      (let loop ((ls lists))
        (cond ((null? ls) #t)
              ((null? (cdr ls)) #t)
              ((and (lset<= = (car ls) (cadr ls))
                    (lset<= = (cadr ls) (car ls)))
               (loop (cdr ls)))
              (else #f))))

    (define (lset-adjoin = l . elts)
      (fold (lambda (x acc) (if (member x acc =) acc (cons x acc))) l elts))

    (define (lset-union = . lists)
      (if (null? lists)
          '()
          (fold (lambda (l acc)
                  (fold (lambda (x acc) (if (member x acc =) acc (cons x acc)))
                        acc l))
                (car lists)
                (cdr lists))))
    (define lset-union! lset-union)

    (define (lset-intersection = l . rest)
      (filter (lambda (x) (every (lambda (other) (member x other =)) rest)) l))
    (define lset-intersection! lset-intersection)

    (define (lset-difference = l . rest)
      (filter (lambda (x)
                (not (any (lambda (other) (member x other =)) rest)))
              l))
    (define lset-difference! lset-difference)

    (define (lset-xor = . lists)
      (fold (lambda (b a)
              (append (lset-difference = a b) (lset-difference = b a)))
            '()
            lists))
    (define lset-xor! lset-xor)

    (define (lset-diff+intersection = l . rest)
      (values (apply lset-difference = l rest)
              (apply lset-intersection = l rest)))
    (define lset-diff+intersection! lset-diff+intersection)))
