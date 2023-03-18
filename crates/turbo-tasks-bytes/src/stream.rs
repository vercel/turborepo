use std::{
    fmt, mem,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::{Context as TaskContext, Poll, Waker},
    vec,
};

use anyhow::Result;
use futures::{Stream as StreamTrait, StreamExt};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Streams allow for streaming values from source to sink.
///
/// A Stream implements both a reader (which implements the Stream trait), and a
/// writer (which can be cloned and sent to any thread). As new values are
/// written, any pending readers will be woken up to receive the new value.
pub struct Stream<T> {
    inner: Arc<Mutex<StreamState<T>>>,
}

/// The StreamState actually holds the data of a Stream, including any pending
/// threads that are pol polling for the next value.
pub enum StreamState<T> {
    /// An Open stream state can still be pushed to, so anyone polling may need
    /// to wait for new dat data.
    Open { data: Vec<T>, wakers: Vec<Waker> },

    /// A Closed stream state cannot be pushed to, so it's anyone polling can
    /// read all values at their leisure.
    Closed { data: Box<[T]> },
}

impl<T> Stream<T> {
    /// Constructs a new Stream, and immediately closes it with only the passed
    /// values.
    pub fn new_closed(data: Vec<T>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamState::Closed {
                data: data.into_boxed_slice(),
            })),
        }
    }

    /// Constructs a new Stream, and leaves it open for new values to be
    /// written.
    pub fn new_open(data: Vec<T>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamState::Open {
                data,
                wakers: vec![],
            })),
        }
    }

    /// Returns a [StreamTrait] implementation to poll values out of our Stream.
    pub fn read(&self) -> StreamRead<T> {
        StreamRead {
            source: self.clone(),
            index: 0,
        }
    }

    /// Returns a writing wrapper to allow pushing new values onto the Stream.
    pub fn write(&self) -> StreamWrite<T> {
        StreamWrite {
            source: self.clone(),
        }
    }
}

impl<T: Send + Sync + 'static> Stream<T> {
    /// Crates a new Stream, which will attempt to eagerly read all values from
    /// another stream.
    // TODO: this would be better if it was lazy on polling.
    pub fn from_stream<S: StreamTrait<Item = T> + Send + Sync + 'static>(input: S) -> Self {
        let stream = Stream::default();
        let writer = stream.write();
        tokio::spawn(async move {
            let mut input = Box::pin(input);
            loop {
                let n = input.next().await;
                match n {
                    None => {
                        let mut lock = writer.lock().unwrap();
                        lock.close(None);
                        break;
                    }
                    Some(v) => {
                        let mut lock = writer.lock().unwrap();
                        lock.push(v)
                    }
                }
            }
        });

        stream
    }
}

impl<T> Clone for Stream<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Stream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<T> Default for Stream<T> {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StreamState::default())),
        }
    }
}

impl<T: PartialEq> PartialEq for Stream<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) || {
            let this = self.inner.lock().unwrap();
            let other = other.inner.lock().unwrap();
            *this == *other
        }
    }
}
impl<T: Eq> Eq for Stream<T> {}

impl<T: Serialize> Serialize for Stream<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let lock = self.inner.lock().map_err(Error::custom)?;
        lock.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Stream<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = <Vec<T>>::deserialize(deserializer)?;
        Ok(Stream::new_closed(data))
    }
}

impl<T> StreamState<T> {
    /// Pushes a new value to the open Stream, waking any pending pollers.
    pub fn push(&mut self, value: T) {
        let Self::Open { data, wakers } = self else {
            panic!("cannot push to an already closed StreamState");
        };

        data.push(value);
        for w in wakers.drain(0..) {
            w.wake();
        }
    }

    /// Closes an open Stream, waking any pending pollers.
    pub fn close(&mut self, value: Option<T>) {
        let Self::Open { data, wakers } = self else {
            panic!("cannot close an already closed StreamState");
        };
        if let Some(value) = value {
            data.push(value);
        }
        let data = mem::take(data).into_boxed_slice();
        let wakers = mem::take(wakers);
        *self = Self::Closed { data };
        for w in wakers {
            w.wake();
        }
    }

    fn get(&mut self, index: usize) -> GetState<'_, T> {
        match self {
            Self::Open { data, wakers } => GetState::Open(data.get(index), wakers),
            Self::Closed { data } => GetState::Closed(data.get(index)),
        }
    }
}

impl<T> Default for StreamState<T> {
    fn default() -> Self {
        Self::Open {
            data: vec![],
            wakers: vec![],
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for StreamState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { data, wakers } => f
                .debug_struct("StreamState::Open")
                .field("data", data)
                .field("wakers", wakers)
                .finish(),
            Self::Closed { data } => f
                .debug_struct("StreamState::Closed")
                .field("data", data)
                .finish(),
        }
    }
}

impl<T: PartialEq> PartialEq for StreamState<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Open { data: a, .. }, Self::Open { data: b, .. }) => a == b,
            (Self::Closed { data: a }, Self::Closed { data: b }) => a == b,
            _ => false,
        }
    }
}
impl<T: Eq> Eq for StreamState<T> {}

impl<T: Serialize> Serialize for StreamState<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        match self {
            Self::Open { .. } => Err(Error::custom("cannot serialize open stream")),
            Self::Closed { data } => data.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for StreamState<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = <Box<[T]>>::deserialize(deserializer)?;
        Ok(StreamState::Closed { data })
    }
}

enum GetState<'a, T> {
    Open(Option<&'a T>, &'a mut Vec<Waker>),
    Closed(Option<&'a T>),
}

/// Implements [StreamTrait] over our Stream.
pub struct StreamRead<T> {
    index: usize,
    source: Stream<T>,
}

impl<T: Clone> StreamTrait for StreamRead<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut source = this.source.inner.lock().unwrap();
        match source.get(this.index) {
            GetState::Open(Some(data), _) | GetState::Closed(Some(data)) => {
                this.index += 1;
                Poll::Ready(Some(data.clone()))
            }
            GetState::Open(None, wakers) => {
                wakers.push(cx.waker().clone());
                Poll::Pending
            }
            GetState::Closed(None) => Poll::Ready(None),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for StreamRead<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamRead")
            .field("index", &self.index)
            .field("source", &self.source)
            .finish()
    }
}

/// Implements basic writing over our Stream.
#[derive(Clone)]
pub struct StreamWrite<T> {
    source: Stream<T>,
}

impl<T> StreamWrite<T> {
    pub fn lock(
        &self,
    ) -> Result<MutexGuard<'_, StreamState<T>>, PoisonError<MutexGuard<'_, StreamState<T>>>> {
        self.source.inner.lock()
    }
}

impl<T: fmt::Debug> fmt::Debug for StreamWrite<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamWrite")
            .field("source", &self.source)
            .finish()
    }
}
