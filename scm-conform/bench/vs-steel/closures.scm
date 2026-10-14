(define (adder k) (lambda (x) (+ x k)))
(define (run n) (let loop ((i 0) (s 0)) (if (= i n) s (loop (+ i 1) (+ s ((adder i) 1))))))
(display (run 1000000)) (newline)
