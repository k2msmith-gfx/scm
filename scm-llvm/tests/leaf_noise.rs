//! `(scm noise)` exists to be compiled: ray's corpus bakes noise into
//! small tables because evaluating it per texel was too slow, and that
//! workaround is only unnecessary if every procedure in the library really
//! does become native code. So this asserts it for **every exported
//! procedure**, against the shipped library, and will fail the moment one
//! of them stops qualifying.

use scm::Vm;
use scm_llvm::{Shape, compile};

/// Every exported procedure, with the shape it returns.
const EXPORTS: &[&str] = &[
    "noise-fract",
    "noise-fade",
    "noise-lerp",
    "noise-smoothstep",
    "noise-hash1",
    "noise-hash2",
    "noise-hash3",
    "value-noise-2",
    "value-noise-3",
    "value-noise-2-tiled",
    "perlin-grad-2",
    "perlin-grad-3",
    "perlin-2",
    "perlin-3",
    "simplex-2",
    "value-noise-2-fbm",
    "value-noise-3-fbm",
    "perlin-2-fbm",
    "perlin-3-fbm",
    "perlin-2-turbulence",
    "perlin-3-turbulence",
    "perlin-2-ridged",
    "worley-2",
    "worley-2-f2",
    "worley-3",
];

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.eval_str("(import (scm noise))").expect("import (scm noise)");
    vm
}

#[test]
fn every_exported_noise_procedure_compiles() {
    let mut vm = vm();
    let mut refused = Vec::new();
    for name in EXPORTS {
        if let Err(r) = compile(&mut vm, name, Shape::Scalar) {
            refused.push(format!("{name}: {}", r.message));
        }
    }
    assert!(refused.is_empty(), "these did not compile:\n  {}", refused.join("\n  "));
}

/// Compiled noise must give the same field as the interpreter, or the
/// texture changes the moment it gets fast.
#[test]
fn compiled_noise_agrees_with_the_interpreter() {
    let mut vm = vm();
    for (name, args) in [
        ("value-noise-2", vec![0.3, 0.7]),
        ("perlin-2", vec![0.3, 0.7]),
        ("perlin-3", vec![0.3, 0.7, 0.1]),
        ("simplex-2", vec![0.3, 0.7]),
        ("worley-2", vec![0.3, 0.7]),
        ("worley-2-f2", vec![0.3, 0.7]),
        ("worley-3", vec![0.3, 0.7, 0.1]),
    ] {
        let f = compile(&mut vm, name, Shape::Scalar).expect(name);
        // sweep a grid rather than one point: a single sample can agree by
        // luck, and the lattice boundaries are where a disagreement hides
        for i in 0..40 {
            let mut a = args.clone();
            a[0] = -2.0 + (i as f64) * 0.137;
            a[1] = 1.0 + (i as f64) * 0.211;
            let native = f.call(&a).unwrap();
            let call = format!(
                "({} {})",
                name,
                a.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(" ")
            );
            let interp = match vm.eval_str(&call).unwrap() {
                scm::Value::Flonum(x) => x,
                v => panic!("{call} gave {v:?}"),
            };
            assert!(
                (native - interp).abs() < 1e-12,
                "{call}: native {native} vs interpreted {interp}"
            );
        }
    }
}

/// An fbm sum over octaves is where a loop could be miscompiled, so it is
/// swept separately with the octave count varying too.
#[test]
fn compiled_fbm_agrees_with_the_interpreter() {
    let mut vm = vm();
    let f = compile(&mut vm, "perlin-2-fbm", Shape::Scalar).expect("perlin-2-fbm");
    for octaves in 0..7 {
        for i in 0..10 {
            let x = 0.11 * (i as f64);
            let y = 3.7 - 0.23 * (i as f64);
            let native = f.call(&[x, y, octaves as f64]).unwrap();
            let interp = match vm
                .eval_str(&format!("(perlin-2-fbm {x:?} {y:?} {octaves})"))
                .unwrap()
            {
                scm::Value::Flonum(v) => v,
                v => panic!("{v:?}"),
            };
            assert!(
                (native - interp).abs() < 1e-12,
                "octaves {octaves} at ({x}, {y}): {native} vs {interp}"
            );
        }
    }
}
