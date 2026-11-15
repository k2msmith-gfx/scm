//! **What each name in a buffer refers to**, as far as the text can say:
//! every FREE use of a name -- one no local binding in the text captures --
//! with, at the head of a call, how many arguments the call passes. Find
//! references and the editor's warnings stand on it (ray
//! docs/editor-crate-plan.md, E8): a reference to a top-level `slab` is
//! where `slab` is used free, not where the word appears, and a name used
//! free that nothing defines is the typo worth underlining.
//!
//! **Generous with bindings, by design.** A name taken to be bound when it
//! is not costs a missed warning; a name taken to be free when a binding
//! form captured it costs a FALSE warning, and an editor that cries wolf is
//! ignored. So every binding form below binds as widely as it could -- a
//! `let*`'s names in all its initialisers, every atom of a `match` pattern
//! -- and quoted data, `syntax-rules` and the library declarations are not
//! looked into at all. A form whose head is a macro the editor does not
//! know is reported (`Ref::under`) for the caller to decide: a macro chooses
//! what its arguments mean.
//!
//! Built on the token scan rather than the reader for the reason [`crate::text`]
//! gives: a buffer being edited mostly does not read. An unclosed form runs
//! to the end of the buffer.

use crate::text::{Doc, Kind};
use std::collections::HashSet;

/// One free use of a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub name: String,
    /// Character offsets of the name.
    pub start: usize,
    pub end: usize,
    /// At the head of a call: the call's arguments.
    pub call: Option<Call>,
    /// The heads of the enclosing forms this pass does not know (a call, or
    /// a macro it cannot tell from one), innermost first. A caller that
    /// knows one of them is a macro may choose not to judge what is inside.
    pub under: Vec<String>,
}

/// A call's arguments, as an arity check needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    /// Every argument.
    pub total: usize,
    /// The arguments before the first keyword (`:radius`) — which in ray is
    /// as often a NAME passed as a value (`(plate :main-plate …)`) as a
    /// keyword argument, so a caller asks the procedure which it takes.
    pub positional: usize,
    /// Whether a keyword follows them.
    pub keywords: bool,
}

/// The text as forms: what the binding forms are matched against.
#[derive(Debug, Clone)]
enum Node {
    Atom { text: String, start: usize, end: usize },
    List(Vec<Node>),
    /// A string, or a datum under `'` / `#;`: no names in it.
    Data,
    /// `#(…)`: data as an expression, but a `match` pattern binds the names
    /// in one, and a quasiquoted one may hold `,x`.
    Vector(Vec<Node>),
    /// `` `x ``: data, but its `,` and `,@` parts are code.
    Quasi(Box<Node>),
    /// `,x` or `,@x` -- code inside a quasiquote.
    Unquote(Box<Node>),
}

/// Every free use of a name in DOC.
pub fn references(doc: &Doc) -> Vec<Ref> {
    let forms = parse(doc);
    let mut walk = Walk { out: Vec::new(), scopes: Vec::new(), under: Vec::new() };
    // Top-level definitions are GLOBAL, not local: a use of one is a
    // reference to it (what find references wants), and whether the name is
    // defined is the caller's question, answered from the whole file.
    for f in &forms {
        walk.toplevel(f);
    }
    walk.out
}

fn parse(doc: &Doc) -> Vec<Node> {
    let toks: Vec<_> = doc.tokens().iter().filter(|t| t.kind != Kind::Comment).copied().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < toks.len() {
        if let Some(n) = node(doc, &toks, &mut i) {
            out.push(n);
        }
    }
    out
}

/// The datum starting at `toks[*i]`, moving `*i` past it. None for a stray
/// closer.
fn node(doc: &Doc, toks: &[crate::text::Token], i: &mut usize) -> Option<Node> {
    let t = toks[*i];
    *i += 1;
    match t.kind {
        Kind::Close | Kind::Comment => None,
        Kind::Str => Some(Node::Data),
        Kind::Open => {
            let mut kids = Vec::new();
            while *i < toks.len() && toks[*i].kind != Kind::Close {
                if let Some(n) = node(doc, toks, i) {
                    kids.push(n);
                }
            }
            *i += 1; // the closer, or past the end
            Some(Node::List(kids))
        }
        Kind::Prefix => {
            let p = doc.slice(t.start, t.end);
            let inner = if *i < toks.len() { node(doc, toks, i) } else { None };
            match p.as_str() {
                "`" => Some(Node::Quasi(Box::new(inner.unwrap_or(Node::Data)))),
                "," | ",@" => Some(Node::Unquote(Box::new(inner.unwrap_or(Node::Data)))),
                // `'` and `#;`: the datum is not code.
                _ => Some(Node::Data),
            }
        }
        Kind::Atom => {
            let text = doc.slice(t.start, t.end);
            // `#(` and `#u8(`: the lexer gives the `#…` and the bracket
            // apart; the vector's contents are data.
            if text.starts_with('#') && *i < toks.len() && toks[*i].kind == Kind::Open && toks[*i].start == t.end {
                return match node(doc, toks, i) {
                    Some(Node::List(kids)) => Some(Node::Vector(kids)),
                    _ => Some(Node::Data),
                };
            }
            Some(Node::Atom { text, start: t.start, end: t.end })
        }
    }
}

/// Is this atom a NAME -- as opposed to a number, a boolean, a character, a
/// keyword, or one of the auxiliary words a form reads by position?
fn is_name(text: &str) -> bool {
    let Some(c) = text.chars().next() else { return false };
    if c == '#' || c == ':' || c == '"' {
        return false;
    }
    if text.ends_with(':') && text.len() > 1 {
        return false; // `radius:` keyword spelling
    }
    // `+inf.0`, `-nan.0`: numbers, though they start like names.
    if matches!(text, "+inf.0" | "-inf.0" | "+nan.0" | "-nan.0") {
        return false;
    }
    if c.is_ascii_digit() || ((c == '+' || c == '-' || c == '.') && text.chars().nth(1).is_some_and(|d| d.is_ascii_digit() || d == '.')) {
        return false;
    }
    !matches!(text, "." | "..." | "_" | "else" | "=>" | "&opt" | "&named" | "&keys" | "&rest" | "unquote" | "unquote-splicing")
}

fn is_keyword(text: &str) -> bool {
    text.starts_with(':') || text.starts_with("#:") || (text.len() > 1 && text.ends_with(':'))
}

struct Walk {
    out: Vec<Ref>,
    scopes: Vec<HashSet<String>>,
    under: Vec<String>,
}

impl Walk {
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }

    fn reference(&mut self, text: &str, start: usize, end: usize, call: Option<Call>) {
        if is_name(text) && !self.bound(text) {
            let under = self.under.iter().rev().cloned().collect();
            self.out.push(Ref { name: text.to_string(), start, end, call, under });
        }
    }

    /// A top-level form: a definition's name is global, the rest is code.
    fn toplevel(&mut self, n: &Node) {
        self.expr(n);
    }

    /// Run `f` with NAMES bound.
    fn with<F: FnOnce(&mut Walk)>(&mut self, names: HashSet<String>, f: F) {
        self.scopes.push(names);
        f(self);
        self.scopes.pop();
    }

    fn exprs(&mut self, ns: &[Node]) {
        for n in ns {
            self.expr(n);
        }
    }

    /// A body: its internal definitions bind throughout it (letrec*), so
    /// they are gathered before any of it is walked.
    fn body(&mut self, ns: &[Node]) {
        let mut names = HashSet::new();
        for n in ns {
            defined_names(n, &mut names);
        }
        self.with(names, |w| w.exprs(ns));
    }

    fn expr(&mut self, n: &Node) {
        match n {
            Node::Atom { text, start, end } => self.reference(text, *start, *end, None),
            Node::Data | Node::Vector(_) => {}
            Node::Quasi(inner) => self.quasi(inner),
            // A stray `,` outside a quasiquote: walk it as code all the same.
            Node::Unquote(inner) => self.expr(inner),
            Node::List(kids) => self.form(kids),
        }
    }

    fn quasi(&mut self, n: &Node) {
        match n {
            Node::Unquote(inner) => self.expr(inner),
            Node::List(kids) | Node::Vector(kids) => kids.iter().for_each(|k| self.quasi(k)),
            Node::Quasi(inner) => self.quasi(inner),
            _ => {}
        }
    }

    fn form(&mut self, kids: &[Node]) {
        let Some(head) = kids.first() else { return };
        let Node::Atom { text: head_text, start, end } = head else {
            // `((f x) y)`: every part is code.
            return self.exprs(kids);
        };
        let rest = &kids[1..];
        // A local binding of the head's name makes it a call, whatever the
        // name would otherwise be.
        let special = if self.bound(head_text) { "" } else { head_text.as_str() };
        match special {
            "quote" | "syntax-rules" | "er-macro-transformer" | "define-library" | "import" | "export"
            | "cond-expand" | "define-record-type" | "include" | "include-ci" | "declare" => {}
            "quasiquote" => rest.iter().for_each(|k| self.quasi(k)),
            "define" | "define*" | "define-values" => self.define(rest),
            "define-syntax" | "let-syntax" | "letrec-syntax" => {
                // The transformers are not code to check; a let-syntax body
                // is, with its macros' names bound.
                if special != "define-syntax"
                    && let Some((bindings, body)) = rest.split_first()
                {
                    let mut names = HashSet::new();
                    binding_pairs(bindings, &mut names);
                    self.with(names, |w| w.body(body));
                }
            }
            "set!" => self.exprs(rest),
            "lambda" | "λ" | "lambda*" | "named-lambda" => {
                let (formals, body) = match (special, rest) {
                    ("named-lambda", [Node::List(f), body @ ..]) => (f.get(1..).unwrap_or(&[]).to_vec(), body),
                    (_, [f, body @ ..]) => (vec![f.clone()], body),
                    _ => return,
                };
                let mut names = HashSet::new();
                for f in &formals {
                    self.formals(f, &mut names);
                }
                self.with(names, |w| w.body(body));
            }
            "case-lambda" => {
                for clause in rest {
                    if let Node::List(c) = clause
                        && let Some((f, body)) = c.split_first()
                    {
                        let mut names = HashSet::new();
                        self.formals(f, &mut names);
                        self.with(names, |w| w.body(body));
                    }
                }
            }
            "let" | "let*" | "letrec" | "letrec*" | "fluid-let" => {
                // Named let: the name binds in the body.
                let (name, rest) = match rest {
                    [Node::Atom { text, .. }, more @ ..] => (Some(text.clone()), more),
                    _ => (None, rest),
                };
                let Some((bindings, body)) = rest.split_first() else { return };
                let mut names: HashSet<String> = name.into_iter().collect();
                let inits = binding_pairs(bindings, &mut names);
                let generous = special != "fluid-let";
                // Bound in the initialisers too (generous: let*, letrec);
                // a plain let's initialisers then see its names, which only
                // ever hides a warning.
                if generous {
                    self.with(names.clone(), |w| w.exprs(&inits));
                } else {
                    self.exprs(&inits);
                }
                self.with(names, |w| w.body(body));
            }
            "let-values" | "let*-values" | "define-values-let" => {
                let Some((Node::List(bs), body)) = rest.split_first() else { return };
                let mut names = HashSet::new();
                let mut inits = Vec::new();
                for b in bs {
                    if let Node::List(pair) = b
                        && let [f, init, ..] = pair.as_slice()
                    {
                        self.formals(f, &mut names);
                        inits.push(init.clone());
                    }
                }
                self.with(names.clone(), |w| w.exprs(&inits));
                self.with(names, |w| w.body(body));
            }
            "receive" => {
                let [f, init, body @ ..] = rest else { return };
                self.expr(init);
                let mut names = HashSet::new();
                self.formals(f, &mut names);
                self.with(names, |w| w.body(body));
            }
            "do" => {
                let [Node::List(specs), test, body @ ..] = rest else { return };
                let mut names = HashSet::new();
                let mut inits = Vec::new();
                for s in specs {
                    if let Node::List(parts) = s {
                        if let Some(Node::Atom { text, .. }) = parts.first() {
                            names.insert(text.clone());
                        }
                        inits.extend(parts.iter().skip(1).cloned());
                    }
                }
                self.with(names, |w| {
                    w.exprs(&inits);
                    w.expr(test_as_list(test));
                    w.body(body);
                });
            }
            "if-let" | "when-let" | "and-let*" | "if-let*" | "when-let*" => {
                let Some((bindings, body)) = rest.split_first() else { return };
                let mut names = HashSet::new();
                let mut inits = Vec::new();
                match bindings {
                    // `(if-let (x e) …)`: one binding, unwrapped.
                    Node::List(b) if matches!(b.first(), Some(Node::Atom { .. })) && b.len() == 2 && special != "and-let*" => {
                        if let Node::Atom { text, .. } = &b[0] {
                            names.insert(text.clone());
                        }
                        inits.push(b[1].clone());
                    }
                    Node::List(bs) => {
                        for b in bs {
                            match b {
                                Node::List(pair) => match pair.as_slice() {
                                    [Node::Atom { text, .. }, init] => {
                                        names.insert(text.clone());
                                        inits.push(init.clone());
                                    }
                                    _ => inits.push(b.clone()),
                                },
                                // `and-let*`'s bare variable: a use.
                                other => inits.push(other.clone()),
                            }
                        }
                    }
                    _ => {}
                }
                self.with(names.clone(), |w| w.exprs(&inits));
                self.with(names, |w| w.body(body));
            }
            "cond" => {
                for clause in rest {
                    if let Node::List(parts) = clause {
                        self.exprs(parts);
                    }
                }
            }
            "case" => {
                let Some((key, clauses)) = rest.split_first() else { return };
                self.expr(key);
                for clause in clauses {
                    if let Node::List(parts) = clause {
                        // The datums are not code; the rest is.
                        self.exprs(parts.get(1..).unwrap_or(&[]));
                    }
                }
            }
            "guard" => {
                let Some((Node::List(spec), body)) = rest.split_first() else { return };
                let mut names = HashSet::new();
                if let Some(Node::Atom { text, .. }) = spec.first() {
                    names.insert(text.clone());
                }
                let clauses = spec.get(1..).unwrap_or(&[]).to_vec();
                self.with(names, |w| {
                    for c in &clauses {
                        if let Node::List(parts) = c {
                            w.exprs(parts);
                        }
                    }
                });
                self.body(body);
            }
            "parameterize" => {
                let Some((Node::List(bs), body)) = rest.split_first() else { return };
                for b in bs {
                    if let Node::List(pair) = b {
                        self.exprs(pair);
                    }
                }
                self.body(body);
            }
            "match" | "match*" => {
                let Some((subject, clauses)) = rest.split_first() else { return };
                self.expr(subject);
                self.match_clauses(clauses);
            }
            "match-lambda" | "match-lambda*" => self.match_clauses(rest),
            "match-let" | "match-let*" | "match-letrec" => {
                let Some((Node::List(bs), body)) = rest.split_first() else { return };
                let mut names = HashSet::new();
                let mut inits = Vec::new();
                for b in bs {
                    if let Node::List(pair) = b
                        && let [pat, init, ..] = pair.as_slice()
                    {
                        self.pattern(pat, &mut names);
                        inits.push(init.clone());
                    }
                }
                self.with(names.clone(), |w| w.exprs(&inits));
                self.with(names, |w| w.body(body));
            }
            "begin" | "when" | "unless" | "if" | "and" | "or" | "delay" | "delay-force" | "make-promise"
            | "assert" | "%try" | "try" => self.exprs(rest),
            _ => {
                // A call -- or a macro this pass does not know.
                let positional = rest.iter().take_while(|k| !matches!(k, Node::Atom { text, .. } if is_keyword(text))).count();
                let keywords = positional < rest.len();
                self.reference(head_text, *start, *end, Some(Call { total: rest.len(), positional, keywords }));
                self.under.push(head_text.clone());
                self.exprs(rest);
                self.under.pop();
            }
        }
    }

    /// A `define`'s name is global at top level -- the caller's business --
    /// and local in a body, where `body` gathered it already; its formals
    /// bind in its body.
    fn define(&mut self, rest: &[Node]) {
        match rest {
            // `(define (f a b) body…)`, curried `(define ((f a) b) …)`.
            [Node::List(sig), body @ ..] => {
                let mut names = HashSet::new();
                let mut sig = sig.clone();
                loop {
                    let tail: Vec<Node> = sig.get(1..).unwrap_or(&[]).to_vec();
                    // `(f a . rest)` comes through as atoms with a `.`.
                    self.formals(&Node::List(tail), &mut names);
                    match sig.first() {
                        Some(Node::List(inner)) => sig = inner.clone(),
                        _ => break,
                    }
                }
                self.with(names, |w| w.body(body));
            }
            [_, value @ ..] => self.exprs(value),
            _ => {}
        }
    }

    /// The names a lambda list binds: atoms, `[b default]` / `(b default)`
    /// optionals whose defaults are code, `:at at` keyword parameters, a
    /// dotted rest. Defaults see the parameters (generous).
    fn formals(&mut self, f: &Node, names: &mut HashSet<String>) {
        match f {
            Node::Atom { text, .. } => {
                if is_name(text) {
                    names.insert(text.clone());
                }
            }
            Node::List(parts) => {
                let mut defaults = Vec::new();
                for p in parts {
                    match p {
                        Node::Atom { text, .. } if is_name(text) => {
                            names.insert(text.clone());
                        }
                        Node::List(opt) => {
                            if let Some(Node::Atom { text, .. }) = opt.first() {
                                names.insert(text.clone());
                            }
                            defaults.extend(opt.iter().skip(1).cloned());
                        }
                        _ => {}
                    }
                }
                let ns = names.clone();
                self.with(ns, |w| w.exprs(&defaults));
            }
            _ => {}
        }
    }

    fn match_clauses(&mut self, clauses: &[Node]) {
        for clause in clauses {
            let Node::List(parts) = clause else { continue };
            let Some((pat, body)) = parts.split_first() else { continue };
            let mut names = HashSet::new();
            self.pattern(pat, &mut names);
            self.with(names, |w| w.body(body));
        }
    }

    /// A `match` pattern (SRFI 204): every name in it binds, except where a
    /// pattern operator takes an expression -- `(? pred x)`, `(= proc x)`.
    fn pattern(&mut self, p: &Node, names: &mut HashSet<String>) {
        match p {
            Node::Atom { text, .. } => {
                if is_name(text) {
                    names.insert(text.clone());
                }
            }
            Node::List(parts) => match parts.first() {
                Some(Node::Atom { text, .. }) if text == "?" || text == "=" => {
                    if let Some(e) = parts.get(1) {
                        self.expr(e);
                    }
                    for q in parts.iter().skip(2) {
                        self.pattern(q, names);
                    }
                }
                Some(Node::Atom { text, .. }) if matches!(text.as_str(), "and" | "or" | "not" | "quote" | "$" | "struct" | "@" | "object") => {
                    for q in parts.iter().skip(1) {
                        self.pattern(q, names);
                    }
                }
                _ => parts.iter().for_each(|q| self.pattern(q, names)),
            },
            Node::Vector(parts) => parts.iter().for_each(|q| self.pattern(q, names)),
            Node::Quasi(inner) => self.quasi_pattern(inner, names),
            _ => {}
        }
    }

    fn quasi_pattern(&mut self, p: &Node, names: &mut HashSet<String>) {
        match p {
            Node::Unquote(inner) => self.pattern(inner, names),
            Node::List(kids) | Node::Vector(kids) => kids.iter().for_each(|k| self.quasi_pattern(k, names)),
            _ => {}
        }
    }
}

/// A `do`'s test clause is `(test result…)`: all code.
fn test_as_list(n: &Node) -> &Node {
    n
}

/// The `(name init)` pairs of a `let`: their names into NAMES, their
/// initialisers out.
fn binding_pairs(bindings: &Node, names: &mut HashSet<String>) -> Vec<Node> {
    let mut inits = Vec::new();
    if let Node::List(bs) = bindings {
        for b in bs {
            match b {
                Node::List(pair) => {
                    if let Some(Node::Atom { text, .. }) = pair.first() {
                        names.insert(text.clone());
                    }
                    inits.extend(pair.iter().skip(1).cloned());
                }
                Node::Atom { text, .. } => {
                    names.insert(text.clone());
                }
                _ => {}
            }
        }
    }
    inits
}

/// The names a body-level form defines: `define`, `define-values`,
/// `define-record-type`'s names, `define-syntax`, and `begin`'s.
fn defined_names(n: &Node, names: &mut HashSet<String>) {
    let Node::List(kids) = n else { return };
    let Some(Node::Atom { text: head, .. }) = kids.first() else { return };
    match head.as_str() {
        "define" | "define*" | "define-syntax" => {
            let mut target = kids.get(1);
            while let Some(Node::List(sig)) = target {
                target = sig.first();
            }
            if let Some(Node::Atom { text, .. }) = target {
                names.insert(text.clone());
            }
        }
        "define-values" => {
            if let Some(Node::List(fs)) = kids.get(1) {
                for f in fs {
                    if let Node::Atom { text, .. } = f
                        && is_name(text)
                    {
                        names.insert(text.clone());
                    }
                }
            }
        }
        "define-record-type" => {
            // Every atom in it is a name it binds or a field: generous.
            fn atoms(n: &Node, names: &mut HashSet<String>) {
                match n {
                    Node::Atom { text, .. } if is_name(text) => {
                        names.insert(text.clone());
                    }
                    Node::List(k) => k.iter().for_each(|x| atoms(x, names)),
                    _ => {}
                }
            }
            kids.iter().skip(1).for_each(|k| atoms(k, names));
        }
        "begin" => kids.iter().skip(1).for_each(|k| defined_names(k, names)),
        _ => {}
    }
}

/// The names DOC defines at top level, for a caller deciding what is
/// defined: everything [`Doc::definitions`] lists, plus what a top-level
/// `begin` holds.
pub fn toplevel_names(doc: &Doc) -> HashSet<String> {
    let mut names: HashSet<String> = doc.definitions().into_iter().map(|d| d.name).collect();
    for f in parse(doc) {
        defined_names(&f, &mut names);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free(text: &str) -> Vec<String> {
        references(&Doc::new(text)).into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn a_local_binding_captures_its_name() {
        assert_eq!(free("(define (f x) (+ x y))"), vec!["+", "y"]);
        assert_eq!(free("(let ((a 1) (b a)) (list a b c))"), vec!["list", "c"]);
        assert_eq!(free("(let loop ((i 0)) (if (< i n) (loop (+ i 1))))"), vec!["<", "n", "+"]);
        assert_eq!(free("(lambda (a [b 2] :at at . rest) (g a b at rest))"), vec!["g"]);
        assert_eq!(free("(define (f) (define (g) h) (g))"), vec!["h"], "an internal define binds in its body");
    }

    #[test]
    fn data_is_not_code() {
        assert_eq!(free("(list 'a '(b c) \"d\" #\\e #t 1.5 -2 :k #(v w))"), vec!["list"]);
        assert_eq!(free("`(a ,b ,@(c d))"), vec!["b", "c", "d"]);
        assert_eq!(free("(case x ((a b) (f)) (else g))"), vec!["x", "f", "g"]);
        assert_eq!(free("(cond ((p x) => q) (else r))"), vec!["p", "x", "q", "r"]);
        assert_eq!(free("(define-syntax m (syntax-rules () ((_ a) (zz a))))"), Vec::<String>::new());
        assert_eq!(free("(let-syntax ((p (syntax-rules () ((_ y) y)))) (p q))"), vec!["q"]);
        assert_eq!(free("(max +inf.0 -nan.0 x)"), vec!["max", "x"]);
    }

    #[test]
    fn the_binding_forms_bind() {
        assert_eq!(free("(match v ((a . b) (f a b)) ((? pair? c) c) (_ d))"), vec!["v", "f", "pair?", "d"]);
        assert_eq!(free("(match p (#(x y z) (vector x y z w)))"), vec!["p", "vector", "w"], "a vector pattern binds");
        assert_eq!(free("(list #(a b) `#(c ,d))"), vec!["list", "d"]);
        assert_eq!(free("(do ((i 0 (+ i 1))) ((= i n) acc) (s i))"), vec!["+", "=", "n", "acc", "s"]);
        assert_eq!(free("(if-let ((x (g))) x y)"), vec!["g", "y"]);
        assert_eq!(free("(and-let* ((x (g)) ((ok? x))) x)"), vec!["g", "ok?"]);
        assert_eq!(free("(receive (a . b) (vals) (f a b))"), vec!["vals", "f"]);
        assert_eq!(free("(let-values (((a b) (vals))) (f a b))"), vec!["vals", "f"]);
        assert_eq!(free("(guard (e ((string? e) e)) (risky))"), vec!["string?", "risky"]);
        assert_eq!(free("(case-lambda ((a) a) ((a b) (+ a b)))"), vec!["+"]);
        assert_eq!(free("(define-record-type point (make-point x y) point? (x point-x))"), Vec::<String>::new());
    }

    #[test]
    fn a_call_counts_its_arguments() {
        let r = references(&Doc::new("(add-sphere \"s\" :radius 1)\n(cube 1 2)"));
        assert_eq!(r[0].call, Some(Call { total: 3, positional: 1, keywords: true }));
        assert_eq!(r[1].call, Some(Call { total: 2, positional: 2, keywords: false }));
        assert_eq!(r[1].start, 28);
    }

    #[test]
    fn a_use_inside_an_unknown_form_says_whose() {
        let r = references(&Doc::new("(with-scene s (box 1))"));
        let inner: Vec<(&str, Vec<String>)> = r.iter().map(|r| (r.name.as_str(), r.under.clone())).collect();
        assert_eq!(inner, vec![("with-scene", vec![]), ("s", vec!["with-scene".into()]), ("box", vec!["with-scene".into()])]);
    }

    #[test]
    fn a_half_typed_buffer_still_answers() {
        assert_eq!(free("(define (f x)\n  (g x"), vec!["g"]);
    }

    #[test]
    fn toplevel_names_include_a_begins() {
        let names = toplevel_names(&Doc::new("(define a 1) (begin (define b 2)) (define (c) 3)"));
        assert!(["a", "b", "c"].iter().all(|n| names.contains(*n)), "{names:?}");
    }
}

/// The bar before any warning is drawn (ray docs/editor-crate-plan.md, E8):
/// over code that runs, every free name is bound by the image or defined
/// in the corpus. SCOPE_CORPUS is directories (`:`-separated) of `.scm`
/// files; SCOPE_NAMES a file of the image's names, one a line (its
/// `(apropos)` and `(%syntax-names)`). Prints what would be warned about.
/// `SCOPE_CORPUS=… SCOPE_NAMES=… cargo test -p scm-text scope_corpus -- --ignored --nocapture`
#[cfg(test)]
mod corpus {
    use super::*;
    use std::collections::HashMap;

    fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                files(&p, out);
            } else if p.extension().is_some_and(|x| x == "scm") {
                out.push(p);
            }
        }
    }

    #[test]
    #[ignore]
    fn scope_corpus() {
        let dirs = std::env::var("SCOPE_CORPUS").expect("SCOPE_CORPUS");
        let names: HashSet<String> = std::fs::read_to_string(std::env::var("SCOPE_NAMES").expect("SCOPE_NAMES"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let mut paths = Vec::new();
        for d in dirs.split(':') {
            files(std::path::Path::new(d), &mut paths);
        }
        let docs: Vec<(std::path::PathBuf, Doc)> =
            paths.into_iter().filter_map(|p| std::fs::read_to_string(&p).ok().map(|t| (p, Doc::new(&t)))).collect();
        let mut defined: HashSet<String> = HashSet::new();
        for (_, d) in &docs {
            defined.extend(toplevel_names(d));
        }
        let mut unbound: HashMap<String, Vec<String>> = HashMap::new();
        let mut refs = 0;
        for (p, d) in &docs {
            for r in references(d) {
                refs += 1;
                if names.contains(&r.name) || defined.contains(&r.name) || r.under.iter().any(|h| names.contains(h) && !is_call_known(h, &names)) {
                    continue;
                }
                let (line, _) = d.position(r.start);
                unbound.entry(r.name).or_default().push(format!("{}:{}", p.display(), line + 1));
            }
        }
        let mut list: Vec<_> = unbound.into_iter().collect();
        list.sort_by_key(|(n, v)| (std::cmp::Reverse(v.len()), n.clone()));
        eprintln!("{} files, {refs} free uses, {} names unbound", docs.len(), list.len());
        for (n, v) in list.iter().take(60) {
            eprintln!("  {n:<28} {:>4}  {}", v.len(), v[0]);
        }
    }

    /// The corpus run cannot tell a macro from a procedure by name alone;
    /// it errs as the editor will, treating every image name as possibly a
    /// macro head only when it is not a known procedure. Here, none is
    /// known, so any image head silences what is under it.
    fn is_call_known(_h: &str, _names: &HashSet<String>) -> bool {
        false
    }
}
