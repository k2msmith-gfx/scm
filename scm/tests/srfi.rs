//! The shipped libraries, gated.
//!
//! Each program in `tests/srfi/` imports `(scm test)` and the library it
//! exercises, and its value is the list of failures -- so `()` is a pass
//! and anything else prints as the failing checks with what they got and
//! wanted. Every program runs on **both tiers**, because a library is
//! ordinary Scheme and the tiers must agree on it like anything else.
//!
//! The libraries are embedded in the binary, so these tests exercise the
//! embedded copies. A file of the same name on `SCM_LIBRARY_PATH` wins,
//! which is the loop for editing one without a rebuild.

use std::path::PathBuf;

use scm::{Tier, Vm, write_str};

fn test_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/srfi")
}

fn run(tier: Tier, name: &str) -> String {
    let path = test_dir().join(name);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut vm = Vm::with_tier(tier);
    match vm.eval_str_named(&src, name) {
        Ok(v) => write_str(&vm.engine, v),
        Err(e) => format!("the program failed: {e}\n{:#?}", vm.backtrace()),
    }
}

#[test]
fn srfi_1_passes_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "1.scm"), "()", "(srfi 1) on {tier:?}");
    }
}

#[test]
fn the_small_srfis_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "small.scm"), "()", "(srfi 2/8/26/111/132) on {tier:?}");
    }
}

#[test]
fn srfi_4_numeric_vectors_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "4.scm"), "()", "(srfi 4) on {tier:?}");
    }
}

#[test]
fn srfi_63_arrays_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "63.scm"), "()", "(srfi 63) on {tier:?}");
    }
}

#[test]
fn srfi_151_bitwise_passes_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "151.scm"), "()", "(srfi 151) on {tier:?}");
    }
}

#[test]
fn the_hash_table_and_comparator_srfis_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "hash.scm"), "()", "(srfi 69/128) on {tier:?}");
    }
}

/// `(srfi 125)` is the second interface onto the same table type, so this
/// also checks that a table made through one works through the other.
#[test]
fn the_intermediate_hash_table_srfi_passes_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "125.scm"), "()", "(srfi 125) on {tier:?}");
    }
}

/// `(srfi 160)`'s operations are written once and aliased per element type,
/// so this covers the whole set on `f64` and then checks that another type
/// and the bytevector-backed `u8` behave the same through their own names.
#[test]
fn the_numeric_vector_operations_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "160.scm"), "()", "(srfi 160) on {tier:?}");
    }
}

/// `(srfi 27)`'s point is reproducibility rather than distribution: a
/// procedural model has to be replayable, so the state must be savable,
/// restorable and derivable from a seed.
#[test]
fn the_random_source_srfi_passes_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "27.scm"), "()", "(srfi 27) on {tier:?}");
    }
}

/// `(srfi 194)`'s ranges are the easy half. This asserts the
/// *distributions*, because a biased sampler in a renderer is a faint
/// wrong answer rather than a crash: the sphere generator's z-component
/// has to be uniform, the ball's points uniform by volume, and Poisson's
/// variance equal to its mean. It runs under a fixed seed, so it is
/// deterministic rather than flaky.
#[test]
fn the_random_data_generators_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "194.scm"), "()", "(srfi 194) on {tier:?}");
    }
}

/// `(srfi 42)` is also the check that two of scm's own decisions coexist:
/// its generators are spelled `:list` and `:range`, which plan §4 makes
/// **keywords** rather than identifiers, so they can be neither macro
/// names nor `syntax-rules` literals. They work because a keyword in a
/// pattern matches as a constant and a generator is only ever a qualifier.
#[test]
fn the_eager_comprehensions_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "42.scm"), "()", "(srfi 42) on {tier:?}");
    }
}

/// `(srfi 158)` is also what finally consumes what two earlier libraries
/// were already producing: `(srfi 160)`'s `make-@vector-generator` and
/// `(srfi 194)`'s samplers are generators in this sense, and until this
/// library there was nothing to pull them with. The test asserts those
/// pipelines, not just the library in isolation.
#[test]
fn the_generators_and_accumulators_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "158.scm"), "()", "(srfi 158) on {tier:?}");
    }
}

/// `(srfi 13)` strings -- the library `(srfi 14)` was brought forward for.
/// It predates R7RS and is inconsistent with *itself* about argument
/// order, so most of this test pins orders: the criterion comes first in
/// `string-every`/`-any`/`-filter`/`-delete` and second in
/// `string-index`/`-skip`/`-count`/`-trim`/`-tokenize`; `string-pad`
/// truncates on the side it pads; `string-fold` takes the character
/// first; and the comparisons answer a mismatch index rather than `#t`.
/// `string-map` and `string-for-each` are defined as supersets of both
/// their R7RS and their SRFI 13 shapes, so importing this library does
/// not take the R7RS behaviour away, and both shapes are asserted.
#[test]
fn srfi_13_strings_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "13.scm"), "()", "(srfi 13) on {tier:?}");
    }
}

/// `(srfi 14)` char-sets, which `(srfi 13)` needs for twelve of its
/// procedures. Char-sets here are **Latin-1**, which the SRFI allows (it
/// makes `char-set:full` relative to the implementation's range), scm's
/// unclaimed `full-unicode` feature already implies, and the reference
/// implementation does itself. That bound has three different answers and
/// the test pins each: membership past it is `#f`, adding past it raises,
/// deleting past it is a no-op. The sixteen `!` variants are
/// **non-destructive**, as `(srfi 1)`'s are and for the same reason, so
/// the test asserts directly that a client cannot corrupt a shared
/// constant. It also pins where the reference's verbatim Latin-1 data
/// diverges from scm's own Unicode-correct predicates -- six characters
/// for `digit`, two for `lower-case`, one for `whitespace` -- so that a
/// change to either side fails here rather than surprising `(srfi 13)`.
#[test]
fn srfi_14_char_sets_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "14.scm"), "()", "(srfi 14) on {tier:?}");
    }
}

/// `(srfi 133)` is `(srfi 160)`'s counterpart for boxed elements, and
/// sixteen of its forty-two exports are R7RS base procedures the prelude
/// already defines to the SRFI's specification -- so they are re-exported
/// rather than rewritten, and the test checks that importing the library
/// really supplies them. The rest of the checks pin the three argument
/// orders this SRFI is easy to get backwards (`kons` takes the state
/// first, `cmp` the element first, `vector-unfold-right` fills from the
/// end) and the two empty-vector answers its prose leaves open.
#[test]
fn srfi_133_vector_operations_pass_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "133.scm"), "()", "(srfi 133) on {tier:?}");
    }
}

/// `(scm linalg)` is not an SRFI -- no SRFI covers linear algebra -- but it
/// is gated the same way. The checks that matter are the ones catching a
/// silently wrong picture: the cross product's handedness, ray's Euler
/// order, a normal under non-uniform scale, and the two layers agreeing.
#[test]
fn the_linear_algebra_library_passes_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "linalg.scm"), "()", "(scm linalg) on {tier:?}");
    }
}

/// `(scm noise)` has no right answer to compare against, so the gate is
/// on properties: deterministic, continuous across lattice boundaries, in
/// the range it claims, exactly zero on the lattice for gradient noise,
/// and exactly wrapping for the tiled variant. That every procedure also
/// compiles, and agrees with the compiled form, is asserted in
/// `scm-llvm/tests/leaf_noise.rs`.
#[test]
fn the_noise_library_passes_on_both_tiers() {
    for tier in [Tier::T0, Tier::T1] {
        assert_eq!(run(tier, "noise.scm"), "()", "(scm noise) on {tier:?}");
    }
}

#[test]
fn equal_hash_agrees_with_equal_and_terminates_on_a_cycle() {
    // The one contract a hash table rests on: if two things are `equal?`
    // they hash alike. `equal-hash` therefore walks exactly what `equal?`
    // recurses into, and bounds its work so a cycle -- which `equal?` is
    // required to survive -- does not hang it.
    let mut vm = Vm::new();
    let v = vm
        .eval_str(
            r#"(define (same? a b) (and (equal? a b) (= (equal-hash a) (equal-hash b))))
               (list (same? (list 1 2 (vector 3 "s")) (list 1 2 (vector 3 "s")))
                     (same? #u8(1 2) #u8(1 2))
                     (same? #f64(1 2) #f64(1 2))
                     (same? (string-copy "s") (string-copy "s"))
                     ;; 1 and 1.0 are not `equal?`, so they need not hash alike.
                     (equal? 1 1.0)
                     ;; -0.0 and 0.0 are `equal?` here, so they must hash alike.
                     (= (equal-hash -0.0) (equal-hash 0.0))
                     ;; A cycle returns a number rather than hanging.
                     (let ((a (list 1 2)) (b (list 1 2)))
                       (set-cdr! (cdr a) a)
                       (set-cdr! (cdr b) b)
                       (and (equal? a b) (= (equal-hash a) (equal-hash b)))))"#,
        )
        .unwrap();
    assert_eq!(write_str(&vm.engine, v), "(#t #t #t #t #f #t #t)");
}

#[test]
fn a_numeric_vector_is_homogeneous_at_scale() {
    // The memory claim -- eight bytes a sample rather than sixteen -- is
    // not an object count, because a flonum is an immediate `Value` and a
    // boxed vector of them is one object too. What *is* observable is
    // that the storage is homogeneous: the element type is enforced on
    // every write, at any size.
    let mut vm = Vm::new();
    vm.eval_str("(import (srfi 4)) (define big (make-f64vector 1000000 1.5))").unwrap();
    let v = vm
        .eval_str(
            "(f64vector-set! big 999999 2.5)
             (list (f64vector-length big) (f64vector-ref big 999999) (f64vector-ref big 0)
                   (guard (e (#t 'refused)) (f64vector-set! big 0 'not-a-number)))",
        )
        .unwrap();
    assert_eq!(write_str(&vm.engine, v), "(1000000 2.5 1.5 refused)");
}

#[test]
fn an_array_view_shares_its_store() {
    // `make-shared-array` must not copy. Identity of the store is the
    // proof -- an allocation count is not, since a copied `f64vector`
    // store would also be a single object.
    let mut vm = Vm::new();
    vm.eval_str("(import (srfi 63)) (define img (make-array a:flor64b 512 512))").unwrap();
    vm.eval_str("(define t (make-shared-array img (lambda (i j) (list j i)) 512 512))").unwrap();
    let v = vm
        .eval_str(
            "(array-set! t 2.5 1 0)
             (list (eqv? (%array-store img) (%array-store t))
                   (array-ref img 0 1) (array-ref t 1 0)
                   (%array-strides img) (%array-strides t))",
        )
        .unwrap();
    assert_eq!(write_str(&vm.engine, v), "(#t 2.5 2.5 (512 1) (1 512))");
}

#[test]
fn every_embedded_library_loads_and_is_found_without_a_path() {
    // No library path, no files beside the binary: the embedded copy is
    // the one found, which is what a `scm build` bundle relies on.
    for name in scm::vm::library::embedded_names() {
        let mut vm = Vm::new();
        vm.import(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(vm.loaded_libraries().contains(&name.to_string()), "{name} not registered");
    }
}

#[test]
fn a_library_on_the_path_overrides_the_embedded_copy() {
    let dir = std::env::temp_dir().join(format!("scm-srfi-override-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("srfi")).unwrap();
    std::fs::write(
        dir.join("srfi/8.sld"),
        "(define-library (srfi 8) (export receive) (import (scheme base))
           (begin (define-syntax receive (syntax-rules () ((_ f e b ...) 'overridden)))))",
    )
    .unwrap();
    let mut vm = Vm::new();
    vm.add_library_path(&dir);
    let v = vm.eval_str("(import (srfi 8)) (receive (a) (values 1) a)").unwrap();
    assert_eq!(write_str(&vm.engine, v), "overridden");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn records_are_disjoint_from_every_other_type() {
    // R7RS §5.5. The box of (srfi 111) is the case that made this matter:
    // its whole content is its identity, so a record that compared
    // structurally would make two distinct boxes `equal?`.
    let mut vm = Vm::new();
    let v = vm
        .eval_str(
            "(import (srfi 111))
             (define b (box 1))
             (list (box? b) (vector? b) (pair? b) (procedure? b) (string? b)
                   (equal? b (box 1)) (equal? b b) (eqv? b b)
                   (promise? (delay 1)) (vector? (delay 1)) (force (delay (* 6 7))))",
        )
        .unwrap();
    assert_eq!(
        write_str(&vm.engine, v),
        "(#t #f #f #f #f #f #t #t #t #f 42)"
    );
}
