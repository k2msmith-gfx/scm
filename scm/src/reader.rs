//! The datum reader.
//!
//! Two things here are decisions rather than mechanics, both from plan §4:
//!
//!   * `:at` reads as a **keyword**, not a symbol. That is the one deliberate
//!     deviation from R7RS datum syntax -- `:` is a legal identifier initial,
//!     so `:at` is a legal R7RS symbol -- and `ReaderOptions::colon_keywords`
//!     turns it off, which is how the conformance suite will run.
//!   * `[a b c]` reads as a vector flagged `bracket`, because brackets
//!     *evaluate* their elements while `#(a b c)` is a constant. R7RS reserves
//!     `[ ] { }` for future extensions, so this costs no conformance at all.
//!
//! And one obligation that is an exit criterion rather than a nicety: a
//! truncated datum reports `Incomplete`, never `Invalid`. The REPL's
//! multi-line entry and the editor panes' paren handling both rest on being
//! able to tell "keep typing" from "that is wrong".

use crate::heap::Engine;
use crate::value::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReadError {
    /// The input ended in the middle of a datum. Ask for more; do not
    /// complain.
    Incomplete,
    Invalid { span: Span, msg: String },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Incomplete => write!(f, "incomplete datum"),
            ReadError::Invalid { span, msg } => {
                write!(f, "{}..{}: {}", span.start, span.end, msg)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReaderOptions {
    /// `:at` is a keyword. Set false to read it as the symbol R7RS says it is.
    pub colon_keywords: bool,
}

impl Default for ReaderOptions {
    fn default() -> Self {
        Self {
            colon_keywords: true,
        }
    }
}

pub struct Reader {
    src: Vec<char>,
    pos: usize,
    opts: ReaderOptions,
}

type RResult<T> = Result<T, ReadError>;

impl Reader {
    pub fn new(src: &str) -> Self {
        Self::with_options(src, ReaderOptions::default())
    }

    pub fn with_options(src: &str, opts: ReaderOptions) -> Self {
        Self {
            src: src.chars().collect(),
            pos: 0,
            opts,
        }
    }

    /// Read every datum in the input.
    pub fn read_all(src: &str, e: &mut Engine) -> RResult<Vec<Value>> {
        let mut r = Reader::new(src);
        let mut out = Vec::new();
        while let Some(v) = r.read(e)? {
            out.push(v);
        }
        Ok(out)
    }

    /// The next datum, or `None` at end of input.
    pub fn read(&mut self, e: &mut Engine) -> RResult<Option<Value>> {
        self.skip_atmosphere()?;
        if self.at_end() {
            return Ok(None);
        }
        self.datum(e).map(Some)
    }

    // ---- character plumbing --------------------------------------------

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn invalid<T>(&self, start: usize, msg: impl Into<String>) -> RResult<T> {
        Err(ReadError::Invalid {
            span: Span {
                start,
                end: self.pos.max(start + 1),
            },
            msg: msg.into(),
        })
    }

    fn is_delimiter(c: char) -> bool {
        c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '"' | ';' | '\'' | '`' | ',')
    }

    /// Whitespace and the three kinds of comment.
    fn skip_atmosphere(&mut self) -> RResult<()> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.pos += 1;
                }
                Some(';') => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        if c == '\n' {
                            break;
                        }
                    }
                }
                Some('#') if self.peek_at(1) == Some('|') => {
                    let start = self.pos;
                    self.pos += 2;
                    let mut depth = 1usize;
                    loop {
                        match (self.peek(), self.peek_at(1)) {
                            (None, _) => return Err(ReadError::Incomplete),
                            (Some('|'), Some('#')) => {
                                self.pos += 2;
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            (Some('#'), Some('|')) => {
                                self.pos += 2;
                                depth += 1;
                            }
                            _ => {
                                self.pos += 1;
                            }
                        }
                    }
                    let _ = start;
                }
                _ => return Ok(()),
            }
        }
    }

    /// `#;` comments out the next datum, so it has to be read and thrown away.
    /// Doing it inside `skip_atmosphere` would need an `Engine`, which the
    /// whitespace path has no business touching.
    fn skip_datum_comments(&mut self, e: &mut Engine) -> RResult<()> {
        loop {
            self.skip_atmosphere()?;
            if self.peek() == Some('#') && self.peek_at(1) == Some(';') {
                self.pos += 2;
                self.skip_atmosphere()?;
                if self.at_end() {
                    return Err(ReadError::Incomplete);
                }
                let _ = self.datum(e)?;
            } else {
                return Ok(());
            }
        }
    }

    // ---- datums ---------------------------------------------------------

    fn datum(&mut self, e: &mut Engine) -> RResult<Value> {
        self.skip_datum_comments(e)?;
        let start = self.pos;
        let c = match self.peek() {
            None => return Err(ReadError::Incomplete),
            Some(c) => c,
        };
        match c {
            '(' => {
                self.pos += 1;
                self.list(e, ')')
            }
            '[' => {
                self.pos += 1;
                let items = self.list_items(e, ']')?;
                Ok(e.vector(items, true))
            }
            ')' | ']' => self.invalid(start, format!("unexpected `{c}`")),
            '"' => self.string(e),
            '\'' => self.abbrev(e, "quote"),
            '`' => self.abbrev(e, "quasiquote"),
            ',' => {
                if self.peek_at(1) == Some('@') {
                    self.pos += 1;
                    self.abbrev(e, "unquote-splicing")
                } else {
                    self.abbrev(e, "unquote")
                }
            }
            '#' => self.hash(e),
            '|' => self.pipe_symbol(e),
            _ => self.token(e),
        }
    }

    fn abbrev(&mut self, e: &mut Engine, name: &str) -> RResult<Value> {
        self.pos += 1;
        self.skip_atmosphere()?;
        if self.at_end() {
            return Err(ReadError::Incomplete);
        }
        let inner = self.datum(e)?;
        let head = e.sym(name);
        Ok(e.list(&[head, inner]))
    }

    /// A list, with R7RS's dotted tail.
    fn list(&mut self, e: &mut Engine, close: char) -> RResult<Value> {
        let mut items: Vec<Value> = Vec::new();
        let mut tail = Value::Null;
        loop {
            self.skip_datum_comments(e)?;
            match self.peek() {
                None => return Err(ReadError::Incomplete),
                Some(c) if c == close => {
                    self.pos += 1;
                    break;
                }
                Some('.') if self.dot_is_tail_marker() => {
                    let dot = self.pos;
                    self.pos += 1;
                    if items.is_empty() {
                        return self.invalid(dot, "`.` with nothing before it");
                    }
                    self.skip_datum_comments(e)?;
                    if self.at_end() {
                        return Err(ReadError::Incomplete);
                    }
                    tail = self.datum(e)?;
                    self.skip_datum_comments(e)?;
                    match self.peek() {
                        None => return Err(ReadError::Incomplete),
                        Some(c) if c == close => {
                            self.pos += 1;
                            break;
                        }
                        _ => return self.invalid(dot, "more than one datum after `.`"),
                    }
                }
                _ => items.push(self.datum(e)?),
            }
        }
        let mut acc = tail;
        for &item in items.iter().rev() {
            acc = e.cons(item, acc);
        }
        Ok(acc)
    }

    fn list_items(&mut self, e: &mut Engine, close: char) -> RResult<Vec<Value>> {
        let mut items = Vec::new();
        loop {
            self.skip_datum_comments(e)?;
            match self.peek() {
                None => return Err(ReadError::Incomplete),
                Some(c) if c == close => {
                    self.pos += 1;
                    return Ok(items);
                }
                _ => items.push(self.datum(e)?),
            }
        }
    }

    /// A lone `.` is the dotted-tail marker; `...` and `.5` are not.
    fn dot_is_tail_marker(&self) -> bool {
        self.peek() == Some('.')
            && match self.peek_at(1) {
                None => true,
                Some(c) => Self::is_delimiter(c),
            }
    }

    fn string(&mut self, e: &mut Engine) -> RResult<Value> {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let c = match self.bump() {
                None => return Err(ReadError::Incomplete),
                Some(c) => c,
            };
            match c {
                '"' => break,
                '\\' => {
                    let esc = match self.bump() {
                        None => return Err(ReadError::Incomplete),
                        Some(c) => c,
                    };
                    match esc {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'a' => out.push('\u{7}'),
                        'b' => out.push('\u{8}'),
                        '0' => out.push('\0'),
                        '\\' => out.push('\\'),
                        '"' => out.push('"'),
                        'x' | 'X' => {
                            let mut hex = String::new();
                            loop {
                                match self.bump() {
                                    None => return Err(ReadError::Incomplete),
                                    Some(';') => break,
                                    Some(h) => hex.push(h),
                                }
                            }
                            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                Some(ch) => out.push(ch),
                                None => {
                                    return self.invalid(start, format!("bad hex escape `\\x{hex};`"));
                                }
                            }
                        }
                        // A backslash before a newline swallows the newline
                        // and the indentation after it.
                        '\n' => {
                            while matches!(self.peek(), Some(c) if c == ' ' || c == '\t') {
                                self.pos += 1;
                            }
                        }
                        other => {
                            return self.invalid(start, format!("unknown string escape `\\{other}`"));
                        }
                    }
                }
                other => out.push(other),
            }
        }
        Ok(e.string(&out))
    }

    fn pipe_symbol(&mut self, e: &mut Engine) -> RResult<Value> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(ReadError::Incomplete),
                Some('|') => break,
                Some('\\') => match self.bump() {
                    None => return Err(ReadError::Incomplete),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(c) => out.push(c),
                },
                Some(c) => out.push(c),
            }
        }
        Ok(e.sym(&out))
    }

    fn hash(&mut self, e: &mut Engine) -> RResult<Value> {
        let start = self.pos;
        match self.peek_at(1) {
            None => Err(ReadError::Incomplete),
            Some('(') => {
                self.pos += 2;
                let items = self.list_items(e, ')')?;
                Ok(e.vector(items, false))
            }
            Some('\\') => self.character(),
            Some('u') | Some('U') => {
                if self.peek_at(2) == Some('8') && self.peek_at(3) == Some('(') {
                    self.pos += 4;
                    self.bytevector(e)
                } else {
                    self.invalid(start, "expected `#u8(`")
                }
            }
            Some(c) if c == 't' || c == 'f' => {
                let tok = self.raw_token();
                match tok.as_str() {
                    "#t" | "#true" => Ok(Value::Bool(true)),
                    "#f" | "#false" => Ok(Value::Bool(false)),
                    other => self.invalid(start, format!("unknown literal `{other}`")),
                }
            }
            Some(c) if "eibodxEIBODX".contains(c) => {
                let tok = self.raw_token();
                match parse_number(&tok) {
                    Ok(Some(v)) => Ok(v),
                    Ok(None) => self.invalid(start, format!("`{tok}` is not a number")),
                    Err(msg) => self.invalid(start, msg),
                }
            }
            Some(other) => self.invalid(start, format!("unsupported `#{other}`")),
        }
    }

    fn bytevector(&mut self, e: &mut Engine) -> RResult<Value> {
        let start = self.pos;
        let items = self.list_items(e, ')')?;
        let mut bytes = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::Fixnum(n) if (0..=255).contains(&n) => bytes.push(n as u8),
                _ => return self.invalid(start, "bytevector elements must be 0..255"),
            }
        }
        Ok(e.bytevector(bytes))
    }

    fn character(&mut self) -> RResult<Value> {
        let start = self.pos;
        self.pos += 2; // `#\`
        let first = match self.bump() {
            None => return Err(ReadError::Incomplete),
            Some(c) => c,
        };
        // A single character, unless a name follows.
        let mut name = String::new();
        name.push(first);
        if first.is_alphanumeric() {
            while let Some(c) = self.peek() {
                if Self::is_delimiter(c) {
                    break;
                }
                name.push(c);
                self.pos += 1;
            }
        }
        if name.chars().count() == 1 {
            return Ok(Value::Char(first));
        }
        let named = match name.to_ascii_lowercase().as_str() {
            "space" => Some(' '),
            "newline" => Some('\n'),
            "tab" => Some('\t'),
            "return" => Some('\r'),
            "nul" | "null" => Some('\0'),
            "alarm" => Some('\u{7}'),
            "backspace" => Some('\u{8}'),
            "delete" => Some('\u{7f}'),
            "escape" => Some('\u{1b}'),
            _ => None,
        };
        if let Some(c) = named {
            return Ok(Value::Char(c));
        }
        if let Some(hex) = name.strip_prefix('x').or_else(|| name.strip_prefix('X'))
            && let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        {
            return Ok(Value::Char(c));
        }
        self.invalid(start, format!("unknown character name `#\\{name}`"))
    }

    /// Everything up to the next delimiter, `#` included if we are sitting on
    /// one.
    fn raw_token(&mut self) -> String {
        let mut out = String::new();
        if self.peek() == Some('#') {
            out.push('#');
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            if Self::is_delimiter(c) {
                break;
            }
            out.push(c);
            self.pos += 1;
        }
        out
    }

    /// A number, a keyword, or a symbol.
    fn token(&mut self, e: &mut Engine) -> RResult<Value> {
        let start = self.pos;
        let tok = self.raw_token();
        if tok.is_empty() {
            return self.invalid(start, "empty token");
        }
        match parse_number(&tok) {
            Ok(Some(v)) => return Ok(v),
            Err(msg) => return self.invalid(start, msg),
            Ok(None) => {}
        }
        if self.opts.colon_keywords && tok.len() > 1 && tok.starts_with(':') {
            return Ok(e.kw(&tok[1..]));
        }
        Ok(e.sym(&tok))
    }
}

/// `Ok(None)` means "not a number, try something else". `Err` means "meant to
/// be a number and is wrong", which must not silently become a symbol.
fn parse_number(tok: &str) -> Result<Option<Value>, String> {
    let mut radix: Option<u32> = None;
    let mut exact: Option<bool> = None;
    let mut rest = tok;

    while let Some(stripped) = rest.strip_prefix('#') {
        let mut cs = stripped.chars();
        let c = match cs.next() {
            None => return Err("`#` with no prefix letter".into()),
            Some(c) => c.to_ascii_lowercase(),
        };
        match c {
            'b' => radix = Some(2),
            'o' => radix = Some(8),
            'd' => radix = Some(10),
            'x' => radix = Some(16),
            'e' => exact = Some(true),
            'i' => exact = Some(false),
            other => return Err(format!("unknown number prefix `#{other}`")),
        }
        rest = &stripped[c.len_utf8()..];
    }
    let prefixed = radix.is_some() || exact.is_some();
    let radix = radix.unwrap_or(10);

    if rest.is_empty() {
        return if prefixed {
            Err("number prefix with no digits".into())
        } else {
            Ok(None)
        };
    }

    // Exact rationals are not supported: `ratios` is not claimed (plan §2.3),
    // so say so rather than reading `1/2` as something else.
    if rest.contains('/') && rest.chars().any(|c| c.is_ascii_digit()) {
        return Err(format!(
            "`{rest}`: exact rationals are not supported (the `ratios` feature is absent)"
        ));
    }

    match rest {
        "+inf.0" => return Ok(Some(Value::Flonum(f64::INFINITY))),
        "-inf.0" => return Ok(Some(Value::Flonum(f64::NEG_INFINITY))),
        "+nan.0" | "-nan.0" => return Ok(Some(Value::Flonum(f64::NAN))),
        _ => {}
    }

    // Integers, in any radix. The sign is handed to `from_str_radix` rather
    // than applied afterwards, because i64::MIN has no positive magnitude --
    // parsing 9223372036854775808 and negating it overflows.
    let (negative, digits) = match rest.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, rest.strip_prefix('+').unwrap_or(rest)),
    };
    if digits.is_empty() {
        // A bare `+` or `-` is the symbol, not a number.
        return if prefixed {
            Err("number prefix with no digits".into())
        } else {
            Ok(None)
        };
    }
    if digits.chars().all(|c| c.is_digit(radix)) {
        let signed = if negative {
            format!("-{digits}")
        } else {
            digits.to_string()
        };
        return match i64::from_str_radix(&signed, radix) {
            Ok(n) => {
                Ok(Some(if exact == Some(false) {
                    Value::Flonum(n as f64)
                } else {
                    Value::Fixnum(n)
                }))
            }
            // Settled in §4.5: overflow is an error, not a silent widening to
            // a flonum. Losing exactness quietly is how a render comes out
            // wrong instead of a stack trace.
            Err(_) => Err(format!(
                "integer literal `{rest}` does not fit in a 64-bit exact integer"
            )),
        };
    }

    // Decimals are radix 10 only.
    if radix != 10 {
        return if prefixed {
            Err(format!("`{rest}` is not a valid radix-{radix} integer"))
        } else {
            Ok(None)
        };
    }

    // Does it even look like a number? `1.5`, `.5`, `1e3`, `-2.5e-7`.
    let looks_numeric = rest.chars().next().is_some_and(|c| {
        c.is_ascii_digit() || c == '+' || c == '-' || c == '.'
    }) && rest.chars().any(|c| c.is_ascii_digit());
    if !looks_numeric {
        return Ok(None);
    }
    match rest.parse::<f64>() {
        Ok(x) => {
            if exact == Some(true) {
                if x.fract() == 0.0 && x.abs() < (i64::MAX as f64) {
                    Ok(Some(Value::Fixnum(x as i64)))
                } else {
                    Err(format!(
                        "`#e{rest}` would need an exact rational, and the `ratios` feature is absent"
                    ))
                }
            } else {
                Ok(Some(Value::Flonum(x)))
            }
        }
        // `1.2.3` and friends: it started like a number and is not one.
        Err(_) => {
            if prefixed {
                Err(format!("`{rest}` is not a number"))
            } else {
                Ok(None)
            }
        }
    }
}
