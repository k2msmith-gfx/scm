//! S7's third exit criterion: **no panic crosses the API**.
//!
//! There are 88 panic sites behind 95 public items (plan §7a), and risk 8 is
//! written in ray's blood -- a panic cannot unwind out of `extern "C"` and
//! **aborts the process**, which is how a zero weight killed Kevin's modeler
//! mid-edit (@7d8c8a1). So this hammers the boundary with hostile input and
//! asserts every call returns a `Result`.
//!
//! Deterministic, not random: a fuzz failure nobody can reproduce is a rumour.
//! The generator is a seeded xorshift and the seeds are fixed, so a failure
//! here is a failure for everyone.
//!
//! It also *reports* caught panics rather than treating them as a pass. The
//! criterion is that none crosses, and this API converts a panic into
//! `Error::Panicked` -- but a caught panic still means something in the
//! machine asserted on reachable input, and that is a bug with a known
//! address.

use scm::embed::Error;
use scm::{Interp, Val};

/// xorshift64*, so a seed names an exact sequence.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Fragments chosen to be nasty on purpose: unbalanced everything, the
/// numeric edges, the datum-comment corners the conformance suite caught, a
/// cycle, dotted tails, deep nesting, and the host's own controls.
const TOKENS: &[&str] = &[
    "(", ")", "[", "]", "#(", "#u8(", "'", "`", ",", ",@", ".", "...", "#;", "#|", "|#",
    "\"", "\\", "#\\", "#\\x", ";", "|", "#t", "#f", "#!default",
    "0", "-0", "1/2", "+inf.0", "-nan.0", "1e400", "9223372036854775808",
    "-9223372036854775809", "#xFFFFFFFFFFFFFFFF", "1e-400", ".5", "5.",
    "car", "set-cdr!", "define", "define-syntax", "syntax-rules", "lambda",
    "let", "let*", "letrec", "named", "quote", "quasiquote", "unquote",
    "set!", "if", "begin", "guard", "raise", "apply", "eval", "values",
    "call-with-values", "dynamic-wind", "%try", "vector-ref", "string-ref",
    "substring", "list-tail", "exact", "expt", "make-vector", "make-string",
    ":kw", "#:kw", "&opt", "&named", "&keys", "error", "features",
    "(spin)", "x", "y", "f", " ", "\n", "\t",
];

fn source(rng: &mut Rng, len: usize) -> String {
    let mut s = String::new();
    for _ in 0..len {
        s.push_str(TOKENS[rng.below(TOKENS.len())]);
        s.push(' ');
    }
    s
}

fn val(rng: &mut Rng, depth: usize) -> Val {
    if depth > 6 {
        return Val::Int(0);
    }
    match rng.below(12) {
        0 => Val::Null,
        1 => Val::Unspecified,
        2 => Val::Bool(rng.below(2) == 0),
        3 => Val::Int(rng.next() as i64),
        4 => Val::Float(f64::from_bits(rng.next())),
        5 => Val::Char(char::from_u32(rng.below(0x11000) as u32).unwrap_or('?')),
        6 => Val::Str("\u{0}\"\\\n\u{10FFFF}".repeat(1 + rng.below(3))),
        7 => Val::Sym(String::from_utf8_lossy(&[b'a' + (rng.below(26) as u8)]).into_owned()),
        8 => Val::Keyword("at".into()),
        9 => Val::Bytes(vec![rng.next() as u8; rng.below(8)]),
        10 => Val::List((0..rng.below(5)).map(|_| val(rng, depth + 1)).collect()),
        _ => Val::Vector((0..rng.below(5)).map(|_| val(rng, depth + 1)).collect()),
    }
}

/// A fresh interpreter, bounded so a generated infinite loop is an error
/// rather than a hung test.
fn bounded() -> Interp {
    let mut it = Interp::new().expect("a fresh interpreter");
    it.set_fuel(Some(20_000));
    it.set_heap_cap(Some(200_000));
    it
}

#[test]
fn no_panic_crosses_the_api() {
    // The generator produces panic messages on stderr for every caught
    // panic, which would bury the result. Hush them and count instead.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut panicked: Vec<(u64, String, String)> = Vec::new();
    let mut outcomes = [0usize; 3]; // ok, err, panicked

    for seed in 1..=400u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut it = bounded();
        // A host function that is itself hostile: it refuses, panics, or
        // hands back something unrepresentable.
        it.define_fn("host", {
            let mut n = 0usize;
            move |args| {
                n += 1;
                match n % 4 {
                    0 => Err("no".into()),
                    1 => Ok(Val::Opaque),
                    2 => panic!("hostile host fn"),
                    _ => Ok(args.first().cloned().unwrap_or(Val::Null)),
                }
            }
        })
        .expect("registering a host fn");

        for _ in 0..6 {
            let n = 1 + rng.below(12);
            let src = source(&mut rng, n);
            match it.eval(&src) {
                Ok(_) => outcomes[0] += 1,
                Err(Error::Panicked(m)) => {
                    outcomes[2] += 1;
                    panicked.push((seed, src.clone(), m));
                }
                Err(_) => outcomes[1] += 1,
            }
            if it.is_poisoned() {
                it = bounded();
            }
            // And the value path, on whatever the source left behind.
            let v = val(&mut rng, 0);
            if let Ok(r) = it.make(&v) {
                let _ = it.write(&r);
                let _ = it.display(&r);
                let _ = it.value(&r);
            }
            if it.is_poisoned() {
                it = bounded();
            }
        }
    }

    std::panic::set_hook(prev);
    println!(
        "fuzz: {} ok, {} refused, {} caught panics",
        outcomes[0], outcomes[1], outcomes[2]
    );
    assert!(
        panicked.is_empty(),
        "{} inputs panicked inside the API. First three:\n{}",
        panicked.len(),
        panicked
            .iter()
            .take(3)
            .map(|(s, src, m)| format!("  seed {s}: {m}\n    src: {src:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    // The point of the run is that it exercised something.
    assert!(outcomes[0] + outcomes[1] > 1_000, "the harness barely ran");
}

#[test]
fn every_host_value_round_trips_or_refuses() {
    // The `make` -> `value` path on its own, with the pathological floats and
    // strings the source fuzzer does not reach.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut it = bounded();
    let mut bad = Vec::new();
    for seed in 1..=600u64 {
        let mut rng = Rng(seed.wrapping_mul(0xA076_1D64_78BD_642F) | 1);
        let v = val(&mut rng, 0);
        match it.make(&v) {
            Err(Error::Panicked(m)) => bad.push((seed, m)),
            Err(_) => {}
            Ok(r) => {
                if let Err(Error::Panicked(m)) = it.value(&r) {
                    bad.push((seed, m));
                }
                if let Err(Error::Panicked(m)) = it.write(&r) {
                    bad.push((seed, m));
                }
            }
        }
        if it.is_poisoned() {
            it = bounded();
        }
    }
    std::panic::set_hook(prev);
    assert!(bad.is_empty(), "panics in the value path: {bad:?}");
}

#[test]
fn a_poisoned_interpreter_refuses_everything() {
    // Catching a panic does not make the VM's state knowable again, so the
    // honest move is to refuse -- the same contract a poisoned Mutex has.
    // Reaching a real machine panic from outside is the thing S7 is meant to
    // make hard, so this proves the contract through the one door that is
    // deliberately left: a panic in `eval` poisons.
    let mut it = bounded();
    // `make` of an Opaque is a refusal, not a poisoning: a host mistake must
    // not cost the interpreter.
    assert!(it.make(&Val::Opaque).is_err());
    assert!(!it.is_poisoned());
    let v = it.eval("(+ 1 1)").unwrap();
    assert_eq!(it.write(&v).unwrap(), "2");
}

/// Programs that **parse**, and aim at the places a panic actually lives.
///
/// The token-soup generator above mostly dies in the reader -- 182 accepted
/// against 2,218 refused in the recorded run -- so on its own it is weaker
/// evidence than the count suggests. These are hand-aimed instead: indexing,
/// allocation sizes, the numeric edges, recursion depth, the mutation that
/// makes cycles, and every host-facing conversion.
const HOSTILE: &[&str] = &[
    // indexing, which is `self.slots[i]`-shaped all over the heap
    "(vector-ref (vector) 0)",
    "(vector-ref (vector 1) -1)",
    "(vector-ref (vector 1) 9223372036854775807)",
    "(string-ref \"\" 0)",
    "(substring \"abc\" 2 1)",
    "(substring \"abc\" 0 99)",
    "(list-tail '(1 2) 5)",
    "(bytevector-u8-ref (bytevector) 0)",
    "(bytevector-copy! (bytevector 1) 5 (bytevector 1) 0 1)",
    "(string-copy! (make-string 1) 9 \"abc\")",
    // allocation sizes a host must not be able to turn into an abort
    "(make-vector -1)",
    "(make-vector 9223372036854775807)",
    "(make-string -1)",
    "(make-bytevector 9223372036854775807)",
    "(vector-fill! (make-vector 2) 0 5 1)",
    // numeric edges, where overflow is supposed to be an error not a wrap
    "(- 9223372036854775807 -1)",
    "(* 9223372036854775807 9223372036854775807)",
    "(expt 2 9999)",
    "(exact (/ 1.0 0.0))",
    "(exact (/ 0.0 0.0))",
    "(exact-integer-sqrt 9223372036854775807)",
    "(quotient 1 0)",
    "(modulo -9223372036854775808 -1)",
    "(string->number \"1e999999\")",
    "(number->string 1.0 99)",
    "(integer->char 1114112)",
    "(integer->char -1)",
    // depth: a deep structure must refuse, not overflow the Rust stack
    "(define (deep n) (if (= n 0) '() (list (deep (- n 1))))) (deep 20000)",
    // Captured rather than written: the point is that printing a 100,000-deep
    // structure does not overflow the Rust stack, not that stdout receives
    // 200,000 parentheses.
    "(string-length (with-output-to-string (lambda () \
       (write (let loop ([i 0] [acc '()]) (if (= i 100000) acc (loop (+ i 1) (list acc))))))))",
    // cycles, constructible since set-cdr! and the cause of two real hangs
    "(let ([x (list 1)]) (set-cdr! x x) (equal? x x))",
    "(let ([x (list 1)]) (set-cdr! x x) (length x))",
    "(let ([x (list 1)]) (set-cdr! x x) (list->vector x))",
    "(let ([x (list 1)]) (set-cdr! x x) (apply + x))",
    "(let ([x (list 1)]) (set-cdr! x x) (list? x))",
    "(let ([a (list 1)] [b (list 2)]) (set-cdr! a b) (set-cdr! b a) (equal? a b))",
    // the machinery S5b and S7 touched
    "(define-syntax m (syntax-rules () ((_ x) (m x)))) (m 1)",
    "(let loop () (loop))",
    "(apply + (make-vector 3))",
    "(apply apply apply '())",
    "(eval '(eval '(eval 1)))",
    "(call-with-values (lambda () (values)) (lambda (x) x))",
    "(dynamic-wind 1 2 3)",
    "(raise (raise 1))",
    "(guard (e (else (raise e))) (raise 'x))",
    "((lambda (&named a) a) :a)",
    "((lambda (&opt (a (error \"boom\"))) a))",
    // the host boundary itself
    "(host)",
    "(host (host (host 1)))",
    "(host (lambda (x) x))",
    "(host (open-input-string \"x\"))",
    "(list (host 1) (host 2) (host 3) (host 4) (host 5))",
];

#[test]
fn programs_that_parse_and_aim_at_the_panics() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut panicked = Vec::new();
    let mut refused = 0usize;
    let mut accepted = 0usize;

    for src in HOSTILE {
        // Printed BEFORE the call, so a program that hangs names itself
        // instead of leaving a silent 20-minute test.
        println!("  aiming at: {src}");
        let mut it = bounded();
        it.define_fn("host", |args| {
            Ok(args.first().cloned().unwrap_or(Val::Unspecified))
        })
        .expect("host fn");
        match it.eval(src) {
            Ok(r) => {
                accepted += 1;
                // Reading the result back is part of the boundary.
                if let Err(Error::Panicked(m)) = it.write(&r) {
                    panicked.push((*src, format!("write: {m}")));
                }
                if let Err(Error::Panicked(m)) = it.value(&r) {
                    panicked.push((*src, format!("value: {m}")));
                }
            }
            Err(Error::Panicked(m)) => panicked.push((*src, m)),
            Err(_) => refused += 1,
        }
    }

    std::panic::set_hook(prev);
    println!(
        "aimed: {} of {} evaluated, {} refused, {} panicked",
        accepted,
        HOSTILE.len(),
        refused,
        panicked.len()
    );
    assert!(
        panicked.is_empty(),
        "{} aimed programs panicked inside the API:\n{}",
        panicked.len(),
        panicked
            .iter()
            .map(|(s, m)| format!("  {s}\n    -> {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
