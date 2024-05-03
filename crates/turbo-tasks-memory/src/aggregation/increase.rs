use std::{hash::Hash, mem::take};

use super::{
    balance_queue::BalanceQueue, AggegatingNode, AggregationContext, AggregationNode,
    AggregationNodeGuard, PreparedInternalOperation, PreparedOperation, StackVec,
};
#[cfg(test)]
pub(super) const LEAF_NUMBER: u32 = 8;
#[cfg(not(test))]
pub(super) const LEAF_NUMBER: u32 = 256;

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    pub(super) fn increase_aggregation_number_internal<
        C: AggregationContext<NodeRef = I, Data = D>,
    >(
        &mut self,
        _ctx: &C,
        node_id: &C::NodeRef,
        new_aggregation_number: u32,
    ) -> Option<PreparedInternalIncreaseAggregationNumber<C>> {
        if self.aggregation_number() >= new_aggregation_number {
            return None;
        }
        Some(PreparedInternalIncreaseAggregationNumber {
            node_id: node_id.clone(),
            uppers: self.uppers_mut().iter().cloned().collect(),
            new_aggregation_number,
        })
    }

    #[cfg(test)]
    pub fn increase_aggregation_number<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        _ctx: &C,
        node_id: &C::NodeRef,
        new_aggregation_number: u32,
    ) -> Option<PreparedIncreaseAggregationNumber<C>> {
        self.increase_aggregation_number_internal(_ctx, node_id, new_aggregation_number)
            .map(PreparedIncreaseAggregationNumber)
    }
}

pub struct PreparedInternalIncreaseAggregationNumber<C: AggregationContext> {
    node_id: C::NodeRef,
    uppers: StackVec<C::NodeRef>,
    new_aggregation_number: u32,
}

impl<C: AggregationContext> PreparedInternalOperation<C>
    for PreparedInternalIncreaseAggregationNumber<C>
{
    type Result = ();
    fn apply(self, ctx: &C, balance_queue: &mut BalanceQueue<C::NodeRef>) {
        let PreparedInternalIncreaseAggregationNumber {
            mut new_aggregation_number,
            node_id,
            uppers,
        } = self;
        let mut need_to_run = true;
        while need_to_run {
            need_to_run = false;
            let mut max = 0;
            for upper_id in &uppers {
                let upper = ctx.node(upper_id);
                let aggregation_number = upper.aggregation_number();
                if aggregation_number != u32::MAX {
                    if aggregation_number > max {
                        max = aggregation_number;
                    }
                    if aggregation_number == new_aggregation_number {
                        new_aggregation_number += 1;
                        if max >= new_aggregation_number {
                            need_to_run = true;
                        }
                    }
                }
            }
        }
        drop(uppers);
        let mut node = ctx.node(&node_id);
        if node.aggregation_number() >= new_aggregation_number {
            return;
        }
        let children = matches!(*node, AggregationNode::Leaf { .. })
            .then(|| node.children().collect::<StackVec<_>>());
        let (uppers, followers) = match &mut *node {
            AggregationNode::Leaf {
                aggregation_number,
                uppers,
            } => {
                let children = children.unwrap();
                if new_aggregation_number < LEAF_NUMBER {
                    *aggregation_number = new_aggregation_number as u8;
                    drop(node);
                    for child_id in children {
                        increase_aggregation_number_internal(
                            ctx,
                            balance_queue,
                            ctx.node(&child_id),
                            &child_id,
                            new_aggregation_number + 1,
                        );
                    }
                    return;
                } else {
                    let uppers_copy = uppers.iter().cloned().collect::<StackVec<_>>();
                    // Convert to Aggregating
                    *node = AggregationNode::Aggegating(Box::new(AggegatingNode {
                        aggregation_number: new_aggregation_number,
                        uppers: take(uppers),
                        followers: children.iter().cloned().collect(),
                        data: node.get_initial_data(),
                        enqueued_balancing: Vec::new(),
                    }));
                    let followers = children;
                    drop(node);
                    (uppers_copy, followers)
                }
            }
            AggregationNode::Aggegating(aggegating) => {
                let AggegatingNode {
                    followers,
                    uppers,
                    aggregation_number,
                    ..
                } = &mut **aggegating;
                let uppers = uppers.iter().cloned().collect::<StackVec<_>>();
                let followers = followers.iter().cloned().collect();
                *aggregation_number = new_aggregation_number;
                drop(node);
                (uppers, followers)
            }
        };
        for follower_id in followers {
            balance_queue.balance(node_id.clone(), follower_id);
        }
        for upper_id in uppers {
            balance_queue.balance(upper_id, node_id.clone());
        }
    }
}

pub fn increase_aggregation_number_internal<C: AggregationContext>(
    ctx: &C,
    balance_queue: &mut BalanceQueue<C::NodeRef>,
    mut node: C::Guard<'_>,
    node_id: &C::NodeRef,
    new_aggregation_number: u32,
) {
    let prepared = node.increase_aggregation_number_internal(ctx, node_id, new_aggregation_number);
    drop(node);
    prepared.apply(ctx, balance_queue);
}

pub struct PreparedIncreaseAggregationNumber<C: AggregationContext>(
    PreparedInternalIncreaseAggregationNumber<C>,
);

impl<C: AggregationContext> PreparedOperation<C> for PreparedIncreaseAggregationNumber<C> {
    type Result = ();
    fn apply(self, ctx: &C) {
        let mut balance_queue = BalanceQueue::new();
        self.0.apply(ctx, &mut balance_queue);
        balance_queue.process(ctx);
    }
}
