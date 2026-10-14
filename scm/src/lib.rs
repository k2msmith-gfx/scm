//! scm -- an R7RS-small Scheme.
//!
//! S1 is the reader, the printer and the round trip between them; S2 is the
//! interpreter in [`vm`], over an explicit control stack. See
//! `docs/plan.md`; the decisions this code is obliged to implement are §3.1
//! (the subset), §4 (keywords, the lambda list, the conversion hazards) and
//! §4.5 (the value representation).
//!
//! There is deliberately no LLVM here and the default feature set is empty.

pub mod code;
pub mod embed;
pub mod heap;
pub mod prim;
pub mod reader;
pub mod repl;
pub mod syntax;
pub mod value;
pub mod vm;
pub mod writer;

pub use embed::{Description, Error as EmbedError, Interp, Rooted, Val};
pub use repl::{Framing, Op, Outcome, Request, Response, Session};
pub use heap::{Engine, Interner, Obj};
pub use reader::{ReadError, Reader, ReaderOptions, Span};
pub use value::{KwId, ObjRef, SymId, Value};
pub use value::PrimId;
pub use code::Tier;
pub use vm::{BreakAction, BreakHandler, FrameView, PRELUDE, Vm, VmError};
pub use writer::{display_str, write_str};

/// Whether a chunk of source is a complete set of datums, needs more input,
/// or is wrong.
///
/// This is the question a REPL asks before evaluating and an editor asks
/// before deciding whether `RET` submits -- the whole reason S1 made
/// *incomplete* and *invalid* different answers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ParseStatus {
    Complete,
    /// Keep typing.
    Incomplete,
    Invalid(String),
}

pub fn parse_status(src: &str) -> ParseStatus {
    // A scratch engine: this is a question about the text, and answering it
    // should not intern anything into a live heap.
    let mut scratch = Engine::new();
    match Reader::read_all(src, &mut scratch) {
        Ok(_) => ParseStatus::Complete,
        Err(ReadError::Incomplete) => ParseStatus::Incomplete,
        Err(e) => ParseStatus::Invalid(format!("{e}")),
    }
}

/// Read one datum from a string, for tests and for the shortest possible
/// path through the whole pipe.
pub fn read_one(src: &str, e: &mut Engine) -> Result<Option<Value>, ReadError> {
    Reader::new(src).read(e)
}
