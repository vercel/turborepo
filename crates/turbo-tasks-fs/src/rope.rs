use std::{
    borrow::Cow,
    cmp::min,
    hash::{Hash, Hasher},
    io::{self, Read, Result as IoResult, Write},
    ops,
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::io::{AsyncRead, ReadBuf};
use turbo_tasks_hash::{hash_xxh3_hash64, DeterministicHash, DeterministicHasher};

type Bytes = Vec<u8>;

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
#[derive(Clone, Debug)]
pub enum Rope {
    Flat(Arc<Bytes>),
    Concat(usize, Vec<Arc<Bytes>>),
}

use Rope::{Concat, Flat};

impl Rope {
    pub fn new(bytes: Bytes) -> Self {
        Flat(Arc::new(bytes))
    }

    pub fn flatten(&self) -> Cow<'_, Bytes> {
        match self {
            Rope::Flat(v) => Cow::Borrowed(v),
            Rope::Concat(..) => {
                let mut buf = Vec::with_capacity(self.len());
                self.flatten_internal(&mut buf);
                Cow::Owned(buf)
            }
        }
    }

    pub fn push_bytes(&mut self, bytes: Bytes) {
        match self {
            Flat(v) => match Arc::get_mut(v) {
                Some(flat) => flat.extend(bytes),

                None => {
                    let l = v.len() + bytes.len();
                    *self = Concat(l, vec![v.clone(), Arc::new(bytes)]);
                }
            },
            Concat(l, c) => {
                *l += bytes.len();
                let last = c.last_mut().expect("always have one");
                match Arc::get_mut(last) {
                    Some(last) => last.extend(bytes),
                    None => {
                        c.push(Arc::new(bytes));
                    }
                }
            }
        }
    }

    pub fn concat(&mut self, other: &Rope) {
        let l = self.len() + other.len();
        match self {
            Flat(left) => match other {
                Flat(right) => {
                    *self = Concat(l, vec![left.clone(), right.clone()]);
                }
                Concat(_, right) => {
                    let mut v = Vec::with_capacity(right.len() + 1);
                    v[0] = left.clone();
                    v.extend(right.clone());
                    *self = Concat(l, v);
                }
            },

            Concat(_, left) => match other {
                Flat(right) => {
                    left.push(right.clone());
                }
                Concat(_, right) => {
                    left.extend(right.clone());
                }
            },
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Flat(f) => f.len(),
            Concat(l, _) => *l,
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

    pub fn to_string(&self) -> Result<String> {
        let mut read = self.read();
        let mut string = String::new();
        <RopeReader as Read>::read_to_string(&mut read, &mut string)
            .map(|_| string)
            .context("failed to convert rope into string")
    }

    fn flatten_internal(&self, buf: &mut Bytes) {
        match self {
            Flat(v) => buf.extend(&**v),
            Concat(_, c) => {
                for v in c {
                    buf.extend(&**v);
                }
            }
        }
    }
}

impl Default for Rope {
    fn default() -> Self {
        vec![].into()
    }
}

impl From<Bytes> for Rope {
    fn from(bytes: Bytes) -> Self {
        Flat(Arc::new(bytes))
    }
}

impl From<&[u8]> for Rope {
    fn from(content: &[u8]) -> Self {
        Flat(Arc::new(content.to_vec()))
    }
}

impl From<&str> for Rope {
    fn from(content: &str) -> Self {
        Flat(Arc::new(content.as_bytes().to_vec()))
    }
}

impl From<String> for Rope {
    fn from(content: String) -> Self {
        Flat(Arc::new(content.into_bytes()))
    }
}

impl Write for Rope {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.push_bytes(bytes.to_owned());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl ops::AddAssign<&str> for Rope {
    fn add_assign(&mut self, rhs: &str) {
        self.push_bytes(rhs.as_bytes().to_vec());
    }
}

impl DeterministicHash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn deterministic_hash<H: DeterministicHasher>(&self, state: &mut H) {
        match self {
            Flat(f) => state.write_bytes(f.as_slice()),

            Concat(_, c) => {
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
            Flat(f) => state.write(f.as_slice()),
            Concat(_, c) => {
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

/// Ropes are always serialized into flat strings, because deserialization won't
/// deduplicate and share the ARCs (being the only possible owner of a bunch
/// doesn't make sense).
impl Serialize for Rope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let mut s = String::new();
        self.read().read_to_string(&mut s).map_err(Error::custom)?;
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for Rope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8>>::deserialize(deserializer)?;
        Ok(Rope::new(bytes))
    }
}

pub struct RopeReader<'a> {
    rope: &'a Rope,
    byte_pos: usize,
    concat_index: usize,
    max_bytes: usize,
}

impl<'a> RopeReader<'a> {
    fn new_full(rope: &'a Rope) -> Self {
        RopeReader {
            rope,
            byte_pos: 0,
            concat_index: 0,
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
            let el = match self.rope {
                Flat(el) => {
                    if self.concat_index > 0 {
                        break;
                    }
                    el
                }

                Concat(_, c) => match c.get(self.concat_index) {
                    Some(el) => el,
                    None => break,
                },
            };

            let got = self.read_bytes(el, remaining, buf);
            if got == 0 {
                break;
            }
            remaining -= got;
            self.max_bytes -= got;
        }
        want - remaining
    }

    fn read_bytes(
        &mut self,
        bytes: &Vec<u8>,
        remaining: usize,
        buf: &mut Option<&mut ReadBuf<'_>>,
    ) -> usize {
        let pos = self.byte_pos;
        let amount = min(min(bytes.len() - pos, remaining), self.max_bytes);
        let end = pos + amount;

        if end == bytes.len() {
            self.byte_pos = 0;
            self.concat_index += 1;
        } else {
            self.byte_pos = end;
        }

        if let Some(buf) = buf.as_mut() {
            buf.put_slice(&bytes[pos..end]);
        }
        amount
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
