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
    /// A lexical frame. The chain of these is the environment; the chain of
    /// `Frame`s in the VM is the *control* stack. Keeping them separate is
    /// what lets a closure outlive the call that made it.
    Env {
        parent: Option<ObjRef>,
        slots: Vec<(SymId, Value)>,
    },
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
    collections: u64,
    /// Values rooted from outside the VM -- the host's, and the VM's own
    /// source forms while it holds them across an evaluation.
    handles: Vec<Option<Value>>,
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
            collections: 0,
            handles: Vec::new(),
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

        let mut work: Vec<Value> = Vec::with_capacity(roots.len() + self.handles.len());
        work.extend_from_slice(roots);
        work.extend(self.handles.iter().flatten().copied());

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
                        work.push(*a);
                        work.push(*d);
                    }
                    Obj::Vector { items, .. } => work.extend(items.iter().copied()),
                    Obj::Env { parent, slots } => {
                        if let Some(p) = parent {
                            work.push(Value::Obj(*p));
                        }
                        work.extend(slots.iter().map(|(_, v)| *v));
                    }
                    Obj::Closure {
                        body,
                        env,
                        opt,
                        named,
                        ..
                    } => {
                        // The body is *source*, and source is heap data too --
                        // and so is every default expression, which is easy to
                        // forget because it only runs when an argument is
                        // missing.
                        work.extend(body.iter().copied());
                        work.extend(opt.iter().filter_map(|p| p.default));
                        work.extend(named.iter().filter_map(|p| p.default));
                        if let Some(p) = env {
                            work.push(Value::Obj(*p));
                        }
                    }
                    Obj::Str(_) | Obj::Bytevector(_) => {}
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
            self.slots[i] = Slot::Free {
                next: self.free_head,
            };
            self.free_head = Some(i as u32);
            freed += 1;
        }
        self.used -= freed;
        self.collections += 1;
        self.threshold = self.min_heap.max(self.used.saturating_mul(2));
        GcStats {
            live: self.used,
            freed,
            collections: self.collections,
        }
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

    pub fn bytevector(&mut self, bytes: Vec<u8>) -> Value {
        self.alloc(Obj::Bytevector(bytes))
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
    /// Iterative, with its own worklist. A 200k-element list is ordinary, and
    /// recursing down one would overflow the C stack.
    pub fn equal(&self, a: Value, b: Value) -> bool {
        let mut work = vec![(a, b)];
        while let Some((a, b)) = work.pop() {
            match (a, b) {
                (Value::Obj(ra), Value::Obj(rb)) => {
                    if ra == rb {
                        continue;
                    }
                    match (self.obj(ra), self.obj(rb)) {
                        (Obj::Pair(a1, d1), Obj::Pair(a2, d2)) => {
                            work.push((*d1, *d2));
                            work.push((*a1, *a2));
                        }
                        (Obj::Str(s1), Obj::Str(s2)) => {
                            if s1 != s2 {
                                return false;
                            }
                        }
                        (Obj::Vector { items: i1, .. }, Obj::Vector { items: i2, .. }) => {
                            if i1.len() != i2.len() {
                                return false;
                            }
                            work.extend(i1.iter().copied().zip(i2.iter().copied()));
                        }
                        (Obj::Bytevector(b1), Obj::Bytevector(b2)) => {
                            if b1 != b2 {
                                return false;
                            }
                        }
                        // Environments and closures compare by identity, and
                        // `ra == rb` above already settled that.
                        _ => return false,
                    }
                }
                // Flonums compare by value, so NaN is not `equal?` to itself.
                // Close enough to `eqv?` on inexact numbers for now; `equal?`
                // gets its real specification at S9.
                _ => {
                    if a != b {
                        return false;
                    }
                }
            }
        }
        true
    }
}
