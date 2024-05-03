use std::hash::Hash;

use super::{
    balance_queue::BalanceQueue,
    in_progress::{start_in_progress, start_in_progress_all},
    increase_aggregation_number_internal, notify_new_follower,
    notify_new_follower::PreparedNotifyNewFollower,
    AggregationContext, AggregationNode, PreparedInternalOperation, PreparedOperation, StackVec,
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
                start_in_progress_all(ctx, &uppers);
                PreparedNewEdge::Leaf {
                    child_aggregation_number,
                    uppers,
                    target_id: target_id.clone(),
                }
            }
            AggregationNode::Aggegating(_) => {
                start_in_progress(ctx, origin_id);
                PreparedNewEdge::Aggegating {
                    notify: self.notify_new_follower(ctx, origin_id, target_id),
                }
            }
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
    type Result = ();
    fn apply(self, ctx: &C) {
        let mut balance_queue = BalanceQueue::new();
        match self {
            PreparedNewEdge::Leaf {
                child_aggregation_number,
                uppers,
                target_id,
            } => {
                for upper_id in uppers {
                    notify_new_follower(
                        ctx,
                        &mut balance_queue,
                        ctx.node(&upper_id),
                        &upper_id,
                        &target_id,
                    );
                }
                {
                    // TODO add to prepared
                    increase_aggregation_number_internal(
                        ctx,
                        &mut balance_queue,
                        ctx.node(&target_id),
                        &target_id,
                        child_aggregation_number,
                    );
                }
            }
            PreparedNewEdge::Aggegating { notify } => notify.apply(ctx, &mut balance_queue),
        }
        balance_queue.process(ctx);
    }
}
