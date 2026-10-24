//! Random programs, and what the tiers say about them.

use std::fmt::Write;

use scm::{Tier, Vm, VmError};

/// xorshift64*, seeded: the same seed gives the same programs on every
/// machine, so a failure is a number to pass around.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}

/// The type of an expression. Programs are generated well-typed so that
/// most of them run to a value; the deliberately ill-typed ones live under
/// `guard`, so an error's *message* is what gets compared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Int,
    Flo,
    Bool,
    Str,
    List,
    Vec,
}

/// A library the program defined and then imported, and the names the
/// import actually brought in -- which is not the same list, because the
/// modifiers drop and rename things.
struct Lib {
    name: String,
    /// Exported procedures of one argument. Every one is **total**: the
    /// bodies use no division and the recursive ones guard their argument,
    /// so a call site can pass any integer without the program hanging or
    /// raising instead of testing the library system.
    unary: Vec<String>,
    /// Exported macros, each taking one expression.
    macros: Vec<String>,
    /// An exported variable and the setter that assigns it, when the
    /// library has them.
    cell: Option<(String, String)>,
}

pub struct Gen {
    rng: Rng,
    /// Variables in scope, innermost last.
    env: Vec<(String, Ty)>,
    /// Top-level procedure definitions emitted so far, as (name, arity).
    procs: Vec<(String, usize)>,
    macros: Vec<String>,
    counters: Vec<String>,
    /// `(variable, setter)` pairs imported from a library, for the
    /// copy-at-import check.
    cells: Vec<(String, String)>,
    /// Imported procedures, kept apart from `procs` as well as in it. The
    /// generic `call` arm picks one name out of all of them, which left
    /// library procedures called about twice in forty programs -- too thin
    /// to be testing anything. They get their own arm.
    lib_procs: Vec<String>,
    counter: u32,
    out: String,
}

impl Gen {
    pub fn new(seed: u64) -> Self {
        Gen { rng: Rng::new(seed), env: Vec::new(), procs: Vec::new(), macros: Vec::new(), counters: Vec::new(), cells: Vec::new(), lib_procs: Vec::new(), counter: 0, out: String::new() }
    }

    fn fresh(&mut self, base: &str) -> String {
        self.counter += 1;
        format!("{base}{}", self.counter)
    }

    /// A whole program: a parameter, some procedures, a record type, and a
    /// final `(list …)` of expressions of every type, whose printed form
    /// is the result the tiers must agree on.
    pub fn program(&mut self) -> String {
        self.out.clear();
        self.env.clear();
        self.procs.clear();
        self.macros.clear();
        self.counters.clear();
        self.cells.clear();
        self.lib_procs.clear();
        self.out.push_str("(define p (make-parameter 1))\n");
        self.out.push_str("(define-record-type pt (make-pt x y) pt? (x pt-x) (y pt-y set-pt-y!))\n");
        // Libraries first, so the procedures below can call what they
        // export. Half the programs have none, so the plain shapes keep
        // their share of the generator.
        if self.rng.chance(1, 2) {
            let mut names: Vec<String> = Vec::new();
            let count = 1 + self.rng.below(2);
            for _ in 0..count {
                let lib = self.library(&names);
                self.import_lib(&lib);
                names.push(lib.name.clone());
            }
        }
        let n = 1 + self.rng.below(3) as usize;
        for _ in 0..n {
            self.procedure();
        }
        if self.rng.chance(1, 2) {
            self.recursive_procedure();
        }
        if self.rng.chance(1, 3) {
            self.mutual_pair();
        }
        if self.rng.chance(1, 3) {
            // A top-level macro, used below: T1 expands it once, T0 lazily.
            let name = self.fresh("twice");
            let _ = writeln!(self.out, "(define-syntax {name} (syntax-rules () ((_ e) (+ e e))))");
            self.macros.push(name);
        }
        if self.rng.chance(1, 2) {
            // A counter closure: captured state mutated across calls.
            let name = self.fresh("tick");
            let _ = writeln!(self.out, "(define {name} (let ((c 0)) (lambda () (set! c (+ c 1)) c)))");
            self.counters.push(name);
        }
        let k = 2 + self.rng.below(4) as usize;
        let mut items = Vec::new();
        for _ in 0..k {
            let ty = *self.rng.pick(&[Ty::Int, Ty::Int, Ty::Bool, Ty::Str, Ty::List, Ty::Vec, Ty::Flo]);
            items.push(self.expr(ty, 4));
        }
        let _ = writeln!(self.out, "(define result (list {}))", items.join(" "));
        self.out.push_str("result\n");
        std::mem::take(&mut self.out)
    }

    /// An integer expression confined to a library body.
    ///
    /// **Not `expr`**: that one can reach for the program's top-level `p`
    /// parameter and `pt` record type, and neither is in scope inside a
    /// library -- the result would be a library that fails to load rather
    /// than a test of the library system. Calls go only to procedures
    /// defined *earlier* in the same body, so the call graph is acyclic
    /// and the program terminates.
    fn lib_expr(&mut self, own: &[String], depth: u32) -> String {
        if depth == 0 || self.rng.chance(1, 3) {
            return if self.rng.chance(1, 2) {
                "x".to_string()
            } else {
                format!("{}", self.rng.below(20) as i64 - 9)
            };
        }
        let d = depth - 1;
        match self.rng.below(6) {
            0 => {
                let (a, b) = (self.lib_expr(own, d), self.lib_expr(own, d));
                format!("(+ {a} {b})")
            }
            1 => {
                let (a, b) = (self.lib_expr(own, d), self.lib_expr(own, d));
                format!("(- {a} {b})")
            }
            // Guarded, so a library body cannot be the thing that makes a
            // program interesting for the wrong reason.
            2 => {
                let (a, b) = (self.lib_expr(own, d), self.lib_expr(own, d));
                format!("(modulo (* {a} {b}) 10007)")
            }
            3 => {
                let (c, a, b) = (
                    self.lib_expr(own, d),
                    self.lib_expr(own, d),
                    self.lib_expr(own, d),
                );
                format!("(if (< {c} 0) {a} {b})")
            }
            4 => {
                let (i, b) = (self.lib_expr(own, d), self.lib_expr(own, d));
                format!("(let ((y {i})) (+ y {b}))")
            }
            _ => match own.last().cloned() {
                Some(f) => {
                    let a = self.lib_expr(own, d);
                    format!("({f} (modulo (abs {a}) 7))")
                }
                None => self.lib_expr(own, d),
            },
        }
    }

    /// Emit a `define-library`, and describe what it exports.
    ///
    /// The library system is 20 embedded libraries deep and until now the
    /// fuzzer generated none of it: no `define-library`, no `import`, no
    /// modifier. What is worth exercising is the parts with tier-specific
    /// paths -- a frame whose definitions are mutually recursive, a macro
    /// bound as a value in that frame, and an export resolved against the
    /// globals because the library does not define it.
    fn library(&mut self, earlier: &[String]) -> Lib {
        self.counter += 1;
        let id = self.counter;
        let name = format!("(fz{id} util)");

        // Nothing from the program's top level is in scope inside a
        // library, so generate the body against an empty world and put
        // the program's back afterwards.
        let saved_env = std::mem::take(&mut self.env);
        let saved_procs = std::mem::take(&mut self.procs);
        let saved_macros = std::mem::take(&mut self.macros);
        let saved_counters = std::mem::take(&mut self.counters);

        let mut defined: Vec<String> = Vec::new();
        let mut exports: Vec<String> = Vec::new();
        let mut unary: Vec<String> = Vec::new();
        let mut macros: Vec<String> = Vec::new();
        let mut cell: Option<(String, String)> = None;
        let mut body = String::new();

        // One procedure always, so the importer has something to call.
        let f = format!("lf{id}");
        let e = self.lib_expr(&defined, 2);
        let _ = writeln!(body, "    (define ({f} x) {e})");
        defined.push(f.clone());
        exports.push(f.clone());
        unary.push(f.clone());

        // A second one, which may call the first.
        if self.rng.chance(1, 2) {
            let g = format!("lg{id}");
            let e = self.lib_expr(&defined, 2);
            let _ = writeln!(body, "    (define ({g} x) {e})");
            defined.push(g.clone());
            exports.push(g.clone());
            unary.push(g.clone());
        }

        // A mutually recursive pair, defined in the order that needs the
        // frame's bindings to be visible to each other rather than in
        // sequence. Only the guarded wrapper is exported, so no call site
        // can hand it a negative and never come back.
        if self.rng.chance(1, 3) {
            let ev = format!("lev{id}");
            let od = format!("lod{id}");
            let w = format!("lmp{id}");
            let _ = writeln!(body, "    (define ({ev} n) (if (= n 0) 1 ({od} (- n 1))))");
            let _ = writeln!(body, "    (define ({od} n) (if (= n 0) 0 ({ev} (- n 1))))");
            let _ = writeln!(body, "    (define ({w} x) ({ev} (modulo (abs x) 10)))");
            exports.push(w.clone());
            unary.push(w);
        }

        // A macro, which a library binds as a value in its frame the way
        // `let-syntax` does.
        if self.rng.chance(1, 2) {
            let m = format!("lm{id}");
            let _ = writeln!(
                body,
                "    (define-syntax {m} (syntax-rules () ((_ e) (+ e e))))"
            );
            exports.push(m.clone());
            macros.push(m);
        }

        // An exported variable and its setter. **This is the documented
        // simplification**: a binding is copied at import, so assigning it
        // inside the library does not reach the importer. Both tiers have
        // to implement the same simplification, which is exactly what a
        // differential fuzzer can say and a hand-written test cannot say
        // for every path.
        if self.rng.chance(1, 3) {
            let v = format!("lv{id}");
            let set = format!("lset{id}");
            let _ = writeln!(body, "    (define {v} 10)");
            let _ = writeln!(body, "    (define ({set} x) (set! {v} x) {v})");
            exports.push(v.clone());
            exports.push(set.clone());
            unary.push(set.clone());
            cell = Some((v, set));
        }

        // An export the library does **not** define, resolved against the
        // globals -- the fallback `(srfi 133)` and `(srfi 130)` both lean
        // on, where a library re-exports a prelude procedure it never
        // mentions.
        let mut export_text = exports.join(" ");
        if self.rng.chance(1, 3) {
            let base = *self.rng.pick(&["length", "reverse", "abs"]);
            let alias = format!("lre{id}");
            export_text.push_str(&format!(" (rename {base} {alias})"));
            if base == "abs" {
                unary.push(alias);
            }
        }

        // Depend on an earlier library, so the chain is more than one deep.
        let deps = match earlier.last() {
            Some(d) if self.rng.chance(1, 3) => format!(" {d}"),
            _ => String::new(),
        };

        let _ = writeln!(self.out, "(define-library {name}");
        let _ = writeln!(self.out, "  (export {export_text})");
        let _ = writeln!(self.out, "  (import (scheme base){deps})");
        let _ = writeln!(self.out, "  (begin");
        self.out.push_str(&body);
        self.out.push_str("    ))\n");

        self.env = saved_env;
        self.procs = saved_procs;
        self.macros = saved_macros;
        self.counters = saved_counters;

        Lib { name, unary, macros, cell }
    }

    /// Import it, under one of the five shapes, and register exactly the
    /// names that shape brings in -- which is the part a modifier can get
    /// wrong.
    fn import_lib(&mut self, lib: &Lib) {
        let pick = if lib.unary.len() > 1 { self.rng.below(5) } else { self.rng.below(3) };
        match pick {
            // Everything, under its own name.
            0 | 1 => {
                let _ = writeln!(self.out, "(import {})", lib.name);
                for n in &lib.unary {
                    self.procs.push((n.clone(), 1));
                    self.lib_procs.push(n.clone());
                }
                for m in &lib.macros {
                    self.macros.push(m.clone());
                }
                if let Some(c) = &lib.cell {
                    self.cells.push(c.clone());
                }
            }
            // A prefix on every name.
            2 => {
                let p = format!("px{}", self.counter);
                let _ = writeln!(self.out, "(import (prefix {} {p}))", lib.name);
                for n in &lib.unary {
                    self.procs.push((format!("{p}{n}"), 1));
                    self.lib_procs.push(format!("{p}{n}"));
                }
                for m in &lib.macros {
                    self.macros.push(format!("{p}{m}"));
                }
                if let Some((v, set)) = &lib.cell {
                    self.cells.push((format!("{p}{v}"), format!("{p}{set}")));
                }
            }
            // Just one name.
            3 => {
                let n = self.rng.pick(&lib.unary).clone();
                let _ = writeln!(self.out, "(import (only {} {n}))", lib.name);
                self.procs.push((n.clone(), 1));
                self.lib_procs.push(n);
            }
            // All but one, renamed.
            _ => {
                let dropped = lib.unary[0].clone();
                let kept = lib.unary[1].clone();
                let alias = format!("rn{}", self.counter);
                let _ = writeln!(
                    self.out,
                    "(import (rename (except {} {dropped}) ({kept} {alias})))",
                    lib.name
                );
                self.procs.push((alias.clone(), 1));
                self.lib_procs.push(alias);
                for n in lib.unary.iter().skip(2) {
                    self.procs.push((n.clone(), 1));
                    self.lib_procs.push(n.clone());
                }
                for m in &lib.macros {
                    self.macros.push(m.clone());
                }
            }
        }
    }

    fn procedure(&mut self) {
        let name = self.fresh("f");
        let arity = 1 + self.rng.below(3) as usize;
        let params: Vec<String> = (0..arity).map(|i| format!("a{i}")).collect();
        let saved = self.env.clone();
        for p in &params {
            self.env.push((p.clone(), Ty::Int));
        }
        let body = match self.rng.below(4) {
            0 => {
                // Internal defines, which T1 pre-scans into slots.
                let h = self.fresh("h");
                self.env.push((h.clone(), Ty::Int));
                let init = self.expr(Ty::Int, 2);
                let b = self.expr(Ty::Int, 2);
                format!("(define {h} {init}) (define ({h}2 q) (+ q {h})) ({h}2 {b})")
            }
            1 => {
                // A local macro: T1 hands the form to T0.
                let b = self.expr(Ty::Int, 2);
                format!("(let-syntax ((dbl (syntax-rules () ((_ e) (* 2 e))))) (dbl {b}))")
            }
            _ => self.expr(Ty::Int, 3),
        };
        self.env = saved;
        let _ = writeln!(self.out, "(define ({name} {}) {body})", params.join(" "));
        self.procs.push((name, arity));
    }

    /// Two procedures calling each other, bounded.
    fn mutual_pair(&mut self) {
        let (a, b) = (self.fresh("ev"), self.fresh("od"));
        let _ = writeln!(self.out, "(define ({a} n) (if (<= n 0) 1 ({b} (- n 1))))");
        let _ = writeln!(self.out, "(define ({b} n) (if (<= n 0) 0 ({a} (- n 1))))");
        self.procs.push((a, 1));
    }

    /// Bounded self-recursion, the shape every loop in the corpus has. The
    /// step may not call an earlier procedure: that one may call back here
    /// with a fresh argument, which is unbounded recursion that only fuel
    /// stops, slowly.
    fn recursive_procedure(&mut self) {
        let name = self.fresh("g");
        let saved = self.env.clone();
        let procs = std::mem::take(&mut self.procs);
        self.env.push(("n".into(), Ty::Int));
        let step = self.expr(Ty::Int, 2);
        self.env = saved;
        self.procs = procs;
        let _ = writeln!(self.out, "(define ({name} n) (if (<= n 0) 0 (+ (modulo {step} 97) ({name} (- n 1)))))");
        self.procs.push((name, 1));
    }

    fn var(&mut self, ty: Ty) -> Option<String> {
        let candidates: Vec<&String> = self.env.iter().filter(|(_, t)| *t == ty).map(|(n, _)| n).collect();
        if candidates.is_empty() {
            return None;
        }
        Some((*self.rng.pick(&candidates)).clone())
    }

    /// `(let ((v init)) body)` with `v` of type `vt`, the body of `ty`.
    fn with_binding(&mut self, vt: Ty, ty: Ty, depth: u32, named: bool) -> String {
        let v = self.fresh("x");
        let init = self.expr(vt, depth - 1);
        self.env.push((v.clone(), vt));
        let body = self.expr(ty, depth - 1);
        self.env.pop();
        if named && vt == Ty::Int && ty == Ty::Int {
            // A named-let loop, bounded by its own counter.
            let i = self.fresh("i");
            format!("(let loop{} (({i} 0) ({v} {init})) (if (>= {i} {}) {v} (loop{} (+ {i} 1) (modulo (+ {body} {i}) 1000))))", self.counter, 1 + self.rng.below(6), self.counter)
        } else {
            let kw = if self.rng.chance(1, 3) { "let*" } else { "let" };
            format!("({kw} (({v} {init})) {body})")
        }
    }

    pub fn expr(&mut self, ty: Ty, depth: u32) -> String {
        if depth == 0 {
            return self.leaf(ty);
        }
        // Reusing a variable is what makes closures and loops interesting.
        if self.rng.chance(1, 4) && let Some(v) = self.var(ty) {
            return v;
        }
        match ty {
            Ty::Int => self.int(depth),
            Ty::Flo => self.flo(depth),
            Ty::Bool => self.bool(depth),
            Ty::Str => self.str(depth),
            Ty::List => self.list(depth),
            Ty::Vec => self.vec(depth),
        }
    }

    fn leaf(&mut self, ty: Ty) -> String {
        if let Some(v) = self.var(ty)
            && self.rng.chance(1, 2)
        {
            return v;
        }
        match ty {
            // Mostly small, but **one in eight is wide enough to need a
            // bignum**. `exact-closed` is a promise (S18), and the tiers
            // have to agree on exact arithmetic past the word as they
            // agree on everything else -- which nothing checked until the
            // generator could produce one.
            Ty::Int => {
                if self.rng.chance(1, 8) {
                    (*self.rng.pick(&[
                        "9223372036854775808",
                        "-9223372036854775809",
                        "18446744073709551616",
                        "99999999999999999999999999",
                        "-170141183460469231731687303715884105728",
                        "(expt 2 100)",
                        "(expt 2 64)",
                        "(* 4294967296 4294967296)",
                    ]))
                        .to_string()
                } else {
                    format!("{}", self.rng.below(41) as i64 - 20)
                }
            }
            Ty::Flo => format!("{}.{}", self.rng.below(20) as i64 - 10, self.rng.below(100)),
            Ty::Bool => if self.rng.chance(1, 2) { "#t".into() } else { "#f".into() },
            Ty::Str => format!("\"{}\"", self.rng.pick(&["", "a", "hello", "x y", "scheme", "q\\\"q"])),
            Ty::List => (*self.rng.pick(&["'()", "'(1 2 3)", "(list 4 5)", "'(0)"])).to_string(),
            Ty::Vec => (*self.rng.pick(&["(vector)", "(vector 1 2 3)", "(make-vector 2 7)"])).to_string(),
        }
    }

    fn int(&mut self, depth: u32) -> String {
        let d = depth - 1;
        // 54, not 50: arms 46, 47 and 48 were added for bignum growth,
        // the copy-at-import check and calls through an import; 50-53 for
        // S14's flat frames; and the `_` arm (49) is a real case
        // (`raise-continuable`) rather than a filler -- leaving the bound
        // alone silently drops whatever sits last.
        match self.rng.below(54) {
            0..=3 => {
                let op = *self.rng.pick(&["+", "-", "*"]);
                let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d));
                if op == "*" { format!("(modulo (* {a} {b}) 10007)") } else { format!("({op} {a} {b})") }
            }
            4 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(quotient {a} (if (= {b} 0) 1 {b}))") }
            5 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(modulo {a} (if (= {b} 0) 1 {b}))") }
            6 => { let op = *self.rng.pick(&["abs", "-"]); let a = self.expr(Ty::Int, d); format!("({op} {a})") }
            7 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); let op = *self.rng.pick(&["min", "max"]); format!("({op} {a} {b})") }
            8 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(if {c} {a} {b})") }
            9 => self.with_binding(Ty::Int, Ty::Int, depth, false),
            10 => self.with_binding(Ty::Int, Ty::Int, depth, true),
            11 => { let l = self.expr(Ty::List, d); format!("(length {l})") }
            12 => { let v = self.expr(Ty::Vec, d); format!("(vector-length {v})") }
            13 => { let (v, i) = (self.expr(Ty::Vec, d), self.expr(Ty::Int, d)); format!("(let ((v {v})) (if (= (vector-length v) 0) 0 (vector-ref v (modulo {i} (vector-length v)))))") }
            14 => { let s = self.expr(Ty::Str, d); format!("(string-length {s})") }
            15 => { let l = self.expr(Ty::List, d); format!("(let ((l {l})) (if (null? l) 0 (car l)))") }
            16 => self.call(d),
            17 => { let x = self.fresh("y"); self.env.push((x.clone(), Ty::Int)); let body = self.expr(Ty::Int, d); self.env.pop(); let arg = self.expr(Ty::Int, d); format!("((lambda ({x}) {body}) {arg})") }
            18 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(call/cc (lambda (k) (if {c} (k {a}) {b})))") }
            19 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(guard (e (#t {a})) (if {c} (raise 'boom) {b}))") }
            20 => { let (s, a) = (self.stmt(d), self.expr(Ty::Int, d)); format!("(begin {s} {a})") }
            21 => { let l = self.expr(Ty::List, d); format!("(apply + {l})") }
            // Growth with no `modulo` guard, so a product really does
            // leave the word. Bounded by construction: squaring a
            // 128-bit value twice is 512 bits, far inside the cap, so the
            // tiers are compared on the arithmetic rather than on which
            // of them reports "too large" first.
            46 => {
                let a = self.expr(Ty::Int, d);
                format!("(let ((n (+ 4294967296 (abs (modulo {a} 1000))))) (* (* n n) (* n n)))")
            }
            22 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(let-values (((u w) (values {a} {b}))) (- u w))") }
            23 => { let a = self.expr(Ty::Int, d); format!("(do ((i 0 (+ i 1)) (s 0 (+ s {a}))) ((= i {}) s))", 1 + self.rng.below(4)) }
            24 => { let (k, a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(case (modulo {k} 3) ((0 1) {a}) (else {b}))") }
            25 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(parameterize ((p {a})) (+ (p) {b}))") }
            26 => { let a = self.expr(Ty::Int, d); format!("(dynamic-wind (lambda () #f) (lambda () {a}) (lambda () #f))") }
            27 => { let x = self.fresh("z"); self.env.push((x.clone(), Ty::Int)); let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); self.env.pop(); let init = self.expr(Ty::Int, d); format!("(let (({x} {init})) (set! {x} {a}) (+ {x} {b}))") }
            28 => { let f = self.expr(Ty::Flo, d); format!("(exact (floor (min 1000.0 (max -1000.0 {f}))))") }
            29 => { let a = self.expr(Ty::Int, d); format!("(string->number (number->string {a}))") }
            30 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(pt-x (make-pt {a} {b}))") }
            31 => { let (c, a) = (self.expr(Ty::Bool, d), self.expr(Ty::Int, d)); let body = self.stmt(d); format!("(let ((r {a})) (when {c} {body}) r)") }
            32 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(guard (e (#t (string-length (error-object-message e)))) (+ {a} (car {b})))") }
            33 => { let a = self.expr(Ty::Int, d); format!("(let ((v (vector 1 2 3))) (vector-set! v 1 {a}) (vector-ref v 1))") }
            34 => match self.macros.last().cloned() { Some(m) => { let a = self.expr(Ty::Int, d); format!("({m} {a})") } None => self.expr(Ty::Int, d) },
            35 => match self.counters.last().cloned() { Some(c) => format!("(+ ({c}) ({c}))"), None => self.expr(Ty::Int, d) },
            // The copy-at-import simplification, read out loud: the setter
            // assigns the library's own variable and returns it, while the
            // importer's copy keeps the value it was imported with. Both
            // tiers must say the same thing about that.
            // A call through the import, which is the whole point of
            // generating libraries: the name resolves through whatever
            // modifier brought it in.
            48 => match self.lib_procs.len() {
                0 => self.expr(Ty::Int, d),
                n => {
                    let f = self.lib_procs[self.rng.below(n as u64) as usize].clone();
                    let a = self.expr(Ty::Int, d);
                    format!("({f} (modulo (abs {a}) 11))")
                }
            },
            47 => match self.cells.last().cloned() {
                Some((v, set)) => {
                    let a = self.expr(Ty::Int, d);
                    format!("(let ((before {v})) (+ (* 100 ({set} {a})) (* 10 before) {v}))")
                }
                None => self.expr(Ty::Int, d),
            },
            36 => {
                // Closures created in a loop, capturing the loop variable,
                // called afterwards: each must see its own binding.
                let a = self.expr(Ty::Int, d);
                format!("(let ((fs (let loop ((i 0) (acc '())) (if (= i 3) acc (loop (+ i 1) (cons (lambda () (+ i {a})) acc)))))) (apply + (map (lambda (f) (f)) fs)))")
            }
            37 => {
                // Other error kinds, compared by message under guard.
                let a = self.expr(Ty::Int, d);
                let bad = *self.rng.pick(&["(vector-ref (vector 1) 5)", "(undefined-variable-xyz)", "((lambda (q) q))", "(5 1)", "(string-ref \"ab\" 9)", "(+ 1 \"a\")", "(error \"custom\" 1 2)", "(raise 42)", "(assert-fail)", "(car '())", "(vector-set! (vector) 0 1)", "(exact 1.5)", "(string->symbol 5)", "(apply + 1)"]);
                format!("(guard (e (#t (if (error-object? e) (string-length (error-object-message e)) {a}))) {bad})")
            }
            38 => { let (k, a) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(let ((al (list (cons 0 {a}) (cons 1 7) (cons 2 9)))) (cdr (assv (modulo (abs {k}) 3) al)))") }
            39 => { let a = self.expr(Ty::Int, d); format!("(let ((r (make-pt {a} 0))) (set-pt-y! r (* 2 (pt-x r))) (pt-y r))") }
            40 => { let a = self.expr(Ty::Int, d); format!("(char->integer (string-ref (number->string (abs {a})) 0))") }
            41 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(letrec ((ev? (lambda (n) (if (= n 0) #t (od? (- n 1))))) (od? (lambda (n) (if (= n 0) #f (ev? (- n 1)))))) (if (ev? (modulo (abs {a}) 7)) {b} (- {b})))") }
            42 => { let (c, a) = (self.expr(Ty::Bool, d), self.expr(Ty::Int, d)); format!("(cond ({c} => (lambda (t) (if t {a} 0))) (else (+ 1 {a})))") }
            43 => { let a = self.expr(Ty::Int, d); format!("(let ((v (make-vector 3 0))) (vector-fill! v {a}) (vector-ref v 2))") }
            44 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(call-with-values (lambda () (values {a} {b})) (lambda (u w) (* 3 (- u w))))") }
            45 => { let a = self.expr(Ty::Int, d); format!("(let ((x {a})) (define (inner) (set! x (+ x 1)) x) (inner) (inner))") }
            // S14: a closure that ESCAPES the let whose variable it captures,
            // called after the let has returned -- the variable must have
            // been put on the heap, not left in a frame slot that is gone.
            50 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(let ((get (let ((x {a})) (lambda (k) (set! x (+ x k)) x)))) (get {b}) (get 1))") }
            // S14: a %try block (guard) reading and writing the enclosing
            // procedure's parameter and let variable: a block is a capture.
            51 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("((lambda (p) (let ((q {b})) (guard (e (#t (+ p q))) (set! q (+ q 1)) (if (> p 0) (raise 'r) (- p q))))) {a})") }
            // S14: same-named nested lets, the inner shadowing the outer and
            // the outer visible again after it.
            52 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(let ((s {a})) (+ (let ((s (* 2 s)) (t s)) (- s t)) (let* ((s (+ s {b})) (s (- s 1))) s) s))") }
            // S14: a flat procedure through every door -- a direct call,
            // `apply`, `map` and a tail self-call.
            53 => { let a = self.expr(Ty::Int, d); format!("(let () (define (f u w) (let ((z (+ u w))) (if (> z 50) z (f (+ u 7) w)))) (+ (f (modulo (abs {a}) 40) 1) (apply f (list 3 4)) (apply + (map (lambda (k) (f k k)) (list 1 2)))))") }
            _ => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(with-exception-handler (lambda (e) {a}) (lambda () (+ {b} (raise-continuable 'c))))") }
        }
    }

    fn call(&mut self, d: u32) -> String {
        if self.procs.is_empty() {
            return self.expr(Ty::Int, d);
        }
        let (name, arity) = self.rng.pick(&self.procs).clone();
        let args: Vec<String> = (0..arity)
            .map(|_| {
                let a = self.expr(Ty::Int, d);
                if name.starts_with('g') { format!("(modulo (abs {a}) 12)") } else { a }
            })
            .collect();
        format!("({name} {})", args.join(" "))
    }

    fn flo(&mut self, depth: u32) -> String {
        let d = depth - 1;
        match self.rng.below(7) {
            0 => { let (a, b) = (self.expr(Ty::Flo, d), self.expr(Ty::Flo, d)); let op = *self.rng.pick(&["+", "-", "*"]); format!("({op} {a} {b})") }
            1 => { let a = self.expr(Ty::Int, d); format!("(exact->inexact {a})") }
            2 => { let a = self.expr(Ty::Flo, d); format!("(sqrt (abs {a}))") }
            3 => { let a = self.expr(Ty::Flo, d); let op = *self.rng.pick(&["floor", "round", "truncate", "sin"]); format!("({op} {a})") }
            4 => self.with_binding(Ty::Flo, Ty::Flo, depth, false),
            5 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Flo, d), self.expr(Ty::Flo, d)); format!("(if {c} {a} {b})") }
            _ => { let (a, b) = (self.expr(Ty::Flo, d), self.expr(Ty::Flo, d)); format!("(/ {a} (if (= {b} 0.0) 1.0 {b}))") }
        }
    }

    fn bool(&mut self, depth: u32) -> String {
        let d = depth - 1;
        match self.rng.below(12) {
            0..=2 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); let op = *self.rng.pick(&["<", "=", ">", "<=", ">="]); format!("({op} {a} {b})") }
            3 => { let a = self.expr(Ty::Int, d); let op = *self.rng.pick(&["even?", "odd?", "zero?", "negative?"]); format!("({op} {a})") }
            4 => { let l = self.expr(Ty::List, d); format!("(null? {l})") }
            5 => { let (a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Bool, d)); let op = *self.rng.pick(&["and", "or"]); format!("({op} {a} {b})") }
            6 => { let a = self.expr(Ty::Bool, d); format!("(not {a})") }
            7 => { let (a, b) = (self.expr(Ty::List, d), self.expr(Ty::List, d)); format!("(equal? {a} {b})") }
            8 => { let (a, b) = (self.expr(Ty::Str, d), self.expr(Ty::Str, d)); format!("(string=? {a} {b})") }
            9 => { let (a, b) = (self.expr(Ty::Flo, d), self.expr(Ty::Flo, d)); format!("(< {a} {b})") }
            10 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Bool, d), self.expr(Ty::Bool, d)); format!("(if {c} {a} {b})") }
            _ => { let a = self.expr(Ty::Int, d); format!("(pt? (make-pt {a} 0))") }
        }
    }

    fn str(&mut self, depth: u32) -> String {
        let d = depth - 1;
        match self.rng.below(8) {
            0 => { let (a, b) = (self.expr(Ty::Str, d), self.expr(Ty::Str, d)); format!("(string-append {a} {b})") }
            1 => { let a = self.expr(Ty::Int, d); format!("(number->string {a})") }
            2 => { let (s, i) = (self.expr(Ty::Str, d), self.expr(Ty::Int, d)); format!("(let ((s {s})) (substring s 0 (modulo {i} (+ 1 (string-length s)))))") }
            3 => { let a = self.expr(Ty::Int, d); format!("(let ((o (open-output-string))) (display {a} o) (write \"-\" o) (get-output-string o))") }
            4 => self.with_binding(Ty::Str, Ty::Str, depth, false),
            5 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::Str, d), self.expr(Ty::Str, d)); format!("(if {c} {a} {b})") }
            6 => { let l = self.expr(Ty::List, d); format!("(apply string-append (map number->string {l}))") }
            _ => { let s = self.expr(Ty::Str, d); format!("(list->string (reverse (string->list {s})))") }
        }
    }

    fn list(&mut self, depth: u32) -> String {
        let d = depth - 1;
        match self.rng.below(11) {
            0 => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(list {a} {b})") }
            1 => { let (a, l) = (self.expr(Ty::Int, d), self.expr(Ty::List, d)); format!("(cons {a} {l})") }
            2 => { let l = self.expr(Ty::List, d); format!("(let ((l {l})) (if (null? l) l (cdr l)))") }
            3 => { let (a, b) = (self.expr(Ty::List, d), self.expr(Ty::List, d)); format!("(append {a} {b})") }
            4 => { let l = self.expr(Ty::List, d); format!("(reverse {l})") }
            5 => { let x = self.fresh("m"); self.env.push((x.clone(), Ty::Int)); let body = self.expr(Ty::Int, d); self.env.pop(); let l = self.expr(Ty::List, d); format!("(map (lambda ({x}) {body}) {l})") }
            6 => { let n = self.expr(Ty::Int, d); format!("(let loop ((i (modulo (abs {n}) 6)) (acc '())) (if (= i 0) acc (loop (- i 1) (cons i acc))))") }
            7 => { let v = self.expr(Ty::Vec, d); format!("(vector->list {v})") }
            8 => self.with_binding(Ty::List, Ty::List, depth, false),
            9 => { let (c, a, b) = (self.expr(Ty::Bool, d), self.expr(Ty::List, d), self.expr(Ty::List, d)); format!("(if {c} {a} {b})") }
            _ => { let (l, i) = (self.expr(Ty::List, d), self.expr(Ty::Int, d)); format!("(let ((l {l})) (list-tail l (modulo {i} (+ 1 (length l)))))") }
        }
    }

    fn vec(&mut self, depth: u32) -> String {
        let d = depth - 1;
        match self.rng.below(7) {
            0 => { let (a, b, c) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(vector {a} {b} {c})") }
            1 => { let (n, a) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(make-vector (modulo (abs {n}) 5) {a})") }
            2 => { let l = self.expr(Ty::List, d); format!("(list->vector {l})") }
            3 => { let (v, i, a) = (self.expr(Ty::Vec, d), self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("(let ((v {v})) (if (> (vector-length v) 0) (vector-set! v (modulo {i} (vector-length v)) {a})) v)") }
            4 => { let x = self.fresh("w"); self.env.push((x.clone(), Ty::Int)); let body = self.expr(Ty::Int, d); self.env.pop(); let v = self.expr(Ty::Vec, d); format!("(vector-map (lambda ({x}) {body}) {v})") }
            5 => self.with_binding(Ty::Vec, Ty::Vec, depth, false),
            _ => { let (a, b) = (self.expr(Ty::Int, d), self.expr(Ty::Int, d)); format!("[{a} {b}]") }
        }
    }

    /// A statement for `begin` and `when`: an effect whose value is dropped.
    fn stmt(&mut self, d: u32) -> String {
        match self.rng.below(4) {
            0 => { let a = self.expr(Ty::Int, d); format!("(display {a})") }
            1 => { let s = self.expr(Ty::Str, d); format!("(display {s})") }
            2 => match self.var(Ty::Int) {
                Some(v) => { let a = self.expr(Ty::Int, d); format!("(set! {v} {a})") }
                None => "(newline)".into(),
            },
            _ => "(newline)".into(),
        }
    }
}

/// What one tier said about a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Value { printed: String, output: String },
    Error { message: String, output: String },
    /// Fuel or a host stop: not comparable across tiers, which spend fuel at
    /// different rates. The program is skipped.
    Stopped,
}

/// Run `src` on a fresh machine of `tier`, with `output` captured.
pub fn run(src: &str, tier: Tier, stress: bool, t2: Option<&dyn Fn(&mut Vm)>) -> Verdict {
    let mut vm = Vm::with_tier(tier);
    if let Some(install) = t2 {
        install(&mut vm);
    }
    vm.engine.set_gc_stress(stress);
    // Enough for any generated program, and a bound on a runaway the
    // generator did not mean to make.
    vm.set_fuel(Some(100_000));
    // Capture: point current-output-port at a string port for the run.
    vm.eval_str("(define %fuzz-out (open-output-string)) (define current-output-port (make-parameter %fuzz-out))")
        .expect("capture setup");
    let result = vm.eval_str(src);
    let output = match vm.eval_str("(get-output-string %fuzz-out)") {
        Ok(v) => scm::display_str(&vm.engine, v),
        Err(_) => String::new(),
    };
    match result {
        Ok(v) => Verdict::Value { printed: scm::write_str(&vm.engine, v), output },
        Err(VmError::OutOfFuel | VmError::Interrupted | VmError::OutOfMemory) => Verdict::Stopped,
        Err(e) => Verdict::Error { message: format!("{e}"), output },
    }
}

/// A disagreement between two tiers on one program.
#[derive(Debug)]
pub struct Mismatch {
    pub seed: u64,
    pub index: u64,
    pub program: String,
    pub tiers: Vec<(String, Verdict)>,
}

/// Generate `count` programs from `seed` and compare every configured
/// tier's verdict. Returns the mismatches and how many programs were
/// skipped as stopped.
pub fn compare(seed: u64, count: u64, stress: bool, t2: Option<&dyn Fn(&mut Vm)>) -> (Vec<Mismatch>, u64) {
    let mut mismatches = Vec::new();
    let mut skipped = 0;
    for index in 0..count {
        let mut g = Gen::new(seed.wrapping_mul(1_000_003).wrapping_add(index));
        let program = g.program();
        let mut verdicts = vec![("T0".to_string(), run(&program, Tier::T0, false, None)), ("T1".to_string(), run(&program, Tier::T1, stress, None))];
        if let Some(install) = t2 {
            verdicts.push(("T2".to_string(), run(&program, Tier::T1, false, Some(install))));
        }
        if verdicts.iter().any(|(_, v)| *v == Verdict::Stopped) {
            skipped += 1;
            continue;
        }
        if verdicts.iter().any(|(_, v)| *v != verdicts[0].1) {
            mismatches.push(Mismatch { seed, index, program, tiers: verdicts });
        }
    }
    (mismatches, skipped)
}
