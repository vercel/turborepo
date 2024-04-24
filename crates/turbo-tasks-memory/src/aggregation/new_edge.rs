use std::hash::Hash;

use super::{
    increase_aggregation_number, notify_new_follower,
    notify_new_follower::PreparedNotifyNewFollower, AggregationContext, AggregationNode,
    PreparedOperation, StackVec,
};

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    #[must_use]
    pub fn handle_new_edge<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        ctx: &C,
        origin_id: &C::NodeRef,
        target_id: &C::NodeRef,
    ) -> impl PreparedOperation<C> {
        match self {
            AggregationNode::Leaf {
                aggregation_number,
                uppers,
            } => {
                let child_aggregation_number = *aggregation_number as u32 + 1;
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                PreparedNewEdge::Leaf {
                    child_aggregation_number,
                    uppers,
                    target_id: target_id.clone(),
                }
            }
            AggregationNode::Aggegating(_) => PreparedNewEdge::Aggegating {
                notify: self.notify_new_follower(ctx, origin_id, target_id),
            },
        }
    }
}

enum PreparedNewEdge<C: AggregationContext> {
    Leaf {
        child_aggregation_number: u32,
        uppers: StackVec<C::NodeRef>,
        target_id: C::NodeRef,
    },
    Aggegating {
        notify: PreparedNotifyNewFollower<C>,
    },
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedNewEdge<C> {
    fn apply(self, ctx: &C) {
        match self {
            PreparedNewEdge::Leaf {
                child_aggregation_number,
                uppers,
                target_id,
            } => {
                {
                    // TODO add to prepared
                    increase_aggregation_number(
                        ctx,
                        ctx.node(&target_id),
                        &target_id,
                        child_aggregation_number,
                    );
                }
                for upper_id in uppers {
                    notify_new_follower(ctx, ctx.node(&upper_id), &upper_id, &target_id);
                }
            }
            PreparedNewEdge::Aggegating { notify } => notify.apply(ctx),
        }
    }
}

pub fn handle_new_edge<C: AggregationContext>(
    ctx: &C,
    mut origin: C::Guard<'_>,
    origin_id: &C::NodeRef,
    target_id: &C::NodeRef,
) {
    let p = origin.handle_new_edge(ctx, origin_id, target_id);
    drop(origin);
    p.apply(ctx);
}
