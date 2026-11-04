//! A hasher for the interpreter's own maps, whose keys are symbol ids,
//! heap references and short names: FxHash's multiply-rotate, written here
//! because the crate takes no dependencies. std's SipHash resists
//! flooding, which these keys cannot be used for, and costs more than the
//! lookup it serves -- `Engine::resolve` runs for every identifier the
//! compiler and the macro expander see.

use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub struct Fx(u64);

const K: u64 = 0x517c_c1b7_2722_0a95;

impl Fx {
    #[inline]
    fn add(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(K);
    }
}

impl Hasher for Fx {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().expect("eight bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(last));
        }
    }

    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.add(n as u64);
    }

    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.add(n as u64);
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }
}

pub type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<Fx>>;
pub type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<Fx>>;
