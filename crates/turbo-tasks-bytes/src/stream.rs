use std::{
    fmt,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::{Context as TaskContext, Poll, Waker},
    vec,
};

use anyhow::Result;
use futures::{Stream as StreamTrait, StreamExt};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub struct Stream<T> {
    inner: Arc<Mutex<StreamInner<T>>>,
}

#[derive(Default)]
pub struct StreamInner<T> {
    closed: bool,
    data: Vec<T>,
    wakers: Vec<Waker>,
}

impl<T> Stream<T> {
    pub fn new_closed(data: Vec<T>) -> Self {
        Stream {
            inner: Arc::new(Mutex::new(StreamInner {
                closed: false,
                data,
                wakers: vec![],
            })),
        }
    }

    pub fn new_open(data: Vec<T>) -> Self {
        Stream {
            inner: Arc::new(Mutex::new(StreamInner {
                closed: false,
                data,
                wakers: vec![],
            })),
        }
    }

    pub fn read(&self) -> StreamRead<T> {
        StreamRead {
            source: self.clone(),
            index: 0,
        }
    }

    pub fn write(&self) -> StreamWrite<T> {
        StreamWrite {
            source: self.clone(),
        }
    }
}

impl<T: Send + Sync + 'static> Stream<T> {
    pub fn from_stream<S: StreamTrait<Item = T> + Send + Sync + 'static>(input: S) -> Self {
        let stream = Stream {
            inner: Arc::new(Mutex::new(StreamInner {
                closed: false,
                data: vec![],
                wakers: vec![],
            })),
        };

        let writer = stream.write();
        tokio::spawn(async move {
            let mut input = Box::pin(input);
            loop {
                let n = input.next().await;
                match n {
                    None => {
                        let mut lock = writer.lock().unwrap();
                        lock.close();
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
        Stream {
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
        Stream {
            inner: Arc::new(Mutex::new(StreamInner {
                closed: true,
                data: vec![],
                wakers: vec![],
            })),
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
        if !lock.closed {
            return Err(Error::custom("cannot serialize open stream"));
        }
        lock.data.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Stream<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = <Vec<T>>::deserialize(deserializer)?;
        Ok(Stream::new_closed(data))
    }
}

impl<T> StreamInner<T> {
    pub fn push(&mut self, value: T) {
        debug_assert!(!self.closed, "cannot push to closed StreamInner");
        self.data.push(value);
        self.wake();
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.wake();
    }

    fn wake(&mut self) {
        for w in self.wakers.drain(0..) {
            w.wake();
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for StreamInner<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamInner")
            .field("closed", &self.closed)
            .field("data", &self.data)
            .finish()
    }
}

impl<T: PartialEq> PartialEq for StreamInner<T> {
    fn eq(&self, other: &Self) -> bool {
        self.closed == other.closed && self.data == other.data
    }
}
impl<T: Eq> Eq for StreamInner<T> {}

pub struct StreamRead<T> {
    index: usize,
    source: Stream<T>,
}

impl<T: Clone> StreamTrait for StreamRead<T> {
    // The Result<Bytes> item type is required for this to be streamable into a
    // [Hyper::Body].
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut source = this.source.inner.lock().unwrap();
        match source.data.get(this.index) {
            Some(data) => {
                this.index += 1;
                Poll::Ready(Some(data.clone()))
            }
            None => {
                if source.closed {
                    Poll::Ready(None)
                } else {
                    source.wakers.push(cx.waker().clone());
                    Poll::Pending
                }
            }
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

#[derive(Clone)]
pub struct StreamWrite<T> {
    source: Stream<T>,
}

impl<T> StreamWrite<T> {
    pub fn lock(
        &self,
    ) -> Result<MutexGuard<'_, StreamInner<T>>, PoisonError<MutexGuard<'_, StreamInner<T>>>> {
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
