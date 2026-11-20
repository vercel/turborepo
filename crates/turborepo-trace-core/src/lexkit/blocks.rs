//! Byte-set classification of 64-byte blocks into bit masks, for skimming
//! lexers.
//!
//! A [`ByteSet`] is stored as two 16-entry nibble tables (the "shuffle lookup"
//! technique): byte `b` is a member iff `lo[b & 15] & hi[b >> 4] != 0`. This is
//! exact for any set whose rows (the members sharing a high nibble) form at
//! most 8 distinct low-nibble patterns, which covers lexer delimiter sets.
//! [`masks2`] / [`masks3`] classify a 64-byte block against several sets at
//! once and return one bit per byte (bit `i` = `block[i]`), so a lexer can jump
//! between "interesting" bytes with `trailing_zeros` instead of a per-byte
//! loop.
//!
//! Implementations: AVX2 or SSSE3 on x86_64 (detected at run time), NEON on
//! aarch64, and a scalar fallback elsewhere. All produce identical masks
//! (tested against the scalar code).

/// Bytes per block.
pub const BLOCK: usize = 64;

/// A set of bytes in nibble-table form. See the module docs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ByteSet {
    lo: [u8; 16],
    hi: [u8; 16],
}

impl ByteSet {
    /// The empty set.
    pub const EMPTY: ByteSet = ByteSet {
        lo: [0; 16],
        hi: [0; 16],
    };

    /// Builds the set of bytes for which `member` returns true, or `None` if
    /// its rows need more than 8 distinct low-nibble patterns. Sets of
    /// ASCII bytes (at most 8 rows) always fit.
    pub fn new(member: impl Fn(u8) -> bool) -> Option<ByteSet> {
        let mut rows = [0u16; 16];
        for b in 0..=255u8 {
            if member(b) {
                rows[(b >> 4) as usize] |= 1 << (b & 15);
            }
        }
        let mut patterns: [u16; 8] = [0; 8];
        let mut n = 0;
        let mut lo = [0u8; 16];
        let mut hi = [0u8; 16];
        for (h, &row) in rows.iter().enumerate() {
            if row == 0 {
                continue;
            }
            let k = match patterns[..n].iter().position(|&p| p == row) {
                Some(k) => k,
                None if n == 8 => return None,
                None => {
                    patterns[n] = row;
                    n += 1;
                    n - 1
                }
            };
            hi[h] |= 1 << k;
            for (l, lo_entry) in lo.iter_mut().enumerate() {
                if row & (1 << l) != 0 {
                    *lo_entry |= 1 << k;
                }
            }
        }
        Some(ByteSet { lo, hi })
    }

    /// Whether `b` is in the set.
    #[inline]
    pub fn contains(&self, b: u8) -> bool {
        self.lo[(b & 15) as usize] & self.hi[(b >> 4) as usize] != 0
    }
}

/// The masks of the bytes of `block` that are in `a` and in `b` (bit `i` =
/// `block[i]`).
#[inline]
pub fn masks2(block: &[u8; BLOCK], a: &ByteSet, b: &ByteSet) -> (u64, u64) {
    let [ma, mb] = masks(block, [a, b]);
    (ma, mb)
}

/// [`masks2`] for three sets.
#[inline]
pub fn masks3(block: &[u8; BLOCK], a: &ByteSet, b: &ByteSet, c: &ByteSet) -> (u64, u64, u64) {
    let [ma, mb, mc] = masks(block, [a, b, c]);
    (ma, mb, mc)
}

/// [`masks2`] for four sets.
#[inline]
pub fn masks4(
    block: &[u8; BLOCK],
    a: &ByteSet,
    b: &ByteSet,
    c: &ByteSet,
    d: &ByteSet,
) -> (u64, u64, u64, u64) {
    let [ma, mb, mc, md] = masks(block, [a, b, c, d]);
    (ma, mb, mc, md)
}

/// [`masks2`] for `N` sets.
#[inline]
pub fn masks_n<const N: usize>(block: &[u8; BLOCK], sets: [&ByteSet; N]) -> [u64; N] {
    masks(block, sets)
}

#[inline]
fn masks<const N: usize>(block: &[u8; BLOCK], sets: [&ByteSet; N]) -> [u64; N] {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 is available (checked above).
            return unsafe { x86::masks_avx2(block, sets) };
        }
        if std::arch::is_x86_feature_detected!("ssse3") {
            // SAFETY: SSSE3 is available (checked above).
            return unsafe { x86::masks_ssse3(block, sets) };
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON is part of the aarch64 baseline.
        return neon::masks(block, sets);
    }
    #[allow(unreachable_code)]
    masks_scalar(block, sets)
}

fn masks_scalar<const N: usize>(block: &[u8; BLOCK], sets: [&ByteSet; N]) -> [u64; N] {
    let mut out = [0u64; N];
    for (i, &c) in block.iter().enumerate() {
        for (m, s) in out.iter_mut().zip(sets) {
            *m |= u64::from(s.contains(c)) << i;
        }
    }
    out
}

/// Portable reference implementation of [`masks2`].
pub fn masks2_scalar(block: &[u8; BLOCK], a: &ByteSet, b: &ByteSet) -> (u64, u64) {
    let [ma, mb] = masks_scalar(block, [a, b]);
    (ma, mb)
}

/// The index of the first byte of `hay` equal to `a`, `b` or `c`.
///
/// Scans 16 bytes per step with no setup, so it beats `memchr3` when the match
/// is usually close (string literal bodies). SSE2 on x86_64, NEON on aarch64,
/// scalar elsewhere.
#[inline]
pub fn find_any3(hay: &[u8], a: u8, b: u8, c: u8) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        // SSE2 is part of the x86_64 baseline.
        return x86::find_any3_sse2(hay, a, b, c);
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON is part of the aarch64 baseline.
        return neon::find_any3(hay, a, b, c);
    }
    #[allow(unreachable_code)]
    find_any3_scalar(hay, a, b, c)
}

fn find_any3_scalar(hay: &[u8], a: u8, b: u8, c: u8) -> Option<usize> {
    hay.iter().position(|&x| x == a || x == b || x == c)
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use std::arch::x86_64::*;

    use super::{BLOCK, ByteSet, find_any3_scalar};

    #[inline]
    pub(super) fn find_any3_sse2(hay: &[u8], a: u8, b: u8, c: u8) -> Option<usize> {
        // SAFETY: SSE2 is part of the x86_64 baseline; every load reads 16 bytes inside
        // `hay`.
        unsafe {
            let (va, vb, vc) = (
                _mm_set1_epi8(a as i8),
                _mm_set1_epi8(b as i8),
                _mm_set1_epi8(c as i8),
            );
            let mut i = 0;
            while i + 16 <= hay.len() {
                let v = _mm_loadu_si128(hay.as_ptr().add(i).cast::<__m128i>());
                let m = _mm_or_si128(
                    _mm_or_si128(_mm_cmpeq_epi8(v, va), _mm_cmpeq_epi8(v, vb)),
                    _mm_cmpeq_epi8(v, vc),
                );
                let bits = _mm_movemask_epi8(m) as u32;
                if bits != 0 {
                    return Some(i + bits.trailing_zeros() as usize);
                }
                i += 16;
            }
            find_any3_scalar(&hay[i..], a, b, c).map(|k| i + k)
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn masks_avx2<const N: usize>(
        block: &[u8; BLOCK],
        sets: [&ByteSet; N],
    ) -> [u64; N] {
        // SAFETY: every load reads 16 or 32 bytes inside the referenced arrays.
        unsafe {
            let t = |x: &[u8; 16]| {
                _mm256_broadcastsi128_si256(_mm_loadu_si128(x.as_ptr().cast::<__m128i>()))
            };
            let nib = _mm256_set1_epi8(0x0f);
            let zero = _mm256_setzero_si256();
            let p = block.as_ptr().cast::<__m256i>();
            let (v0, v1) = (_mm256_loadu_si256(p), _mm256_loadu_si256(p.add(1)));
            let (lo0, lo1) = (_mm256_and_si256(v0, nib), _mm256_and_si256(v1, nib));
            let hi0 = _mm256_and_si256(_mm256_srli_epi16::<4>(v0), nib);
            let hi1 = _mm256_and_si256(_mm256_srli_epi16::<4>(v1), nib);
            let one = |s: &ByteSet| {
                let (lt, ht) = (t(&s.lo), t(&s.hi));
                let m0 =
                    _mm256_and_si256(_mm256_shuffle_epi8(lt, lo0), _mm256_shuffle_epi8(ht, hi0));
                let m1 =
                    _mm256_and_si256(_mm256_shuffle_epi8(lt, lo1), _mm256_shuffle_epi8(ht, hi1));
                let z0 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(m0, zero)) as u32;
                let z1 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(m1, zero)) as u32;
                u64::from(!z0) | (u64::from(!z1) << 32)
            };
            sets.map(one)
        }
    }

    #[target_feature(enable = "ssse3")]
    pub(super) unsafe fn masks_ssse3<const N: usize>(
        block: &[u8; BLOCK],
        sets: [&ByteSet; N],
    ) -> [u64; N] {
        // SAFETY: every load reads 16 bytes inside the referenced arrays.
        unsafe {
            let t = |x: &[u8; 16]| _mm_loadu_si128(x.as_ptr().cast::<__m128i>());
            let lo_t = sets.map(|s| t(&s.lo));
            let hi_t = sets.map(|s| t(&s.hi));
            let nib = _mm_set1_epi8(0x0f);
            let zero = _mm_setzero_si128();
            let mut out = [0u64; N];
            for i in 0..4 {
                let v = _mm_loadu_si128(block.as_ptr().add(16 * i).cast::<__m128i>());
                let lo = _mm_and_si128(v, nib);
                let hi = _mm_and_si128(_mm_srli_epi16::<4>(v), nib);
                for k in 0..N {
                    let m =
                        _mm_and_si128(_mm_shuffle_epi8(lo_t[k], lo), _mm_shuffle_epi8(hi_t[k], hi));
                    let z = _mm_movemask_epi8(_mm_cmpeq_epi8(m, zero)) as u32;
                    out[k] |= u64::from(!z & 0xffff) << (16 * i);
                }
            }
            out
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::*;

    use super::{BLOCK, ByteSet, find_any3_scalar};

    #[inline]
    pub(super) fn find_any3(hay: &[u8], a: u8, b: u8, c: u8) -> Option<usize> {
        // SAFETY: NEON is always available on aarch64; every load reads 16 bytes inside
        // `hay`.
        unsafe {
            let (va, vb, vc) = (vdupq_n_u8(a), vdupq_n_u8(b), vdupq_n_u8(c));
            let mut i = 0;
            while i + 16 <= hay.len() {
                let v = vld1q_u8(hay.as_ptr().add(i));
                let m = vorrq_u8(vorrq_u8(vceqq_u8(v, va), vceqq_u8(v, vb)), vceqq_u8(v, vc));
                // Four bits per byte.
                let nib = vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(
                    vreinterpretq_u16_u8(m),
                )));
                if nib != 0 {
                    return Some(i + (nib.trailing_zeros() / 4) as usize);
                }
                i += 16;
            }
            find_any3_scalar(&hay[i..], a, b, c).map(|k| i + k)
        }
    }

    #[inline]
    pub(super) fn masks<const N: usize>(block: &[u8; BLOCK], sets: [&ByteSet; N]) -> [u64; N] {
        // SAFETY: NEON is always available on aarch64; every load reads 16 bytes inside
        // the referenced arrays.
        unsafe {
            let lo_t = sets.map(|s| vld1q_u8(s.lo.as_ptr()));
            let hi_t = sets.map(|s| vld1q_u8(s.hi.as_ptr()));
            let nib = vdupq_n_u8(0x0f);
            // Low and high nibbles of each of the block's four 16-byte lanes.
            let halves: [(uint8x16_t, uint8x16_t); 4] = std::array::from_fn(|i| {
                let v = vld1q_u8(block.as_ptr().add(16 * i));
                (vandq_u8(v, nib), vshrq_n_u8::<4>(v))
            });
            let mut out = [0u64; N];
            for ((o, lt), ht) in out.iter_mut().zip(&lo_t).zip(&hi_t) {
                *o = movemask64(
                    halves.map(|(lo, hi)| vtstq_u8(vqtbl1q_u8(*lt, lo), vqtbl1q_u8(*ht, hi))),
                );
            }
            out
        }
    }

    /// 64 comparison lanes (0x00 / 0xFF) to a bit mask.
    #[inline(always)]
    unsafe fn movemask64(c: [uint8x16_t; 4]) -> u64 {
        // SAFETY: plain register arithmetic on a constant table.
        unsafe {
            const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
            let bits = vld1q_u8(BITS.as_ptr());
            let s0 = vpaddq_u8(vandq_u8(c[0], bits), vandq_u8(c[1], bits));
            let s1 = vpaddq_u8(vandq_u8(c[2], bits), vandq_u8(c[3], bits));
            let s = vpaddq_u8(s0, s1);
            let s = vpaddq_u8(s, s);
            vgetq_lane_u64::<0>(vreinterpretq_u64_u8(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sets() -> Vec<ByteSet> {
        vec![
            ByteSet::new(|b| b"'\"`/{}()[]?:<!".contains(&b) || b >= 0x80).unwrap(),
            ByteSet::new(|b| b.is_ascii_alphanumeric() || b"$_\\".contains(&b)).unwrap(),
            ByteSet::new(|b| b == b'\n').unwrap(),
            ByteSet::new(|b| b"iermnclvdt".contains(&b)).unwrap(),
            ByteSet::EMPTY,
            ByteSet::new(|_| true).unwrap(),
        ]
    }

    #[test]
    fn membership_is_exact() {
        let ev = |b: u8| b"'\"`/{}()[]?:<!".contains(&b) || b >= 0x80;
        let s = ByteSet::new(ev).unwrap();
        for b in 0..=255u8 {
            assert_eq!(s.contains(b), ev(b), "{b:#x}");
        }
        let id = |b: u8| b.is_ascii_alphanumeric() || b"$_\\".contains(&b);
        let s = ByteSet::new(id).unwrap();
        for b in 0..=255u8 {
            assert_eq!(s.contains(b), id(b), "{b:#x}");
            assert!(!ByteSet::EMPTY.contains(b));
        }
    }

    #[test]
    fn too_many_patterns_is_rejected() {
        // 9 high nibbles, each with a different single low nibble.
        assert!(ByteSet::new(|b| (b >> 4) < 9 && (b & 15) == (b >> 4)).is_none());
        assert!(ByteSet::new(|b| (b >> 4) < 8 && (b & 15) == (b >> 4)).is_some());
    }

    #[test]
    fn find_any3_matches_scalar() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for len in 0..300 {
            for _ in 0..20 {
                let hay: Vec<u8> = (0..len)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        if seed.is_multiple_of(23) {
                            b"'\\\n"[(seed % 3) as usize]
                        } else {
                            b'a' + (seed % 26) as u8
                        }
                    })
                    .collect();
                assert_eq!(
                    find_any3(&hay, b'\'', b'\\', b'\n'),
                    find_any3_scalar(&hay, b'\'', b'\\', b'\n'),
                    "{len}"
                );
            }
        }
    }

    #[test]
    fn simd_matches_scalar() {
        let sets = sets();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut block = [0u8; BLOCK];
        for round in 0..20_000 {
            for byte in block.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = if round % 3 == 0 {
                    b"a(b)'c'/\n{}[]?:<!\x80\xff_$\\ 09zZ"[(seed % 26) as usize]
                } else {
                    seed as u8
                };
            }
            for a in &sets {
                for b in &sets {
                    let want = masks2_scalar(&block, a, b);
                    assert_eq!(masks2(&block, a, b), want);
                    let c = &sets[(seed % sets.len() as u64) as usize];
                    let want3 = masks_scalar(&block, [a, b, c]);
                    assert_eq!(masks3(&block, a, b, c), (want3[0], want3[1], want3[2]));
                    assert_eq!(masks(&block, [a, b, c]), want3);
                    let want4 = masks_scalar(&block, [a, b, c, b]);
                    assert_eq!(
                        masks4(&block, a, b, c, b),
                        (want4[0], want4[1], want4[2], want4[3])
                    );
                    #[cfg(target_arch = "x86_64")]
                    if std::arch::is_x86_feature_detected!("ssse3") {
                        // SAFETY: SSSE3 is available (checked above).
                        assert_eq!(
                            unsafe { x86::masks_ssse3(&block, [a, b]) },
                            [want.0, want.1]
                        );
                        // SAFETY: as above.
                        assert_eq!(unsafe { x86::masks_ssse3(&block, [a, b, c]) }, want3);
                    }
                }
            }
        }
    }
}
