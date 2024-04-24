use super::{
    followers::{add_follower, add_follower_count, remove_follower, remove_follower_count},
    increase_aggregation_number, AggegatingNode, AggregationContext, AggregationNode,
};

pub(super) fn notify_aggregation_number_changed<C: AggregationContext>(
    ctx: &C,
    upper: C::Guard<'_>,
    upper_id: &C::NodeRef,
    inner_id: &C::NodeRef,
    inner_aggregation_number: u32,
) {
    let AggregationNode::Aggegating(aggregating) = &*upper else {
        unreachable!();
    };
    let AggegatingNode {
        aggregation_number, ..
    } = **aggregating;
    if inner_aggregation_number == u32::MAX {
        return;
    }
    if inner_aggregation_number < aggregation_number {
        return;
    }
    if inner_aggregation_number == aggregation_number {
        drop(upper);
        increase_aggregation_number(
            ctx,
            ctx.node(inner_id),
            inner_id,
            inner_aggregation_number + 1,
        );
        return;
    }
    // Inner is currently higher than the upper. That's an invariant violation.
    // We convert the inner to a follower.
    add_follower(ctx, upper, upper_id, inner_id);
    let mut follower = ctx.node(inner_id);
    let count = follower.uppers_mut().remove_entry(upper_id) - 1;
    if count == 0 {
        return;
    }
    drop(follower);
    let upper = ctx.node(upper_id);
    if count > 0 {
        add_follower_count(ctx, upper, upper_id, inner_id, count as usize);
    } else {
        remove_follower_count(ctx, upper, inner_id, (-count) as usize);
    }
}
