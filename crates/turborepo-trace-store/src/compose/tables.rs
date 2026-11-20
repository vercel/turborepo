//! The probe, membership and config-input tables of a composed graph, their
//! inverted indexes, and the cache file both are kept in (`<store
//! dir>/cache/tables`).
//!
//! The tables are a function of the snapshot's shards (see
//! [`super::Graph::build_tables`]). Every `affected` run needs them, so they
//! are stored once per snapshot and then *mapped*: the cache file holds each
//! array as little-endian words at an aligned offset, and a load checks the
//! file's checksum and points the arrays at the mapping instead of copying 100
//! MB into vectors and inverting them again.

use std::{ops::Deref, path::Path, sync::Arc};

use memmap2::Mmap;

/// Two words that are stored and mapped together: a span `(start, end)`, a
/// membership `(directory, pattern)`, an inverted membership `(node, pattern)`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub(super) struct Pair(pub u32, pub u32);

/// Plain data that can be read in place from a mapped file.
///
/// # Safety
/// Every bit pattern must be a valid value, and the type must have no padding.
pub(super) unsafe trait Word: Copy + 'static {}
// SAFETY: integers; `Pair` is `repr(C)` of two `u32`.
unsafe impl Word for u8 {}
// SAFETY: as above.
unsafe impl Word for u32 {}
// SAFETY: as above.
unsafe impl Word for Pair {}

/// An array that was built in memory or is part of the mapped cache file.
pub(super) enum Slab<T: Word> {
    /// Built by this process.
    Owned(Vec<T>),
    /// `len` values at byte offset `off` of the mapping (aligned for `T`,
    /// inside the file).
    Mapped {
        map: Arc<Mmap>,
        off: usize,
        len: usize,
    },
}

impl<T: Word> Default for Slab<T> {
    fn default() -> Self {
        Slab::Owned(Vec::new())
    }
}

impl<T: Word> From<Vec<T>> for Slab<T> {
    fn from(v: Vec<T>) -> Self {
        Slab::Owned(v)
    }
}

impl<T: Word> Deref for Slab<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        match self {
            Slab::Owned(v) => v,
            Slab::Mapped { map, off, len } => {
                // SAFETY: `decode` checked that `off` is aligned for `T` and that
                // `off + len * size_of::<T>()` lies inside the mapping; `T: Word` accepts
                // any bytes; the mapping is read-only and lives as long as `self`.
                unsafe { std::slice::from_raw_parts(map.as_ptr().add(*off).cast::<T>(), *len) }
            }
        }
    }
}

impl<T: Word> std::fmt::Debug for Slab<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Slab({} values)", self.len())
    }
}

/// A sorted string table: `spans[i]` is the range of string `i` in `text`.
#[derive(Default, Debug)]
pub(super) struct Interned {
    /// UTF-8 (checked when a cache file is mapped).
    text: Slab<u8>,
    spans: Slab<Pair>,
}

impl Interned {
    /// The table of `strings` (sorted, distinct).
    pub(super) fn new(strings: &[&str]) -> Interned {
        let total: usize = strings.iter().map(|s| s.len()).sum();
        let mut text = Vec::with_capacity(total);
        let mut spans = Vec::with_capacity(strings.len());
        for s in strings {
            let start = text.len() as u32;
            text.extend_from_slice(s.as_bytes());
            spans.push(Pair(start, text.len() as u32));
        }
        Interned {
            text: text.into(),
            spans: spans.into(),
        }
    }

    /// The number of strings.
    pub(super) fn len(&self) -> usize {
        self.spans.len()
    }

    fn text(&self) -> &str {
        // SAFETY: built from `&str`s, or validated by `decode` when the file was
        // mapped.
        unsafe { std::str::from_utf8_unchecked(&self.text) }
    }

    pub(super) fn get(&self, i: u32) -> &str {
        let Pair(a, b) = self.spans[i as usize];
        &self.text()[a as usize..b as usize]
    }

    /// The id of `s` (strings are sorted).
    pub(super) fn find(&self, s: &str) -> Option<u32> {
        let (mut lo, mut hi) = (0usize, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.get(mid as u32).cmp(s) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid as u32),
            }
        }
        None
    }

    /// The ids of the strings that start with `prefix`: one range, the strings
    /// are sorted.
    pub(super) fn starting_with(&self, prefix: &str) -> std::ops::Range<u32> {
        let (mut lo, mut hi) = (0usize, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.get(mid as u32) < prefix {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut end = lo;
        while end < self.len() && self.get(end as u32).starts_with(prefix) {
            end += 1;
        }
        lo as u32..end as u32
    }
}

/// Per node (CSR): the string ids of its probes, memberships `(directory,
/// pattern)` and config inputs.
#[derive(Default, Debug)]
pub(super) struct Tables {
    pub strings: Interned,
    pub probe_off: Slab<u32>,
    pub probes: Slab<u32>,
    pub member_off: Slab<u32>,
    pub members: Slab<Pair>,
    pub input_off: Slab<u32>,
    pub inputs: Slab<u32>,
}

/// Inverted indexes over [`Tables`]: per string id, the nodes that probe it,
/// list it as a membership directory (with the pattern id), or read it as an
/// input. Built by counting sort over ids, without string comparisons.
#[derive(Default, Debug)]
pub(super) struct Inverted {
    pub probe_off: Slab<u32>,
    pub probe_nodes: Slab<u32>,
    pub member_off: Slab<u32>,
    /// `(node, pattern)`, per directory grouped by pattern.
    pub member_nodes: Slab<Pair>,
    pub input_off: Slab<u32>,
    pub input_nodes: Slab<u32>,
}

/// CSR by key from per-node lists (`off`/`items`), keyed by `key(item)`.
fn invert<T: Copy, V: Copy + Default>(
    nkeys: usize,
    node_off: &[u32],
    items: &[T],
    key: impl Fn(T) -> u32,
    val: impl Fn(u32, T) -> V,
) -> (Vec<u32>, Vec<V>) {
    let mut off = vec![0u32; nkeys + 1];
    for &it in items {
        off[key(it) as usize + 1] += 1;
    }
    for i in 0..nkeys {
        off[i + 1] += off[i];
    }
    let mut cur = off[..nkeys].to_vec();
    let mut out = vec![V::default(); items.len()];
    for n in 0..node_off.len().saturating_sub(1) {
        for &it in &items[node_off[n] as usize..node_off[n + 1] as usize] {
            let k = key(it) as usize;
            out[cur[k] as usize] = val(n as u32, it);
            cur[k] += 1;
        }
    }
    (off, out)
}

impl Inverted {
    /// The inverted indexes of `t`.
    pub(super) fn of(t: &Tables) -> Inverted {
        let n = t.strings.len();
        let (probe_off, probe_nodes) = invert(n, &t.probe_off, &t.probes, |i| i, |n, _| n);
        let (member_off, mut member_nodes) = invert(
            n,
            &t.member_off,
            &t.members,
            |Pair(d, _)| d,
            |n, Pair(_, p)| Pair(n, p),
        );
        // Per directory, group by pattern: `affected` matches each distinct pattern
        // once per directory (a root `**/__mocks__/**` is recorded on every
        // jest test).
        for w in member_off.windows(2) {
            member_nodes[w[0] as usize..w[1] as usize]
                .sort_unstable_by_key(|&Pair(node, pat)| (pat, node));
        }
        let (input_off, input_nodes) = invert(n, &t.input_off, &t.inputs, |i| i, |n, _| n);
        Inverted {
            probe_off: probe_off.into(),
            probe_nodes: probe_nodes.into(),
            member_off: member_off.into(),
            member_nodes: member_nodes.into(),
            input_off: input_off.into(),
            input_nodes: input_nodes.into(),
        }
    }
}

/// The cache file: magic, key, the directory (byte offset and value count of
/// each of the [`SECTIONS`] arrays), the arrays (each at a multiple of 8
/// bytes), and the xxh3-64 of everything before it. Words are little-endian; on
/// a big-endian host the file is neither written nor read.
pub(super) const TABLES_MAGIC: &[u8; 8] = b"DTTABL2\n";
const SECTIONS: usize = 14;
const HEADER: usize = 8 + 16 + SECTIONS * 16;

fn bytes_of<T: Word>(v: &[T]) -> &[u8] {
    // SAFETY: `T: Word` has no padding, so its bytes are initialized.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

/// The cache file's bytes for `t` and its inverted indexes; `None` on a
/// big-endian host.
pub(super) fn encode(t: &Tables, inv: &Inverted, key: u128) -> Option<Vec<u8>> {
    if cfg!(target_endian = "big") {
        return None;
    }
    let sections: [(&[u8], usize); SECTIONS] = [
        (&t.strings.text[..], t.strings.text.len()),
        (bytes_of(&t.strings.spans), t.strings.spans.len()),
        (bytes_of(&t.probe_off), t.probe_off.len()),
        (bytes_of(&t.probes), t.probes.len()),
        (bytes_of(&t.member_off), t.member_off.len()),
        (bytes_of(&t.members), t.members.len()),
        (bytes_of(&t.input_off), t.input_off.len()),
        (bytes_of(&t.inputs), t.inputs.len()),
        (bytes_of(&inv.probe_off), inv.probe_off.len()),
        (bytes_of(&inv.probe_nodes), inv.probe_nodes.len()),
        (bytes_of(&inv.member_off), inv.member_off.len()),
        (bytes_of(&inv.member_nodes), inv.member_nodes.len()),
        (bytes_of(&inv.input_off), inv.input_off.len()),
        (bytes_of(&inv.input_nodes), inv.input_nodes.len()),
    ];
    let total: usize = HEADER + sections.iter().map(|(b, _)| b.len() + 8).sum::<usize>() + 8;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(TABLES_MAGIC);
    out.extend_from_slice(&key.to_le_bytes());
    out.resize(HEADER, 0);
    for (i, (bytes, len)) in sections.iter().enumerate() {
        out.resize(out.len().next_multiple_of(8), 0);
        let (at, off) = (24 + i * 16, out.len() as u64);
        out[at..at + 8].copy_from_slice(&off.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&(*len as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out.resize(out.len().next_multiple_of(8), 0);
    let sum = xxhash_rust::xxh3::xxh3_64(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    Some(out)
}

/// Rewrites the cache file at `path` under the key `to` when it holds the
/// tables of `from`: the same tables, for a manifest whose shards differ only
/// in what the tables do not read ([`super::reuse`]). Returns whether it did.
pub(super) fn rekey(path: &Path, from: u128, to: u128) -> bool {
    let Ok(mut b) = std::fs::read(path) else {
        return false;
    };
    if b.len() < HEADER + 8 || &b[..8] != TABLES_MAGIC || b[8..24] != from.to_le_bytes() {
        return false;
    }
    let body = b.len() - 8;
    if xxhash_rust::xxh3::xxh3_64(&b[..body]).to_le_bytes() != b[body..] {
        return false;
    }
    b[8..24].copy_from_slice(&to.to_le_bytes());
    let sum = xxhash_rust::xxh3::xxh3_64(&b[..body]);
    b[body..].copy_from_slice(&sum.to_le_bytes());
    crate::write_atomic(path, &b).is_ok()
}

/// Maps the cache file at `path` when it holds the tables of `key` for a graph
/// of `nodes` nodes.
pub(super) fn open(path: &Path, key: u128, nodes: usize) -> Option<(Tables, Inverted)> {
    if cfg!(target_endian = "big") {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: the file is replaced by a rename, never written in place
    // (`write_atomic`).
    let map = Arc::new(unsafe { Mmap::map(&file) }.ok()?);
    decode(map, key, nodes)
}

fn decode(map: Arc<Mmap>, key: u128, nodes: usize) -> Option<(Tables, Inverted)> {
    let b: &[u8] = &map;
    if b.len() < HEADER + 8 || &b[..8] != TABLES_MAGIC || b[8..24] != key.to_le_bytes() {
        return None;
    }
    let (body, sum) = b.split_at(b.len() - 8);
    if xxhash_rust::xxh3::xxh3_64(body).to_le_bytes() != sum {
        return None;
    }
    let word = |at: usize| -> Option<usize> {
        Some(u64::from_le_bytes(body.get(at..at + 8)?.try_into().ok()?) as usize)
    };
    // Section `i` as a slab of `T`: aligned and inside the body.
    fn slab<T: Word>(map: &Arc<Mmap>, body_len: usize, off: usize, len: usize) -> Option<Slab<T>> {
        let size = std::mem::size_of::<T>();
        let end = off.checked_add(len.checked_mul(size)?)?;
        if end > body_len
            || !(map.as_ptr() as usize + off).is_multiple_of(std::mem::align_of::<T>())
        {
            return None;
        }
        Some(Slab::Mapped {
            map: Arc::clone(map),
            off,
            len,
        })
    }
    let mut dir = [(0usize, 0usize); SECTIONS];
    for (i, d) in dir.iter_mut().enumerate() {
        *d = (word(24 + i * 16)?, word(24 + i * 16 + 8)?);
    }
    let n = body.len();
    let words = |i: usize| slab::<u32>(&map, n, dir[i].0, dir[i].1);
    let pairs = |i: usize| slab::<Pair>(&map, n, dir[i].0, dir[i].1);
    let text = slab::<u8>(&map, n, dir[0].0, dir[0].1)?;
    std::str::from_utf8(&text).ok()?;
    let t = Tables {
        strings: Interned {
            text,
            spans: pairs(1)?,
        },
        probe_off: words(2)?,
        probes: words(3)?,
        member_off: words(4)?,
        members: pairs(5)?,
        input_off: words(6)?,
        inputs: words(7)?,
    };
    let inv = Inverted {
        probe_off: words(8)?,
        probe_nodes: words(9)?,
        member_off: words(10)?,
        member_nodes: pairs(11)?,
        input_off: words(12)?,
        input_nodes: words(13)?,
    };
    // Shapes: one offset per node (and per string) plus one, offsets inside their
    // arrays, spans inside the text. The values are the checksummed output of
    // `encode`.
    let strings = t.strings.len();
    let csr = |off: &[u32], items: usize, rows: usize| {
        off.len() == rows + 1 && off.last().is_some_and(|&e| e as usize == items)
    };
    let text_len = t.strings.text.len();
    let ok = csr(&t.probe_off, t.probes.len(), nodes)
        && csr(&t.member_off, t.members.len(), nodes)
        && csr(&t.input_off, t.inputs.len(), nodes)
        && csr(&inv.probe_off, inv.probe_nodes.len(), strings)
        && csr(&inv.member_off, inv.member_nodes.len(), strings)
        && csr(&inv.input_off, inv.input_nodes.len(), strings)
        && t.strings
            .spans
            .iter()
            .all(|&Pair(a, b)| a <= b && b as usize <= text_len);
    ok.then_some((t, inv))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Tables, Inverted) {
        let strings = ["", "a", "a/b.ts", "a/é", "z/**"];
        let t = Tables {
            strings: Interned::new(&strings),
            // Three nodes.
            probe_off: vec![0, 2, 2, 3].into(),
            probes: vec![1, 2, 2].into(),
            member_off: vec![0, 0, 1, 2].into(),
            members: vec![Pair(1, 4), Pair(0, 4)].into(),
            input_off: vec![0, 1, 1, 1].into(),
            inputs: vec![3].into(),
        };
        let inv = Inverted::of(&t);
        (t, inv)
    }

    fn same(a: &(Tables, Inverted), b: &(Tables, Inverted)) {
        assert_eq!(a.0.strings.len(), b.0.strings.len());
        for i in 0..a.0.strings.len() as u32 {
            assert_eq!(a.0.strings.get(i), b.0.strings.get(i));
        }
        assert_eq!(*a.0.probe_off, *b.0.probe_off);
        assert_eq!(*a.0.probes, *b.0.probes);
        assert_eq!(*a.0.member_off, *b.0.member_off);
        assert_eq!(*a.0.members, *b.0.members);
        assert_eq!(*a.0.input_off, *b.0.input_off);
        assert_eq!(*a.0.inputs, *b.0.inputs);
        assert_eq!(*a.1.probe_off, *b.1.probe_off);
        assert_eq!(*a.1.probe_nodes, *b.1.probe_nodes);
        assert_eq!(*a.1.member_off, *b.1.member_off);
        assert_eq!(*a.1.member_nodes, *b.1.member_nodes);
        assert_eq!(*a.1.input_off, *b.1.input_off);
        assert_eq!(*a.1.input_nodes, *b.1.input_nodes);
    }

    #[test]
    fn inverts() {
        let (t, inv) = sample();
        assert_eq!(t.strings.find("a/b.ts"), Some(2));
        assert_eq!(t.strings.find("b"), None);
        // String 2 is probed by nodes 0 and 2.
        assert_eq!(
            &inv.probe_nodes[inv.probe_off[2] as usize..inv.probe_off[3] as usize],
            [0, 2]
        );
        // Directory "" is listed by node 2 with pattern 4.
        assert_eq!(
            &inv.member_nodes[inv.member_off[0] as usize..inv.member_off[1] as usize],
            [Pair(2, 4)]
        );
        assert_eq!(
            &inv.input_nodes[inv.input_off[3] as usize..inv.input_off[4] as usize],
            [0]
        );
    }

    #[test]
    fn cache_file_round_trips_mapped() {
        let built = sample();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tables");
        let bytes = encode(&built.0, &built.1, 7).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let mapped = open(&path, 7, 3).expect("valid cache file");
        assert!(matches!(mapped.0.probes, Slab::Mapped { .. }));
        same(&built, &mapped);
        // Another key, another node count, a flipped byte or a cut file: not used.
        assert!(open(&path, 8, 3).is_none());
        assert!(open(&path, 7, 4).is_none());
        let mut bad = bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        std::fs::write(&path, &bad).unwrap();
        assert!(open(&path, 7, 3).is_none());
        std::fs::write(&path, &bytes[..bytes.len() - 9]).unwrap();
        assert!(open(&path, 7, 3).is_none());
        std::fs::write(&path, b"").unwrap();
        assert!(open(&path, 7, 3).is_none());
    }
}
