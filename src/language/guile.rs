//! **GNU Guile** (docs/editor-crate-plan.md, E6), the editor's second
//! language — and the test of the `Language` boundary that a second
//! language was always going to be. Guile is Scheme, so the scan, the
//! indentation and the call around the point are Scheme's; what differs is
//! what the editor may assume about the TEXT:
//!
//! - `#:keyword` is Guile's keyword syntax, coloured as a keyword (scm's
//!   reads as a literal).
//! - **No read-error marks.** Scheme's are scm's reader's answer, and scm
//!   rejects Guile syntax (`#:key`, `#{sym}#`) that Guile reads fine; marks
//!   from the wrong reader would be noise on every keyword argument. The
//!   right reader is Guile's own, in the Guile process — a later step.
//!
//! Where a Guile buffer's sends go is the shell's (`15-guile.scm`): a Guile
//! the editor starts, over scm's REPL protocol.

use super::{Diagnostic, Language, Scan, Tok};

pub struct Guile;

pub static GUILE: Guile = Guile;

impl Language for Guile {
    fn name(&self) -> &'static str {
        "guile"
    }

    fn scan(&self, lines: &[&str]) -> Scan {
        let mut scan = super::SCHEME.scan(lines);
        for (line, spans) in lines.iter().zip(scan.spans.iter_mut()) {
            for span in spans.iter_mut() {
                if line.get(span.0..span.1).is_some_and(|w| w.starts_with("#:")) {
                    span.2 = Tok::Keyword;
                }
            }
        }
        scan
    }

    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize {
        super::SCHEME.indent(lines, line, byte)
    }

    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String> {
        super::SCHEME.enclosing_head(lines, line, byte)
    }

    fn diagnose(&self, _lines: &[&str], _scan: &Scan) -> Vec<Diagnostic> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guile_keywords_colour_as_keywords_and_nothing_is_marked() {
        let lines = ["(define* (f #:key (n 1)) n)", "(f #:n 2)"];
        let scan = GUILE.scan(&lines);
        let kw: Vec<&str> = scan
            .spans
            .iter()
            .zip(lines)
            .flat_map(|(spans, l)| spans.iter().filter(|s| s.2 == Tok::Keyword).map(move |s| &l[s.0..s.1]))
            .collect();
        assert_eq!(kw, vec!["#:key", "#:n"]);
        assert!(GUILE.diagnose(&lines, &scan).is_empty());
        assert_eq!(GUILE.enclosing_head(&lines, 1, 4).as_deref(), Some("f"));
    }
}
