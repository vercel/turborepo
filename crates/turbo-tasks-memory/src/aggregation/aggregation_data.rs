use std::ops::{Deref, DerefMut};

use super::{
    increase_aggregation_number_internal, AggregationContext, AggregationNode, AggregationNodeGuard,
};
use crate::aggregation::balance_queue::BalanceQueue;

/// Gives an reference to the root aggregated info for a given item.
pub fn aggregation_data<'l, C>(
    ctx: &'l C,
    node_id: &C::NodeRef,
) -> AggregationDataGuard<C::Guard<'l>>
where
    C: AggregationContext + 'l,
{
    let guard = ctx.node(node_id);
    if guard.aggregation_number() == u32::MAX {
        AggregationDataGuard { guard }
    } else {
        let mut balance_queue = BalanceQueue::new();
        increase_aggregation_number_internal(
            ctx,
            &mut balance_queue,
            guard,
            node_id,
            u32::MAX,
            u32::MAX,
        );
        balance_queue.process(ctx);
        let guard = ctx.node(node_id);
        debug_assert!(guard.aggregation_number() == u32::MAX);
        AggregationDataGuard { guard }
    }
}

pub fn prepare_aggregation_data<C: AggregationContext>(ctx: &C, node_id: &C::NodeRef) {
    let mut balance_queue = BalanceQueue::new();
    increase_aggregation_number_internal(
        ctx,
        &mut balance_queue,
        ctx.node(node_id),
        node_id,
        u32::MAX,
        u32::MAX,
    );
    balance_queue.process(ctx);
}

/// A reference to the root aggregated info of a node.
pub struct AggregationDataGuard<G> {
    guard: G,
}

impl<G> AggregationDataGuard<G> {
    pub fn into_inner(self) -> G {
        self.guard
    }
}

impl<G: AggregationNodeGuard> Deref for AggregationDataGuard<G> {
    type Target = G::Data;

    fn deref(&self) -> &Self::Target {
        match &*self.guard {
            AggregationNode::Leaf { .. } => unreachable!(),
            AggregationNode::Aggegating(aggregating) => &aggregating.data,
        }
    }
}

impl<G: AggregationNodeGuard> DerefMut for AggregationDataGuard<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut *self.guard {
            AggregationNode::Leaf { .. } => unreachable!(),
            AggregationNode::Aggegating(aggregating) => &mut aggregating.data,
        }
    }
}
