(define (run n) (let loop ((i 0) (s 0)) (if (= i n) s (loop (+ i 1) (+ s (modulo i 7))))))
(display (run 50000000)) (newline)
