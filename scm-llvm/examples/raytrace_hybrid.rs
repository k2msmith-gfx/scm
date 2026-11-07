//! examples/raytracer, with its ray/shape tests leaf-compiled.
//!
//! Loads `raytracer.scm` (without its closing `(main)`), then `leaves.scm`,
//! which flattens the scene into a table of numbers and redefines
//! `scene-intersect` and `scene-occluded?` to call two host functions. Those
//! are the leaves `nearest-shape` and `occluded`, compiled here. Shading,
//! recursion, glass and output stay interpreted, so this measures the hybrid
//! ray's own texels use: Scheme drives, the hot pure arithmetic is native.
//!
//!     cargo run --release -p scm-llvm --example raytrace_hybrid -- \
//!         [--size WxH] [--out FILE]
use std::time::Instant;

use scm::{Interp, Val};
use scm_llvm::{Shape, compile};

const TRACER: &str = include_str!("../../examples/raytracer/raytracer.scm");
const LEAVES: &str = include_str!("../../examples/raytracer/leaves.scm");

fn main() {
    let body = TRACER.trim_end().strip_suffix("(main)").expect("raytracer.scm ends with (main)");
    let mut interp = Interp::new().expect("an interpreter");
    // Stand-ins so leaves.scm loads; replaced below once the leaves compile.
    for name in ["native-nearest", "native-occluded"] {
        interp.define_fn(name, |_| Err("not compiled yet".into())).unwrap();
    }
    interp.eval(body).expect("raytracer.scm loads");
    interp.eval(LEAVES).expect("leaves.scm loads");

    let t = Instant::now();
    let nearest = compile(interp.vm_mut(), "nearest-shape", Shape::Scalar).unwrap_or_else(|r| panic!("{r}"));
    let occluded = compile(interp.vm_mut(), "occluded", Shape::Scalar).unwrap_or_else(|r| panic!("{r}"));
    eprintln!("compiled the two leaves in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);

    fn six(args: &[Val]) -> Result<[f64; 6], String> {
        let mut out = [0.0; 6];
        if args.len() != 6 {
            return Err(format!("wants 6 numbers, got {}", args.len()));
        }
        for (o, a) in out.iter_mut().zip(args) {
            *o = match a {
                Val::Float(f) => *f,
                Val::Int(i) => *i as f64,
                other => return Err(format!("not a number: {other:?}")),
            };
        }
        Ok(out)
    }
    interp
        .define_fn("native-nearest", move |a| Ok(Val::Float(nearest.call(&six(a)?).unwrap())))
        .unwrap();
    interp
        .define_fn("native-occluded", move |a| Ok(Val::Float(occluded.call(&six(a)?).unwrap())))
        .unwrap();

    interp.eval("(main)").expect("the render runs");
}
