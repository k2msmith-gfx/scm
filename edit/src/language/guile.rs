//! **GNU Guile** (docs/editor-crate-plan.md, E6), the editor's second
//! language — and the test of the `Language` boundary that a second
//! language was always going to be. Guile is Scheme, so the scan, the
//! indentation and the call around the point are Scheme's; what differs is
//! what the editor may assume about the TEXT:
//!
//! - `#:keyword` is Guile's keyword syntax, coloured as a keyword (scm's
//!   reads as a literal).
//! - **Its read marks are not scm's.** Scheme's are scm's reader's answer,
//!   and scm rejects Guile syntax (`#:key`, `#{sym}#`) that Guile reads
//!   fine; marks from the wrong reader would be noise on every keyword
//!   argument. So what is marked here is what needs no reader at all — a
//!   bracket that closes nothing, a form left open — and what GUILE cannot
//!   read is asked of Guile itself, in the Guile process, when one is
//!   running (`scm_binding/readcheck.rs`, 15-guile).
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

    fn title(&self) -> &'static str {
        "Guile"
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

    /// The brackets, which are the same in every Scheme: [`bracket_marks`].
    fn diagnose(&self, lines: &[&str], scan: &Scan) -> Vec<Diagnostic> {
        bracket_marks(lines, scan)
    }
}

/// What is wrong with the brackets, which needs no reader: a closer with
/// nothing to close (or closing the wrong kind) is an error where it
/// stands; a form left open at the end is marked quietly at its outermost
/// opener, as scm's Scheme marks one. The scan's brackets are the code's —
/// none from inside a string, a comment or a `#\(`. Guile's and Common
/// Lisp's (whose readers are asked the rest, when one is running).
pub(super) fn bracket_marks(lines: &[&str], scan: &Scan) -> Vec<Diagnostic> {
    let mut open: Vec<u8> = Vec::new();
    for p in &scan.parens {
        let closes = match p.ch {
            b')' => b'(',
            b']' => b'[',
            b'}' => b'{',
            opener => {
                open.push(opener);
                continue;
            }
        };
        let message = match open.pop() {
            Some(opener) if opener == closes => continue,
            Some(opener) => format!("`{}` closes a `{}`", p.ch as char, opener as char),
            None => format!("`{}` closes nothing", p.ch as char),
        };
        return vec![Diagnostic { start: (p.line, p.byte), end: (p.line, p.byte + 1), message, severity: super::Severity::Error }];
    }
    // (A string or a `#|` comment left open is the reader's to say, when
    // one is running.)
    if open.is_empty() { Vec::new() } else { super::scheme::open_end(lines, scan) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guile_keywords_colour_as_keywords_and_are_not_marked() {
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

    #[test]
    fn brackets_are_marked_with_no_reader_to_ask() {
        use crate::language::Severity;
        let marks = |src: &str| {
            let lines: Vec<&str> = src.split('\n').collect();
            GUILE.diagnose(&lines, &GUILE.scan(&lines)).into_iter().map(|d| (d.start, d.severity, d.message)).collect::<Vec<_>>()
        };
        // Guile's own syntax is not an error: keywords, `#{…}#`, a bracket
        // in a string, a comment or a character.
        assert!(marks("(define* (f #:key (n 1)) n)\n(list #{a b}# \"(\" #\\( ; )\n      'ok)").is_empty());
        // A form left open: quietly, at its OUTERMOST opener.
        let open = marks("(ok)\n(define (f x)\n  (g x");
        assert_eq!(open.len(), 1);
        assert_eq!((open[0].0, open[0].1), ((1, 0), Severity::Open), "{open:?}");
        // A closer that closes nothing, or the wrong kind: an error, there.
        let stray = marks("(f 1))\n(g)");
        assert_eq!((stray[0].0, stray[0].1), ((0, 5), Severity::Error), "{stray:?}");
        assert!(stray[0].2.contains("closes nothing"));
        let wrong = marks("(f [1 2)]");
        assert_eq!(wrong[0].0, (0, 7));
        assert!(wrong[0].2.contains("closes a `[`"), "{wrong:?}");
    }
}
