use std::hash::Hash;

use super::{
    notify_lost_follower, notify_lost_follower::PreparedNotifyLostFollower, AggregationContext,
    AggregationNode, PreparedOperation, StackVec,
};

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    pub fn handle_lost_edge<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        ctx: &C,
        origin_id: &C::NodeRef,
        target_id: &C::NodeRef,
    ) -> Option<PreparedLostEdge<C>> {
        match self {
            AggregationNode::Leaf { uppers, .. } => {
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                Some(
                    PreparedLostEdgeInner::Leaf {
                        uppers,
                        target_id: target_id.clone(),
                    }
                    .into(),
                )
            }
            AggregationNode::Aggegating(_) => {
                let notify = self.notify_lost_follower(ctx, origin_id, target_id);
                notify.map(|notify| notify.into())
            }
        }
    }

    pub fn handle_lost_edges<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        ctx: &C,
        origin_id: &C::NodeRef,
        target_ids: impl IntoIterator<Item = C::NodeRef>,
    ) -> Option<PreparedLostEdges<C>> {
        match self {
            AggregationNode::Leaf { uppers, .. } => {
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                Some(
                    PreparedLostEdgesInner::Leaf {
                        uppers,
                        target_ids: target_ids.into_iter().collect(),
                    }
                    .into(),
                )
            }
            AggregationNode::Aggegating(_) => {
                let notify = target_ids
                    .into_iter()
                    .filter_map(|target_id| self.notify_lost_follower(ctx, origin_id, &target_id))
                    .collect::<StackVec<_>>();
                (!notify.is_empty()).then(|| notify.into())
            }
        }
    }
}

pub struct PreparedLostEdge<C: AggregationContext> {
    inner: PreparedLostEdgeInner<C>,
}

impl<C: AggregationContext> From<PreparedLostEdgeInner<C>> for PreparedLostEdge<C> {
    fn from(inner: PreparedLostEdgeInner<C>) -> Self {
        Self { inner }
    }
}

impl<C: AggregationContext> From<PreparedNotifyLostFollower<C>> for PreparedLostEdge<C> {
    fn from(notify: PreparedNotifyLostFollower<C>) -> Self {
        Self {
            inner: PreparedLostEdgeInner::Aggregating { notify },
        }
    }
}

enum PreparedLostEdgeInner<C: AggregationContext> {
    Leaf {
        uppers: StackVec<C::NodeRef>,
        target_id: C::NodeRef,
    },
    Aggregating {
        notify: PreparedNotifyLostFollower<C>,
    },
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedLostEdge<C> {
    type Result = ();
    fn apply(self, ctx: &C) {
        match self.inner {
            PreparedLostEdgeInner::Leaf { uppers, target_id } => {
                for upper_id in uppers {
                    notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, &target_id);
                }
            }
            PreparedLostEdgeInner::Aggregating { notify } => {
                notify.apply(ctx);
            }
        }
    }
}

pub struct PreparedLostEdges<C: AggregationContext> {
    inner: PreparedLostEdgesInner<C>,
}

impl<C: AggregationContext> From<PreparedLostEdgesInner<C>> for PreparedLostEdges<C> {
    fn from(inner: PreparedLostEdgesInner<C>) -> Self {
        Self { inner }
    }
}

impl<C: AggregationContext> From<StackVec<PreparedNotifyLostFollower<C>>> for PreparedLostEdges<C> {
    fn from(notify: StackVec<PreparedNotifyLostFollower<C>>) -> Self {
        Self {
            inner: PreparedLostEdgesInner::Aggregating { notify },
        }
    }
}

enum PreparedLostEdgesInner<C: AggregationContext> {
    Leaf {
        uppers: StackVec<C::NodeRef>,
        target_ids: StackVec<C::NodeRef>,
    },
    Aggregating {
        notify: StackVec<PreparedNotifyLostFollower<C>>,
    },
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedLostEdges<C> {
    type Result = ();
    fn apply(self, ctx: &C) {
        match self.inner {
            PreparedLostEdgesInner::Leaf { uppers, target_ids } => {
                // TODO This could be more efficient
                for upper_id in uppers {
                    let mut upper = ctx.node(&upper_id);
                    let prepared = target_ids
                        .iter()
                        .filter_map(|target_id| {
                            upper.notify_lost_follower(ctx, &upper_id, target_id)
                        })
                        .collect::<StackVec<_>>();
                    drop(upper);
                    prepared.apply(ctx);
                }
            }
            PreparedLostEdgesInner::Aggregating { notify } => {
                notify.apply(ctx);
            }
        }
    }
}

pub fn handle_lost_edge<C: AggregationContext>(
    ctx: &C,
    mut origin: C::Guard<'_>,
    origin_id: &C::NodeRef,
    target_id: &C::NodeRef,
) {
    let p = origin.handle_lost_edge(ctx, origin_id, target_id);
    drop(origin);
    p.apply(ctx);
}

pub fn handle_lost_edges<C: AggregationContext>(
    ctx: &C,
    mut origin: C::Guard<'_>,
    origin_id: &C::NodeRef,
    target_ids: impl IntoIterator<Item = C::NodeRef>,
) {
    let p = origin.handle_lost_edges(ctx, origin_id, target_ids);
    drop(origin);
    p.apply(ctx);
}
