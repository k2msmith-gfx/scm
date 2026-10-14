//! T1 -- the bytecode a procedure body compiles to (plan §5, S12).
//!
//! A `Code` is what T1 runs where T0 walks source: the same explicit frame
//! stack, the same environment objects, the same `apply`, the same catch
//! and wind frames -- so a compiled frame and an interpreted one can sit on
//! one stack, and anything the compiler declines runs on T0 unchanged. The
//! difference is that variables are resolved to a (depth, index) once,
//! macros are expanded once, special forms are decided once, and arguments
//! are evaluated straight onto a value stack with no vector per call.

use std::cell::Cell;
use std::rc::Rc;

use crate::heap::{NamedParam, Param};
use crate::value::{SymId, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Push `consts[i]`.
    Const(u32),
    /// Push the variable `idx` slots into the frame `depth` levels up.
    Local { depth: u16, idx: u16 },
    /// Pop into that slot.
    SetLocal { depth: u16, idx: u16 },
    /// As `Local`, for a slot an internal `define` fills: `Undefined` there
    /// means the define has not run, and the read raises "unbound
    /// variable" as T0 does. `sym` names it for the message.
    LocalDef { depth: u16, idx: u16, sym: u32 },
    /// As `SetLocal` for such a slot: `set!` before the define is an error.
    SetLocalDef { depth: u16, idx: u16, sym: u32 },
    /// Push slot `idx` of the permanent frame `frame` -- a library's -- by
    /// address: how a library macro's reference to a library-private
    /// binding compiles instead of declining.
    Slot { frame: u32, idx: u16 },
    /// Push the global named by `globals[i]`, or fail as T0 does.
    Global(u32),
    /// Pop into the global, with `set!`'s rule: it must already exist.
    SetGlobal(u32),
    /// Pop into the global, creating it.
    DefineGlobal(u32),
    Unspecified,
    Pop,
    Jump(u32),
    /// Pop; branch if false.
    JumpIfFalse(u32),
    /// The callee and `argc` arguments are on the stack, callee lowest.
    Call { argc: u32, form: u32 },
    /// The same, discarding this frame first: a proper tail call.
    TailCall { argc: u32, form: u32 },
    /// Pop the frame's result and leave.
    Return,
    /// Push a closure over the current environment from `lambdas[i]`.
    MakeClosure(u32),
    /// Open a new environment frame named by `names[i]`, taking `ninit`
    /// values from the stack and leaving the rest unspecified.
    PushEnv { names: u32, ninit: u32 },
    /// Back to the parent frame.
    PopEnv,
    /// Move the top of the stack below the `n` values beneath it: how a
    /// named `let`'s loop closure gets under its already-evaluated inits.
    InsertBelow(u32),
    /// Pop `n` values into a `[…]` vector.
    MakeVector(u32),
    /// `(%try e)`: run `blocks[i]` under a catch frame.
    Try(u32),
    /// `(%parameterize ((p v) …) body)`: `pairs` parameter/value pairs are
    /// on the stack; install them, run `blocks[i]`, restore.
    Parameterize { pairs: u32, block: u32 },
    /// Hand `forms[i]` to T0 in the current environment and resume with
    /// its value: the fallback for a sub-form the compiler does not take,
    /// such as a macro definition inside a body.
    Interp(u32),
}

/// A nested `lambda`: its formals as T0 parses them, its body as source
/// (for a recompile and for T0's default-argument evaluation), and its code.
pub struct Lambda {
    pub name: Option<SymId>,
    pub params: Rc<Vec<SymId>>,
    pub opt: Rc<Vec<Param>>,
    pub named: Rc<Vec<NamedParam>>,
    pub keys: Option<SymId>,
    pub rest: Option<SymId>,
    pub body: Rc<Vec<Value>>,
    pub code: Rc<Code>,
}

impl std::fmt::Debug for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Code({} ops, {} consts, {} lambdas)", self.ops.len(), self.consts.len(), self.lambdas.len())
    }
}

pub struct Code {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
    /// `(name, resolved)`: the name as written and its hygiene-resolved
    /// form, tried in that order, as T0's lookup does.
    pub globals: Vec<(SymId, SymId)>,
    pub lambdas: Vec<Rc<Lambda>>,
    pub names: Vec<Rc<Vec<SymId>>>,
    /// The call forms, for `current_call` and the backtrace.
    pub forms: Vec<Value>,
    pub blocks: Vec<Rc<Code>>,
    /// For each call op: the pc where its callee's evaluation began, the pc
    /// of the op itself, and its form. Sorted by start. A backtrace asks
    /// which calls are *pending* at a pc -- the ones whose range contains
    /// it -- which is the same frame T0's `Operand` would have shown, at no
    /// cost per call.
    pub calls: Vec<(u32, u32, u32)>,
    /// Slots a procedure body's internal `define`s need, appended to the
    /// call frame after the parameters.
    pub defines: Rc<Vec<SymId>>,
    /// The macro table's generation this was compiled under; a later
    /// `define-syntax` recompiles on next call, so a redefined macro takes
    /// effect as it would in T0.
    pub macro_gen: u64,
    /// T2: how often this has run, the native function once it is hot, and
    /// whether the native compiler declined it. The operand-stack depth it
    /// needs is computed by that compiler.
    pub hits: Cell<u32>,
    pub native: Cell<Option<crate::vm::native::NativeFn>>,
    pub native_declined: Cell<bool>,
    pub max_depth: Cell<u32>,
}

impl Code {
    /// Every heap value this code keeps alive.
    pub fn trace(&self, out: &mut Vec<Value>) {
        out.extend(self.consts.iter().copied());
        out.extend(self.forms.iter().copied());
        for l in &self.lambdas {
            out.extend(l.body.iter().copied());
            out.extend(l.opt.iter().filter_map(|p| p.default));
            out.extend(l.named.iter().filter_map(|p| p.default));
            l.code.trace(out);
        }
        for b in &self.blocks {
            b.trace(out);
        }
    }

    /// The call forms pending at `pc` (the pc *after* the op in progress),
    /// innermost first: every call whose arguments were still being
    /// evaluated. The call *at* the op itself is not pending -- T0 has
    /// popped that frame by the time the callee runs, and `current_call`
    /// names it -- so the range is half-open. `calls` is in emission
    /// order, which is innermost first already.
    pub fn pending_calls(&self, pc: usize) -> Vec<Value> {
        let at = pc.saturating_sub(1) as u32;
        self.calls
            .iter()
            .filter(|&&(start, call, _)| start <= at && at < call)
            .map(|&(_, _, f)| self.forms[f as usize])
            .collect()
    }
}

/// Whether a closure has code, and if not, why not.
#[derive(Clone, Default)]
pub enum CodeState {
    /// Not yet asked: compiled on first call.
    #[default]
    Unknown,
    Compiled(Rc<Code>),
    /// The compiler declined, so T0 runs the body.
    Interpreted,
}

impl std::fmt::Debug for CodeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeState::Unknown => write!(f, "Unknown"),
            CodeState::Compiled(c) => write!(f, "Compiled({} ops)", c.ops.len()),
            CodeState::Interpreted => write!(f, "Interpreted"),
        }
    }
}

/// Which engine runs a procedure body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// The AST interpreter: the semantic oracle, kept forever.
    T0,
    /// The bytecode VM: the default.
    T1,
}
