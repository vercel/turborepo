use std::{
    cmp::min,
    fmt::Debug,
    hash::{Hash, Hasher},
    io::{self, Read, Result as IoResult, Write},
    mem, ops,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use futures::Stream;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::io::{AsyncRead, ReadBuf};
use turbo_tasks_hash::{hash_xxh3_hash64, DeterministicHash, DeterministicHasher};

#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
#[derive(Clone, Debug)]
pub struct Rope {
    length: usize,

    #[turbo_tasks(debug_ignore, trace_ignore)]
    frozen: Vec<Bytes>,

    #[turbo_tasks(debug_ignore, trace_ignore)]
    mutable: Option<BytesMut>,
}

impl Rope {
    pub fn new(bytes: Vec<u8>) -> Self {
        Rope {
            length: bytes.len(),
            frozen: vec![bytes.into()],
            mutable: Some(BytesMut::default()),
        }
    }

    pub fn empty() -> Self {
        Rope {
            length: 0,
            frozen: vec![],
            mutable: Some(BytesMut::default()),
        }
    }

    pub fn frozen(bytes: Vec<u8>) -> Self {
        Rope {
            length: bytes.len(),
            frozen: vec![bytes.into()],
            mutable: None,
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        debug_assert!(self.mutable.is_some(), "rope is already frozen");
        self.length += bytes.len();
        self.mutable.as_mut().unwrap().extend_from_slice(bytes);
    }

    pub fn concat(&mut self, other: &Rope) {
        debug_assert!(self.mutable.is_some(), "rope is already frozen");
        debug_assert!(
            other.mutable.is_none(),
            "rope contains mutable data, must be frozen before reading"
        );

        self.length += other.len();
        let mutable = self.mutable.as_mut().unwrap();
        if !mutable.is_empty() {
            let mutable = mem::take(mutable);
            self.frozen.push(mutable.freeze());
        }
        self.frozen.extend(other.frozen.clone());
    }

    pub fn freeze(&mut self) {
        debug_assert!(self.mutable.is_some(), "rope is already frozen");

        if let Some(mutable) = self.mutable.take() {
            if !mutable.is_empty() {
                self.frozen.push(mutable.freeze());
            }
        }
    }

    pub fn len(&self) -> usize {
        self.length
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

    pub fn stream(&self) -> RopeStream {
        RopeStream::new(self)
    }

    pub fn to_string(&self) -> Result<String> {
        let mut read = self.read();
        let mut string = String::with_capacity(self.len());
        <RopeReader as Read>::read_to_string(&mut read, &mut string)
            .map(|_| string)
            .context("failed to convert rope into string")
    }
}

impl Default for Rope {
    fn default() -> Self {
        Rope::empty()
    }
}

impl From<&[u8]> for Rope {
    fn from(content: &[u8]) -> Self {
        Rope::frozen(content.to_vec())
    }
}

impl From<Vec<u8>> for Rope {
    fn from(content: Vec<u8>) -> Self {
        Rope::frozen(content)
    }
}

impl From<&str> for Rope {
    fn from(content: &str) -> Self {
        Rope::frozen(content.as_bytes().to_vec())
    }
}

impl From<String> for Rope {
    fn from(content: String) -> Self {
        Rope::frozen(content.into_bytes())
    }
}

impl Write for Rope {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.push_bytes(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl ops::AddAssign<&str> for Rope {
    fn add_assign(&mut self, rhs: &str) {
        self.push_bytes(rhs.as_bytes());
    }
}

impl DeterministicHash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn deterministic_hash<H: DeterministicHasher>(&self, state: &mut H) {
        for v in &self.frozen {
            state.write_bytes(v.as_ref());
        }
        if let Some(m) = &self.mutable {
            state.write_bytes(m.as_ref());
        }
    }
}

impl Hash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn hash<H: Hasher>(&self, state: &mut H) {
        for v in &self.frozen {
            state.write(v.as_ref());
        }
        if let Some(m) = &self.mutable {
            state.write(m.as_ref());
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
        let s = self.to_string().map_err(Error::custom)?;
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for Rope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8>>::deserialize(deserializer)?;
        Ok(Rope::frozen(bytes))
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
        debug_assert!(
            self.rope.mutable.is_none(),
            "rope contains mutable data, must be frozen before reading"
        );

        let mut remaining = want;
        while remaining > 0 {
            let bytes = match self.rope.frozen.get(self.concat_index) {
                Some(el) => el,
                None => break,
            };

            let got = self.read_bytes(bytes, remaining, buf);
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
        bytes: &Bytes,
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

pub struct RopeStream {
    rope: Rope,
    concat_index: usize,
    size_hint: usize,
}

impl RopeStream {
    fn new(rope: &Rope) -> Self {
        RopeStream {
            rope: rope.clone(),
            concat_index: 0,
            size_hint: rope.len(),
        }
    }
}

impl Stream for RopeStream {
    type Item = Result<Bytes>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        debug_assert!(
            this.rope.mutable.is_none(),
            "rope contains mutable data, must be frozen before reading"
        );

        match this.rope.frozen.get(this.concat_index) {
            None => Poll::Ready(None),
            Some(bytes) => {
                this.concat_index += 1;
                this.size_hint -= bytes.len();

                Poll::Ready(Some(Ok(bytes.clone())))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.size_hint, Some(self.size_hint))
    }
}
