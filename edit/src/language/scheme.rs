//! **Scheme, as the editor reads it** — `scm-text`'s token scan (E1c), the
//! one the modeler's `M-.` index and `scm-lsp` use too, so a fix to how
//! Scheme is read reaches all three. Until E1c the editor had a lexer of
//! its own; the two agreed on every line of the tree's 388 Scheme files
//! (209,799 lines, a million spans) before the old one went.
//!
//! What the editor adds to the tokens is the highlight CLASS: `:keyword`,
//! literal (`#t`, `#\c`, numbers), a form's head (the first thing after a
//! `(` on its own line, not after a vector's `#(`). The COLORS are theme
//! slots (code-comment/string/keyword/number/call/paren), so the look
//! stays the host's. It reads Scheme's spelling: `;` line comments, `#| … |#`
//! block comments (nesting), `#;` datum comments, `#t`/`#f`/`#\c` literals
//! and `#(` vectors. Until 2026-11-02 the old lexer read Janet's — `#` as a
//! line comment — so `(set-denoise #t)` coloured from the `#t` to the end of
//! the line as a comment and lost its closing paren from matching. The whole
//! buffer re-scans per draw — sub-millisecond at scene-file sizes — and a
//! per-line signature limits reshaping to lines whose spans actually changed.
//!
//! `scm-text` counts CHARACTERS and the widget counts BYTES within a line,
//! so every token is walked back to (line, byte) once, in order.

use scm_text::text::{Kind, Token};

use super::{Diagnostic, Language, ParenPos, Rescan, Scan, Severity, Tok};

pub struct Scheme;

/// The one there is: every buffer's language until E6 brings a second.
pub static SCHEME: Scheme = Scheme;

impl Language for Scheme {
    fn name(&self) -> &'static str {
        "scheme"
    }

    fn title(&self) -> &'static str {
        "Scheme"
    }

    fn scan(&self, lines: &[&str]) -> Scan {
        let toks = Tokens::new(lines);
        let mut out = Scan { spans: vec![Vec::new(); lines.len()], parens: Vec::new(), clean: toks.clean_lines() };
        let span = |out: &mut Scan, (l0, b0): (usize, usize), (l1, b1): (usize, usize), t: Tok| {
            for (l, line) in lines.iter().enumerate().take(l1 + 1).skip(l0) {
                let s = if l == l0 { b0 } else { 0 };
                let e = if l == l1 { b1 } else { line.len() };
                if let Some(v) = out.spans.get_mut(l) {
                    v.push((s, e, t));
                }
            }
        };
        for (i, p) in toks.placed.iter().enumerate() {
            match p.tok.kind {
                Kind::Comment => span(&mut out, p.start, p.end, Tok::Comment),
                Kind::Str => span(&mut out, p.start, p.end, Tok::Str),
                Kind::Prefix if toks.is_datum_comment(i) => span(&mut out, p.start, p.end, Tok::Comment),
                Kind::Prefix => {}
                Kind::Open | Kind::Close => {
                    let ch = lines[p.start.0].as_bytes()[p.start.1];
                    out.parens.push(ParenPos { line: p.start.0, byte: p.start.1, ch });
                    span(&mut out, p.start, p.end, Tok::Paren);
                }
                Kind::Atom => {
                    if let Some(tok) = toks.class(i) {
                        span(&mut out, p.start, p.end, tok);
                    }
                }
            }
        }
        out
    }

    /// Re-scan from the last clean line start at or before the edit to the
    /// first clean one after it whose text the edit left alone: text after
    /// a clean start scans the same whatever came before, so the old scan
    /// holds above and (shifted) below. When the stretch does not end clean
    /// — a string just opened — it runs on to the next clean line, and at
    /// worst to the end.
    fn rescan(&self, old: &Scan, old_lines: &[String], lines: &[&str]) -> Option<Rescan> {
        let (on, nn) = (old_lines.len(), lines.len());
        if old.clean.len() != on || old.spans.len() != on {
            return None;
        }
        let prefix = old_lines.iter().zip(lines).take_while(|(a, b)| a.as_str() == **b).count();
        let room = on.min(nn) - prefix;
        let suffix = old_lines.iter().rev().zip(lines.iter().rev()).take(room).take_while(|(a, b)| a.as_str() == **b).count();
        let start = (0..=prefix.min(on.saturating_sub(1))).rev().find(|&l| old.clean[l]).unwrap_or(0);
        // Old lines from here down are the edit's unchanged suffix.
        let first_kept = on - suffix;
        let shift = |l: usize| l + nn - on;
        for old_end in (first_kept..=on).filter(|&k| k == on || old.clean[k]) {
            let new_end = shift(old_end);
            if new_end < start {
                continue;
            }
            // A probe line after the stretch says how the stretch ends: one
            // space, so that a string or comment left open runs INTO it (an
            // empty line would start exactly where the open token ends, and
            // look clean).
            // (Running to the end of the text, there is nothing below to
            // hold, and no probe.)
            let to_end = new_end == nn;
            let probe = if to_end { None } else { Some(" ") };
            let part: Vec<&str> = lines[start..new_end].iter().copied().chain(probe).collect();
            let mut scan = self.scan(&part);
            // The stretch must end clean for the old scan below to hold.
            if !to_end {
                let ends_clean = scan.clean.pop().unwrap_or(true);
                scan.spans.pop();
                if !ends_clean {
                    continue;
                }
            }
            let mut spans = old.spans[..start].to_vec();
            spans.extend(scan.spans);
            spans.extend_from_slice(&old.spans[old_end..]);
            let mut clean = old.clean[..start].to_vec();
            clean.extend(scan.clean);
            clean.extend_from_slice(&old.clean[old_end..]);
            let mut parens: Vec<ParenPos> = old.parens.iter().filter(|p| p.line < start).copied().collect();
            parens.extend(scan.parens.iter().filter(|p| p.line < new_end - start).map(|p| ParenPos { line: p.line + start, ..*p }));
            parens.extend(old.parens.iter().filter(|p| p.line >= old_end).map(|p| ParenPos { line: shift(p.line), ..*p }));
            return Some(Rescan { scan: Scan { spans, parens, clean }, start, old_end, new_end });
        }
        None
    }

    /// The innermost unclosed bracket's column + 2 for a `(` (the Lisp
    /// convention), +1 for `[`/`{`, 0 at top level. Strings and comments
    /// don't count.
    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize {
        match Tokens::new(lines).open_at((line, byte)).last() {
            Some(&(b'(', _, col)) => col + 2,
            Some(&(_, _, col)) => col + 1,
            None => 0,
        }
    }

    /// The head symbol of the innermost UNCLOSED `(` at the point — in
    /// `(add-sphere :at [1 2|` that's "add-sphere". Parens inside strings
    /// and comments don't count. Same-line heads only (the overwhelmingly
    /// common lisp shape).
    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String> {
        let open = Tokens::new(lines).open_at((line, byte));
        let &(_, ln, b) = open.iter().rev().find(|&&(ch, _, _)| ch == b'(')?;
        let rest = lines[ln][b + 1..].trim_start();
        let head: String = rest
            .chars()
            .take_while(|&c| !matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ' ' | '\t' | '"' | ';'))
            .collect();
        (!head.is_empty()).then_some(head)
    }

    /// What scm's READER says, when the host has installed one
    /// ([`set_reader`]): the datum that will not read, or — when the text
    /// ends inside a form — the outermost bracket left open, which is where
    /// the missing closer belongs. That one is quiet, not an error: a form
    /// being typed is open until its last paren, and a red mark under the
    /// cursor at every keystroke would be noise (scm-lsp's rule, which
    /// reports it as information).
    fn diagnose(&self, lines: &[&str], scan: &Scan) -> Vec<Diagnostic> {
        let Some(read) = READER.get() else { return Vec::new() };
        // Form by form when the scan says where they are; the reader's
        // `#!fold-case` carries from one form to the next, so not then.
        if scan.clean.len() == lines.len() && !lines.iter().any(|l| l.contains("fold-case"))
            && let Some(d) = read_by_form(*read, lines, scan)
        {
            return d;
        }
        match read(&lines.join("\n")) {
            ReadCheck::Fine => Vec::new(),
            ReadCheck::Invalid { start, end, message } => {
                // Offsets in order, as the walker wants them.
                let mut walk = Walker::new(lines);
                let (from, to) = (walk.at(start), walk.at(end.max(start)));
                vec![Diagnostic { start: from, end: to, message, severity: Severity::Error }]
            }
            ReadCheck::Incomplete => open_end(lines, scan),
        }
    }
}

/// What reading one top-level stretch said, offsets in its own text.
#[derive(Clone)]
enum FormRead {
    Fine,
    Incomplete,
    Invalid { start: usize, end: usize, message: String },
}

thread_local! {
    /// Each top-level stretch's reading, by a hash of its text: a keystroke
    /// changes one, and only that one is read again.
    static FORM_READS: std::cell::RefCell<std::collections::HashMap<u64, FormRead>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The whole text's reading, one top-level stretch at a time — the lines
/// between clean line starts ([`Scan::clean`]), which read the same alone as
/// in place, since nothing carries across a clean start. The first stretch
/// that will not read is the text's error, as reading it whole finds it. A
/// stretch that ends inside a form is the text's open end when it is the
/// last; earlier, the scan and the reader disagree about where a form ends,
/// and None sends the caller back to reading the whole text.
pub(crate) fn read_by_form(read: fn(&str) -> ReadCheck, lines: &[&str], scan: &Scan) -> Option<Vec<Diagnostic>> {
    use std::hash::{Hash, Hasher};
    let starts: Vec<usize> = (0..lines.len()).filter(|&l| l == 0 || scan.clean[l]).collect();
    FORM_READS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 50_000 {
            cache.clear();
        }
        for (k, &a) in starts.iter().enumerate() {
            let b = starts.get(k + 1).copied().unwrap_or(lines.len());
            let part = &lines[a..b];
            let key = {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                part.hash(&mut h);
                h.finish()
            };
            let got = cache
                .entry(key)
                .or_insert_with(|| match read(&part.join("\n")) {
                    ReadCheck::Fine => FormRead::Fine,
                    ReadCheck::Incomplete => FormRead::Incomplete,
                    ReadCheck::Invalid { start, end, message } => FormRead::Invalid { start, end, message },
                })
                .clone();
            match got {
                FormRead::Fine => {}
                FormRead::Invalid { start, end, message } => {
                    let mut walk = Walker::new(part);
                    let (from, to) = (walk.at(start), walk.at(end.max(start)));
                    let (from, to) = ((from.0 + a, from.1), (to.0 + a, to.1));
                    return Some(vec![Diagnostic { start: from, end: to, message, severity: Severity::Error }]);
                }
                FormRead::Incomplete if b == lines.len() => return Some(open_end(lines, scan)),
                FormRead::Incomplete => return None,
            }
        }
        Some(Vec::new())
    })
}

/// The text ends inside a form: the outermost bracket left open, which is
/// where the missing closer belongs — or, with none, the string or `#|`
/// comment still open.
fn open_end(lines: &[&str], scan: &Scan) -> Vec<Diagnostic> {
    let mut open: Vec<&ParenPos> = Vec::new();
    for p in &scan.parens {
        if matches!(p.ch, b'(' | b'[' | b'{') {
            open.push(p);
        } else {
            open.pop();
        }
    }
    let (at, message) = match open.first() {
        Some(p) => ((p.line, p.byte), format!("`{}` is never closed: what follows it reads as inside it", p.ch as char)),
        None => {
            let last = lines.len().saturating_sub(1);
            ((last, lines.get(last).map_or(0, |l| l.len())), "a string or `#|` comment is never closed".to_string())
        }
    };
    let end = (at.0, (at.1 + 1).min(lines.get(at.0).map_or(0, |l| l.len())));
    vec![Diagnostic { start: at, end, message, severity: Severity::Open }]
}

/// What a READER says of a whole text, in CHARACTER offsets — the
/// interpreter's own answer, which is what decides whether a send will
/// read. Installed by the binding (`scm_binding`), because this module may
/// not name the VM (`layering.rs`).
pub enum ReadCheck {
    Fine,
    /// The text ends inside a datum.
    Incomplete,
    Invalid { start: usize, end: usize, message: String },
}

static READER: std::sync::OnceLock<fn(&str) -> ReadCheck> = std::sync::OnceLock::new();

/// Install the reader [`Scheme::diagnose`] asks. The first one stays: there
/// is one interpreter's reader per process.
pub fn set_reader(read: fn(&str) -> ReadCheck) {
    let _ = READER.set(read);
}

/// A token, and where it starts and ends as (line, byte).
struct Placed {
    tok: Token,
    start: (usize, usize),
    end: (usize, usize),
}

/// The buffer's tokens, placed.
struct Tokens<'a> {
    lines: &'a [&'a str],
    placed: Vec<Placed>,
}

impl<'a> Tokens<'a> {
    fn new(lines: &'a [&'a str]) -> Self {
        let doc = scm_text::Doc::new(&lines.join("\n"));
        let mut walk = Walker::new(lines);
        let placed = doc
            .tokens()
            .iter()
            .map(|&tok| Placed { tok, start: walk.at(tok.start), end: walk.at(tok.end) })
            .collect();
        Tokens { lines, placed }
    }

    fn text(&self, i: usize) -> &'a str {
        let p = &self.placed[i];
        if p.start.0 == p.end.0 {
            &self.lines[p.start.0][p.start.1..p.end.1]
        } else {
            // Only strings and block comments span lines, and no caller
            // classifies those by their text.
            &self.lines[p.start.0][p.start.1..]
        }
    }

    fn is_datum_comment(&self, i: usize) -> bool {
        self.placed[i].tok.kind == Kind::Prefix && self.text(i) == "#;"
    }

    /// Which lines start clean: outside any token (a string or a `#|`
    /// comment running across lines), at depth 0, with no prefix (`'`,
    /// `#;`) waiting for its datum.
    fn clean_lines(&self) -> Vec<bool> {
        let mut out = Vec::with_capacity(self.lines.len());
        let (mut i, mut depth, mut waiting, mut spans) = (0usize, 0usize, false, false);
        for l in 0..self.lines.len() {
            while i < self.placed.len() && self.placed[i].start < (l, 0) {
                let p = &self.placed[i];
                match p.tok.kind {
                    Kind::Open => depth += 1,
                    Kind::Close => depth = depth.saturating_sub(1),
                    _ => {}
                }
                if p.tok.kind != Kind::Comment {
                    waiting = p.tok.kind == Kind::Prefix;
                }
                i += 1;
            }
            // The last token taken may run to this line's start or past it —
            // `#\` and the newline it names end exactly there, and leave the
            // line an empty span of their own.
            if i > 0 {
                spans = self.placed[i - 1].end >= (l, 0) && self.placed[i - 1].start < (l, 0);
            }
            out.push(depth == 0 && !waiting && !spans);
        }
        out
    }

    /// An atom's highlight class, or `None` for plain body text.
    fn class(&self, i: usize) -> Option<Tok> {
        let word = self.text(i);
        let first = *word.as_bytes().first()?;
        let next_opens = self
            .placed
            .get(i + 1)
            .is_some_and(|n| n.tok.kind == Kind::Open && n.tok.start == self.placed[i].tok.end);
        if word.starts_with("#\\") {
            Some(Tok::Str)
        } else if word == "#" && next_opens {
            // `#(`: a vector opens like `(`, coloured as one.
            Some(Tok::Paren)
        } else if first == b':' {
            Some(Tok::Keyword)
        } else if first == b'#'
            || first.is_ascii_digit()
            || (word.len() > 1 && matches!(first, b'-' | b'+' | b'.') && word.as_bytes()[1].is_ascii_digit())
        {
            // `#t`, `#f`, `#true`, `#e1.5`: literals, coloured as numbers.
            Some(Tok::Number)
        } else if self.is_head(i) {
            Some(Tok::Call)
        } else {
            None
        }
    }

    /// Is the atom at `i` a form's head — the first thing after a `(` on
    /// its own line? Comments and `#;` between don't count; a vector's `#(`
    /// opens data, not a call, and a `[` or `{` is not a call either.
    fn is_head(&self, i: usize) -> bool {
        let line = self.placed[i].start.0;
        let Some(at) = (0..i)
            .rev()
            .find(|&j| !(self.placed[j].tok.kind == Kind::Comment || self.is_datum_comment(j)))
        else {
            return false;
        };
        let open = &self.placed[at];
        if open.tok.kind != Kind::Open || open.start.0 != line || self.text(at) != "(" {
            return false;
        }
        // The token before the `(`: a bare `#` glued to it makes a vector.
        !(at > 0 && {
            let q = &self.placed[at - 1];
            q.tok.kind == Kind::Atom && q.tok.end == open.tok.start && self.text(at - 1) == "#"
        })
    }

    /// The brackets still open just before `point`, outermost first, as
    /// (bracket, line, byte).
    fn open_at(&self, point: (usize, usize)) -> Vec<(u8, usize, usize)> {
        let mut stack = Vec::new();
        for (i, p) in self.placed.iter().enumerate() {
            if p.start >= point {
                break;
            }
            match p.tok.kind {
                Kind::Open => stack.push((*self.text(i).as_bytes().last().unwrap_or(&b'('), p.start.0, p.start.1)),
                Kind::Close => {
                    stack.pop();
                }
                _ => {}
            }
        }
        stack
    }
}

/// Char offsets → (line, byte), for offsets visited in increasing order —
/// the tokens' order — so the whole walk is one pass.
struct Walker<'a> {
    lines: &'a [&'a str],
    line: usize,
    /// The char offset where `line` starts, and its length in chars.
    line_start: usize,
    line_chars: usize,
    /// Inside `line`: a char count and the byte it is at.
    chars: usize,
    byte: usize,
}

impl<'a> Walker<'a> {
    fn new(lines: &'a [&'a str]) -> Self {
        let line_chars = lines.first().map_or(0, |l| l.chars().count());
        Walker { lines, line: 0, line_start: 0, line_chars, chars: 0, byte: 0 }
    }

    fn at(&mut self, off: usize) -> (usize, usize) {
        // Whole lines first: each holds its chars and one '\n'.
        while self.line + 1 < self.lines.len() && off > self.line_start + self.line_chars {
            self.line_start += self.line_chars + 1;
            self.line += 1;
            self.line_chars = self.lines[self.line].chars().count();
            self.chars = 0;
            self.byte = 0;
        }
        let text = self.lines.get(self.line).copied().unwrap_or("");
        let want = off - self.line_start;
        if text.len() == self.line_chars {
            // ASCII (bytes = chars), which nearly every line is.
            self.chars = want;
            self.byte = want;
            return (self.line, self.byte);
        }
        for c in text[self.byte..].chars() {
            if self.chars >= want {
                break;
            }
            self.chars += 1;
            self.byte += c.len_utf8();
        }
        (self.line, self.byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::find_partner;

    /// A re-scan after an edit is the scan of the edited text, exactly — over
    /// thousands of random edits that open and close strings, `#|` comments,
    /// brackets and `#;`, across lines and not.
    #[test]
    fn rescan_equals_a_full_scan_after_random_edits() {
        let base = "(define (f x)\n  \"a string\n   over lines\" ; c\n  (g x #\\( 'y))\n\n#| block\n (not code) |#\n(define v #(1 2 \"3\"))\n#;(skipped (form))\n(h [a {b}] `(c ,d))\n\n(define s \"(\")\n";
        let bits = ["(", ")", "\"", "#|", "|#", "#;", "'", "\n", "x", " ", "#\\(", ";", "[", "]", "\n\n", "(define (k)\n  1)\n", "#\\"];
        for start_seed in [0x2545_f491_4f6c_dd1d_u64, 0x9e37_79b9_7f4a_7c15, 0x0123_4567_89ab_cdef] {
        let mut text: Vec<char> = base.repeat(4).chars().collect();
        let mut seed = start_seed;
        let mut rnd = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as usize
        };
        let mut rescans = 0;
        for step in 0..3000 {
            let old: String = text.iter().collect();
            let old_lines: Vec<String> = old.split('\n').map(str::to_string).collect();
            let old_refs: Vec<&str> = old_lines.iter().map(String::as_str).collect();
            let old_scan = SCHEME.scan(&old_refs);
            // An edit: insert a piece, or delete a short run (newlines too).
            let at = rnd(text.len() + 1);
            if rnd(3) == 0 && !text.is_empty() {
                let at = at.min(text.len() - 1);
                let n = 1 + rnd(6);
                text.drain(at..(at + n).min(text.len()));
            } else {
                let piece: Vec<char> = bits[rnd(bits.len())].chars().collect();
                text.splice(at..at, piece);
            }
            if text.len() > 4000 {
                text.truncate(2000);
            }
            let new: String = text.iter().collect();
            let lines: Vec<&str> = new.split('\n').collect();
            let full = SCHEME.scan(&lines);
            if let Some(r) = SCHEME.rescan(&old_scan, &old_lines, &lines) {
                rescans += 1;
                if r.scan != full {
                    let first = (0..lines.len())
                        .find(|&l| r.scan.spans.get(l) != full.spans.get(l) || r.scan.clean.get(l) != full.clean.get(l))
                        .unwrap_or(usize::MAX);
                    let around = |ls: &[&str], l: usize| ls[l.saturating_sub(3)..(l + 2).min(ls.len())].join("⏎");
                    panic!(
                        "step {step}: re-scan [{}, {}→{}) of {} lines (was {}) differs first at line {first}\n  new text near: {}\n  old text near: {}\n  rescan {:?} clean {:?}\n  full   {:?} clean {:?}\n  parens equal: {}",
                        r.start, r.old_end, r.new_end, lines.len(), old_refs.len(),
                        around(&lines, first.min(lines.len() - 1)), around(&old_refs, first.min(old_refs.len() - 1)),
                        r.scan.spans.get(first), r.scan.clean.get(first), full.spans.get(first), full.clean.get(first),
                        r.scan.parens == full.parens
                    );
                }
            }
        }
        assert!(rescans > 2500, "it re-scanned rather than giving up: {rescans}");
        }
    }

    #[test]
    fn lexer_classifies_scheme_tokens() {
        let find = |spans: &[(usize, usize, Tok)], t: Tok| spans.iter().filter(|s| s.2 == t).count();
        let scan = SCHEME.scan(&["(add-sphere :at [1 2] 0.5) ; hi"]);
        let spans = &scan.spans[0];
        assert_eq!(find(spans, Tok::Call), 1, "head symbol is a call");
        assert_eq!(find(spans, Tok::Keyword), 1, ":at is a keyword");
        assert_eq!(find(spans, Tok::Number), 3, "1 2 0.5 are numbers");
        assert_eq!(find(spans, Tok::Comment), 1);
        assert_eq!(find(spans, Tok::Paren), 4);
        assert_eq!(scan.parens.len(), 4);

        // `#t` is a literal, not Janet's comment: the closing paren still
        // counts, and only the `;` tail is a comment.
        let line = "(set-denoise #t) ; on";
        let scan = SCHEME.scan(&[line]);
        assert_eq!(scan.parens.len(), 2, "both parens count");
        let comments: Vec<_> = scan.spans[0].iter().filter(|s| s.2 == Tok::Comment).collect();
        assert_eq!(comments.len(), 1);
        assert_eq!(&line[comments[0].0..comments[0].1], "; on");
        assert_eq!(find(&scan.spans[0], Tok::Number), 1, "#t is a literal");

        // A vector opens a paren; a character `#\\(` does not.
        let scan = SCHEME.scan(&["(list #(1 2) #\\( 3)"]);
        assert_eq!(scan.parens.len(), 4, "( #( ) ) — the #\\( character is not a paren");

        // A string carries across lines; code resumes after the closer.
        let scan = SCHEME.scan(&["(print \"open", "still\" (f)"]);
        assert_eq!(scan.spans[1][0], (0, 6, Tok::Str));
        assert_eq!(scan.parens.iter().filter(|p| p.line == 1).count(), 2, "parens after the string close count again");

        // Block comments carry across lines and nest; parens inside don't count.
        let scan = SCHEME.scan(&["#| a ) #| b", "c |# d |# (g)"]);
        assert_eq!(scan.spans[0], vec![(0, 11, Tok::Comment)]);
        assert_eq!(scan.spans[1][0], (0, 9, Tok::Comment));
        assert_eq!(scan.parens.len(), 2, "only (g)'s parens count");
    }

    /// Lines are bytes to the widget and characters to `scm-text`: a token
    /// after a `λ` or an emoji must land on its own bytes.
    #[test]
    fn offsets_survive_multibyte_text() {
        let lines = ["(λ (x) \"☕\")", "  (café :k)"];
        let scan = SCHEME.scan(&lines);
        let str_span = scan.spans[0].iter().find(|s| s.2 == Tok::Str).unwrap();
        assert_eq!(&lines[0][str_span.0..str_span.1], "\"☕\"");
        assert_eq!(scan.spans[0].iter().find(|s| s.2 == Tok::Call).map(|s| &lines[0][s.0..s.1]), Some("λ"));
        let kw = scan.spans[1].iter().find(|s| s.2 == Tok::Keyword).unwrap();
        assert_eq!(&lines[1][kw.0..kw.1], ":k");
        let last = scan.parens.last().unwrap();
        assert_eq!((last.line, &lines[1][last.byte..last.byte + 1]), (1, ")"));
        assert_eq!(SCHEME.enclosing_head(&lines, 1, lines[1].len() - 1).as_deref(), Some("café"));
    }

    #[test]
    fn indent_follows_the_innermost_open_paren() {
        let lines = ["(define (f)", "  (let ((x 1"];
        // After line 1: innermost open is the second `(` of `((` at byte 8
        // → a `(` indents its body two past itself = 10.
        assert_eq!(SCHEME.indent(&lines, 1, lines[1].len()), 10);
        // A `[` aligns one past itself.
        assert_eq!(SCHEME.indent(&["(f [x"], 0, 5), 4);
        // After just line 0: `(f)` closed; innermost is `(` at 0 → 2.
        assert_eq!(SCHEME.indent(&lines[..1], 0, lines[0].len()), 2);
        // Parens inside strings/comments don't count.
        let tricky = ["(a \"( [\" ; ("];
        assert_eq!(SCHEME.indent(&tricky, 0, tricky[0].len()), 2);
        assert_eq!(SCHEME.indent(&["(done)"], 0, 6), 0);
    }

    #[test]
    fn paren_partner_matching_skips_across_lines() {
        let parens = SCHEME.scan(&["(a [b", "   c] d)"]).parens;
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
