(define (tak x y z) (if (not (< y x)) z (tak (tak (- x 1) y z) (tak (- y 1) z x) (tak (- z 1) x y))))
(display (let r ((k 0) (v 0)) (if (= k 20) v (r (+ k 1) (tak 24 16 8))))) (newline)
