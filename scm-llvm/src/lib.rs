//! scm-llvm -- the leaf compiler (plan §6, S11).
//!
//! A *leaf* is a pure numeric procedure: a texture, a field, a noise
//! function. This crate compiles one to native code that is
//!
//! - **VM-free** -- no interpreter state is reachable from it;
//! - **re-entrant and `Send + Sync`** -- N render workers call it at once;
//! - **allocation-free** -- every value is an `f64` in a register;
//! - **restricted by construction** -- anything impure is refused at compile
//!   time, with a message naming the call that did it.
//!
//! The subset is "pure", not "primitives-only" (leaf-survey §4): authored
//! helpers compile transitively, loops are named `let`s, a closure may read
//! a precomputed table, and the corpus's `(map (lambda (c) …) (list r g b))`
//! colour return is specialised at the call site. Numbers are `f64`
//! throughout; where T0 keeps an exact integer the compiled code holds the
//! same value inexactly, which the differential test compares numerically.
//!
//! ```no_run
//! # use scm_llvm::{compile, Shape};
//! let mut vm = scm::Vm::new();
//! vm.eval_str("(define (field x y z) (- (sqrt (+ (* x x) (* y y) (* z z))) 1.0))").unwrap();
//! let f = compile(&mut vm, "field", Shape::Scalar).unwrap();
//! assert_eq!(f.call(&[0.0, 0.0, 2.0]), Some(1.0));
//! ```
//!
//! What is *not* here: GC integration, deopt, invalidation, safepoints. A
//! leaf has none of those to integrate with, which is why this is 0.8 of a
//! stage and the general tier (S13) is not.

mod codegen;
mod jit;
mod live;
mod subset;

use std::fmt;
use std::sync::Arc;

use scm::Vm;

pub use live::Live;
pub use subset::{Analysis, Summary};

/// What a compiled leaf returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// One `f64`: a field, a noise function, a helper.
    Scalar,
    /// Three `f64`s in registers where the ABI allows (plan §6 item 5): a
    /// texture's colour.
    Vec3,
}

/// Why a procedure was not compiled. `via` is the call chain that reached
/// the problem, innermost last, so a refusal deep in a helper says how the
/// texture got there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub message: String,
    pub via: Vec<String>,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.via.is_empty() {
            write!(f, " (in {})", self.via.join(" -> "))?;
        }
        Ok(())
    }
}

impl std::error::Error for Refusal {}

/// A 3-float result, `repr(C)` so the register convention is the ABI's own.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A compiled leaf: a native function pointer plus the JIT that owns its
/// code. Cloning shares the code; the code lives until the last clone is
/// dropped, which is what lets a render worker hold one across a pass while
/// the REPL replaces the definition (S11a).
#[derive(Clone)]
pub struct Compiled {
    name: String,
    arity: usize,
    shape: Shape,
    addr: usize,
    _owner: Arc<jit::Jit>,
}

impl fmt::Debug for Compiled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#<leaf {} /{} {:?} @{:#x}>", self.name, self.arity, self.shape, self.addr)
    }
}

impl Compiled {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn arity(&self) -> usize {
        self.arity
    }
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// The raw code address. Its signature is `extern "C"` with `arity` `f64`
    /// arguments; `Scalar` returns `f64`, and `Vec3` follows [`vec3_abi`].
    pub fn address(&self) -> usize {
        self.addr
    }

    /// Call a `Scalar` leaf. `None` if the arity or shape does not match.
    pub fn call(&self, args: &[f64]) -> Option<f64> {
        if self.shape != Shape::Scalar || args.len() != self.arity {
            return None;
        }
        // Safety: the address is a function this crate emitted with exactly
        // this signature, kept alive by `_owner`.
        Some(unsafe {
            match args {
                [] => std::mem::transmute::<usize, extern "C" fn() -> f64>(self.addr)(),
                [a] => std::mem::transmute::<usize, extern "C" fn(f64) -> f64>(self.addr)(*a),
                [a, b] => std::mem::transmute::<usize, extern "C" fn(f64, f64) -> f64>(self.addr)(*a, *b),
                [a, b, c] => std::mem::transmute::<usize, extern "C" fn(f64, f64, f64) -> f64>(self.addr)(*a, *b, *c),
                [a, b, c, d] => std::mem::transmute::<usize, extern "C" fn(f64, f64, f64, f64) -> f64>(self.addr)(*a, *b, *c, *d),
                [a, b, c, d, e] => std::mem::transmute::<usize, extern "C" fn(f64, f64, f64, f64, f64) -> f64>(self.addr)(*a, *b, *c, *d, *e),
                [a, b, c, d, e, g] => std::mem::transmute::<usize, extern "C" fn(f64, f64, f64, f64, f64, f64) -> f64>(self.addr)(*a, *b, *c, *d, *e, *g),
                _ => return None,
            }
        })
    }

    /// Call a `Vec3` leaf. `None` if the arity or shape does not match.
    pub fn call3(&self, args: &[f64]) -> Option<Vec3> {
        if self.shape != Shape::Vec3 || args.len() != self.arity {
            return None;
        }
        Some(unsafe {
            match vec3_abi() {
                Vec3Abi::Registers => match args {
                    [a] => std::mem::transmute::<usize, extern "C" fn(f64) -> Vec3>(self.addr)(*a),
                    [a, b] => std::mem::transmute::<usize, extern "C" fn(f64, f64) -> Vec3>(self.addr)(*a, *b),
                    [a, b, c] => std::mem::transmute::<usize, extern "C" fn(f64, f64, f64) -> Vec3>(self.addr)(*a, *b, *c),
                    [a, b, c, d] => std::mem::transmute::<usize, extern "C" fn(f64, f64, f64, f64) -> Vec3>(self.addr)(*a, *b, *c, *d),
                    _ => return None,
                },
                Vec3Abi::OutPointer => {
                    let mut out = Vec3::default();
                    let o: *mut Vec3 = &mut out;
                    match args {
                        [a] => std::mem::transmute::<usize, extern "C" fn(*mut Vec3, f64)>(self.addr)(o, *a),
                        [a, b] => std::mem::transmute::<usize, extern "C" fn(*mut Vec3, f64, f64)>(self.addr)(o, *a, *b),
                        [a, b, c] => std::mem::transmute::<usize, extern "C" fn(*mut Vec3, f64, f64, f64)>(self.addr)(o, *a, *b, *c),
                        [a, b, c, d] => std::mem::transmute::<usize, extern "C" fn(*mut Vec3, f64, f64, f64, f64)>(self.addr)(o, *a, *b, *c, *d),
                        _ => return None,
                    }
                    out
                }
            }
        })
    }
}

/// How a `Vec3` comes back -- the target hook plan §2.5 asked for from the
/// first commit, because one platform cannot settle it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vec3Abi {
    /// `{double, double, double}` returned by value. On AAPCS64 that is a
    /// homogeneous floating-point aggregate and lands in `d0`–`d2`, matching
    /// a `repr(C)` struct of three `f64` on the Rust side. Verified at S5b.
    Registers,
    /// `void f(Vec3* out, args…)`. SysV x86-64 would return a 24-byte
    /// aggregate through memory anyway, so an explicit out-pointer is the
    /// same cost and has only one spelling. **Not yet verified on x86-64**
    /// (plan §2.5: Linux is sequenced, not dropped).
    OutPointer,
}

pub fn vec3_abi() -> Vec3Abi {
    if cfg!(target_arch = "aarch64") {
        Vec3Abi::Registers
    } else {
        Vec3Abi::OutPointer
    }
}

/// Compile the global procedure `name` and everything it reaches.
///
/// The analysis runs first and decides purity; codegen then decides
/// expressibility. Either refusal names the call and the chain.
pub fn compile(vm: &mut Vm, name: &str, shape: Shape) -> Result<Compiled, Refusal> {
    let summary = Analysis::new(vm).verdict(name)?;
    let (addr, owner) = jit::build(vm, name, shape, &summary)?;
    Ok(Compiled {
        name: name.to_string(),
        arity: summary.arity,
        shape,
        addr,
        _owner: owner,
    })
}

/// The purity verdict alone, with no code emitted: what a REPL asks before
/// promising native speed, and what a host asks to explain a refusal.
pub fn verdict(vm: &mut Vm, name: &str) -> Result<Summary, Refusal> {
    Analysis::new(vm).verdict(name)
}

/// The LLVM IR this crate would emit for `name`, after optimisation. For
/// tests and for reading.
pub fn ir(vm: &mut Vm, name: &str, shape: Shape) -> Result<String, Refusal> {
    let summary = Analysis::new(vm).verdict(name)?;
    jit::ir(vm, name, shape, &summary)
}
