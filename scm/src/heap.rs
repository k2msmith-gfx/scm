//! Heap objects, the interner, and the collector.
//!
//! Per plan §3.3: **precise, non-moving, single-threaded mark-sweep**, and
//! three of those four are consequences of earlier decisions rather than fresh
//! choices.
//!
//!   * *Precise*, because the VM's explicit stack already is the root set
//!     (§5). No conservative scanning of C frames, no LLVM statepoints.
//!   * *Non-moving*, so an `ObjRef` stays valid forever. A freed slot is
//!     reused but never relocated, which is what lets the embedder hold a
//!     value without an indirection layer.
//!   * *Single-threaded*, which is **free**: §6 restricts compiled leaf
//!     functions to pure allocation-free code, so nothing on a rayon worker
//!     ever touches this heap. No locks, no barriers, no cross-thread
//!     stop-the-world.
//!
//! The roots live in the `Vm`, so it gathers them and calls [`Engine::collect`].

use std::collections::HashMap;
use std::rc::Rc;

use crate::value::{KwId, ObjRef, SymId, Value};

/// An optional positional parameter. `default` is an *unevaluated expression*,
/// evaluated at call time only when the argument is absent -- which is the
/// whole point (plan §4): a body-side default cannot tell `:smooth #f` from an
/// omitted `:smooth`, and would rewrite the former into the latter's default.
#[derive(Clone, Debug)]
pub struct Param {
    pub name: SymId,
    pub default: Option<Value>,
}

/// A keyword parameter. The parameter's name *is* the keyword, so `kw` is
/// interned alongside `name` at closure-creation time and matching a call site
/// is a `KwId` comparison rather than a string compare.
#[derive(Clone, Debug)]
pub struct NamedParam {
    pub name: SymId,
    pub kw: KwId,
    pub default: Option<Value>,
}

/// What kind of condition an error object carries. R7RS asks `read-error?`
/// and `file-error?` to distinguish these, so the distinction has to be
/// recorded when the object is made rather than guessed at later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    Plain,
    Read,
    File,
}

/// A port. Textual ports carry characters, binary ports carry bytes, and
/// R7RS keeps them apart deliberately -- `read-char` on a binary port is an
/// error, not a reinterpretation.
/// Which element type a homogeneous numeric vector holds (SRFI 4).
///
/// `u8` is absent on purpose: R7RS's bytevector **is** the u8 vector, and
/// two representations of one thing is how they drift apart. `(srfi 4)`'s
/// `u8vector` names are the bytevector operations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NumTag {
    S8,
    S16,
    U16,
    S32,
    U32,
    S64,
    U64,
    F32,
    F64,
}

impl NumTag {
    pub fn name(self) -> &'static str {
        match self {
            NumTag::S8 => "s8",
            NumTag::S16 => "s16",
            NumTag::U16 => "u16",
            NumTag::S32 => "s32",
            NumTag::U32 => "u32",
            NumTag::S64 => "s64",
            NumTag::U64 => "u64",
            NumTag::F32 => "f32",
            NumTag::F64 => "f64",
        }
    }

    pub fn from_name(s: &str) -> Option<NumTag> {
        Some(match s {
            "s8" => NumTag::S8,
            "s16" => NumTag::S16,
            "u16" => NumTag::U16,
            "s32" => NumTag::S32,
            "u32" => NumTag::U32,
            "s64" => NumTag::S64,
            "u64" => NumTag::U64,
            "f32" => NumTag::F32,
            "f64" => NumTag::F64,
            _ => return None,
        })
    }

    pub fn is_float(self) -> bool {
        matches!(self, NumTag::F32 | NumTag::F64)
    }
}

/// A homogeneous numeric vector: the elements are stored **unboxed**, which
/// is the whole reason it exists. A `Vec<Value>` of a million flonums costs
/// sixteen bytes and a tag check per element; an `f64` image or height field
/// costs eight bytes and nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum NumVec {
    S8(Vec<i8>),
    S16(Vec<i16>),
    U16(Vec<u16>),
    S32(Vec<i32>),
    U32(Vec<u32>),
    S64(Vec<i64>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl NumVec {
    pub fn tag(&self) -> NumTag {
        match self {
            NumVec::S8(_) => NumTag::S8,
            NumVec::S16(_) => NumTag::S16,
            NumVec::U16(_) => NumTag::U16,
            NumVec::S32(_) => NumTag::S32,
            NumVec::U32(_) => NumTag::U32,
            NumVec::S64(_) => NumTag::S64,
            NumVec::U64(_) => NumTag::U64,
            NumVec::F32(_) => NumTag::F32,
            NumVec::F64(_) => NumTag::F64,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            NumVec::S8(v) => v.len(),
            NumVec::S16(v) => v.len(),
            NumVec::U16(v) => v.len(),
            NumVec::S32(v) => v.len(),
            NumVec::U32(v) => v.len(),
            NumVec::S64(v) => v.len(),
            NumVec::U64(v) => v.len(),
            NumVec::F32(v) => v.len(),
            NumVec::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Element `i` as a Scheme number: exact for the integer types,
    /// inexact for the float ones.
    pub fn get(&self, i: usize) -> Option<Value> {
        Some(match self {
            NumVec::S8(v) => Value::Fixnum(*v.get(i)? as i64),
            NumVec::S16(v) => Value::Fixnum(*v.get(i)? as i64),
            NumVec::U16(v) => Value::Fixnum(*v.get(i)? as i64),
            NumVec::S32(v) => Value::Fixnum(*v.get(i)? as i64),
            NumVec::U32(v) => Value::Fixnum(*v.get(i)? as i64),
            NumVec::S64(v) => Value::Fixnum(*v.get(i)?),
            // A u64 past i64::MAX has no exact representation here (§2.3
            // declines bignums), so say so rather than wrap into a
            // negative number.
            NumVec::U64(v) => {
                let x = *v.get(i)?;
                match i64::try_from(x) {
                    Ok(n) => Value::Fixnum(n),
                    Err(_) => Value::Flonum(x as f64),
                }
            }
            NumVec::F32(v) => Value::Flonum(*v.get(i)? as f64),
            NumVec::F64(v) => Value::Flonum(*v.get(i)?),
        })
    }

    /// Store `x` at `i`. The error names the type and the value, because
    /// "out of range for s8" is the whole content of the complaint.
    pub fn set(&mut self, i: usize, x: Value) -> Result<(), String> {
        let tag = self.tag();
        if i >= self.len() {
            return Err(format!("index {i} is out of range for a {}vector of length {}", tag.name(), self.len()));
        }
        if tag.is_float() {
            let f = match x {
                Value::Flonum(f) => f,
                Value::Fixnum(n) => n as f64,
                _ => return Err(format!("a {}vector holds numbers, not {x:?}", tag.name())),
            };
            match self {
                NumVec::F32(v) => v[i] = f as f32,
                NumVec::F64(v) => v[i] = f,
                _ => unreachable!("float tag"),
            }
            return Ok(());
        }
        // The integer types take an exact integer in range. A flonum is
        // refused rather than truncated: losing the fraction silently is
        // how a wrong answer gets stored.
        let Value::Fixnum(n) = x else {
            let what = match x {
                Value::Flonum(f) => format!("the inexact {f}"),
                other => format!("{other:?}"),
            };
            return Err(format!("a {}vector holds exact integers, not {what}", tag.name()));
        };
        macro_rules! store {
            ($v:expr, $t:ty) => {{
                match <$t>::try_from(n) {
                    Ok(y) => {
                        $v[i] = y;
                        Ok(())
                    }
                    Err(_) => Err(format!("{n} is out of range for a {}vector", tag.name())),
                }
            }};
        }
        match self {
            NumVec::S8(v) => store!(v, i8),
            NumVec::S16(v) => store!(v, i16),
            NumVec::U16(v) => store!(v, u16),
            NumVec::S32(v) => store!(v, i32),
            NumVec::U32(v) => store!(v, u32),
            NumVec::S64(v) => {
                v[i] = n;
                Ok(())
            }
            NumVec::U64(v) => store!(v, u64),
            NumVec::F32(_) | NumVec::F64(_) => unreachable!("handled above"),
        }
    }

    /// `n` elements, every one `fill`.
    pub fn make(tag: NumTag, n: usize, fill: Value) -> Result<NumVec, String> {
        let mut v = match tag {
            NumTag::S8 => NumVec::S8(vec![0; n]),
            NumTag::S16 => NumVec::S16(vec![0; n]),
            NumTag::U16 => NumVec::U16(vec![0; n]),
            NumTag::S32 => NumVec::S32(vec![0; n]),
            NumTag::U32 => NumVec::U32(vec![0; n]),
            NumTag::S64 => NumVec::S64(vec![0; n]),
            NumTag::U64 => NumVec::U64(vec![0; n]),
            NumTag::F32 => NumVec::F32(vec![0.0; n]),
            NumTag::F64 => NumVec::F64(vec![0.0; n]),
        };
        // One range check for the fill, then a fast fill.
        if n > 0 && !matches!(fill, Value::Fixnum(0)) {
            v.set(0, fill)?;
            let first = v.get(0).expect("just set");
            for i in 1..n {
                v.set(i, first)?;
            }
        }
        Ok(v)
    }

    /// A copy of `start..end`.
    pub fn slice(&self, start: usize, end: usize) -> NumVec {
        macro_rules! cut {
            ($c:path, $v:expr) => {
                $c($v[start..end].to_vec())
            };
        }
        match self {
            NumVec::S8(v) => cut!(NumVec::S8, v),
            NumVec::S16(v) => cut!(NumVec::S16, v),
            NumVec::U16(v) => cut!(NumVec::U16, v),
            NumVec::S32(v) => cut!(NumVec::S32, v),
            NumVec::U32(v) => cut!(NumVec::U32, v),
            NumVec::S64(v) => cut!(NumVec::S64, v),
            NumVec::U64(v) => cut!(NumVec::U64, v),
            NumVec::F32(v) => cut!(NumVec::F32, v),
            NumVec::F64(v) => cut!(NumVec::F64, v),
        }
    }
}

#[derive(Clone, Debug)]
pub enum PortKind {
    /// An in-memory input port over a string.
    StringIn { chars: Vec<char>, pos: usize },
    /// An in-memory output port, accumulating.
    StringOut(String),
    /// An in-memory **binary** input port.
    BytesIn { bytes: Vec<u8>, pos: usize },
    /// An in-memory **binary** output port.
    BytesOut(Vec<u8>),
    Stdout,
    Stderr,
    /// The process's stdin, **not yet read**. The first input primitive to
    /// touch it reads stdin to the end and turns it into a `StringIn`, so
    /// installing it costs nothing and a script that never reads never
    /// blocks -- eager reading hung `scm -e` under any harness that leaves
    /// stdin open, which is every CI. Only `scm-cli` creates one.
    Stdin,
    /// A file being written: buffered here, written on flush and on close.
    FileOut { path: String, buf: String },
}

impl PortKind {
    /// Whether this is an input port, which is the question most of the port
    /// predicates actually ask -- and which was spelled
    /// `matches!(k, StringIn { .. })` in five places, so adding a second kind
    /// of input port would have silently made each of them wrong.
    pub fn is_input(&self) -> bool {
        matches!(self, PortKind::StringIn { .. } | PortKind::BytesIn { .. } | PortKind::Stdin)
    }

    pub fn is_binary(&self) -> bool {
        matches!(self, PortKind::BytesIn { .. } | PortKind::BytesOut(_))
    }
}

#[derive(Clone, Debug)]
pub enum Obj {
    Pair(Value, Value),
    /// R7RS strings are mutable and indexed by character, and `full-unicode`
    /// is not claimed, so `Vec<char>` buys O(1) `string-ref` and a trivial
    /// `string-set!` for 4 bytes a character (plan §4.5).
    Str(Vec<char>),
    Vector {
        items: Vec<Value>,
        /// How it was written, which decides how it *evaluates* (plan §4.5):
        /// `[a b c]` is a constructor whose elements are evaluated, `#(a b c)`
        /// is an R7RS self-evaluating constant. Same elements, different
        /// meaning, so the datum has to remember which one it was.
        ///
        /// `equal?` ignores this. The writer does not.
        bracket: bool,
    },
    Bytevector(Vec<u8>),
    /// An exact integer outside `i64`. **Always out of range**: a value
    /// that fits is a `Value::Fixnum`, which `prim.rs` normalises to on
    /// every result, because two representations of one integer would make
    /// `eqv?` and `equal-hash` disagree with themselves.
    Bignum(crate::bigint::BigInt),
    /// A parameter object (R7RS `make-parameter`). `slot` indexes the VM's
    /// table of current values; the converter, if any, is applied by
    /// `parameterize`'s expansion rather than here, so that calling it is an
    /// ordinary Scheme call the machine already knows how to do.
    Parameter {
        slot: usize,
        converter: Option<Value>,
    },
    /// An error object (R7RS `error-object?`). A Rust-originated refusal
    /// becomes one when it is *caught*, so `guard` can ask it for a message
    /// and irritants rather than being handed a bare string.
    ErrorObj {
        message: Value,
        irritants: Value,
        kind: ErrorKind,
    },
    Port { kind: PortKind, open: bool },
    /// Several values travelling together, as produced by `(values a b)`.
    ///
    /// One value is *not* wrapped -- `(values x)` is `x` -- so this object
    /// only exists for zero or two-or-more, and only `call-with-values`
    /// unpacks it. R7RS says it is an error for it to reach a continuation
    /// wanting a single value; here it would arrive as an opaque object
    /// instead, so the printer makes it visible rather than letting it look
    /// like something ordinary.
    Values(Vec<Value>),
    /// A macro bound by `let-syntax` or `letrec-syntax`.
    ///
    /// A *local* macro lives in the ordinary environment chain, as a value,
    /// which is what gives `let-syntax` correct lexical scoping **for free**:
    /// a closure made in the body captures the frame holding the macro, so
    /// the macro is still there when the closure is called later. The
    /// alternative -- installing into the global table and removing it after
    /// -- would be dynamic scoping wearing a lexical form's clothes, and
    /// `(let-syntax ((m …)) (lambda () (m)))` would break the moment it
    /// escaped.
    Macro(Rc<crate::syntax::SyntaxRules>),
    /// An **escape** continuation, as captured by
    /// `call-with-current-continuation`.
    ///
    /// Plan §3.1 settled continuations as a three-rung ladder and committed
    /// to this rung: escape-only, which is what `call/cc` is used for in
    /// practice -- early exit from a loop, a non-local return, the
    /// `with-exception-handler` escape the suite tests. A re-entrant
    /// continuation would need the control stack copied, and the explicit
    /// stack (§5) makes that *possible* later without making it necessary
    /// now.
    ///
    /// `id` is what makes "escaped its extent" detectable: the stack depth
    /// alone would be ambiguous, since an unrelated call can reach the same
    /// depth later. Invoking a continuation searches the live stack for its
    /// mark, and not finding one is an error rather than a wild jump.
    Continuation { id: u64 },
    /// A pseudo-random generator's state. Mutable, and reachable only through
    /// a parameter, so `parameterize` can rebind it the way ray's `:ray-rng`
    /// does -- which is what makes a `:seed` replayable.
    Rng(u64),
    /// A lexical frame. The chain of these is the environment; the chain of
    /// `Frame`s in the VM is the *control* stack. Keeping them separate is
    /// what lets a closure outlive the call that made it.
    Env {
        parent: Option<ObjRef>,
        slots: Vec<(SymId, Value)>,
    },
    /// A homogeneous numeric vector (SRFI 4), stored unboxed.
    NumVector(NumVec),
    /// A multidimensional array (SRFI 63) as a **view** over a backing
    /// store: an index `(i j …)` lands at `offset + i*strides[0] +
    /// j*strides[1] + …`. Strides rather than a shape alone is what makes
    /// `make-shared-array` a view -- a row, a column, a transpose or a
    /// subrectangle shares the store instead of copying it -- and the
    /// arithmetic happens here rather than in Scheme.
    ///
    /// `store` is a numeric vector, a bytevector, a vector or a string,
    /// which is every prototype `(srfi 63)` offers.
    Array {
        dims: Rc<Vec<usize>>,
        strides: Rc<Vec<isize>>,
        offset: usize,
        store: Value,
    },
    /// A record type descriptor, from `define-record-type`. Two types with
    /// the same name are distinct objects, which is what R7RS wants and
    /// what comparing names would get wrong.
    RecordType { name: SymId, fields: Rc<Vec<SymId>> },
    /// An instance. `rtype` is the descriptor it was made from, compared by
    /// identity. A heap type of its own, rather than a tagged vector, is
    /// what makes a record **disjoint from every other type** as R7RS
    /// §5.5 requires: `vector?` is false of one, and two distinct records
    /// with equal fields are not `equal?`, because `equal?` recurses into
    /// pairs, vectors, strings and bytevectors and compares everything
    /// else by identity.
    Record { rtype: Value, fields: Vec<Value> },
    /// `params` and `body` are behind `Rc` so that applying a closure clones
    /// two pointers instead of two vectors -- a tail loop applies the same
    /// closure 10^8 times and must not allocate to do it.
    Closure {
        name: Option<SymId>,
        /// Required positionals.
        params: Rc<Vec<SymId>>,
        /// `&opt`: optional positionals, filled in order.
        opt: Rc<Vec<Param>>,
        /// `&named`: keyword arguments, matched by name.
        named: Rc<Vec<NamedParam>>,
        /// `&keys`: every keyword argument, as an alist.
        keys: Option<SymId>,
        /// A dotted tail, collecting leftover *positional* arguments.
        rest: Option<SymId>,
        body: Rc<Vec<Value>>,
        env: Option<ObjRef>,
        /// T1's compiled form of `body`, filled in on first call.
        code: crate::code::CodeState,
    },
}

/// A heap slot. Freed slots form a list through `next`, so allocation is a pop
/// and nothing is ever relocated.
#[derive(Debug)]
enum Slot {
    Free { next: Option<u32> },
    Used(Obj),
}

/// Symbols and keywords, interned separately because they are different types.
///
/// Per-`Engine`, never global. Plan §4.5 and §7: Steel's startup image was
/// invalidated by a *static* interner, and "no global interner state the image
/// cannot reconstruct" lands exactly here. It also means two engines in one
/// process cannot corrupt each other, which the modeler's scene-VM worker
/// wants.
#[derive(Default, Debug)]
pub struct Interner {
    sym_names: Vec<String>,
    sym_ids: HashMap<String, u32>,
    kw_names: Vec<String>,
    kw_ids: HashMap<String, u32>,
}

impl Interner {
    pub fn symbol(&mut self, name: &str) -> SymId {
        if let Some(&id) = self.sym_ids.get(name) {
            return SymId(id);
        }
        let id = self.sym_names.len() as u32;
        self.sym_names.push(name.to_string());
        self.sym_ids.insert(name.to_string(), id);
        SymId(id)
    }

    pub fn keyword(&mut self, name: &str) -> KwId {
        if let Some(&id) = self.kw_ids.get(name) {
            return KwId(id);
        }
        let id = self.kw_names.len() as u32;
        self.kw_names.push(name.to_string());
        self.kw_ids.insert(name.to_string(), id);
        KwId(id)
    }

    /// A symbol no source text can name.
    ///
    /// The name is pushed into the table but deliberately **not** into the
    /// lookup map, so interning the same spelling from source produces a
    /// *different* `SymId`. That is what makes a renamed, template-introduced
    /// identifier genuinely uncapturable rather than merely unlikely to
    /// collide.
    pub fn fresh(&mut self, base: &str) -> SymId {
        let id = self.sym_names.len() as u32;
        self.sym_names.push(format!("{base}.{id}"));
        SymId(id)
    }

    /// Look a symbol up *without* interning it. `param_now` needs this: asking
    /// for a name that was never defined must not create it.
    pub fn lookup(&self, name: &str) -> Option<SymId> {
        self.sym_ids.get(name).copied().map(SymId)
    }

    pub fn sym_name(&self, id: SymId) -> &str {
        &self.sym_names[id.0 as usize]
    }

    pub fn kw_name(&self, id: KwId) -> &str {
        &self.kw_names[id.0 as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcStats {
    pub live: usize,
    pub freed: usize,
    pub collections: u64,
}

/// Where a form was written.
///
/// Line and column are computed **at read time** and the source *text* is not
/// kept -- only its name, behind an `Rc` so every form in one file shares it.
/// Retaining the text would turn a long REPL session into a slow leak of
/// every expression ever typed, for information nothing reads back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loc {
    /// `"<repl:7>"`, or a file path.
    pub source: Rc<str>,
    /// 1-based, the way an editor counts.
    pub line: u32,
    pub col: u32,
}

impl std::fmt::Display for Loc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.source, self.line, self.col)
    }
}

/// A rooted value. While the handle is held, the value is a root.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handle(usize);

/// Everything a running Scheme owns below the control stack.
#[derive(Debug)]
pub struct Engine {
    slots: Vec<Slot>,
    marks: Vec<bool>,
    free_head: Option<u32>,
    used: usize,
    /// Collect when `used` reaches this; recomputed from the live set after
    /// every collection.
    threshold: usize,
    /// A floor, so a small program never collects at all.
    min_heap: usize,
    /// The host's limit (plan §3.3). Exceeding it is an ordinary Scheme error
    /// at a safepoint, never an abort.
    heap_cap: Option<usize>,
    /// Collect at every safepoint. This is how collector bugs stop being
    /// nondeterministic, and it is an S3 exit criterion rather than a debug
    /// nicety.
    stress: bool,
    /// Slot vectors of swept environment frames, kept for the next frames.
    /// A call allocates one frame; without this that was one `malloc` and
    /// one `free` per call in both tiers, and the profile said so.
    env_pool: Vec<Vec<(SymId, Value)>>,
    collections: u64,
    /// Values rooted from outside the VM -- the host's, and the VM's own
    /// source forms while it holds them across an evaluation.
    handles: Vec<Option<Value>>,
    /// Where each *compound* form was written, for backtraces (plan §8
    /// obligation 4).
    ///
    /// A side table because a `Value` is 16 bytes with no room, and only
    /// source structure needs an entry -- the size is the program's, not the
    /// execution's. Atoms are skipped: nothing in a backtrace points at a
    /// number.
    ///
    /// Pruned during the sweep. A slot that has been freed and reused would
    /// otherwise report the location of whatever *used* to live there, which
    /// is worse than reporting nothing.
    locs: HashMap<ObjRef, Loc>,
    /// Renamed macro identifiers, pointing back at what they meant where the
    /// macro was defined. Consulted only when a name is not lexically bound,
    /// which is what gives a template both hygiene and referential
    /// transparency -- see `syntax.rs`.
    /// A renamed template identifier, and **where it meant something**.
    ///
    /// The env is what `let-syntax` needs: a top-level macro's definition
    /// environment is the globals, so `None` covers every macro there was
    /// until now -- but a local macro's template may refer to a *local*
    /// binding, and `(let ((x 'outer)) (let-syntax ((m … x)) (let ((x
    /// 'inner)) (m))))` must give `outer`. Referential transparency is the
    /// half of hygiene that this is.
    ///
    /// The envs here are GC roots, which keeps a local macro's definition
    /// frame alive for as long as its alias exists. That is a real cost and
    /// it is bounded the same way the alias table itself is.
    aliases: HashMap<SymId, (SymId, Option<ObjRef>)>,
    pub interner: Interner,
    prim_names: Vec<String>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            slots: Vec::new(),
            marks: Vec::new(),
            free_head: None,
            used: 0,
            threshold: 4096,
            min_heap: 4096,
            heap_cap: None,
            stress: false,
            env_pool: Vec::new(),
            collections: 0,
            handles: Vec::new(),
            locs: HashMap::new(),
            aliases: HashMap::new(),
            interner: Interner::default(),
            prim_names: Vec::new(),
        }
    }

    // ---- allocation -----------------------------------------------------

    pub fn alloc(&mut self, obj: Obj) -> Value {
        self.used += 1;
        match self.free_head {
            Some(i) => {
                let next = match &self.slots[i as usize] {
                    Slot::Free { next } => *next,
                    Slot::Used(_) => unreachable!("the free list points at a used slot"),
                };
                self.free_head = next;
                self.slots[i as usize] = Slot::Used(obj);
                Value::Obj(ObjRef(i))
            }
            None => {
                let i = self.slots.len() as u32;
                self.slots.push(Slot::Used(obj));
                self.marks.push(false);
                Value::Obj(ObjRef(i))
            }
        }
    }

    pub fn obj(&self, r: ObjRef) -> &Obj {
        match &self.slots[r.0 as usize] {
            Slot::Used(o) => o,
            Slot::Free { .. } => panic!("dangling ObjRef {}: that slot is free", r.0),
        }
    }

    pub fn obj_mut(&mut self, r: ObjRef) -> &mut Obj {
        match &mut self.slots[r.0 as usize] {
            Slot::Used(o) => o,
            Slot::Free { .. } => panic!("dangling ObjRef {}: that slot is free", r.0),
        }
    }

    /// Live objects.
    pub fn obj_count(&self) -> usize {
        self.used
    }

    /// Slots ever allocated, live or free: the heap's high-water mark.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    // ---- the collector --------------------------------------------------

    pub fn set_heap_cap(&mut self, cap: Option<usize>) {
        self.heap_cap = cap;
    }

    pub fn set_gc_stress(&mut self, on: bool) {
        self.stress = on;
    }

    /// Set the floor, and take effect immediately. A caller asking for a
    /// smaller heap means *now*, so this lowers the live threshold as well as
    /// the floor -- raising it only would make `set_min_heap(1024)` on a fresh
    /// engine do nothing at all, since the threshold already starts at 4096.
    pub fn set_min_heap(&mut self, n: usize) {
        self.min_heap = n;
        self.threshold = n;
    }

    pub fn should_collect(&self) -> bool {
        self.stress || self.used >= self.threshold
    }

    pub fn over_heap_cap(&self) -> bool {
        self.heap_cap.is_some_and(|cap| self.used > cap)
    }

    pub fn collections(&self) -> u64 {
        self.collections
    }

    /// Mark from `roots`, then sweep.
    ///
    /// The mark phase uses an explicit worklist. A recursive marker would
    /// overflow the C stack on a long list -- the same trap the interpreter
    /// avoids by not recursing (§5), with the same requirement-8 consequence:
    /// the host must survive whatever the script allocated.
    pub fn collect(&mut self, roots: &[Value]) -> GcStats {
        self.marks.clear();
        self.marks.resize(self.slots.len(), false);

        // Only a heap reference can mark anything, so nothing else goes on
        // the worklist: a million-element vector of fixnums used to cost a
        // million pushes and pops per collection. `visited` counts what
        // the mark phase looked at either way, and sets how long the next
        // collection waits -- marking work is amortised over at least as
        // many allocations, so a large live vector is not re-marked every
        // few thousand frames.
        let is_obj = |v: &Value| matches!(v, Value::Obj(_));
        let mut visited: usize = roots.len();
        let mut work: Vec<Value> = Vec::with_capacity(roots.len() + self.handles.len());
        work.extend(roots.iter().copied().filter(is_obj));
        work.extend(self.handles.iter().flatten().copied());
        // A local macro's definition frame has to outlive its expansion, so
        // the environments recorded beside the aliases are roots.
        work.extend(
            self.aliases
                .values()
                .filter_map(|&(_, env)| env.map(Value::Obj)),
        );

        while let Some(v) = work.pop() {
            let Value::Obj(r) = v else { continue };
            let i = r.0 as usize;
            if self.marks[i] {
                continue;
            }
            self.marks[i] = true;
            match &self.slots[i] {
                // A root pointing at a free slot is a rooting bug, and it is
                // far better to hear about it here than to watch a reused slot
                // be mistaken for the object that used to be there.
                Slot::Free { .. } => panic!("gc: a root points at free slot {i}"),
                Slot::Used(obj) => match obj {
                    Obj::Pair(a, d) => {
                        visited += 2;
                        if is_obj(a) {
                            work.push(*a);
                        }
                        if is_obj(d) {
                            work.push(*d);
                        }
                    }
                    Obj::Vector { items, .. } => {
                        visited += items.len();
                        work.extend(items.iter().copied().filter(is_obj));
                    }
                    Obj::Env { parent, slots } => {
                        visited += slots.len();
                        if let Some(p) = parent {
                            work.push(Value::Obj(*p));
                        }
                        work.extend(slots.iter().map(|(_, v)| *v).filter(is_obj));
                    }
                    Obj::Closure {
                        body,
                        env,
                        opt,
                        named,
                        code,
                        ..
                    } => {
                        // The body is *source*, and source is heap data too --
                        // and so is every default expression, which is easy to
                        // forget because it only runs when an argument is
                        // missing. And so are the constants in compiled code.
                        work.extend(body.iter().copied());
                        work.extend(opt.iter().filter_map(|p| p.default));
                        work.extend(named.iter().filter_map(|p| p.default));
                        if let crate::code::CodeState::Compiled(c) = code {
                            c.trace(&mut work);
                        }
                        if let Some(p) = env {
                            work.push(Value::Obj(*p));
                        }
                    }
                    Obj::Parameter { converter, .. } => work.extend(converter.iter().copied()),
                    Obj::ErrorObj {
                        message, irritants, ..
                    } => {
                        work.push(*message);
                        work.push(*irritants);
                    }
                    Obj::Port { .. } => {}
                    Obj::Values(vs) => work.extend(vs.iter().copied()),
                    Obj::Array { store, .. } => {
                        visited += 1;
                        if is_obj(store) {
                            work.push(*store);
                        }
                    }
                    Obj::Record { rtype, fields } => {
                        visited += fields.len() + 1;
                        if is_obj(rtype) {
                            work.push(*rtype);
                        }
                        work.extend(fields.iter().copied().filter(is_obj));
                    }
                    // A type descriptor holds interned names, and a
                    // numeric vector holds unboxed numbers: neither has
                    // anything collectable inside.
                    Obj::RecordType { .. } | Obj::NumVector(_) => {}
                    // A macro's patterns and templates are heap data, so
                    // they must be traced -- this is the `macro_roots` bug
                    // again, and gc stress is what caught it the first time.
                    Obj::Macro(m) => m.trace(&mut work),
                    // A continuation holds an id, not a value, so it marks
                    // nothing.
                    Obj::Str(_)
                    | Obj::Bytevector(_)
                    | Obj::Bignum(_)
                    | Obj::Rng(_)
                    | Obj::Continuation { .. } => {}
                },
            }
        }

        // Sweep. Overwriting a slot drops its `Obj`, which runs the `Drop` of
        // anything Rust-owned inside -- so a foreign object's resources are
        // released without a separate finalizer mechanism. A *Scheme-visible*
        // finalizer is absent because nothing needs one yet.
        let mut freed = 0;
        for i in 0..self.slots.len() {
            if self.marks[i] || matches!(self.slots[i], Slot::Free { .. }) {
                continue;
            }
            // A dead frame's vector goes back to the pool, bounded so a
            // one-off deep recursion does not pin its memory forever.
            if self.env_pool.len() < 4096
                && let Slot::Used(Obj::Env { slots, .. }) = &mut self.slots[i]
            {
                let mut v = std::mem::take(slots);
                v.clear();
                self.env_pool.push(v);
            }
            self.slots[i] = Slot::Free {
                next: self.free_head,
            };
            self.free_head = Some(i as u32);
            freed += 1;
        }
        // A freed slot's location must go with it: the slot will be reused,
        // and reporting where its previous occupant was written is worse than
        // reporting nothing. Cheap because the table is program-sized.
        if freed > 0 {
            let marks = &self.marks;
            self.locs
                .retain(|r, _| marks.get(r.0 as usize).copied().unwrap_or(false));
        }
        self.used -= freed;
        self.collections += 1;
        self.threshold = self.min_heap.max(self.used.saturating_mul(2)).max(visited);
        GcStats {
            live: self.used,
            freed,
            collections: self.collections,
        }
    }

    // ---- source locations -----------------------------------------------

    /// Remember where a form was written. Only compound values get an entry.
    pub fn record_loc(&mut self, v: Value, loc: Loc) {
        if let Value::Obj(r) = v {
            self.locs.insert(r, loc);
        }
    }

    pub fn loc_of(&self, v: Value) -> Option<&Loc> {
        match v {
            Value::Obj(r) => self.locs.get(&r),
            _ => None,
        }
    }

    pub fn loc_count(&self) -> usize {
        self.locs.len()
    }

    // ---- handles --------------------------------------------------------

    /// Root a value. It stays live until [`Engine::unroot`]. This is what the
    /// embedding API (S7) gets built out of, and what the VM uses for source
    /// forms it holds across an evaluation.
    pub fn root(&mut self, v: Value) -> Handle {
        if let Some(i) = self.handles.iter().position(|h| h.is_none()) {
            self.handles[i] = Some(v);
            return Handle(i);
        }
        self.handles.push(Some(v));
        Handle(self.handles.len() - 1)
    }

    pub fn unroot(&mut self, h: Handle) {
        self.handles[h.0] = None;
    }

    pub fn handle_value(&self, h: Handle) -> Option<Value> {
        self.handles[h.0]
    }

    pub fn rooted_count(&self) -> usize {
        self.handles.iter().flatten().count()
    }

    // ---- macro renaming -------------------------------------------------

    pub fn set_alias(&mut self, fresh: SymId, original: SymId, def_env: Option<ObjRef>) {
        self.aliases.insert(fresh, (original, def_env));
    }

    /// Where a renamed identifier's name meant what it meant.
    pub fn alias_env(&self, sym: SymId) -> Option<Option<ObjRef>> {
        self.aliases.get(&sym).map(|&(_, env)| env)
    }

    /// Follow a renamed identifier back to what it meant at the macro's
    /// definition site. Idempotent for ordinary symbols.
    pub fn resolve(&self, mut sym: SymId) -> SymId {
        // A chain can form when a macro expands into another macro's output.
        for _ in 0..64 {
            match self.aliases.get(&sym) {
                Some(&(next, _)) if next != sym => sym = next,
                _ => return sym,
            }
        }
        sym
    }

    // ---- primitives -----------------------------------------------------

    pub fn register_prim_name(&mut self, name: &str) -> u32 {
        self.prim_names.push(name.to_string());
        (self.prim_names.len() - 1) as u32
    }

    pub fn prim_name(&self, id: crate::value::PrimId) -> &str {
        &self.prim_names[id.0 as usize]
    }

    // ---- constructors ---------------------------------------------------

    pub fn cons(&mut self, car: Value, cdr: Value) -> Value {
        self.alloc(Obj::Pair(car, cdr))
    }

    pub fn string(&mut self, s: &str) -> Value {
        self.alloc(Obj::Str(s.chars().collect()))
    }

    pub fn vector(&mut self, items: Vec<Value>, bracket: bool) -> Value {
        self.alloc(Obj::Vector { items, bracket })
    }

    pub fn error_obj(&mut self, message: Value, irritants: Value, kind: ErrorKind) -> Value {
        self.alloc(Obj::ErrorObj {
            message,
            irritants,
            kind,
        })
    }

    pub fn numvector(&mut self, v: NumVec) -> Value {
        self.alloc(Obj::NumVector(v))
    }

    /// An array view over `store`, row-major with no offset.
    pub fn array(&mut self, store: Value, dims: Vec<usize>) -> Value {
        let strides = row_major_strides(&dims);
        self.alloc(Obj::Array {
            dims: Rc::new(dims),
            strides: Rc::new(strides),
            offset: 0,
            store,
        })
    }

    /// An array view with strides and an offset given: `make-shared-array`.
    pub fn array_view(
        &mut self,
        store: Value,
        dims: Vec<usize>,
        strides: Vec<isize>,
        offset: usize,
    ) -> Value {
        self.alloc(Obj::Array {
            dims: Rc::new(dims),
            strides: Rc::new(strides),
            offset,
            store,
        })
    }

    /// Element `k` of the array at `r`, counting in row-major order over
    /// its *logical* shape -- so it is the same element on both sides of a
    /// comparison however either one is strided.
    pub fn array_element(&self, r: ObjRef, k: usize) -> Option<Value> {
        let Obj::Array { dims, strides, offset, store } = self.obj(r) else {
            return None;
        };
        let at = flat_index(dims, strides, *offset, k)?;
        self.store_ref(*store, at)
    }

    /// Element `i` of whatever backs an array.
    pub fn store_ref(&self, store: Value, i: usize) -> Option<Value> {
        match store {
            Value::Obj(r) => match self.obj(r) {
                Obj::NumVector(nv) => nv.get(i),
                Obj::Bytevector(b) => b.get(i).map(|x| Value::Fixnum(*x as i64)),
                Obj::Vector { items, .. } => items.get(i).copied(),
                Obj::Str(cs) => cs.get(i).copied().map(Value::Char),
                _ => None,
            },
            _ => None,
        }
    }

    /// How many elements the store holds.
    pub fn store_len(&self, store: Value) -> Option<usize> {
        match store {
            Value::Obj(r) => match self.obj(r) {
                Obj::NumVector(nv) => Some(nv.len()),
                Obj::Bytevector(b) => Some(b.len()),
                Obj::Vector { items, .. } => Some(items.len()),
                Obj::Str(cs) => Some(cs.len()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Store `x` at `i` in whatever backs an array.
    pub fn store_set(&mut self, store: Value, i: usize, x: Value) -> Result<(), String> {
        let Value::Obj(r) = store else {
            return Err("not an array store".into());
        };
        match self.obj_mut(r) {
            Obj::NumVector(nv) => nv.set(i, x),
            Obj::Bytevector(b) => match x {
                Value::Fixnum(n) if (0..=255).contains(&n) => {
                    if i >= b.len() {
                        return Err(format!("index {i} is out of range"));
                    }
                    b[i] = n as u8;
                    Ok(())
                }
                _ => Err("a bytevector holds octets".into()),
            },
            Obj::Vector { items, .. } => {
                if i >= items.len() {
                    return Err(format!("index {i} is out of range"));
                }
                items[i] = x;
                Ok(())
            }
            Obj::Str(cs) => match x {
                Value::Char(c) => {
                    if i >= cs.len() {
                        return Err(format!("index {i} is out of range"));
                    }
                    cs[i] = c;
                    Ok(())
                }
                _ => Err("a string holds characters".into()),
            },
            _ => Err("not an array store".into()),
        }
    }

    pub fn record_type(&mut self, name: SymId, fields: Vec<SymId>) -> Value {
        self.alloc(Obj::RecordType { name, fields: Rc::new(fields) })
    }

    pub fn record(&mut self, rtype: Value, fields: Vec<Value>) -> Value {
        self.alloc(Obj::Record { rtype, fields })
    }

    pub fn port(&mut self, kind: PortKind) -> Value {
        self.alloc(Obj::Port { kind, open: true })
    }

    pub fn bytevector(&mut self, bytes: Vec<u8>) -> Value {
        self.alloc(Obj::Bytevector(bytes))
    }

    /// An exact integer from a `BigInt`, **normalised**: a value that fits
    /// comes back a `Fixnum` and only what does not is allocated. Both the
    /// reader and the arithmetic go through this, so there is one rule for
    /// where the fixnum/bignum boundary is rather than two that can drift.
    pub fn exact_integer(&mut self, b: crate::bigint::BigInt) -> Value {
        match b.to_i64() {
            Some(n) => Value::Fixnum(n),
            None => self.alloc(Obj::Bignum(b)),
        }
    }

    /// An empty slot vector with room for `cap`, from the pool when it has
    /// one. Hand it to [`Engine::env`] filled.
    pub fn slots(&mut self, cap: usize) -> Vec<(SymId, Value)> {
        match self.env_pool.pop() {
            Some(mut v) => {
                v.reserve(cap);
                v
            }
            None => Vec::with_capacity(cap),
        }
    }

    pub fn env(&mut self, parent: Option<ObjRef>, slots: Vec<(SymId, Value)>) -> ObjRef {
        match self.alloc(Obj::Env { parent, slots }) {
            Value::Obj(r) => r,
            _ => unreachable!("alloc returns Value::Obj"),
        }
    }

    pub fn sym(&mut self, name: &str) -> Value {
        Value::Symbol(self.interner.symbol(name))
    }

    pub fn kw(&mut self, name: &str) -> Value {
        Value::Keyword(self.interner.keyword(name))
    }

    /// Build a proper list.
    pub fn list(&mut self, items: &[Value]) -> Value {
        let mut acc = Value::Null;
        for &item in items.iter().rev() {
            acc = self.cons(item, acc);
        }
        acc
    }

    /// `equal?`, which is what the round-trip gate is phrased in terms of.
    ///
    /// Deliberately ignores how a vector was written: `[1 2 3]` and `#(1 2 3)`
    /// contain the same elements and so are `equal?`, even though they do not
    /// evaluate the same way.
    ///
    /// R7RS: "`equal?` must always terminate even if its arguments contain
    /// cycles." The first version walked structure naively, and the R7RS
    /// `equal` benchmark found both of its failure modes in one afternoon:
    /// it looped on a cycle, and on a tree with sharing it walked the
    /// *unshared* size -- 100! for the published input. This is Adams and
    /// Dybvig's two-phase scheme: a bounded plain walk, which is all ordinary
    /// data ever needs, and when that runs out of budget a second walk that
    /// unions each pair of containers it compares, so a pair met twice --
    /// through sharing or a cycle -- is assumed equal the second time. That
    /// assumption is the coinductive reading of `equal?` on graphs, and it
    /// is the one the standard means.
    pub fn equal(&self, a: Value, b: Value) -> bool {
        match self.equal_walk(a, b, Some(4096)) {
            Some(r) => r,
            None => self.equal_walk(a, b, None).expect("the unbounded walk decides"),
        }
    }

    /// `budget: Some(n)` is the plain walk, giving up with `None` after `n`
    /// container comparisons. `None` is the union-find walk, which always
    /// decides.
    fn equal_walk(&self, a: Value, b: Value, budget: Option<usize>) -> Option<bool> {
        let mut work = vec![(a, b)];
        let mut left = budget;
        let mut parent: HashMap<u32, u32> = HashMap::new();
        fn find(parent: &mut HashMap<u32, u32>, mut x: u32) -> u32 {
            while let Some(&p) = parent.get(&x) {
                if p == x {
                    break;
                }
                // Halve the path on the way up.
                if let Some(&gp) = parent.get(&p) {
                    parent.insert(x, gp);
                }
                x = p;
            }
            x
        }
        while let Some((a, b)) = work.pop() {
            match (a, b) {
                (Value::Obj(ra), Value::Obj(rb)) => {
                    if ra == rb {
                        continue;
                    }
                    match (self.obj(ra), self.obj(rb)) {
                        (Obj::Pair(..), Obj::Pair(..)) | (Obj::Vector { .. }, Obj::Vector { .. }) => {
                            match left.as_mut() {
                                Some(0) => return None,
                                Some(n) => *n -= 1,
                                None => {
                                    let (xa, xb) = (find(&mut parent, ra.0), find(&mut parent, rb.0));
                                    if xa == xb {
                                        continue;
                                    }
                                    parent.insert(xa, xb);
                                }
                            }
                            match (self.obj(ra), self.obj(rb)) {
                                (Obj::Pair(a1, d1), Obj::Pair(a2, d2)) => {
                                    work.push((*d1, *d2));
                                    work.push((*a1, *a2));
                                }
                                (Obj::Vector { items: i1, .. }, Obj::Vector { items: i2, .. }) => {
                                    if i1.len() != i2.len() {
                                        return Some(false);
                                    }
                                    work.extend(i1.iter().copied().zip(i2.iter().copied()));
                                }
                                _ => unreachable!("matched above"),
                            }
                        }
                        (Obj::Str(s1), Obj::Str(s2)) => {
                            if s1 != s2 {
                                return Some(false);
                            }
                        }
                        (Obj::Bytevector(b1), Obj::Bytevector(b2)) => {
                            if b1 != b2 {
                                return Some(false);
                            }
                        }
                        // Two bignums are `eqv?` when they are the same
                        // integer, which is what `=` says too: both are
                        // exact, so there is no exactness to differ on.
                        (Obj::Bignum(a), Obj::Bignum(b)) => {
                            if a != b {
                                return Some(false);
                            }
                        }
                        // Same element type and same elements, as for a
                        // bytevector: these are aggregates of numbers and
                        // `equal?` recurses into aggregates.
                        (Obj::NumVector(a), Obj::NumVector(b)) => {
                            if a != b {
                                return Some(false);
                            }
                        }
                        // SRFI 63 extends `equal?` to arrays: same shape
                        // and `equal?` elements, whatever each one's store
                        // or strides. Two views of one store compare equal
                        // if they see the same elements, which is the
                        // point of comparing a transpose with a copy.
                        (
                            Obj::Array { dims: d1, .. },
                            Obj::Array { dims: d2, .. },
                        ) => {
                            if d1 != d2 {
                                return Some(false);
                            }
                            let n: usize = d1.iter().product();
                            for k in 0..n {
                                match (self.array_element(ra, k), self.array_element(rb, k)) {
                                    (Some(x), Some(y)) => work.push((x, y)),
                                    _ => return Some(false),
                                }
                            }
                        }
                        // Environments and closures compare by identity, and
                        // `ra == rb` above already settled that.
                        _ => return Some(false),
                    }
                }
                // Flonums compare by value, so NaN is not `equal?` to itself.
                _ => {
                    if a != b {
                        return Some(false);
                    }
                }
            }
        }
        Some(true)
    }
}


/// Row-major strides for `dims`: the last dimension is contiguous.
pub fn row_major_strides(dims: &[usize]) -> Vec<isize> {
    let mut strides = vec![0isize; dims.len()];
    let mut acc: isize = 1;
    for i in (0..dims.len()).rev() {
        strides[i] = acc;
        acc *= dims[i] as isize;
    }
    strides
}

/// Where the `k`-th element in row-major order over `dims` lands in the
/// store, given `strides` and `offset`. `None` if it would fall outside.
pub fn flat_index(dims: &[usize], strides: &[isize], offset: usize, k: usize) -> Option<usize> {
    let mut rest = k;
    let mut at = offset as isize;
    for i in (0..dims.len()).rev() {
        let d = dims[i];
        if d == 0 {
            return None;
        }
        let idx = rest % d;
        rest /= d;
        at += strides[i] * idx as isize;
    }
    if rest != 0 || at < 0 {
        return None;
    }
    Some(at as usize)
}
