use std::{
    future::{Future, IntoFuture},
    pin::Pin,
    task::ready,
};

use anyhow::Result;
use futures::{
    future::{join_all, JoinAll},
    stream::FuturesUnordered,
    FutureExt, Stream,
};

/// Future for the [JoinIterExt::join] method.
pub struct Join<F>
where
    F: Future,
{
    inner: JoinAll<F>,
}

impl<T, F> Future for Join<F>
where
    T: Unpin,
    F: Future<Output = T>,
{
    type Output = Vec<T>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.inner.poll_unpin(cx)
    }
}

/// Future for the [TryJoinIterExt::try_join] method.
pub struct TryJoin<F>
where
    F: Future,
{
    inner: JoinAll<F>,
}

impl<T, F> Future for TryJoin<F>
where
    T: Unpin,
    F: Future<Output = Result<T>>,
{
    type Output = Result<Vec<T>>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.inner.poll_unpin(cx) {
            std::task::Poll::Ready(res) => {
                std::task::Poll::Ready(res.into_iter().collect::<Result<Vec<_>>>())
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

pub struct TryFlatMapRecursiveJoin<T, C, F, CI>
where
    C: FnMut(&T) -> Option<F>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    output: Vec<T>,
    futures: FuturesUnordered<F>,
    filter_flat_map: C,
}

impl<T, C, F, CI> Future for TryFlatMapRecursiveJoin<T, C, F, CI>
where
    C: FnMut(&T) -> Option<F>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Output = Result<Vec<T>>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        loop {
            let futures = unsafe { Pin::new_unchecked(&mut this.futures) };
            if let Some(result) = ready!(futures.poll_next(cx)) {
                match result {
                    Ok(children) => {
                        for item in children {
                            match (this.filter_flat_map)(&item) {
                                Some(future) => {
                                    this.futures.push(future);
                                }
                                None => {}
                            }
                            this.output.push(item);
                        }
                    }
                    Err(err) => return std::task::Poll::Ready(Err(err)),
                }
            } else {
                return std::task::Poll::Ready(Ok(std::mem::take(&mut this.output)));
            }
        }
    }
}

pub trait JoinIterExt<T, F>: Iterator
where
    T: Unpin,
    F: Future<Output = T>,
{
    /// Returns a future that resolves to a vector of the outputs of the futures
    /// in the iterator.
    fn join(self) -> Join<F>;
}

pub trait TryJoinIterExt<T, F>: Iterator
where
    T: Unpin,
    F: Future<Output = Result<T>>,
{
    /// Returns a future that resolves to a vector of the outputs of the futures
    /// in the iterator, or to an error if one of the futures fail.
    ///
    /// Unlike `Futures::future::try_join_all`, this returns the Error that
    /// occurs first in the list of futures, not the first to fail in time.
    fn try_join(self) -> TryJoin<F>;
}

pub trait TryFlatMapRecursiveJoinIterExt<T, C, F, CI>: Iterator
where
    C: FnMut(&T) -> Option<F>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    /// Applies the `filter_flat_map` function on each item in the iterator, and
    /// on each item that is returned by `filter_flat_map`, recursively.
    ///
    /// Collects all items from the iterator and all items returns by
    /// `filter_flat_map` into a vector.
    ///
    /// `filter_flat_map` will execute concurrently
    ///
    /// **Beware:**
    /// * The order of the returned items is undefined.
    /// * Circular references must be handled within `filter_flat_map`: return
    ///   `None` to stop the recursion.
    ///
    /// Returns a future that resolve to a [Result<Vec<T>>]. It will
    /// resolve to the first error that occurs.
    fn try_flat_map_recursive_join(
        self,
        filter_flat_map: C,
    ) -> TryFlatMapRecursiveJoin<T, C, F, CI>;
}

impl<T, F, IF, It> JoinIterExt<T, F> for It
where
    T: Unpin,
    F: Future<Output = T>,
    IF: IntoFuture<Output = T, IntoFuture = F>,
    It: Iterator<Item = IF>,
{
    fn join(self) -> Join<F> {
        Join {
            inner: join_all(self.map(|f| f.into_future())),
        }
    }
}

impl<T, F, IF, It> TryJoinIterExt<T, F> for It
where
    T: Unpin,
    F: Future<Output = Result<T>>,
    IF: IntoFuture<Output = Result<T>, IntoFuture = F>,
    It: Iterator<Item = IF>,
{
    fn try_join(self) -> TryJoin<F> {
        TryJoin {
            inner: join_all(self.map(|f| f.into_future())),
        }
    }
}

impl<T, C, F, CI, It> TryFlatMapRecursiveJoinIterExt<T, C, F, CI> for It
where
    C: FnMut(&T) -> Option<F>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
    It: Iterator<Item = T>,
{
    fn try_flat_map_recursive_join(
        self,
        mut filter_flat_map: C,
    ) -> TryFlatMapRecursiveJoin<T, C, F, CI> {
        let futures = FuturesUnordered::new();
        let mut output = Vec::new();
        for item in self {
            match filter_flat_map(&item) {
                Some(future) => {
                    futures.push(future);
                }
                None => {}
            }
            output.push(item);
        }
        TryFlatMapRecursiveJoin {
            output,
            futures,
            filter_flat_map,
        }
    }
}
