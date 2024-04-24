use super::{
    add_followers::add_follower, increase_aggregation_number, uppers::get_aggregated_remove_change,
    AggegatingNode, AggregationContext, AggregationNode, PreparedOperation,
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
    add_follower(ctx, upper, inner_id);
    let mut follower = ctx.node(inner_id);
    let count = follower.uppers_mut().remove_entry(upper_id) - 1;
    let remove_change = if count > -1 {
        // An upper was removed, we need to update aggregated data.
        get_aggregated_remove_change(ctx, &mut follower)
    } else {
        None
    };
    drop(follower);
    if count == 0 && remove_change.is_none() {
        return;
    }
    let mut upper = ctx.node(upper_id);
    let remove_job = remove_change.and_then(|remove_change| upper.apply_change(ctx, remove_change));
    let add_follower_job =
        (count > 0).then(|| upper.add_follower_count(ctx, inner_id, count as usize));
    let remove_follower_job =
        (count < 0).then(|| upper.remove_follower_count(ctx, inner_id, (-count) as usize));
    drop(upper);
    remove_job.apply(ctx);
    add_follower_job.apply(ctx);
    remove_follower_job.apply(ctx);
}
