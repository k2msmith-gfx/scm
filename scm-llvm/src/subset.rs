//! The purity and reachability analysis (plan §6 item 1).
//!
//! The deliverable is not "reject impure code" but **the call that made a
//! procedure uncompilable**, with the chain that reached it. The analysis
//! is fail-closed: a primitive is impure until it is listed here, because
//! the failure mode of an optimistic guess is a data race in a render
//! thread. The list is exactly what `codegen` can emit, so a procedure the
//! analysis accepts is one the compiler can build -- one refusal, one
//! vocabulary.
//!
//! Carried over from the S5b probe (`spikes/s5b-leaf`), which found that
//! the analysis is the easy half and that `map` must be specialised at the
//! call site before purity can even be asked.

use std::collections::{BTreeSet, HashMap};

use scm::heap::Obj;
use scm::{ObjRef, SymId, Value, Vm};

use crate::Refusal;

/// Primitives a leaf may call: every one of them is a register operation,
/// a libm call, or a read of an immutable table.
///
/// Not here, and it is deliberate: `vector-set!` and `set-car!` mutate what
/// may outlive the call; `random` is stateful; `vector-push!` allocates;
/// `display` and the ports are I/O; `cons`, `car` and `cdr` would need a
/// heap. `list` and `vector` appear only as the 3-float return and as the
/// sequence `map` is specialised over, which codegen checks.
pub const PURE_PRIMS: &[&str] = &[
    "+", "-", "*", "/", "=", "<", "<=", ">", ">=", "abs", "min", "max", "square",
    "quotient", "remainder", "modulo", "floor-quotient", "floor-remainder",
    "truncate-quotient", "truncate-remainder", "expt", "exact", "inexact",
    "exact->inexact", "inexact->exact", "floor", "ceiling", "round", "truncate",
    "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "exp", "log",
    "zero?", "positive?", "negative?", "even?", "odd?", "nan?", "not",
    "vector-ref", "vector-length", "list", "vector", "map",
    // Reading a numeric vector or an array: the data ray's fields and
    // textures sample. Writing one is absent on purpose -- a leaf runs on
    // N render threads at once.
    "numvector-ref", "numvector-length", "array-ref",
];

/// Heads with binding or evaluation structure the walk must respect. `let`
/// and named `let` are core forms (plan §6), so the loop and the bindings
/// are in the form where the author wrote them.
const SPECIAL: &[&str] = &[
    "quote", "if", "lambda", "begin", "set!", "define", "quasiquote", "and", "or", "let",
];

/// What the analysis learned about an accepted procedure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub arity: usize,
    /// Authored procedures it reaches, transitively.
    pub calls: BTreeSet<String>,
    /// Globals it reads as data: the tables item (2) snapshots.
    pub captured: BTreeSet<String>,
}

#[derive(Default)]
struct Acc {
    calls: BTreeSet<String>,
    captured: BTreeSet<String>,
}

pub struct Analysis<'a> {
    vm: &'a mut Vm,
    seen: HashMap<SymId, Result<Summary, Refusal>>,
    /// The call chain, so a refusal can say how it was reached.
    chain: Vec<String>,
    /// The captured environment of the procedure being walked (S11b).
    ///
    /// A free variable is resolved HERE before the globals, which is both
    /// the lexical order and safe: a top-level closure has no environment
    /// at all — the global table is a separate structure — so only a
    /// procedure created inside another procedure has one, and that is
    /// exactly ray's texel idiom. Verified rather than assumed: `tex` from
    /// `(define tex (wood-texel …))` carries an env and `ring` does not.
    envs: Vec<Option<ObjRef>>,
}

/// Walk an environment chain for `sym`. `None` means "not in any frame",
/// which is where the global lookup takes over.
pub fn env_lookup(vm: &Vm, mut env: Option<ObjRef>, sym: SymId) -> Option<Value> {
    while let Some(r) = env {
        let Obj::Env { parent, slots } = vm.engine.obj(r) else { return None };
        if let Some((_, v)) = slots.iter().rev().find(|(s, _)| *s == sym) {
            return Some(*v);
        }
        env = *parent;
    }
    None
}

/// Can a leaf read this captured value? A number becomes a constant in the
/// emitted code and a sequence of numbers becomes data; anything else is
/// refused, and the refusal prints the value so the author can see what
/// their closure actually caught.
pub fn captured_readable(vm: &Vm, v: Value) -> bool {
    match v {
        Value::Fixnum(_) | Value::Flonum(_) => true,
        Value::Obj(r) => matches!(
            vm.engine.obj(r),
            Obj::Vector { .. } | Obj::NumVector(_) | Obj::Array { .. } | Obj::Pair(..)
        ),
        _ => false,
    }
}

impl<'a> Analysis<'a> {
    pub fn new(vm: &'a mut Vm) -> Self {
        Self { vm, seen: HashMap::new(), chain: Vec::new(), envs: Vec::new() }
    }

    pub fn verdict(&mut self, name: &str) -> Result<Summary, Refusal> {
        let sym = self.vm.engine.interner.symbol(name);
        self.function(sym)
    }

    fn sym_name(&self, s: SymId) -> String {
        self.vm.engine.interner.sym_name(s).to_string()
    }

    fn refuse(&self, why: impl Into<String>) -> Refusal {
        Refusal { message: why.into(), via: self.chain.clone() }
    }

    fn function(&mut self, sym: SymId) -> Result<Summary, Refusal> {
        if let Some(v) = self.seen.get(&sym) {
            return v.clone();
        }
        let name = self.sym_name(sym);
        let result = self.function_inner(sym, &name);
        self.seen.insert(sym, result.clone());
        result
    }

    fn function_inner(&mut self, sym: SymId, name: &str) -> Result<Summary, Refusal> {
        let Some(v) = self.vm.global(sym) else {
            return Err(self.refuse(format!("`{name}` is unbound")));
        };
        self.procedure(Some(sym), v, name)
    }

    /// A procedure given as a VALUE rather than by name — S11b's entry for
    /// `make-texture`, which receives its texel function as an argument and
    /// never as a global. `sym` is `Some` only when the value did come from
    /// a global, which is what the recursion memo is keyed on.
    pub fn verdict_proc(&mut self, v: Value, name: &str) -> Result<Summary, Refusal> {
        self.procedure(None, v, name)
    }

    fn procedure(&mut self, sym: Option<SymId>, v: Value, name: &str) -> Result<Summary, Refusal> {
        let Value::Obj(r) = v else {
            return Err(self.refuse(format!("`{name}` is not a procedure")));
        };
        let Obj::Closure { params, opt, named, keys, rest, body, env, .. } =
            self.vm.engine.obj(r).clone()
        else {
            return Err(self.refuse(format!("`{name}` is not an authored procedure")));
        };
        if !opt.is_empty() || !named.is_empty() || keys.is_some() || rest.is_some() {
            return Err(self.refuse(format!(
                "`{name}` has an optional, named or rest parameter, so it has no fixed native signature"
            )));
        }
        // Recursion: assume compilable while inside it -- the ordinary
        // fixed-point start, and what lets a self-recursive helper pass.
        if let Some(sym) = sym {
            self.seen.insert(
                sym,
                Ok(Summary { arity: params.len(), calls: BTreeSet::new(), captured: BTreeSet::new() }),
            );
        }
        let mut locals: Vec<SymId> = params.to_vec();
        let mut acc = Acc::default();
        self.chain.push(name.to_string());
        // A HELPER'S FREE VARIABLES ARE ITS OWN, not the caller's: each
        // procedure is walked against the environment it was created in, so
        // two closures that both call a variable `k` cannot be confused.
        self.envs.push(env);
        let mut out = Ok(());
        match defines_as_lets(self.vm, &body) {
            Ok(body) => {
                for form in body {
                    if let Err(why) = self.walk(form, &mut locals, &mut acc) {
                        out = Err(self.refuse(why));
                        break;
                    }
                }
            }
            Err(why) => out = Err(self.refuse(why)),
        }
        self.envs.pop();
        self.chain.pop();
        out.map(|()| Summary { arity: params.len(), calls: acc.calls, captured: acc.captured })
    }

    /// Walk one expression. `Err` carries the reason, which is the deliverable.
    fn walk(&mut self, expr: Value, locals: &mut Vec<SymId>, acc: &mut Acc) -> Result<(), String> {
        match expr {
            Value::Fixnum(_) | Value::Flonum(_) | Value::Bool(_) => Ok(()),
            Value::Char(_) | Value::Keyword(_) | Value::Null | Value::Unspecified
            | Value::Undefined | Value::Eof => {
                Err(format!("a literal `{}` has no numeric form", scm::write_str(&self.vm.engine, expr)))
            }
            Value::Primitive(_) => Ok(()),
            Value::Symbol(s) => self.var(s, locals, acc),
            Value::Obj(r) => match self.vm.engine.obj(r).clone() {
                Obj::Str(_) | Obj::Bytevector(_) => Err("a string literal has no numeric form".into()),
                // `[a b c]` is a constructor (plan §4.9): the 3-float return
                // in the corpus's own spelling.
                Obj::Vector { items, bracket: true } => {
                    for it in items {
                        self.walk(it, locals, acc)?;
                    }
                    Ok(())
                }
                // `#(…)` is a constant table.
                Obj::Vector { bracket: false, .. } => Ok(()),
                Obj::Pair(..) => self.combination(expr, locals, acc),
                other => Err(format!("a literal {other:?} has no compiled form")),
            },
        }
    }

    /// A variable: local, authored procedure, pure primitive, or captured
    /// immutable data -- item (2).
    fn var(&mut self, s: SymId, locals: &mut [SymId], acc: &mut Acc) -> Result<(), String> {
        let resolved = self.vm.engine.resolve(s);
        if locals.contains(&s) || locals.contains(&resolved) {
            return Ok(());
        }
        let name = self.sym_name(resolved);
        // S11b: THE CAPTURED ENVIRONMENT COMES FIRST, because that is
        // lexical scope — a closure's `rings` is its own even if a global of
        // that name exists. It cannot change what an existing capture means,
        // because a top-level procedure has no environment at all.
        let env = self.envs.last().copied().flatten();
        let captured = env_lookup(self.vm, env, resolved).or_else(|| env_lookup(self.vm, env, s));
        if let Some(v) = captured
            && captured_readable(self.vm, v)
        {
            acc.captured.insert(name);
            return Ok(());
        }
        // A CAPTURED PROCEDURE FALLS THROUGH TO THE GLOBAL PATH, which is
        // the one that knows how to analyse a call. A library's procedures
        // close over the library's own frame, so `value-noise-2` finds
        // `noise-fade` in its environment as well as in the global table —
        // and taking the environment's answer refused five of `(scm noise)`
        // for holding a procedure (found by the S11 gate, 2026-10-19). Only
        // DATA is read from a frame; everything else is resolved as before.
        match self.vm.global(resolved) {
            None => match captured {
                // Captured, but not something a leaf can read or call: say
                // what it holds, because "unbound" would be a lie.
                Some(v) => Err(format!(
                    "`{name}` is captured from a closure and holds `{}`, and a leaf can only \
                     read numbers and sequences of them",
                    scm::write_str(&self.vm.engine, v)
                )),
                None => Err(format!("`{name}` is unbound")),
            },
            // Judged by *which primitive it is*, not by the name it was
            // reached through: `f64vector-ref` is an alias of
            // `numvector-ref`, and so are the other nine element types, so
            // one entry covers them all. It also means `(define my+ +)` is
            // accepted and `(define out display)` is not, both correctly.
            Some(Value::Primitive(id)) => {
                let prim = self.vm.engine.prim_name(id).to_string();
                if PURE_PRIMS.contains(&prim.as_str()) {
                    Ok(())
                } else if prim == name {
                    Err(format!("`{name}` is not a pure primitive"))
                } else {
                    Err(format!("`{name}` is `{prim}`, which is not a pure primitive"))
                }
            }
            Some(Value::Obj(r)) => match self.vm.engine.obj(r) {
                Obj::Closure { .. } => {
                    acc.calls.insert(name.clone());
                    match self.function(resolved) {
                        Ok(s) => {
                            acc.calls.extend(s.calls);
                            acc.captured.extend(s.captured);
                            Ok(())
                        }
                        Err(r) => Err(format!("{} (reached through `{name}`)", r.message)),
                    }
                }
                // Item (2): a global holding a table is a snapshot, not a
                // call. Whether its contents are numbers is codegen's check.
                // A numeric vector and an array are the same thing at
                // larger scale -- an image, a height field, a volume.
                Obj::Vector { .. } | Obj::NumVector(_) | Obj::Array { .. } => {
                    acc.captured.insert(name);
                    Ok(())
                }
                other => Err(format!("`{name}` holds a {} and a leaf can only read numbers and vectors of them", kind(other))),
            },
            Some(Value::Fixnum(_) | Value::Flonum(_)) => {
                acc.captured.insert(name);
                Ok(())
            }
            Some(other) => Err(format!(
                "`{name}` is `{}`, and a leaf can only read numbers and vectors of them",
                scm::write_str(&self.vm.engine, other)
            )),
        }
    }

    fn combination(&mut self, expr: Value, locals: &mut Vec<SymId>, acc: &mut Acc) -> Result<(), String> {
        // Expansion is lazy in T0, so a closure's body is unexpanded source.
        let expr = expand(self.vm, expr)?;
        let parts = list(self.vm, expr)?;
        let Some(&head) = parts.first() else {
            return Err("the empty combination `()` is not an expression".into());
        };

        // Item (4): `(map f (list a b c))` becomes the applications
        // `(f a) (f b) (f c)`, and `map` never appears. Recognised before the
        // head is classified, or the verdict is a false refusal.
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
            if !shadowed && hname == "map" {
                return Err("`map` over a sequence of unknown length cannot be specialised, and the generic `map` allocates".into());
            }
        } else {
            return Err("a computed operator cannot be compiled".into());
        }
        for p in &parts {
            self.walk(*p, locals, acc)?;
        }
        Ok(())
    }

    /// `(map f SEQ …)` with sequences of known length -> `f` once per
    /// position. One sequence gives `f` followed by the elements, which is
    /// what the analysis walks; TWO give the corpus's colour blend,
    /// `(map (lambda (e l) …) early late)`, and are zipped.
    fn specialise_map(&mut self, parts: &[Value]) -> Result<Option<Vec<Value>>, String> {
        let [head, f, args @ ..] = parts else {
            return Ok(None);
        };
        if args.is_empty() {
            return Ok(None);
        }
        let Value::Symbol(h) = *head else {
            return Ok(None);
        };
        if self.sym_name(self.vm.engine.resolve(h)) != "map" {
            return Ok(None);
        }
        let env = self.envs.last().copied().flatten();
        let mut seqs = Vec::new();
        for a in args {
            match literal_sequence_in(self.vm, *a, env)? {
                Some(items) => seqs.push(items),
                None => return Ok(None),
            }
        }
        let n = seqs[0].len();
        if seqs.iter().any(|s| s.len() != n) {
            return Err(format!(
                "`map` over sequences of different lengths ({})",
                seqs.iter().map(|s| s.len().to_string()).collect::<Vec<_>>().join(", ")
            ));
        }
        // Every element of every sequence is an argument `f` is applied to,
        // and the walk only needs to see each of them once.
        let mut out = Vec::with_capacity(n * seqs.len() + 1);
        out.push(*f);
        for s in &seqs {
            out.extend(s.iter().copied());
        }
        Ok(Some(out))
    }

    fn special(&mut self, head: &str, args: &[Value], locals: &mut Vec<SymId>, acc: &mut Acc) -> Result<(), String> {
        match head {
            "quote" => match args.first() {
                Some(Value::Fixnum(_) | Value::Flonum(_) | Value::Bool(_)) => Ok(()),
                Some(&Value::Obj(r)) if matches!(self.vm.engine.obj(r), Obj::Vector { .. }) => Ok(()),
                _ => Err("quoted structure has no numeric form".into()),
            },
            "quasiquote" => Err("`quasiquote` builds structure at run time".into()),
            "if" | "begin" | "and" | "or" => {
                for a in args {
                    self.walk(*a, locals, acc)?;
                }
                Ok(())
            }
            "lambda" => {
                // Only as the procedure of a specialised `map`, which is the
                // one place a lambda reaches here. Its body must be pure.
                let Some(&formals) = args.first() else {
                    return Err("`lambda` with no formals".into());
                };
                let depth = locals.len();
                for p in list(self.vm, formals).unwrap_or_default() {
                    if let Value::Symbol(s) = p {
                        locals.push(s);
                    }
                }
                for a in defines_as_lets(self.vm, &args[1..])? {
                    self.walk(a, locals, acc)?;
                }
                locals.truncate(depth);
                Ok(())
            }
            // Item (3): local mutation is allowed, mutation that outlives
            // the call is not. The distinction is exactly scope.
            "set!" => {
                let Some(&Value::Symbol(t)) = args.first() else {
                    return Err("`set!` with a non-symbol target".into());
                };
                let resolved = self.vm.engine.resolve(t);
                if !(locals.contains(&t) || locals.contains(&resolved)) {
                    return Err(format!("`set! {}` mutates a global, which outlives the call", self.sym_name(resolved)));
                }
                for a in &args[1..] {
                    self.walk(*a, locals, acc)?;
                }
                Ok(())
            }
            // `(let [name] ((v init) …) body …)`: inits in the enclosing
            // scope, so they are walked before the variables are pushed.
            "let" => {
                let (name, rest) = match args.first() {
                    Some(&Value::Symbol(n)) => (Some(n), &args[1..]),
                    _ => (None, args),
                };
                let Some((spec, body)) = rest.split_first() else {
                    return Err("`let` with no binding list".into());
                };
                let mut vars = Vec::new();
                for pair in binding_items(self.vm, *spec)? {
                    let parts = binding_items(self.vm, pair)?;
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
                for b in defines_as_lets(self.vm, body)? {
                    self.walk(b, locals, acc)?;
                }
                locals.truncate(depth);
                Ok(())
            }
            // Leading definitions in a body were rewritten to `let`s before
            // the walk, so a `define` that reaches here is somewhere else.
            "define" => Err("an internal `define` is a leaf only where it opens a body".into()),
            _ => unreachable!("{head} is in SPECIAL but not handled"),
        }
    }
}

fn kind(o: &Obj) -> &'static str {
    match o {
        Obj::Pair(..) => "list",
        Obj::Str(_) => "string",
        Obj::Closure { .. } => "procedure",
        Obj::Port { .. } => "port",
        Obj::Parameter { .. } => "parameter",
        _ => "non-numeric object",
    }
}

/// The elements of `(list a b c)`, `(vector a b c)`, `[a b c]` or `'(a b c)`,
/// or `None` if the length is not statically known.
/// The elements of a sequence whose LENGTH is known at compile time, so
/// `map` over it can be unrolled: a literal `(list …)`, `(vector …)`,
/// `[…]`, a quoted list — or, S11b, a variable captured from a closure that
/// holds one. The last case is what ray's textures need: `early` and `late`
/// are a procedure's colour arguments, closed over by the texel lambda, and
/// `(map f early late)` is how every one of them blends two colours.
pub fn literal_sequence_in(
    vm: &mut Vm,
    v: Value,
    env: Option<ObjRef>,
) -> Result<Option<Vec<Value>>, String> {
    if let Value::Symbol(s) = v
        && let Some(found) = {
            let r = vm.engine.resolve(s);
            env_lookup(vm, env, r).or_else(|| env_lookup(vm, env, s))
        }
    {
        return Ok(sequence_items(vm, found));
    }
    let Value::Obj(r) = v else {
        return Ok(None);
    };
    if let Obj::Vector { items, bracket: true } = vm.engine.obj(r) {
        return Ok(Some(items.clone()));
    }
    let v = expand(vm, v)?;
    let parts = list(vm, v)?;
    let Some(&Value::Symbol(h)) = parts.first() else {
        return Ok(None);
    };
    let name = vm.engine.interner.sym_name(vm.engine.resolve(h)).to_string();
    match name.as_str() {
        "list" | "vector" => Ok(Some(parts[1..].to_vec())),
        "quote" => match parts.get(1) {
            Some(&q) => Ok(Some(list(vm, q).unwrap_or_default())),
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

/// The elements of a sequence value, as literals. `None` for anything that
/// is not a sequence of numbers — the caller turns that into its own
/// refusal, which can say what it was looking at.
pub fn sequence_items(vm: &Vm, v: Value) -> Option<Vec<Value>> {
    let Value::Obj(r) = v else { return None };
    match vm.engine.obj(r) {
        Obj::Vector { items, .. } => Some(items.clone()),
        // `get` already hands back a `Value`, exact or inexact as the
        // element type is; the leaf's one numeric type sorts that out.
        Obj::NumVector(nv) => (0..nv.len()).map(|i| nv.get(i)).collect(),
        Obj::Pair(..) => {
            // A proper list, walked by hand: `list` in the Vm would allocate.
            let mut out = Vec::new();
            let mut cur = v;
            while let Value::Obj(p) = cur {
                let Obj::Pair(h, t) = vm.engine.obj(p) else { return None };
                out.push(*h);
                cur = *t;
            }
            matches!(cur, Value::Null).then_some(out)
        }
        _ => None,
    }
}

/// A body whose leading forms are internal `define`s, as nested `let`s:
/// `(define a e1) (define b e2) body …` becomes
/// `((let ((a e1)) (let ((b e2)) body …)))`. A body without them comes back
/// unchanged. The analysis and the code generator both read bodies through
/// this, so they agree on what a body means.
///
/// Internal definitions are `letrec*`, and `let` nesting is `let*`. The two
/// differ only where an init reaches its own name or a later one, so that is
/// refused, conservatively by name. ray's texels define numbers in order
/// (`a`, then `sf` from it, then `hinge` from both), which is `let*` exactly.
/// An internal *procedure* stays refused: a leaf holds numbers, not
/// procedures, so it belongs at top level, where the analysis reaches it as
/// a helper.
pub fn defines_as_lets(vm: &mut Vm, body: &[Value]) -> Result<Vec<Value>, String> {
    let mut defs: Vec<(SymId, Value)> = Vec::new();
    let mut i = 0;
    while i < body.len() {
        match as_define(vm, body[i])? {
            Some(d) => defs.push(d),
            None => break,
        }
        i += 1;
    }
    if defs.is_empty() {
        return Ok(body.to_vec());
    }
    let rest = &body[i..];
    for form in rest {
        if let Some((name, _)) = as_define(vm, *form)? {
            let name = vm.engine.interner.sym_name(vm.engine.resolve(name)).to_string();
            return Err(format!("the internal `define` of `{name}` comes after an expression; definitions open a body"));
        }
    }
    if rest.is_empty() {
        return Err("a body of only definitions has no value".into());
    }
    for (k, (name, init)) in defs.iter().enumerate() {
        for (later, _) in &defs[k..] {
            if mentions(vm, *init, *later) {
                let (n, l) = (vm.engine.resolve(*name), vm.engine.resolve(*later));
                let (n, l) = (vm.engine.interner.sym_name(n).to_string(), vm.engine.interner.sym_name(l).to_string());
                return Err(if n == l {
                    format!("the internal `define` of `{n}` refers to itself, which is `letrec`, not `let`")
                } else {
                    format!("the internal `define` of `{n}` refers to `{l}`, defined after it, which is `letrec`, not `let`")
                });
            }
        }
    }
    let let_sym = vm.engine.sym("let");
    let mut inner: Vec<Value> = rest.to_vec();
    for (name, init) in defs.iter().rev() {
        let binding = vm.engine.list(&[Value::Symbol(*name), *init]);
        let bindings = vm.engine.list(&[binding]);
        let mut form = vec![let_sym, bindings];
        form.extend(inner);
        inner = vec![vm.engine.list(&form)];
    }
    Ok(inner)
}

/// `(define name init)` as its parts, `None` for any other form. The other
/// shapes of `define` are refusals that say what to write instead.
fn as_define(vm: &mut Vm, form: Value) -> Result<Option<(SymId, Value)>, String> {
    let Value::Obj(r) = form else { return Ok(None) };
    if !matches!(vm.engine.obj(r), Obj::Pair(..)) {
        return Ok(None);
    }
    let form = expand(vm, form)?;
    let parts = list(vm, form)?;
    let Some(&Value::Symbol(h)) = parts.first() else { return Ok(None) };
    if vm.engine.interner.sym_name(vm.engine.resolve(h)) != "define" {
        return Ok(None);
    }
    match parts.as_slice() {
        [_, Value::Symbol(name), init] => Ok(Some((*name, *init))),
        [_, Value::Symbol(name)] => {
            let name = vm.engine.interner.sym_name(vm.engine.resolve(*name)).to_string();
            Err(format!("the internal `define` of `{name}` has no value"))
        }
        [_, Value::Obj(sig), ..] if matches!(vm.engine.obj(*sig), Obj::Pair(..)) => {
            let name = match vm.engine.obj(*sig) {
                Obj::Pair(Value::Symbol(s), _) => vm.engine.interner.sym_name(vm.engine.resolve(*s)).to_string(),
                _ => "?".into(),
            };
            Err(format!(
                "`{name}` is an internal procedure, and a leaf holds numbers, not procedures; define it at top level"
            ))
        }
        _ => Err("a malformed internal `define`".into()),
    }
}

/// Does `expr` refer to the variable `name`? By resolved name. The binding
/// forms are scoped, because ray writes `(define n (let loop ((n 0) …) …))`
/// and that inner `n` is not the one being defined. Anything else is read
/// as if every symbol in it were a reference: a false yes is a refusal,
/// never a wrong program.
fn mentions(vm: &Vm, expr: Value, name: SymId) -> bool {
    let target = vm.engine.resolve(name);
    refers(vm, expr, target)
}

fn refers(vm: &Vm, expr: Value, target: SymId) -> bool {
    let is = |v: &Value| matches!(v, Value::Symbol(s) if vm.engine.resolve(*s) == target);
    match expr {
        Value::Symbol(_) => is(&expr),
        Value::Obj(r) => match vm.engine.obj(r) {
            Obj::Vector { items, .. } => items.iter().any(|i| refers(vm, *i, target)),
            Obj::Pair(..) => {
                let Ok(parts) = list(vm, expr) else { return any_symbol(vm, expr, target) };
                let head = match parts.first() {
                    Some(&Value::Symbol(h)) => vm.engine.interner.sym_name(vm.engine.resolve(h)).to_string(),
                    _ => String::new(),
                };
                let binds = |v: Value| binding_names(vm, v).iter().any(is);
                match (head.as_str(), parts.as_slice()) {
                    ("quote", _) => false,
                    ("lambda", [_, formals, body @ ..]) => {
                        !binds(*formals) && body.iter().any(|b| refers(vm, *b, target))
                    }
                    ("let", [_, Value::Symbol(loop_name), spec, body @ ..]) => {
                        let items = binding_items(vm, *spec).unwrap_or_default();
                        pairs(vm, &items).iter().any(|(_, init)| refers(vm, *init, target))
                            || (!is(&Value::Symbol(*loop_name))
                                && !pairs(vm, &items).iter().any(|(v, _)| is(v))
                                && body.iter().any(|b| refers(vm, *b, target)))
                    }
                    ("let", [_, spec, body @ ..]) => {
                        let items = binding_items(vm, *spec).unwrap_or_default();
                        let ps = pairs(vm, &items);
                        ps.iter().any(|(_, init)| refers(vm, *init, target))
                            || (!ps.iter().any(|(v, _)| is(v)) && body.iter().any(|b| refers(vm, *b, target)))
                    }
                    ("let*", [_, spec, body @ ..]) => {
                        let items = binding_items(vm, *spec).unwrap_or_default();
                        for (v, init) in pairs(vm, &items) {
                            if refers(vm, init, target) {
                                return true;
                            }
                            if is(&v) {
                                return false;
                            }
                        }
                        body.iter().any(|b| refers(vm, *b, target))
                    }
                    ("letrec" | "letrec*", [_, spec, body @ ..]) => {
                        let items = binding_items(vm, *spec).unwrap_or_default();
                        let ps = pairs(vm, &items);
                        !ps.iter().any(|(v, _)| is(v))
                            && (ps.iter().any(|(_, init)| refers(vm, *init, target))
                                || body.iter().any(|b| refers(vm, *b, target)))
                    }
                    _ => parts.iter().any(|p| refers(vm, *p, target)),
                }
            }
            _ => false,
        },
        _ => false,
    }
}

/// `(variable init)` pairs of a binding list; a malformed one is skipped,
/// which only ever makes `refers` answer yes more often.
fn pairs(vm: &Vm, items: &[Value]) -> Vec<(Value, Value)> {
    items
        .iter()
        .filter_map(|p| match binding_items(vm, *p).ok()?.as_slice() {
            [v, init] => Some((*v, *init)),
            _ => None,
        })
        .collect()
}

/// Every symbol in a lambda list, proper, dotted or a lone rest symbol.
fn binding_names(vm: &Vm, mut formals: Value) -> Vec<Value> {
    let mut out = Vec::new();
    loop {
        match formals {
            Value::Symbol(_) => {
                out.push(formals);
                return out;
            }
            Value::Obj(r) => match vm.engine.obj(r) {
                Obj::Pair(a, d) => {
                    out.push(*a);
                    formals = *d;
                }
                _ => return out,
            },
            _ => return out,
        }
    }
}

/// The blind fallback for structure `list` cannot read.
fn any_symbol(vm: &Vm, expr: Value, target: SymId) -> bool {
    let mut stack = vec![expr];
    while let Some(v) = stack.pop() {
        match v {
            Value::Symbol(s) if vm.engine.resolve(s) == target => return true,
            Value::Obj(r) => match vm.engine.obj(r) {
                Obj::Pair(a, d) => {
                    stack.push(*a);
                    stack.push(*d);
                }
                Obj::Vector { items, .. } => stack.extend(items.iter().copied()),
                _ => {}
            },
            _ => {}
        }
    }
    false
}

/// Drive T0's expander to a fixed point; the cap is requirement 8's.
pub fn expand(vm: &mut Vm, mut expr: Value) -> Result<Value, String> {
    for _ in 0..10_000 {
        match vm.expand_once(expr) {
            Ok(Some(next)) => expr = next,
            Ok(None) => return Ok(expr),
            Err(e) => return Err(format!("expansion failed: {e}")),
        }
    }
    Err("macro expansion did not terminate".into())
}

/// A proper list as a vector. An improper tail is a refusal, not a panic.
pub fn list(vm: &Vm, mut v: Value) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    loop {
        match v {
            Value::Null => return Ok(out),
            Value::Obj(r) => match vm.engine.obj(r) {
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

/// A binding list or a binding: a list, or a bracket vector (plan §4.9).
pub fn binding_items(vm: &Vm, v: Value) -> Result<Vec<Value>, String> {
    if let Value::Obj(r) = v
        && let Obj::Vector { items, bracket: true } = vm.engine.obj(r)
    {
        return Ok(items.clone());
    }
    list(vm, v)
}
