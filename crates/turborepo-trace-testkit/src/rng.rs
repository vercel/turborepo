//! A tiny deterministic PRNG for input generation.

/// xorshift64* generator. Deterministic for a given seed; not cryptographic.
#[derive(Clone, Debug)]
pub struct XorShift(u64);

impl XorShift {
    /// Creates a generator. A zero seed is replaced by a fixed non-zero
    /// constant.
    pub fn new(seed: u64) -> Self {
        XorShift(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n`; returns 0 when `n == 0`.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// A random byte.
    pub fn byte(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }

    /// `true` with probability `1/n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    /// A random element of a non-empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len())])
        }
    }
}

/// FNV-1a, used to derive per-language seeds.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        let mut a = XorShift::new(42);
        let mut b = XorShift::new(42);
        for _ in 0..1000 {
            let x = a.below(7);
            assert_eq!(x, b.below(7));
            assert!(x < 7);
        }
        assert_eq!(XorShift::new(0).below(0), 0);
        assert!(XorShift::new(1).pick::<u8>(&[]).is_none());
    }
}
