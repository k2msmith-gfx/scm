//! **What the editor knows about the language it edits** (docs/editor-crate-
//! plan.md, E1b): how its text lexes into highlight classes, where its
//! brackets are, how a new line indents, and the call around the cursor.
//! The widget asks a buffer's [`Language`] and nothing else, so a second
//! language (E6) is a second implementation, not an edit to the widget.
//! Scheme ([`scheme::Scheme`]) is the one there is.
//!
//! The highlight COLOURS are not here: each [`Tok`] is a theme slot
//! (code-comment/string/keyword/number/call/paren), so the look stays the
//! host's policy.

pub mod scheme;

pub use scheme::SCHEME;

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
#[derive(Default)]
pub struct Scan {
    pub spans: Vec<Spans>,
    pub parens: Vec<ParenPos>,
}

pub trait Language: Sync {
    /// Scan the buffer, one entry per line.
    fn scan(&self, lines: &[&str]) -> Scan;
    /// The column a NEW line at (`line`, `byte`) indents to.
    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize;
    /// The name of the call the point at (`line`, `byte`) is inside — what
    /// eldoc describes. `None` when there is none.
    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String>;
}

/// A code-context delimiter's position, buffer-wide.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
