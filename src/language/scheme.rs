//! **Scheme, as the editor reads it** — a tiny Lisp lexer, per line,
//! with the state a line hands the next (S4).
//!
//! Lexing is mechanism (like glyph rendering), so it lives here; the COLORS
//! are theme slots (code-comment/string/keyword/number/call/paren), so the
//! look stays Scheme policy. It lexes Scheme's spelling: `;` line comments,
//! `#| … |#` block comments (nesting), `#;` datum comments, `#t`/`#f`/`#\c`
//! literals and `#(` vectors. Until 2026-11-02 it lexed Janet's — `#` as a
//! line comment — so `(set-denoise #t)` coloured from the `#t` to the end of
//! the line as a comment and lost its closing paren from matching. The whole
//! buffer re-lexes per draw — microseconds at scene-file sizes — and a
//! per-line signature limits reshaping to lines whose spans actually changed.

use super::{Language, ParenPos, Scan, Tok};

pub struct Scheme;

/// The one there is: every buffer's language until E6 brings a second.
pub static SCHEME: Scheme = Scheme;

impl Language for Scheme {
    fn scan(&self, lines: &[&str]) -> Scan {
        let mut st = LexIn::Code;
        let mut out = Scan { spans: Vec::with_capacity(lines.len()), parens: Vec::new() };
        let mut scratch = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            scratch.clear();
            let (spans, next) = lex_line(st, line, &mut scratch);
            out.parens.extend(scratch.iter().map(|&(byte, ch)| ParenPos { line: i, byte, ch }));
            out.spans.push(spans);
            st = next;
        }
        out
    }

    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize {
        target_indent(lines, line, byte)
    }

    /// The head symbol of the innermost UNCLOSED `(` at the point — in
    /// `(add-sphere :at [1 2|` that's "add-sphere". Parens inside strings
    /// and comments don't count. Same-line heads only (the overwhelmingly
    /// common lisp shape).
    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String> {
        let mut st = LexIn::Code;
        let mut stack: Vec<(usize, usize, u8)> = Vec::new();
        let mut scratch = Vec::new();
        for (ln, text) in lines.iter().enumerate() {
            if ln > line {
                break;
            }
            let slice = if ln == line { &text[..byte.min(text.len())] } else { text };
            scratch.clear();
            let (_, next) = lex_line(st, slice, &mut scratch);
            for &(b, ch) in &scratch {
                match ch {
                    b'(' | b'[' | b'{' => stack.push((ln, b, ch)),
                    _ => {
                        stack.pop();
                    }
                }
            }
            st = next;
        }
        let (ln, b) = stack.iter().rev().find(|&&(_, _, ch)| ch == b'(').map(|&(l, b, _)| (l, b))?;
        let rest = lines[ln][b + 1..].trim_start();
        let head: String = rest
            .chars()
            .take_while(|&c| !matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ' ' | '\t' | '"' | ';'))
            .collect();
        (!head.is_empty()).then_some(head)
    }
}

/// Lexer state carried across lines (multi-line strings and comments).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LexIn {
    #[default]
    Code,
    /// Inside a `"…"` string.
    Str,
    /// Inside a `#| … |#` block comment, this many deep (they nest).
    Block(u8),
}

/// Scan a block comment from `i` at nesting `depth` → (end, depth left).
/// Depth 0 means it closed at `end`.
fn scan_block(b: &[u8], mut i: usize, mut depth: u8) -> (usize, u8) {
    let n = b.len();
    while i < n {
        if b[i] == b'|' && i + 1 < n && b[i + 1] == b'#' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return (i, 0);
            }
        } else if b[i] == b'#' && i + 1 < n && b[i + 1] == b'|' {
            depth = depth.saturating_add(1);
            i += 2;
        } else {
            i += 1;
        }
    }
    (n, depth)
}

/// Lex one line given the carry-in state → (spans, carry-out). Code-context
/// delimiters land in `parens` as (byte, ch) for match-finding and indent.
pub fn lex_line(state: LexIn, line: &str, parens: &mut Vec<(usize, u8)>) -> (Vec<(usize, usize, Tok)>, LexIn) {
    let b = line.as_bytes();
    let n = b.len();
    let mut spans = Vec::new();
    let mut i = 0usize;
    match state {
        LexIn::Str => {
            let mut closed = false;
            while i < n {
                match b[i] {
                    b'\\' => i += 2,
                    b'"' => {
                        i += 1;
                        closed = true;
                        break;
                    }
                    _ => i += 1,
                }
            }
            spans.push((0, i.min(n), Tok::Str));
            if !closed {
                return (spans, LexIn::Str);
            }
        }
        LexIn::Block(depth) => {
            let (end, left) = scan_block(b, 0, depth);
            spans.push((0, end, Tok::Comment));
            if left > 0 {
                return (spans, LexIn::Block(left));
            }
            i = end;
        }
        LexIn::Code => {}
    }
    let is_delim = |c: u8| {
        matches!(c, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b' ' | b'\t' | b'"' | b';')
    };
    // True right after an open `(` — the next symbol is the form's head.
    let mut call_pending = false;
    while i < n {
        match b[i] {
            b';' => {
                spans.push((i, n, Tok::Comment));
                i = n;
            }
            b'#' if i + 1 < n && b[i + 1] == b'|' => {
                let start = i;
                let (end, left) = scan_block(b, i + 2, 1);
                spans.push((start, end, Tok::Comment));
                if left > 0 {
                    return (spans, LexIn::Block(left));
                }
                i = end;
            }
            b'#' if i + 1 < n && b[i + 1] == b';' => {
                // A datum comment: the mark is coloured, the datum it
                // comments out still lexes (its parens must still match).
                spans.push((i, i + 2, Tok::Comment));
                i += 2;
            }
            b'#' if i + 1 < n && b[i + 1] == b'(' => {
                // A vector literal: `#(` opens like `(`, but its first
                // element is data, not a call.
                parens.push((i + 1, b'('));
                spans.push((i, i + 2, Tok::Paren));
                call_pending = false;
                i += 2;
            }
            b'#' if i + 1 < n && b[i + 1] == b'\\' => {
                // A character: `#\(` is a paren CHARACTER, not a paren.
                let start = i;
                i = (i + 3).min(n);
                while i < n && !is_delim(b[i]) {
                    i += 1;
                }
                spans.push((start, i, Tok::Str));
                call_pending = false;
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut closed = false;
                while i < n {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            closed = true;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                spans.push((start, i.min(n), Tok::Str));
                if !closed {
                    return (spans, LexIn::Str);
                }
                call_pending = false;
            }
            b'\'' | b'`' | b',' => {
                // Quote prefixes: the form after them is data.
                i += 1;
                call_pending = false;
            }
            c @ (b'(' | b'[' | b'{' | b')' | b']' | b'}') => {
                parens.push((i, c));
                spans.push((i, i + 1, Tok::Paren));
                call_pending = c == b'(';
                i += 1;
            }
            b' ' | b'\t' => i += 1,
            _ => {
                let start = i;
                while i < n && !is_delim(b[i]) {
                    i += 1;
                }
                let word = &line[start..i];
                let first = word.as_bytes()[0];
                let tok = if first == b':' {
                    Some(Tok::Keyword)
                } else if first == b'#'
                    || first.is_ascii_digit()
                    || (word.len() > 1
                        && matches!(first, b'-' | b'+' | b'.')
                        && word.as_bytes()[1].is_ascii_digit())
                {
                    // `#t`, `#f`, `#true`, `#e1.5`: literals, coloured as numbers.
                    Some(Tok::Number)
                } else if call_pending {
                    Some(Tok::Call)
                } else {
                    None
                };
                if let Some(t) = tok {
                    spans.push((start, i, t));
                }
                call_pending = false;
            }
        }
    }
    (spans, LexIn::Code)
}

/// The column a NEW line at (`upto_line`, `upto_byte`) should indent to:
/// the innermost unclosed `(`'s column + 2 (the Lisp convention), +1 for
/// `[`/`{`, 0 at top level. Strings and comments don't count.
pub fn target_indent(lines: &[&str], upto_line: usize, upto_byte: usize) -> usize {
    let mut st = LexIn::Code;
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let mut scratch = Vec::new();
    for (ln, text) in lines.iter().enumerate() {
        if ln > upto_line {
            break;
        }
        let slice = if ln == upto_line { &text[..upto_byte.min(text.len())] } else { text };
        scratch.clear();
        let (_, next) = lex_line(st, slice, &mut scratch);
        for &(byte, ch) in &scratch {
            match ch {
                b'(' | b'[' | b'{' => stack.push((ch, byte)),
                _ => {
                    stack.pop();
                }
            }
        }
        st = next;
    }
    match stack.last() {
        Some(&(b'(', col)) => col + 2,
        Some(&(_, col)) => col + 1,
        None => 0,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::find_partner;

    #[test]
    fn lexer_classifies_scheme_tokens() {
        let mut parens = Vec::new();
        let (spans, out) =
            lex_line(LexIn::Code, "(add-sphere :at [1 2] 0.5) ; hi", &mut parens);
        assert_eq!(out, LexIn::Code);
        let find = |spans: &[(usize, usize, Tok)], t: Tok| spans.iter().filter(|s| s.2 == t).count();
        assert_eq!(find(&spans, Tok::Call), 1, "head symbol is a call");
        assert_eq!(find(&spans, Tok::Keyword), 1, ":at is a keyword");
        assert_eq!(find(&spans, Tok::Number), 3, "1 2 0.5 are numbers");
        assert_eq!(find(&spans, Tok::Comment), 1);
        assert_eq!(find(&spans, Tok::Paren), 4);
        assert_eq!(parens.len(), 4);

        // `#t` is a literal, not Janet's comment: the closing paren still
        // counts, and only the `;` tail is a comment.
        let mut p = Vec::new();
        let line = "(set-denoise #t) ; on";
        let (spans, _) = lex_line(LexIn::Code, line, &mut p);
        assert_eq!(p.len(), 2, "both parens count");
        let comments: Vec<_> = spans.iter().filter(|s| s.2 == Tok::Comment).collect();
        assert_eq!(comments.len(), 1);
        assert_eq!(&line[comments[0].0..comments[0].1], "; on");
        assert_eq!(find(&spans, Tok::Number), 1, "#t is a literal");

        // A vector opens a paren; a character `#\(` does not.
        let mut p = Vec::new();
        let (_, _) = lex_line(LexIn::Code, "(list #(1 2) #\\( 3)", &mut p);
        assert_eq!(p.len(), 4, "( #( ) ) — the #\\( character is not a paren");

        // Multi-line string state carries; code resumes after the closer.
        let (_, st) = lex_line(LexIn::Code, "(print \"open", &mut Vec::new());
        assert_eq!(st, LexIn::Str);
        let mut p2 = Vec::new();
        let (spans, st) = lex_line(st, "still\" (f)", &mut p2);
        assert_eq!(st, LexIn::Code);
        assert_eq!(spans[0], (0, 6, Tok::Str));
        assert_eq!(p2.len(), 2, "parens after the string close count again");

        // Block comments carry across lines and nest; parens inside don't count.
        let mut p3 = Vec::new();
        let (_, st) = lex_line(LexIn::Code, "#| a ) #| b", &mut p3);
        assert_eq!(st, LexIn::Block(2));
        let (_, st) = lex_line(st, "c |# d |# (g)", &mut p3);
        assert_eq!(st, LexIn::Code);
        assert_eq!(p3.len(), 2, "only (g)'s parens count");
    }

    #[test]
    fn indent_follows_the_innermost_open_paren() {
        let lines = ["(define (f)", "  (let ((x 1"];
        // After line 1: innermost open is the second `(` of `((` at byte 8
        // → a `(` indents its body two past itself = 10.
        assert_eq!(target_indent(&lines, 1, lines[1].len()), 10);
        // A `[` aligns one past itself.
        assert_eq!(target_indent(&["(f [x"], 0, 5), 4);
        // After just line 0: `(f)` closed; innermost is `(` at 0 → 2.
        assert_eq!(target_indent(&lines[..1], 0, lines[0].len()), 2);
        // Parens inside strings/comments don't count.
        let tricky = ["(a \"( [\" ; ("];
        assert_eq!(target_indent(&tricky, 0, tricky[0].len()), 2);
        assert_eq!(target_indent(&["(done)"], 0, 6), 0);
    }

    #[test]
    fn paren_partner_matching_skips_across_lines() {
        let text = ["(a [b", "   c] d)"];
        let mut parens = Vec::new();
        let mut st = LexIn::Code;
        for (i, l) in text.iter().enumerate() {
            let mut p = Vec::new();
            let (_, next) = lex_line(st, l, &mut p);
            parens.extend(p.iter().map(|&(byte, ch)| ParenPos { line: i, byte, ch }));
            st = next;
        }
        // parens: ( [ ] )
        assert_eq!(find_partner(&parens, 0), Some(3), "( matches the final )");
        assert_eq!(find_partner(&parens, 1), Some(2), "[ matches ]");
        assert_eq!(find_partner(&parens, 3), Some(0), ") matches back to (");
        let unbalanced = [ParenPos { line: 0, byte: 0, ch: b')' }];
        assert_eq!(find_partner(&unbalanced, 0), None);
    }

    #[test]
    fn enclosing_head_is_the_innermost_open_call() {
        let lines = ["(add-sphere :at [1 2", "  (f \"(g\" ; (h"];
        assert_eq!(SCHEME.enclosing_head(&lines, 0, lines[0].len()).as_deref(), Some("add-sphere"));
        // A string's and a comment's parens don't open anything.
        assert_eq!(SCHEME.enclosing_head(&lines, 1, lines[1].len()).as_deref(), Some("f"));
        assert_eq!(SCHEME.enclosing_head(&["(done) x"], 0, 8), None);
    }
}
