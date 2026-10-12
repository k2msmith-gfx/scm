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

/// A reference to a heap object.
///
/// An index rather than a pointer. The plan says "behind a pointer"; an index
/// is the same thing for every purpose that matters here -- it stays valid
/// across heap growth, so the heap is still non-moving as far as any value can
/// tell -- and it costs no `unsafe`, which requirement 8 cares about more than
/// it cares about one indirection.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ObjRef(pub u32);

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
    /// The empty list. Not "nil": there is no nil in this language, and an
    /// absent argument binds `#f` (§4).
    Null,
    Unspecified,
    Eof,
    Obj(ObjRef),
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
    fn only_false_is_false() {
        assert!(!Value::Bool(false).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(Value::Null.is_truthy());
        assert!(Value::Fixnum(0).is_truthy());
        assert!(Value::Unspecified.is_truthy());
    }
}
