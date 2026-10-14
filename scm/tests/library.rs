//! `define-library` and `import`: a library is a frame, importing copies.

use scm::{Tier, Value, Vm, write_str};

fn run(tier: Tier, src: &str) -> Result<String, String> {
    let mut vm = Vm::with_tier(tier);
    vm.eval_str(src).map(|v| write_str(&vm.engine, v)).map_err(|e| format!("{e}"))
}

/// Both tiers must say the same thing.
fn both(src: &str) -> Result<String, String> {
    let a = run(Tier::T0, src);
    let b = run(Tier::T1, src);
    assert_eq!(a, b, "T0 and T1 disagree on:\n{src}");
    a
}

const LIB: &str = r#"
(define-library (acme geometry)
  (export area (rename scale* scale) pi* square-it)
  (import (scheme base))
  (begin
    (define pi* 3.0)
    (define (helper x) (* x x))
    (define (area r) (* pi* (helper r)))
    (define (scale* v k) (* v k))
    ;; a macro that expands to a reference to a *private* helper
    (define-syntax square-it (syntax-rules () ((_ e) (helper e))))))
"#;

#[test]
fn a_library_exports_what_it_says_and_hides_the_rest() {
    let src = format!("{LIB}(import (acme geometry)) (list (area 2) (scale 3 4) pi* (square-it 5))");
    assert_eq!(both(&src).unwrap(), "(12.0 12 3.0 25)");
    // The private helper and the internal name of the renamed export are
    // not visible.
    let src = format!("{LIB}(import (acme geometry)) helper");
    assert!(both(&src).unwrap_err().contains("unbound variable: helper"));
    let src = format!("{LIB}(import (acme geometry)) scale*");
    assert!(both(&src).unwrap_err().contains("unbound variable: scale*"));
    // And the library's definitions did not land in the globals at all.
    let src = format!("{LIB}area");
    assert!(both(&src).unwrap_err().contains("unbound"));
}

#[test]
fn import_sets_only_except_prefix_and_rename() {
    let src = format!("{LIB}(import (only (acme geometry) area)) (area 1)");
    assert_eq!(both(&src).unwrap(), "3.0");
    let src = format!("{LIB}(import (only (acme geometry) area)) pi*");
    assert!(both(&src).unwrap_err().contains("unbound"));
    let src = format!("{LIB}(import (except (acme geometry) pi*)) (list (area 1) (scale 2 2))");
    assert_eq!(both(&src).unwrap(), "(3.0 4)");
    let src = format!("{LIB}(import (prefix (acme geometry) geo:)) (geo:area 1)");
    assert_eq!(both(&src).unwrap(), "3.0");
    let src = format!("{LIB}(import (rename (acme geometry) (area circle-area))) (circle-area 1)");
    assert_eq!(both(&src).unwrap(), "3.0");
    let src = format!("{LIB}(import (prefix (rename (only (acme geometry) area) (area a)) g/)) (g/a 1)");
    assert_eq!(both(&src).unwrap(), "3.0");
    let src = format!("{LIB}(import (only (acme geometry) nope))");
    assert!(both(&src).unwrap_err().contains("`nope` is not exported"));
}

#[test]
fn a_macro_export_reaching_a_private_helper_compiles() {
    // `square-it` expands to `(helper e)`, and `helper` lives in the
    // library's frame: the compiler reads it by address rather than
    // declining, so a hot loop over it runs on T1.
    let src = format!("{LIB}(import (acme geometry)) (define (f n) (let loop ((i 0) (s 0)) (if (= i n) s (loop (+ i 1) (+ s (square-it i)))))) (list (f 100) (car (cdr (cdr (%tier-stats)))))");
    let mut vm = Vm::with_tier(Tier::T1);
    let v = vm.eval_str(&src).unwrap();
    let shown = write_str(&vm.engine, v);
    assert!(shown.starts_with("(328350 "), "{shown}");
    assert_eq!(shown, "(328350 0)", "no unit was declined");
}

#[test]
fn libraries_import_each_other_and_cycles_are_reported() {
    let src = format!(
        "{LIB}
(define-library (acme shapes)
  (export disk)
  (import (scheme base) (prefix (acme geometry) g:))
  (begin (define (disk r) (list 'disk (g:area r)))))
(import (acme shapes)) (disk 1)"
    );
    assert_eq!(both(&src).unwrap(), "(disk 3.0)");
    let src = "(define-library (a) (export x) (import (b)) (begin (define x 1)))
               (define-library (b) (export y) (import (a)) (begin (define y 2)))";
    // (b) is not loaded when (a) asks for it, so it is looked for on disk.
    assert!(both(src).unwrap_err().contains("library not found: (b)"));
    assert!(both("(import (no such library))").unwrap_err().contains("library not found: (no such library)"));
}

#[test]
fn a_library_file_with_include_and_cond_expand() {
    let dir = std::env::temp_dir().join(format!("scm-lib-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("acme")).unwrap();
    std::fs::write(dir.join("acme/util.sld"), r#"
(define-library (acme util)
  (export twice tag)
  (import (scheme base))
  (include "util-body.scm")
  (cond-expand
    ((library (scheme base)) (begin (define tag 'has-base)))
    (else (begin (define tag 'no-base)))))
"#).unwrap();
    std::fs::write(dir.join("acme/util-body.scm"), "(define (twice x) (* 2 x))\n").unwrap();
    for tier in [Tier::T0, Tier::T1] {
        let mut vm = Vm::with_tier(tier);
        vm.add_library_path(&dir);
        let v = vm.eval_str("(import (acme util)) (list (twice 21) tag)").unwrap();
        assert_eq!(write_str(&vm.engine, v), "(42 has-base)");
        assert_eq!(vm.loaded_libraries(), vec!["(acme util)".to_string()]);
        // From the host, by name.
        let mut vm2 = Vm::with_tier(tier);
        vm2.add_library_path(&dir);
        vm2.import("(prefix (acme util) u:)").unwrap();
        assert_eq!(vm2.eval_str("(u:twice 4)").unwrap(), Value::Fixnum(8));
    }
    // A self-import through the file is a cycle, not a hang.
    std::fs::write(dir.join("acme/loop.sld"), "(define-library (acme loop) (export z) (import (acme loop)) (begin (define z 1)))").unwrap();
    let mut vm = Vm::new();
    vm.add_library_path(&dir);
    let e = format!("{}", vm.eval_str("(import (acme loop))").unwrap_err());
    assert!(e.contains("imports itself"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cond_expand_at_top_level_and_standard_imports_are_no_ops() {
    assert_eq!(both("(import (scheme base) (scheme write)) (cond-expand (scm 'ours) (else 'theirs))").unwrap(), "ours");
    assert_eq!(both("(cond-expand ((and r7rs (not nonsense)) 1) (else 2))").unwrap(), "1");
    assert_eq!(both("(cond-expand ((library (scheme base)) 'yes) (else 'no))").unwrap(), "yes");
    assert_eq!(both("(cond-expand ((library (no such)) 'yes) (else 'no))").unwrap(), "no");
}

#[test]
fn a_define_syntax_inside_a_body_is_scoped_there() {
    // Previously an internal define-syntax leaked into the global table.
    let src = "(define (f x) (define-syntax dbl (syntax-rules () ((_ e) (* 2 e)))) (dbl x)) (list (f 4) (guard (e (#t 'unbound)) (dbl 1)))";
    assert_eq!(both(src).unwrap(), "(8 unbound)");
}

#[test]
fn library_frames_survive_collection() {
    let src = format!("{LIB}(import (acme geometry)) (let loop ((i 0) (acc '())) (if (= i 20000) (area 1) (loop (+ i 1) (cons (make-vector 2 i) acc))))");
    let mut vm = Vm::new();
    vm.engine.set_gc_stress(true);
    let v = vm.eval_str(&src).unwrap();
    assert_eq!(write_str(&vm.engine, v), "3.0");
}
