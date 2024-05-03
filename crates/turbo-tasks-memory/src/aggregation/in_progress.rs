use std::hash::Hash;

use super::{AggregationContext, AggregationNode, StackVec};

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    pub(super) fn finish_in_progress<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        ctx: &C,
        node_id: &I,
    ) {
        let value = ctx
            .atomic_in_progress_counter(node_id)
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        debug_assert!(value > 0);
        if value == 1 {
            if let AggregationNode::Aggegating(aggegating) = &mut *self {
                aggegating.waiting_for_in_progress.notify();
            }
        }
    }
}

pub fn finish_in_progress_without_node<C: AggregationContext>(ctx: &C, node_id: &C::NodeRef) {
    let value = ctx
        .atomic_in_progress_counter(node_id)
        .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    debug_assert!(value > 0);
    if value == 1 {
        let mut node = ctx.node(node_id);
        if let AggregationNode::Aggegating(aggegating) = &mut *node {
            aggegating.waiting_for_in_progress.notify();
        }
    }
}

pub fn start_in_progress_all<C: AggregationContext>(ctx: &C, node_ids: &StackVec<C::NodeRef>) {
    for node_id in node_ids {
        start_in_progress(ctx, node_id);
    }
}

pub fn start_in_progress<C: AggregationContext>(ctx: &C, node_id: &C::NodeRef) {
    start_in_progress_count(ctx, node_id, 1);
}

pub fn start_in_progress_count<C: AggregationContext>(ctx: &C, node_id: &C::NodeRef, count: u32) {
    if count == 0 {
        return;
    }
    ctx.atomic_in_progress_counter(node_id)
        .fetch_add(count, std::sync::atomic::Ordering::Release);
}

pub fn is_in_progress<C: AggregationContext>(ctx: &C, node_id: &C::NodeRef) -> bool {
    let counter = ctx
        .atomic_in_progress_counter(node_id)
        .load(std::sync::atomic::Ordering::Acquire);
    counter > 0
}
