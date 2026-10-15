//! What shape must a linear-algebra procedure have to be compiled?
//!
//! The answer decides the whole design of `(scm linalg)`, so it is pinned
//! here rather than inferred: a leaf takes **scalars**, may read a global
//! table, and returns one or three `f64`s. A sequence can be the return
//! shape but not an intermediate value and not a parameter.

use scm::Vm;
use scm_llvm::{Shape, compile};

/// The claim `(scm linalg)` is built on: its component layer really is
/// leaf-compilable, and its vector layer really is not. Tested against the
/// shipped library rather than a fixture, so the library cannot drift away
/// from its own documentation.
#[test]
fn the_linalg_component_layer_compiles() {
    let mut vm = Vm::new();
    vm.eval_str("(import (scm linalg))").expect("import (scm linalg)");
    for (name, shape) in [
        ("v3-dot*", Shape::Scalar),
        ("v3-length*", Shape::Scalar),
        ("v3-length-squared*", Shape::Scalar),
        ("v3-distance*", Shape::Scalar),
        ("v3-distance-squared*", Shape::Scalar),
        ("v3-cross*", Shape::Vec3),
        ("v3-normalize*", Shape::Vec3),
        ("v3-add*", Shape::Vec3),
        ("v3-sub*", Shape::Vec3),
        ("v3-scale*", Shape::Vec3),
        ("v3-lerp*", Shape::Vec3),
        ("v3-reflect*", Shape::Vec3),
    ] {
        let r = compile(&mut vm, name, shape);
        assert!(r.is_ok(), "{name} should compile: {:?}", r.err());
    }
}

/// And the vector layer is refused, which is why both layers exist.
#[test]
fn the_linalg_vector_layer_is_refused() {
    let mut vm = Vm::new();
    vm.eval_str("(import (scm linalg))").expect("import (scm linalg)");
    for name in ["v3-dot", "v3-cross", "v3-length", "v3-normalize"] {
        let r = compile(&mut vm, name, Shape::Scalar);
        assert!(r.is_err(), "{name} takes vectors, so it cannot be compiled");
    }
}

const SRC: &str = r#"
;; component-wise: scalars in, scalar out
(define (dot* ax ay az bx by bz) (+ (* ax bx) (* ay by) (* az bz)))
(define (len* x y z) (sqrt (dot* x y z x y z)))
;; component-wise: scalars in, three out
(define (cross* ax ay az bx by bz)
  (list (- (* ay bz) (* az by))
        (- (* az bx) (* ax bz))
        (- (* ax by) (* ay bx))))
(define (normalize* x y z)
  (let ([n (len* x y z)])
    (if (= n 0.0) (list 0.0 0.0 0.0) (list (/ x n) (/ y n) (/ z n)))))
;; the vector-valued spelling, for comparison
(define (dot-v a b)
  (+ (* (vector-ref a 0) (vector-ref b 0))
     (* (vector-ref a 1) (vector-ref b 1))
     (* (vector-ref a 2) (vector-ref b 2))))
;; an intermediate sequence
(define (via-intermediate ax ay az)
  (let ([v (list ax ay az)]) (vector-ref v 0)))
"#;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.eval_str(SRC).expect("fixture");
    vm
}

#[test]
fn component_wise_procedures_compile() {
    let mut vm = vm();
    for (name, shape) in [
        ("dot*", Shape::Scalar),
        ("len*", Shape::Scalar),
        ("cross*", Shape::Vec3),
        ("normalize*", Shape::Vec3),
    ] {
        let r = compile(&mut vm, name, shape);
        assert!(r.is_ok(), "{name} should compile: {:?}", r.err());
    }
}

/// The reason `(scm linalg)` has a component-wise layer at all: a procedure
/// that takes a *vector* cannot be compiled, because a sequence is not a
/// value in the leaf subset.
#[test]
fn a_vector_parameter_is_refused() {
    let mut vm = vm();
    let r = compile(&mut vm, "dot-v", Shape::Scalar);
    assert!(r.is_err(), "a vector parameter must be refused");
    let msg = r.err().unwrap().message;
    assert!(!msg.is_empty(), "the refusal should say why");
}

/// A sequence may be the return shape, but not an intermediate value.
#[test]
fn an_intermediate_sequence_is_refused() {
    let mut vm = vm();
    let r = compile(&mut vm, "via-intermediate", Shape::Scalar);
    assert!(r.is_err(), "an intermediate sequence must be refused");
    assert!(
        r.err().unwrap().message.contains("allocates"),
        "the refusal should name the allocation"
    );
}
