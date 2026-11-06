//! The T1 compiler: a procedure body, or a top-level form, to `Code`.
//!
//! Resolution happens once, here, instead of on every evaluation: a
//! variable becomes a (depth, index) into the environment chain or a global
//! cell; a macro is expanded; a special form is decided. Everything the
//! compiler is unsure of it **declines**, and the caller runs the form on
//! T0 -- so a malformed `if` still raises T0's error, `let-syntax` still
//! works, and agreement between the tiers is by construction. The count of
//! declines is the honest measure of how much of a program T1 ran.
//!
//! Scoping follows T0's `lookup` exactly: the chain for the name as
//! written, then a global of that name, then a global of its
//! hygiene-resolved name. A closure created by T0 and compiled on its
//! first call resolves its free variables against the *actual* runtime
//! chain it captured, so the two engines can nest in either order.

use std::cell::{Cell, RefCell};
use crate::fx::{HashMap, HashSet};
use std::rc::Rc;

use super::{Formals, Vm};
use crate::code::{ArithOp, ArithSite, Code, Lambda, Op, Src};
use crate::heap::{AliasEnv, Obj};
use crate::syntax::SyntaxRules;
use crate::value::{ObjRef, SymId, Value};

/// Why the compiler would not take a form. Not an error: T0 gets the form.
pub(super) struct Decline(pub String);

type R<T> = Result<T, Decline>;

fn decline<T>(why: impl Into<String>) -> R<T> {
    Err(Decline(why.into()))
}

enum Var {
    /// `checked`: a slot an internal `define` fills, read through
    /// `LocalDef` so a use before the define is "unbound" as on T0.
    Local { depth: u16, idx: u16, checked: bool },
    /// S14: a variable of a flat scope of THIS unit -- a slot of the frame
    /// on the value stack.
    Flat { slot: u16, checked: bool },
    /// S14c: the name of a loop of this unit (see `Kind::Loop`).
    Loop { id: usize },
    /// S14: a variable of a flat scope of an ENCLOSING unit. Its frame is
    /// not this one, so the reference is a capture: the scope is recorded
    /// as such, its unit recompiles with it on the heap, and this pass's
    /// code -- which is void -- is never run.
    Elsewhere,
    /// A slot of a permanent frame -- a library's -- by address.
    Slot { frame: ObjRef, idx: u16 },
    Global(SymId, SymId),
    /// A macro bound in a scope: a `let-syntax`/`letrec-syntax` this unit
    /// (or one it is nested in) compiled -- `scoped`, whose renames name
    /// the binding form ([`AliasEnv::Scope`]) -- or a finished library's
    /// own, whose renames name the library frame. A use is expanded here,
    /// at compile time; any other use of the name declines.
    Macro { rules: Rc<SyntaxRules>, scoped: bool },
}

/// The macros a `let-syntax`/`letrec-syntax` binds, in binding order: the
/// first names of the scope at the index they are recorded with.
type SyntaxScope = Rc<[Rc<SyntaxRules>]>;

/// S14. What a compile-time scope is at run time.
#[derive(Clone)]
enum Kind {
    /// An `Obj::Env` in the environment chain, read by (depth, index).
    Heap,
    /// Slots of the frame on the value stack: `slots[i]` holds the scope's
    /// `i`th name. `unit` and `id` name the scope across the nested
    /// compilers its body spawns, so one that reaches into it can say so.
    Flat { slots: Vec<u16>, unit: usize, id: usize },
    /// S14c: a named `let`'s name, compiled as a LOOP -- no closure and no
    /// runtime frame. Only a call of it in loop-tail position means
    /// anything; any other use is a capture, and the unit recompiles the
    /// loop as the closure T0 would make.
    Loop { unit: usize, id: usize },
}

/// S14c: a named `let` compiled as a loop -- its variables' slots, the
/// slots its body's internal defines need, and the pc its body starts at.
struct LoopInfo {
    id: usize,
    slots: Vec<u16>,
    defs: Vec<u16>,
    top: u32,
}

/// S14. Shared by a unit's compiler and every compiler nested in it.
#[derive(Default)]
struct Capture {
    next_unit: Cell<usize>,
    /// `(unit, scope id)` of every flat scope that something outside its
    /// frame referred to -- a nested `lambda`, a `%try`/`%parameterize`
    /// block, a hand-off to T0. Each must be an `Obj::Env`, and its unit
    /// compiles again with it on the heap.
    captured: RefCell<HashSet<(usize, usize)>>,
}

/// S14. What a unit inherits from the compiler it is nested in.
#[derive(Clone)]
struct Outer {
    scopes: Vec<Vec<SymId>>,
    syntax: Vec<(usize, SyntaxScope)>,
    defs_from: Vec<usize>,
    kinds: Vec<Kind>,
    runtime: Option<ObjRef>,
    cap: Rc<Capture>,
}

struct Cx<'a> {
    vm: &'a mut Vm,
    /// Compile-time frames, innermost last. A `Heap` one becomes one
    /// runtime environment frame; a `Flat` one becomes frame slots.
    scopes: Vec<Vec<SymId>>,
    kinds: Vec<Kind>,
    /// Which scopes are a `let-syntax`/`letrec-syntax`'s, innermost last:
    /// `(index into scopes, its macros)`.
    syntax: Vec<(usize, SyntaxScope)>,
    cap: Rc<Capture>,
    /// This unit's id, and the ids of its scopes the capture pass forced
    /// onto the heap. Scope ids count up in the order scopes open, which a
    /// recompile repeats exactly.
    unit: usize,
    force_heap: Rc<HashSet<usize>>,
    next_scope: usize,
    nslots: u16,
    flat_params: bool,
    slot_names: Vec<(u16, SymId, u32, u32)>,
    /// S14c: the loops this unit compiles as jumps, innermost last, and
    /// the ones the expression being compiled is in TAIL position of --
    /// where a call of the loop's name may jump instead of calling.
    loops: Vec<LoopInfo>,
    lt: Vec<usize>,
    /// For each scope, the index from which its slots belong to internal
    /// defines rather than parameters or `let` variables.
    defs_from: Vec<usize>,
    /// The runtime chain beneath the compile-time frames, for a closure
    /// compiled after its environment already exists.
    runtime: Option<ObjRef>,
    ops: Vec<Op>,
    consts: Vec<Value>,
    globals: Vec<(SymId, SymId)>,
    global_index: HashMap<SymId, u32>,
    lambdas: Vec<Rc<Lambda>>,
    names: Vec<Rc<Vec<SymId>>>,
    forms: Vec<Value>,
    blocks: Vec<Rc<Code>>,
    ariths: Vec<ArithSite>,
    calls: Vec<(u32, u32, u32)>,
    defines: Rc<Vec<SymId>>,
    /// While a body is scanned for its defines: every head the scan
    /// expanded through, so `scan_body` can tell whether the expansion is
    /// still good once the body's own names are in scope.
    scanning: u32,
    heads: Vec<SymId>,
}

impl Vm {
    /// A top-level form. Declines what T0 should see: macro definitions,
    /// malformed syntax, anything with a local macro.
    pub(super) fn compile_toplevel(&mut self, form: Value) -> R<Rc<Code>> {
        let outer = Outer::root(None);
        unit(self, &outer, |cx| {
            // A top-level `define-syntax` is the one syntax definition that
            // can be handed to T0 as a sub-form: nothing in this unit was
            // compiled against it. Inside a body the uses that follow it
            // would have been, so there it declines the unit.
            let expanded = cx.expand(form)?;
            if let Value::Obj(r) = expanded
                && let Obj::Pair(Value::Symbol(h), _) = *cx.vm.engine.obj(r)
                && let Some(sp) = cx.special(h)?
                && (sp == cx.vm.syms.define_syntax
                    || sp == cx.vm.syms.define_library
                    || sp == cx.vm.syms.import
                    || sp == cx.vm.syms.cond_expand)
            {
                // Library forms are T0's as well: nothing in this unit was
                // compiled against what they bind.
                cx.interp(expanded)?;
                cx.ops.push(Op::Return);
                return Ok(());
            }
            // Not in tail position: a top-level unit has no caller to
            // return to, so nothing is lost.
            cx.expr(expanded, false)?;
            cx.ops.push(Op::Return);
            Ok(())
        })
    }

    /// A closure's body, given the environment it captured. Called on the
    /// first application of a closure T0 created.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compile_closure(
        &mut self,
        params: &[SymId],
        opt: &[crate::heap::Param],
        named: &[crate::heap::NamedParam],
        keys: Option<SymId>,
        rest: Option<SymId>,
        body: &[Value],
        env: Option<ObjRef>,
    ) -> R<Rc<Code>> {
        let mut frame: Vec<SymId> = params.to_vec();
        frame.extend(opt.iter().map(|p| p.name));
        frame.extend(rest);
        frame.extend(named.iter().map(|p| p.name));
        frame.extend(keys);
        // S26: the supplied-flags, after every parameter -- `apply` binds
        // them in this order.
        frame.extend(opt.iter().map(|p| p.given));
        frame.extend(named.iter().map(|p| p.given));
        let plain = opt.is_empty() && named.is_empty() && keys.is_none() && rest.is_none();
        let outer = Outer::root(env);
        unit(self, &outer, |cx| cx.body_unit(frame.clone(), body, plain))
    }
}

impl Outer {
    fn root(runtime: Option<ObjRef>) -> Self {
        Outer {
            scopes: Vec::new(),
            syntax: Vec::new(),
            defs_from: Vec::new(),
            kinds: Vec::new(),
            runtime,
            cap: Rc::new(Capture::default()),
        }
    }
}

/// S14. Compile one unit -- a procedure body, a top-level form, a block --
/// until no flat scope of it is captured. `build` emits the code; a pass in
/// which something reached into one of this unit's flat scopes from outside
/// its frame is thrown away, and the unit compiles again with those scopes
/// on the heap. Compilation is deterministic, so the scopes open in the
/// same order and keep their ids; each pass forces at least one more scope
/// onto the heap, so the loop ends.
fn unit(vm: &mut Vm, outer: &Outer, build: impl Fn(&mut Cx) -> R<()>) -> R<Rc<Code>> {
    let id = outer.cap.next_unit.get();
    outer.cap.next_unit.set(id + 1);
    let mut force: HashSet<usize> = HashSet::default();
    loop {
        let mut cx = Cx::new(vm, outer, id, Rc::new(force.clone()));
        build(&mut cx)?;
        let more: Vec<usize> = outer
            .cap
            .captured
            .borrow()
            .iter()
            .filter(|&&(u, s)| u == id && !force.contains(&s))
            .map(|&(_, s)| s)
            .collect();
        if more.is_empty() {
            return Ok(cx.finish());
        }
        force.extend(more);
    }
}

impl<'a> Cx<'a> {
    fn new(vm: &'a mut Vm, outer: &Outer, unit: usize, force_heap: Rc<HashSet<usize>>) -> Self {
        Cx {
            vm,
            scopes: outer.scopes.clone(),
            kinds: outer.kinds.clone(),
            syntax: outer.syntax.clone(),
            cap: Rc::clone(&outer.cap),
            unit,
            force_heap,
            next_scope: 0,
            nslots: 0,
            flat_params: false,
            slot_names: Vec::new(),
            loops: Vec::new(),
            lt: Vec::new(),
            defs_from: outer.defs_from.clone(),
            runtime: outer.runtime,
            ops: Vec::new(),
            consts: Vec::new(),
            globals: Vec::new(),
            global_index: HashMap::default(),
            lambdas: Vec::new(),
            names: Vec::new(),
            forms: Vec::new(),
            blocks: Vec::new(),
            ariths: Vec::new(),
            calls: Vec::new(),
            defines: Rc::new(Vec::new()),
            scanning: 0,
            heads: Vec::new(),
        }
    }

    /// What a unit nested here inherits.
    fn outer(&self) -> Outer {
        Outer {
            scopes: self.scopes.clone(),
            syntax: self.syntax.clone(),
            defs_from: self.defs_from.clone(),
            kinds: self.kinds.clone(),
            runtime: self.runtime,
            cap: Rc::clone(&self.cap),
        }
    }

    /// The next scope of this unit: flat, unless a previous pass found it
    /// captured. Answers its id and whether it is flat.
    fn open_scope(&mut self, flat_wanted: bool) -> (usize, bool) {
        let id = self.next_scope;
        self.next_scope += 1;
        (id, flat_wanted && !self.force_heap.contains(&id))
    }

    fn new_slot(&mut self) -> R<u16> {
        let s = self.nslots;
        self.nslots = self.nslots.checked_add(1).ok_or_else(|| Decline("too many variables for one frame".into()))?;
        Ok(s)
    }

    fn finish(self) -> Rc<Code> {
        Rc::new(Code {
            ops: self.ops,
            consts: self.consts,
            globals: self.globals,
            lambdas: self.lambdas,
            names: self.names,
            forms: self.forms,
            blocks: self.blocks,
            ariths: self.ariths,
            calls: self.calls,
            defines: self.defines,
            macro_gen: self.vm.macro_gen,
            hits: Cell::new(0),
            native: Cell::new(None),
            native_declined: Cell::new(false),
            max_depth: Cell::new(0),
            nslots: self.nslots,
            flat_params: self.flat_params,
            slot_names: self.slot_names,
        })
    }

    /// A procedure body in a fresh frame of `frame` (its parameters), plus
    /// the slots its internal defines need. Ends in `Return`.
    ///
    /// S14: with a PLAIN lambda list the parameter scope may be flat: slot 0
    /// is the callee and the parameters follow, where the call left them,
    /// then the defines. Anything else keeps today's `Obj::Env`.
    fn body_unit(&mut self, mut frame: Vec<SymId>, body: &[Value], plain: bool) -> R<()> {
        if body.is_empty() {
            return decline("an empty body");
        }
        let mut defs = Vec::new();
        let body = &self.scan_body(body, &frame, &mut defs)?;
        self.defs_from.push(frame.len());
        frame.extend(defs.iter().copied());
        let (id, flat) = self.open_scope(plain);
        if flat {
            self.flat_params = true;
            self.new_slot()?; // the callee
            let mut slots = Vec::with_capacity(frame.len());
            for &name in &frame {
                let s = self.new_slot()?;
                slots.push(s);
                self.slot_names.push((s, name, 0, u32::MAX));
            }
            self.kinds.push(Kind::Flat { slots, unit: self.unit, id });
        } else {
            self.kinds.push(Kind::Heap);
        }
        self.defines = Rc::new(defs);
        self.scopes.push(frame);
        self.body(body, true)?;
        self.ops.push(Op::Return);
        Ok(())
    }

    // ---- resolution ------------------------------------------------------

    /// T0's `lookup` -- the chain for the name as written, then a global
    /// of that name, then a renamed name where its macro was defined, then
    /// the global of the hygiene-resolved name.
    fn resolve(&self, sym: SymId) -> R<Var> {
        if let Some(v) = self.resolve_exact(sym, self.scopes.len())? {
            return Ok(v);
        }
        let resolved = self.vm.engine.resolve(sym);
        if resolved == sym || self.vm.global(sym).is_some() {
            return Ok(Var::Global(sym, resolved));
        }
        match self.vm.engine.alias_env(sym) {
            // A template-introduced name from a macro defined in a *local*
            // scope means what it meant there (T0's `alias_env` step). A
            // library's frame is such a scope and it is permanent, so a
            // reference into it is a direct slot read, and once the library
            // has loaded a name absent from it is absent for good and means
            // its global. Any other frame has no address this compiler can
            // give it, and T0 keeps those closures -- the suite's "forward
            // hygienic refs" case.
            Some(AliasEnv::Frame(def)) => {
                if let Obj::Env { parent: None, slots } = self.vm.engine.obj(def) {
                    if let Some(idx) = slots.iter().rposition(|(s, _)| self.vm.engine.resolve(*s) == resolved) {
                        if let Value::Obj(m) = slots[idx].1
                            && let Obj::Macro(rules) = self.vm.engine.obj(m)
                        {
                            if self.vm.sealed_library_frame(def) {
                                return Ok(Var::Macro { rules: Rc::clone(rules), scoped: false });
                            }
                            return decline("a local macro");
                        }
                        return Ok(Var::Slot { frame: def, idx: narrow(idx)? });
                    }
                    if self.vm.sealed_library_frame(def) {
                        return Ok(Var::Global(sym, resolved));
                    }
                }
                decline("a reference through a locally defined macro")
            }
            // A `let-syntax`/`letrec-syntax` macro's: what the name means
            // at that form, which is the scopes outside it (or, recursive,
            // up to and including it) -- however many scopes the use is
            // nested in since.
            Some(AliasEnv::Scope { origin, recursive }) => {
                let Some(k) = self.syntax_scope(origin) else {
                    return decline("a reference through a locally defined macro");
                };
                self.capture_scope(k);
                let top = if recursive { k + 1 } else { k };
                Ok(self.resolve_exact(resolved, top)?.unwrap_or(Var::Global(sym, resolved)))
            }
            Some(AliasEnv::Global) | None => Ok(Var::Global(sym, resolved)),
        }
    }

    /// `sym`, exactly as written, in `scopes[..top]` and then the runtime
    /// chain beneath them -- T0's `lookup_exact` without the globals. The
    /// depth counts every heap scope above `top` too, since the code runs
    /// at the innermost one.
    fn resolve_exact(&self, sym: SymId, top: usize) -> R<Option<Var>> {
        let mut depth: usize = self.kinds[top..].iter().filter(|k| matches!(k, Kind::Heap)).count();
        for i in (0..top).rev() {
            let (scope, from, kind) = (&self.scopes[i], self.defs_from[i], &self.kinds[i]);
            if let Some(idx) = scope.iter().rposition(|&s| s == sym) {
                if let Some(rules) = self.syntax_macro(i, idx) {
                    self.capture_scope(i);
                    return Ok(Some(Var::Macro { rules, scoped: true }));
                }
                return Ok(Some(match kind {
                    Kind::Heap => Var::Local { depth: narrow(depth)?, idx: narrow(idx)?, checked: idx >= from },
                    Kind::Flat { slots, unit, .. } if *unit == self.unit => {
                        Var::Flat { slot: slots[idx], checked: idx >= from }
                    }
                    Kind::Flat { unit, id, .. } => {
                        self.cap.captured.borrow_mut().insert((*unit, *id));
                        Var::Elsewhere
                    }
                    Kind::Loop { unit, id } if *unit == self.unit => Var::Loop { id: *id },
                    Kind::Loop { unit, id } => {
                        self.cap.captured.borrow_mut().insert((*unit, *id));
                        Var::Elsewhere
                    }
                }));
            }
            if matches!(kind, Kind::Heap) {
                depth += 1;
            }
        }
        let mut cur = self.runtime;
        while let Some(r) = cur {
            let Obj::Env { parent, slots } = self.vm.engine.obj(r) else {
                return decline("a corrupt environment");
            };
            if let Some(idx) = slots.iter().rposition(|(s, _)| *s == sym) {
                if let Value::Obj(m) = slots[idx].1
                    && let Obj::Macro(rules) = self.vm.engine.obj(m)
                {
                    // A finished library's macro is one object for good;
                    // any other frame's is made per evaluation.
                    if parent.is_none() && self.vm.sealed_library_frame(r) {
                        return Ok(Some(Var::Macro { rules: Rc::clone(rules), scoped: false }));
                    }
                    return decline("a local macro");
                }
                // A frame that already exists: whether this slot is a
                // define's cannot be told, so it is read checked.
                return Ok(Some(Var::Local { depth: narrow(depth)?, idx: narrow(idx)?, checked: true }));
            }
            depth += 1;
            cur = *parent;
        }
        Ok(None)
    }

    /// The macro at `idx` of scope `i`, when that scope is a
    /// `let-syntax`'s and the slot is one of its macros.
    fn syntax_macro(&self, i: usize, idx: usize) -> Option<Rc<SyntaxRules>> {
        let (_, macros) = self.syntax.iter().rev().find(|(at, _)| *at == i)?;
        macros.get(idx).cloned()
    }

    /// The innermost scope whose `let-syntax` made the rules `origin`.
    fn syntax_scope(&self, origin: u64) -> Option<usize> {
        self.syntax
            .iter()
            .rev()
            .find(|(_, macros)| macros.iter().any(|m| m.origin() == Some(origin)))
            .map(|&(at, _)| at)
    }

    /// Something in this unit means a name of scope `i`'s through its
    /// macros. If `i` is an enclosing unit's flat scope, it has to exist at
    /// run time: a closure compiled again from its environment (after a
    /// new macro) or handed to T0 finds the macro by looking there.
    fn capture_scope(&self, i: usize) {
        if let Kind::Flat { unit, id, .. } = &self.kinds[i]
            && *unit != self.unit
        {
            self.cap.captured.borrow_mut().insert((*unit, *id));
        }
    }

    fn is_local(&self, sym: SymId) -> R<bool> {
        Ok(matches!(
            self.resolve(sym)?,
            Var::Local { .. }
                | Var::Slot { .. }
                | Var::Flat { .. }
                | Var::Loop { .. }
                | Var::Elsewhere
                | Var::Macro { .. }
        ))
    }

    /// Is this head a special form here? Only when nothing shadows it --
    /// local or global -- which is T0's rule.
    fn special(&self, head: SymId) -> R<Option<SymId>> {
        if self.is_local(head)? || self.vm.global(head).is_some() {
            return Ok(None);
        }
        Ok(Some(self.vm.engine.resolve(head)))
    }

    /// Expand a macro use to a fixed point, with T0's shadowing rule: a
    /// local binding of the head means it is a call, not a macro use.
    fn expand(&mut self, mut expr: Value) -> R<Value> {
        for _ in 0..10_000 {
            let Value::Obj(r) = expr else { return Ok(expr) };
            let Obj::Pair(Value::Symbol(head), _) = *self.vm.engine.obj(r) else {
                return Ok(expr);
            };
            if self.scanning > 0 {
                self.heads.push(head);
            }
            match self.resolve(head)? {
                Var::Macro { rules, scoped } => {
                    let out = if scoped {
                        rules.expand_scoped(&mut self.vm.engine, expr)
                    } else {
                        rules.expand(&mut self.vm.engine, expr)
                    };
                    match out {
                        Ok(next) => {
                            expr = next;
                            continue;
                        }
                        Err(e) => return decline(format!("expansion failed: {e}")),
                    }
                }
                Var::Global(..) => {}
                _ => return Ok(expr),
            }
            match self.vm.expand_once(expr) {
                Ok(Some(next)) => expr = next,
                Ok(None) => return Ok(expr),
                Err(e) => return decline(format!("expansion failed: {e}")),
            }
        }
        decline("macro expansion did not terminate")
    }

    // ---- emission --------------------------------------------------------

    fn konst(&mut self, v: Value) {
        let i = self.consts.len();
        self.consts.push(v);
        self.ops.push(Op::Const(i as u32));
    }

    fn global_slot(&mut self, sym: SymId, resolved: SymId) -> u32 {
        if let Some(&i) = self.global_index.get(&sym) {
            return i;
        }
        let i = self.globals.len() as u32;
        self.globals.push((sym, resolved));
        self.global_index.insert(sym, i);
        i
    }

    fn here(&self) -> u32 {
        self.ops.len() as u32
    }

    fn patch(&mut self, at: usize, target: u32) {
        self.ops[at] = match self.ops[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            other => other,
        };
    }

    /// S30: the `Arith` for `(op a b)` just compiled -- the callee's ops at
    /// `start..callee_end`, each operand's at its entry of `bounds` -- as
    /// one `ArithSrc` where an operand is a single flat-slot read or a
    /// constant, and as a plain `Arith` otherwise.
    ///
    /// Reading a slot or a constant has no effect and cannot fail, so it
    /// can move later than it was written, with two guards. The callee's
    /// read folds in only when NO operand stays on the stack, since an
    /// operand's evaluation could redefine the operator and T0 read it
    /// first. A first operand folds in past a second that stays on the
    /// stack only when the second's ops cannot write that slot -- a `set!`
    /// of it, or a loop jump, which reassigns the loop's slots.
    fn fuse_arith(&mut self, op: ArithOp, prim: u32, form: u32, start: u32, callee_end: u32, bounds: &[(u32, u32)]) {
        let src = |s: &Self, (from, to): (u32, u32)| -> Src {
            if to != from + 1 {
                return Src::Stack;
            }
            match s.ops[from as usize] {
                Op::Arg(i) => Src::Arg(i),
                Op::Const(c) => Src::Const(c),
                _ => Src::Stack,
            }
        };
        let a = src(self, bounds[0]);
        let b = bounds.get(1).map_or(Src::Const(0), |&r| src(self, r));
        let unary = bounds.len() == 1;
        let callee_single = callee_end == start + 1 && matches!(self.ops[start as usize], Op::Global(_));
        let writes_a = |s: &Self, (from, to): (u32, u32)| -> bool {
            s.ops[from as usize..to as usize].iter().any(|o| match (a, o) {
                (Src::Arg(i), Op::SetArg(j) | Op::SetArgDef { slot: j, .. }) => i == *j,
                (_, Op::JumpCall { .. }) => true,
                _ => false,
            })
        };
        // Which reads fold: a plain `Arith` if none does.
        let fold_b = !unary && b != Src::Stack;
        let fold_a = a != Src::Stack && (unary || fold_b || !writes_a(self, bounds[1]));
        if !fold_a && !fold_b {
            let at = self.here();
            self.ops.push(Op::Arith { op, prim, form });
            self.calls.push((start, at, form));
            return;
        }
        let fold_callee = callee_single && fold_a && (unary || fold_b);
        let global = match (fold_callee, self.ops[start as usize]) {
            (true, Op::Global(g)) => Some(g),
            _ => None,
        };
        // Delete from the back so earlier pcs stay valid.
        if fold_b {
            self.delete_op(bounds[1].0 as usize);
        }
        if fold_a {
            self.delete_op(bounds[0].0 as usize);
        }
        if global.is_some() {
            self.delete_op(start as usize);
        }
        let at = self.here();
        let site = self.ariths.len() as u32;
        self.ariths.push(ArithSite {
            op,
            prim,
            form,
            global,
            a: if fold_a { a } else { Src::Stack },
            b: if fold_b { b } else { Src::Stack },
            branch: None,
        });
        self.ops.push(Op::ArithSrc(site));
        self.calls.push((if global.is_some() { at } else { start }, at, form));
    }

    /// S30: the expression just compiled is an `if`'s test; if its last op
    /// is a comparison, make it an `ArithSrc` (if it is not one already) so
    /// the `JumpIfFalse` about to follow can be folded into it. Returns the
    /// site to give the branch target to.
    fn fuse_branch(&mut self) -> Option<usize> {
        let last = self.ops.len().checked_sub(1)?;
        let is_cmp = |op: ArithOp| matches!(op, ArithOp::Eq | ArithOp::Lt | ArithOp::Gt | ArithOp::Le | ArithOp::Ge);
        match self.ops[last] {
            Op::ArithSrc(i) if is_cmp(self.ariths[i as usize].op) => Some(i as usize),
            Op::Arith { op, prim, form } if is_cmp(op) => {
                let site = self.ariths.len();
                self.ariths.push(ArithSite { op, prim, form, global: None, a: Src::Stack, b: Src::Stack, branch: None });
                self.ops[last] = Op::ArithSrc(site as u32);
                Some(site)
            }
            _ => None,
        }
    }

    /// S30: remove `ops[at]`, moving every pc after it down by one: jump
    /// targets, the pending-call ranges, the slot-name ranges, the branch
    /// targets of fused comparisons and the tops of the loops still open.
    /// A forward jump not yet patched holds 0, which no deletion moves.
    fn delete_op(&mut self, at: usize) {
        self.ops.remove(at);
        let at = at as u32;
        let fix = |t: &mut u32| {
            if *t > at {
                *t -= 1;
            }
        };
        for op in &mut self.ops {
            match op {
                Op::Jump(t) | Op::JumpIfFalse(t) | Op::JumpCall { target: t, .. } => fix(t),
                _ => {}
            }
        }
        for (s, c, _) in &mut self.calls {
            fix(s);
            fix(c);
        }
        for (_, _, lo, hi) in &mut self.slot_names {
            fix(lo);
            if *hi != u32::MAX {
                fix(hi);
            }
        }
        for site in &mut self.ariths {
            if let Some(t) = &mut site.branch {
                fix(t);
            }
        }
        for l in &mut self.loops {
            fix(&mut l.top);
        }
    }

    // ---- forms -------------------------------------------------------------

    /// A sequence: every value but the last is dropped.
    fn body(&mut self, forms: &[Value], tail: bool) -> R<()> {
        let Some((last, rest)) = forms.split_last() else {
            self.ops.push(Op::Unspecified);
            return Ok(());
        };
        for f in rest {
            self.expr(*f, false)?;
            self.ops.push(Op::Pop);
        }
        self.expr_lt(*last, tail)
    }

    /// An expression that is NOT in tail position of any loop: a call of a
    /// loop's name inside it is a real call (and so a capture). Every
    /// sub-expression goes through here except the few positions that
    /// carry loop-tail on (`expr_lt`).
    fn expr(&mut self, expr: Value, tail: bool) -> R<()> {
        let saved = std::mem::take(&mut self.lt);
        let out = self.expr_lt(expr, tail);
        self.lt = saved;
        out
    }

    /// An expression in whatever loop-tail position the caller is in: the
    /// last form of a body, an `if`'s branches, a flat `let`'s body.
    fn expr_lt(&mut self, expr: Value, tail: bool) -> R<()> {
        match expr {
            Value::Fixnum(_)
            | Value::Flonum(_)
            | Value::Bool(_)
            | Value::Char(_)
            | Value::Keyword(_)
            | Value::Primitive(_)
            | Value::Unspecified
            | Value::Eof
            | Value::Undefined => {
                self.konst(expr);
                Ok(())
            }
            Value::Null => decline("empty application: ()"),
            Value::Symbol(sym) => {
                match self.resolve(sym)? {
                    Var::Local { depth, idx, checked: false } => self.ops.push(Op::Local { depth, idx }),
                    Var::Local { depth, idx, checked: true } => self.ops.push(Op::LocalDef { depth, idx, sym: sym.0 }),
                    Var::Flat { slot, checked: false } => self.ops.push(Op::Arg(slot)),
                    Var::Flat { slot, checked: true } => self.ops.push(Op::ArgDef { slot, sym: sym.0 }),
                    // A loop's name used as a VALUE: it has to be the
                    // closure after all.
                    Var::Loop { id } => {
                        self.cap.captured.borrow_mut().insert((self.unit, id));
                        self.ops.push(Op::Unspecified);
                    }
                    // A void pass: see `Var::Elsewhere`.
                    Var::Elsewhere => self.ops.push(Op::Unspecified),
                    Var::Slot { frame, idx } => self.ops.push(Op::Slot { frame: frame.0, idx }),
                    Var::Global(s, r) => {
                        let i = self.global_slot(s, r);
                        self.ops.push(Op::Global(i));
                    }
                    // T0 answers the macro object itself; nothing should.
                    Var::Macro { .. } => return decline("a local macro used as a value"),
                }
                Ok(())
            }
            Value::Obj(r) => match self.vm.engine.obj(r).clone() {
                Obj::Str(_) | Obj::Bytevector(_) => {
                    self.konst(expr);
                    Ok(())
                }
                Obj::Vector { items, bracket } => {
                    if !bracket {
                        self.konst(expr);
                        return Ok(());
                    }
                    for it in &items {
                        self.expr(*it, false)?;
                    }
                    self.ops.push(Op::MakeVector(items.len() as u32));
                    Ok(())
                }
                Obj::Pair(..) => self.combination(expr, tail),
                // A closure, parameter, port … evaluates to itself in T0.
                _ => {
                    self.konst(expr);
                    Ok(())
                }
            },
        }
    }

    fn combination(&mut self, expr: Value, tail: bool) -> R<()> {
        let expr = self.expand(expr)?;
        // An expansion need not be a combination: `(and)` is `#t`, `(and x)`
        // is `x`.
        if !matches!(expr, Value::Obj(r) if matches!(self.vm.engine.obj(r), Obj::Pair(..))) {
            return self.expr_lt(expr, tail);
        }
        let parts = self.list(expr)?;
        let Some(&head) = parts.first() else {
            return decline("empty application: ()");
        };
        if let Value::Symbol(h) = head
            && let Some(resolved) = self.special(h)?
        {
            let s = &self.vm.syms;
            let args = &parts[1..];
            if resolved == s.given {
                return self.given(args);
            }
            if resolved == s.quote {
                return match args {
                    [v] => {
                        let d = self.vm.strip_renames(*v);
                        self.konst(d);
                        Ok(())
                    }
                    _ => decline("quote"),
                };
            }
            if resolved == s.if_ {
                return self.if_form(args, tail);
            }
            if resolved == s.begin {
                return self.body(args, tail);
            }
            if resolved == s.lambda {
                return match args {
                    [formals, body @ ..] if !body.is_empty() => self.lambda(None, *formals, body),
                    _ => decline("lambda"),
                };
            }
            if resolved == s.let_ {
                return self.let_form(args, tail);
            }
            if resolved == s.quasiquote {
                return match args {
                    [tmpl] => {
                        let form = self.vm.qq(*tmpl, 1).map_err(|e| Decline(format!("{e}")))?;
                        self.expr(form, tail)
                    }
                    _ => decline("quasiquote"),
                };
            }
            if resolved == s.try_ {
                return match args {
                    [e] => {
                        let block = self.block(*e)?;
                        self.ops.push(Op::Try(block));
                        Ok(())
                    }
                    _ => decline("%try"),
                };
            }
            if resolved == s.parameterize {
                return self.parameterize(args);
            }
            if resolved == s.set || resolved == s.define {
                return self.assign(resolved == s.define, args);
            }
            if resolved == s.define_syntax {
                // The forms after it in this body would be compiled before
                // it ran. T0 takes the whole unit.
                return decline("a syntax definition in a body");
            }
            if resolved == s.define_library || resolved == s.import || resolved == s.cond_expand {
                return decline("a library form");
            }
            if resolved == s.let_syntax || resolved == s.letrec_syntax {
                let recursive = resolved == s.letrec_syntax;
                let Value::Obj(form) = expr else { return decline("let-syntax") };
                let args = args.to_vec();
                return self.let_syntax(form, &args, tail, recursive);
            }
        }
        // S14c: a call of a loop's name. In tail position of that loop (or
        // of an inner one) with the right count, it is a JUMP: the new
        // values into the loop's slots, its defines reset, back to the
        // top. Anywhere else it must be a real call, so the loop is
        // recompiled as the closure.
        if let Value::Symbol(h) = head
            && let Var::Loop { id } = self.resolve(h)?
        {
            let args = &parts[1..];
            let jumpable = self.lt.contains(&id)
                && self.loops.iter().any(|l| l.id == id && l.slots.len() == args.len());
            if !jumpable {
                self.cap.captured.borrow_mut().insert((self.unit, id));
                self.ops.push(Op::Unspecified);
                return Ok(());
            }
            for a in args {
                self.expr(*a, false)?;
            }
            let l = self.loops.iter().find(|l| l.id == id).expect("a loop in scope");
            let (slots, defs, top) = (l.slots.clone(), l.defs.clone(), l.top);
            for &s in slots.iter().rev() {
                self.ops.push(Op::SetArg(s));
            }
            let _ = defs; // reset at `top`, on every pass through it
            let form = self.forms.len() as u32;
            self.forms.push(expr);
            self.ops.push(Op::JumpCall { target: top, form });
            // Unreachable, but the stack depth after a call is one value.
            self.ops.push(Op::Unspecified);
            return Ok(());
        }
        // S14e: two-operand arithmetic or comparison (or unary minus) on a
        // global that holds the primitive of that name right now: the
        // operands, then one `Arith` op in place of a callee and a call.
        if let Value::Symbol(h) = head
            && let Var::Global(s, r) = self.resolve(h)?
            && let Some(op) = arith_op(self.vm.engine.interner.sym_name(r), parts.len() - 1)
            && let Some(Value::Primitive(pid)) = self.vm.global(s).or_else(|| self.vm.global(r))
            && self.vm.plain_prim(pid)
        {
            let start = self.here();
            self.expr(parts[0], false)?;
            let callee_end = self.here();
            let mut bounds = Vec::with_capacity(2);
            for p in &parts[1..] {
                let from = self.here();
                self.expr(*p, false)?;
                bounds.push((from, self.here()));
            }
            let form = self.forms.len() as u32;
            self.forms.push(expr);
            self.fuse_arith(op, pid.0, form, start, callee_end, &bounds);
            return Ok(());
        }
        // An application. The callee is evaluated first, then the
        // arguments left to right, as T0 does.
        let start = self.here();
        for p in &parts {
            self.expr(*p, false)?;
        }
        let form = self.forms.len() as u32;
        self.forms.push(expr);
        let argc = (parts.len() - 1) as u32;
        let call_pc = self.here();
        self.ops.push(if tail { Op::TailCall { argc, form } } else { Op::Call { argc, form } });
        self.calls.push((start, call_pc, form));
        Ok(())
    }

    fn if_form(&mut self, args: &[Value], tail: bool) -> R<()> {
        let (test, then_e, else_e) = match args {
            [t, c] => (*t, *c, None),
            [t, c, a] => (*t, *c, Some(*a)),
            _ => return decline("if"),
        };
        self.expr(test, false)?;
        let fused = self.fuse_branch();
        let jf = self.ops.len();
        self.ops.push(Op::JumpIfFalse(0));
        self.expr_lt(then_e, tail)?;
        let jend = self.ops.len();
        self.ops.push(Op::Jump(0));
        let else_at = self.here();
        self.patch(jf, else_at);
        if let Some(site) = fused {
            self.ariths[site].branch = Some(else_at);
        }
        match else_e {
            Some(e) => self.expr_lt(e, tail)?,
            None => self.ops.push(Op::Unspecified),
        }
        let end = self.here();
        self.patch(jend, end);
        Ok(())
    }

    /// `(let [name] ((v init) …) body …)`, a core form in T0 too.
    fn let_form(&mut self, args: &[Value], tail: bool) -> R<()> {
        let (name, rest) = match args.first() {
            Some(&Value::Symbol(n)) => (Some(n), &args[1..]),
            _ => (None, args),
        };
        let Some((spec, body)) = rest.split_first() else {
            return decline("let");
        };
        if body.is_empty() {
            return decline("let");
        }
        let mut vars = Vec::new();
        let mut inits = Vec::new();
        for pair in self.binding_items(*spec)? {
            match self.binding_items(pair)?.as_slice() {
                [Value::Symbol(v), init] => {
                    vars.push(*v);
                    inits.push(*init);
                }
                _ => return decline("let"),
            }
        }
        // Inits in the enclosing scope.
        for init in &inits {
            self.expr(*init, false)?;
        }
        match name {
            None => {
                let mut defs = Vec::new();
                let body = &self.scan_body(body, &vars, &mut defs)?;
                self.let_scope(vars, defs, body, tail, None)
            }
            Some(n) => {
                // S14c: two scope ids either way, so a recompile sees the
                // same numbering: the loop's name, and its variables.
                let (lid, as_loop) = self.open_scope(true);
                let (vid, vars_flat) = self.open_scope(true);
                if as_loop && vars_flat {
                    return self.loop_form(n, lid, vid, vars, body, tail);
                }
                // The loop variable lives in a frame of its own, as in T0,
                // so the closure can see itself; the inits are already on
                // the stack, evaluated outside it.
                let ni = self.names.len() as u32;
                self.names.push(Rc::new(vec![n]));
                self.ops.push(Op::PushEnv { names: ni, ninit: 0 });
                // The loop name's frame is always a heap one: the loop
                // closure it holds refers to it.
                self.scopes.push(vec![n]);
                self.defs_from.push(1);
                self.kinds.push(Kind::Heap);
                let lam = self.lambda_inner(Some(n), vars.clone(), Vec::new(), Vec::new(), None, None, body);
                self.scopes.pop();
                self.defs_from.pop();
                self.kinds.pop();
                let lam = lam?;
                self.scopes.push(vec![n]);
                self.defs_from.push(1);
                self.kinds.push(Kind::Heap);
                self.ops.push(Op::MakeClosure(lam));
                self.ops.push(Op::SetLocal { depth: 0, idx: 0 });
                self.ops.push(Op::Local { depth: 0, idx: 0 });
                self.scopes.pop();
                self.defs_from.pop();
                self.kinds.pop();
                let argc = vars.len() as u32;
                self.ops.push(Op::InsertBelow(argc));
                let form = self.forms.len() as u32;
                let mut call = vec![Value::Symbol(n)];
                call.extend(inits.iter().copied());
                let form_v = self.vm.engine.list(&call);
                self.forms.push(form_v);
                let call_pc = self.here();
                self.ops.push(if tail { Op::TailCall { argc, form } } else { Op::Call { argc, form } });
                self.calls.push((call_pc, call_pc, form));
                if !tail {
                    self.ops.push(Op::PopEnv);
                }
                Ok(())
            }
        }
    }

    /// An unnamed `let`'s scope -- or a `let-syntax`'s, with `syn` -- once
    /// its inits are on the stack: `vars` and then the body's `defs`.
    fn let_scope(
        &mut self,
        vars: Vec<SymId>,
        defs: Vec<SymId>,
        body: &[Value],
        tail: bool,
        syn: Option<SyntaxScope>,
    ) -> R<()> {
        let mut frame = vars.clone();
        frame.extend(defs);
        let (id, flat) = self.open_scope(true);
        if flat {
            // S14: the inits are on the stack; each pops into its
            // own slot, last first, and a define's slot starts
            // undefined -- explicitly, since a slot can be entered
            // again once loops are jumps.
            let mut slots = Vec::with_capacity(frame.len());
            for _ in &frame {
                slots.push(self.new_slot()?);
            }
            for &s in slots[..vars.len()].iter().rev() {
                self.ops.push(Op::SetArg(s));
            }
            for &s in &slots[vars.len()..] {
                self.konst(Value::Undefined);
                self.ops.push(Op::SetArg(s));
            }
            let start = self.here();
            self.push_syntax(&syn);
            self.scopes.push(frame.clone());
            self.defs_from.push(vars.len());
            self.kinds.push(Kind::Flat { slots: slots.clone(), unit: self.unit, id });
            let out = self.body(body, tail);
            self.scopes.pop();
            self.defs_from.pop();
            self.kinds.pop();
            self.pop_syntax(&syn);
            out?;
            let end = self.here();
            for (&s, &n) in slots.iter().zip(&frame) {
                self.slot_names.push((s, n, start, end));
            }
            return Ok(());
        }
        let ni = self.names.len() as u32;
        self.names.push(Rc::new(frame.clone()));
        self.ops.push(Op::PushEnv { names: ni, ninit: vars.len() as u32 });
        self.push_syntax(&syn);
        self.scopes.push(frame);
        self.defs_from.push(vars.len());
        self.kinds.push(Kind::Heap);
        // Not loop-tail inside: a jump out would leave this frame
        // on the chain.
        let saved_lt = std::mem::take(&mut self.lt);
        let out = self.body(body, tail);
        self.lt = saved_lt;
        self.scopes.pop();
        self.defs_from.pop();
        self.kinds.pop();
        self.pop_syntax(&syn);
        out?;
        if !tail {
            self.ops.push(Op::PopEnv);
        }
        Ok(())
    }

    fn push_syntax(&mut self, syn: &Option<SyntaxScope>) {
        if let Some(m) = syn {
            self.syntax.push((self.scopes.len(), Rc::clone(m)));
        }
    }

    fn pop_syntax(&mut self, syn: &Option<SyntaxScope>) {
        if syn.is_some() {
            self.syntax.pop();
        }
    }

    /// `let-syntax` and `letrec-syntax`, as a scope whose slots hold the
    /// macro objects -- constants, made here once. Every use of a name in
    /// it is expanded here, so at run time the slots are read only if
    /// something hands the scope to T0 or recompiles a closure inside it
    /// from its environment; either of those captures the scope, which
    /// puts it on the heap where T0's lookup finds the macros by their
    /// origin. Otherwise it is flat and costs a store per macro.
    ///
    /// A body with definitions declines: T0 puts them in the form's own
    /// frame, and this would need the frame to hold them.
    fn let_syntax(&mut self, form: ObjRef, args: &[Value], tail: bool, recursive: bool) -> R<()> {
        let who = if recursive { "letrec-syntax" } else { "let-syntax" };
        let Some((spec, body)) = args.split_first() else {
            return decline(who);
        };
        if body.is_empty() {
            return decline(who);
        }
        let mut names = Vec::new();
        let mut macros = Vec::new();
        for (i, b) in self.binding_items(*spec)?.into_iter().enumerate() {
            match self.binding_items(b)?.as_slice() {
                [Value::Symbol(n), rules] => {
                    let parsed = self
                        .vm
                        .local_rules(form, i, *n, *rules, recursive)
                        .map_err(|e| Decline(format!("{who}: {e}")))?;
                    names.push(*n);
                    macros.push(parsed);
                }
                _ => return decline(who),
            }
        }
        let syn: SyntaxScope = macros.into();
        // The body's definitions, found with the macros in scope, since a
        // use of one may expand into a `define`.
        let some = Some(Rc::clone(&syn));
        self.push_syntax(&some);
        self.scopes.push(names.clone());
        self.defs_from.push(names.len());
        self.kinds.push(Kind::Heap);
        let mut defs = Vec::new();
        let scanned = self.scan_defines(body, &mut defs);
        self.scopes.pop();
        self.defs_from.pop();
        self.kinds.pop();
        self.pop_syntax(&some);
        scanned?;
        if !defs.is_empty() {
            return decline(format!("a definition in a {who} body"));
        }
        for rules in syn.iter() {
            let m = self.vm.engine.alloc(Obj::Macro(Rc::clone(rules)));
            self.konst(m);
        }
        self.let_scope(names, Vec::new(), body, tail, Some(syn))
    }

    /// S14c: a named `let` as a loop. The inits are on the stack; they pop
    /// into the variables' slots, the body is compiled with this loop added
    /// to the loop-tail set (on top of any loop this one is itself in tail
    /// position of), and a self-call there jumps back to `top`. Defines in
    /// the body get slots too, reset to undefined on every pass, so a read
    /// before the define is "unbound" on each iteration, as on T0.
    fn loop_form(&mut self, name: SymId, lid: usize, vid: usize, vars: Vec<SymId>, body: &[Value], tail: bool) -> R<()> {
        let mut defs = Vec::new();
        let mut binds = vars.clone();
        binds.push(name);
        let body = &self.scan_body(body, &binds, &mut defs)?;
        let mut slots = Vec::with_capacity(vars.len());
        for _ in &vars {
            slots.push(self.new_slot()?);
        }
        let mut def_slots = Vec::with_capacity(defs.len());
        for _ in &defs {
            def_slots.push(self.new_slot()?);
        }
        for &s in slots.iter().rev() {
            self.ops.push(Op::SetArg(s));
        }
        let top = self.here();
        for &s in &def_slots {
            self.konst(Value::Undefined);
            self.ops.push(Op::SetArg(s));
        }
        let mut frame = vars.clone();
        frame.extend(defs.iter().copied());
        let mut all = slots.clone();
        all.extend(def_slots.iter().copied());
        self.scopes.push(vec![name]);
        self.defs_from.push(1);
        self.kinds.push(Kind::Loop { unit: self.unit, id: lid });
        self.scopes.push(frame.clone());
        self.defs_from.push(vars.len());
        self.kinds.push(Kind::Flat { slots: all.clone(), unit: self.unit, id: vid });
        self.loops.push(LoopInfo { id: lid, slots, defs: def_slots, top });
        let mut lt = self.lt.clone();
        lt.push(lid);
        let saved_lt = std::mem::replace(&mut self.lt, lt);
        let out = self.body(body, tail);
        self.lt = saved_lt;
        self.loops.pop();
        for _ in 0..2 {
            self.scopes.pop();
            self.defs_from.pop();
            self.kinds.pop();
        }
        out?;
        let end = self.here();
        for (&s, &n) in all.iter().zip(&frame) {
            self.slot_names.push((s, n, top, end));
        }
        Ok(())
    }

    /// S26: `(given? NAME)` is a read of NAME's supplied-flag, which `apply`
    /// binds in the same frame as the parameter. T0's rule is that the
    /// nearest binding of NAME must be that parameter; here that is "NAME
    /// and its flag resolve to the same frame". Anything else -- unbound, not
    /// a parameter, shadowed, the wrong shape -- is declined, so T0 runs the
    /// unit and raises its error at the moment it reaches the form, which is
    /// the moment T0 alone would have raised it.
    fn given(&mut self, args: &[Value]) -> R<()> {
        let [Value::Symbol(name)] = args else {
            return decline("given?: not one name");
        };
        let Some(&flag) = self.vm.given_flags.get(name) else {
            return decline("given?: not a parameter");
        };
        let same_frame = match (self.resolve(*name)?, self.resolve(flag)?) {
            (Var::Local { depth: a, .. }, Var::Local { depth: b, .. }) => a == b,
            _ => false,
        };
        if !same_frame {
            return decline("given?: not an optional or keyword parameter here");
        }
        self.expr(Value::Symbol(flag), false)
    }

    fn lambda(&mut self, name: Option<SymId>, formals: Value, body: &[Value]) -> R<()> {
        let f = self.vm.parse_formals(formals).map_err(|e| Decline(format!("{e}")))?;
        let Formals { params, opt, named, keys, rest } = f;
        let i = self.lambda_inner(name, params, opt, named, keys, rest, body)?;
        self.ops.push(Op::MakeClosure(i));
        Ok(())
    }

    /// Compile a nested lambda in the current scope chain; returns its index.
    #[allow(clippy::too_many_arguments)]
    fn lambda_inner(
        &mut self,
        name: Option<SymId>,
        params: Vec<SymId>,
        opt: Vec<crate::heap::Param>,
        named: Vec<crate::heap::NamedParam>,
        keys: Option<SymId>,
        rest: Option<SymId>,
        body: &[Value],
    ) -> R<u32> {
        if body.is_empty() {
            return decline("a procedure body cannot be empty");
        }
        let mut frame: Vec<SymId> = params.clone();
        frame.extend(opt.iter().map(|p| p.name));
        frame.extend(rest);
        frame.extend(named.iter().map(|p| p.name));
        frame.extend(keys);
        frame.extend(opt.iter().map(|p| p.given));
        frame.extend(named.iter().map(|p| p.given));
        let plain = opt.is_empty() && named.is_empty() && keys.is_none() && rest.is_none();
        let outer = self.outer();
        let code = unit(self.vm, &outer, |cx| cx.body_unit(frame.clone(), body, plain))?;
        let i = self.lambdas.len() as u32;
        self.lambdas.push(Rc::new(Lambda {
            name,
            params: Rc::new(params),
            opt: Rc::new(opt),
            named: Rc::new(named),
            keys,
            rest,
            body: Rc::new(body.to_vec()),
            code,
        }));
        Ok(i)
    }

    /// A sub-block for `%try` and `%parameterize`: same scope, own code,
    /// run as a nested frame that returns to this one.
    fn block(&mut self, e: Value) -> R<u32> {
        // A unit of its own, run as a nested frame with its own base: a
        // flat variable of this frame it refers to is a capture.
        let outer = self.outer();
        let code = unit(self.vm, &outer, |cx| {
            cx.expr(e, true)?;
            cx.ops.push(Op::Return);
            Ok(())
        })?;
        let i = self.blocks.len() as u32;
        self.blocks.push(code);
        Ok(i)
    }

    fn parameterize(&mut self, args: &[Value]) -> R<()> {
        let Some((spec, body)) = args.split_first() else {
            return decline("%parameterize");
        };
        if body.is_empty() {
            return decline("%parameterize");
        }
        let pairs = self.list(*spec)?;
        let mut n = 0u32;
        for p in pairs {
            match self.list(p)?.as_slice() {
                [param, value] => {
                    self.expr(*param, false)?;
                    self.expr(*value, false)?;
                    n += 1;
                }
                _ => return decline("%parameterize"),
            }
        }
        let mut forms = vec![self.vm.engine.sym("begin")];
        forms.extend_from_slice(body);
        let body_form = self.vm.engine.list(&forms);
        let block = self.block(body_form)?;
        self.ops.push(Op::Parameterize { pairs: n, block });
        Ok(())
    }

    /// `define` and `set!`. An internal define writes a pre-allocated slot;
    /// a top-level define creates a global; `set!` finds its binding as T0's
    /// `bind` does.
    fn assign(&mut self, define: bool, args: &[Value]) -> R<()> {
        let (sym, value_is_closure) = match args {
            [Value::Obj(r), body @ ..] if define => {
                let Obj::Pair(name, formals) = *self.vm.engine.obj(*r) else {
                    return decline("define");
                };
                let Value::Symbol(name_sym) = name else {
                    return decline("define");
                };
                if body.is_empty() {
                    return decline("define");
                }
                // A docstring is a property of the definition, as in T0.
                if body.len() > 1
                    && let Value::Obj(r0) = body[0]
                    && let Obj::Str(chars) = self.vm.engine.obj(r0)
                {
                    let doc: String = chars.iter().collect();
                    let key = self.vm.engine.resolve(name_sym);
                    self.vm.docs.insert(key, doc);
                }
                self.lambda(Some(name_sym), formals, body)?;
                (name_sym, true)
            }
            [Value::Symbol(sym)] if define => {
                self.ops.push(Op::Unspecified);
                (*sym, false)
            }
            [Value::Symbol(sym), value] => {
                self.expr(*value, false)?;
                (*sym, false)
            }
            _ => return decline(if define { "define" } else { "set!" }),
        };
        let _ = value_is_closure;
        if define {
            // T0 defines into the innermost frame when there is one. The
            // slot was reserved by the body scan; if it was not, the form
            // sits somewhere the scan did not look, and T0 can have it.
            if let Some(scope) = self.scopes.last() {
                match scope.iter().rposition(|&s| s == sym) {
                    Some(idx) => match self.kinds.last() {
                        Some(Kind::Flat { slots, .. }) => self.ops.push(Op::SetArg(slots[idx])),
                        _ => self.ops.push(Op::SetLocal { depth: 0, idx: narrow(idx)? }),
                    },
                    None => return decline("a define outside a body"),
                }
            } else if self.runtime.is_some() {
                return decline("a define into an interpreted frame");
            } else {
                let r = self.vm.engine.resolve(sym);
                let i = self.global_slot(sym, r);
                self.ops.push(Op::DefineGlobal(i));
            }
        } else {
            // T0's `bind` follows a renamed name only to a global, never
            // into the frame a local macro was defined in.
            if matches!(self.vm.engine.alias_env(sym), Some(AliasEnv::Scope { .. }))
                && self.resolve_exact(sym, self.scopes.len())?.is_none()
            {
                return decline("set! of a name a local macro introduced");
            }
            match self.resolve(sym)? {
                Var::Macro { .. } => return decline("set! of a local macro"),
                Var::Local { depth, idx, checked: false } => self.ops.push(Op::SetLocal { depth, idx }),
                Var::Local { depth, idx, checked: true } => self.ops.push(Op::SetLocalDef { depth, idx, sym: sym.0 }),
                Var::Flat { slot, checked: false } => self.ops.push(Op::SetArg(slot)),
                Var::Flat { slot, checked: true } => self.ops.push(Op::SetArgDef { slot, sym: sym.0 }),
                Var::Loop { id } => {
                    // `set!` of a loop's name: it has to be the closure.
                    self.cap.captured.borrow_mut().insert((self.unit, id));
                    self.ops.push(Op::Pop);
                }
                Var::Elsewhere => self.ops.push(Op::Pop),
                Var::Slot { .. } => return decline("set! of a library binding"),
                Var::Global(s, r) => {
                    let i = self.global_slot(s, r);
                    self.ops.push(Op::SetGlobal(i));
                }
            }
        }
        self.ops.push(Op::Unspecified);
        Ok(())
    }

    /// The names a body's internal defines bind, looking through `begin`
    /// and the branches of `if` as T0's dynamic `bind` would reach them,
    /// but not into forms that open frames of their own.
    /// Scan a body for its defines, and answer the forms the body should
    /// compile: the scan's own expansions, unless a head they went through
    /// is one of `binds` or the defines -- the names the body's scope is
    /// about to bind, which the scan ran without, so a second expansion
    /// could differ. Then the forms as written. Expanding every body form
    /// twice, once here and once to compile it, was 43% of all expansion.
    fn scan_body(&mut self, forms: &[Value], binds: &[SymId], out: &mut Vec<SymId>) -> R<Vec<Value>> {
        let mark = self.heads.len();
        self.scanning += 1;
        let scanned = self.scan_expanded(forms, out);
        self.scanning -= 1;
        let expanded = scanned?;
        let clash = self.heads[mark..].iter().any(|&h| {
            let r = self.vm.engine.resolve(h);
            [h, r].iter().any(|n| binds.contains(n) || out.contains(n))
        });
        self.heads.truncate(mark);
        Ok(if clash { forms.to_vec() } else { expanded })
    }

    fn scan_expanded(&mut self, forms: &[Value], out: &mut Vec<SymId>) -> R<Vec<Value>> {
        let mut expanded = Vec::with_capacity(forms.len());
        for &f in forms {
            let e = self.expand(f)?;
            self.scan_defines(&[e], out)?;
            expanded.push(e);
        }
        Ok(expanded)
    }

    fn scan_defines(&mut self, forms: &[Value], out: &mut Vec<SymId>) -> R<()> {
        for &f in forms {
            let f = self.expand(f)?;
            let Value::Obj(r) = f else { continue };
            if !matches!(self.vm.engine.obj(r), Obj::Pair(..)) {
                continue;
            }
            let Obj::Pair(Value::Symbol(h), _) = *self.vm.engine.obj(r) else { continue };
            let Some(resolved) = self.special(h)? else { continue };
            let s = &self.vm.syms;
            if resolved == s.define {
                let parts = self.list(f)?;
                let name = match parts.get(1) {
                    Some(Value::Symbol(n)) => *n,
                    Some(&Value::Obj(p)) => match *self.vm.engine.obj(p) {
                        Obj::Pair(Value::Symbol(n), _) => n,
                        _ => return decline("define"),
                    },
                    _ => return decline("define"),
                };
                if !out.contains(&name) {
                    out.push(name);
                }
            } else if resolved == s.begin {
                let parts = self.list(f)?;
                self.scan_defines(&parts[1..], out)?;
            } else if resolved == s.if_ {
                let parts = self.list(f)?;
                if parts.len() > 2 {
                    self.scan_defines(&parts[2..], out)?;
                }
            }
        }
        Ok(())
    }

    /// Hand `form` to T0 at run time. T0 is handed the environment, and
    /// it can name any variable in scope, so every flat scope in sight is
    /// captured -- this unit's, which recompile on the heap, and any
    /// enclosing unit's.
    fn interp(&mut self, form: Value) -> R<()> {
        for k in &self.kinds {
            if let Kind::Flat { unit, id, .. } | Kind::Loop { unit, id } = k {
                self.cap.captured.borrow_mut().insert((*unit, *id));
            }
        }
        let i = self.forms.len() as u32;
        self.forms.push(form);
        self.ops.push(Op::Interp(i));
        self.vm.interp_forms += 1;
        Ok(())
    }

    // ---- helpers -------------------------------------------------------------

    fn list(&self, v: Value) -> R<Vec<Value>> {
        self.vm.list_to_vec(v).map_err(|e| {
            Decline(format!("{e}: {}", super::abbreviate(&crate::writer::write_str(&self.vm.engine, v))))
        })
    }

    fn binding_items(&self, v: Value) -> R<Vec<Value>> {
        self.vm.binding_items(v).map_err(|e| Decline(format!("{e}")))
    }
}

/// S14e: which `ArithOp` a call of NAME with ARGC operands is, if any.
fn arith_op(name: &str, argc: usize) -> Option<ArithOp> {
    Some(match (name, argc) {
        ("+", 2) => ArithOp::Add,
        ("-", 2) => ArithOp::Sub,
        ("-", 1) => ArithOp::Neg,
        ("*", 2) => ArithOp::Mul,
        ("/", 2) => ArithOp::Div,
        ("=", 2) => ArithOp::Eq,
        ("<", 2) => ArithOp::Lt,
        (">", 2) => ArithOp::Gt,
        ("<=", 2) => ArithOp::Le,
        (">=", 2) => ArithOp::Ge,
        _ => return None,
    })
}

fn narrow(n: usize) -> R<u16> {
    u16::try_from(n).map_err(|_| Decline("too many variables for one frame".into()))
}
