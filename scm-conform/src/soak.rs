//! Sustained use, as opposed to intense use.
//!
//! `gc.rs` collects at **every safepoint**, which is the right way to find a
//! collector bug: a value held only in a Rust local shows up immediately and
//! deterministically. What it cannot find is anything that only appears over
//! time -- a slow leak of a few cells per round, or a non-moving heap
//! fragmenting until the high-water mark climbs without the live set
//! climbing with it.
//!
//! So this runs rounds of ordinary allocating work and watches two numbers
//! after a forced collection at each round's end:
//!
//!   * **live objects** -- if this trends up, something is retained.
//!   * **the slot high-water** -- if this trends up while live does not,
//!     the heap is fragmenting.
//!
//! The verdict is a comparison of windows rather than an absolute: a leak of
//! `k` cells per round is invisible in one round and unmistakable over
//! hundreds, which is the whole point of measuring this way.

use scm::Vm;

/// One round of work. Deliberately a mix: each line is a different shape of
/// allocation, and several of them are the shapes a collector gets wrong --
/// a cycle, a closure over mutable state, a structure mutated after it is
/// reachable, and an escape that abandons frames.
pub const ROUND: &str = r#"
(define (round-work n)
  ;; closures over mutable state, called and dropped
  (let ((counters (map (lambda (i) (let ((c i)) (lambda () (set! c (+ c 1)) c)))
                       (iota 40))))
    (for-each (lambda (f) (f) (f)) counters))
  ;; a cycle, made unreachable before the round ends
  (let ((a (list 1 2 3)) (b (list 4 5 6)))
    (set-cdr! (cddr a) b)
    (set-cdr! (cddr b) a)
    (car a))
  ;; a structure mutated after it is reachable from a vector
  (let ((v (make-vector 50 '())))
    (let loop ((i 0))
      (if (< i 50)
          (begin (vector-set! v i (cons i (vector-ref v (max 0 (- i 1)))))
                 (loop (+ i 1)))))
    (vector-length v))
  ;; strings and a string port
  (let ((s (with-output-to-string (lambda () (display (iota 60))))))
    (string-length (string-append s s)))
  ;; records
  (let ((ps (map (lambda (i) (make-pt i (* i i))) (iota 30))))
    (apply + (map pt-y ps)))
  ;; a hash table, filled and dropped
  (let ((h (make-hash-table equal?)))
    (let loop ((i 0))
      (if (< i 60) (begin (hash-table-set! h (list i) i) (loop (+ i 1)))))
    (hash-table-size h))
  ;; deep non-tail recursion, so frames are allocated and released
  (let ((deep (lambda (k) (let rec ((i 0)) (if (= i k) 0 (+ 1 (rec (+ i 1))))))))
    (deep 400))
  ;; an escape continuation abandoning frames mid-flight
  (call/cc (lambda (k) (let rec ((i 0)) (if (= i 300) (k 'escaped) (begin (rec (+ i 1)) 'no)))))
  ;; exact integers past the word, so bignums are allocated too
  (let loop ((i 0) (acc 1)) (if (= i 30) (modulo acc 1000003) (loop (+ i 1) (* acc 1000003))))
  n)
(round-work 1)
"#;

/// The prelude a round needs: a record type and a hash-table constructor.
pub const SETUP: &str = r#"
(import (srfi 1) (srfi 69))
(define-record-type pt (make-pt x y) pt? (x pt-x) (y pt-y))
"#;

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub round: usize,
    /// Live objects after a forced collection.
    pub live: usize,
    /// Slots ever allocated: the heap's high-water mark.
    pub slots: usize,
    pub collections: u64,
}

pub struct Soak {
    vm: Vm,
    pub samples: Vec<Sample>,
}

impl Soak {
    pub fn new() -> Self {
        let mut vm = Vm::new();
        vm.eval_str(SETUP).expect("soak setup");
        Soak { vm, samples: Vec::new() }
    }

    /// Run `rounds` rounds, sampling after a forced collection at the end of
    /// each. Returns `Err` with the round number if the work itself failed.
    pub fn run(&mut self, rounds: usize) -> Result<(), (usize, String)> {
        for round in 0..rounds {
            self.vm
                .eval_str(ROUND)
                .map_err(|e| (round, format!("{e}")))?;
            self.vm.collect_now();
            self.samples.push(Sample {
                round,
                live: self.vm.engine.obj_count(),
                slots: self.vm.engine.slot_count(),
                collections: self.vm.engine.collections(),
            });
        }
        Ok(())
    }

    /// The steady-state verdict.
    ///
    /// Compares the **worst** reading in the last fifth of the run against
    /// the worst in the fifth after warmup. A max rather than a mean,
    /// because a leak that only shows at a peak is still a leak; and a ratio
    /// rather than a difference, because the absolute size is the round's
    /// business and not ours.
    pub fn verdict(&self) -> Verdict {
        let n = self.samples.len();
        let fifth = (n / 5).max(1);
        let early = &self.samples[fifth..fifth * 2];
        let late = &self.samples[n - fifth..];
        let worst = |w: &[Sample], f: fn(&Sample) -> usize| w.iter().map(f).max().unwrap_or(0);
        Verdict {
            rounds: n,
            early_live: worst(early, |s| s.live),
            late_live: worst(late, |s| s.live),
            early_slots: worst(early, |s| s.slots),
            late_slots: worst(late, |s| s.slots),
            collections: self.samples.last().map(|s| s.collections).unwrap_or(0),
        }
    }
}

impl Default for Soak {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Verdict {
    pub rounds: usize,
    pub early_live: usize,
    pub late_live: usize,
    pub early_slots: usize,
    pub late_slots: usize,
    pub collections: u64,
}

impl Verdict {
    /// Live objects must not trend up: that is retention.
    pub fn live_growth(&self) -> f64 {
        self.late_live as f64 / self.early_live.max(1) as f64
    }

    /// The high-water mark must not trend up either. Growing here while
    /// `live_growth` stays flat is what fragmentation looks like.
    pub fn slot_growth(&self) -> f64 {
        self.late_slots as f64 / self.early_slots.max(1) as f64
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} rounds, {} collections\n  live  {} -> {}  ({:.3}x)\n  slots {} -> {}  ({:.3}x)",
            self.rounds,
            self.collections,
            self.early_live,
            self.late_live,
            self.live_growth(),
            self.early_slots,
            self.late_slots,
            self.slot_growth(),
        )
    }
}
