//! A leaf that samples bulk data: numeric vectors and arrays.
//!
//! This is what makes `(srfi 4)` and `(srfi 63)` pay off in a render
//! rather than only in scripting. An image, a height field or a volume is
//! captured as a **snapshot** -- plan §6 item (2) -- held by the compiled
//! handle, so the data outlives every caller and a render worker calling
//! an older generation keeps the data that generation read.

use rayon::prelude::*;
use scm::{Value, Vm};
use scm_llvm::{Live, Shape, compile, verdict};

const SRC: &str = r#"
(import (srfi 4) (srfi 63))

;; A height field, and a 4x4 image over an f64 store.
(define heights (f64vector 0.0 1.0 2.0 3.0 4.0 5.0 6.0 7.0))
(define weights (f64vector 0.5 0.25 0.125 0.0625 0.03125 0.015625 0.0078125 0.00390625))
(define img (make-array a:flor64b 4 4))
(let loop ((i 0))
  (when (< i 4)
    (let inner ((j 0))
      (when (< j 4)
        (array-set! img (+ (* 10.0 i) j) i j)
        (inner (+ j 1))))
    (loop (+ i 1))))

(define (h-at i) (f64vector-ref heights (modulo (exact (floor i)) 8)))
(define (w-at i) (f64vector-ref weights (modulo (exact (floor i)) 8)))
;; Two captured vectors of the SAME length, which is what the old
;; find-the-table-by-its-length lookup got wrong.
(define (both i) (+ (h-at i) (w-at i)))
(define (raw i) (f64vector-ref heights i))
(define (how-many) (f64vector-length heights))
(define (pixel y x) (array-ref img y x))
;; The shape of a real texel function: four taps and a blend.
(define (bilinear y x)
  (let* ((y0 (exact (floor y))) (x0 (exact (floor x)))
         (y1 (modulo (+ y0 1) 4)) (x1 (modulo (+ x0 1) 4))
         (fy (- y y0)) (fx (- x x0))
         (a (array-ref img y0 x0)) (b (array-ref img y0 x1))
         (c (array-ref img y1 x0)) (d (array-ref img y1 x1)))
    (+ (* (- 1.0 fy) (+ (* (- 1.0 fx) a) (* fx b)))
       (* fy (+ (* (- 1.0 fx) c) (* fx d))))))

;; A reversing view: a negative stride, which the index arithmetic must
;; handle as a signed quantity.
(define back (make-shared-array img (lambda (j) (list 0 (- 3 j))) 4))
(define (backwards j) (array-ref back j))

;; And what must be refused: a leaf may not write data N threads share.
(define (poke i) (begin (f64vector-set! heights i 9.0) 1.0))
(define (poke-array i) (begin (array-set! img 9.0 0 0) 1.0))
"#;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.eval_str(SRC).unwrap();
    vm
}

fn t0(vm: &mut Vm, src: &str) -> f64 {
    match vm.eval_str(src) {
        Ok(Value::Flonum(x)) => x,
        Ok(Value::Fixnum(n)) => n as f64,
        other => panic!("T0 on {src}: {other:?}"),
    }
}

#[test]
fn a_leaf_that_samples_a_numeric_vector_agrees_with_t0() {
    let mut vm = vm();
    // The index is spelled as an *exact* integer for T0 and arrives as the
    // same `f64` value compiled: the leaf subset has one numeric type, so
    // an index is integral-but-inexact there while `numvector-ref` wants
    // an exact one. The same boundary the S11 fixture met with `fbm`'s
    // octave count; the value compared is identical either way.
    //
    // Only in-range indices here. Out of range the two deliberately
    // differ -- T0 raises, a leaf has no way to and reads NaN -- which the
    // next test is about.
    for (name, arity) in [("h-at", 1), ("w-at", 1), ("both", 1), ("how-many", 0), ("raw", 1)] {
        let f = compile(&mut vm, name, Shape::Scalar).unwrap_or_else(|r| panic!("{name}: {r}"));
        for k in 0..8 {
            let args: Vec<f64> = (0..arity).map(|_| k as f64).collect();
            let native = f.call(&args).unwrap();
            let call = format!(
                "({name} {})",
                args.iter().map(|a| format!("{}", *a as i64)).collect::<Vec<_>>().join(" ")
            );
            let interp = t0(&mut vm, &call);
            assert_eq!(native.to_bits(), interp.to_bits(), "{call}");
        }
    }
    // Two captured vectors of the same length must not be confused:
    // `both` reads one element from each, and the old lookup found a
    // table by matching its length.
    let f = compile(&mut vm, "both", Shape::Scalar).unwrap();
    assert_eq!(f.call(&[1.0]), Some(1.25));
    assert_eq!(f.call(&[2.0]), Some(2.125));
}

#[test]
fn a_leaf_that_samples_an_array_agrees_with_t0() {
    let mut vm = vm();
    // `pixel` takes subscripts, so they are exact for T0 as above.
    let pixel = compile(&mut vm, "pixel", Shape::Scalar).unwrap();
    for i in 0..4 {
        for j in 0..4 {
            let native = pixel.call(&[i as f64, j as f64]).unwrap();
            let interp = t0(&mut vm, &format!("(pixel {i} {j})"));
            assert_eq!(native.to_bits(), interp.to_bits(), "pixel {i} {j}");
        }
    }
    // `bilinear` takes coordinates and floors them itself, so both sides
    // see the same inexact arguments.
    let bilinear = compile(&mut vm, "bilinear", Shape::Scalar).unwrap();
    for i in 0..4 {
        for j in 0..4 {
            let (y, x) = (i as f64 + 0.25, j as f64 + 0.5);
            let native = bilinear.call(&[y, x]).unwrap();
            let interp = t0(&mut vm, &format!("(bilinear {y:?} {x:?})"));
            assert_eq!(native.to_bits(), interp.to_bits(), "bilinear {y} {x}");
        }
    }
    // A reversing view: the stride is negative and must stay signed.
    let back = compile(&mut vm, "backwards", Shape::Scalar).unwrap();
    for j in 0..4 {
        let native = back.call(&[j as f64]).unwrap();
        let interp = t0(&mut vm, &format!("(backwards {j})"));
        assert_eq!(native, interp, "backwards {j}");
    }
    assert_eq!(back.call(&[0.0]), Some(3.0));
}

#[test]
fn an_index_outside_the_data_reads_nan_rather_than_memory() {
    let mut vm = vm();
    let raw = compile(&mut vm, "raw", Shape::Scalar).unwrap();
    assert_eq!(raw.call(&[7.0]), Some(7.0));
    assert!(raw.call(&[8.0]).unwrap().is_nan(), "past the end");
    assert!(raw.call(&[-1.0]).unwrap().is_nan(), "before the start");
    assert!(raw.call(&[1e18]).unwrap().is_nan(), "absurdly far past the end");
    // Each array subscript is checked against its own dimension, so an
    // index that is inside the store but outside the shape is refused too.
    let pixel = compile(&mut vm, "pixel", Shape::Scalar).unwrap();
    assert_eq!(pixel.call(&[1.0, 1.0]), Some(11.0));
    assert!(pixel.call(&[4.0, 0.0]).unwrap().is_nan(), "row past the shape");
    assert!(pixel.call(&[0.0, 4.0]).unwrap().is_nan(), "column past the shape");
    assert!(pixel.call(&[-1.0, 0.0]).unwrap().is_nan());
}

#[test]
fn writing_shared_data_is_refused() {
    // A leaf runs on N render threads at once, so there is no safe way to
    // let it write what they all read.
    let mut vm = vm();
    // `f64vector-set!` is the primitive `numvector-set!`, which is not on
    // the pure list -- and the message says so under either name.
    let r = verdict(&mut vm, "poke").unwrap_err();
    assert!(r.message.contains("not a pure primitive"), "{r}");
    assert!(r.message.contains("numvector-set!"), "{r}");
    // `array-set!` is refused too, though for the other reason available:
    // it is `(srfi 63)`'s Scheme procedure with a rest parameter, so it
    // has no fixed native signature. Refused is refused.
    let r = verdict(&mut vm, "poke-array").unwrap_err();
    assert!(
        r.message.contains("rest parameter") || r.message.contains("not a pure primitive"),
        "{r}"
    );
}

#[test]
fn the_captured_data_is_a_snapshot_and_refresh_catches_up() {
    let mut vm = vm();
    let old = compile(&mut vm, "h-at", Shape::Scalar).unwrap();
    assert_eq!(old.call(&[3.0]), Some(3.0));
    // Mutating the vector does not reach code already compiled: that is
    // what "snapshot" means, and the reason a render in flight stays
    // consistent.
    vm.eval_str("(f64vector-set! heights 3 99.0)").unwrap();
    assert_eq!(old.call(&[3.0]), Some(3.0));
    // Recompiling takes the new contents.
    let new = compile(&mut vm, "h-at", Shape::Scalar).unwrap();
    assert_eq!(new.call(&[3.0]), Some(99.0));
    // And the old code still works, because its snapshot is held by its
    // own handle -- the S11a property, now for bulk data.
    assert_eq!(old.call(&[3.0]), Some(3.0));
    // `Live` is the same story through the generation counter. A write to
    // the vector's elements moves no binding, so `recompile` is the
    // explicit way to pick it up.
    let live = Live::compile(&mut vm, "h-at", Shape::Scalar).unwrap();
    assert_eq!(live.snapshot().0.call(&[3.0]), Some(99.0));
    vm.eval_str("(f64vector-set! heights 3 7.5)").unwrap();
    assert_eq!(live.refresh(&mut vm).unwrap(), None, "no binding changed");
    live.recompile(&mut vm).unwrap();
    assert_eq!(live.snapshot().0.call(&[3.0]), Some(7.5));
}

#[test]
fn a_sampling_leaf_runs_from_a_rayon_pool_after_the_vm_is_gone() {
    let bilinear = {
        let mut vm = vm();
        compile(&mut vm, "bilinear", Shape::Scalar).unwrap()
        // The Vm -- and the heap holding the array -- drops here. The
        // snapshot the handle carries is what keeps the data alive.
    };
    const N: usize = 256;
    let serial: Vec<f64> = (0..N * N)
        .map(|i| bilinear.call(&[(i / N) as f64 * 0.01, (i % N) as f64 * 0.01]).unwrap())
        .collect();
    let parallel: Vec<f64> = (0..N * N)
        .into_par_iter()
        .map(|i| bilinear.call(&[(i / N) as f64 * 0.01, (i % N) as f64 * 0.01]).unwrap())
        .collect();
    assert_eq!(serial, parallel);
    assert!(serial.iter().all(|x| x.is_finite()), "every tap was in bounds");
    assert!(serial.iter().any(|x| *x != serial[0]), "the image is not flat");
}
