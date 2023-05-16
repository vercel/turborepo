use std::future::Future;

use anyhow::Result;
use tracing::Span;

use super::VisitControlFlow;

/// A trait that allows a graph traversal to visit the edges of a node
/// transitively.
pub trait Visit<Node, Abort = !, Impl = ()> {
    type Edge;
    type EdgesIntoIter: IntoIterator<Item = Self::Edge>;
    type EdgesFuture: Future<Output = Result<Self::EdgesIntoIter>>;

    /// Visits an edge to get to the neighbor node. Should return a
    /// [`VisitControlFlow`] that indicates whether to:
    /// * continue visiting the neighbor node edges;
    /// * skip visiting the neighbor node's edges;
    /// * abort the traversal entirely.
    fn visit(&mut self, edge: Self::Edge) -> VisitControlFlow<Node, Abort>;

    /// Returns a future that resolves to the outgoing edges of the given
    /// `node`.
    fn edges(&mut self, node: &Node) -> Self::EdgesFuture;
}

// The different `Impl*` here are necessary in order to avoid the `Conflicting
// implementations of trait` error when implementing `Visit` on different
// kinds of `FnMut`.
// See https://users.rust-lang.org/t/conflicting-implementation-when-implementing-traits-for-fn/53359/3

pub struct ImplRef;

impl<Node, VisitFn, NeighFut, NeighIt> Visit<Node, !, ImplRef> for VisitFn
where
    VisitFn: FnMut(&Node) -> NeighFut,
    NeighFut: Future<Output = Result<NeighIt>>,
    NeighIt: IntoIterator<Item = Node>,
{
    type Edge = Node;
    type EdgesIntoIter = NeighIt;
    type EdgesFuture = NeighFut;

    fn visit(&mut self, edge: Self::Edge) -> VisitControlFlow<Node> {
        VisitControlFlow::Continue(edge, Span::current())
    }

    fn edges(&mut self, node: &Node) -> Self::EdgesFuture {
        (self)(node)
    }
}

pub struct ImplValue;

impl<Node, VisitFn, NeighFut, NeighIt> Visit<Node, !, ImplValue> for VisitFn
where
    Node: Clone,
    VisitFn: FnMut(Node) -> NeighFut,
    NeighFut: Future<Output = Result<NeighIt>>,
    NeighIt: IntoIterator<Item = Node>,
{
    type Edge = Node;
    type EdgesIntoIter = NeighIt;
    type EdgesFuture = NeighFut;

    fn visit(&mut self, edge: Self::Edge) -> VisitControlFlow<Node> {
        VisitControlFlow::Continue(edge, Span::current())
    }

    fn edges(&mut self, node: &Node) -> Self::EdgesFuture {
        (self)(node.clone())
    }
}

pub struct WithSpan<Node, Abort, Impl, VisitImpl, F>
where
    VisitImpl: Visit<Node, Abort, Impl>,
    F: FnMut(&Node) -> Span,
{
    visit: VisitImpl,
    func: F,
    phantom: std::marker::PhantomData<(Node, Abort, Impl)>,
}

impl<Node, Abort, Impl, VisitImpl, F> WithSpan<Node, Abort, Impl, VisitImpl, F>
where
    VisitImpl: Visit<Node, Abort, Impl>,
    F: FnMut(&Node) -> Span,
{
    pub fn new(visit: VisitImpl, func: F) -> Self {
        Self {
            visit,
            func,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<Node, Abort, Impl, VisitImpl, F> Visit<Node, Abort, Impl>
    for WithSpan<Node, Abort, Impl, VisitImpl, F>
where
    VisitImpl: Visit<Node, Abort, Impl>,
    F: FnMut(&Node) -> Span,
{
    type Edge = VisitImpl::Edge;
    type EdgesIntoIter = VisitImpl::EdgesIntoIter;
    type EdgesFuture = VisitImpl::EdgesFuture;

    fn visit(&mut self, edge: Self::Edge) -> VisitControlFlow<Node, Abort> {
        match self.visit.visit(edge) {
            VisitControlFlow::Continue(node, _) => {
                let span = (self.func)(&node);
                VisitControlFlow::Continue(node, span)
            }
            VisitControlFlow::Skip(node) => VisitControlFlow::Skip(node),
            VisitControlFlow::Abort(abort) => VisitControlFlow::Abort(abort),
        }
    }

    fn edges(&mut self, node: &Node) -> Self::EdgesFuture {
        self.visit.edges(node)
    }
}
