//! An embeddable R7RS-small Scheme, with no dependencies.
//!
//! `scm` is a Scheme implementation built to be *hosted*: a Rust program
//! creates an [`Interp`], evaluates source, and exchanges values with it
//! through a small typed API that cannot hand out a dangling reference.
//! The crate has **no dependencies**, so a host takes this and nothing
//! else, and the default feature set is empty, so `cargo build` needs no
//! LLVM toolchain.
//!
//! # Embedding
//!
//! ```
//! use scm::{Interp, Val};
//!
//! let mut scm = Interp::new()?;
//!
//! // Evaluate source. The result is a `Rooted` handle: it keeps the value
//! // alive and un-moved across later collections, and releases on drop.
//! let r = scm.eval("(let loop ((i 0) (acc 1)) (if (= i 5) acc (loop (+ i 1) (* acc 2))))")?;
//! assert_eq!(scm.value(&r)?, Val::Int(32));
//! assert_eq!(scm.write(&r)?, "32");
//!
//! // Expose a Rust function to Scheme. `Err` becomes an ordinary Scheme
//! // error that `guard` can catch; a panic inside is caught too.
//! scm.define_fn("host-add", |args| match args {
//!     [Val::Int(a), Val::Int(b)] => Ok(Val::Int(a + b)),
//!     _ => Err("host-add: wants two exact integers".into()),
//! })?;
//! let r = scm.eval("(host-add 20 22)")?;
//! assert_eq!(scm.value(&r)?, Val::Int(42));
//!
//! // Exact integers are unbounded, so one too wide for an `i64` crosses
//! // the boundary as text rather than being truncated.
//! let r = scm.eval("(expt 2 100)")?;
//! assert_eq!(scm.value(&r)?, Val::BigInt("1267650600228229401496703205376".into()));
//! # Ok::<(), scm::EmbedError>(())
//! ```
//!
//! # A runaway script cannot take the host down
//!
//! This is a requirement rather than a feature, and it is why the entry
//! points all return [`Result`]:
//!
//! ```
//! use scm::{Interp, embed::Error};
//!
//! let mut scm = Interp::new()?;
//! scm.set_fuel(Some(10_000));
//! // An infinite loop runs out of fuel and reports, rather than hanging.
//! assert!(matches!(scm.eval("(let spin () (spin))"), Err(Error::OutOfFuel)));
//!
//! let mut scm = Interp::new()?;
//! scm.set_heap_cap(Some(1 << 16));
//! // And a script whose *live* data will not fit hits the heap cap. The
//! // cap is only reported once a collection has been tried, so a program
//! // that fits after its garbage goes keeps running.
//! let runaway = "(let loop ((i 0) (acc '())) (if (= i 1000000) acc (loop (+ i 1) (cons i acc))))";
//! assert!(matches!(scm.eval(runaway), Err(Error::OutOfMemory)));
//! # Ok::<(), scm::EmbedError>(())
//! ```
//!
//! [`Interp::set_fuel`] bounds work, [`Interp::set_heap_cap`] bounds
//! memory, [`Interp::interrupt_handle`] stops a script from another thread,
//! and [`Interp::eval_with_timeout`] bounds wall-clock time. A caught panic
//! **poisons** the interpreter rather than pretending the VM is still
//! intelligible, which [`Interp::is_poisoned`] reports.
//!
//! # What is in the box
//!
//! | | |
//! |---|---|
//! | [`embed`] | the host API: [`Interp`], [`Rooted`], [`Val`], [`Host`] |
//! | [`vm`] | the interpreter and the bytecode VM, over one explicit frame stack |
//! | [`heap`] | the precise, non-moving mark-sweep collector |
//! | [`reader`] / [`writer`] | datum syntax, and the round trip between them |
//! | [`syntax`] | `syntax-rules` |
//! | [`prim`] | the primitives |
//! | [`bigint`] | arbitrary-precision exact integers |
//! | [`repl`] | the editor protocol the terminal, socket and Emacs clients share |
//!
//! Three engines run every program and must agree on all of it: an AST
//! interpreter kept as the semantic oracle, a bytecode VM as the default,
//! and — behind the `jit` feature of the `scm-cli` crate — an LLVM tier.
//! A differential fuzzer compares them on random programs.
//!
//! Beyond the embedding API the modules are `pub` because the CLI, the
//! conformance runner and the LLVM crate are all built on them; [`embed`]
//! is the surface with the stability obligation.
//!
//! See the repository README for the language's compatibility notes and the
//! 36 libraries built into the binary, and `docs/plan.md` for why any of
//! it is shaped the way it is.

pub mod bigint;
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

pub use embed::{Description, Error as EmbedError, Host, Interp, Proc, Rooted, Val};
pub use repl::{Framing, Op, Outcome, Request, Response, Session};
pub use heap::{Engine, Interner, Obj, OutputCallback};
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
