use std::{
    cmp::min,
    fmt::Debug,
    hash::{Hash, Hasher},
    io::{self, Read, Result as IoResult, Write},
    mem::size_of,
    ops,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use futures::Stream;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::io::{AsyncRead, ReadBuf};
use turbo_tasks_hash::{hash_xxh3_hash64, DeterministicHash, DeterministicHasher};

/// A Rope provides an efficient structure for sharing bytes/strings between
/// multiple sources. Cloning a Rope is cheapish (clone of Vec<4 usizes>), and
/// the contents of one Rope can be shared with another using that clone.
///
/// Ropes are immutable, in order to construct one see [RopeBuilder].
#[turbo_tasks::value(shared, serialization = "none", eq = "manual")]
#[derive(Clone, Debug, Default)]
pub struct Rope {
    /// Total length of all contained bytes.
    length: usize,

    /// Stores all bytes committed to this rope.
    #[turbo_tasks(debug_ignore, trace_ignore)]
    data: Vec<Bytes>,
}

/// RopeBuilder provides a mutable container to append bytes/strings. This can
/// also append _other_ Rope instances cheaply, allowing efficient sharing of
/// the contents without a full clone of the bytes.
#[derive(Default)]
pub struct RopeBuilder {
    /// Total length of all prevoiusly committed bytes.
    length: usize,

    /// Immutable bytes references that have been appended to this builder. The
    /// rope's is the combination of all these committed bytes.
    committed: Vec<Bytes>,

    /// Mutable bytes collection where non-static/non-shared bytes are written.
    /// This builds until the next time a static or shared bytes is
    /// appended, in which case we split the buffer and commit. Finishing
    /// the builder also commits these bytes.
    writable: BytesMut,
}

impl Rope {
    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Returns a Read/AsyncRead instance over all bytes.
    pub fn read(&'_ self) -> RopeReader<'_> {
        self.slice(0, self.len())
    }

    /// Returns a Read/AsyncRead instance over a slice of bytes.
    pub fn slice(&'_ self, start: usize, end: usize) -> RopeReader<'_> {
        RopeReader::new(self, start, end)
    }

    /// Returns Stream instance over all bytes.
    ///
    /// This is different than a Read, as we return our bytes references
    /// direclty intead of reading them into a provided buffer.
    pub fn stream(&self) -> RopeStream {
        RopeStream::new(self)
    }

    // TODO
    /// Returns a String instance of all bytes.
    pub fn to_string(&self) -> Result<String> {
        let mut read = self.read();
        let mut string = String::with_capacity(self.len());
        <RopeReader as Read>::read_to_string(&mut read, &mut string)
            .map(|_| string)
            .context("failed to convert rope into string")
    }
}

impl From<&'static [u8]> for Rope {
    fn from(content: &'static [u8]) -> Self {
        Rope {
            length: content.len(),
            data: vec![Bytes::from_static(content)],
        }
    }
}

impl From<&'static str> for Rope {
    fn from(content: &'static str) -> Self {
        Rope::from(content.as_bytes())
    }
}

impl From<Vec<u8>> for Rope {
    fn from(bytes: Vec<u8>) -> Self {
        Rope {
            length: bytes.len(),
            data: vec![bytes.into()],
        }
    }
}

impl From<String> for Rope {
    fn from(content: String) -> Self {
        Rope::from(content.into_bytes())
    }
}

impl RopeBuilder {
    /// Push owned bytes into the Rope.
    ///
    /// If possible use [push_static_bytes] or `+=` operation instead, as they
    /// will create a reference to shared memory instead of cloning the
    /// bytes.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.length += bytes.len();
        self.writable.extend_from_slice(bytes);
    }

    /// Push static bytes into the Rope.
    pub fn push_static_bytes(&mut self, bytes: &'static [u8]) {
        // If the string is smaller than the cost of a Bytes reference (4 usizes), then
        // it's more efficient to own the bytes in a new buffer. We may be able to reuse
        // that buffer when more bytes are pushed.
        if bytes.len() < size_of::<Bytes>() {
            return self.push_bytes(bytes);
        }

        // We may have pending bytes from a prior push.
        self.finish();

        self.length += bytes.len();
        self.committed.push(Bytes::from_static(bytes));
    }

    /// Concatenate another Rope instance into our builder. This is much more
    /// effeicient than pushing actual bytes, since we can share the other
    /// Rope's references without copying the underlying data.
    pub fn concat(&mut self, other: &Rope) {
        // We may have pending bytes from a prior push.
        self.finish();

        self.length += other.len();
        self.committed.extend(other.data.clone());
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Writes any pending bytes into our committed queue.
    ///
    /// This may be called multiple times without issue.
    pub fn finish(&mut self) {
        if !self.writable.is_empty() {
            self.committed.push(self.writable.split().freeze());
        }
    }

    /// Constructs our final, immutable Rope instance.
    pub fn build(mut self) -> Rope {
        self.finish();
        Rope {
            length: self.length,
            data: self.committed,
        }
    }
}

impl From<Vec<u8>> for RopeBuilder {
    fn from(bytes: Vec<u8>) -> Self {
        RopeBuilder {
            length: bytes.len(),
            committed: vec![],
            writable: bytes.as_slice().into(),
        }
    }
}

impl Write for RopeBuilder {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.push_bytes(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        self.finish();
        Ok(())
    }
}

impl ops::AddAssign<&'static str> for RopeBuilder {
    fn add_assign(&mut self, rhs: &'static str) {
        self.push_static_bytes(rhs.as_bytes());
    }
}

impl DeterministicHash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn deterministic_hash<H: DeterministicHasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        for v in &self.data {
            state.write_bytes(v.as_ref());
        }
    }
}

impl Hash for Rope {
    /// Ropes with similar contents hash the same, regardless of their
    /// structure.
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        for v in &self.data {
            state.write(v.as_ref());
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

impl Serialize for Rope {
    /// Ropes are always serialized into contiguous strings, because
    /// deserialization won't deduplicate and share the Arcs (being the only
    /// possible owner of a individual "shared" data doesn't make sense).
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let s = self.to_string().map_err(Error::custom)?;
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for Rope {
    /// Deserializes strings into a contiguous, immutable Rope.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8>>::deserialize(deserializer)?;
        Ok(Rope::from(bytes))
    }
}

/// Implements the Read/AsyncRead trait over a Rope.
pub struct RopeReader<'a> {
    data: &'a Vec<Bytes>,
    byte_pos: usize,
    concat_index: usize,
    max_bytes: usize,
}

impl<'a> RopeReader<'a> {
    fn new(rope: &'a Rope, start: usize, end: usize) -> Self {
        let mut reader = RopeReader {
            data: &rope.data,
            byte_pos: 0,
            concat_index: 0,
            max_bytes: end,
        };

        if start > 0 {
            reader.read_internal(start, &mut None);
        }

        reader
    }

    /// A shared implemenation for reading bytes. This takes the basic
    /// operations needed for both Read and AsyncRead.
    fn read_internal(&mut self, want: usize, buf: &mut Option<&mut ReadBuf<'_>>) -> usize {
        let mut remaining = want;
        while remaining > 0 {
            let bytes = match self.data.get(self.concat_index) {
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

    /// A helper to isolate how many bytes we can read and copying the dat over.
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
    /// Reads the Rope into the provided buffer.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Ok(self.read_internal(buf.len(), &mut Some(&mut ReadBuf::new(buf))))
    }
}

impl<'a> AsyncRead for RopeReader<'a> {
    /// Reads the Rope into the provided buffer, asynchronously.
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

/// Implements the Stream trait over a Rope.
pub struct RopeStream {
    data: Vec<Bytes>,
    concat_index: usize,
    size_hint: usize,
}

impl RopeStream {
    fn new(rope: &Rope) -> Self {
        RopeStream {
            data: rope.data.clone(),
            concat_index: 0,
            size_hint: rope.len(),
        }
    }
}

impl Stream for RopeStream {
    /// The Result<Bytes> item type is required for this to be streamable into a
    /// [Hyper::Body].
    type Item = Result<Bytes>;

    /// Returns a "result" of reading the next shared bytes reference. This differes from
    /// [Read::read] by not copying any memory.
    fn poll_next(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.data.get(this.concat_index) {
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
