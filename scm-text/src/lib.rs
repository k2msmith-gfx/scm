//! Scheme source as an editor sees it.
//!
//! [`text`] scans a buffer into tokens and answers structural questions from
//! them -- the definitions in it, the symbol at a cursor, the call around it --
//! without the reader, because a buffer being edited is unparseable most of
//! the time ([`text`]'s module doc says why that matters). [`index`] keeps
//! every top-level definition across a set of files, so a jump can leave the
//! file it starts in.
//!
//! Split out of `scm-lsp` (2026-11-02) so a second editor can link it: ray's
//! modeler has its own editor pane, inside the session, and wanted the
//! index rather than a language server to talk to. Positions come in both
//! currencies -- characters, which an editor that is not LSP counts, and
//! UTF-16 code units, which LSP counts -- because the index keeps no text to
//! convert one into the other later.

pub mod index;
pub mod text;

pub use index::{Index, Site};
pub use text::{Definition, Doc};
