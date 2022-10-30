use std::{
    cmp::min,
    hash::{Hash, Hasher},
    io::{self, Read},
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

use anyhow::Result;
use serde::{Deserialize, Serialize, Serializer};
use tokio::io::{AsyncRead, ReadBuf};
use turbo_tasks_hash::{hash_xxh3_hash64, DeterministicHash, DeterministicHasher};

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
#[derive(Clone, Debug, Deserialize)]
pub enum Rope {
    Flat(Vec<u8>),
    Concat(usize, Vec<RopeElem>),
}

impl Rope {
    pub fn flatten(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.len());
        self.flatten_internal(&mut buf);
        buf
    }

    pub fn push_bytes(&mut self, bytes: Vec<u8>) {
        match self {
            Rope::Flat(v) => v.extend(bytes),
            Rope::Concat(l, c) => {
                *l += bytes.len();
                match c.last_mut() {
                    None | Some(RopeElem::Borrowed(..)) => c.push(RopeElem::Owned(bytes)),
                    Some(RopeElem::Owned(v)) => v.extend(bytes),
                }
            }
        }
    }

    pub fn concat(&mut self, other: Arc<Rope>) {
        match self {
            Rope::Flat(v) => {
                let l = v.len() + other.len();
                *self = Rope::Concat(
                    l,
                    vec![RopeElem::Owned(mem::take(v)), RopeElem::Borrowed(other)],
                );
            }
            Rope::Concat(l, c) => {
                *l += other.len();
                c.push(RopeElem::Borrowed(other));
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Rope::Flat(f) => f.len(),
            Rope::Concat(l, _) => *l,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn slice(&'_ self, start: usize, end: usize) -> RopeReader<'_> {
        RopeReader::new_slice(self, start, end)
    }

    pub fn read(&'_ self) -> RopeReader<'_> {
        RopeReader::new_full(self)
    }

    pub fn to_string(&self) -> Option<String> {
        let mut read = self.read();
        let mut string = String::new();
        match <RopeReader as Read>::read_to_string(&mut read, &mut string) {
            Ok(_) => Some(string),
            Err(_) => None,
        }
    }

    fn flatten_internal(&self, buf: &mut Vec<u8>) {
        match self {
            Rope::Flat(v) => buf.extend(v),
            Rope::Concat(_, c) => {
                for v in c {
                    v.flatten_internal(buf);
                }
            }
        }
    }
}

impl Default for Rope {
    fn default() -> Self {
        Rope::Flat(vec![])
    }
}

impl DeterministicHash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn deterministic_hash<H: DeterministicHasher>(&self, state: &mut H) {
        match self {
            Rope::Flat(f) => state.write_bytes(f.as_slice()),

            Rope::Concat(_, c) => {
                for v in c {
                    v.deterministic_hash(state);
                }
            }
        }
    }
}

impl Hash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Rope::Flat(f) => state.write(f.as_slice()),
            Rope::Concat(_, c) => {
                for v in c {
                    v.hash(state);
                }
            }
        }
    }
}

impl PartialEq for Rope {
    /// Ropes with similar contents are equals, regardless of their structure.
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        hash_xxh3_hash64(self) == hash_xxh3_hash64(other)
    }
}
impl Eq for Rope {}

/// Ropes are always serialized into flat vecs, because we don't the
/// deserialization won't deduplicate and share the ARCs (being the only
/// possible owner of a bunch doesn't make sense).
impl Serialize for Rope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTupleVariant;
        let mut flat = serializer.serialize_tuple_variant("Rope", 0, "Flat", 1)?;
        flat.serialize_field(&self.flatten())?;
        flat.end()
    }
}

#[turbo_tasks::value(shared, eq = "manual")]
#[derive(Clone, Debug)]
pub enum RopeElem {
    Owned(Vec<u8>),
    // TODO: This owned by others and can grow (but not shrink).
    // We should keep a usize to cap the length we'll access.
    Borrowed(Arc<Rope>),
}

impl RopeElem {
    fn flatten_internal(&self, buf: &mut Vec<u8>) {
        match self {
            RopeElem::Owned(v) => buf.extend(v),
            RopeElem::Borrowed(v) => v.flatten_internal(buf),
        }
    }
}

impl DeterministicHash for RopeElem {
    /// RopeElem with similar contents hash the same, regardless of their
    /// structure.
    fn deterministic_hash<H: DeterministicHasher>(&self, state: &mut H) {
        match self {
            RopeElem::Owned(v) => state.write_bytes(v.as_slice()),
            RopeElem::Borrowed(r) => r.deterministic_hash(state),
        }
    }
}

impl Hash for RopeElem {
    /// RopeElem with similar contents hash the same, regardless of their
    /// structure.
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            RopeElem::Owned(v) => state.write(v.as_slice()),
            RopeElem::Borrowed(r) => r.hash(state),
        }
    }
}

impl PartialEq for RopeElem {
    /// Ropes with similar contents are equals, regardless of their structure.
    fn eq(&self, other: &Self) -> bool {
        hash_xxh3_hash64(self) == hash_xxh3_hash64(other)
    }
}
impl Eq for RopeElem {}

pub struct RopeReader<'a> {
    stack: Vec<RopeReaderState<'a>>,
    max_bytes: usize,
}

impl<'a> RopeReader<'a> {
    fn new_full(rope: &'a Rope) -> Self {
        RopeReader {
            stack: vec![RopeReaderState::full_rope(rope)],
            max_bytes: rope.len(),
        }
    }

    fn new_slice(rope: &'a Rope, start: usize, end: usize) -> Self {
        let mut reader = RopeReader::new_full(rope);
        reader.read_internal(start, &mut None);
        reader.max_bytes = end - start;
        reader
    }

    fn read_internal(&mut self, want: usize, buf: &mut Option<&mut ReadBuf<'_>>) -> usize {
        let mut remaining = want;

        while remaining > 0 {
            let mut current = match self.stack.last_mut() {
                None => break,
                Some(l) => l,
            };

            match current.inner {
                RopeReaderStateElem::Owned(v) => {
                    let pos = current.index;
                    let amount = min(v.len() - pos, remaining);
                    let end = pos + amount;

                    if let Some(buf) = buf.as_mut() {
                        buf.put_slice(&v[pos..end]);
                    }
                    remaining -= amount;

                    if end == v.len() {
                        self.stack.pop();
                    } else {
                        current.index = end;
                    }
                }

                RopeReaderStateElem::Concat(c) => {
                    let pos = current.index;
                    let el = &c[pos];
                    let end = pos + 1;

                    if end == c.len() {
                        self.stack.pop();
                    } else {
                        current.index = end;
                    }

                    self.stack.push(RopeReaderState::full_elem(el));
                }
            }
        }
        want - remaining
    }
}

struct RopeReaderState<'a> {
    inner: RopeReaderStateElem<'a>,
    index: usize,
}

enum RopeReaderStateElem<'a> {
    Owned(&'a Vec<u8>),
    Concat(&'a Vec<RopeElem>),
}

impl<'a> RopeReaderState<'a> {
    fn full_rope(inner: &'a Rope) -> Self {
        match inner {
            Rope::Flat(v) => RopeReaderState {
                inner: RopeReaderStateElem::Owned(v),
                index: 0,
            },
            Rope::Concat(_, v) => RopeReaderState {
                inner: RopeReaderStateElem::Concat(v),
                index: 0,
            },
        }
    }

    fn full_elem(inner: &'a RopeElem) -> Self {
        match inner {
            RopeElem::Owned(v) => RopeReaderState {
                inner: RopeReaderStateElem::Owned(v),
                index: 0,
            },
            RopeElem::Borrowed(v) => RopeReaderState::full_rope(v),
        }
    }
}

impl<'a> Read for RopeReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Ok(self.read_internal(buf.len(), &mut Some(&mut ReadBuf::new(buf))))
    }
}

impl<'a> AsyncRead for RopeReader<'a> {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.read_internal(buf.remaining(), &mut Some(buf));
        Poll::Ready(Ok(()))
    }
}
