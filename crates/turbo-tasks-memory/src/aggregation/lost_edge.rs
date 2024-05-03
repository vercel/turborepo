use std::hash::Hash;

use super::{
    in_progress::{start_in_progress, start_in_progress_count},
    notify_lost_follower::PreparedNotifyLostFollower,
    AggregationContext, AggregationNode, PreparedOperation, StackVec,
};

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    #[must_use]
    pub fn handle_lost_edges<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        ctx: &C,
        origin_id: &C::NodeRef,
        target_ids: impl IntoIterator<Item = C::NodeRef>,
    ) -> Option<PreparedLostEdges<C>> {
        match self {
            AggregationNode::Leaf { uppers, .. } => {
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                let target_ids: StackVec<_> = target_ids.into_iter().collect();
                for upper_id in &uppers {
                    start_in_progress_count(ctx, upper_id, target_ids.len() as u32);
                }
                Some(PreparedLostEdgesInner::Leaf { uppers, target_ids }.into())
            }
            AggregationNode::Aggegating(_) => {
                let notify = target_ids
                    .into_iter()
                    .filter_map(|target_id| {
                        start_in_progress(ctx, origin_id);
                        self.notify_lost_follower(ctx, origin_id, &target_id)
                    })
                    .collect::<StackVec<_>>();
                (!notify.is_empty()).then(|| notify.into())
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

#[cfg(test)]
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
