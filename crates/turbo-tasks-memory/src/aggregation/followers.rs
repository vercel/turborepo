use super::{
    notify_lost_follower, notify_new_follower, AggregationContext, AggregationNode, StackVec,
};
use crate::count_hash_set::RemovePositiveCountResult;

pub fn add_follower<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    follower_id: &C::NodeRef,
) {
    let AggregationNode::Aggegating(aggregating) = &mut *node else {
        unreachable!();
    };
    if aggregating.followers.add_clonable(follower_id) {
        let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
        drop(node);
        for upper_id in uppers {
            notify_new_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
        }
    }
}

pub fn add_follower_count<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    follower_id: &C::NodeRef,
    follower_count: usize,
) -> isize {
    let AggregationNode::Aggegating(aggregating) = &mut *node else {
        unreachable!();
    };
    if aggregating
        .followers
        .add_clonable_count(follower_id, follower_count)
    {
        let count = aggregating.followers.get_count(follower_id);
        let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
        drop(node);
        for upper_id in uppers {
            notify_new_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
        }
        count
    } else {
        aggregating.followers.get_count(follower_id)
    }
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
            notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
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
            notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
        }
    }
}

pub struct RemovePositveFollowerCountResult {
    pub removed_count: usize,
    pub remaining_count: isize,
}

pub fn remove_positive_follower_count<C: AggregationContext>(
    ctx: &C,
    mut node: C::Guard<'_>,
    follower_id: &C::NodeRef,
    follower_count: usize,
) -> RemovePositveFollowerCountResult {
    let AggregationNode::Aggegating(aggregating) = &mut *node else {
        unreachable!();
    };
    let RemovePositiveCountResult {
        removed,
        removed_count,
        count,
    } = aggregating
        .followers
        .remove_positive_clonable_count(follower_id, follower_count);

    if removed {
        let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
        drop(node);
        for upper_id in uppers {
            notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
        }
    }
    RemovePositveFollowerCountResult {
        removed_count,
        remaining_count: count,
    }
}
