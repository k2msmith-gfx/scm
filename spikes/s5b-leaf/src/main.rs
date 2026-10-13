//! S5b — the leaf probe (plan §6).
//!
//! Not the leaf compiler. The question is narrower and the answer is cheap:
//! **which of the five items in plan §6 does the representation as it exists
//! at S5 make hard?** Answering it now costs 0.2; answering it at S11 means
//! reworking a value representation shaped by ten stages of interpreter
//! convenience.
//!
//!     cargo run                     # the analysis. No LLVM.
//!     cargo run --features llvm     # and the codegen + JIT half.

mod subset;
#[cfg(feature = "llvm")]
mod codegen;

use subset::{Analysis, Verdict};

/// ray's real texture stack, in the Scheme that exists today. Taken from the
/// corpus shapes the leaf survey counted, not invented for the spike:
/// `hash2` -> `fade` -> `vnoise` -> `fbm` -> a colour blend, with the
/// accumulator loop, a closure over a precomputed table, and the 3-vector
/// return that every texture has.
const FIXTURE: &str = r#"
(define (fract x) (- x (floor x)))
(define (hash2 x y)
  (fract (* 43758.5453 (sin (+ (* x 12.9898) (* y 78.233))))))
(define (fade t) (* t t (- 3.0 (* 2.0 t))))
(define (mix a b t) (+ a (* t (- b a))))

(define (vnoise x y)
  (let* ([xi (floor x)] [yi (floor y)]
         [xf (- x xi)]  [yf (- y yi)]
         [u (fade xf)]  [v (fade yf)])
    (mix (mix (hash2 xi yi) (hash2 (+ xi 1.0) yi) u)
         (mix (hash2 xi (+ yi 1.0)) (hash2 (+ xi 1.0) (+ yi 1.0)) u)
         v)))

(define (fbm x y octaves)
  (let loop ([i 0] [amp 0.5] [freq 1.0] [sum 0.0])
    (if (= i octaves)
        sum
        (loop (+ i 1) (* amp 0.5) (* freq 2.0)
              (+ sum (* amp (vnoise (* x freq) (* y freq))))))))

(define (texel u v)
  (let ([n (fbm (* u 8.0) (* v 8.0) 4)])
    (map (lambda (c) (mix 0.1 c n)) (list 0.8 0.5 0.2))))

;; item (5): the 3-float return, written the way the corpus writes it.
(define (texel3 u v)
  (let ([n (vnoise (* u 8.0) (* v 8.0))])
    (map (lambda (c) (mix 0.1 c n)) (list 0.8 0.5 0.2))))

;; item (2): a precomputed immutable table, read through a closure.
(define heights (list->vector (list 0.1 0.4 0.9 0.3)))
(define (h-at i) (vector-ref heights (modulo i 4)))

;; item (3): local mutation in a bounded loop, the `fbm` accumulator shape
;; written the way the corpus writes it.
(define (ridge x y)
  (let loop ([k 0] [acc 0.0])
    (if (= k 3) acc (loop (+ k 1) (+ acc (abs (- 0.5 (vnoise (* x k) y))))))))

;; --- and four that MUST be refused, each for a different reason ---------
(define (impure-io x) (begin (display x) x))
(define *tally* 0)
(define (impure-global x) (begin (set! *tally* (+ *tally* 1)) x))
(define (impure-mutate v) (begin (vector-set! v 0 1.0) v))
(define (impure-reach x) (* 2.0 (impure-io x)))
"#;

fn main() {
    let mut vm = scm::Vm::new();
    vm.eval_str(FIXTURE).expect("the fixture must load");

    println!("S5b — leaf probe");
    println!("{}", "=".repeat(68));
    println!("\n## 1. Purity and reachability (plan §6 item 1)\n");

    let expect_ok = [
        "fract", "hash2", "fade", "mix", "vnoise", "fbm", "texel", "h-at", "ridge",
    ];
    let expect_blocked = [
        "impure-io",
        "impure-global",
        "impure-mutate",
        "impure-reach",
    ];

    let mut wrong = 0;
    for name in expect_ok {
        let v = Analysis::new(&mut vm).verdict(name);
        match &v {
            Verdict::Ok { calls, captured } => {
                let mut note = String::new();
                if !calls.is_empty() {
                    note.push_str(&format!(" calls {{{}}}", join(calls)));
                }
                if !captured.is_empty() {
                    note.push_str(&format!(" captures {{{}}}", join(captured)));
                }
                println!("  COMPILABLE  {name:<14}{note}");
            }
            Verdict::Blocked { why, via } => {
                wrong += 1;
                println!("  !! BLOCKED  {name:<14} {why}   via {via:?}");
            }
        }
    }
    println!();
    for name in expect_blocked {
        let v = Analysis::new(&mut vm).verdict(name);
        match &v {
            Verdict::Blocked { why, via } => {
                println!("  REFUSED     {name:<14} {why}");
                println!("              {:<14} via {}", "", via.join(" -> "));
            }
            Verdict::Ok { .. } => {
                wrong += 1;
                println!("  !! ACCEPTED {name:<14} should have been refused");
            }
        }
    }

    println!("\n{}", "-".repeat(68));
    if wrong == 0 {
        println!("analysis: every verdict as expected, and each refusal names its call.");
    } else {
        println!("analysis: {wrong} WRONG VERDICTS");
    }

    #[cfg(feature = "llvm")]
    {
        println!("\n## 2. Codegen and the return convention (items 3, 5)\n");
        codegen::run(&mut vm);
    }
    #[cfg(not(feature = "llvm"))]
    println!("\n(built without --features llvm: the analysis half needs no toolchain)");
}

fn join(s: &std::collections::BTreeSet<String>) -> String {
    s.iter().cloned().collect::<Vec<_>>().join(" ")
}
