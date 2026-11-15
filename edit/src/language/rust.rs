//! **Rust, as the editor reads it** — piece 1 of Rust in `edit` (Kevin,
//! 2026-11-14): highlighting, indentation and the call around the point,
//! compiled in, with nothing running beside the editor. A language server
//! (rust-analyzer) is a later, opt-in step (docs/editor-crate-plan.md, E7);
//! nothing here waits for one.
//!
//! The lexer is hand-written, as Scheme's is, and reads what highlighting
//! and bracket matching need to get right: `//` and nesting `/* */`
//! comments, strings with escapes, raw strings (`r#"…"#`, any number of
//! hashes), byte and C strings, a char literal (`'x'`, `'\n'`) told from a
//! lifetime (`'a`, `'static`), numbers (`1..2` is two numbers and a range,
//! `1.5e-3` one), raw identifiers (`r#type`), and attributes. A bracket
//! inside any of those is not a bracket.
//!
//! The classes are the theme's slots: keywords and lifetimes as keywords,
//! `true`/`false` with the numbers, a function's name where it is called or
//! defined, and a macro's name with its `!`, as a call.
//!
//! **Indentation is rustfmt's block style**: four columns in from the line
//! the innermost open bracket is on, back out to that line for a closing
//! bracket, and four more for an expression carried onto the next line (a
//! method chain, the right side of a `let`). Inside a string the editor
//! never adds spaces: they would be part of the string.
//!
//! **No read marks** ([`Language::diagnose`]'s default). What is wrong with
//! Rust is the compiler's verdict, and the compiler is not in the editor.

use super::{Language, ParenPos, Scan, Tok};

pub struct Rust;

pub static RUST: Rust = Rust;

/// The keywords rustfmt's edition reserves and code actually uses.
/// `union` and `macro_rules` are contextual: `union` is an ordinary name
/// far more often than a keyword, and `macro_rules!` reads as a macro.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "yield",
];

/// The step rustfmt indents by.
const STEP: usize = 4;

impl Language for Rust {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn scan(&self, lines: &[&str]) -> Scan {
        let toks = tokens(lines);
        let mut out = Scan { spans: vec![Vec::new(); lines.len()], parens: Vec::new() };
        let span = |out: &mut Scan, (l0, b0): (usize, usize), (l1, b1): (usize, usize), t: Tok| {
            for (l, line) in lines.iter().enumerate().take(l1 + 1).skip(l0) {
                let s = if l == l0 { b0 } else { 0 };
                let e = if l == l1 { b1 } else { line.len() };
                if let Some(v) = out.spans.get_mut(l) {
                    v.push((s, e, t));
                }
            }
        };
        // The `#` of an attribute and the name just inside its `[`.
        let mut attr_name = false;
        for (i, t) in toks.iter().enumerate() {
            let class = match t.kind {
                Kind::Comment => Some(Tok::Comment),
                Kind::Str | Kind::Char => Some(Tok::Str),
                Kind::Keyword | Kind::Lifetime => Some(Tok::Keyword),
                Kind::Number | Kind::Literal => Some(Tok::Number),
                Kind::Open | Kind::Close => {
                    let ch = lines[t.start.0].as_bytes()[t.start.1];
                    out.parens.push(ParenPos { line: t.start.0, byte: t.start.1, ch });
                    Some(Tok::Paren)
                }
                Kind::Punct(b'#') if is_attribute(&toks, i) => {
                    attr_name = true;
                    Some(Tok::Keyword)
                }
                Kind::Punct(b'!') if attr_name => Some(Tok::Keyword),
                Kind::Punct(b'!') if i > 0 && is_macro(&toks, i - 1) => Some(Tok::Call),
                Kind::Punct(_) => None,
                Kind::Ident if attr_name => {
                    attr_name = false;
                    Some(Tok::Keyword)
                }
                Kind::Ident if is_call(lines, &toks, i) => Some(Tok::Call),
                Kind::Ident => None,
            };
            if let Some(class) = class {
                span(&mut out, t.start, t.end, class);
            }
            // Only `#`, `!` and `[` stand between an attribute's `#` and its name.
            if !matches!(t.kind, Kind::Open | Kind::Punct(b'!' | b'#')) {
                attr_name = false;
            }
        }
        out
    }

    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize {
        let toks = tokens(lines);
        let at = (line, byte);
        let here = lines.get(line).copied().unwrap_or("");
        // Inside a string, add nothing to it; inside a block comment, keep
        // the comment's own margin.
        if let Some(t) = toks.iter().find(|t| t.start < at && (at < t.end || (at == t.end && unclosed(lines, t)))) {
            return match t.kind {
                Kind::Str if byte <= leading(here) => leading(here),
                Kind::Str => 0,
                _ => leading(here),
            };
        }
        let mut open: Vec<&Token> = Vec::new();
        let mut prev: Option<&Token> = None;
        for t in toks.iter().take_while(|t| t.end <= at) {
            match t.kind {
                Kind::Comment => continue,
                Kind::Open => open.push(t),
                Kind::Close => {
                    open.pop();
                }
                _ => {}
            }
            prev = Some(t);
        }
        let rest = here.get(byte..).unwrap_or("").trim_start();
        let closer = rest.starts_with([')', ']', '}']);
        let inner = open.last().map(|t| lines[t.start.0].as_bytes()[t.start.1]);
        let base = match open.last() {
            Some(t) if closer => leading(lines[t.start.0]),
            Some(t) => leading(lines[t.start.0]) + STEP,
            None => 0,
        };
        // An expression carried on: the line before stopped mid-statement,
        // in a block or at the top level (an argument list's items end in
        // commas, and step in once only).
        let carried = !closer
            && !rest.starts_with('{')
            && !rest.starts_with("where")
            && !rest.starts_with("else")
            && matches!(inner, None | Some(b'{'))
            && prev.is_some_and(|p| match p.kind {
                Kind::Punct(b';' | b',') | Kind::Open => false,
                Kind::Close if is_brace(lines, p) => false,
                // `#[derive(Debug)]` ends an attribute, not an expression.
                Kind::Close => !lines[p.start.0].trim_start().starts_with('#'),
                _ => true,
            });
        match prev {
            // A chain continued from a closing `})` stays level with it:
            // rustfmt's `foo(|x| {` … `})` / `.unwrap_or(v)`.
            Some(p) if carried && rest.starts_with('.') && lines[p.start.0].trim_start().starts_with(['}', ')', ']']) => {
                leading(lines[p.start.0])
            }
            _ if carried => base + STEP,
            _ => base,
        }
    }

    /// The function the point is an argument of: the innermost unclosed `(`
    /// with a name before it — `foo`, `Vec::new`, `println!`, a method's
    /// `collect` — looking out through tuples and `[`, never out of a `{`
    /// (a block or closure body is a context of its own).
    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String> {
        let toks = tokens(lines);
        let mut open: Vec<usize> = Vec::new();
        for (i, t) in toks.iter().enumerate().take_while(|(_, t)| t.end <= (line, byte)) {
            match t.kind {
                Kind::Open => open.push(i),
                Kind::Close => {
                    open.pop();
                }
                _ => {}
            }
        }
        for &i in open.iter().rev() {
            let t = &toks[i];
            match lines[t.start.0].as_bytes()[t.start.1] {
                b'{' => return None,
                b'(' => {
                    if let Some(name) = callee(lines, &toks, i) {
                        return Some(name);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Comment,
    Str,
    Char,
    Lifetime,
    Number,
    /// `true`, `false`.
    Literal,
    Keyword,
    Ident,
    Open,
    Close,
    Punct(u8),
}

/// A token from `start` to `end`, each a (line, byte); a block comment or
/// a string may run over several lines.
#[derive(Clone, Copy, Debug)]
struct Token {
    kind: Kind,
    start: (usize, usize),
    end: (usize, usize),
}

fn ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// The width of a line's indentation, in bytes, as the widget counts it.
fn leading(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// The whole buffer as tokens, in order. Whitespace is dropped; anything
/// unrecognised is a one-character punctuation token.
fn tokens(lines: &[&str]) -> Vec<Token> {
    let mut out = Vec::new();
    let (mut l, mut b) = (0, 0);
    while l < lines.len() {
        let s = lines[l].as_bytes();
        if b >= s.len() {
            l += 1;
            b = 0;
            continue;
        }
        let start = (l, b);
        let at = |k: usize| s.get(b + k).copied();
        let (kind, end) = match s[b] {
            b' ' | b'\t' | b'\r' => {
                b += 1;
                continue;
            }
            b'/' if at(1) == Some(b'/') => (Kind::Comment, (l, s.len())),
            b'/' if at(1) == Some(b'*') => (Kind::Comment, block_comment(lines, l, b + 2)),
            b'"' => (Kind::Str, string(lines, l, b + 1)),
            b'\'' => char_or_lifetime(s, l, b),
            b'(' | b'[' | b'{' => (Kind::Open, (l, b + 1)),
            b')' | b']' | b'}' => (Kind::Close, (l, b + 1)),
            b'0'..=b'9' => (Kind::Number, (l, number(s, b))),
            c if ident_byte(c) => word(lines, s, l, b),
            c => (Kind::Punct(c), (l, b + 1)),
        };
        out.push(Token { kind, start, end });
        (l, b) = end;
    }
    out
}

/// What starts with a letter: a string with a prefix (`b"`, `c"`, `r#"`,
/// `br"`), a byte char (`b'x'`), a raw identifier (`r#type`), a keyword or
/// a name.
fn word(lines: &[&str], s: &[u8], l: usize, b: usize) -> (Kind, (usize, usize)) {
    let mut p = b;
    if matches!(s[p], b'b' | b'c') {
        p += 1;
    }
    if s.get(p) == Some(&b'r') {
        let hashes = s[p + 1..].iter().take_while(|&&c| c == b'#').count();
        if s.get(p + 1 + hashes) == Some(&b'"') {
            return (Kind::Str, raw_string(lines, l, p + 2 + hashes, hashes));
        }
    }
    if p > b && s.get(p) == Some(&b'"') {
        return (Kind::Str, string(lines, l, p + 1));
    }
    if s[b] == b'b' && s.get(b + 1) == Some(&b'\'') {
        let (_, end) = char_or_lifetime(s, l, b + 1);
        return (Kind::Char, end);
    }
    let from = if s[b] == b'r' && s.get(b + 1) == Some(&b'#') && s.get(b + 2).is_some_and(|&c| ident_byte(c)) {
        b + 2
    } else {
        b
    };
    let e = from + s[from..].iter().take_while(|&&c| ident_byte(c)).count();
    let text = &lines[l][b..e];
    let kind = if from > b {
        Kind::Ident
    } else if text == "true" || text == "false" {
        Kind::Literal
    } else if KEYWORDS.contains(&text) {
        Kind::Keyword
    } else {
        Kind::Ident
    };
    (kind, (l, e))
}

/// The end of a `/* … */` opened just before (`l`, `b`), nesting as Rust's
/// do; the buffer's end when it never closes.
fn block_comment(lines: &[&str], mut l: usize, mut b: usize) -> (usize, usize) {
    let mut depth = 1;
    while l < lines.len() {
        let s = lines[l].as_bytes();
        while b < s.len() {
            match (s[b], s.get(b + 1)) {
                (b'/', Some(b'*')) => {
                    depth += 1;
                    b += 2;
                }
                (b'*', Some(b'/')) => {
                    depth -= 1;
                    b += 2;
                    if depth == 0 {
                        return (l, b);
                    }
                }
                _ => b += 1,
            }
        }
        l += 1;
        b = 0;
    }
    buffer_end(lines)
}

/// The end of a string whose body starts at (`l`, `b`): just past the
/// first `"` no backslash escapes. A backslash at a line's end continues
/// the string onto the next.
fn string(lines: &[&str], mut l: usize, mut b: usize) -> (usize, usize) {
    while l < lines.len() {
        let s = lines[l].as_bytes();
        while b < s.len() {
            match s[b] {
                b'\\' => b += 2,
                b'"' => return (l, b + 1),
                _ => b += 1,
            }
        }
        l += 1;
        b = 0;
    }
    buffer_end(lines)
}

/// The end of a raw string with `hashes` hashes whose body starts at
/// (`l`, `b`): past the `"` followed by as many `#`.
fn raw_string(lines: &[&str], mut l: usize, mut b: usize, hashes: usize) -> (usize, usize) {
    while l < lines.len() {
        let s = lines[l].as_bytes();
        while b < s.len() {
            if s[b] == b'"' && s.len() >= b + 1 + hashes && s[b + 1..b + 1 + hashes].iter().all(|&c| c == b'#') {
                return (l, b + 1 + hashes);
            }
            b += 1;
        }
        l += 1;
        b = 0;
    }
    buffer_end(lines)
}

/// Does token `t` run to the buffer's end without closing — a string or a
/// `/*` comment still being typed? The point at its end is inside it.
fn unclosed(lines: &[&str], t: &Token) -> bool {
    let last = &lines[t.end.0][..t.end.1];
    let first = &lines[t.start.0][t.start.1..];
    t.end == buffer_end(lines)
        && match t.kind {
            Kind::Comment => first.starts_with("/*") && !(last.ends_with("*/") && (t.end.0 > t.start.0 || last.len() >= t.start.1 + 4)),
            Kind::Str => !last.ends_with(['"', '#']) || (t.end.0 == t.start.0 && t.end.1 - t.start.1 < 2),
            _ => false,
        }
}

fn buffer_end(lines: &[&str]) -> (usize, usize) {
    let last = lines.len().saturating_sub(1);
    (last, lines.get(last).map_or(0, |s| s.len()))
}

/// At a `'`: a char literal when one character (or an escape) and a closing
/// `'` follow, else a lifetime or label when a name does, else a lone `'`.
fn char_or_lifetime(s: &[u8], l: usize, b: usize) -> (Kind, (usize, usize)) {
    if s.get(b + 1) == Some(&b'\\') {
        let close = s[(b + 3).min(s.len())..].iter().position(|&c| c == b'\'');
        return (Kind::Char, (l, close.map_or(s.len(), |k| b + 3 + k + 1)));
    }
    if let Some(&c) = s.get(b + 1) {
        let width = match c {
            0..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        };
        if s.get(b + 1 + width) == Some(&b'\'') {
            return (Kind::Char, (l, b + 2 + width));
        }
        if ident_byte(c) && !c.is_ascii_digit() {
            let e = b + 1 + s[b + 1..].iter().take_while(|&&c| ident_byte(c)).count();
            return (Kind::Lifetime, (l, e));
        }
    }
    (Kind::Punct(b'\''), (l, b + 1))
}

/// The end of a number starting at `b`: digits, a base prefix, `_`, a
/// suffix (`u8`, `f32`), one decimal point followed by a digit (so `1..2`
/// stops at the range), and a signed exponent outside hex.
fn number(s: &[u8], b: usize) -> usize {
    let hex = s.get(b + 1).is_some_and(|&c| c == b'x' || c == b'X') && s[b] == b'0';
    let mut e = b;
    let mut point = false;
    loop {
        e += s[e..].iter().take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_').count();
        let next = s.get(e).copied();
        let digit_after = |k: usize| s.get(e + k).is_some_and(|c| c.is_ascii_digit());
        if !hex && matches!(s[e - 1], b'e' | b'E') && matches!(next, Some(b'+' | b'-')) && digit_after(1) {
            e += 1;
        } else if !point && !hex && next == Some(b'.') && digit_after(1) {
            point = true;
            e += 1;
        } else {
            return e;
        }
    }
}

/// Is token `i` a `#` that opens an attribute — `#[` or `#![`?
fn is_attribute(toks: &[Token], i: usize) -> bool {
    let open_at = |k: usize| toks.get(k).is_some_and(|t| t.kind == Kind::Open);
    match toks.get(i + 1).map(|t| t.kind) {
        Some(Kind::Open) => true,
        Some(Kind::Punct(b'!')) => open_at(i + 2),
        _ => false,
    }
}

/// Is token `p` a `{` or `}`?
fn is_brace(lines: &[&str], p: &Token) -> bool {
    matches!(lines[p.start.0].as_bytes()[p.start.1], b'{' | b'}')
}

/// Do tokens `a` and `b` touch, with nothing between them?
fn adjacent(a: &Token, b: &Token) -> bool {
    a.end == b.start
}

/// Is token `i` a name a `!` follows directly — `println!`, `vec!` — and
/// not one compared with `!=`?
fn is_macro(toks: &[Token], i: usize) -> bool {
    let (Some(t), Some(bang)) = (toks.get(i), toks.get(i + 1)) else { return false };
    t.kind == Kind::Ident
        && bang.kind == Kind::Punct(b'!')
        && adjacent(t, bang)
        && !toks.get(i + 2).is_some_and(|n| n.kind == Kind::Punct(b'=') && adjacent(bang, n))
}

/// Is name token `i` a function called or defined there — `foo(`,
/// `collect::<Vec<_>>(`, a macro's name, or the name after `fn`?
fn is_call(lines: &[&str], toks: &[Token], i: usize) -> bool {
    let word = |t: &Token| &lines[t.start.0][t.start.1..t.end.1];
    if i > 0 && toks[i - 1].kind == Kind::Keyword && word(&toks[i - 1]) == "fn" {
        return true;
    }
    if is_macro(toks, i) {
        return true;
    }
    let mut j = i + 1;
    // A turbofish: `::<…>` between the name and its `(`.
    let punct = |k: usize, c: u8| toks.get(k).is_some_and(|t| t.kind == Kind::Punct(c));
    if punct(j, b':') && punct(j + 1, b':') && punct(j + 2, b'<') {
        let mut depth = 0;
        j += 2;
        while let Some(t) = toks.get(j) {
            match t.kind {
                Kind::Punct(b'<') => depth += 1,
                Kind::Punct(b'>') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Kind::Punct(b';') => return false,
                Kind::Open if is_brace(lines, t) => return false,
                _ => {}
            }
            j += 1;
        }
        j += 1;
    }
    toks.get(j).is_some_and(|t| t.kind == Kind::Open && lines[t.start.0].as_bytes()[t.start.1] == b'(')
}

/// The name before the `(` at token `i`, as written: `foo`, `Vec::new`,
/// `println!`, or a method's own name. None for a tuple or a keyword's
/// parenthesis (`if (…)`).
fn callee(lines: &[&str], toks: &[Token], i: usize) -> Option<String> {
    let word = |k: usize| &lines[toks[k].start.0][toks[k].start.1..toks[k].end.1];
    let punct = |k: usize, c: u8| toks[k].kind == Kind::Punct(c);
    let mut k = i.checked_sub(1)?;
    let bang = punct(k, b'!');
    if bang {
        k = k.checked_sub(1)?;
    }
    // Back over a turbofish to the name it belongs to.
    if !bang && punct(k, b'>') {
        let mut depth = 0;
        loop {
            match toks[k].kind {
                Kind::Punct(b'>') => depth += 1,
                Kind::Punct(b'<') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k = k.checked_sub(1)?;
        }
        k = k.checked_sub(3)?;
        if !(punct(k + 1, b':') && punct(k + 2, b':')) {
            return None;
        }
    }
    if toks[k].kind != Kind::Ident {
        return None;
    }
    let mut name = word(k).to_string();
    // Leading path segments: `Vec::` `std::mem::`, `Self::`.
    while k >= 3 && punct(k - 1, b':') && punct(k - 2, b':') && matches!(toks[k - 3].kind, Kind::Ident | Kind::Keyword) {
        name = format!("{}::{name}", word(k - 3));
        k -= 3;
    }
    if bang {
        name.push('!');
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each highlighted stretch of `src` as (text, class), in order.
    fn classes(src: &str) -> Vec<(String, Tok)> {
        let lines: Vec<&str> = src.lines().collect();
        let scan = RUST.scan(&lines);
        let mut out = Vec::new();
        for (l, spans) in scan.spans.iter().enumerate() {
            for &(s, e, t) in spans {
                out.push((lines[l][s..e].to_string(), t));
            }
        }
        out
    }

    fn of(src: &str, class: Tok) -> Vec<String> {
        classes(src).into_iter().filter(|(_, t)| *t == class).map(|(w, _)| w).collect()
    }

    #[test]
    fn keywords_literals_and_lifetimes() {
        let src = "pub fn get<'a>(x: &'a Self) -> bool { let r#type = true; return false }";
        assert_eq!(of(src, Tok::Keyword), vec!["pub", "fn", "'a", "'a", "Self", "let", "return"]);
        assert_eq!(of(src, Tok::Number), vec!["true", "false"]);
        assert_eq!(of(src, Tok::Call), vec!["get"]);
    }

    #[test]
    fn strings_chars_and_comments_hide_their_brackets() {
        let src = r##"let s = "a \" ( b"; let c = '('; let r = r#"he said "(" "#; // ) not a bracket"##;
        assert_eq!(of(src, Tok::Str), vec![r#""a \" ( b""#, "'('", r##"r#"he said "(" "#"##]);
        assert_eq!(of(src, Tok::Comment), vec!["// ) not a bracket"]);
        assert!(RUST.scan(&[src]).parens.is_empty());
        assert_eq!(of("let b = b'x'; let s = b\"hi\"; let c = '\\n';", Tok::Str), vec!["b'x'", "b\"hi\"", "'\\n'"]);
        // A char is not a lifetime, and a lifetime is not a char.
        assert_eq!(of("fn f<'a>(c: char) { 'outer: loop { break 'outer } }", Tok::Str), Vec::<String>::new());
        assert_eq!(of("let q = 'q';", Tok::Keyword), vec!["let"]);
    }

    #[test]
    fn a_block_comment_nests_and_runs_over_lines() {
        let src = "a /* one /* two */ still\nin it */ b(c)";
        assert_eq!(of(src, Tok::Comment), vec!["/* one /* two */ still", "in it */"]);
        assert_eq!(of(src, Tok::Call), vec!["b"]);
        assert_eq!(RUST.scan(&src.lines().collect::<Vec<_>>()).parens.len(), 2);
    }

    #[test]
    fn a_string_runs_over_lines() {
        let src = "let s = \"one (\ntwo\"; f()";
        assert_eq!(of(src, Tok::Str), vec!["\"one (", "two\""]);
        assert_eq!(RUST.scan(&src.lines().collect::<Vec<_>>()).parens.len(), 2);
        let raw = "let s = r#\"one \"\ntwo\"#; f()";
        assert_eq!(of(raw, Tok::Str), vec!["r#\"one \"", "two\"#"]);
    }

    #[test]
    fn numbers_stop_at_a_range() {
        let src = "for i in 1..20 { x = 1.5e-3 + 0xFF_u8 as f32 + 2f64 + t.0; }";
        assert_eq!(of(src, Tok::Number), vec!["1", "20", "1.5e-3", "0xFF_u8", "2f64", "0"]);
    }

    #[test]
    fn calls_macros_and_attributes() {
        let src = "#[derive(Debug)] fn main() { println!(\"{}\", v.iter().collect::<Vec<_>>()); if a != b {} }";
        assert_eq!(of(src, Tok::Call), vec!["main", "println", "!", "iter", "collect"]);
        assert_eq!(of(src, Tok::Keyword), vec!["#", "derive", "fn", "if"]);
        assert_eq!(of("#![allow(dead_code)]", Tok::Keyword), vec!["#", "!", "allow"]);
    }

    fn indent_after(src: &str) -> usize {
        // Enter at the end of the last line.
        let lines: Vec<&str> = src.split('\n').collect();
        let last = lines.len() - 1;
        RUST.indent(&lines, last, lines[last].len())
    }

    fn reindent(src: &str, line: usize) -> usize {
        let lines: Vec<&str> = src.split('\n').collect();
        RUST.indent(&lines, line, 0)
    }

    #[test]
    fn indent_is_rustfmt_block_style() {
        assert_eq!(indent_after("fn main() {"), 4);
        assert_eq!(indent_after("fn main() {\n    let x = 1;"), 4);
        assert_eq!(indent_after("fn main() {\n    if x {"), 8);
        assert_eq!(indent_after("fn main() {\n    foo(a,"), 8);
        assert_eq!(indent_after("struct S;"), 0);
        // A closing bracket goes back out to its opener's line.
        assert_eq!(reindent("fn main() {\n    if x {\n        y();\n        }", 3), 4);
        assert_eq!(reindent("fn main() {\n    x\n        }", 2), 0);
        // Enter between the braces of `{}` puts the `}` back out.
        let lines = ["fn main() {}"];
        assert_eq!(RUST.indent(&lines, 0, 11), 0);
    }

    #[test]
    fn a_carried_expression_steps_in_once() {
        let chain = "fn main() {\n    let v = xs\n        .iter()\n        .map(|x| {\n            x + 1\n        })\n        .collect();\n    v";
        for (line, want) in [(1, 4), (2, 8), (3, 8), (4, 12), (5, 8), (6, 8), (7, 4)] {
            assert_eq!(reindent(chain, line), want, "line {line}: {:?}", chain.split('\n').nth(line));
        }
        // An attribute's `]` is not a statement left open.
        assert_eq!(indent_after("#[derive(Debug)]"), 0);
        assert_eq!(indent_after("impl S {\n    #[inline]"), 4);
        // A chain off a closure's `})` stays level with it; a let-else's
        // `else` sits with its `let`.
        assert_eq!(reindent("fn f() -> bool {\n    with(|l| {\n        l.x\n    })\n    .unwrap_or(false)", 4), 4);
        assert_eq!(reindent("fn f() {\n    let Some(x) = y.long_call()\n    else {", 2), 4);
        // A where clause.
        assert_eq!(reindent("fn f<T>(t: T)\nwhere\n    T: Clone,\n{", 3), 0);
        assert_eq!(reindent("fn f<T>(t: T)\nwhere\n    T: Clone,\n{", 2), 4);
    }

    #[test]
    fn inside_a_string_nothing_is_added() {
        let src = "let s = \"one\n  two\";";
        assert_eq!(reindent(src, 1), 2, "TAB leaves a string's line as it is");
        let lines: Vec<&str> = src.split('\n').collect();
        assert_eq!(RUST.indent(&lines, 0, 11), 0, "Enter inside a string adds no spaces to it");
        // A block comment keeps its margin.
        assert_eq!(indent_after("fn f() {\n    /* a\n     b"), 5);
    }

    #[test]
    fn the_call_around_the_point() {
        let head = |src: &str| {
            let lines: Vec<&str> = src.split('\n').collect();
            let last = lines.len() - 1;
            RUST.enclosing_head(&lines, last, lines[last].len())
        };
        assert_eq!(head("foo(a, b").as_deref(), Some("foo"));
        assert_eq!(head("let v = Vec::with_capacity(").as_deref(), Some("Vec::with_capacity"));
        assert_eq!(head("std::mem::swap(&mut a, ").as_deref(), Some("std::mem::swap"));
        assert_eq!(head("println!(\"{}\", x").as_deref(), Some("println!"));
        assert_eq!(head("xs.iter().collect::<Vec<_>>(").as_deref(), Some("collect"));
        assert_eq!(head("foo((a, ").as_deref(), Some("foo"), "out through a tuple");
        assert_eq!(head("foo(\"(\", ").as_deref(), Some("foo"), "a string's paren does not count");
        assert_eq!(head("foo(|x| { bar"), None, "not out of a closure's body");
        assert_eq!(head("fn main() { let x = 1"), None);
        assert_eq!(head("if (a"), None);
        assert_eq!(head("Self::new(").as_deref(), Some("Self::new"));
    }

    #[test]
    fn rust_is_a_language_by_name() {
        assert_eq!(super::super::by_name("rust").map(|l| l.name()), Some("rust"));
    }
}

/// The lexer and the indent rule over a tree of Rust:
/// `RUST_CORPUS=~/devel/ray cargo test -p edit --lib rust_corpus -- --ignored --nocapture`.
/// Every file's brackets must balance as the lexer reads them (a string
/// or comment misread shows as a bracket out of place); how often TAB
/// would leave a rustfmt-formatted line where it is, is reported.
#[cfg(test)]
mod corpus {
    use super::*;

    fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name();
            if name == "target" || name.to_string_lossy().starts_with('.') {
                continue;
            }
            if p.is_dir() {
                files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    #[test]
    #[ignore]
    fn rust_corpus() {
        let Ok(root) = std::env::var("RUST_CORPUS") else { return };
        let mut paths = Vec::new();
        files(std::path::Path::new(&root), &mut paths);
        let (mut lines_n, mut agree, mut unbalanced) = (0, 0, Vec::new());
        let mut misses: std::collections::BTreeMap<String, usize> = Default::default();
        let t0 = std::time::Instant::now();
        for p in &paths {
            let Ok(src) = std::fs::read_to_string(p) else { continue };
            let lines: Vec<&str> = src.lines().collect();
            let scan = RUST.scan(&lines);
            let mut stack = Vec::new();
            let ok = scan.parens.iter().all(|q| match q.ch {
                b'(' | b'[' | b'{' => {
                    stack.push(q.ch);
                    true
                }
                c => stack.pop() == Some(match c { b')' => b'(', b']' => b'[', _ => b'{' }),
            }) && stack.is_empty();
            if !ok {
                unbalanced.push(p.display().to_string());
            }
            // TAB on each non-blank line outside strings/comments: indent
            // is O(buffer), so sample files of moderate size only.
            if lines.len() <= 1500 {
                for (i, l) in lines.iter().enumerate() {
                    let t = l.trim_start();
                    if t.is_empty() || t.starts_with("//") || t.starts_with('*') {
                        continue;
                    }
                    lines_n += 1;
                    if RUST.indent(&lines, i, 0) == leading(l) {
                        agree += 1;
                    } else if misses.len() < 40 || misses.contains_key(t) {
                        *misses.entry(format!("{}:{}: {t}", p.display(), i + 1)).or_default() += 1;
                    }
                }
            }
        }
        eprintln!(
            "{} files in {:?}; unbalanced {}: {:?}\nTAB keeps {agree}/{lines_n} lines ({:.1}%)",
            paths.len(),
            t0.elapsed(),
            unbalanced.len(),
            unbalanced,
            100.0 * agree as f64 / lines_n.max(1) as f64
        );
        for m in misses.keys().take(40) {
            eprintln!("  {m}");
        }
        assert!(unbalanced.is_empty());
    }
}
