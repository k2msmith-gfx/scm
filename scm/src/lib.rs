//! scm -- an R7RS-small Scheme.
//!
//! S1 is the reader, the printer and the round trip between them; S2 is the
//! interpreter in [`vm`], over an explicit control stack. See
//! `docs/plan.md`; the decisions this code is obliged to implement are §3.1
//! (the subset), §4 (keywords, the lambda list, the conversion hazards) and
//! §4.5 (the value representation).
//!
//! There is deliberately no LLVM here and the default feature set is empty.

pub mod heap;
pub mod prim;
pub mod reader;
pub mod value;
pub mod vm;
pub mod writer;

pub use heap::{Engine, Interner, Obj};
pub use reader::{ReadError, Reader, ReaderOptions, Span};
pub use value::{KwId, ObjRef, SymId, Value};
pub use value::PrimId;
pub use vm::{Vm, VmError};
pub use writer::{display_str, write_str};

/// Read one datum from a string, for tests and for the shortest possible
/// path through the whole pipe.
pub fn read_one(src: &str, e: &mut Engine) -> Result<Option<Value>, ReadError> {
    Reader::new(src).read(e)
}
