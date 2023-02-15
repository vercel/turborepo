use std::{future::Future, pin::Pin};

use anyhow::Result;
use futures::{stream::FuturesUnordered, Stream};
use pin_project_lite::pin_project;

use super::{graph_store::GraphStore, GetChildren};

/// [`GraphTraversal`] is a utility type that can be used to traverse a graph of
/// nodes, where each node can have a variable number of children. The traversal
/// is done in parallel, and the order of the nodes in the traversal result is
/// determined by the [`GraphStore`] parameter.
pub struct GraphTraversal<S> {
    _store: std::marker::PhantomData<S>,
}

impl<S> GraphTraversal<S> {
    /// Visits the graph starting from the given `roots`, and returns a future
    /// that will resolve to the traversal result.
    pub fn visit<T, I, C, A, Impl>(
        roots: I,
        mut get_children: C,
    ) -> GraphTraversalFuture<T, S, C, A, Impl>
    where
        S: GraphStore<T>,
        I: IntoIterator<Item = T>,
        C: GetChildren<T, A, Impl>,
    {
        let mut store = S::default();
        let futures = FuturesUnordered::new();
        for item in roots {
            let (parent_handle, item) = store.insert(None, item);
            match get_children.get_children(item) {
                GraphTraversalControlFlow::Continue(future) => {
                    futures.push(WithHandle::new(future, parent_handle));
                }
                GraphTraversalControlFlow::Abort(abort) => {
                    return GraphTraversalFuture {
                        state: GraphTraversalState::Aborted { abort },
                    };
                }
                GraphTraversalControlFlow::Skip => {}
            }
        }
        GraphTraversalFuture {
            state: GraphTraversalState::Running(GraphTraversalRunningState {
                store,
                futures,
                get_children,
            }),
        }
    }
}

/// A future that resolves to a [`GraphStore`] containing the result of a graph
/// traversal.
pub struct GraphTraversalFuture<T, S, C, A, Impl>
where
    S: GraphStore<T>,
    C: GetChildren<T, A, Impl>,
{
    state: GraphTraversalState<T, S, C, A, Impl>,
}

enum GraphTraversalState<T, S, C, A, Impl>
where
    S: GraphStore<T>,
    C: GetChildren<T, A, Impl>,
{
    Completed,
    Aborted { abort: A },
    Running(GraphTraversalRunningState<T, S, C, A, Impl>),
}

struct GraphTraversalRunningState<T, S, C, A, Impl>
where
    S: GraphStore<T>,
    C: GetChildren<T, A, Impl>,
{
    store: S,
    futures: FuturesUnordered<WithHandle<C::Future, S::Handle>>,
    get_children: C,
}

impl<T, S, C, A, Impl> Default for GraphTraversalState<T, S, C, A, Impl>
where
    S: GraphStore<T>,
    C: GetChildren<T, A, Impl>,
{
    fn default() -> Self {
        GraphTraversalState::Completed
    }
}

pub enum GraphTraversalResult<R, A> {
    Completed(R),
    Aborted(A),
}

impl<R> GraphTraversalResult<R, !> {
    pub fn completed(self) -> R {
        match self {
            GraphTraversalResult::Completed(result) => result,
            GraphTraversalResult::Aborted(_) => unreachable!("the type parameter `A` is `!`"),
        }
    }
}

impl<T, S, C, A, Impl> Future for GraphTraversalFuture<T, S, C, A, Impl>
where
    S: GraphStore<T>,
    C: GetChildren<T, A, Impl>,
{
    type Output = GraphTraversalResult<Result<S>, A>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };

        let result;
        (this.state, result) = match std::mem::take(&mut this.state) {
            GraphTraversalState::Completed => {
                panic!("polled after completion")
            }
            GraphTraversalState::Aborted { abort } => (
                GraphTraversalState::Completed,
                std::task::Poll::Ready(GraphTraversalResult::Aborted(abort)),
            ),
            GraphTraversalState::Running(mut running) => 'outer: loop {
                let futures_pin = unsafe { Pin::new_unchecked(&mut running.futures) };
                match futures_pin.poll_next(cx) {
                    std::task::Poll::Ready(Some((parent_handle, Ok(children)))) => {
                        for item in children {
                            let (child_handle, item) =
                                running.store.insert(Some(parent_handle.clone()), item);

                            match running.get_children.get_children(item) {
                                GraphTraversalControlFlow::Continue(future) => {
                                    running.futures.push(WithHandle::new(future, child_handle));
                                }
                                GraphTraversalControlFlow::Skip => {}
                                GraphTraversalControlFlow::Abort(abort) => {
                                    break 'outer (
                                        GraphTraversalState::Completed,
                                        std::task::Poll::Ready(GraphTraversalResult::Aborted(
                                            abort,
                                        )),
                                    );
                                }
                            }
                        }
                    }
                    std::task::Poll::Ready(Some((_, Err(err)))) => {
                        break (
                            GraphTraversalState::Completed,
                            std::task::Poll::Ready(GraphTraversalResult::Completed(Err(err))),
                        );
                    }
                    std::task::Poll::Ready(None) => {
                        break (
                            GraphTraversalState::Completed,
                            std::task::Poll::Ready(GraphTraversalResult::Completed(Ok(
                                running.store
                            ))),
                        );
                    }
                    std::task::Poll::Pending => {
                        break (
                            GraphTraversalState::Running(running),
                            std::task::Poll::Pending,
                        );
                    }
                }
            },
        };

        result
    }
}

pin_project! {
    struct WithHandle<T, P>
    where
        T: Future,
    {
        #[pin]
        future: T,
        handle: Option<P>,
    }
}

impl<T, H> WithHandle<T, H>
where
    T: Future,
{
    pub fn new(future: T, handle: H) -> Self {
        Self {
            future,
            handle: Some(handle),
        }
    }
}

impl<T, H> Future for WithHandle<T, H>
where
    T: Future,
{
    type Output = (H, T::Output);

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.project();
        match this.future.poll(cx) {
            std::task::Poll::Ready(result) => std::task::Poll::Ready((
                this.handle.take().expect("polled after completion"),
                result,
            )),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

pub enum GraphTraversalControlFlow<F, A = !> {
    /// The traversal should continue with the given future.
    Continue(F),
    /// The traversal should abort and return immediately.
    Abort(A),
    /// The traversal should skip the current node.
    Skip,
}
