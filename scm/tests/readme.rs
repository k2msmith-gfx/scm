//! Every Rust snippet in the repository README, compiled and run.
//!
//! The README is the front page and its code is the first thing a reader
//! tries, so it is a test rather than prose. The prose claims next to the
//! code are checked too -- the features list, `(/ 8 3)` staying inexact,
//! an exact integer past the word -- because those are the sentences most
//! likely to go stale without anything noticing.

use scm::{Interp, Val};

#[test]
fn every_rust_snippet_in_the_readme_holds() -> Result<(), scm::EmbedError> {
    // ── the opening example ──────────────────────────────────────────
    let mut scm = Interp::new()?;
    scm.define_fn("host-add", |args| match args {
        [Val::Int(a), Val::Int(b)] => Ok(Val::Int(a + b)),
        _ => Err("host-add: wants two exact integers".into()),
    })?;
    let r = scm.eval("(host-add 20 22)")?;
    assert_eq!(scm.value(&r)?, Val::Int(42));

    // ── evaluating, and exchanging values ────────────────────────────
    let r = scm.eval("(map (lambda (x) (* x x)) '(1 2 3))")?;
    assert_eq!(
        scm.value(&r)?,
        Val::List(vec![Val::Int(1), Val::Int(4), Val::Int(9)])
    );
    assert_eq!(scm.write(&r)?, "(1 4 9)");

    let v = scm.make(&Val::Vector(vec![
        Val::Str("a".into()),
        Val::Keyword("at".into()),
    ]))?;
    assert_eq!(scm.write(&v)?, "#(\"a\" :at)");

    // ── host objects, and arguments read in place ────────────────────
    {
        use scm::HostObject;

        struct Buffer { size: usize }

        let mut scm = Interp::new()?;
        scm.define_fn_args("make-buffer", |a| {
            Ok(Val::Host(HostObject::new("buffer", Buffer { size: a.int(0)? as usize })))
        })?;
        scm.define_fn_args("buffer-write!", |a| {
            let buffer = a.host_as::<Buffer>(0, "buffer")?;   // the Rc the object shares
            let floats = a.f32s(1)?;                           // borrowed, for the call
            Ok(Val::Bool(floats.len() * 4 <= buffer.size))
        })?;
        let r = scm.eval("(import (srfi 4)) (define b (make-buffer 64)) (list b (buffer-write! b (make-f32vector 16 0.0)))")?;
        assert_eq!(scm.write(&r)?, "(#<buffer> #t)");
    }

    // ── calling in both directions ───────────────────────────────────
    scm.define_fn("clamp", |args| match args {
        [Val::Float(x)] => Ok(Val::Float(x.clamp(0.0, 1.0))),
        _ => Err("clamp: wants one real".into()),
    })?;
    let r = scm.eval("(clamp 2.5)")?;
    assert_eq!(scm.value(&r)?, Val::Float(1.0));

    scm.define_fn_with("twice-apply", |host, args| match args {
        [Val::Proc(f), Val::Int(n)] => {
            let once = host.call(f, &[Val::Int(*n)])?;
            host.call(f, &[once])
        }
        _ => Err("twice-apply: wants a procedure and an integer".into()),
    })?;
    let r = scm.eval("(twice-apply (lambda (x) (* x 3)) 2)")?;
    assert_eq!(scm.value(&r)?, Val::Int(18));

    let r = scm.eval("(lambda (x) (+ x 1))")?;
    if let Val::Proc(f) = scm.value(&r)? {
        assert_eq!(scm.call(&f, &[Val::Int(41)])?, Val::Int(42));
    } else {
        panic!("expected a procedure");
    }

    // ── the compatibility claims the README makes in prose ───────────
    let r = scm.eval("(* 9223372036854775807 2)")?;
    assert_eq!(scm.value(&r)?, Val::BigInt("18446744073709551614".into()));
    let r = scm.eval("(/ 8 3)")?;
    assert_eq!(scm.value(&r)?, Val::Float(2.6666666666666665));
    let r = scm.eval("(features)")?;
    let features = scm.write(&r)?;
    for claimed in ["r7rs", "ieee-float", "exact-closed", "scm"] {
        assert!(features.contains(claimed), "{claimed} missing: {features}");
    }
    for absent in ["ratios", "exact-complex", "full-unicode"] {
        assert!(!features.contains(absent), "{absent} claimed: {features}");
    }

    // ── the library example ──────────────────────────────────────────
    scm.eval(
        "(define-library (acme util)
           (export twice)
           (import (scheme base))
           (begin (define (twice x) (* 2 x))))",
    )?;
    scm.eval("(import (prefix (acme util) u:))")?;
    let r = scm.eval("(u:twice 21)")?;
    assert_eq!(scm.value(&r)?, Val::Int(42));

    Ok(())
}
