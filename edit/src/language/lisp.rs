//! **Common Lisp** (docs/sbcl-plan.md, L2): the language of a `.lisp`
//! buffer, whose sends go to SBCL. The scan, the clean line starts and the
//! incremental re-scan are Scheme's ([`super::scheme::scan_tokens`]), but
//! the TOKENS are this file's: scm-text reads Scheme, and Common Lisp's
//! reader differs where it matters to brackets —
//!
//! - `|a (b|` is ONE symbol, spaces, brackets and all; scm-text splits it
//!   at the space and opens a list.
//! - `[`, `]`, `{` and `}` are ordinary constituents, not brackets.
//! - `\` escapes the next character inside a symbol (`foo\ bar`).
//! - `#'f`, `#.x`, `#+feature`, `#-feature`, `#p"…"`, `#c(…)`, `#2A(…)`,
//!   `#S(…)`, `#1=` prefix the datum after them, as `'` does.
//!
//! What it adds to Scheme's colouring: `#:uninterned` and `&key` as
//! keywords, `t` and `nil` as literals, a feature expression (`#+sbcl`) as a
//! keyword, and the name after `#'` as a call. Indentation is Common Lisp's,
//! not Scheme's: a body form (`defun`, `let`, `when`, anything `def…` or
//! `with-…`) indents its body two in; a call's arguments line up under the
//! first one; the definitions of an `flet`, `labels` or `macrolet` are
//! bodies too.
//!
//! What a reader would say beyond the brackets is SBCL's, asked when one is
//! running (L4).

use scm_text::text::{Kind, Token};

use super::scheme::{Dialect, Tokens, rescan_with, scan_tokens};
use super::{Diagnostic, Language, Rescan, Scan, Tok};

pub struct Lisp;

pub static LISP: Lisp = Lisp;

impl Language for Lisp {
    fn name(&self) -> &'static str {
        "lisp"
    }

    fn title(&self) -> &'static str {
        "Lisp"
    }

    fn scan(&self, lines: &[&str]) -> Scan {
        let toks = tokens(&lines.join("\n"));
        scan_tokens(lines, &Tokens::of(lines, Dialect::Lisp, &toks))
    }

    fn rescan(&self, old: &Scan, old_lines: &[String], lines: &[&str]) -> Option<Rescan> {
        rescan_with(self, old, old_lines, lines)
    }

    fn indent(&self, lines: &[&str], line: usize, byte: usize) -> usize {
        let toks = tokens(&lines.join("\n"));
        indent_at(&Tokens::of(lines, Dialect::Lisp, &toks), (line, byte))
    }

    /// The head of the innermost unclosed `(` around the point, as written
    /// (`mapcar`, `cl:mapcar`), when it is on that `(`'s line.
    fn enclosing_head(&self, lines: &[&str], line: usize, byte: usize) -> Option<String> {
        let toks = tokens(&lines.join("\n"));
        let t = Tokens::of(lines, Dialect::Lisp, &toks);
        let open = innermost_open(&t, (line, byte))?;
        let head = open + 1;
        (head < t.placed.len() && t.placed[head].tok.kind == Kind::Atom && t.placed[head].start.0 == t.placed[open].start.0)
            .then(|| t.text(head).to_string())
    }

    fn diagnose(&self, lines: &[&str], scan: &Scan) -> Vec<Diagnostic> {
        super::guile::bracket_marks(lines, scan)
    }
}

/// Characters that end a token (and are not part of one).
fn terminates(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | '"' | '\'' | '`' | ',' | ';')
}

/// TEXT's tokens as Common Lisp's reader would split it, in CHARACTERS —
/// scm-text's [`Token`]s, so the editor's scan and its structural editing
/// take them as they take Scheme's. Unclosed strings, `|…|` and `#|…|#`
/// run to the end, as scm-text's do.
pub fn tokens(text: &str) -> Vec<Token> {
    let c: Vec<char> = text.chars().collect();
    let n = c.len();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut opens: Vec<usize> = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<Token>, kind, start, end, depth| out.push(Token { kind, start, end, depth });
    // A symbol or number from `i`: constituents, with `\` escaping one
    // character and `|…|` escaping a run, to the next terminator.
    let symbol_end = |mut j: usize| -> usize {
        while j < n {
            match c[j] {
                '\\' => j = (j + 2).min(n),
                '|' => {
                    j += 1;
                    while j < n && c[j] != '|' {
                        j += if c[j] == '\\' { 2 } else { 1 };
                    }
                    j = (j + 1).min(n);
                }
                ch if terminates(ch) => break,
                _ => j += 1,
            }
        }
        j
    };
    while i < n {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        match ch {
            ';' => {
                while i < n && c[i] != '\n' {
                    i += 1;
                }
                push(&mut out, Kind::Comment, start, i, depth);
            }
            '"' => {
                i += 1;
                while i < n && c[i] != '"' {
                    i += if c[i] == '\\' { 2 } else { 1 };
                }
                i = (i + 1).min(n);
                push(&mut out, Kind::Str, start, i, depth);
            }
            '(' => {
                push(&mut out, Kind::Open, start, start + 1, depth);
                opens.push(depth);
                depth += 1;
                i += 1;
            }
            ')' => {
                // A closer carries the depth of the `(` it closes.
                let d = opens.pop().unwrap_or(0);
                depth = d;
                push(&mut out, Kind::Close, start, start + 1, d);
                i += 1;
            }
            '\'' | '`' => {
                push(&mut out, Kind::Prefix, start, start + 1, depth);
                i += 1;
            }
            ',' => {
                i += if c.get(i + 1) == Some(&'@') || c.get(i + 1) == Some(&'.') { 2 } else { 1 };
                push(&mut out, Kind::Prefix, start, i, depth);
            }
            '#' => {
                let next = c.get(i + 1).copied();
                match next {
                    Some('|') => {
                        // A block comment, nesting.
                        let mut level = 0usize;
                        let mut j = i;
                        while j < n {
                            if c[j] == '#' && c.get(j + 1) == Some(&'|') {
                                level += 1;
                                j += 2;
                            } else if c[j] == '|' && c.get(j + 1) == Some(&'#') {
                                level -= 1;
                                j += 2;
                                if level == 0 {
                                    break;
                                }
                            } else {
                                j += 1;
                            }
                        }
                        i = j.min(n);
                        push(&mut out, Kind::Comment, start, i, depth);
                    }
                    Some('\\') => {
                        // A character: the one after `#\`, whatever it is,
                        // and, after a letter, the rest of a name (`#\Space`).
                        let mut j = (i + 3).min(n);
                        if i + 2 < n && c[i + 2].is_alphanumeric() {
                            j = symbol_end(j);
                        }
                        i = j;
                        push(&mut out, Kind::Atom, start, i, depth);
                    }
                    Some('(') => {
                        // A vector: `#` prefixes the list.
                        i += 1;
                        push(&mut out, Kind::Prefix, start, i, depth);
                    }
                    Some('\'' | '.' | '+' | '-' | 'p' | 'P' | 'c' | 'C' | 's' | 'S' | 'a' | 'A') => {
                        i += 2;
                        push(&mut out, Kind::Prefix, start, i, depth);
                    }
                    Some(d) if d.is_ascii_digit() => {
                        // `#2A(…)` and `#1=` prefix a datum; `#1#` and
                        // `#16r1F` are atoms.
                        let mut j = i + 1;
                        while j < n && c[j].is_ascii_digit() {
                            j += 1;
                        }
                        match c.get(j) {
                            Some('a' | 'A' | '=') => {
                                i = j + 1;
                                push(&mut out, Kind::Prefix, start, i, depth);
                            }
                            _ => {
                                i = symbol_end(j);
                                push(&mut out, Kind::Atom, start, i, depth);
                            }
                        }
                    }
                    _ => {
                        // `#:sym`, `#x1F`, `#b101`, `#*1011`, and anything
                        // else `#` begins: one atom.
                        i = symbol_end(i + 1);
                        push(&mut out, Kind::Atom, start, i, depth);
                    }
                }
            }
            _ => {
                i = symbol_end(i).max(i + 1);
                push(&mut out, Kind::Atom, start, i, depth);
            }
        }
    }
    out
}

/// A prefix's highlight class: a feature expression's `#+`/`#-` is a
/// keyword; the rest are punctuation.
pub(super) fn prefix_class(word: &str) -> Option<Tok> {
    matches!(word, "#+" | "#-").then_some(Tok::Keyword)
}

/// Whether WORD reads as a number: an integer, a ratio, a decimal with an
/// optional exponent marker (`1.5d0`, `2e10`, `.5`), with a sign.
fn is_number(word: &str) -> bool {
    let w = word.strip_prefix(['+', '-']).unwrap_or(word);
    if w.is_empty() {
        return false;
    }
    if let Some((a, b)) = w.split_once('/') {
        return !a.is_empty() && !b.is_empty() && a.bytes().all(|d| d.is_ascii_digit()) && b.bytes().all(|d| d.is_ascii_digit());
    }
    let (mantissa, exponent) = match w.find(|ch: char| matches!(ch.to_ascii_lowercase(), 'e' | 's' | 'f' | 'd' | 'l')) {
        Some(k) => (&w[..k], Some(&w[k + 1..])),
        None => (w, None),
    };
    let digits = mantissa.bytes().filter(|d| d.is_ascii_digit()).count();
    let mantissa_ok = digits > 0 && mantissa.bytes().all(|d| d.is_ascii_digit() || d == b'.') && mantissa.matches('.').count() <= 1;
    let exponent_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && e.bytes().all(|d| d.is_ascii_digit())
    });
    mantissa_ok && exponent_ok && (exponent.is_none() || mantissa.bytes().any(|d| d.is_ascii_digit()))
}

/// An atom's highlight class, or `None` for plain body text.
pub(super) fn class(t: &Tokens, i: usize) -> Option<Tok> {
    let word = t.text(i);
    let before = (i > 0).then(|| (t.placed[i - 1].tok.kind, t.text(i - 1), t.placed[i - 1].tok.end == t.placed[i].tok.start));
    match before {
        // The name after `#'` is a function's.
        Some((Kind::Prefix, "#'", true)) => return Some(Tok::Call),
        // A feature expression is the `#+`'s.
        Some((Kind::Prefix, "#+" | "#-", true)) => return Some(Tok::Keyword),
        _ => {}
    }
    if word.starts_with("#\\") {
        Some(Tok::Str)
    } else if word.starts_with(':') || word.starts_with("#:") || word.starts_with('&') {
        Some(Tok::Keyword)
    } else if is_number(word) || word.starts_with('#') || word.eq_ignore_ascii_case("t") || word.eq_ignore_ascii_case("nil") {
        Some(Tok::Number)
    } else if t.is_head(i) && !quoted(t, i - 1) {
        Some(Tok::Call)
    } else {
        None
    }
}

/// Whether the `(` at `open` is quoted data: `'(a b)`, `` `(a ,b) ``.
fn quoted(t: &Tokens, open: usize) -> bool {
    open > 0
        && t.placed[open - 1].tok.kind == Kind::Prefix
        && t.placed[open - 1].tok.end == t.placed[open].tok.start
        && matches!(t.text(open - 1), "'" | "`")
}

/// The index of the innermost `(` still open just before `point`.
fn innermost_open(t: &Tokens, point: (usize, usize)) -> Option<usize> {
    let mut stack = Vec::new();
    for (i, p) in t.placed.iter().enumerate() {
        if p.start >= point {
            break;
        }
        match p.tok.kind {
            Kind::Open => stack.push(i),
            Kind::Close => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.pop()
}

/// The open `(` enclosing the one at `open`, if any.
fn enclosing(t: &Tokens, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for j in (0..open).rev() {
        match t.placed[j].tok.kind {
            Kind::Close => depth += 1,
            Kind::Open if depth == 0 => return Some(j),
            Kind::Open => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The head symbol of the form opened at `open`, lower case and without
/// its package, when there is one.
fn head_of(t: &Tokens, open: usize) -> Option<String> {
    let h = t.placed.get(open + 1)?;
    (h.tok.kind == Kind::Atom).then(|| {
        let word = t.text(open + 1);
        word.rsplit(':').next().unwrap_or(word).to_ascii_lowercase()
    })
}

/// A form whose body indents two in, as Emacs's Common Lisp indentation
/// has it: definitions, binding forms, the `with-` macros.
fn body_form(head: &str) -> bool {
    head.starts_with("def") && head != "default"
        || head.starts_with("with-")
        || head.starts_with("do-")
        || matches!(
            head,
            "lambda" | "let" | "let*" | "flet" | "labels" | "macrolet" | "symbol-macrolet" | "when" | "unless"
                | "dolist" | "dotimes" | "do" | "do*" | "case" | "ecase" | "ccase" | "typecase" | "etypecase"
                | "ctypecase" | "handler-case" | "handler-bind" | "restart-case" | "restart-bind"
                | "unwind-protect" | "progn" | "prog1" | "prog2" | "block" | "catch" | "tagbody"
                | "multiple-value-bind" | "destructuring-bind" | "multiple-value-prog1" | "eval-when"
                | "locally" | "ignore-errors" | "print-unreadable-object" | "progv"
        )
}

/// The column a new line at `point` indents to.
fn indent_at(t: &Tokens, point: (usize, usize)) -> usize {
    let Some(open) = innermost_open(t, point) else { return 0 };
    let col = t.placed[open].start.1;
    let line = t.placed[open].start.0;
    let data = col + 1;
    // Quoted, or a `#(` vector: data, each element under the first.
    if open > 0 && t.placed[open - 1].tok.kind == Kind::Prefix && t.placed[open - 1].tok.end == t.placed[open].tok.start && t.text(open - 1) != "#'" {
        return data;
    }
    let Some(head) = head_of(t, open).filter(|_| t.placed[open + 1].start.0 == line) else { return data };
    // A local function's definition, in an flet's list of them: a body.
    if let Some(list) = enclosing(t, open)
        && let Some(form) = enclosing(t, list)
        && form + 2 == list
        && matches!(head_of(t, form).as_deref(), Some("flet" | "labels" | "macrolet"))
    {
        return col + 2;
    }
    if body_form(&head) {
        return col + 2;
    }
    // A call: its arguments line up under the first, when the first is on
    // the head's line.
    let first = open + 2;
    if first < t.placed.len() && t.placed[first].start.0 == line && t.placed[first].start < point && t.placed[first].tok.kind != Kind::Comment {
        return t.placed[first].start.1;
    }
    data
}

// ── What a send reads, and in which package (L3) ───────────────────────────

/// (line, byte) in TEXT as a character offset.
pub fn char_offset(text: &str, line: usize, byte: usize) -> usize {
    let mut off = 0;
    for (l, s) in text.split('\n').enumerate() {
        if l == line {
            return off + s.get(..byte.min(s.len())).map_or(s.chars().count(), |b| b.chars().count());
        }
        off += s.chars().count() + 1;
    }
    off.saturating_sub(1)
}

/// Every top-level form of TEXT, prefixes included (`'(a b)`, and `#+sbcl
/// (f)` with what it guards), as character spans in order. A stray `)` is
/// stepped over.
pub fn toplevel_forms(text: &str) -> Vec<(usize, usize)> {
    let toks = tokens(text);
    let s = crate::sexp::Sexps::with_tokens(text, toks.clone());
    let mut out = Vec::new();
    let mut p = 0;
    loop {
        match s.next_datum(p) {
            Some((a, mut b)) if b > p || a > p => {
                // `#+feature form` is one: the feature, then what it guards.
                let starts = |w: &str| text.chars().skip(a).take(2).collect::<String>() == w;
                if (starts("#+") || starts("#-"))
                    && let Some((_, e)) = s.next_datum(b)
                {
                    b = e;
                }
                out.push((a, b));
                p = b;
            }
            _ => match toks.iter().find(|t| t.start >= p && t.kind == Kind::Close) {
                Some(t) => p = t.end,
                None => return out,
            },
        }
    }
}

/// The top-level form the point (a character offset) is in, else the
/// nearest one before it — what C-M-x sends.
pub fn form_at(text: &str, p: usize) -> Option<(usize, usize)> {
    let all = toplevel_forms(text);
    all.iter().find(|&&(a, b)| a <= p && p <= b).or_else(|| all.iter().rev().find(|&&(_, b)| b <= p)).copied()
}

/// The expression that ends just before the point — what C-x C-e sends.
pub fn form_before(text: &str, p: usize) -> Option<(usize, usize)> {
    crate::sexp::Sexps::with_tokens(text, tokens(text)).prev_datum(p)
}

/// The package a form at the point (a character offset) is read in: the
/// designator of the last top-level `(in-package …)` before it, as
/// `find-package` wants it — `:my-pkg`, `#:my-pkg` and `my-pkg` upper case,
/// `"My-Pkg"` and `|My-Pkg|` as written. None when there is none above.
pub fn package_at(text: &str, p: usize) -> Option<String> {
    let toks = tokens(text);
    let chars: Vec<char> = text.chars().collect();
    let word = |t: &Token| -> String { chars[t.start..t.end].iter().collect() };
    let mut found = None;
    for (i, t) in toks.iter().enumerate() {
        if t.start >= p {
            break;
        }
        if t.kind != Kind::Open || t.depth != 0 {
            continue;
        }
        let (Some(head), Some(arg)) = (toks.get(i + 1), toks.get(i + 2)) else { continue };
        let name = word(head).to_ascii_lowercase();
        if head.kind != Kind::Atom || !matches!(name.as_str(), "in-package" | "cl:in-package" | "common-lisp:in-package") {
            continue;
        }
        let w = word(arg);
        found = match arg.kind {
            Kind::Str => Some(w.trim_matches('"').to_string()),
            Kind::Atom if w.starts_with('|') && w.ends_with('|') && w.len() >= 2 => Some(w[1..w.len() - 1].to_string()),
            Kind::Atom => Some(w.trim_start_matches('#').trim_start_matches(':').to_ascii_uppercase()),
            _ => found,
        };
    }
    found
}

/// Is TEXT whole — every `(` closed, and no string, `|…|` or `#|` comment
/// left open? What the REPL panel asks before it sends a line.
pub fn complete(text: &str) -> bool {
    let toks = tokens(text);
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0i64;
    for t in &toks {
        match t.kind {
            Kind::Open => depth += 1,
            Kind::Close => depth -= 1,
            _ => {}
        }
    }
    let Some(last) = toks.last() else { return true };
    let w: String = chars[last.start..last.end].iter().collect();
    let open_token = match last.kind {
        // A string closes with an unescaped `"`.
        Kind::Str => w.len() < 2 || !w.ends_with('"') || {
            let body = &w[1..w.len() - 1];
            body.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1
        },
        Kind::Comment => w.starts_with("#|") && !w.ends_with("|#"),
        Kind::Atom => w.chars().filter(|&c| c == '|').count() % 2 == 1,
        _ => false,
    };
    depth <= 0 && !open_token
}

/// A symbol's name as M-. matches it: no package prefix, lower case —
/// `shapes::Area` and `area` are one name — and a `|Mixed|` name as written.
pub fn bare_name(word: &str) -> String {
    let w = match word.rfind(':') {
        Some(i) if !word.starts_with('|') => &word[i + 1..],
        _ => word,
    };
    match w.strip_prefix('|').and_then(|w| w.strip_suffix('|')) {
        Some(inner) => inner.to_string(),
        None => w.to_ascii_lowercase(),
    }
}

/// What a top-level form IS, to find it again after edits: `defun area`
/// for a definition (its head and the name it defines, as [`bare_name`]
/// has them), else its text with the white space folded.
pub fn form_key(form: &str) -> String {
    let toks = tokens(form);
    let chars: Vec<char> = form.chars().collect();
    let word = |t: &Token| -> String { chars[t.start..t.end].iter().collect() };
    if let (Some(open), Some(head)) = (toks.first(), toks.get(1))
        && open.kind == Kind::Open
        && head.kind == Kind::Atom
    {
        let h = bare_name(&word(head));
        if h.starts_with("def")
            && let Some(name) = defined_name(&toks, 0, &word)
        {
            return format!("{h} {}", bare_name(&word(name)));
        }
    }
    form.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The name a `(def… NAME …)` form whose `(` is `toks[open]` defines: the
/// atom after the head, or the first of a list there — `(point
/// (:conc-name p-))`, and `(setf x)`'s second.
fn defined_name<'t>(toks: &'t [Token], open: usize, word: &dyn Fn(&Token) -> String) -> Option<&'t Token> {
    match toks.get(open + 2) {
        Some(n) if n.kind == Kind::Atom => Some(n),
        Some(n) if n.kind == Kind::Open => match toks.get(open + 3) {
            Some(s) if s.kind == Kind::Atom && bare_name(&word(s)) == "setf" => toks.get(open + 4),
            other => other,
        }
        .filter(|n| n.kind == Kind::Atom),
        _ => None,
    }
}

/// Where TEXT defines NAME (as [`bare_name`] has it): each top-level form
/// whose head begins `def` — `defun`, `defmacro`, `defvar`, `defclass`,
/// `define-condition`, a `defsomething` of the program's own — naming it,
/// as (line, column, the defining word), the column in characters at the
/// name. `(defstruct (point …))` names `point`, `(defun (setf x) …)` `x`.
pub fn definitions(text: &str, name: &str) -> Vec<(usize, usize, String)> {
    let toks = tokens(text);
    let chars: Vec<char> = text.chars().collect();
    let word = |t: &Token| -> String { chars[t.start..t.end].iter().collect() };
    let mut line_starts = vec![0];
    line_starts.extend(chars.iter().enumerate().filter(|(_, c)| **c == '\n').map(|(i, _)| i + 1));
    let at = |p: usize| {
        let line = line_starts.partition_point(|&s| s <= p) - 1;
        (line, p - line_starts[line])
    };
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if t.kind != Kind::Open || t.depth != 0 {
            continue;
        }
        let Some(head) = toks.get(i + 1).filter(|h| h.kind == Kind::Atom) else { continue };
        let form = bare_name(&word(head));
        if !form.starts_with("def") {
            continue;
        }
        if let Some(n) = defined_name(&toks, i, &word)
            && bare_name(&word(n)) == name
        {
            let (line, column) = at(n.start);
            out.push((line, column, form));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_a_name_is_defined() {
        let src = "(in-package :shapes)\n(defun area (s) s)\n(defstruct (point (:conc-name p-)) x)\n(defmethod shapes::Area ((s point)) 0)\n(defun (setf area) (v s) v)\n(let ((area 1)) (defun inner () area))\n(define-widget area)\n";
        assert_eq!(
            definitions(src, &bare_name("AREA")),
            [(1, 7, "defun".to_string()), (3, 11, "defmethod".to_string()), (4, 13, "defun".to_string()), (6, 15, "define-widget".to_string())],
            "top-level forms only: the defun inside the let is not one"
        );
        assert_eq!(definitions(src, "point"), [(2, 12, "defstruct".to_string())]);
        assert!(definitions(src, &bare_name("|Area|")).is_empty(), "a |Mixed| name is another name");
        assert_eq!(bare_name("|Mixed Case|"), "Mixed Case");
        assert_eq!(bare_name("cl-user::Foo"), "foo");
        assert_eq!(form_key("(defun Area (r)\n  (* r r))"), "defun area");
        assert_eq!(form_key("(defmethod geo::area ((s sq)) 0)"), "defmethod area");
        assert_eq!(form_key("(print\n   (area 2))"), "(print (area 2))");
    }

    fn words(src: &str) -> Vec<(Kind, String)> {
        let chars: Vec<char> = src.chars().collect();
        tokens(src).iter().map(|t| (t.kind, chars[t.start..t.end].iter().collect())).collect()
    }

    /// Common Lisp's reader, where it differs from Scheme's: one token for
    /// `|a (b|`, `[` and `{` as constituents, the `#` dispatches.
    #[test]
    fn tokens_are_common_lisps() {
        use Kind::*;
        let w = |s: &str| words(s).into_iter().map(|(k, t)| format!("{k:?}:{t}")).collect::<Vec<_>>().join(" ");
        assert_eq!(w("(defun |weird (name| () 1)"), "Open:( Atom:defun Atom:|weird (name| Open:( Close:) Atom:1 Close:)");
        assert_eq!(w("foo\\ bar foo[1] {x}"), "Atom:foo\\ bar Atom:foo[1] Atom:{x}");
        assert_eq!(w("(mapcar #'car xs)"), "Open:( Atom:mapcar Prefix:#' Atom:car Atom:xs Close:)");
        assert_eq!(w("#\\Space #\\( #\\) #\\| #\\\" #\\;"), "Atom:#\\Space Atom:#\\( Atom:#\\) Atom:#\\| Atom:#\\\" Atom:#\\;");
        assert_eq!(w("#+sbcl (f) #-(or a b) x"), "Prefix:#+ Atom:sbcl Open:( Atom:f Close:) Prefix:#- Open:( Atom:or Atom:a Atom:b Close:) Atom:x");
        assert_eq!(w("#(1 2) #2A((1)) #c(1 2) #p\"/x\" #.(f) #1=(a . #1#)"),
            "Prefix:# Open:( Atom:1 Atom:2 Close:) Prefix:#2A Open:( Open:( Atom:1 Close:) Close:) Prefix:#c Open:( Atom:1 Atom:2 Close:) Prefix:#p Str:\"/x\" Prefix:#. Open:( Atom:f Close:) Prefix:#1= Open:( Atom:a Atom:. Atom:#1# Close:)");
        assert_eq!(w("#:gensym :key &rest #x1F #*101 1/2 -3.5d0"), "Atom:#:gensym Atom::key Atom:&rest Atom:#x1F Atom:#*101 Atom:1/2 Atom:-3.5d0");
        assert_eq!(w("#| a #| nested |# (( |# (b)"), "Comment:#| a #| nested |# (( |# Open:( Atom:b Close:)");
        assert_eq!(w("`(a ,b ,@c) ; ( no"), "Prefix:` Open:( Atom:a Prefix:, Atom:b Prefix:,@ Atom:c Close:) Comment:; ( no");
        let _ = (Open, Close, Atom, Str, Comment, Prefix);
        // Depths: a closer carries its opener's.
        let d: Vec<usize> = tokens("(a (b) c)").iter().map(|t| t.depth).collect();
        assert_eq!(d, [0, 1, 1, 2, 1, 1, 0]);
    }

    fn classes(src: &str) -> Vec<(String, Tok)> {
        let lines: Vec<&str> = src.split('\n').collect();
        let scan = LISP.scan(&lines);
        scan.spans.iter().zip(&lines).flat_map(|(spans, l)| spans.iter().filter(|s| s.2 != Tok::Paren).map(move |s| (l[s.0..s.1].to_string(), s.2))).collect()
    }

    #[test]
    fn colours_are_common_lisps() {
        let got = classes("(defun f (x &key (n 1)) \"Doc.\" (mapcar #'1+ (list t nil :k #:g #\\a 1/2 2.5d0 #+sbcl x)))");
        let want: Vec<(&str, Tok)> = vec![
            ("defun", Tok::Call),
            // A lambda list's and a binding's first names are heads, as
            // Scheme colours them.
            ("x", Tok::Call),
            ("&key", Tok::Keyword),
            ("n", Tok::Call),
            ("1", Tok::Number),
            ("\"Doc.\"", Tok::Str),
            ("mapcar", Tok::Call),
            ("1+", Tok::Call),
            ("list", Tok::Call),
            ("t", Tok::Number),
            ("nil", Tok::Number),
            (":k", Tok::Keyword),
            ("#:g", Tok::Keyword),
            ("#\\a", Tok::Str),
            ("1/2", Tok::Number),
            ("2.5d0", Tok::Number),
            ("#+", Tok::Keyword),
            ("sbcl", Tok::Keyword),
        ];
        assert_eq!(got, want.into_iter().map(|(w, t)| (w.to_string(), t)).collect::<Vec<_>>());
        // Data, not calls: a vector's, an array's and a quoted list's heads.
        // (A list nested inside data still colours its head: the scan does
        // not follow a quote down.)
        let data = "#(a b) #2A(c) '(d e) `(f ,g)";
        assert!(classes(data).iter().all(|(_, t)| *t != Tok::Call), "{:?}", classes(data));
        // `1+` is a function, `+1` a number.
        assert_eq!(classes("(1+ +1)"), [("1+".to_string(), Tok::Call), ("+1".to_string(), Tok::Number)]);
    }

    fn indent_after(src: &str) -> usize {
        let lines: Vec<&str> = src.split('\n').collect();
        let last = lines.len() - 1;
        LISP.indent(&lines, last, lines[last].len())
    }

    /// A new line at the end of each text indents as Emacs's Common Lisp
    /// mode would put it.
    #[test]
    fn indentation_is_common_lisps() {
        assert_eq!(indent_after(""), 0);
        assert_eq!(indent_after("(defun sq (x)"), 2, "a definition's body");
        assert_eq!(indent_after("(let ((a 1)"), 6, "the next binding, under the first");
        assert_eq!(indent_after("(let ((a 1))"), 2, "the let's body");
        assert_eq!(indent_after("(when (> x 1)"), 2);
        assert_eq!(indent_after("(with-open-file (s path)"), 2);
        assert_eq!(indent_after("(format t \"~a\""), 8, "an argument, under the first");
        assert_eq!(indent_after("(if (zerop n)"), 4, "if lines up its arguments in CL");
        assert_eq!(indent_after("(list"), 1, "nothing after the head: under it");
        assert_eq!(indent_after("'(a b"), 2, "data: under the first element");
        assert_eq!(indent_after("(cond ((zerop n) 1)"), 6);
        assert_eq!(indent_after("(flet ((f (x)"), 9, "a local function's body, two in from its `(f`");
        assert_eq!(indent_after("(labels ((f (x) x)\n         (g (y)"), 11);
        assert_eq!(indent_after("(cl:defun f ()"), 2, "a package prefix does not hide a definition");
        assert_eq!(indent_after("(defun f ()\n  (let ((s \"(\"))"), 4, "a bracket in a string is not one");
        assert_eq!(indent_after("(foo |a (b| ;; (\n"), 5);
    }

    #[test]
    fn the_call_around_the_point() {
        let lines = ["(mapcar #'car", "        (cl:remove-if #'null xs"];
        assert_eq!(LISP.enclosing_head(&lines, 1, 20).as_deref(), Some("cl:remove-if"));
        assert_eq!(LISP.enclosing_head(&lines, 0, 10).as_deref(), Some("mapcar"));
        assert_eq!(LISP.enclosing_head(&["x"], 0, 1), None);
    }

    /// Brackets are marked with no SBCL running, and Common Lisp's syntax is
    /// not an error.
    #[test]
    fn brackets_are_marked_and_cl_syntax_is_not() {
        use crate::language::Severity;
        let marks = |src: &str| {
            let lines: Vec<&str> = src.split('\n').collect();
            LISP.diagnose(&lines, &LISP.scan(&lines)).into_iter().map(|d| (d.start, d.severity)).collect::<Vec<_>>()
        };
        assert!(marks("(defun |a (b| (x) #\\( \"(\" ; (\n  #| ( |# (list [x] {y} #'car))").is_empty());
        assert_eq!(marks("(f 1))"), [((0, 5), Severity::Error)]);
        assert_eq!(marks("(ok)\n(defun f (x)\n  (g x"), [((1, 0), Severity::Open)]);
    }

    /// A re-scan after an edit is a full scan of the edited text: the same
    /// shared re-scan as Scheme's, over Common Lisp's tokens.
    #[test]
    fn rescan_equals_a_full_scan() {
        let base = "(defun f (x)\n  \"a string\n   over\" ; c\n  (g x #\\( '|a (b|))\n#| block\n (no) |#\n(h #(1 2) #'car)\n";
        let edits = ["|", "\"", "#|", "|#", "(", ")", "\n", ";", "#\\(", "x"];
        let mut text = base.repeat(3);
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        for _ in 0..800 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let old = text.clone();
            let chars: Vec<char> = text.chars().collect();
            let at = (seed % (chars.len() as u64 + 1)) as usize;
            let piece = edits[(seed >> 20) as usize % edits.len()];
            text = chars[..at].iter().chain(piece.chars().collect::<Vec<_>>().iter()).chain(chars[at..].iter()).collect();
            let old_lines: Vec<String> = old.split('\n').map(str::to_string).collect();
            let old_refs: Vec<&str> = old_lines.iter().map(String::as_str).collect();
            let new_lines: Vec<&str> = text.split('\n').collect();
            let full = LISP.scan(&new_lines);
            if let Some(r) = LISP.rescan(&LISP.scan(&old_refs), &old_lines, &new_lines) {
                assert_eq!(r.scan, full, "after inserting {piece:?} at {at}");
            }
        }
    }

    /// Structural editing goes by Common Lisp's tokens: `|a (b|` is one
    /// symbol, and `#'f` and `#(…)` move as one expression with their prefix.
    #[test]
    fn expressions_are_common_lisps() {
        let text = "(f |a (b| #'g #(1 2)) next";
        let s = crate::sexp::Sexps::with_tokens(text, tokens(text));
        let pos = |w: &str| text.find(w).unwrap();
        assert_eq!(s.forward(0), Some(pos(" next")), "over the whole form");
        assert_eq!(s.forward(pos("|a")), Some(pos(" #'g")), "a |…| symbol is one");
        assert_eq!(s.forward(pos("#'g")), Some(pos(" #(")), "#' with its name");
        assert_eq!(s.forward(pos("#(")), Some(pos(") next")), "a vector with its #");
    }

    /// What C-M-x and C-x C-e send, and in which package.
    #[test]
    fn what_a_send_reads() {
        let text = "(in-package :foo)\n(defun f (x) |a (b| x) ; c\n'(1 2) #+sbcl (g)\n(in-package \"Bar\")\n(h)";
        let at = |w: &str| text.find(w).unwrap();
        let forms: Vec<&str> = toplevel_forms(text).iter().map(|&(a, b)| &text[a..b]).collect();
        assert_eq!(forms, ["(in-package :foo)", "(defun f (x) |a (b| x)", "'(1 2)", "#+sbcl (g)", "(in-package \"Bar\")", "(h)"]);
        let span = |p| form_at(text, p).map(|(a, b)| &text[a..b]);
        assert_eq!(span(at("x) |a")), Some("(defun f (x) |a (b| x)"), "from inside it");
        assert_eq!(span(at(" ; c")), Some("(defun f (x) |a (b| x)"), "the nearest before");
        let before = |p| form_before(text, p).map(|(a, b)| &text[a..b]);
        assert_eq!(before(at(" x) ;") ), Some("|a (b|"));
        assert_eq!(before(at(" #+sbcl")), Some("'(1 2)"), "with its quote");
        assert_eq!(package_at(text, at("(defun")).as_deref(), Some("FOO"));
        assert_eq!(package_at(text, at("(h)")).as_deref(), Some("Bar"));
        assert_eq!(package_at(text, 0), None);
        assert_eq!(package_at("(in-package #:my-pkg)\nx", 22).as_deref(), Some("MY-PKG"));
        assert_eq!(char_offset("ab\nλx", 1, 2), 4, "bytes to characters");
    }

    #[test]
    fn whole_text() {
        for yes in ["(+ 1 2)", "x", "", "(f \"a)\")", "'|a b|", "#| c |# 1", "(f) ; (", "\"a\\\\\""] {
            assert!(complete(yes), "{yes}");
        }
        for no in ["(+ 1", "\"abc", "|a b", "#| c", "(f \"a)", "\"a\\\""] {
            assert!(!complete(no), "{no}");
        }
    }

    #[test]
    fn lisp_is_a_language_by_name() {
        assert_eq!(super::super::by_name("lisp").map(|l| l.title()), Some("Lisp"));
    }

    #[test]
    fn numbers() {
        for yes in ["1", "-2", "+3", "1/2", "1.5", ".5", "2e10", "1.5d0", "-3.5D-2", "7."] {
            assert!(is_number(yes), "{yes}");
        }
        for no in ["1+", "1-", "+", "-", "e5", "a1", "1/", "/2", "1.2.3", "1e", ".", "..."] {
            assert!(!is_number(no), "{no}");
        }
    }
}
