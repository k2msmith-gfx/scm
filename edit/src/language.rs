//! **What the editor knows about the language it edits** (docs/editor-crate-
//! plan.md, E1b): how its text lexes into highlight classes, where its
//! brackets are, how a new line indents, and the call around the cursor.
//! The widget asks a buffer's [`Language`] and nothing else, so a second
//! language (E6) is a second implementation, not an edit to the widget.
//! There are four: scm's Scheme ([`scheme::Scheme`]), GNU Guile
//! ([`guile::Guile`]), Common Lisp ([`lisp::Lisp`]) and Rust
//! ([`rust::Rust`]), each buffer set to one by name ([`by_name`]).
//!
//! The highlight COLOURS are not here: each [`Tok`] is a theme slot
//! (code-comment/string/keyword/number/call/paren), so the look stays the
//! host's policy.

pub mod guile;
pub mod lisp;
pub mod rust;
pub mod scheme;

pub use guile::GUILE;
pub use lisp::LISP;
pub use rust::RUST;
pub use scheme::SCHEME;

/// A language by the name a buffer is set to (`%editor-set-language`).
pub fn by_name(name: &str) -> Option<&'static dyn Language> {
    match name {
        "scheme" => Some(&SCHEME),
        "guile" => Some(&GUILE),
        "lisp" => Some(&LISP),
        "rust" => Some(&RUST),
        _ => None,
    }
}

/// A highlight class. A language maps its own tokens onto these; text in no
/// class draws in the body ink.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tok {
    Comment,
    Str,
    /// A `:keyword`.
    Keyword,
    Number,
    /// The head symbol of a form — `(add-sphere …)`'s add-sphere.
    Call,
    Paren,
}

/// One line's highlighted stretches: (start byte, end byte, class).
pub type Spans = Vec<(usize, usize, Tok)>;

/// A whole buffer's scan: each line's spans, and every code-context bracket
/// in buffer order (none from inside a string or comment).
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Scan {
    pub spans: Vec<Spans>,
    pub parens: Vec<ParenPos>,
    /// Per line: does it start CLEAN — outside every string, comment and
    /// bracket, with no quote or `#;` waiting for its datum? Text after a
    /// clean line start scans (and reads) the same whatever came before
    /// it, which is what lets an edit re-scan only its own stretch. Empty
    /// for a language that does not say.
    pub clean: Vec<bool>,
}

/// A scan made by re-scanning an edited stretch: the new scan, and the
/// lines it re-made — `start..new_end` in the new text, which replaced
/// `start..old_end` of the old; the lines after were shifted.
pub struct Rescan {
    pub scan: Scan,
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

pub trait Language: Sync {
    /// The language's name, as a buffer is set to it and the shell asks
    /// which backend a buffer's sends go to: "scheme" (scm's), "guile".
    fn name(&self) -> &'static str;
    /// The language as the modeline names it: "Scheme", "Rust".
    fn title(&self) -> &'static str;
    /// Scan the buffer, one entry per line.
    fn scan(&self, lines: &[&str]) -> Scan;
    /// The scan of `lines` from `old`, the scan of `old_lines` (the text
    /// before an edit), re-scanning only what the edit could have changed —
    /// or None, to scan the whole buffer (the default).
    fn rescan(&self, _old: &Scan, _old_lines: &[String], _lines: &[&str]) -> Option<Rescan> {
        None
    }
    /// The column a NEW line at (`line`, `byte`) indents to.
    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize;
    /// The name of the call the point at (`line`, `byte`) is inside — what
    /// eldoc describes. `None` when there is none.
    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String>;
    /// What is wrong with the text as the language READS it (E3f), given
    /// the scan just made of it. None by default: a language with no reader
    /// to ask marks nothing, which is not the same as saying it is fine.
    fn diagnose(&self, _lines: &[&str], _scan: &Scan) -> Vec<Diagnostic> {
        Vec::new()
    }
}

/// A stretch of text something is wrong with, from `start` to `end`, each
/// a (line, byte) — the widget underlines it and the shell can say why.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Diagnostic {
    pub start: (usize, usize),
    pub end: (usize, usize),
    pub message: String,
    pub severity: Severity,
}

/// How bad: what the underline's ink says.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Severity {
    /// Text that will not read, or a compiler's error: the error ink.
    Error,
    /// It reads, but something is likely wrong — a name nothing binds, a
    /// call with the wrong count (E8), a server's warning: the warning ink.
    Warning,
    /// A form still open, which is what text being typed usually is: the
    /// error ink, faint.
    Open,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// A code-context delimiter's position, buffer-wide.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ParenPos {
    pub line: usize,
    pub byte: usize,
    pub ch: u8,
}

/// The partner of `parens[at]`, same-type depth counting (well-formed code).
pub fn find_partner(parens: &[ParenPos], at: usize) -> Option<usize> {
    let p = parens[at];
    let (open, close) = match p.ch {
        b'(' | b')' => (b'(', b')'),
        b'[' | b']' => (b'[', b']'),
        _ => (b'{', b'}'),
    };
    let mut depth = 0i32;
    if p.ch == open {
        for (j, q) in parens.iter().enumerate().skip(at + 1) {
            if q.ch == open {
                depth += 1;
            } else if q.ch == close {
                if depth == 0 {
                    return Some(j);
                }
                depth -= 1;
            }
        }
    } else {
        for j in (0..at).rev() {
            let q = parens[j];
            if q.ch == close {
                depth += 1;
            } else if q.ch == open {
                if depth == 0 {
                    return Some(j);
                }
                depth -= 1;
            }
        }
    }
    None
}
