//! Heap objects, and the interner.
//!
//! There is no collector yet -- that is S3. What matters now is that the shape
//! is the one a collector can be dropped into: every object lives in one place
//! the sweep can walk, and nothing outside holds a raw pointer to it.

use std::collections::HashMap;

use crate::value::{KwId, ObjRef, SymId, Value};

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

/// Everything a running Scheme owns. It will grow a great deal; for S1 it is
/// the heap and the interner.
#[derive(Default, Debug)]
pub struct Engine {
    objs: Vec<Obj>,
    pub interner: Interner,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self, obj: Obj) -> Value {
        let r = ObjRef(self.objs.len() as u32);
        self.objs.push(obj);
        Value::Obj(r)
    }

    pub fn obj(&self, r: ObjRef) -> &Obj {
        &self.objs[r.0 as usize]
    }

    pub fn obj_mut(&mut self, r: ObjRef) -> &mut Obj {
        &mut self.objs[r.0 as usize]
    }

    pub fn obj_count(&self) -> usize {
        self.objs.len()
    }

    // ---- constructors used by the reader and by tests -------------------

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
    pub fn equal(&self, a: Value, b: Value) -> bool {
        match (a, b) {
            (Value::Obj(ra), Value::Obj(rb)) => match (self.obj(ra), self.obj(rb)) {
                (Obj::Pair(a1, d1), Obj::Pair(a2, d2)) => {
                    self.equal(*a1, *a2) && self.equal(*d1, *d2)
                }
                (Obj::Str(s1), Obj::Str(s2)) => s1 == s2,
                (Obj::Vector { items: i1, .. }, Obj::Vector { items: i2, .. }) => {
                    i1.len() == i2.len()
                        && i1.iter().zip(i2.iter()).all(|(&x, &y)| self.equal(x, y))
                }
                (Obj::Bytevector(b1), Obj::Bytevector(b2)) => b1 == b2,
                _ => false,
            },
            // Flonums compare by value, so NaN is not `equal?` to itself.
            // That matches `eqv?` on inexact numbers closely enough for S1;
            // revisit when `equal?` gets its real specification at S9.
            _ => a == b,
        }
    }
}
