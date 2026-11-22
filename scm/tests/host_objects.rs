//! lux's plan M1–M3, as tests: float vectors that cross, arguments read in
//! place, and values the host makes that Scheme holds.
//!
//! lux draws with wgpu from Scheme, so its buffers and textures are values
//! a Scheme renderer keeps, and its uniforms are `f32vector`s written every
//! frame. Before these, a buffer could only be an integer the host looked
//! up, with no lifetime, and an `f32vector` reached a host function as
//! `Val::Opaque` -- its numbers never arrived.

use std::cell::Cell;
use std::rc::Rc;

use scm::{HostObject, Interp, Val};

/// Enough allocation to collect several times over.
const CHURN: &str = "(let loop ((i 0)) (if (= i 200000) i (begin (make-vector 4 i) (loop (+ i 1)))))";

/// A host value that counts its own drops, standing for a GPU buffer.
struct Buffer {
    size: usize,
    dropped: Rc<Cell<usize>>,
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}

fn with_buffers() -> (Interp, Rc<Cell<usize>>) {
    let mut it = Interp::new().unwrap();
    let dropped = Rc::new(Cell::new(0));
    let d = dropped.clone();
    it.define_fn_args("make-buffer", move |a| {
        let size = a.int(0)? as usize;
        Ok(Val::Host(HostObject::new("buffer", Buffer { size, dropped: d.clone() })))
    })
    .unwrap();
    it.define_fn_args("buffer-size", |a| {
        let b = a.host_as::<Buffer>(0, "buffer")?;
        Ok(Val::Int(b.size as i64))
    })
    .unwrap();
    (it, dropped)
}

fn eval(it: &mut Interp, src: &str) -> String {
    let r = it.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    it.write(&r).unwrap()
}

// ---- M3: host objects ---------------------------------------------------

#[test]
fn a_host_object_is_a_value_scheme_holds_and_passes_back() {
    let (mut it, _) = with_buffers();
    assert_eq!(eval(&mut it, "(define b (make-buffer 256)) (buffer-size b)"), "256");
    assert_eq!(eval(&mut it, "b"), "#<buffer>");
    assert_eq!(eval(&mut it, "(host-object? b)"), "#t");
    assert_eq!(eval(&mut it, "(host-object? (vector 1 2))"), "#f");
    assert_eq!(eval(&mut it, "(host-object-kind b)"), "buffer");
    // It goes in lists, vectors and records like any value.
    assert_eq!(eval(&mut it, "(buffer-size (cadr (list 1 b 3)))"), "256");
}

#[test]
fn the_same_host_value_is_the_same_scheme_object() {
    // Handed to Scheme twice, one buffer is one object: eq?, and a key an
    // eq? table finds.
    let mut it = Interp::new().unwrap();
    let shared = HostObject::new("buffer", 7u32);
    let s = shared.clone();
    it.define_fn_args("the-buffer", move |_| Ok(Val::Host(s.clone()))).unwrap();
    it.define_fn_args("identity", |a| a.val(0)).unwrap();
    assert_eq!(eval(&mut it, "(eq? (the-buffer) (the-buffer))"), "#t");
    assert_eq!(eval(&mut it, "(let ((b (the-buffer))) (eq? b (identity b)))"), "#t");
    assert_eq!(
        eval(&mut it, "(import (srfi 69)) (let ((t (make-hash-table eq?))) (hash-table-set! t (the-buffer) 'found) (hash-table-ref/default t (the-buffer) #f))"),
        "found"
    );
    // Two values are two objects, whatever they hold.
    let other = HostObject::new("buffer", 7u32);
    it.define_fn_args("another-buffer", move |_| Ok(Val::Host(other.clone()))).unwrap();
    assert_eq!(eval(&mut it, "(eq? (the-buffer) (another-buffer))"), "#f");
    assert_eq!(eval(&mut it, "(equal? (the-buffer) (another-buffer))"), "#f");
}

#[test]
fn a_host_object_is_released_when_scheme_lets_go() {
    let (mut it, dropped) = with_buffers();
    eval(&mut it, "(define keep (make-buffer 1))");
    eval(&mut it, "(make-buffer 2) (make-buffer 3)"); // made and dropped
    it.eval(CHURN).unwrap();
    assert_eq!(dropped.get(), 2, "the two nobody holds are released, the held one is not");
    assert_eq!(eval(&mut it, "(buffer-size keep)"), "1");
    // `keep` has lived through collections, so it is OLD, and minor
    // collections no longer look at it: it goes at the next FULL one. A
    // host holding large GPU buffers can ask for that.
    eval(&mut it, "(set! keep #f)");
    it.eval(CHURN).unwrap();
    assert_eq!(dropped.get(), 2, "an old object waits for a full collection");
    it.collect_garbage().unwrap();
    assert_eq!(dropped.get(), 3);
}

#[test]
fn a_host_object_the_host_also_holds_outlives_scheme() {
    // The host's own clone keeps the value; Scheme's object goes, and the
    // value handed back later is a fresh object for the same value.
    let mut it = Interp::new().unwrap();
    let dropped = Rc::new(Cell::new(0));
    let held = HostObject::new("buffer", Buffer { size: 9, dropped: dropped.clone() });
    let h = held.clone();
    it.define_fn_args("held-buffer", move |_| Ok(Val::Host(h.clone()))).unwrap();
    eval(&mut it, "(held-buffer)");
    it.eval(CHURN).unwrap();
    assert_eq!(dropped.get(), 0);
    assert_eq!(held.get::<Buffer>().map(|b| b.size), Some(9));
    assert_eq!(eval(&mut it, "(host-object-kind (held-buffer))"), "buffer");
}

#[test]
fn the_wrong_host_object_is_refused_by_name() {
    let (mut it, _) = with_buffers();
    it.define_fn_args("make-texture", |_| Ok(Val::Host(HostObject::new("texture", "pixels")))).unwrap();
    let e = it.eval("(buffer-size (make-texture))").unwrap_err().to_string();
    assert!(e.contains("buffer-size: argument 1 must be a buffer, got #<texture>"), "{e}");
    let e = it.eval("(buffer-size 5)").unwrap_err().to_string();
    assert!(e.contains("argument 1 must be a host object, got 5"), "{e}");
    let e = it.eval("(host-object-kind 5)").unwrap_err().to_string();
    assert!(e.contains("not a host object"), "{e}");
}

// ---- M2: arguments read in place ----------------------------------------

#[test]
fn an_f32vector_is_borrowed_not_copied() {
    let mut it = Interp::new().unwrap();
    it.define_fn_args("sum", |a| Ok(Val::Float(a.f32s(0)?.iter().map(|&x| x as f64).sum())))
        .unwrap();
    assert_eq!(eval(&mut it, "(import (srfi 4)) (sum (f32vector 1.5 2.5 4))"), "8.0");
    assert_eq!(eval(&mut it, "(sum (make-f32vector 100000 0.5))"), "50000.0");
    let e = it.eval("(sum (f64vector 1.0))").unwrap_err().to_string();
    assert!(e.contains("sum: argument 1 must be an f32vector, got #f64(1.0)"), "{e}");
}

#[test]
fn a_host_function_writes_into_an_f32vector() {
    // How a GPU readback reaches Scheme: the host fills a vector Scheme made.
    let mut it = Interp::new().unwrap();
    it.define_fn_args("fill!", |a| {
        let k = a.real(1)? as f32;
        for (i, x) in a.f32s_mut(0)?.iter_mut().enumerate() {
            *x = k * i as f32;
        }
        Ok(Val::Unspecified)
    })
    .unwrap();
    assert_eq!(eval(&mut it, "(import (srfi 4)) (let ((v (make-f32vector 4 0.0))) (fill! v 2) v)"), "#f32(0.0 2.0 4.0 6.0)");
}

#[test]
fn the_other_accessors_read_and_refuse() {
    let mut it = Interp::new().unwrap();
    it.define_fn_args("probe", |a| {
        Ok(Val::List(vec![
            Val::Int(a.len() as i64),
            Val::Int(a.int(0)?),
            Val::Float(a.real(1)?),
            Val::Int(a.bytes(2)?.len() as i64),
            Val::Float(a.f64s(3)?[1]),
            a.val(4)?,
        ]))
    })
    .unwrap();
    assert_eq!(
        eval(&mut it, "(import (srfi 4)) (probe 3 2 (bytevector 1 2) (f64vector 0.5 1.5) '(:at 1))"),
        "(5 3 2.0 2 1.5 (:at 1))"
    );
    let e = it.eval("(probe 1.5 2 (bytevector) (f64vector 0.0 0.0) 0)").unwrap_err().to_string();
    assert!(e.contains("probe: argument 1 must be an exact integer, got 1.5"), "{e}");
    let e = it.eval("(probe 1)").unwrap_err().to_string();
    assert!(e.contains("probe: expects at least 2 arguments, got 1"), "{e}");
}

#[test]
fn a_panic_in_an_args_function_is_a_scheme_error() {
    let mut it = Interp::new().unwrap();
    it.define_fn_args("boom", |_| panic!("the host broke")).unwrap();
    let e = it.eval("(boom)").unwrap_err().to_string();
    assert!(e.contains("boom") || e.contains("the host broke"), "{e}");
    assert_eq!(eval(&mut it, "(+ 1 2)"), "3", "the interpreter is still usable");
}

// ---- M1: float vectors cross as values -----------------------------------

#[test]
fn float_vectors_cross_both_ways() {
    let mut it = Interp::new().unwrap();
    let r = it.eval("(import (srfi 4)) (f32vector 1.5 -2)").unwrap();
    assert_eq!(it.value(&r).unwrap(), Val::F32s(vec![1.5, -2.0]));
    let r = it.eval("(f64vector 0.25)").unwrap();
    assert_eq!(it.value(&r).unwrap(), Val::F64s(vec![0.25]));
    it.define_fn("halves", |_| Ok(Val::F32s(vec![0.5, 0.5]))).unwrap();
    assert_eq!(eval(&mut it, "(let ((v (halves))) (list (f32vector? v) (f32vector-length v)))"), "(#t 2)");
    // Other numeric vectors are still opaque to a host.
    let r = it.eval("(s16vector 1 2)").unwrap();
    assert_eq!(it.value(&r).unwrap(), Val::Opaque);
}
