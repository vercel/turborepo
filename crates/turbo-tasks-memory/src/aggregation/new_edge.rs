use super::{
    balance_queue::BalanceQueue,
    in_progress::start_in_progress_all,
    increase::{
        increase_aggregation_number_immediately, PreparedInternalIncreaseAggregationNumber,
        LEAF_NUMBER,
    },
    increase_aggregation_number_internal, notify_new_follower,
    notify_new_follower::PreparedNotifyNewFollower,
    AggregationContext, AggregationNode, PreparedInternalOperation, PreparedOperation, StackVec,
};

#[cfg(test)]
const BUFFER_SPACE: u32 = 1;
#[cfg(not(test))]
const BUFFER_SPACE: u32 = 3;

const MAX_UPPERS_TIMES_CHILDREN: usize = 128;

#[tracing::instrument(level = tracing::Level::TRACE, name = "handle_new_edge_preparation", skip_all)]
pub fn handle_new_edge<'l, C: AggregationContext>(
    ctx: &C,
    origin: &mut C::Guard<'l>,
    origin_id: &C::NodeRef,
    target_id: &C::NodeRef,
    number_of_children: usize,
) -> impl PreparedOperation<C> {
    match **origin {
        AggregationNode::Leaf {
            ref mut aggregation_number,
            ref uppers,
        } => {
            if number_of_children.count_ones() == 1
                && uppers.len() * number_of_children > MAX_UPPERS_TIMES_CHILDREN
            {
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                start_in_progress_all(ctx, &uppers);
                let increase = increase_aggregation_number_immediately(
                    ctx,
                    origin,
                    origin_id.clone(),
                    LEAF_NUMBER,
                    LEAF_NUMBER,
                )
                .unwrap();
                Some(PreparedNewEdge::Upgraded {
                    uppers,
                    target_id: target_id.clone(),
                    increase,
                })
            } else {
                let min_aggregation_number = *aggregation_number as u32 + 1;
                let target_aggregation_number = *aggregation_number as u32 + 1 + BUFFER_SPACE;
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                start_in_progress_all(ctx, &uppers);
                Some(PreparedNewEdge::Leaf {
                    min_aggregation_number,
                    target_aggregation_number,
                    uppers,
                    target_id: target_id.clone(),
                })
            }
        }
        AggregationNode::Aggegating(_) => origin
            .notify_new_follower_not_in_progress(ctx, origin_id, target_id)
            .map(|notify| PreparedNewEdge::Aggegating { notify }),
    }
}
enum PreparedNewEdge<C: AggregationContext> {
    Leaf {
        min_aggregation_number: u32,
        target_aggregation_number: u32,
        uppers: StackVec<C::NodeRef>,
        target_id: C::NodeRef,
    },
    Upgraded {
        uppers: StackVec<C::NodeRef>,
        target_id: C::NodeRef,
        increase: PreparedInternalIncreaseAggregationNumber<C>,
    },
    Aggegating {
        notify: PreparedNotifyNewFollower<C>,
    },
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedNewEdge<C> {
    type Result = ();
    #[tracing::instrument(level = tracing::Level::TRACE, name = "handle_new_edge", skip_all)]
    fn apply(self, ctx: &C) {
        let mut balance_queue = BalanceQueue::new();
        match self {
            PreparedNewEdge::Leaf {
                min_aggregation_number,
                target_aggregation_number,
                uppers,
                target_id,
            } => {
                let _span = tracing::trace_span!("leaf").entered();
                {
                    let _span =
                        tracing::trace_span!("increase_aggregation_number_internal").entered();
                    // TODO add to prepared
                    increase_aggregation_number_internal(
                        ctx,
                        &mut balance_queue,
                        ctx.node(&target_id),
                        &target_id,
                        min_aggregation_number,
                        target_aggregation_number,
                    );
                }
                for upper_id in uppers {
                    notify_new_follower(
                        ctx,
                        &mut balance_queue,
                        ctx.node(&upper_id),
                        &upper_id,
                        &target_id,
                    );
                }
            }
            PreparedNewEdge::Upgraded {
                uppers,
                target_id,
                increase,
            } => {
                // Since it was added to a leaf node, we would add it to the uppers
                for upper_id in uppers {
                    notify_new_follower(
                        ctx,
                        &mut balance_queue,
                        ctx.node(&upper_id),
                        &upper_id,
                        &target_id,
                    );
                }
                // The balancing will attach it to the aggregated node later
                increase.apply(ctx, &mut balance_queue);
            }
            PreparedNewEdge::Aggegating { notify } => notify.apply(ctx, &mut balance_queue),
        }
        let _span = tracing::trace_span!("balance_queue").entered();
        balance_queue.process(ctx);
    }
}
