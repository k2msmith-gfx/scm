(define (run n) (let loop ((i 0) (s 0)) (if (= i n) s (loop (+ i 1) (+ s (call/cc (lambda (k) (if (= 0 (modulo i 2)) (k 1) 2))))))))
(display (run 300000)) (newline)
