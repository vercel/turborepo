use std::{
    fmt, mem,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
};

use anyhow::Result;
use futures::{Stream as StreamTrait, StreamExt};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Streams allow for streaming values from source to sink.
///
/// A Stream implements both a reader (which implements the Stream trait), and a
/// writer (which can be sent to another thread). As new values are written, any
/// pending readers will be woken up to receive the new value.
#[derive(Debug)]
pub struct Stream<T> {
    inner: Arc<Mutex<StreamState<T>>>,
}

/// The StreamState actually holds the data of a Stream.
pub enum StreamState<T> {
    /// An OpenStream is tied directly to a source stream, and will lazily pull
    /// new values out as a reader reaches the end of our already-pulled
    /// data.
    OpenStream {
        source: Box<dyn StreamTrait<Item = T> + Send + Sync + Unpin>,
        pulled: Vec<T>,
    },

    /// A Closed stream state cannot be pushed to, so it's anyone polling can
    /// read all values at their leisure.
    Closed(Box<[T]>),
}

impl<T> Stream<T> {
    /// Constructs a new Stream, and immediately closes it with only the passed
    /// values.
    pub fn new_closed(data: Vec<T>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamState::Closed(data.into_boxed_slice()))),
        }
    }

    /// Returns a [StreamTrait] implementation to poll values out of our Stream.
    pub fn read(&self) -> StreamRead<T> {
        StreamRead {
            source: self.clone(),
            index: 0,
        }
    }

    /// Crates a new Stream, which will lazily pull from the source stream.
    pub fn from_stream<S: StreamTrait<Item = T> + Send + Sync + Unpin + 'static>(
        source: S,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamState::OpenStream {
                source: Box::new(source),
                pulled: vec![],
            })),
        }
    }
}

impl<T: Send + Sync + 'static> Stream<T> {
    /// Constructs a new Stream, and leaves it open for new values to be
    /// written.
    pub fn new_open(data: Vec<T>) -> (UnboundedSender<T>, Self) {
        let (sender, receiver) = unbounded_channel();
        (
            sender,
            Self {
                inner: Arc::new(Mutex::new(StreamState::OpenStream {
                    source: Box::new(ReceiverStream { receiver }),
                    pulled: data,
                })),
            },
        )
    }
}

impl<T> Clone for Stream<T> {
    // The derived Clone impl will only work if `T: Clone`, which is wrong because
    // we just need to clone the Arc, not the internal data.
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Default for Stream<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamState::Closed(Box::new([])))),
        }
    }
}

impl<T: PartialEq> PartialEq for Stream<T> {
    // A Stream is equal if its the same internal pointer, or both streams are
    // closed with equivalent values.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) || {
            let left = self.inner.lock().unwrap();
            let right = other.inner.lock().unwrap();

            match (&*left, &*right) {
                (StreamState::Closed(a), StreamState::Closed(b)) => a == b,
                _ => false,
            }
        }
    }
}
impl<T: Eq> Eq for Stream<T> {}

impl<T: Serialize> Serialize for Stream<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let lock = self.inner.lock().map_err(Error::custom)?;
        match &*lock {
            StreamState::Closed(data) => data.serialize(serializer),
            _ => Err(Error::custom("cannot serialize open stream")),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Stream<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = <Vec<T>>::deserialize(deserializer)?;
        Ok(Stream::new_closed(data))
    }
}

impl<T: fmt::Debug> fmt::Debug for StreamState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenStream { pulled: data, .. } => f
                .debug_struct("StreamState::OpenStream")
                .field("data", data)
                .finish(),
            Self::Closed(data) => f.debug_tuple("StreamState::Closed").field(data).finish(),
        }
    }
}

/// Implements [StreamTrait] over our Stream.
#[derive(Debug)]
pub struct StreamRead<T> {
    index: usize,
    source: Stream<T>,
}

impl<T: Clone> StreamTrait for StreamRead<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let index = this.index;
        let mut inner = this.source.inner.lock().unwrap();
        match &mut *inner {
            StreamState::OpenStream {
                source,
                pulled: data,
            } => match data.get(index) {
                // If the current reader can be satisfied by a value we've already pulled, then just
                // do that.
                Some(v) => {
                    this.index += 1;
                    Poll::Ready(Some(v.clone()))
                }
                None => match source.poll_next_unpin(cx) {
                    // Else, if the source stream is ready to give us a new value, we can
                    // immediately store that and return it to the caller. Any other readers will
                    // be able to read the value from the already-pulled data.
                    Poll::Ready(Some(v)) => {
                        this.index += 1;
                        data.push(v.clone());
                        Poll::Ready(Some(v))
                    }
                    // If the source stream is finished, then we can transition to the closed state
                    // to drop the source stream.
                    Poll::Ready(None) => {
                        let data = mem::take(data).into_boxed_slice();
                        *inner = StreamState::Closed(data);
                        Poll::Ready(None)
                    }
                    // Else, we need to wait for the source stream to give us a new value. The
                    // source stream will be responsible for waking the TaskContext.
                    Poll::Pending => Poll::Pending,
                },
            },

            // The Closed state is easiest. We either have the value at the index, or not, there is
            // no need to return pending.
            StreamState::Closed(data) => Poll::Ready(data.get(index).map(|v| {
                this.index += 1;
                v.clone()
            })),
        }
    }
}

/// A small wrapper around a channel Receiver which allows it to be used as a
/// source Stream.
struct ReceiverStream<T> {
    receiver: UnboundedReceiver<T>,
}

impl<T> StreamTrait for ReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
