//! The buffer, and everything answerable from its text alone.
//!
//! **Why a token scan and not scm's reader.** The reader is the right
//! authority on whether text is a program -- which is exactly what
//! `diagnostics` asks it, in `main.rs`. But an editor's buffer is
//! *unparseable most of the time*: every half-typed form is an incomplete
//! datum, and a reader-based outline would vanish on the keystroke after
//! `(define (`. So structure here comes from a token scan that degrades
//! instead of failing, which is what `imenu` has always done. The two uses
//! are not redundant: one answers "is this right", the other "what is
//! around the cursor", and only the first has an answer worth refusing.
//!
//! **Offsets are characters, not bytes.** scm's `Span` indexes the `Vec<char>`
//! its reader works on, and LSP counts UTF-16 code units from a line start.
//! Neither is a byte offset, so bytes never appear in this file.

/// What a lexeme is. The outline and the cursor questions need no more
/// detail than this -- an atom's *text* is read from the source when it
/// matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Open,
    Close,
    Atom,
    Str,
    Comment,
    /// `'`, `` ` ``, `,`, `,@` and the `#;` datum comment: they prefix a
    /// datum without nesting it, so they must not move the depth.
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    /// Nesting the token sits *at*: a top-level `(` is 0 and its contents
    /// are 1. A `)` carries the depth of the `(` it closes, so a form's
    /// brackets agree.
    pub depth: usize,
}

pub struct Doc {
    chars: Vec<char>,
    /// Char offset where each line begins.
    line_starts: Vec<usize>,
    tokens: Vec<Token>,
}

/// What ends a token. `#` is absent on purpose: `#t`, `#(`, `#\a` and
/// `#u8(` all begin with it, and the lexer handles those cases itself.
fn is_delimiter(c: char) -> bool {
    c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | '"' | ';' | '\'' | '`' | ',')
}

impl Doc {
    pub fn new(text: &str) -> Doc {
        let chars: Vec<char> = text.chars().collect();
        let mut line_starts = vec![0usize];
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                line_starts.push(i + 1);
            }
        }
        let tokens = lex(&chars);
        Doc { chars, line_starts, tokens }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// The token stream, for the tests: every question above is answered
    /// from it, so a lexing bug is cheaper to find here than through the
    /// answer it corrupted.
    #[cfg(test)]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        self.chars[start.min(self.chars.len())..end.min(self.chars.len())]
            .iter()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// A char offset as an LSP position: a 0-based line, and **UTF-16 code
    /// units** from the line's start. An editor that counted characters
    /// would put the cursor in the wrong place the moment a line held an
    /// emoji or a `λ`.
    pub fn position(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.chars.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col: usize = self.chars[self.line_starts[line]..offset]
            .iter()
            .map(|c| c.len_utf16())
            .sum();
        (line as u32, col as u32)
    }

    /// The inverse. A position past the end of a line clamps to its end,
    /// because a client may send one and refusing would be worse than
    /// answering.
    pub fn offset(&self, line: u32, character: u32) -> usize {
        let Some(&start) = self.line_starts.get(line as usize) else {
            return self.chars.len();
        };
        let end = self
            .line_starts
            .get(line as usize + 1)
            .map(|n| n - 1)
            .unwrap_or(self.chars.len());
        let mut units = 0usize;
        for (i, c) in self.chars[start..end].iter().enumerate() {
            if units >= character as usize {
                return start + i;
            }
            units += c.len_utf16();
        }
        end
    }

    /// The token at `offset`.
    ///
    /// A token that **contains** the offset wins over one that merely
    /// *ends* at it, and the order matters more than it looks. Both cases
    /// are real — the cursor sits inside a name, or just after it — but
    /// they collide at a boundary: in `(helper`, with the cursor on the
    /// `h`, the opening paren ends at exactly that offset. Taking the
    /// first token whose range included the offset inclusively returned
    /// the paren, so hover and go-to-definition quietly did nothing for
    /// every symbol written straight after a delimiter, which is most of
    /// them in a Lisp.
    fn token_at(&self, offset: usize) -> Option<&Token> {
        self.tokens
            .iter()
            .find(|t| t.start <= offset && offset < t.end)
            .or_else(|| self.tokens.iter().find(|t| t.end == offset))
    }

    /// The name at the cursor, for hover and go-to-definition.
    pub fn symbol_at(&self, offset: usize) -> Option<(String, usize, usize)> {
        let t = self.token_at(offset)?;
        if t.kind != Kind::Atom {
            return None;
        }
        Some((self.slice(t.start, t.end), t.start, t.end))
    }

    /// The prefix being typed, for completion: the atom text up to the
    /// cursor, which may be empty just after a `(`.
    pub fn prefix_at(&self, offset: usize) -> String {
        let mut start = offset.min(self.chars.len());
        while start > 0 && !is_delimiter(self.chars[start - 1]) {
            start -= 1;
        }
        self.slice(start, offset)
    }

    /// Whether the cursor is inside a string or a comment, where neither
    /// completion nor signature help should fire.
    pub fn in_text(&self, offset: usize) -> bool {
        self.tokens
            .iter()
            .any(|t| matches!(t.kind, Kind::Str | Kind::Comment) && t.start < offset && offset < t.end)
    }

    /// The innermost form still open at `offset`: its head name and which
    /// argument the cursor is in. This is signature help's whole question.
    ///
    /// Counted over *forms* rather than atoms, so a nested call counts once
    /// and `(vector-ref (f x) |)` is argument 1, not 3.
    pub fn enclosing_call(&self, offset: usize) -> Option<(String, usize, u32)> {
        let mut open: Vec<usize> = Vec::new(); // indices into self.tokens
        for (i, t) in self.tokens.iter().enumerate() {
            if t.start >= offset {
                break;
            }
            match t.kind {
                Kind::Open => open.push(i),
                Kind::Close => {
                    // A `)` at or past the cursor does not close anything
                    // the cursor is inside.
                    if t.end <= offset {
                        open.pop();
                    }
                }
                _ => {}
            }
        }
        let opener = *open.last()?;
        let depth = self.tokens[opener].depth;
        let mut head: Option<String> = None;
        let mut arg = 0u32;
        for t in &self.tokens[opener + 1..] {
            if t.start >= offset {
                break;
            }
            // Only the form's own children count, not a nested form's.
            if t.depth != depth + 1 || matches!(t.kind, Kind::Comment | Kind::Prefix | Kind::Close) {
                continue;
            }
            match head {
                None if t.kind == Kind::Atom => head = Some(self.slice(t.start, t.end)),
                None => return None, // `((f x) y)`: the head is not a name
                Some(_) => arg += 1,
            }
        }
        let head = head?;
        Some((head, self.tokens[opener].start, arg))
    }

    /// The token indices of a form's direct children: the things at one
    /// level in, skipping whatever is nested deeper.
    fn children(&self, open: usize) -> Vec<usize> {
        let depth = self.tokens[open].depth;
        let mut out = Vec::new();
        for (i, t) in self.tokens.iter().enumerate().skip(open + 1) {
            if t.kind == Kind::Close && t.depth == depth {
                break;
            }
            if t.depth == depth + 1 && !matches!(t.kind, Kind::Comment | Kind::Close) {
                out.push(i);
            }
        }
        out
    }

    /// The names a `define-record-type` binds *besides* the type: its
    /// constructor, its predicate, and an accessor and modifier per field.
    ///
    /// They are ordinary top-level bindings and they are right there in the
    /// text, but they are not where a `define`'s name is, so the general
    /// case above misses every one of them -- which made `point-x`
    /// unreachable by go-to-definition in the file that defines it.
    ///
    ///     (define-record-type point
    ///       (make-point x y)        ; constructor
    ///       point?                  ; predicate
    ///       (x point-x)             ; accessor
    ///       (y point-y set-y!))     ; accessor and modifier
    fn record_type_names(&self, open: usize, end: usize) -> Vec<Definition> {
        let kids = self.children(open);
        let mut out = Vec::new();
        // [0] is `define-record-type`, [1] the type name.
        let mut seen_predicate = false;
        for &k in kids.iter().skip(2) {
            let t = self.tokens[k];
            match t.kind {
                // A bare symbol here is the predicate.
                Kind::Atom if !seen_predicate => {
                    seen_predicate = true;
                    out.push(Definition {
                        name: self.slice(t.start, t.end),
                        form: "define-record-type".into(),
                        detail: Some("predicate".into()),
                        name_at: t.start,
                        start: t.start,
                        end,
                    });
                }
                Kind::Open => {
                    let inner: Vec<usize> = self
                        .children(k)
                        .into_iter()
                        .filter(|&i| self.tokens[i].kind == Kind::Atom)
                        .collect();
                    // Before the predicate this is the constructor; after
                    // it, a field spec whose first atom is the field.
                    let (skip, what) = if seen_predicate {
                        (1, ["accessor", "modifier"])
                    } else {
                        (0, ["constructor", "constructor"])
                    };
                    for (n, &i) in inner.iter().skip(skip).enumerate() {
                        let a = self.tokens[i];
                        out.push(Definition {
                            name: self.slice(a.start, a.end),
                            form: "define-record-type".into(),
                            detail: Some(what[n.min(1)].into()),
                            name_at: a.start,
                            start: a.start,
                            end,
                        });
                        // A constructor's remaining atoms are its fields,
                        // not names it binds.
                        if !seen_predicate {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Every name a `define-values` binds, not just the first.
    fn define_values_names(&self, open: usize, end: usize) -> Vec<Definition> {
        let kids = self.children(open);
        let Some(&formals) = kids.get(1) else { return Vec::new() };
        if self.tokens[formals].kind != Kind::Open {
            return Vec::new();
        }
        self.children(formals)
            .into_iter()
            .filter(|&i| self.tokens[i].kind == Kind::Atom)
            .map(|i| {
                let t = self.tokens[i];
                Definition {
                    name: self.slice(t.start, t.end),
                    form: "define-values".into(),
                    detail: None,
                    name_at: t.start,
                    start: t.start,
                    end,
                }
            })
            .collect()
    }

    /// Every top-level definition, for the outline and for jumping to one.
    ///
    /// Top level means depth 0, so an internal `define` is deliberately not
    /// listed: an outline of a file should be the file's shape, not every
    /// binding in it.
    pub fn definitions(&self) -> Vec<Definition> {
        const FORMS: [&str; 6] = [
            "define",
            "define-syntax",
            "define-values",
            "define-record-type",
            "define-library",
            "define-structure",
        ];
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.tokens.len() {
            let t = self.tokens[i];
            if t.kind != Kind::Open || t.depth != 0 {
                i += 1;
                continue;
            }
            // The closing bracket of this form, for the symbol's range.
            let end = self.tokens[i..]
                .iter()
                .find(|c| c.kind == Kind::Close && c.depth == 0)
                .map(|c| c.end)
                // Unclosed, because the file is mid-edit: run to the end
                // rather than dropping the definition the user is writing.
                .unwrap_or(self.chars.len());
            let head = self.tokens.get(i + 1).copied();
            let form = head
                .filter(|h| h.kind == Kind::Atom)
                .map(|h| self.slice(h.start, h.end));
            if let Some(form) = form.filter(|f| FORMS.contains(&f.as_str())) {
                // `(define x …)` names an atom; `(define (f a) …)` names the
                // atom inside the next form. `define-library` names a list
                // -- `(scm noise)` -- which is the name a reader wants to
                // see, so it is taken whole.
                let named = self.tokens.get(i + 2).copied();
                let (name, name_at) = match named {
                    Some(n) if n.kind == Kind::Atom => (self.slice(n.start, n.end), n.start),
                    Some(n) if n.kind == Kind::Open && form == "define-library" => {
                        let close = self.tokens[i + 2..]
                            .iter()
                            .find(|c| c.kind == Kind::Close && c.depth == n.depth)
                            .map(|c| c.end)
                            .unwrap_or(n.end);
                        (self.slice(n.start, close), n.start)
                    }
                    Some(n) if n.kind == Kind::Open => match self.tokens.get(i + 3) {
                        Some(a) if a.kind == Kind::Atom => (self.slice(a.start, a.end), a.start),
                        _ => {
                            i += 1;
                            continue;
                        }
                    },
                    _ => {
                        i += 1;
                        continue;
                    }
                };
                let arity = named
                    .filter(|n| n.kind == Kind::Open && form == "define")
                    .map(|n| {
                        let close = self.tokens[i + 2..]
                            .iter()
                            .find(|c| c.kind == Kind::Close && c.depth == n.depth)
                            .map(|c| c.end)
                            .unwrap_or(n.end);
                        self.slice(n.start, close)
                    });
                match form.as_str() {
                    "define-record-type" => {
                        out.push(Definition {
                            name,
                            form: form.clone(),
                            detail: Some("type".into()),
                            name_at,
                            start: t.start,
                            end,
                        });
                        out.extend(self.record_type_names(i, end));
                    }
                    "define-values" => {
                        let extra = self.define_values_names(i, end);
                        if extra.is_empty() {
                            out.push(Definition { name, form, detail: arity, name_at, start: t.start, end });
                        } else {
                            out.extend(extra);
                        }
                    }
                    _ => out.push(Definition { name, form, detail: arity, name_at, start: t.start, end }),
                }
            }
            i += 1;
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub name: String,
    /// Which `define…` introduced it, so the outline can show a kind.
    pub form: String,
    /// `(f a b)` for a procedure, so the outline shows the call shape --
    /// the thing a signature is read for.
    pub detail: Option<String>,
    pub name_at: usize,
    pub start: usize,
    pub end: usize,
}

/// One pass, left to right. Nothing here can fail: unterminated anything
/// runs to the end of the buffer, because a buffer being edited is
/// unterminated most of the time and an error would throw away the
/// structure the editor still needs.
fn lex(chars: &[char]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    let n = chars.len();
    while i < n {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        match c {
            ';' => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                out.push(Token { kind: Kind::Comment, start, end: i, depth });
            }
            '(' | '[' | '{' => {
                i += 1;
                out.push(Token { kind: Kind::Open, start, end: i, depth });
                depth += 1;
            }
            ')' | ']' | '}' => {
                i += 1;
                depth = depth.saturating_sub(1);
                out.push(Token { kind: Kind::Close, start, end: i, depth });
            }
            '"' => {
                i += 1;
                while i < n {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                out.push(Token { kind: Kind::Str, start, end: i.min(n), depth });
            }
            '\'' | '`' => {
                i += 1;
                out.push(Token { kind: Kind::Prefix, start, end: i, depth });
            }
            ',' => {
                i += 1;
                if i < n && chars[i] == '@' {
                    i += 1;
                }
                out.push(Token { kind: Kind::Prefix, start, end: i, depth });
            }
            '#' if i + 1 < n => {
                match chars[i + 1] {
                    // A block comment nests, so it is counted rather than
                    // searched for: `#| #| |# |#` ends once.
                    '|' => {
                        i += 2;
                        let mut nest = 1usize;
                        while i < n && nest > 0 {
                            if i + 1 < n && chars[i] == '#' && chars[i + 1] == '|' {
                                nest += 1;
                                i += 2;
                            } else if i + 1 < n && chars[i] == '|' && chars[i + 1] == '#' {
                                nest -= 1;
                                i += 2;
                            } else {
                                i += 1;
                            }
                        }
                        out.push(Token { kind: Kind::Comment, start, end: i.min(n), depth });
                    }
                    // `#;` comments out the datum that follows, which still
                    // lexes. A commented-out definition therefore shows in
                    // the outline; that is a known and cheap wrongness.
                    ';' => {
                        i += 2;
                        out.push(Token { kind: Kind::Prefix, start, end: i, depth });
                    }
                    // `#\(` is a character, not a bracket. Getting this
                    // wrong unbalances every form after it.
                    '\\' => {
                        i += 3.min(n - i);
                        while i < n && !is_delimiter(chars[i]) {
                            i += 1;
                        }
                        out.push(Token { kind: Kind::Atom, start, end: i, depth });
                    }
                    // `#(`, `#u8(`: a vector's bracket still nests.
                    _ => {
                        i += 1;
                        while i < n && !is_delimiter(chars[i]) {
                            i += 1;
                        }
                        out.push(Token { kind: Kind::Atom, start, end: i, depth });
                    }
                }
            }
            _ => {
                while i < n && !is_delimiter(chars[i]) {
                    i += 1;
                }
                out.push(Token { kind: Kind::Atom, start, end: i, depth });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<Kind> {
        Doc::new(text).tokens().iter().map(|t| t.kind).collect()
    }

    #[test]
    fn a_character_literal_is_not_a_bracket() {
        // `#\(` unbalances everything after it if it lexes as an Open, and
        // the symptom is an outline that stops halfway down a file.
        let d = Doc::new("(display #\\() (define x 1)");
        assert_eq!(
            kinds("(display #\\() (define x 1)"),
            vec![Kind::Open, Kind::Atom, Kind::Atom, Kind::Close,
                 Kind::Open, Kind::Atom, Kind::Atom, Kind::Atom, Kind::Close]
        );
        // And the definition after it is still found.
        assert_eq!(d.definitions().len(), 1);
        assert_eq!(d.definitions()[0].name, "x");
    }

    #[test]
    fn brackets_parens_and_strings_lex_apart() {
        assert_eq!(
            kinds("(let ([x \"a)(b\"]) x)"),
            vec![Kind::Open, Kind::Atom, Kind::Open, Kind::Open, Kind::Atom,
                 Kind::Str, Kind::Close, Kind::Close, Kind::Atom, Kind::Close]
        );
    }

    #[test]
    fn a_block_comment_nests() {
        // `#| #| |# |#` ends once, and a search for the first `|#` would
        // leave the rest of the file inside a comment.
        let d = Doc::new("#| a #| b |# c |# (define x 1)");
        assert_eq!(d.definitions().len(), 1);
        assert_eq!(kinds("#| a #| b |# c |#"), vec![Kind::Comment]);
    }

    #[test]
    fn utf16_columns_are_not_character_columns() {
        // LSP counts UTF-16 code units. An emoji is two, so a client told
        // "character 2" for text after it would put the cursor inside the
        // emoji -- and `λ`, which is one unit, must still be one.
        let d = Doc::new("(f \"🙂\" λ)");
        let text: Vec<char> = "(f \"🙂\" λ)".chars().collect();
        let lambda_at = text.iter().position(|c| *c == 'λ').unwrap();
        let (line, col) = d.position(lambda_at);
        assert_eq!(line, 0);
        // ( f space " emoji(2) " space = 1+1+1+1+2+1+1
        assert_eq!(col, 8);
        // And it round-trips.
        assert_eq!(d.offset(line, col), lambda_at);
    }

    #[test]
    fn a_position_past_the_end_of_a_line_clamps() {
        let d = Doc::new("(a)\n(b)\n");
        assert_eq!(d.offset(0, 99), 3);
        assert_eq!(d.offset(99, 0), d.len());
    }

    #[test]
    fn the_symbol_at_the_cursor_includes_scms_own_spellings() {
        // `:at` and `&opt` are single tokens: a hover on one must not come
        // back as `at` or as nothing.
        let d = Doc::new("(set-marker :tex-scale 2 &opt x)");
        let text = "(set-marker :tex-scale 2 &opt x)";
        let colon = text.find(":tex-scale").unwrap();
        assert_eq!(d.symbol_at(colon).unwrap().0, ":tex-scale");
        // The cursor just after a name is the common case, not inside it.
        assert_eq!(d.symbol_at(colon + ":tex-scale".len()).unwrap().0, ":tex-scale");
        let amp = text.find("&opt").unwrap();
        assert_eq!(d.symbol_at(amp).unwrap().0, "&opt");
    }

    #[test]
    fn a_symbol_right_after_a_delimiter_is_found() {
        // The cursor on the `h` of `(helper`: the paren *ends* at that
        // offset while the atom *contains* it, and containing must win.
        // Getting this wrong made hover and go-to-definition do nothing
        // for most symbols in a Lisp, since most follow a bracket.
        let text = "(define (go) (helper 21))";
        let d = Doc::new(text);
        let h = text.find("helper").unwrap();
        assert_eq!(d.symbol_at(h).map(|x| x.0), Some("helper".to_string()));
        // And the cursor just after the name still resolves to it.
        assert_eq!(d.symbol_at(h + 6).map(|x| x.0), Some("helper".to_string()));
        // Inside, too.
        assert_eq!(d.symbol_at(h + 3).map(|x| x.0), Some("helper".to_string()));
        // On the bracket itself: not a symbol.
        assert_eq!(d.symbol_at(h - 1).map(|x| x.0), None);
    }

    #[test]
    fn a_prefix_is_what_has_been_typed_not_the_whole_atom() {
        let d = Doc::new("(vector-re");
        assert_eq!(d.prefix_at(d.len()), "vector-re");
        // Just after an open paren there is no prefix, and completion
        // should offer everything rather than nothing.
        let d = Doc::new("(");
        assert_eq!(d.prefix_at(1), "");
    }

    #[test]
    fn the_enclosing_call_counts_forms_not_atoms() {
        let text = "(vector-ref (f x y) ";
        let d = Doc::new(text);
        let (head, _, arg) = d.enclosing_call(text.len()).unwrap();
        assert_eq!(head, "vector-ref");
        // The nested call is argument 0; the cursor is in argument 1. An
        // atom count would say 3 and point signature help at the wrong
        // parameter.
        assert_eq!(arg, 1);
    }

    #[test]
    fn the_enclosing_call_is_the_innermost_open_one() {
        let text = "(a (b c ";
        let d = Doc::new(text);
        assert_eq!(d.enclosing_call(text.len()).unwrap().0, "b");
        // A closed form is not enclosing.
        let text = "(a (b c) ";
        let d = Doc::new(text);
        assert_eq!(d.enclosing_call(text.len()).unwrap().0, "a");
    }

    #[test]
    fn definitions_are_the_files_shape_not_every_binding() {
        let src = "\
(define x 1)
(define (area r) \"Circle area.\" (* 3 r r))
(define-syntax swap! (syntax-rules () ((_ a b) 1)))
(define-record-type point (make-point x y) point? (x point-x))
(define-library (scm noise)
  (export f))
(define (outer a)
  (define inner 2)   ; not top level, deliberately
  inner)
";
        let d = Doc::new(src);
        let got: Vec<(String, String)> = d
            .definitions()
            .into_iter()
            .map(|x| (x.name, x.form))
            .collect();
        assert_eq!(
            got,
            vec![
                ("x".to_string(), "define".to_string()),
                ("area".to_string(), "define".to_string()),
                ("swap!".to_string(), "define-syntax".to_string()),
                // A record type brings its constructor, predicate and
                // accessors with it -- see `a_record_type_binds_more_than
                // _its_name`. They are top-level bindings, so leaving them
                // out made them unreachable by go-to-definition.
                ("point".to_string(), "define-record-type".to_string()),
                ("make-point".to_string(), "define-record-type".to_string()),
                ("point?".to_string(), "define-record-type".to_string()),
                ("point-x".to_string(), "define-record-type".to_string()),
                ("(scm noise)".to_string(), "define-library".to_string()),
                ("outer".to_string(), "define".to_string()),
            ]
        );
        // A procedure carries its call shape, which is what an outline is
        // read for.
        let area = d.definitions().into_iter().find(|x| x.name == "area").unwrap();
        assert_eq!(area.detail.as_deref(), Some("(area r)"));
    }

    #[test]
    fn a_definition_being_written_is_still_found() {
        // The whole reason structure comes from tokens: the reader would
        // refuse this text, and the outline would blink out on the
        // keystroke after `(define (`.
        let d = Doc::new("(define x 1)\n(define (half-typed\n");
        let names: Vec<String> = d.definitions().into_iter().map(|x| x.name).collect();
        assert_eq!(names, vec!["x", "half-typed"]);
    }

    #[test]
    fn a_bracket_binding_does_not_become_a_definition() {
        // `[x 1]` is a vector in scm and a binding everywhere a list is
        // expected; neither is a top-level define, and depth keeps them out.
        let d = Doc::new("(let ([x 1] [y 2])\n  (define inner 3)\n  y)");
        assert!(d.definitions().is_empty());
    }

    #[test]
    fn inside_a_string_or_comment_is_known() {
        let text = "(f \"abc\") ; note\n(g)";
        let d = Doc::new(text);
        assert!(d.in_text(text.find("abc").unwrap()));
        assert!(d.in_text(text.find("note").unwrap()));
        assert!(!d.in_text(text.find("(g)").unwrap() + 1));
    }
}

#[cfg(test)]
mod more_tests {
    use super::*;

    #[test]
    fn a_record_type_binds_more_than_its_name() {
        // The constructor, the predicate and every accessor and modifier
        // are ordinary top-level bindings, and they are in the text -- but
        // they are not where a `define`'s name is, so the general case
        // missed all of them. `point-x` was unreachable by
        // go-to-definition in the very file that defines it.
        let d = Doc::new(
            "(define-record-type point\n\
            \x20 (make-point x y)\n\
            \x20 point?\n\
            \x20 (x point-x)\n\
            \x20 (y point-y set-point-y!))\n",
        );
        let got: Vec<(String, Option<String>)> = d
            .definitions()
            .into_iter()
            .map(|x| (x.name, x.detail))
            .collect();
        assert_eq!(
            got,
            vec![
                ("point".to_string(), Some("type".to_string())),
                ("make-point".to_string(), Some("constructor".to_string())),
                ("point?".to_string(), Some("predicate".to_string())),
                ("point-x".to_string(), Some("accessor".to_string())),
                ("point-y".to_string(), Some("accessor".to_string())),
                ("set-point-y!".to_string(), Some("modifier".to_string())),
            ]
        );
    }

    #[test]
    fn a_constructors_field_list_is_not_a_set_of_names() {
        // `(make-point x y)` binds `make-point`; `x` and `y` name fields
        // and are already bound by nothing at all.
        let d = Doc::new("(define-record-type p (make-p a b c) p?)\n");
        let names: Vec<String> = d.definitions().into_iter().map(|x| x.name).collect();
        assert_eq!(names, vec!["p", "make-p", "p?"]);
    }

    #[test]
    fn define_values_binds_every_name() {
        let d = Doc::new("(define-values (q r) (floor/ 7 2))\n");
        let names: Vec<String> = d.definitions().into_iter().map(|x| x.name).collect();
        assert_eq!(names, vec!["q", "r"]);
        // The single-name spelling still works.
        let d = Doc::new("(define-values one (values 1))\n");
        let names: Vec<String> = d.definitions().into_iter().map(|x| x.name).collect();
        assert_eq!(names, vec!["one"]);
    }

    #[test]
    fn a_record_type_being_written_does_not_panic() {
        // Mid-edit text reaches every one of these paths.
        for src in [
            "(define-record-type",
            "(define-record-type point",
            "(define-record-type point (",
            "(define-record-type point (make-point x",
            "(define-record-type point (make-point x y) point? (x",
            "(define-values",
            "(define-values (",
        ] {
            let _ = Doc::new(src).definitions();
        }
    }
}
