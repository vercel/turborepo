use std::hash::Hash;

use super::{
    notify_lost_follower, notify_new_follower, AggregationContext, AggregationNode,
    PreparedOperation, StackVec,
};

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    pub(super) fn add_follower<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        _ctx: &C,
        _upper_id: &C::NodeRef,
        follower_id: &C::NodeRef,
    ) -> Option<PreparedAddFollower<C>> {
        let AggregationNode::Aggegating(aggregating) = self else {
            unreachable!();
        };
        if aggregating.followers.add_clonable(follower_id) {
            let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
            Some(PreparedAddFollower {
                uppers,
                follower_id: follower_id.clone(),
            })
        } else {
            None
        }
    }

    pub(super) fn add_follower_count<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        _ctx: &C,
        _upper_id: &C::NodeRef,
        follower_id: &C::NodeRef,
        follower_count: usize,
    ) -> Option<PreparedAddFollower<C>> {
        let AggregationNode::Aggegating(aggregating) = self else {
            unreachable!();
        };
        if aggregating
            .followers
            .add_clonable_count(follower_id, follower_count)
        {
            let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
            Some(PreparedAddFollower {
                uppers,
                follower_id: follower_id.clone(),
            })
        } else {
            None
        }
    }
}

pub(super) struct PreparedAddFollower<C: AggregationContext> {
    uppers: StackVec<C::NodeRef>,
    follower_id: C::NodeRef,
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedAddFollower<C> {
    fn apply(self, ctx: &C) {
        let PreparedAddFollower {
            uppers,
            follower_id,
        } = self;
        for upper_id in uppers {
            notify_new_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
        }
    }
}

pub fn add_follower<C: AggregationContext>(
    ctx: &C,
    mut aggregating: C::Guard<'_>,
    aggregating_id: &C::NodeRef,
    follower_id: &C::NodeRef,
) {
    let p = aggregating.add_follower(ctx, aggregating_id, follower_id);
    drop(aggregating);
    p.apply(ctx);
}

pub fn add_follower_count<C: AggregationContext>(
    ctx: &C,
    mut aggregating: C::Guard<'_>,
    aggregating_id: &C::NodeRef,
    follower_id: &C::NodeRef,
    follower_count: usize,
) {
    let p = aggregating.add_follower_count(ctx, aggregating_id, follower_id, follower_count);
    drop(aggregating);
    p.apply(ctx);
}

pub fn remove_follower<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    follower_id: &C::NodeRef,
) {
    let AggregationNode::Aggegating(aggregating) = &mut *node else {
        unreachable!();
    };
    if aggregating.followers.remove_clonable(follower_id) {
        let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
        drop(node);
        for upper_id in uppers {
            notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, follower_id);
        }
    }
}

pub fn remove_follower_count<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    follower_id: &C::NodeRef,
    follower_count: usize,
) {
    let AggregationNode::Aggegating(aggregating) = &mut *node else {
        unreachable!();
    };
    if aggregating
        .followers
        .remove_clonable_count(follower_id, follower_count)
    {
        let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
        drop(node);
        for upper_id in uppers {
            notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, follower_id);
        }
    }
}

pub fn remove_follower_all_count<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    follower_id: &C::NodeRef,
) -> isize {
    let AggregationNode::Aggegating(aggregating) = &mut *node else {
        unreachable!();
    };
    let count = aggregating.followers.remove_entry(follower_id);
    if count > 0 {
        let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
        drop(node);
        for upper_id in uppers {
            notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, follower_id);
        }
    } else {
        drop(node);
    }
    count
}
