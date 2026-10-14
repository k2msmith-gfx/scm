//! The datum reader.
//!
//! Two things here are decisions rather than mechanics, both from plan §4:
//!
//!   * `:at` reads as a **keyword**, not a symbol. That is the one deliberate
//!     deviation from R7RS datum syntax -- `:` is a legal identifier initial,
//!     so `:at` is a legal R7RS symbol -- and `ReaderOptions::colon_keywords`
//!     turns it off, which is how the conformance suite will run. `#:at` is
//!     *also* accepted, and never emitted (plan §4.9): a model writing keyword
//!     arguments emits Racket's and Guile's spelling from habit, and
//!     LLM-generated scripts are a stated consumer.
//!   * `[a b c]` reads as a vector flagged `bracket`, because brackets
//!     *evaluate* their elements while `#(a b c)` is a constant. R7RS reserves
//!     `[ ] { }` for future extensions, so this costs no conformance at all.
//!
//! And one obligation that is an exit criterion rather than a nicety: a
//! truncated datum reports `Incomplete`, never `Invalid`. The REPL's
//! multi-line entry and the editor panes' paren handling both rest on being
//! able to tell "keep typing" from "that is wrong".

use std::collections::HashMap;
use std::rc::Rc;

use crate::heap::{Engine, Loc, Obj};
use crate::value::ObjRef;
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
    /// What to call this input in a location: `"<repl:7>"`, or a file path.
    ///
    /// An `Rc` so every form read from one input shares the one string --
    /// there is an entry per form, and a `String` each would be the same text
    /// hundreds of times.
    source: Rc<str>,
    /// Character offsets where each line begins, built once on first use so
    /// a location is a binary search rather than a rescan. Without it,
    /// recording a location per form would make reading quadratic.
    line_starts: Vec<usize>,
    /// `#N=` labels seen so far, for `#N#` to refer back to.
    ///
    /// Per *datum*, not per reader: R7RS scopes a label to the datum it
    /// appears in, so two successive top-level forms may both use `#0=`.
    labels: HashMap<u32, Value>,
    /// Set by `#!fold-case`, cleared by `#!no-fold-case`.
    fold_case: bool,
}

type RResult<T> = Result<T, ReadError>;

impl Reader {
    pub fn new(src: &str) -> Self {
        Self::with_options(src, ReaderOptions::default())
    }

    /// Named, so a backtrace can say *which* input a form came from.
    pub fn named(src: &str, source: &str) -> Self {
        let mut r = Self::with_options(src, ReaderOptions::default());
        r.source = Rc::from(source);
        r
    }

    pub fn with_options(src: &str, opts: ReaderOptions) -> Self {
        let chars: Vec<char> = src.chars().collect();
        let mut line_starts = vec![0usize];
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            src: chars,
            pos: 0,
            opts,
            source: Rc::from("<input>"),
            line_starts,
            labels: HashMap::new(),
            fold_case: false,
        }
    }

    /// The 1-based line and column of a character offset.
    fn loc_at(&self, offset: usize) -> Loc {
        // The last line start at or before `offset`.
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Loc {
            source: Rc::clone(&self.source),
            line: line as u32 + 1,
            col: (offset - self.line_starts[line]) as u32 + 1,
        }
    }

    /// Read every datum in the input.
    pub fn read_all(src: &str, e: &mut Engine) -> RResult<Vec<Value>> {
        Self::read_all_named(src, "<input>", e)
    }

    pub fn read_all_named(src: &str, source: &str, e: &mut Engine) -> RResult<Vec<Value>> {
        let mut r = Reader::named(src, source);
        let mut out = Vec::new();
        while let Some(v) = r.read(e)? {
            out.push(v);
        }
        Ok(out)
    }

    /// How many characters have been consumed. A port-backed `read` needs
    /// this to advance its cursor by exactly one datum.
    pub fn consumed(&self) -> usize {
        self.pos
    }

    /// The next datum, or `None` at end of input.
    pub fn read(&mut self, e: &mut Engine) -> RResult<Option<Value>> {
        self.skip_atmosphere()?;
        if self.at_end() {
            return Ok(None);
        }
        // A label is scoped to its datum, so two successive top-level forms
        // may each use `#0=` and mean different things.
        self.labels.clear();
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
            // `#!fold-case` and `#!no-fold-case` are R7RS reader
            // *directives*, not datums: they change how later symbols are
            // read and produce no value. So they belong here with the other
            // things that are skipped rather than in `hash`, which has to
            // return something.
            if self.peek() == Some('#') && self.peek_at(1) == Some('!') {
                const FOLD: &str = "#!fold-case";
                const NO_FOLD: &str = "#!no-fold-case";
                if self.matches_word(NO_FOLD) {
                    self.pos += NO_FOLD.chars().count();
                    self.fold_case = false;
                    continue;
                }
                if self.matches_word(FOLD) {
                    self.pos += FOLD.chars().count();
                    self.fold_case = true;
                    continue;
                }
            }
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

    /// Read one datum, and **remember where it was written** if it is a form.
    ///
    /// Only pairs and vectors get a location: a backtrace never points at a
    /// number, and an entry per atom would multiply the table for nothing.
    /// Note that this records the *outermost* cell of each form -- the inner
    /// cons cells of a list come from the spine loop, not from here -- so
    /// there is one entry per form, which is what a backtrace wants.
    fn datum(&mut self, e: &mut Engine) -> RResult<Value> {
        self.skip_datum_comments(e)?;
        let at = self.pos;
        let v = self.datum_inner(e)?;
        if let Value::Obj(r) = v
            && matches!(e.obj(r), Obj::Pair(..) | Obj::Vector { .. })
        {
            let loc = self.loc_at(at);
            e.record_loc(v, loc);
        }
        Ok(v)
    }

    fn datum_inner(&mut self, e: &mut Engine) -> RResult<Value> {
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
            // A lone `.` is only ever a list's tail marker, which `list`
            // handles before it ever gets here. Reaching it as a *datum*
            // means something asked for a datum and found a dot -- which is
            // exactly what `(a #;. b)` and `(#; #;x . z)` do, and R7RS makes
            // all three of those read errors rather than silently dropping
            // the dot.
            '.' if self.dot_is_tail_marker() => {
                self.invalid(start, "`.` where a datum was expected")
            }
            '#' => self.hash(e),
            '|' => self.pipe_symbol(e),
            _ => self.token(e),
        }
    }

    /// `#N= <datum>`, including when the datum refers to itself.
    ///
    /// A placeholder is registered under the label *before* the datum is
    /// read, so a `#N#` inside it has something to return; afterwards every
    /// occurrence of the placeholder is replaced by the real datum. That
    /// substitution is what makes `#0=(1 . #0#)` an actual cycle rather than
    /// a structure containing a stray marker.
    ///
    /// The walk is iterative with a visited set, because by the time it runs
    /// the structure may already be circular -- recursing here would be the
    /// fourth latent abort of exactly the same kind.
    fn labelled(&mut self, n: u32, start: usize, e: &mut Engine) -> RResult<Value> {
        let placeholder = e.cons(Value::Unspecified, Value::Unspecified);
        self.labels.insert(n, placeholder);
        self.skip_datum_comments(e)?;
        if self.at_end() {
            return Err(ReadError::Incomplete);
        }
        let datum = self.datum(e)?;
        if datum == placeholder {
            return self.invalid(start, format!("`#{n}=#{n}#` defines nothing"));
        }
        let Value::Obj(ph) = placeholder else {
            unreachable!("cons returns an object")
        };
        let mut seen: Vec<ObjRef> = Vec::new();
        let mut work = vec![datum];
        while let Some(v) = work.pop() {
            let Value::Obj(r) = v else { continue };
            if r == ph || seen.contains(&r) {
                continue;
            }
            seen.push(r);
            match e.obj(r).clone() {
                Obj::Pair(car, cdr) => {
                    let car = if car == placeholder { datum } else { car };
                    let cdr = if cdr == placeholder { datum } else { cdr };
                    *e.obj_mut(r) = Obj::Pair(car, cdr);
                    work.push(car);
                    work.push(cdr);
                }
                Obj::Vector { items, bracket } => {
                    let patched: Vec<Value> = items
                        .iter()
                        .map(|&i| if i == placeholder { datum } else { i })
                        .collect();
                    work.extend(patched.iter().copied());
                    *e.obj_mut(r) = Obj::Vector {
                        items: patched,
                        bracket,
                    };
                }
                _ => {}
            }
        }
        self.labels.insert(n, datum);
        Ok(datum)
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

    /// Does the input at the cursor spell `word`, followed by a delimiter?
    ///
    /// The delimiter check matters: `#!fold-caseX` is not the directive, and
    /// silently accepting its prefix would swallow the `X`.
    fn matches_word(&self, word: &str) -> bool {
        let chars: Vec<char> = word.chars().collect();
        if self.src.len() < self.pos + chars.len() {
            return false;
        }
        if self.src[self.pos..self.pos + chars.len()] != chars[..] {
            return false;
        }
        match self.src.get(self.pos + chars.len()) {
            None => true,
            Some(&c) => Self::is_delimiter(c),
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
                        // R7RS adds `\|`, so a symbol's delimiter can be
                        // written in a string the same way.
                        '|' => out.push('|'),
                        // A line continuation. R7RS §6.7 is
                        //
                        //     \ <intraline whitespace>* <line ending>
                        //       <intraline whitespace>*
                        //
                        // and only the middle of that was handled: a
                        // backslash immediately before a newline worked, but
                        // trailing spaces after the backslash -- which no
                        // editor shows and every editor leaves behind -- made
                        // it an "unknown string escape". Four conformance
                        // assertions, and the kind of thing that fails for a
                        // user exactly once and is then never trusted again.
                        ' ' | '\t' | '\n' | '\r' => {
                            self.pos -= 1; // put the whitespace back
                            while matches!(self.peek(), Some(' ' | '\t')) {
                                self.pos += 1;
                            }
                            match self.peek() {
                                Some('\n') => self.pos += 1,
                                Some('\r') => {
                                    self.pos += 1;
                                    if self.peek() == Some('\n') {
                                        self.pos += 1;
                                    }
                                }
                                None => return Err(ReadError::Incomplete),
                                Some(c) => {
                                    return self.invalid(
                                        start,
                                        format!("a `\\` continuation must reach the end of the line, found `{c}`"),
                                    );
                                }
                            }
                            while matches!(self.peek(), Some(' ' | '\t')) {
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
        let start = self.pos;
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
                    // `|H\x65;llo|` is the symbol `Hello`. Without this the
                    // escape came through literally as `x65;`.
                    Some('x') | Some('X') => {
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
                                return self.invalid(start, format!("bad hex escape in |…|: {hex}"));
                            }
                        }
                    }
                    Some(c) => out.push(c),
                },
                Some(c) => out.push(c),
            }
        }
        Ok(e.sym(&out))
    }

    /// The element-type name of a `#<tag>(` at the cursor, if that is what
    /// is there: `#f64(` -> `f64`. Looks only at the text, so the caller
    /// can use it to decide *whether* to take this branch at all.
    fn numvector_tag(&self) -> Option<&'static str> {
        for name in ["s8", "s16", "s32", "s64", "u16", "u32", "u64", "f32", "f64"] {
            let mut i = 1;
            let mut matched = true;
            for want in name.chars() {
                if self.peek_at(i).map(|c| c.to_ascii_lowercase()) != Some(want) {
                    matched = false;
                    break;
                }
                i += 1;
            }
            if matched && self.peek_at(i) == Some('(') {
                return Some(name);
            }
        }
        None
    }

    /// `#f64(1.0 2.0)`. `Ok(None)` when the text is not one of these, so
    /// the caller can report what it expected.
    fn numvector(&mut self, e: &mut Engine) -> RResult<Option<Value>> {
        let Some(name) = self.numvector_tag() else {
            return Ok(None);
        };
        let start = self.pos;
        self.pos += 2 + name.len();
        let items = self.list_items(e, ')')?;
        let tag = crate::heap::NumTag::from_name(name).expect("from the same list");
        let mut v = match crate::heap::NumVec::make(tag, items.len(), Value::Fixnum(0)) {
            Ok(v) => v,
            Err(m) => return self.invalid(start, m),
        };
        for (i, x) in items.iter().enumerate() {
            if let Err(m) = v.set(i, *x) {
                return self.invalid(start, m);
            }
        }
        Ok(Some(e.numvector(v)))
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
                } else if let Some(v) = self.numvector(e)? {
                    Ok(v)
                } else {
                    self.invalid(start, "expected `#u8(`, `#u16(`, `#u32(` or `#u64(`")
                }
            }
            // `#s8(`, `#s16(`, `#s32(`, `#s64(` -- the signed widths. A
            // bare `#s` is nothing else, so there is no ambiguity.
            Some('s') | Some('S') => match self.numvector(e)? {
                Some(v) => Ok(v),
                None => self.invalid(start, "expected `#s8(`, `#s16(`, `#s32(` or `#s64(`"),
            },
            // `#f32(` and `#f64(` come before `#f`: the longest match wins,
            // or every float vector would read as the boolean false.
            Some('f') | Some('F') if self.numvector_tag().is_some() => {
                match self.numvector(e)? {
                    Some(v) => Ok(v),
                    None => self.invalid(start, "expected `#f32(` or `#f64(`"),
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
            // `#:at`, accepted because a model will write it; the printer
            // only ever emits `:at` (plan §4.9).
            Some(':') => {
                self.pos += 2;
                let name = self.raw_token();
                if name.is_empty() {
                    return self.invalid(start, "`#:` with no keyword name");
                }
                Ok(e.kw(&name))
            }
            Some(c) if "eibodxEIBODX".contains(c) => {
                let tok = self.raw_token();
                match parse_number(&tok) {
                    Ok(Some(v)) => Ok(v),
                    Ok(None) => self.invalid(start, format!("`{tok}` is not a number")),
                    Err(msg) => self.invalid(start, msg),
                }
            }
            // `#N=` labels a datum and `#N#` refers back to it, which is how
            // a CYCLE is written. The writer already emits these -- it had to,
            // or `write` would spin on a cycle -- so until now the printer
            // produced output its own reader could not read. A round-trip
            // hole in a feature of mine, found by someone else's suite
            // because I had tested only the direction I wrote first.
            Some(d) if d.is_ascii_digit() => {
                // `hash` dispatches on `peek_at(1)`, so nothing is consumed
                // yet and `self.pos` still points at the `#`.
                self.pos += 1;
                let mut digits = String::new();
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        digits.push(c);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let Ok(n) = digits.parse::<u32>() else {
                    return self.invalid(start, format!("`#{digits}` is not a datum label"));
                };
                match self.bump() {
                    Some('#') => match self.labels.get(&n) {
                        Some(&v) => Ok(v),
                        None => self.invalid(start, format!("`#{n}#` has no `#{n}=`")),
                    },
                    Some('=') => self.labelled(n, start, e),
                    None => Err(ReadError::Incomplete),
                    Some(c) => {
                        self.invalid(start, format!("expected `=` or `#` after `#{n}`, found `{c}`"))
                    }
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
        if self.fold_case {
            return Ok(e.sym(&tok.to_lowercase()));
        }
        Ok(e.sym(&tok))
    }
}

/// Would this token read as a number? The writer asks, so that a symbol
/// spelled like one gets `|…|` rather than coming back as a number.
pub fn reads_as_number(tok: &str) -> bool {
    matches!(parse_number(tok), Ok(Some(_)))
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
    // so say so rather than reading `1/2` as something else. But only for a
    // token that *is* a rational -- digits on both sides of one slash. `n/2`
    // and `constant-pi/2` are identifiers, and the R7RS benchmarks use them.
    if let Some((num, den)) = rest.split_once('/') {
        let num = num.strip_prefix(['+', '-']).unwrap_or(num);
        let is_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_digit(radix));
        if is_digits(num) && is_digits(den) {
            return Err(format!(
                "`{rest}`: exact rationals are not supported (the `ratios` feature is absent)"
            ));
        }
    }

    // R7RS spells these in lower case but says nothing about matching case,
    // and the suite writes `+InF.0`.
    match rest.to_ascii_lowercase().as_str() {
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
    // `s`, `f`, `d` and `l` as exponent markers. R7RS says support for them
    // is no longer *required*, and they cost three lines -- so a model or an
    // older file that writes `1d2` gets 100.0 rather than a symbol.
    let rest: String = {
        let mut out = String::with_capacity(rest.len());
        let mut marked = false;
        for c in rest.chars() {
            if !marked
                && matches!(c, 's' | 'S' | 'f' | 'F' | 'd' | 'D' | 'l' | 'L')
                && out.chars().any(|p| p.is_ascii_digit())
            {
                out.push('e');
                marked = true;
            } else {
                out.push(c);
            }
        }
        out
    };
    let rest = rest.as_str();
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
