//! Arbitrary-precision exact integers.
//!
//! Written here rather than taken from a crate because the `scm` crate has
//! **no dependencies** and that is worth keeping: an embedder takes this
//! library and nothing else. The scope is accordingly small -- what a
//! Scheme's exact integers need and nothing more. There is no modular
//! exponentiation, no primality testing and no subquadratic multiplication;
//! if a profile ever asks for Karatsuba it can be added behind the same
//! API.
//!
//! **Limbs are `u32`, little-endian, with no trailing zeros.** The `u32`
//! choice is deliberate: every intermediate product of two limbs fits in a
//! `u64`, so the whole implementation stays in safe arithmetic that the
//! compiler checks, where `u64` limbs would need `u128` everywhere and
//! would make the division loop harder to read and to trust.
//!
//! Zero is the empty magnitude and is never negative, which makes `neg`
//! a function of the representation rather than a flag to keep in sync.
//!
//! **A `BigInt` here is always out of `i64` range.** Normalising a result
//! back to `Value::Fixnum` whenever it fits is the caller's job (`prim.rs`),
//! and it is not optional: if a small value could exist in both
//! representations then `eqv?`, `equal-hash` and every `match` on `Value`
//! would disagree with themselves. `to_i64` is what that check uses.

use std::cmp::Ordering;

/// Ordered by value, as the `Ord` trait rather than an inherent `cmp`
/// that would shadow the standard method at every call site.
impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.neg, other.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => Self::cmp_mag(&self.mag, &other.mag),
            (true, true) => Self::cmp_mag(&other.mag, &self.mag),
        }
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Ord::cmp(self, other))
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BigInt {
    /// Always `false` when `mag` is empty, so zero has one representation.
    neg: bool,
    /// Little-endian base-2^32 limbs, most significant last, no trailing
    /// zeros.
    mag: Vec<u32>,
}

const BASE: u64 = 1 << 32;
const MASK: u64 = 0xffff_ffff;

fn strip(mag: &mut Vec<u32>) {
    while mag.last() == Some(&0) {
        mag.pop();
    }
}

impl BigInt {
    pub fn zero() -> Self {
        BigInt { neg: false, mag: Vec::new() }
    }

    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub fn is_negative(&self) -> bool {
        self.neg
    }

    fn make(neg: bool, mut mag: Vec<u32>) -> Self {
        strip(&mut mag);
        BigInt { neg: neg && !mag.is_empty(), mag }
    }

    pub fn from_i64(n: i64) -> Self {
        // `i64::MIN` has no positive counterpart, so go through u128 of the
        // unsigned magnitude rather than negating in i64.
        let neg = n < 0;
        let m = n.unsigned_abs();
        Self::from_u64_mag(neg, m)
    }

    pub fn from_u64_mag(neg: bool, m: u64) -> Self {
        let mut mag = vec![(m & MASK) as u32, (m >> 32) as u32];
        strip(&mut mag);
        BigInt { neg: neg && !mag.is_empty(), mag }
    }

    /// `Some(n)` when this fits an `i64`. The one place the fixnum/bignum
    /// boundary is decided.
    pub fn to_i64(&self) -> Option<i64> {
        if self.mag.len() > 2 {
            return None;
        }
        let mut m: u64 = 0;
        for (i, &l) in self.mag.iter().enumerate() {
            m |= (l as u64) << (32 * i);
        }
        if self.neg {
            // `-(2^63)` is representable, `2^63` is not.
            if m > (i64::MAX as u64) + 1 {
                None
            } else if m == (i64::MAX as u64) + 1 {
                Some(i64::MIN)
            } else {
                Some(-(m as i64))
            }
        } else if m > i64::MAX as u64 {
            None
        } else {
            Some(m as i64)
        }
    }

    pub fn neg(&self) -> Self {
        Self::make(!self.neg, self.mag.clone())
    }

    pub fn abs(&self) -> Self {
        Self::make(false, self.mag.clone())
    }

    fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
        if a.len() != b.len() {
            return a.len().cmp(&b.len());
        }
        for i in (0..a.len()).rev() {
            if a[i] != b[i] {
                return a[i].cmp(&b[i]);
            }
        }
        Ordering::Equal
    }

    fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
        let mut carry = 0u64;
        for i in 0..a.len().max(b.len()) {
            let x = *a.get(i).unwrap_or(&0) as u64;
            let y = *b.get(i).unwrap_or(&0) as u64;
            let t = x + y + carry;
            out.push((t & MASK) as u32);
            carry = t >> 32;
        }
        if carry != 0 {
            out.push(carry as u32);
        }
        out
    }

    /// `a - b`, requiring `a >= b` by magnitude.
    fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(a.len());
        let mut borrow = 0i64;
        for (i, &limb) in a.iter().enumerate() {
            let x = limb as i64;
            let y = *b.get(i).unwrap_or(&0) as i64;
            let mut t = x - y - borrow;
            if t < 0 {
                t += BASE as i64;
                borrow = 1;
            } else {
                borrow = 0;
            }
            out.push(t as u32);
        }
        debug_assert_eq!(borrow, 0, "sub_mag called with a < b");
        strip(&mut out);
        out
    }

    pub fn add(&self, other: &Self) -> Self {
        if self.neg == other.neg {
            Self::make(self.neg, Self::add_mag(&self.mag, &other.mag))
        } else {
            match Self::cmp_mag(&self.mag, &other.mag) {
                Ordering::Equal => Self::zero(),
                Ordering::Greater => {
                    Self::make(self.neg, Self::sub_mag(&self.mag, &other.mag))
                }
                Ordering::Less => {
                    Self::make(other.neg, Self::sub_mag(&other.mag, &self.mag))
                }
            }
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut out = vec![0u32; self.mag.len() + other.mag.len()];
        for (i, &x) in self.mag.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &y) in other.mag.iter().enumerate() {
                let t = (x as u64) * (y as u64) + out[i + j] as u64 + carry;
                out[i + j] = (t & MASK) as u32;
                carry = t >> 32;
            }
            let mut k = i + other.mag.len();
            while carry != 0 {
                let t = out[k] as u64 + carry;
                out[k] = (t & MASK) as u32;
                carry = t >> 32;
                k += 1;
            }
        }
        Self::make(self.neg != other.neg, out)
    }

    /// Divide by one limb, most significant first, keeping the running
    /// remainder in a `u64`. This is the fast path and also what
    /// `to_string_radix` runs on, which is why it is separate.
    fn divrem_small(mag: &[u32], d: u32) -> (Vec<u32>, u32) {
        let mut q = vec![0u32; mag.len()];
        let mut rem = 0u64;
        for i in (0..mag.len()).rev() {
            let cur = (rem << 32) | mag[i] as u64;
            q[i] = (cur / d as u64) as u32;
            rem = cur % d as u64;
        }
        strip(&mut q);
        (q, rem as u32)
    }

    fn shl_bits(mag: &[u32], shift: u32) -> Vec<u32> {
        if shift == 0 {
            return mag.to_vec();
        }
        let mut out = Vec::with_capacity(mag.len() + 1);
        let mut carry = 0u32;
        for &l in mag {
            out.push((l << shift) | carry);
            carry = l >> (32 - shift);
        }
        if carry != 0 {
            out.push(carry);
        }
        out
    }

    fn shr_bits(mag: &[u32], shift: u32) -> Vec<u32> {
        if shift == 0 {
            let mut v = mag.to_vec();
            strip(&mut v);
            return v;
        }
        let mut out = vec![0u32; mag.len()];
        for i in 0..mag.len() {
            let hi = if i + 1 < mag.len() {
                (mag[i + 1] as u64) << (32 - shift)
            } else {
                0
            };
            out[i] = (((mag[i] as u64) >> shift) | hi) as u32;
        }
        strip(&mut out);
        out
    }

    /// Knuth's algorithm D, in the Hacker's Delight formulation of the
    /// multiply-and-subtract step: the borrow is carried as a signed value
    /// and `t >> 32` on a negative `i64` is the arithmetic shift that
    /// propagates it.
    fn divrem_mag(u: &[u32], v: &[u32]) -> (Vec<u32>, Vec<u32>) {
        debug_assert!(v.len() >= 2);
        if Self::cmp_mag(u, v) == Ordering::Less {
            return (Vec::new(), u.to_vec());
        }
        let n = v.len();
        let m = u.len() - n;
        let shift = v[n - 1].leading_zeros();
        let vn = Self::shl_bits(v, shift);
        let mut un = Self::shl_bits(u, shift);
        un.resize(u.len() + 1, 0);
        let mut q = vec![0u32; m + 1];

        for j in (0..=m).rev() {
            let top = ((un[j + n] as u64) << 32) | un[j + n - 1] as u64;
            let mut qhat = top / vn[n - 1] as u64;
            let mut rhat = top % vn[n - 1] as u64;
            loop {
                if qhat >= BASE
                    || qhat * vn[n - 2] as u64
                        > ((rhat << 32) | un[j + n - 2] as u64)
                {
                    qhat -= 1;
                    rhat += vn[n - 1] as u64;
                    if rhat < BASE {
                        continue;
                    }
                }
                break;
            }
            // Multiply and subtract.
            let mut borrow = 0i64;
            for i in 0..n {
                let p = qhat * vn[i] as u64;
                let t = (un[i + j] as i64) - borrow - ((p & MASK) as i64);
                un[i + j] = t as u32;
                borrow = ((p >> 32) as i64) - (t >> 32);
            }
            let t = (un[j + n] as i64) - borrow;
            un[j + n] = t as u32;

            if t < 0 {
                // qhat was one too large: give one back and add the divisor
                // in again.
                qhat -= 1;
                let mut carry = 0u64;
                for i in 0..n {
                    let s = un[i + j] as u64 + vn[i] as u64 + carry;
                    un[i + j] = (s & MASK) as u32;
                    carry = s >> 32;
                }
                un[j + n] = (un[j + n] as u64 + carry) as u32;
            }
            q[j] = qhat as u32;
        }

        let r = Self::shr_bits(&un[..n], shift);
        strip(&mut q);
        (q, r)
    }

    /// Truncating division: the quotient rounds toward zero and the
    /// remainder takes the dividend's sign, which is what Scheme's
    /// `quotient` and `remainder` mean.
    pub fn divrem(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() {
            return None;
        }
        let (qm, rm) = if other.mag.len() == 1 {
            let (q, r) = Self::divrem_small(&self.mag, other.mag[0]);
            (q, if r == 0 { Vec::new() } else { vec![r] })
        } else {
            Self::divrem_mag(&self.mag, &other.mag)
        };
        Some((
            Self::make(self.neg != other.neg, qm),
            Self::make(self.neg, rm),
        ))
    }

    pub fn pow(&self, mut e: u32) -> Self {
        let mut base = self.clone();
        let mut acc = Self::from_i64(1);
        while e > 0 {
            if e & 1 == 1 {
                acc = acc.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                base = base.mul(&base);
            }
        }
        acc
    }

    pub fn gcd(&self, other: &Self) -> Self {
        let mut a = self.abs();
        let mut b = other.abs();
        while !b.is_zero() {
            let (_, r) = a.divrem(&b).expect("b is non-zero");
            a = b;
            b = r.abs();
        }
        a
    }

    pub fn shl(&self, bits: u32) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let limbs = (bits / 32) as usize;
        let rest = bits % 32;
        let mut mag = vec![0u32; limbs];
        mag.extend_from_slice(&Self::shl_bits(&self.mag, rest));
        Self::make(self.neg, mag)
    }

    /// Arithmetic right shift, i.e. floor division by `2^bits`, so that a
    /// negative value shifts toward negative infinity as the two's
    /// complement reading of the operation requires.
    pub fn shr(&self, bits: u32) -> Self {
        let limbs = (bits / 32) as usize;
        if limbs >= self.mag.len() {
            return if self.neg { Self::from_i64(-1) } else { Self::zero() };
        }
        let rest = bits % 32;
        let shifted = Self::shr_bits(&self.mag[limbs..], rest);
        let out = Self::make(self.neg, shifted);
        if !self.neg {
            return out;
        }
        // Negative and something was shifted out: round away from zero.
        let dropped_limbs = self.mag[..limbs].iter().any(|&l| l != 0);
        let dropped_bits =
            rest != 0 && (self.mag[limbs] & ((1u32 << rest) - 1)) != 0;
        if dropped_limbs || dropped_bits {
            out.sub(&Self::from_i64(1))
        } else {
            out
        }
    }

    /// The number of bits in the magnitude. Used to refuse a result that
    /// would be astronomically large *before* computing it, which is the
    /// only point at which refusing is cheap.
    pub fn bit_len(&self) -> u64 {
        match self.mag.last() {
            None => 0,
            Some(&top) => {
                (self.mag.len() as u64 - 1) * 32 + (32 - top.leading_zeros()) as u64
            }
        }
    }

    pub fn to_f64(&self) -> f64 {
        let mut acc = 0.0f64;
        for &l in self.mag.iter().rev() {
            acc = acc * (BASE as f64) + l as f64;
        }
        if self.neg { -acc } else { acc }
    }

    /// An integer-valued `f64`, exactly. `None` for a non-finite or
    /// non-integral argument.
    pub fn from_f64(f: f64) -> Option<Self> {
        if !f.is_finite() || f.fract() != 0.0 {
            return None;
        }
        if f == 0.0 {
            return Some(Self::zero());
        }
        let neg = f < 0.0;
        let bits = f.abs().to_bits();
        let raw_exp = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & 0x000f_ffff_ffff_ffff;
        // Subnormals cannot be integral except zero, handled above.
        let mantissa = frac | (1u64 << 52);
        let exp = raw_exp - 1075; // 1023 bias + 52 fraction bits
        let base = Self::from_u64_mag(neg, mantissa);
        Some(if exp >= 0 {
            base.shl(exp as u32)
        } else {
            // Exact, because f was integral: the bits shifted out are zero.
            base.shr((-exp) as u32)
        })
    }

    pub fn to_string_radix(&self, radix: u32) -> String {
        debug_assert!((2..=16).contains(&radix));
        if self.is_zero() {
            return "0".to_string();
        }
        // The largest power of `radix` that fits a u32, so one short
        // division yields several digits.
        let mut chunk = 1u32;
        let mut digits_per_chunk = 0;
        while (chunk as u64) * (radix as u64) < BASE {
            chunk *= radix;
            digits_per_chunk += 1;
        }
        let mut out = Vec::new();
        let mut mag = self.mag.clone();
        while !mag.is_empty() {
            let (q, r) = Self::divrem_small(&mag, chunk);
            let mut r = r;
            for _ in 0..digits_per_chunk {
                out.push(std::char::from_digit(r % radix, radix).unwrap());
                r /= radix;
                if q.is_empty() && r == 0 {
                    break;
                }
            }
            mag = q;
        }
        while out.last() == Some(&'0') {
            out.pop();
        }
        if self.neg {
            out.push('-');
        }
        out.iter().rev().collect()
    }

    /// The digits of an integer in `radix`, with an optional sign. `None`
    /// on anything else, so the caller can fall through to its other
    /// number syntaxes.
    pub fn parse(s: &str, radix: u32) -> Option<Self> {
        let (neg, digits) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        if digits.is_empty() {
            return None;
        }
        let mut acc = Self::zero();
        let big_radix = Self::from_i64(radix as i64);
        for c in digits.chars() {
            let d = c.to_digit(radix)?;
            acc = acc.mul(&big_radix).add(&Self::from_i64(d as i64));
        }
        Some(Self::make(neg, acc.mag))
    }

    /// A hash that agrees with the one `prim.rs` computes for an equal
    /// fixnum, which `equal-hash` depends on.
    pub fn hash_u64(&self) -> u64 {
        let mut h = 1469598103934665603u64;
        for &l in &self.mag {
            h ^= l as u64;
            h = h.wrapping_mul(1099511628211);
        }
        if self.neg {
            h = h.wrapping_mul(31).wrapping_add(1);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(n: i64) -> BigInt {
        BigInt::from_i64(n)
    }

    /// A deterministic xorshift, so the randomised checks below are the
    /// same every run and a failure can be reproduced from the seed alone.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn round_trips_through_i64_including_the_edges() {
        for n in [0i64, 1, -1, 2, -2, i64::MAX, i64::MIN, i64::MIN + 1, 1 << 32, -(1 << 32)] {
            assert_eq!(big(n).to_i64(), Some(n), "{n}");
        }
    }

    #[test]
    fn two_to_the_sixty_three_is_the_first_value_that_does_not_fit() {
        // The literal that does not read today, and the reason `u64?`
        // stopped at 2^63-1.
        let two63 = big(i64::MAX).add(&big(1));
        assert_eq!(two63.to_i64(), None);
        assert_eq!(two63.to_string_radix(10), "9223372036854775808");
        // Its negation does fit, which is the asymmetry `to_i64` has to
        // get right.
        assert_eq!(two63.neg().to_i64(), Some(i64::MIN));
    }

    #[test]
    fn arithmetic_agrees_with_i128_on_randomised_inputs() {
        let mut rng = Rng(20_261_015);
        for _ in 0..4000 {
            let a = rng.next() as i64 / 3;
            let b = rng.next() as i64 / 3;
            let (ba, bb) = (big(a), big(b));
            let (ia, ib) = (a as i128, b as i128);
            assert_eq!(ba.add(&bb).to_string_radix(10), (ia + ib).to_string(), "{a}+{b}");
            assert_eq!(ba.sub(&bb).to_string_radix(10), (ia - ib).to_string(), "{a}-{b}");
            assert_eq!(ba.mul(&bb).to_string_radix(10), (ia * ib).to_string(), "{a}*{b}");
            if b != 0 {
                let (q, r) = ba.divrem(&bb).unwrap();
                assert_eq!(q.to_string_radix(10), (ia / ib).to_string(), "{a}/{b}");
                assert_eq!(r.to_string_radix(10), (ia % ib).to_string(), "{a}%{b}");
            }
            assert_eq!(ba.cmp(&bb), ia.cmp(&ib), "{a} cmp {b}");
        }
    }

    /// The multi-limb divisor path, which randomised i64 inputs never
    /// reach because their divisors fit in one or two limbs.
    #[test]
    fn division_is_exact_on_wide_operands() {
        let mut rng = Rng(7);
        for _ in 0..600 {
            let mut a = BigInt::zero();
            let mut b = BigInt::zero();
            for _ in 0..6 {
                a = a.mul(&big(1 << 31)).add(&big((rng.next() >> 33) as i64));
            }
            for _ in 0..3 {
                b = b.mul(&big(1 << 31)).add(&big((rng.next() >> 33) as i64));
            }
            if b.is_zero() {
                continue;
            }
            let (q, r) = a.divrem(&b).unwrap();
            // The defining property, checked rather than the digits:
            // a == q*b + r, and |r| < |b|.
            assert_eq!(q.mul(&b).add(&r), a);
            assert_eq!(r.abs().cmp(&b.abs()), Ordering::Less);
        }
    }

    #[test]
    fn division_signs_are_truncating_like_scheme_quotient() {
        for (a, b, q, r) in [
            (7i64, 2i64, 3i64, 1i64),
            (-7, 2, -3, -1),
            (7, -2, -3, 1),
            (-7, -2, 3, -1),
        ] {
            let (bq, br) = big(a).divrem(&big(b)).unwrap();
            assert_eq!(bq.to_i64(), Some(q), "{a}/{b}");
            assert_eq!(br.to_i64(), Some(r), "{a}%{b}");
        }
    }

    #[test]
    fn dividing_by_zero_has_no_answer_rather_than_panicking() {
        assert!(big(5).divrem(&big(0)).is_none());
    }

    #[test]
    fn powers_and_factorials_are_right() {
        assert_eq!(big(2).pow(64).to_string_radix(10), "18446744073709551616");
        assert_eq!(big(10).pow(30).to_string_radix(10), format!("1{}", "0".repeat(30)));
        let mut f = big(1);
        for i in 1..=30 {
            f = f.mul(&big(i));
        }
        assert_eq!(f.to_string_radix(10), "265252859812191058636308480000000");
    }

    #[test]
    fn strings_round_trip_in_every_radix_we_print() {
        let n = big(2).pow(100).add(&big(12345));
        for radix in [2u32, 8, 10, 16] {
            let s = n.to_string_radix(radix);
            assert_eq!(BigInt::parse(&s, radix), Some(n.clone()), "radix {radix}");
            let m = n.neg();
            let s = m.to_string_radix(radix);
            assert_eq!(BigInt::parse(&s, radix), Some(m), "radix {radix} negative");
        }
    }

    #[test]
    fn parsing_rejects_what_is_not_an_integer() {
        assert!(BigInt::parse("", 10).is_none());
        assert!(BigInt::parse("-", 10).is_none());
        assert!(BigInt::parse("12x", 10).is_none());
        assert!(BigInt::parse("1.5", 10).is_none());
        assert!(BigInt::parse("9", 8).is_none());
    }

    #[test]
    fn f64_conversion_is_exact_where_it_can_be() {
        for n in [0i64, 1, -1, 1 << 52, -(1 << 52), i64::MAX / 3] {
            let f = n as f64;
            if f as i64 == n {
                assert_eq!(BigInt::from_f64(f).and_then(|b| b.to_i64()), Some(n), "{n}");
            }
        }
        assert_eq!(BigInt::from_f64(1.5), None);
        assert_eq!(BigInt::from_f64(f64::NAN), None);
        assert_eq!(BigInt::from_f64(f64::INFINITY), None);
        // 2^80 has no i64 but is exactly a double, and must survive both ways.
        let big80 = big(2).pow(80);
        assert_eq!(BigInt::from_f64(big80.to_f64()), Some(big80.clone()));
        assert_eq!(big80.to_f64(), 2f64.powi(80));
    }

    #[test]
    fn shifting_left_then_right_returns_the_value() {
        let n = big(2).pow(70).add(&big(999));
        for bits in [1u32, 31, 32, 33, 64, 100] {
            assert_eq!(n.shl(bits).shr(bits), n, "{bits}");
        }
    }

    /// A right shift is floor division, so it rounds a negative value away
    /// from zero -- the two's complement reading, not truncation.
    #[test]
    fn right_shift_of_a_negative_floors() {
        assert_eq!(big(-1).shr(1).to_i64(), Some(-1));
        assert_eq!(big(-3).shr(1).to_i64(), Some(-2));
        assert_eq!(big(-4).shr(1).to_i64(), Some(-2));
        assert_eq!(big(-1).shr(100).to_i64(), Some(-1));
        let n = big(2).pow(80).neg().sub(&big(1));
        assert_eq!(n.shr(80).to_i64(), Some(-2));
    }

    #[test]
    fn bit_len_is_right() {
        assert_eq!(BigInt::zero().bit_len(), 0);
        assert_eq!(big(1).bit_len(), 1);
        assert_eq!(big(255).bit_len(), 8);
        assert_eq!(big(256).bit_len(), 9);
        assert_eq!(big(2).pow(64).bit_len(), 65);
        assert_eq!(big(-1).bit_len(), 1);
    }

    #[test]
    fn zero_has_one_representation() {
        assert!(!big(0).is_negative());
        assert!(!big(0).neg().is_negative());
        assert_eq!(big(5).sub(&big(5)), BigInt::zero());
        assert_eq!(big(5).mul(&big(0)), BigInt::zero());
        assert_eq!(big(0).to_string_radix(10), "0");
        assert_eq!(big(0).hash_u64(), BigInt::zero().hash_u64());
    }

    #[test]
    fn gcd_is_non_negative_and_right() {
        assert_eq!(big(12).gcd(&big(18)).to_i64(), Some(6));
        assert_eq!(big(-12).gcd(&big(18)).to_i64(), Some(6));
        assert_eq!(big(0).gcd(&big(5)).to_i64(), Some(5));
        assert_eq!(big(5).gcd(&big(0)).to_i64(), Some(5));
        let a = big(2).pow(40).mul(&big(3));
        let b = big(2).pow(30).mul(&big(9));
        assert_eq!(a.gcd(&b), big(2).pow(30).mul(&big(3)));
    }
}
