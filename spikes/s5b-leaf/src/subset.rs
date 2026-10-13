//! The purity/reachability analysis of plan §6 item (1), over the IR as it
//! actually exists at S5.
//!
//! This is a *probe*, not the leaf compiler. Its output is a verdict per
//! function and, when a function is rejected, **the call that did it** --
//! because §6's requirement is not "reject impure code" but "report which
//! call made a function uncompilable".
//!
//! The analysis is fail-closed: an unrecognised primitive is impure until
//! someone says otherwise. A purity analysis that guesses optimistically is
//! worse than none, since the failure mode is a data race in a render thread.

use std::collections::{BTreeSet, HashMap};

use scm::heap::Obj;
use scm::{SymId, Value, Vm};

/// Primitives a leaf function may call. Everything else is refused.
///
/// Note what is *not* here and could look safe: `vector-set!` and `set-car!`
/// mutate structure that may outlive the call; `random` is stateful;
/// `vector-push!` allocates and reallocates; `format`, `display` and the
/// ports are I/O. `exact`/`inexact` are pure but allocate nothing, so they
/// stay.
pub const PURE_PRIMS: &[&str] = &[
    // arithmetic and comparison
    "+", "-", "*", "/", "=", "<", "<=", ">", ">=", "abs", "min", "max", "square",
    "quotient", "remainder", "modulo", "gcd", "lcm", "expt", "exact", "inexact",
    "exact->inexact", "inexact->exact", "floor", "ceiling", "round", "truncate",
    "floor-quotient", "floor-remainder", "truncate-quotient", "truncate-remainder",
    // transcendental -- the whole content of a noise function
    "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "exp", "log",
    // predicates
    "zero?", "positive?", "negative?", "even?", "odd?", "number?", "integer?",
    "exact?", "inexact?", "nan?", "finite?", "infinite?", "not", "null?", "pair?",
    "eq?", "eqv?", "equal?", "boolean?", "procedure?", "vector?",
    // reads of immutable structure, which is item (2)'s whole point
    "car", "cdr", "cons", "list", "length", "vector-ref", "vector-length",
    "vector->list", "list->vector", "vector",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Compilable, with the authored functions it reaches and the captured
    /// globals it reads.
    Ok {
        calls: BTreeSet<String>,
        captured: BTreeSet<String>,
    },
    /// Rejected, naming the call and the chain that reached it.
    Blocked { why: String, via: Vec<String> },
}

pub struct Analysis<'a> {
    vm: &'a mut Vm,
    seen: HashMap<SymId, Verdict>,
    /// The call chain, so a rejection can say how it was reached.
    chain: Vec<String>,
}

/// Heads with binding or evaluation structure the walk must respect. `let`
/// and friends are macros and are gone by the time we look, so this is the
/// genuine core.
const SPECIAL: &[&str] = &[
    "quote", "if", "lambda", "begin", "set!", "define", "quasiquote", "and", "or",
    // `let` and named `let` are core forms now, not macros. For this analysis
    // that is a straight simplification: the binding structure is in the form
    // where the author wrote it, instead of having to be recovered from an
    // applied literal lambda or a `letrec` cell.
    "let",
];

impl<'a> Analysis<'a> {
    pub fn new(vm: &'a mut Vm) -> Self {
        Self { vm, seen: HashMap::new(), chain: Vec::new() }
    }

    pub fn verdict(&mut self, name: &str) -> Verdict {
        let sym = self.vm.engine.interner.symbol(name);
        self.function(sym)
    }

    fn sym_name(&self, s: SymId) -> String {
        self.vm.engine.interner.sym_name(s).to_string()
    }

    fn blocked(&self, why: impl Into<String>) -> Verdict {
        Verdict::Blocked { why: why.into(), via: self.chain.clone() }
    }

    fn function(&mut self, sym: SymId) -> Verdict {
        if let Some(v) = self.seen.get(&sym) {
            return v.clone();
        }
        let name = self.sym_name(sym);
        // Recursion: assume compilable while we are inside it, which is the
        // ordinary fixed-point start and is what lets `fbm`'s loop work.
        self.seen.insert(
            sym,
            Verdict::Ok { calls: BTreeSet::new(), captured: BTreeSet::new() },
        );

        let Some(v) = self.vm.global(sym) else {
            let v = self.blocked(format!("`{name}` is unbound"));
            self.seen.insert(sym, v.clone());
            return v;
        };
        let Value::Obj(r) = v else {
            let v = self.blocked(format!("`{name}` is not a procedure"));
            self.seen.insert(sym, v.clone());
            return v;
        };
        let Obj::Closure { params, opt, named, keys, rest, body, .. } =
            self.vm.engine.obj(r).clone()
        else {
            let v = self.blocked(format!("`{name}` is not an authored procedure"));
            self.seen.insert(sym, v.clone());
            return v;
        };
        if !opt.is_empty() || !named.is_empty() || keys.is_some() || rest.is_some() {
            // Not a refusal in principle, but a leaf function's signature has
            // to be fixed to have a native calling convention at all.
            let v = self.blocked(format!(
                "`{name}` has an optional, named or rest parameter, so it has no \
                 fixed native signature"
            ));
            self.seen.insert(sym, v.clone());
            return v;
        }

        let mut locals: Vec<SymId> = params.to_vec();
        let mut acc = Acc::default();
        self.chain.push(name.clone());
        let mut out = Verdict::Ok {
            calls: BTreeSet::new(),
            captured: BTreeSet::new(),
        };
        for form in body.iter() {
            if let Err(why) = self.walk(*form, &mut locals, &mut acc) {
                out = self.blocked(why);
                break;
            }
        }
        self.chain.pop();
        if let Verdict::Ok { .. } = out {
            out = Verdict::Ok { calls: acc.calls, captured: acc.captured };
        }
        self.seen.insert(sym, out.clone());
        out
    }

    /// Walk one expression. `Err` carries the reason, which is the deliverable.
    fn walk(
        &mut self,
        expr: Value,
        locals: &mut Vec<SymId>,
        acc: &mut Acc,
    ) -> Result<(), String> {
        match expr {
            Value::Fixnum(_)
            | Value::Flonum(_)
            | Value::Bool(_)
            | Value::Char(_)
            | Value::Keyword(_)
            | Value::Null
            | Value::Unspecified
            | Value::Eof => Ok(()),
            Value::Primitive(_) => Ok(()),
            Value::Symbol(s) => self.var(s, locals, acc),
            Value::Obj(r) => match self.vm.engine.obj(r).clone() {
                Obj::Str(_) | Obj::Bytevector(_) => Ok(()),
                Obj::Vector { items, .. } => {
                    for it in items {
                        self.walk(it, locals, acc)?;
                    }
                    Ok(())
                }
                Obj::Pair(..) => self.combination(expr, locals, acc),
                other => Err(format!("a literal {other:?} has no compiled form")),
            },
        }
    }

    /// A variable reference: local, authored function, pure primitive, or
    /// captured immutable data -- which is item (2).
    fn var(
        &mut self,
        s: SymId,
        locals: &mut [SymId],
        acc: &mut Acc,
    ) -> Result<(), String> {
        let resolved = self.vm.engine.resolve(s);
        if locals.contains(&s) || locals.contains(&resolved) {
            return Ok(());
        }
        let name = self.sym_name(resolved);
        match self.vm.global(resolved) {
            None => Err(format!("`{name}` is unbound")),
            Some(Value::Primitive(_)) => {
                if PURE_PRIMS.contains(&name.as_str()) {
                    Ok(())
                } else {
                    Err(format!("`{name}` is not a pure primitive"))
                }
            }
            Some(Value::Obj(r)) => match self.vm.engine.obj(r) {
                Obj::Closure { .. } => {
                    acc.calls.insert(name.clone());
                    match self.function(resolved) {
                        Verdict::Ok { calls, captured } => {
                            acc.calls.extend(calls);
                            acc.captured.extend(captured);
                            Ok(())
                        }
                        Verdict::Blocked { why, .. } => {
                            Err(format!("{why} (reached through `{name}`)"))
                        }
                    }
                }
                // Item (2): a global holding immutable data is a captured
                // snapshot, not a call. Whether it IS immutable is the
                // question the spike exists to ask.
                Obj::Vector { .. } | Obj::Pair(..) | Obj::Str(_) | Obj::Bytevector(_) => {
                    acc.captured.insert(name);
                    Ok(())
                }
                other => Err(format!("`{name}` holds a {other:?}")),
            },
            Some(_) => {
                // A global number or boolean: a constant.
                acc.captured.insert(name);
                Ok(())
            }
        }
    }

    fn combination(
        &mut self,
        expr: Value,
        locals: &mut Vec<SymId>,
        acc: &mut Acc,
    ) -> Result<(), String> {
        // Macros are expanded lazily by T0, so the body on the heap is
        // unexpanded source. Drive the expander here; the depth cap is the
        // same requirement-8 reasoning as the interpreter's.
        let mut expr = expr;
        for _ in 0..10_000 {
            match self.vm.expand_once(expr) {
                Ok(Some(next)) => expr = next,
                Ok(None) => break,
                Err(e) => return Err(format!("expansion failed: {e}")),
            }
        }
        let parts = self.list(expr)?;
        let Some(&head) = parts.first() else {
            return Err("the empty combination `()` is not an expression".into());
        };

        // Item (4), and the probe's sharpest finding: every texture in the
        // corpus returns its colour through `(map (lambda (c) ...) (list a b
        // c))`, and the generic `map` is variadic and builds its result with
        // `cons`+`reverse`. There is no compiling THAT. What item (4)
        // actually requires is specialising the CALL SITE -- a literal list
        // of known length and an inline lambda become n applications -- so
        // purity is asked about the lambda's body and the elements, and
        // `map` never appears. Recognise the shape before classifying the
        // head, or the verdict is a false refusal.
        if let Some(rewritten) = self.specialise_map(&parts)? {
            for form in rewritten {
                self.walk(form, locals, acc)?;
            }
            return Ok(());
        }

        if let Value::Symbol(h) = head {
            let resolved = self.vm.engine.resolve(h);
            let hname = self.sym_name(resolved);
            let shadowed = locals.contains(&h) || locals.contains(&resolved);
            if !shadowed && SPECIAL.contains(&hname.as_str()) {
                return self.special(&hname, &parts[1..], locals, acc);
            }
        }
        for p in &parts {
            self.walk(*p, locals, acc)?;
        }
        Ok(())
    }

    /// `(map f (list e1 .. en))` with a literal list -> the n applications
    /// `(f e1) .. (f en)`, which is what makes item (4) tractable. Returns
    /// `None` when the shape does not match, so the caller falls through to
    /// the ordinary rules and `map` is refused on its own merits.
    fn specialise_map(&mut self, parts: &[Value]) -> Result<Option<Vec<Value>>, String> {
        let [head, f, arg] = parts else {
            return Ok(None);
        };
        let Value::Symbol(h) = *head else {
            return Ok(None);
        };
        let resolved = self.vm.engine.resolve(h);
        if self.sym_name(resolved) != "map" {
            return Ok(None);
        }
        // The sequence has to have a statically known length, which means a
        // literal `(list ...)` or a quoted list. Anything else is a real
        // refusal rather than a missed opportunity.
        let items = self.literal_sequence(*arg)?;
        let Some(items) = items else {
            return Err(
                "`map` over a sequence of unknown length cannot be specialised, and \
                 the generic `map` is variadic and allocates"
                    .into(),
            );
        };
        let mut out = Vec::with_capacity(items.len() + 1);
        // The procedure is walked once on its own -- an inline lambda, or a
        // named pure function.
        out.push(*f);
        out.extend(items);
        Ok(Some(out))
    }

    /// The elements of `(list a b c)` or `'(a b c)`, or `None` if the length
    /// is not statically known.
    fn literal_sequence(&mut self, v: Value) -> Result<Option<Vec<Value>>, String> {
        let Value::Obj(_) = v else {
            return Ok(None);
        };
        let parts = self.list(v)?;
        let Some(&Value::Symbol(h)) = parts.first() else {
            return Ok(None);
        };
        let resolved = self.vm.engine.resolve(h);
        match self.sym_name(resolved).as_str() {
            "list" => Ok(Some(parts[1..].to_vec())),
            "quote" => match parts.get(1) {
                Some(&q) => Ok(Some(self.list(q).unwrap_or_default())),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn special(
        &mut self,
        head: &str,
        args: &[Value],
        locals: &mut Vec<SymId>,
        acc: &mut Acc,
    ) -> Result<(), String> {
        match head {
            // Quoted data is a constant; nothing inside it is a reference.
            "quote" => Ok(()),
            "quasiquote" => Err("`quasiquote` builds structure at run time".into()),
            "if" | "begin" | "and" | "or" => {
                for a in args {
                    self.walk(*a, locals, acc)?;
                }
                Ok(())
            }
            "lambda" => {
                // Item (4): an inline lambda is fine as long as its body is.
                // Whether it MONOMORPHISES is codegen's problem, not purity's.
                let Some(&formals) = args.first() else {
                    return Err("`lambda` with no formals".into());
                };
                let depth = locals.len();
                self.bind_formals(formals, locals)?;
                for a in &args[1..] {
                    self.walk(*a, locals, acc)?;
                }
                locals.truncate(depth);
                Ok(())
            }
            // Item (3): local mutation is allowed, mutation that outlives the
            // call is not. The distinction is exactly scope.
            "set!" => {
                let Some(&Value::Symbol(t)) = args.first() else {
                    return Err("`set!` with a non-symbol target".into());
                };
                let resolved = self.vm.engine.resolve(t);
                if !(locals.contains(&t) || locals.contains(&resolved)) {
                    return Err(format!(
                        "`set! {}` mutates a global, which outlives the call",
                        self.sym_name(resolved)
                    ));
                }
                for a in &args[1..] {
                    self.walk(*a, locals, acc)?;
                }
                Ok(())
            }
            // `(let [name] ((v init) ...) body ...)`. The inits are evaluated
            // in the ENCLOSING scope, which is the one thing a walk has to
            // get right here, so they are walked before the variables are
            // pushed.
            "let" => {
                let (name, rest) = match args.first() {
                    Some(&Value::Symbol(n)) => (Some(n), &args[1..]),
                    _ => (None, args),
                };
                let Some((spec, body)) = rest.split_first() else {
                    return Err("`let` with no binding list".into());
                };
                let mut vars = Vec::new();
                for pair in self.binding_items(*spec)? {
                    let parts = self.binding_items(pair)?;
                    match parts.as_slice() {
                        [Value::Symbol(v), init] => {
                            self.walk(*init, locals, acc)?;
                            vars.push(*v);
                        }
                        _ => return Err("a `let` binding that is not (variable init)".into()),
                    }
                }
                let depth = locals.len();
                locals.extend(vars);
                locals.extend(name);
                for b in body {
                    self.walk(*b, locals, acc)?;
                }
                locals.truncate(depth);
                Ok(())
            }
            "define" => {
                // An internal define: bind it, then walk the value.
                match args.first() {
                    Some(&Value::Symbol(t)) => {
                        locals.push(t);
                        for a in &args[1..] {
                            self.walk(*a, locals, acc)?;
                        }
                        Ok(())
                    }
                    Some(&inner) => {
                        // `(define (f x) ...)`
                        let parts = self.list(inner)?;
                        if let Some(&Value::Symbol(t)) = parts.first() {
                            locals.push(t);
                        }
                        let depth = locals.len();
                        for &p in &parts[1..] {
                            if let Value::Symbol(p) = p {
                                locals.push(p);
                            }
                        }
                        for a in &args[1..] {
                            self.walk(*a, locals, acc)?;
                        }
                        locals.truncate(depth);
                        Ok(())
                    }
                    None => Err("`define` with nothing to define".into()),
                }
            }
            _ => unreachable!("{head} is in SPECIAL but not handled"),
        }
    }

    fn bind_formals(
        &mut self,
        formals: Value,
        locals: &mut Vec<SymId>,
    ) -> Result<(), String> {
        match formals {
            Value::Symbol(s) => {
                locals.push(s);
                Ok(())
            }
            Value::Null => Ok(()),
            Value::Obj(_) => {
                let parts = self.list(formals)?;
                for p in parts {
                    if let Value::Symbol(s) = p {
                        locals.push(s);
                    }
                }
                Ok(())
            }
            _ => Err("a formal that is not a symbol".into()),
        }
    }

    /// A binding list or a binding: a proper list, or a bracket vector, which
    /// plan §4.9 makes paren-ish in every binding form.
    fn binding_items(&self, v: Value) -> Result<Vec<Value>, String> {
        if let Value::Obj(r) = v
            && let Obj::Vector {
                items,
                bracket: true,
            } = self.vm.engine.obj(r)
        {
            return Ok(items.clone());
        }
        self.list(v)
    }

    /// A proper list as a vector. An improper tail is a refusal, not a panic.
    fn list(&self, mut v: Value) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        loop {
            match v {
                Value::Null => return Ok(out),
                Value::Obj(r) => match self.vm.engine.obj(r) {
                    Obj::Pair(car, cdr) => {
                        out.push(*car);
                        v = *cdr;
                    }
                    _ => return Err("an improper list in an expression".into()),
                },
                _ => return Err("an improper list in an expression".into()),
            }
        }
    }
}

#[derive(Default)]
pub struct Acc {
    pub calls: BTreeSet<String>,
    pub captured: BTreeSet<String>,
}
