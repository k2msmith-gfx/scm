//! **Structural editing for Lisp buffers** (Kevin, 2026-11-16): moving,
//! selecting and reshaping by EXPRESSION rather than by character — Emacs's
//! `C-M-` sexp commands, paredit's slurp/barf/wrap/splice/raise, pairing
//! brackets as they are typed, and comment toggling.
//!
//! Pure: every question is asked of the text and a point, both in
//! CHARACTERS (scm-text's unit), and every edit comes back as one
//! [`Change`] — a range replaced — which the editor turns into bytes and
//! applies as a single undo step. The structure is `scm-text`'s token scan,
//! the one the editor highlights and indents from, so a string or a comment
//! is never mistaken for code here either, and half-typed text degrades
//! instead of failing.

use scm_text::scope;
use scm_text::text::{Doc, Kind, Token};

/// Replace `start..end` (chars) with `text`; the cursor lands at `point`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub point: usize,
}

/// The text, scanned once, with each bracket's partner.
pub struct Sexps {
    chars: Vec<char>,
    toks: Vec<Token>,
    /// For each token, the index of its partner bracket (Open ↔ Close).
    partner: Vec<Option<usize>>,
}

/// The closer an opener wants (`#(` and `#u8(` close with `)`).
fn closer(open: char) -> char {
    match open {
        '[' => ']',
        '{' => '}',
        _ => ')',
    }
}

impl Sexps {
    pub fn new(text: &str) -> Sexps {
        Sexps::with_tokens(text, Doc::new(text).tokens().to_vec())
    }

    /// The same over tokens a language other than Scheme split the text
    /// into — Common Lisp's (`language::lisp::tokens`), whose `|a (b|` is
    /// one symbol.
    pub fn with_tokens(text: &str, toks: Vec<Token>) -> Sexps {
        let mut partner = vec![None; toks.len()];
        let mut stack = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            match t.kind {
                Kind::Open => stack.push(i),
                Kind::Close => {
                    if let Some(o) = stack.pop() {
                        partner[o] = Some(i);
                        partner[i] = Some(o);
                    }
                }
                _ => {}
            }
        }
        Sexps { chars: text.chars().collect(), toks, partner }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    fn slice(&self, s: usize, e: usize) -> String {
        self.chars[s.min(self.len())..e.min(self.len())].iter().collect()
    }

    fn char_at(&self, p: usize) -> Option<char> {
        self.chars.get(p).copied()
    }

    /// The token covering `p` strictly inside it (`start < p < end`).
    fn inside(&self, p: usize) -> Option<usize> {
        self.toks.iter().position(|t| t.start < p && p < t.end)
    }

    /// Is `p` in a string or a comment, where typing is just typing? A line
    /// comment still holds the point at its end, before the newline.
    pub fn in_text(&self, p: usize) -> bool {
        self.toks.iter().any(|t| match t.kind {
            Kind::Str => t.start < p && p < t.end,
            Kind::Comment => t.start < p && p <= t.end,
            _ => false,
        })
    }

    /// The datum that starts at token `i` (after any prefixes), as
    /// `(start, end)`; None for a closer or a comment.
    fn datum_from(&self, i: usize) -> Option<(usize, usize)> {
        let t = self.toks[i];
        match t.kind {
            Kind::Atom | Kind::Str => Some((t.start, t.end)),
            Kind::Open => Some((t.start, self.partner[i].map_or(self.len(), |c| self.toks[c].end))),
            Kind::Prefix => {
                let next = (i + 1..self.toks.len()).find(|&j| self.toks[j].kind != Kind::Comment)?;
                self.datum_from(next).map(|(_, e)| (t.start, e))
            }
            Kind::Close | Kind::Comment => None,
        }
    }

    /// Extend a datum starting at token `i` back over the prefixes that
    /// quote or comment it out.
    fn with_prefixes(&self, mut i: usize, end: usize) -> (usize, usize) {
        while i > 0 && self.toks[i - 1].kind == Kind::Prefix {
            i -= 1;
        }
        (self.toks[i].start, end)
    }

    /// The datum after `p` at `p`'s own level, or the atom `p` is in.
    pub fn next_datum(&self, p: usize) -> Option<(usize, usize)> {
        if let Some(i) = self.inside(p) {
            let t = self.toks[i];
            if matches!(t.kind, Kind::Atom | Kind::Str) {
                return Some((t.start, t.end));
            }
        }
        for (i, t) in self.toks.iter().enumerate() {
            if t.start < p {
                continue;
            }
            match t.kind {
                Kind::Comment => continue,
                Kind::Close => return None,
                _ => return self.datum_from(i),
            }
        }
        None
    }

    /// The datum before `p` at `p`'s own level, or the atom `p` is in.
    pub fn prev_datum(&self, p: usize) -> Option<(usize, usize)> {
        if let Some(i) = self.inside(p) {
            let t = self.toks[i];
            if matches!(t.kind, Kind::Atom | Kind::Str) {
                return Some(self.with_prefixes(i, t.end));
            }
        }
        for i in (0..self.toks.len()).rev() {
            let t = self.toks[i];
            if t.end > p {
                continue;
            }
            match t.kind {
                Kind::Comment | Kind::Prefix => continue,
                Kind::Open => return None,
                Kind::Close => {
                    let o = self.partner[i]?;
                    return Some(self.with_prefixes(o, t.end));
                }
                Kind::Atom | Kind::Str => return Some(self.with_prefixes(i, t.end)),
            }
        }
        None
    }

    /// The innermost list around `p`: its opener's and closer's token
    /// indices (the closer None while it is unwritten).
    fn enclosing(&self, p: usize) -> Option<(usize, Option<usize>)> {
        let mut stack = Vec::new();
        for (i, t) in self.toks.iter().enumerate() {
            if t.start >= p {
                break;
            }
            match t.kind {
                Kind::Open if t.end <= p => stack.push(i),
                Kind::Close if t.end <= p => {
                    stack.pop();
                }
                _ => {}
            }
        }
        let o = *stack.last()?;
        Some((o, self.partner[o]))
    }

    /// The list around `p`, both brackets written: `(open, close)` tokens.
    fn list(&self, p: usize) -> Option<(Token, Token)> {
        let (o, c) = self.enclosing(p)?;
        Some((self.toks[o], self.toks[c?]))
    }

    // ── Motion ────────────────────────────────────────────────────────────

    /// C-M-f: past the next expression.
    pub fn forward(&self, p: usize) -> Option<usize> {
        self.next_datum(p).map(|(_, e)| e)
    }

    /// C-M-b: to the start of the previous expression.
    pub fn backward(&self, p: usize) -> Option<usize> {
        self.prev_datum(p).map(|(s, _)| s)
    }

    /// C-M-u: to the opening bracket of the list around the point.
    pub fn up(&self, p: usize) -> Option<usize> {
        self.enclosing(p).map(|(o, _)| self.toks[o].start)
    }

    /// Out past the closing bracket of the list around the point.
    pub fn up_forward(&self, p: usize) -> Option<usize> {
        self.list(p).map(|(_, c)| c.end)
    }

    /// C-M-d: into the next list.
    pub fn down(&self, p: usize) -> Option<usize> {
        self.toks
            .iter()
            .filter(|t| t.start >= p)
            .take_while(|t| t.kind != Kind::Close)
            .find(|t| t.kind == Kind::Open)
            .map(|t| t.end)
    }

    /// vi's `%`: on a bracket, to its partner; elsewhere, to the partner of
    /// the next bracket on the line.
    pub fn matching(&self, p: usize) -> Option<usize> {
        let line_end = (p..self.len()).find(|&i| self.chars[i] == '\n').unwrap_or(self.len());
        let i = self.toks.iter().position(|t| {
            matches!(t.kind, Kind::Open | Kind::Close) && t.end > p && t.end - 1 < line_end.max(p + 1)
        })?;
        let j = self.partner[i]?;
        let t = self.toks[j];
        // An opener's bracket is its LAST char (`#(`), a closer's its only one.
        Some(if t.kind == Kind::Open { t.end - 1 } else { t.start })
    }

    // ── Edits ─────────────────────────────────────────────────────────────

    /// C-M-k: delete from the point to the end of the next expression.
    pub fn kill_forward(&self, p: usize) -> Option<Change> {
        let (_, e) = self.next_datum(p)?;
        Some(Change { start: p, end: e, text: String::new(), point: p })
    }

    /// C-M-backspace: delete from the previous expression's start to the point.
    pub fn kill_backward(&self, p: usize) -> Option<Change> {
        let (s, _) = self.prev_datum(p)?;
        Some(Change { start: s, end: p, text: String::new(), point: s })
    }

    /// C-M-t: swap the expressions either side of the point.
    pub fn transpose(&self, p: usize) -> Option<Change> {
        let (a0, a1) = self.prev_datum(p)?;
        let (b0, b1) = self.next_datum(p)?;
        if b0 < a1 {
            return None;
        }
        let text = format!("{}{}{}", self.slice(b0, b1), self.slice(a1, b0), self.slice(a0, a1));
        Some(Change { start: a0, end: b1, text, point: b1 })
    }

    /// C-): the list around the point takes in the expression after it.
    pub fn slurp_forward(&self, p: usize) -> Option<Change> {
        let (open, close) = self.list(p)?;
        let (_, e) = self.next_datum(close.end)?;
        let mut moved = self.slice(close.end, e);
        if open.end == close.start {
            moved = moved.trim_start().to_string();
        }
        let text = format!("{moved}{}", self.slice(close.start, close.end));
        Some(Change { start: close.start, end: e, text, point: p })
    }

    /// C-}: the list around the point lets its last expression out.
    pub fn slurp_backward(&self, p: usize) -> Option<Change> {
        let (open, _) = self.list(p)?;
        let (s, _) = self.prev_datum(open.start)?;
        let mut moved = self.slice(s, open.start);
        if self.list(p).is_some_and(|(o, c)| o.end == c.start) {
            moved = moved.trim_end().to_string();
        }
        let text = format!("{}{moved}", self.slice(open.start, open.end));
        // Everything after the opener shifts by how much the span changed.
        let point = (p + text.chars().count()).saturating_sub(open.end - s);
        Some(Change { start: s, end: open.end, text, point })
    }

    pub fn barf_forward(&self, p: usize) -> Option<Change> {
        let (open, close) = self.list(p)?;
        let (last, _) = self.prev_datum(close.start).filter(|&(s, _)| s >= open.end)?;
        let anchor = self.prev_datum(last).filter(|&(s, _)| s >= open.end).map_or(open.end, |(_, e)| e);
        let mut moved = self.slice(anchor, close.start);
        if !moved.starts_with(char::is_whitespace) {
            moved.insert(0, ' ');
        }
        let closing = self.slice(close.start, close.end);
        let text = format!("{closing}{moved}");
        let grown = text.chars().count() - (close.end - anchor);
        let point = if p > anchor { (p + 1 + grown).min(anchor + text.chars().count()) } else { p };
        Some(Change { start: anchor, end: close.end, text, point })
    }

    /// C-{: the list around the point lets its first expression out.
    pub fn barf_backward(&self, p: usize) -> Option<Change> {
        let (open, close) = self.list(p)?;
        let (_, first_end) = self.next_datum(open.end).filter(|&(_, e)| e <= close.start)?;
        let anchor = self.next_datum(first_end).filter(|&(_, e)| e <= close.start).map_or(close.start, |(s, _)| s);
        let mut moved = self.slice(open.end, anchor);
        if !moved.ends_with(char::is_whitespace) {
            moved.push(' ');
        }
        let opening = self.slice(open.start, open.end);
        let text = format!("{moved}{opening}");
        let point = if p >= anchor { p + text.chars().count() - (anchor - open.start) } else { p.saturating_sub(open.end - open.start).max(open.start) };
        Some(Change { start: open.start, end: anchor, text, point })
    }

    /// M-(: wrap the expression at the point in a list (an empty one where
    /// there is none), the point just inside.
    pub fn wrap(&self, p: usize) -> Change {
        match self.next_datum(p) {
            Some((s, e)) => Change { start: s, end: e, text: format!("({})", self.slice(s, e)), point: s + 1 },
            None => Change { start: p, end: p, text: "()".into(), point: p + 1 },
        }
    }

    /// M-s: remove the brackets of the list around the point.
    pub fn splice(&self, p: usize) -> Option<Change> {
        let (open, close) = self.list(p)?;
        let text = self.slice(open.end, close.start);
        Some(Change { start: open.start, end: close.end, text, point: p - (open.end - open.start) })
    }

    /// M-r: replace the list around the point with the expression at it.
    pub fn raise(&self, p: usize) -> Option<Change> {
        let (s, e) = self.next_datum(p)?;
        let (open, close) = self.list(s.max(p))?;
        if s < open.end || e > close.start {
            return None;
        }
        Some(Change { start: open.start, end: close.end, text: self.slice(s, e), point: open.start })
    }

    /// Comment the expression at the point out with `#;`, or back in.
    pub fn toggle_datum_comment(&self, p: usize) -> Option<Change> {
        let (s, e) = self.next_datum(p).or_else(|| self.prev_datum(p))?;
        // The point already past a `#;`: the comment is the token before.
        if let Some(t) = self.toks.iter().rev().find(|t| t.end <= s && t.kind != Kind::Comment)
            && t.kind == Kind::Prefix
            && self.slice(t.start, t.end) == "#;"
            && self.slice(t.end, s).trim().is_empty()
        {
            return Some(Change { start: t.start, end: s, text: String::new(), point: p - (s - t.start) });
        }
        if self.slice(s, s + 2) == "#;" {
            let rest = self.slice(s + 2, e);
            let trimmed = rest.trim_start();
            let gone = 2 + rest.chars().count() - trimmed.chars().count();
            return Some(Change { start: s, end: e, text: trimmed.to_string(), point: p.saturating_sub(gone).max(s) });
        }
        Some(Change { start: s, end: s, text: "#;".into(), point: p + 2 })
    }

    // ── Typing ────────────────────────────────────────────────────────────

    /// A character typed at `p`, kept balanced: an opener brings its closer
    /// with it, a closer steps out past the list's own, a quote brings its
    /// partner or steps over the string's end. None means "just type it" —
    /// inside a string or a comment, after `#\`, or anything else.
    pub fn type_char(&self, p: usize, ch: char) -> Option<Change> {
        let after_char_prefix = p >= 2 && self.slice(p - 2, p) == "#\\";
        let in_atom = self.inside(p).is_some_and(|i| self.toks[i].kind == Kind::Atom);
        match ch {
            '(' | '[' | '{' if !self.in_text(p) && !after_char_prefix && !in_atom => {
                Some(Change { start: p, end: p, text: format!("{ch}{}", closer(ch)), point: p + 1 })
            }
            ')' | ']' | '}' if !self.in_text(p) && !after_char_prefix => {
                // Out past the list's own closer, dropping blank space left
                // before it; an unclosed list or the top level types it.
                let (open, close) = self.list(p)?;
                let _ = open;
                if self.slice(p, close.start).trim().is_empty() {
                    Some(Change { start: p, end: close.start, text: String::new(), point: p + close.end - close.start })
                } else {
                    Some(Change { start: p, end: p, text: String::new(), point: close.end })
                }
            }
            '"' => {
                if self.toks.iter().any(|t| t.kind == Kind::Comment && t.start < p && p <= t.end) || after_char_prefix {
                    return None;
                }
                match self.toks.iter().find(|t| t.kind == Kind::Str && t.start < p && p < t.end) {
                    // At the string's closing quote: step over it.
                    Some(t) if p + 1 == t.end && self.char_at(p) == Some('"') => {
                        Some(Change { start: p, end: p, text: String::new(), point: p + 1 })
                    }
                    Some(_) => Some(Change { start: p, end: p, text: "\\\"".into(), point: p + 2 }),
                    None => Some(Change { start: p, end: p, text: "\"\"".into(), point: p + 1 }),
                }
            }
            _ => None,
        }
    }

    /// Backspace at `p`, kept balanced: an empty pair goes as one, a
    /// bracket or quote with something between it and its partner is
    /// stepped over rather than deleted. None means an ordinary backspace.
    pub fn backspace(&self, p: usize) -> Option<Change> {
        let c = self.char_at(p.checked_sub(1)?)?;
        let step = Change { start: p, end: p, text: String::new(), point: p - 1 };
        let tok = self.toks.iter().position(|t| t.end == p && matches!(t.kind, Kind::Open | Kind::Close | Kind::Str));
        match (c, tok) {
            ('(' | '[' | '{', Some(i)) if self.toks[i].kind == Kind::Open => match self.partner[i] {
                Some(j) if self.toks[j].start == p => {
                    Some(Change { start: self.toks[i].start, end: self.toks[j].end, text: String::new(), point: self.toks[i].start })
                }
                Some(_) => Some(step),
                None => None,
            },
            (')' | ']' | '}', Some(i)) if self.toks[i].kind == Kind::Close && self.partner[i].is_some() => Some(step),
            // The closing quote of a string: into it.
            ('"', Some(i)) if self.toks[i].kind == Kind::Str && self.toks[i].end - self.toks[i].start >= 2 => Some(step),
            ('"', None) => {
                // The opening quote: an empty string goes whole, else stay.
                let t = self.toks.iter().find(|t| t.kind == Kind::Str && t.start == p - 1)?;
                if t.end == p + 1 && self.char_at(p) == Some('"') {
                    Some(Change { start: p - 1, end: p + 1, text: String::new(), point: p - 1 })
                } else {
                    Some(step)
                }
            }
            _ => None,
        }
    }

    /// Delete forward at `p`, balanced the same way.
    pub fn delete_forward(&self, p: usize) -> Option<Change> {
        let c = self.char_at(p)?;
        let step = Change { start: p, end: p, text: String::new(), point: p + 1 };
        let tok = self.toks.iter().position(|t| matches!(t.kind, Kind::Open | Kind::Close) && t.end == p + 1);
        match (c, tok) {
            ('(' | '[' | '{', Some(i)) if self.toks[i].kind == Kind::Open => match self.partner[i] {
                Some(j) if self.toks[j].start == p + 1 => {
                    Some(Change { start: self.toks[i].start, end: self.toks[j].end, text: String::new(), point: self.toks[i].start })
                }
                Some(_) => Some(step),
                None => None,
            },
            (')' | ']' | '}', Some(i)) if self.partner[i].is_some() => Some(step),
            ('"', _) if !self.in_text(p) || self.toks.iter().any(|t| t.kind == Kind::Str && t.end == p + 1) => {
                match self.toks.iter().find(|t| t.kind == Kind::Str && (t.start == p || t.end == p + 1)) {
                    Some(t) if t.start == p && t.end == p + 2 => {
                        Some(Change { start: p, end: p + 2, text: String::new(), point: p })
                    }
                    Some(_) => Some(step),
                    None => None,
                }
            }
            _ => None,
        }
    }
}

impl Sexps {
    /// The form a macro expansion is asked about: the list starting at the
    /// point, else the innermost list around it.
    pub fn form_at(&self, p: usize) -> Option<(usize, usize)> {
        if let Some((s, e)) = self.next_datum(p)
            && s == p
            && self.toks.iter().any(|t| t.start == s && matches!(t.kind, Kind::Open | Kind::Prefix))
        {
            return Some((s, e));
        }
        let (open, close) = self.list(p)?;
        Some((open.start, close.end))
    }

    /// The top-level form the point is in or at the start of — what C-M-q
    /// re-indents.
    pub fn toplevel_at(&self, p: usize) -> Option<(usize, usize)> {
        let mut at = p;
        let mut outer = None;
        while let Some((open, close)) = self.list(at) {
            outer = Some((open.start, close.end));
            at = open.start;
        }
        outer.or_else(|| self.next_datum(p).filter(|&(s, _)| !self.slice(p, s).contains('\n')))
    }

    /// The line numbers (0-based) whose first character is inside a string
    /// or a block comment: their indentation is the text's, not the code's.
    pub fn lines_in_text(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut line = 0;
        for (i, c) in self.chars.iter().enumerate() {
            if *c == '\n' {
                line += 1;
                let start = i + 1;
                if self.toks.iter().any(|t| {
                    (t.kind == Kind::Str || (t.kind == Kind::Comment && self.slice(t.start, t.start + 2) == "#|"))
                        && t.start < start
                        && start < t.end
                }) {
                    out.push(line);
                }
            }
        }
        out
    }
}

// ── Pretty-printing (an expansion comes back on one line) ──────────────────

enum Node {
    Atom(String),
    /// Opener text (`(`, `#(`, `[`), the elements, the closer.
    List(String, Vec<Node>, String),
    Prefixed(String, Box<Node>),
}

impl Node {
    fn flat(&self) -> String {
        match self {
            Node::Atom(a) => a.clone(),
            Node::List(o, xs, c) => format!("{o}{}{c}", xs.iter().map(Node::flat).collect::<Vec<_>>().join(" ")),
            Node::Prefixed(p, x) => format!("{p}{}", x.flat()),
        }
    }

    fn head(&self) -> Option<&str> {
        match self {
            Node::List(_, xs, _) => match xs.first() {
                Some(Node::Atom(a)) => Some(a),
                _ => None,
            },
            _ => None,
        }
    }
}

fn nodes(text: &str) -> Vec<Node> {
    let doc = Doc::new(text);
    let chars: Vec<char> = text.chars().collect();
    let slice = |t: &Token| chars[t.start..t.end].iter().collect::<String>();
    let toks: Vec<Token> = doc.tokens().iter().filter(|t| t.kind != Kind::Comment).copied().collect();
    fn one(toks: &[Token], i: &mut usize, slice: &dyn Fn(&Token) -> String) -> Option<Node> {
        let t = toks.get(*i)?;
        *i += 1;
        Some(match t.kind {
            Kind::Open => {
                let mut xs = Vec::new();
                while *i < toks.len() && toks[*i].kind != Kind::Close {
                    if let Some(n) = one(toks, i, slice) {
                        xs.push(n);
                    }
                }
                let close = toks.get(*i).map_or(")".to_string(), slice);
                *i += 1;
                Node::List(slice(t), xs, close)
            }
            Kind::Prefix => Node::Prefixed(slice(t), Box::new(one(toks, i, slice).unwrap_or(Node::Atom(String::new())))),
            Kind::Close => return None,
            _ => Node::Atom(slice(t)),
        })
    }
    let mut i = 0;
    let mut out = Vec::new();
    while i < toks.len() {
        if let Some(n) = one(&toks, &mut i, &slice) {
            out.push(n);
        }
    }
    out
}

/// Forms whose body indents by two under the head rather than lining up
/// with the first argument — how they are written by hand.
fn body_form(head: &str) -> bool {
    matches!(
        head,
        "define" | "define*" | "define-syntax" | "define-record-type" | "lambda" | "lambda*" | "let" | "let*"
            | "letrec" | "letrec*" | "let-values" | "let*-values" | "when" | "unless" | "do" | "case" | "begin"
            | "syntax-rules" | "guard" | "parameterize" | "match" | "if-let" | "when-let" | "with-exception-handler"
    )
}

fn render(n: &Node, col: usize, width: usize, out: &mut String) {
    let flat = n.flat();
    if col + flat.chars().count() <= width {
        out.push_str(&flat);
        return;
    }
    match n {
        Node::Atom(a) => out.push_str(a),
        Node::Prefixed(p, x) => {
            out.push_str(p);
            render(x, col + p.chars().count(), width, out);
        }
        Node::List(o, xs, c) => {
            out.push_str(o);
            let inner = col + o.chars().count();
            let Some((first, rest)) = xs.split_first() else {
                out.push_str(c);
                return;
            };
            render(first, inner, width, out);
            // A head and its first argument share the line; the rest line up
            // under that argument, or go two in for a body form. A list of
            // lists (a let's bindings) lines up under its first.
            let (same_line, under) = match n.head() {
                Some(h) if body_form(h) => (1, col + 2),
                Some(h) if rest.len() > 1 => (1, inner + h.chars().count() + 1),
                _ => (0, inner),
            };
            let mut at = inner + first.flat().chars().count();
            for (k, x) in rest.iter().enumerate() {
                if k < same_line {
                    out.push(' ');
                    render(x, at + 1, width, out);
                    at = col + width; // whatever follows breaks
                } else {
                    out.push('\n');
                    out.push_str(&" ".repeat(under));
                    render(x, under, width, out);
                }
            }
            let _ = at;
            out.push_str(c);
        }
    }
}

/// TEXT's forms laid out to fit WIDTH columns, Lisp style: a form that fits
/// on its line stays there; one that does not breaks after its head and
/// first argument.
pub fn pretty(text: &str, width: usize) -> String {
    let mut out = String::new();
    for (k, n) in nodes(text).iter().enumerate() {
        if k > 0 {
            out.push('\n');
        }
        render(n, 0, width, &mut out);
    }
    out
}

// ── Rename ─────────────────────────────────────────────────────────────────

/// What renaming the name at the point would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub name: String,
    /// A top-level name — its definitions and every free use, which other
    /// files may share — rather than a local bound by one form here.
    pub global: bool,
    /// The occurrences, as char ranges, in order.
    pub sites: Vec<(usize, usize)>,
}

/// Is the occurrence at `start` a FREE use when `sub` (which begins at
/// `base`) is read alone?
fn free_in(text: &[char], base: usize, end: usize, start: usize) -> bool {
    let sub: String = text[base..end].iter().collect();
    scope::references(&Doc::new(&sub)).iter().any(|r| r.start + base == start)
}

/// Every top-level definition of NAME and every free use of it in TEXT.
pub fn global_sites(text: &str, name: &str) -> Vec<(usize, usize)> {
    let doc = Doc::new(text);
    let toplevel = |at: usize| doc.tokens().iter().any(|t| t.start <= at && at < t.end && t.depth <= 1);
    let mut sites: Vec<(usize, usize)> = scope::references(&doc)
        .into_iter()
        .filter(|r| r.name == name)
        .map(|r| (r.start, r.end))
        .collect();
    let n = name.chars().count();
    sites.extend(
        doc.definitions()
            .into_iter()
            .filter(|d| d.name == name && toplevel(d.start))
            .map(|d| (d.name_at, d.name_at + n)),
    );
    sites.sort();
    sites.dedup();
    sites
}

impl Sexps {
    /// What renaming the name at `p` would change, or None off a name.
    pub fn rename_at(&self, text: &str, p: usize) -> Option<Rename> {
        let doc = Doc::new(text);
        let (name, s, _) = doc.symbol_at(p)?;
        let global_here = global_sites(text, &name);
        if global_here.iter().any(|&(a, _)| a == s) {
            return Some(Rename { name, global: true, sites: global_here });
        }
        // A local: the smallest list around it where it is no longer free
        // is the form that binds it.
        let binder = |at: usize| -> Option<(usize, usize)> {
            let mut q = at;
            while let Some((open, close)) = self.list(q) {
                if !free_in(&self.chars, open.start, close.end, at) {
                    return Some((open.start, close.end));
                }
                q = open.start;
            }
            None
        };
        let (b0, b1) = binder(s)?;
        let sites = self
            .toks
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                t.kind == Kind::Atom
                    && t.start >= b0
                    && t.end <= b1
                    && self.slice(t.start, t.end) == name
                    && !(*i > 0 && self.toks[i - 1].kind == Kind::Prefix && self.toks[i - 1].end == t.start)
            })
            .map(|(_, t)| (t.start, t.end))
            .filter(|&(a, _)| binder(a) == Some((b0, b1)))
            .collect();
        Some(Rename { name, global: false, sites })
    }
}

/// The change that puts NEW at every site (char ranges, in order) of
/// TEXT, with `p` carried along.
pub fn rename_change(text: &str, sites: &[(usize, usize)], new: &str, p: usize) -> Change {
    let chars: Vec<char> = text.chars().collect();
    let (Some(&(first, _)), Some(&(_, last))) = (sites.first(), sites.last()) else {
        return Change { start: p, end: p, text: String::new(), point: p };
    };
    let mut out = String::new();
    let mut at = first;
    let mut point = p;
    let n = new.chars().count() as isize;
    for &(s, e) in sites {
        out.extend(&chars[at..s]);
        out.push_str(new);
        if e <= p {
            // A site before the point moves it by the length difference.
            point = (point as isize + n - (e - s) as isize) as usize;
        } else if s < p {
            // The point inside the renamed site: to its start.
            point -= p - s;
        }
        at = e;
    }
    Change { start: first, end: last, text: out, point }
}

/// M-;: comment lines `l0..=l1` with `;; ` at their shared indent, or —
/// when every non-blank one is already a comment — take the comment off.
/// `lines` is the whole buffer; the answer is the new text of those lines.
pub fn toggle_line_comments(lines: &[&str], l0: usize, l1: usize) -> Vec<String> {
    let range = &lines[l0..=l1.min(lines.len().saturating_sub(1))];
    let blank = |l: &str| l.trim().is_empty();
    let all_commented = range.iter().filter(|l| !blank(l)).all(|l| l.trim_start().starts_with(';'))
        && range.iter().any(|l| !blank(l));
    if all_commented {
        return range
            .iter()
            .map(|l| {
                let indent = l.len() - l.trim_start().len();
                let body = l[indent..].trim_start_matches(';');
                let body = body.strip_prefix(' ').unwrap_or(body);
                format!("{}{body}", &l[..indent])
            })
            .collect();
    }
    let indent = range.iter().filter(|l| !blank(l)).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    range
        .iter()
        .map(|l| if blank(l) { l.to_string() } else { format!("{};; {}", &l[..indent], &l[indent..]) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply OP to the text with `|` marking the point; the result with the
    /// new point marked.
    fn run(src: &str, op: impl Fn(&Sexps, usize) -> Option<Change>) -> Option<String> {
        let p = src.chars().position(|c| c == '|').unwrap();
        let text: String = src.chars().filter(|&c| c != '|').collect();
        let s = Sexps::new(&text);
        let c = op(&s, p)?;
        let chars: Vec<char> = text.chars().collect();
        let mut out: String = chars[..c.start].iter().collect();
        out.push_str(&c.text);
        out.extend(&chars[c.end..]);
        let mut marked: Vec<char> = out.chars().collect();
        marked.insert(c.point, '|');
        Some(marked.into_iter().collect())
    }

    fn moved(src: &str, op: impl Fn(&Sexps, usize) -> Option<usize>) -> Option<String> {
        run(src, |s, p| op(s, p).map(|q| Change { start: 0, end: 0, text: String::new(), point: q }))
    }

    #[test]
    fn motion_by_expression() {
        assert_eq!(moved("(a |(b c) d)", Sexps::forward).unwrap(), "(a (b c)| d)");
        assert_eq!(moved("(a (b c)| d)", Sexps::backward).unwrap(), "(a |(b c) d)");
        assert_eq!(moved("(a b|)", Sexps::forward), None, "nothing after b in the list");
        assert_eq!(moved("x 'fo|o y", Sexps::forward).unwrap(), "x 'foo| y", "mid-atom goes to its end");
        assert_eq!(moved("x 'foo| y", Sexps::backward).unwrap(), "x |'foo y", "a quote is part of it");
        assert_eq!(moved("(a ; ) not a paren\n b|)", Sexps::backward).unwrap(), "(a ; ) not a paren\n |b)");
        assert_eq!(moved("(a \"(\" |b)", Sexps::backward).unwrap(), "(a |\"(\" b)", "a string is one");
        assert_eq!(moved("(a (b |c))", Sexps::up).unwrap(), "(a |(b c))");
        assert_eq!(moved("(a (b |c))", Sexps::up_forward).unwrap(), "(a (b c)|)");
        assert_eq!(moved("|x (a b)", Sexps::down).unwrap(), "x (|a b)");
        assert_eq!(moved("|(a [b])", Sexps::matching).unwrap(), "(a [b]|)");
        assert_eq!(moved("(a [b]|)", Sexps::matching).unwrap(), "|(a [b])");
        assert_eq!(moved("|x #(1 2)", Sexps::matching).unwrap(), "x #(1 2|)", "the next bracket on the line");
    }

    #[test]
    fn kill_and_transpose() {
        assert_eq!(run("(a| (b c) d)", Sexps::kill_forward).unwrap(), "(a| d)");
        assert_eq!(run("(a (b c)| d)", Sexps::kill_backward).unwrap(), "(a | d)");
        assert_eq!(run("(a |b)", Sexps::transpose).unwrap(), "(b a|)");
        assert_eq!(run("(x| (y z))", Sexps::transpose).unwrap(), "((y z) x|)");
    }

    #[test]
    fn slurp_and_barf() {
        assert_eq!(run("(a |b) c d", Sexps::slurp_forward).unwrap(), "(a |b c) d");
        assert_eq!(run("(|) c", Sexps::slurp_forward).unwrap(), "(|c)");
        assert_eq!(run("(a |b c) d", Sexps::barf_forward).unwrap(), "(a |b) c d");
        assert_eq!(run("(a b |c) d", Sexps::barf_forward).unwrap(), "(a b) |c d");
        assert_eq!(run("a (b |c)", Sexps::slurp_backward).unwrap(), "(a b |c)");
        assert_eq!(run("(a b |c)", Sexps::barf_backward).unwrap(), "a (b |c)");
        assert_eq!(run("(a |b) ", Sexps::slurp_forward), None, "nothing to take in");
    }

    #[test]
    fn wrap_splice_raise() {
        assert_eq!(run("(f |x y)", |s, p| Some(s.wrap(p))).unwrap(), "(f (|x) y)");
        assert_eq!(run("(f |)", |s, p| Some(s.wrap(p))).unwrap(), "(f (|))");
        assert_eq!(run("(f (g |x) y)", Sexps::splice).unwrap(), "(f g |x y)");
        assert_eq!(run("(f (g |x) y)", Sexps::raise).unwrap(), "(f |x y)");
        assert_eq!(run("(if c |(do-it) #f)", Sexps::raise).unwrap(), "|(do-it)");
    }

    #[test]
    fn datum_comments() {
        assert_eq!(run("(a |(b c))", Sexps::toggle_datum_comment).unwrap(), "(a #;|(b c))");
        assert_eq!(run("(a #;|(b c))", Sexps::toggle_datum_comment).unwrap(), "(a |(b c))");
    }

    #[test]
    fn typing_keeps_brackets_balanced() {
        let t = |c| move |s: &Sexps, p| s.type_char(p, c);
        assert_eq!(run("(f |)", t('(')).unwrap(), "(f (|))");
        assert_eq!(run("(f [|])", t(']')).unwrap(), "(f []|)", "steps over its own closer");
        assert_eq!(run("(f x|  )", t(')')).unwrap(), "(f x)|", "and drops the space before it");
        assert_eq!(run("(f |x)", t(')')).unwrap(), "(f x)|", "out of the list from anywhere in it");
        assert_eq!(run("x|", t(')')), None, "the top level just types it");
        assert_eq!(run("(f |", t(')')), None, "an unclosed list just types it");
        assert_eq!(run("\"a|\"", t('(')), None, "in a string, just typing");
        assert_eq!(run("; a|", t('(')), None, "in a comment too");
        assert_eq!(run("#\\|", t('(')), None, "after #\\");
        assert_eq!(run("(f |)", t('"')).unwrap(), "(f \"|\")");
        assert_eq!(run("\"ab|\"", t('"')).unwrap(), "\"ab\"|", "steps over the closing quote");
        assert_eq!(run("\"a|b\"", t('"')).unwrap(), "\"a\\\"|b\"", "escapes one in the middle");
        assert_eq!(run("(f |)", t('x')), None);
    }

    #[test]
    fn deleting_keeps_brackets_balanced() {
        assert_eq!(run("(f (|))", Sexps::backspace).unwrap(), "(f |)", "an empty pair goes whole");
        assert_eq!(run("(f (|x))", Sexps::backspace).unwrap(), "(f |(x))", "a full one is stepped over");
        assert_eq!(run("(f x)|", Sexps::backspace).unwrap(), "(f x|)", "into a list from its end");
        assert_eq!(run("\"|\"", Sexps::backspace).unwrap(), "|");
        assert_eq!(run("\"ab\"|", Sexps::backspace).unwrap(), "\"ab|\"");
        assert_eq!(run("(f x|)", Sexps::backspace), None, "an ordinary character");
        assert_eq!(run("(f |())", Sexps::delete_forward).unwrap(), "(f |)");
        assert_eq!(run("(f |(x))", Sexps::delete_forward).unwrap(), "(f (|x))");
        assert_eq!(run("(f x|)", Sexps::delete_forward).unwrap(), "(f x)|");
    }

    #[test]
    fn form_and_toplevel_at() {
        let s = Sexps::new("(define (f x)\n  (when x (g x)))\n(h)");
        assert_eq!(s.form_at(16), Some((16, 30)), "at a list's opener, that list");
        assert_eq!(s.form_at(22), Some((16, 30)), "inside it, the list around");
        assert_eq!(s.toplevel_at(25), Some((0, 31)));
        assert_eq!(s.toplevel_at(32), Some((32, 35)));
    }

    #[test]
    fn pretty_breaks_what_does_not_fit() {
        assert_eq!(pretty("(f   a  b)", 40), "(f a b)");
        let p = pretty("(let ((tmp x) (other (long-call y z))) (set! x y) (set! y tmp))", 30);
        assert_eq!(p, "(let ((tmp x)\n      (other (long-call y z)))\n  (set! x y)\n  (set! y tmp))");
        let q = pretty("(some-function argument-one argument-two argument-three)", 30);
        assert_eq!(q, "(some-function argument-one\n               argument-two\n               argument-three)");
    }

    fn renamed(src: &str, new: &str) -> Option<(bool, String)> {
        let p = src.chars().position(|c| c == '|').unwrap();
        let text: String = src.chars().filter(|&c| c != '|').collect();
        let r = Sexps::new(&text).rename_at(&text, p)?;
        let c = rename_change(&text, &r.sites, new, p);
        let chars: Vec<char> = text.chars().collect();
        let mut out: String = chars[..c.start].iter().collect();
        out.push_str(&c.text);
        out.extend(&chars[c.end..]);
        Some((r.global, out))
    }

    #[test]
    fn rename_follows_scope() {
        // A local: only the form that binds it, and not a shadowing one.
        let src = "(define (f |x) (let ((y x)) (+ x y (let ((x 2)) x))))\n(define (g x) x)";
        assert_eq!(
            renamed(src, "n").unwrap(),
            (false, "(define (f n) (let ((y n)) (+ n y (let ((x 2)) x))))\n(define (g x) x)".into())
        );
        // A top-level name: its definition and every free use, not a local
        // that shadows it or the symbol quoted.
        let src = "(define (sl|ab w) w)\n(slab 1)\n(let ((slab 2)) slab)\n'slab\n(map slab xs)";
        assert_eq!(
            renamed(src, "plate").unwrap(),
            (true, "(define (plate w) w)\n(plate 1)\n(let ((slab 2)) slab)\n'slab\n(map plate xs)".into())
        );
        assert_eq!(renamed("(f \"te|xt\")", "z"), None, "not on a name");
    }

    #[test]
    fn line_comments_toggle() {
        let lines = ["(define (f x)", "  (g x)", "", "  (h x))"];
        let on = toggle_line_comments(&lines, 1, 3);
        assert_eq!(on, ["  ;; (g x)", "", "  ;; (h x))"]);
        let refs: Vec<&str> = on.iter().map(String::as_str).collect();
        assert_eq!(toggle_line_comments(&refs, 0, 2), ["  (g x)", "", "  (h x))"]);
        assert_eq!(toggle_line_comments(&["x"], 0, 0), [";; x"]);
    }
}
