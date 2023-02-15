use std::{collections::HashSet, future::Future};

use anyhow::Result;

use super::GraphTraversalControlFlow;

/// A trait that allows a graph traversal to get the children of a node.
pub trait GetChildren<T, A = !, Impl = ()> {
    type Children: IntoIterator<Item = T>;
    type Future: Future<Output = Result<Self::Children>>;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future, A>;
}

// The different `Impl*` here are necessary in order to avoid the `Conflicting
// implementations of trait` error when implementing `GetChildren` on different
// kinds of `FnMut`.
// See https://users.rust-lang.org/t/conflicting-implementation-when-implementing-traits-for-fn/53359/3

pub struct ImplRef;

impl<T, C, F, CI> GetChildren<T, !, ImplRef> for C
where
    C: FnMut(&T) -> F,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Children = CI;
    type Future = F;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future> {
        GraphTraversalControlFlow::Continue((self)(item))
    }
}

pub struct ImplRefOption;

impl<T, C, F, CI> GetChildren<T, !, ImplRefOption> for C
where
    C: FnMut(&T) -> Option<F>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Children = CI;
    type Future = F;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future> {
        match (self)(item) {
            Some(future) => GraphTraversalControlFlow::Continue(future),
            None => GraphTraversalControlFlow::Skip,
        }
    }
}

pub struct ImplRefControlFlow;

impl<T, C, F, CI, A> GetChildren<T, A, ImplRefControlFlow> for C
where
    T: Clone,
    C: FnMut(&T) -> GraphTraversalControlFlow<F, A>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Children = CI;
    type Future = F;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future, A> {
        (self)(item)
    }
}

pub struct ImplValue;

impl<T, C, F, CI> GetChildren<T, !, ImplValue> for C
where
    T: Clone,
    C: FnMut(T) -> F,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Children = CI;
    type Future = F;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future> {
        GraphTraversalControlFlow::Continue((self)(item.clone()))
    }
}

pub struct ImplValueOption;

impl<T, C, F, CI> GetChildren<T, !, ImplValueOption> for C
where
    T: Clone,
    C: FnMut(T) -> Option<F>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Children = CI;
    type Future = F;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future> {
        match (self)(item.clone()) {
            Some(future) => GraphTraversalControlFlow::Continue(future),
            None => GraphTraversalControlFlow::Skip,
        }
    }
}

pub struct ImplValueControlFlow;

impl<T, C, F, CI, A> GetChildren<T, A, ImplValueControlFlow> for C
where
    T: Clone,
    C: FnMut(T) -> GraphTraversalControlFlow<F, A>,
    F: Future<Output = Result<CI>>,
    CI: IntoIterator<Item = T>,
{
    type Children = CI;
    type Future = F;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future, A> {
        (self)(item.clone())
    }
}

/// A [`GetChildren`] implementation that skips nodes that have already been
/// visited. This is necessary to avoid repeated work when traversing non-tree
/// graphs (i.e. where a child can have more than one parent).
#[derive(Debug)]
pub struct SkipDuplicates<T, C, A, Impl> {
    get_children: C,
    visited: HashSet<T>,
    _a: std::marker::PhantomData<A>,
    _impl: std::marker::PhantomData<Impl>,
}

impl<T, C, A, Impl> SkipDuplicates<T, C, A, Impl> {
    /// Create a new [`SkipDuplicates`] that wraps the given [`GetChildren`].
    pub fn new(get_children: C) -> Self {
        Self {
            get_children,
            visited: HashSet::new(),
            _a: std::marker::PhantomData,
            _impl: std::marker::PhantomData,
        }
    }
}

impl<T, C, A, Impl> GetChildren<T, A, Impl> for SkipDuplicates<T, C, A, Impl>
where
    T: Eq + std::hash::Hash + Clone,
    C: GetChildren<T, A, Impl>,
{
    type Children = C::Children;
    type Future = C::Future;

    fn get_children(&mut self, item: &T) -> GraphTraversalControlFlow<Self::Future, A> {
        if !self.visited.contains(item) {
            self.visited.insert(item.clone());
            self.get_children.get_children(item)
        } else {
            GraphTraversalControlFlow::Skip
        }
    }
}
