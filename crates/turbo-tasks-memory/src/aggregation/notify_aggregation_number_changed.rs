use super::{balance_edge, AggegatingNode, AggregationContext, AggregationNode};

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
    if inner_aggregation_number == u32::MAX || aggregation_number == u32::MAX {
        // This should stay an inner.
        return;
    }
    if inner_aggregation_number < aggregation_number {
        // This should also stay an inner.
        return;
    }
    drop(upper);
    balance_edge(
        ctx,
        &upper_id,
        &inner_id,
        aggregation_number,
        inner_aggregation_number,
        true,
    );
}
