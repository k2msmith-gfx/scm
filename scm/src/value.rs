//! The value representation settled in plan §4.5.
//!
//! A `Value` is 16 bytes, `Copy`, and has no `Drop`: the heap owns every
//! object, so a value is a tag and a payload and nothing else.

/// An interned symbol.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SymId(pub u32);

/// An interned keyword. Distinct from `SymId` by type, not just by tag --
/// `(eq? :at 'at)` is `#f` and the type system should say so too.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct KwId(pub u32);

/// An index into the engine's primitive table. Primitives are identified by
/// index rather than by function pointer so an error message can name the one
/// that refused.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PrimId(pub u32);

/// A reference to a heap object.
///
/// An index rather than a pointer. The plan says "behind a pointer"; an index
/// is the same thing for every purpose that matters here -- it stays valid
/// across heap growth, so the heap is still non-moving as far as any value can
/// tell -- and it costs no `unsafe`, which requirement 8 cares about more than
/// it cares about one indirection.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ObjRef(pub u32);

/// `repr(u8)` gives the layout a definition native code can rely on (S13):
/// the tag is the first byte, in declaration order from 0, and each
/// variant's payload sits at its natural alignment after it -- an `i64` or
/// `f64` at offset 8, a `u32` at 4, a `bool` at 1. `layout_is_defined`
/// below pins it.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value {
    /// An exact integer. No bignums: `ratios` and `exact-closed` are not
    /// claimed (plan §2.3), and overflow is an error rather than a silent
    /// widening (§4.5).
    Fixnum(i64),
    Flonum(f64),
    Bool(bool),
    Char(char),
    Symbol(SymId),
    Keyword(KwId),
    Primitive(PrimId),
    /// The empty list. Not "nil": there is no nil in this language, and an
    /// absent argument binds `#f` (§4).
    Null,
    Unspecified,
    Eof,
    Obj(ObjRef),
    /// The slot of an internal `define` before its definition has run. No
    /// expression produces it: a read finds it and raises "unbound
    /// variable", which is what T0, whose slot does not exist yet, says.
    /// Last, so every other tag keeps its number (S13's native code reads
    /// them).
    Undefined,
}

impl Value {
    pub fn is_truthy(self) -> bool {
        // Scheme has exactly one false value.
        !matches!(self, Value::Bool(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_is_sixteen_bytes() {
        // §4.5 says 16 bytes. If this ever fails, the plan is wrong or a
        // variant grew a payload it should not have.
        assert_eq!(std::mem::size_of::<Value>(), 16);
    }

    #[test]
    fn layout_is_defined() {
        fn bytes(v: Value) -> [u8; 16] {
            unsafe { std::mem::transmute(v) }
        }
        let b = bytes(Value::Fixnum(-2));
        assert_eq!(b[0], 0);
        assert_eq!(i64::from_ne_bytes(b[8..16].try_into().unwrap()), -2);
        let b = bytes(Value::Flonum(1.5));
        assert_eq!(b[0], 1);
        assert_eq!(f64::from_ne_bytes(b[8..16].try_into().unwrap()), 1.5);
        let b = bytes(Value::Bool(true));
        assert_eq!((b[0], b[1]), (2, 1));
        let b = bytes(Value::Char('x'));
        assert_eq!((b[0], u32::from_ne_bytes(b[4..8].try_into().unwrap())), (3, 'x' as u32));
        let b = bytes(Value::Symbol(SymId(7)));
        assert_eq!((b[0], u32::from_ne_bytes(b[4..8].try_into().unwrap())), (4, 7));
        assert_eq!(bytes(Value::Keyword(KwId(1)))[0], 5);
        assert_eq!(bytes(Value::Primitive(PrimId(9)))[0], 6);
        assert_eq!(bytes(Value::Null)[0], 7);
        assert_eq!(bytes(Value::Unspecified)[0], 8);
        assert_eq!(bytes(Value::Eof)[0], 9);
        let b = bytes(Value::Obj(ObjRef(3)));
        assert_eq!((b[0], u32::from_ne_bytes(b[4..8].try_into().unwrap())), (10, 3));
        assert_eq!(bytes(Value::Undefined)[0], 11);
    }

    #[test]
    fn only_false_is_false() {
        assert!(!Value::Bool(false).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(Value::Null.is_truthy());
        assert!(Value::Fixnum(0).is_truthy());
        assert!(Value::Unspecified.is_truthy());
    }
}
