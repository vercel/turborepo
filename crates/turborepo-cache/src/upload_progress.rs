use std::{
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
    time::Instant,
};

use futures::Stream;
use pin_project::pin_project;

type State<const BUCKETS: usize> = Mutex<(usize, [(usize, usize); BUCKETS])>;

/// Consists of a total file upload time and a ring buffer of bytes sent per
/// second over some time interval.
#[pin_project]
pub struct UploadProgress<const BUCKETS: usize, const INTERVAL: usize, S: Stream> {
    /// A pair of bucket generation and bytes uploaded in that bucket.
    ///
    /// We need to store the generation to ensure that we don't accidentally
    /// read from an expired bucket if there is a gap in writing.
    state: Arc<State<BUCKETS>>,
    start: Instant,
    #[pin]
    inner: S,
}

impl<const BUCKETS: usize, const INTERVAL: usize, S: Stream> UploadProgress<BUCKETS, INTERVAL, S> {
    /// Create a new `UploadProgress` with the given stream and interval.
    pub fn new(inner: S, size: Option<usize>) -> (Self, UploadProgressQuery<BUCKETS, INTERVAL>) {
        let state = Arc::new(Mutex::new((0, [(0, 0); BUCKETS])));
        let now = Instant::now();
        let query = UploadProgressQuery::new(now, Arc::downgrade(&state), size);

        (
            Self {
                state,
                start: now,
                inner,
            },
            query,
        )
    }
}

impl<const BUCKETS: usize, const INTERVAL: usize, S: Stream> Stream
    for UploadProgress<BUCKETS, INTERVAL, S>
where
    S::Item: ProgressLen,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().project();
        match this.inner.poll_next(cx) {
            Poll::Ready(Some(item)) => {
                // same as `curr_gen_index` but we can't borrow `self` twice
                let (curr_gen, index) = {
                    // usize fits 570 million years of milliseconds since start on 64 bit
                    let gen = (this.start.elapsed().as_millis() as usize) / INTERVAL;
                    (gen, gen % BUCKETS)
                };
                let mut state = this.state.lock().unwrap();
                let (gen, value) = &mut state.1[index];
                if *gen != curr_gen {
                    *gen = curr_gen;
                    *value = item.len();
                } else {
                    *value += item.len();
                }

                state.0 += item.len();

                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

trait ProgressLen {
    fn len(&self) -> usize;
}

impl ProgressLen for bytes::Bytes {
    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: ProgressLen, E> ProgressLen for Result<T, E> {
    fn len(&self) -> usize {
        match self {
            Ok(t) => t.len(),
            Err(_) => 0,
        }
    }
}

#[derive(Clone)]
pub struct UploadProgressQuery<const BUCKETS: usize, const INTERVAL: usize> {
    start: Instant,
    state: Weak<State<BUCKETS>>,
    size: Option<usize>,
}

impl<const BUCKETS: usize, const INTERVAL: usize> UploadProgressQuery<BUCKETS, INTERVAL> {
    fn new(start: Instant, state: Weak<State<BUCKETS>>, size: Option<usize>) -> Self {
        Self { start, state, size }
    }

    // Note: this usize is since the upload started so, on 64 bit systems, it
    // should be good for 584.5 million years. Downcasting is probably safe...
    fn curr_gen(&self) -> usize {
        let since = self.start.elapsed().as_millis() as usize;
        since / self.interval_ms()
    }

    pub const fn interval_ms(&self) -> usize {
        INTERVAL
    }

    /// Get the total number of bytes uploaded.
    ///
    /// Returns `None` if the `UploadProgress` has been dropped.
    pub fn bytes(&self) -> Option<usize> {
        self.state.upgrade().map(|s| s.lock().unwrap().0)
    }

    pub fn size(&self) -> Option<usize> {
        self.size
    }

    pub fn done(&self) -> bool {
        self.state.strong_count() == 0
    }

    /// Get the average bytes per second over the last `SIZE` intervals.
    ///
    /// Returns `None` if the `UploadProgress` has been dropped.
    pub fn average_bps(&self) -> Option<f64> {
        let curr_gen = self.curr_gen();
        let min_gen = curr_gen.saturating_sub(BUCKETS);
        self.state.upgrade().map(|s| {
            let s = s.lock().unwrap();
            let total_bytes =
                s.1.iter()
                    .filter(|(gen, _)| *gen >= min_gen)
                    .map(|(_, bytes)| *bytes)
                    .sum::<usize>();

            // buckets * interval = milliseconds, so we multiply by 1000 to get seconds
            (total_bytes as f64 / (BUCKETS * INTERVAL) as f64) * 1000.0
        })
    }
}
