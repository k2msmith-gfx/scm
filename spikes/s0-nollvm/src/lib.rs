//! Stub standing in for the `scm` crate: the LLVM backend must be optional.
pub fn eval(x: i64) -> i64 { x * 3 + 1 }

#[cfg(feature = "jit")]
pub mod jit {
    pub fn available() -> bool { true }
}

#[cfg(not(feature = "jit"))]
pub mod jit {
    pub fn available() -> bool { false }
}
