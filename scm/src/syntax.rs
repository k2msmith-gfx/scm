//! `syntax-rules`: pattern matching, template expansion, and hygiene.
//!
//! ## Hygiene, and why it is renaming rather than sets-of-scopes
//!
//! Plan §4 names sets-of-scopes (Flatt's model, as in Racket) as the
//! destination, because it is the only model that extends cleanly to
//! procedural macros. This is the smaller mechanism that gets the same
//! guarantees for `syntax-rules` alone: **every template-introduced identifier
//! is renamed to a symbol no source text can name, with an alias back to what
//! it meant where the macro was defined.**
//!
//! One rule, two jobs:
//!
//!   * A template *reference* -- the `if` in `(if test …)` inside a template --
//!     becomes `if.123`, which nothing can shadow, and resolves through its
//!     alias to the `if` of the definition site. That is referential
//!     transparency.
//!   * A template *binder* -- the `t` in `(let ((t …)) …)` -- becomes `t.124`,
//!     and the template's own references to `t` become the same `t.124`, so
//!     they find each other. A user's `t` is a different symbol entirely and
//!     cannot be captured. That is hygiene.
//!
//! Pattern variables substitute the *user's* syntax untouched, so the user's
//! identifiers keep meaning what they meant at the call site.
//!
//! ## Brackets are paren-ish for matching
//!
//! `(let ([x 1] [y 2]) …)` is the shape a model writes, because it is
//! Racket/R6RS house style (plan §4.9). Those brackets are *vector
//! constructors* when evaluated (§4.5), so a pattern of `((name val) …)`
//! would not match them. Rather than teach every binding macro to accept
//! both, the **matcher** treats a bracket vector as a list -- one place, and
//! every binding and clause form inherits it. A `#(…)` vector is not
//! paren-ish, which is exactly the distinction the reader's `bracket` flag
//! already records.

use std::collections::HashMap;
use std::rc::Rc;

use crate::heap::{Engine, Obj};
use crate::value::{ObjRef, SymId, Value};

pub struct SyntaxRules {
    pub name: SymId,
    literals: Vec<SymId>,
    ellipsis: SymId,
    rules: Vec<(Value, Value)>,
    /// The environment the macro was defined in. Template references resolve
    /// here, not at the call site.
    def_env: Option<ObjRef>,
}

#[derive(Clone, Debug)]
enum Binding {
    One(Value),
    Many(Vec<Binding>),
}

type Bindings = HashMap<SymId, Binding>;

pub type SynResult<T> = Result<T, String>;

/// Everything in a form that could be a list, as a `Vec` plus an improper
/// tail. A bracket vector reads as a proper list here; `#(…)` does not.
fn as_list(e: &Engine, v: Value) -> Option<(Vec<Value>, Value)> {
    match v {
        Value::Null => Some((Vec::new(), Value::Null)),
        Value::Obj(r) => match e.obj(r) {
            Obj::Pair(..) => {
                let mut items = Vec::new();
                let mut cur = v;
                loop {
                    match cur {
                        Value::Obj(r2) => match e.obj(r2) {
                            Obj::Pair(car, cdr) => {
                                items.push(*car);
                                cur = *cdr;
                            }
                            _ => return Some((items, cur)),
                        },
                        _ => return Some((items, cur)),
                    }
                }
            }
            // The §4.9 rule.
            Obj::Vector {
                items,
                bracket: true,
            } => Some((items.clone(), Value::Null)),
            _ => None,
        },
        _ => None,
    }
}

impl SyntaxRules {
    /// Parse `(syntax-rules (literal …) (pattern template) …)`, or the R7RS
    /// form with a custom ellipsis identifier first.
    pub fn parse(
        e: &mut Engine,
        name: SymId,
        spec: Value,
        def_env: Option<ObjRef>,
    ) -> SynResult<SyntaxRules> {
        let (items, tail) =
            as_list(e, spec).ok_or_else(|| "syntax-rules: expected a list".to_string())?;
        if !matches!(tail, Value::Null) {
            return Err("syntax-rules: improper list".into());
        }
        let head = items
            .first()
            .copied()
            .ok_or_else(|| "syntax-rules: empty".to_string())?;
        if !matches!(head, Value::Symbol(s) if e.interner.sym_name(e.resolve(s)) == "syntax-rules")
        {
            return Err("expected (syntax-rules …)".into());
        }
        let mut idx = 1;
        let mut ellipsis = e.interner.symbol("...");
        // R7RS allows `(syntax-rules ellipsis (literals) rules …)`.
        if let Some(Value::Symbol(s)) = items.get(idx).copied() {
            ellipsis = s;
            idx += 1;
        }
        let lits = items
            .get(idx)
            .copied()
            .ok_or_else(|| "syntax-rules: expected a literal list".to_string())?;
        idx += 1;
        let (lit_items, _) =
            as_list(e, lits).ok_or_else(|| "syntax-rules: literals must be a list".to_string())?;
        let mut literals = Vec::new();
        for l in lit_items {
            match l {
                Value::Symbol(s) => literals.push(e.resolve(s)),
                _ => return Err("syntax-rules: literals must be identifiers".into()),
            }
        }
        let mut rules = Vec::new();
        for rule in &items[idx..] {
            let (parts, _) = as_list(e, *rule)
                .ok_or_else(|| "syntax-rules: each rule is (pattern template)".to_string())?;
            match parts.as_slice() {
                [pat, tmpl] => rules.push((*pat, *tmpl)),
                _ => return Err("syntax-rules: each rule is (pattern template)".into()),
            }
        }
        if rules.is_empty() {
            return Err("syntax-rules: no rules".into());
        }
        Ok(SyntaxRules {
            name,
            literals,
            ellipsis,
            rules,
            def_env,
        })
    }

    /// Expand a use of this macro. `form` is the whole call.
    pub fn expand(&self, e: &mut Engine, form: Value) -> SynResult<Value> {
        for (pat, tmpl) in &self.rules {
            let mut b = Bindings::new();
            // The pattern's own head is ignored -- it names the macro, and
            // R7RS says it is not a pattern variable.
            let (pat_items, pat_tail) = match as_list(e, *pat) {
                Some(x) => x,
                None => continue,
            };
            let (form_items, form_tail) = match as_list(e, form) {
                Some(x) => x,
                None => continue,
            };
            if pat_items.is_empty() || form_items.is_empty() {
                continue;
            }
            let pat_rest = (pat_items[1..].to_vec(), pat_tail);
            let form_rest = (form_items[1..].to_vec(), form_tail);
            if self.match_seq(e, &pat_rest.0, pat_rest.1, &form_rest.0, form_rest.1, &mut b) {
                let mut renames: HashMap<SymId, SymId> = HashMap::new();
                return self.instantiate(e, *tmpl, &b, &mut renames);
            }
        }
        Err(format!(
            "no matching syntax rule for {}",
            e.interner.sym_name(self.name)
        ))
    }

    fn is_ellipsis(&self, e: &Engine, v: Value) -> bool {
        matches!(v, Value::Symbol(s) if e.resolve(s) == self.ellipsis)
    }

    fn match_one(&self, e: &Engine, pat: Value, input: Value, b: &mut Bindings) -> bool {
        match pat {
            Value::Symbol(s) => {
                let r = e.resolve(s);
                let name = e.interner.sym_name(r);
                if name == "_" {
                    return true;
                }
                if self.literals.contains(&r) {
                    // A literal matches only the same identifier.
                    return matches!(input, Value::Symbol(i) if e.resolve(i) == r);
                }
                b.insert(r, Binding::One(input));
                true
            }
            Value::Null => matches!(input, Value::Null),
            Value::Obj(_) => match as_list(e, pat) {
                Some((pitems, ptail)) => match as_list(e, input) {
                    Some((iitems, itail)) => self.match_seq(e, &pitems, ptail, &iitems, itail, b),
                    None => false,
                },
                // A `#(…)` pattern, or a string/bytevector constant.
                None => e.equal(pat, input),
            },
            _ => e.equal(pat, input),
        }
    }

    fn match_seq(
        &self,
        e: &Engine,
        pats: &[Value],
        ptail: Value,
        inputs: &[Value],
        itail: Value,
        b: &mut Bindings,
    ) -> bool {
        // Where is the ellipsis, if any?
        let ell = pats.iter().position(|&p| self.is_ellipsis(e, p));
        match ell {
            None => {
                if pats.len() != inputs.len() {
                    return false;
                }
                for (p, i) in pats.iter().zip(inputs.iter()) {
                    if !self.match_one(e, *p, *i, b) {
                        return false;
                    }
                }
                // An improper pattern tail matches the rest.
                match ptail {
                    Value::Null => matches!(itail, Value::Null),
                    t => self.match_one(e, t, itail, b),
                }
            }
            Some(k) if k >= 1 => {
                let repeat = pats[k - 1];
                let head = &pats[..k - 1];
                let tail = &pats[k + 1..];
                if inputs.len() < head.len() + tail.len() {
                    return false;
                }
                for (p, i) in head.iter().zip(inputs.iter()) {
                    if !self.match_one(e, *p, *i, b) {
                        return false;
                    }
                }
                let mid_end = inputs.len() - tail.len();
                let mut collected: Vec<Bindings> = Vec::new();
                for i in &inputs[head.len()..mid_end] {
                    let mut sub = Bindings::new();
                    if !self.match_one(e, repeat, *i, &mut sub) {
                        return false;
                    }
                    collected.push(sub);
                }
                // Every variable under the ellipsis becomes a `Many`, even
                // when it matched zero times -- otherwise a template with
                // `(x ...)` over an empty match would report an unbound
                // pattern variable instead of splicing nothing.
                let mut vars = Vec::new();
                self.pattern_vars(e, repeat, &mut vars);
                for v in vars {
                    let items = collected
                        .iter()
                        .map(|sub| sub.get(&v).cloned().unwrap_or(Binding::One(Value::Null)))
                        .collect();
                    b.insert(v, Binding::Many(items));
                }
                for (p, i) in tail.iter().zip(inputs[mid_end..].iter()) {
                    if !self.match_one(e, *p, *i, b) {
                        return false;
                    }
                }
                match ptail {
                    Value::Null => matches!(itail, Value::Null),
                    t => self.match_one(e, t, itail, b),
                }
            }
            // `(... x)` as a pattern is not meaningful.
            Some(_) => false,
        }
    }

    fn pattern_vars(&self, e: &Engine, pat: Value, out: &mut Vec<SymId>) {
        match pat {
            Value::Symbol(s) => {
                let r = e.resolve(s);
                let name = e.interner.sym_name(r);
                if name != "_" && r != self.ellipsis && !self.literals.contains(&r) {
                    out.push(r);
                }
            }
            Value::Obj(_) => {
                if let Some((items, tail)) = as_list(e, pat) {
                    for i in items {
                        self.pattern_vars(e, i, out);
                    }
                    if !matches!(tail, Value::Null) {
                        self.pattern_vars(e, tail, out);
                    }
                }
            }
            _ => {}
        }
    }

    /// Build the expansion: substitute pattern variables, rename everything
    /// else the template introduced.
    fn instantiate(
        &self,
        e: &mut Engine,
        tmpl: Value,
        b: &Bindings,
        renames: &mut HashMap<SymId, SymId>,
    ) -> SynResult<Value> {
        match tmpl {
            Value::Symbol(s) => {
                let r = e.resolve(s);
                if let Some(binding) = b.get(&r) {
                    return match binding {
                        Binding::One(v) => Ok(*v),
                        Binding::Many(_) => Err(format!(
                            "{}: pattern variable {} needs ellipsis",
                            e.interner.sym_name(self.name),
                            e.interner.sym_name(r)
                        )),
                    };
                }
                Ok(Value::Symbol(self.rename(e, r, renames)))
            }
            Value::Obj(_) => {
                let Some((items, tail)) = as_list(e, tmpl) else {
                    // `#(…)` template: expand the elements, keep the kind.
                    if let Value::Obj(r) = tmpl
                        && let Obj::Vector { items, bracket } = e.obj(r).clone()
                    {
                        let mut out = Vec::with_capacity(items.len());
                        for i in items {
                            out.push(self.instantiate(e, i, b, renames)?);
                        }
                        return Ok(e.vector(out, bracket));
                    }
                    return Ok(tmpl);
                };
                // `(... template)` escapes a literal ellipsis.
                if items.len() == 2 && self.is_ellipsis(e, items[0]) {
                    return Ok(items[1]);
                }
                let bracket = matches!(tmpl, Value::Obj(r)
                    if matches!(e.obj(r), Obj::Vector { bracket: true, .. }));
                let mut out: Vec<Value> = Vec::new();
                let mut i = 0;
                while i < items.len() {
                    let sub = items[i];
                    let reps = {
                        let mut n = 0;
                        while i + 1 + n < items.len() && self.is_ellipsis(e, items[i + 1 + n]) {
                            n += 1;
                        }
                        n
                    };
                    if reps == 0 {
                        out.push(self.instantiate(e, sub, b, renames)?);
                        i += 1;
                        continue;
                    }
                    // One level of splicing per `...`, so `(a ... ...)`
                    // flattens two levels of `Many`.
                    out.extend(self.instantiate_repeated(e, sub, b, renames, reps)?);
                    i += 1 + reps;
                }
                let tail = if matches!(tail, Value::Null) {
                    Value::Null
                } else {
                    self.instantiate(e, tail, b, renames)?
                };
                if bracket && matches!(tail, Value::Null) {
                    return Ok(e.vector(out, true));
                }
                let mut acc = tail;
                for v in out.into_iter().rev() {
                    acc = e.cons(v, acc);
                }
                Ok(acc)
            }
            _ => Ok(tmpl),
        }
    }

    /// Expand `sub` once per repetition of the pattern variables it mentions.
    fn instantiate_repeated(
        &self,
        e: &mut Engine,
        sub: Value,
        b: &Bindings,
        renames: &mut HashMap<SymId, SymId>,
        depth: usize,
    ) -> SynResult<Vec<Value>> {
        let mut vars = Vec::new();
        self.pattern_vars(e, sub, &mut vars);
        let n = vars
            .iter()
            .filter_map(|v| match b.get(v) {
                Some(Binding::Many(items)) => Some(items.len()),
                _ => None,
            })
            .max();
        let Some(n) = n else {
            return Err(format!(
                "{}: `...` with no pattern variable to repeat",
                e.interner.sym_name(self.name)
            ));
        };
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let mut slice = b.clone();
            for v in &vars {
                if let Some(Binding::Many(items)) = b.get(v)
                    && let Some(item) = items.get(k)
                {
                    slice.insert(*v, item.clone());
                }
            }
            if depth > 1 {
                out.extend(self.instantiate_repeated(e, sub, &slice, renames, depth - 1)?);
            } else {
                out.push(self.instantiate(e, sub, &slice, renames)?);
            }
        }
        Ok(out)
    }

    /// One fresh name per template identifier per expansion, so the template's
    /// own references find each other and nothing else can.
    fn rename(
        &self,
        e: &mut Engine,
        sym: SymId,
        renames: &mut HashMap<SymId, SymId>,
    ) -> SymId {
        if let Some(&r) = renames.get(&sym) {
            return r;
        }
        let base = e.interner.sym_name(sym).to_string();
        let fresh = e.interner.fresh(&base);
        e.set_alias(fresh, sym);
        renames.insert(sym, fresh);
        fresh
    }

    pub fn def_env(&self) -> Option<ObjRef> {
        self.def_env
    }

    /// Patterns and templates are heap data, and the macro table is the only
    /// thing holding them.
    pub fn trace(&self, out: &mut Vec<Value>) {
        for (pat, tmpl) in &self.rules {
            out.push(*pat);
            out.push(*tmpl);
        }
        if let Some(r) = self.def_env {
            out.push(Value::Obj(r));
        }
    }
}

pub type MacroTable = HashMap<SymId, Rc<SyntaxRules>>;
