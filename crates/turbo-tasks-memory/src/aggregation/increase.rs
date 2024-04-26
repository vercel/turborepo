use std::{hash::Hash, mem::take};

use super::{
    balance_edge::balance_edge, AggegatingNode, AggregationContext, AggregationNode,
    AggregationNodeGuard, PreparedOperation, StackVec,
};
pub(super) const LEAF_NUMBER: u8 = 4;

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    pub(super) fn increase_aggregation_number<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        _ctx: &C,
        node_id: &C::NodeRef,
        new_aggregation_number: u32,
    ) -> Option<PreparedIncreaseAggregationNumber<C>> {
        if self.aggregation_number() >= new_aggregation_number {
            return None;
        }
        Some(PreparedIncreaseAggregationNumber {
            node_id: node_id.clone(),
            uppers: self.uppers_mut().iter().cloned().collect(),
            new_aggregation_number,
        })
    }
}

pub struct PreparedIncreaseAggregationNumber<C: AggregationContext> {
    node_id: C::NodeRef,
    uppers: StackVec<C::NodeRef>,
    new_aggregation_number: u32,
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedIncreaseAggregationNumber<C> {
    type Result = ();
    fn apply(self, ctx: &C) {
        let PreparedIncreaseAggregationNumber {
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
        let mut node = ctx.node(&node_id);
        let children = matches!(*node, AggregationNode::Leaf { .. })
            .then(|| node.children().collect::<StackVec<_>>());
        let followers = match &mut *node {
            AggregationNode::Leaf {
                aggregation_number,
                uppers,
            } => {
                let children = children.unwrap();
                if new_aggregation_number < LEAF_NUMBER as u32 {
                    *aggregation_number = new_aggregation_number as u8;
                    drop(node);
                    let prepared = children
                        .into_iter()
                        .map(|child_id| {
                            let mut child = ctx.node(&child_id);
                            child.increase_aggregation_number(
                                ctx,
                                &child_id,
                                new_aggregation_number + 1,
                            )
                        })
                        .collect::<StackVec<_>>();
                    prepared.apply(ctx);
                    return;
                } else {
                    // Convert to Aggregating
                    *node = AggregationNode::Aggegating(Box::new(AggegatingNode {
                        aggregation_number: new_aggregation_number,
                        uppers: take(uppers),
                        followers: children.iter().cloned().collect(),
                        data: node.get_initial_data(),
                    }));
                    let followers = children;
                    drop(node);
                    followers
                }
            }
            AggregationNode::Aggegating(aggegating) => {
                let AggegatingNode {
                    followers,
                    aggregation_number,
                    ..
                } = &mut **aggegating;
                let followers = followers.iter().cloned().collect();
                *aggregation_number = new_aggregation_number;
                drop(node);
                followers
            }
        };
        let node_aggregation_number = new_aggregation_number;
        for follower_id in followers {
            balance_edge(
                ctx,
                &node_id,
                &follower_id,
                node_aggregation_number,
                0,
                false,
            );
        }
        for upper_id in uppers {
            balance_edge(ctx, &upper_id, &node_id, 0, node_aggregation_number, true);
        }
    }
}

pub fn increase_aggregation_number<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    node_id: &C::NodeRef,
    new_aggregation_number: u32,
) {
    let prepared = node.increase_aggregation_number(ctx, &node_id, new_aggregation_number);
    drop(node);
    prepared.apply(ctx);
}
