//! The SRFI libraries, gated.
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
