use std::cmp::Ordering;

use super::{
    followers::{
        add_follower_count, remove_follower_count, remove_positive_follower_count,
        RemovePositveFollowerCountResult,
    },
    increase_aggregation_number,
    uppers::{
        add_upper_count, remove_positive_upper_count, remove_upper_count,
        RemovePositiveUpperCountResult,
    },
    AggregationContext, AggregationNode,
};
use crate::count_hash_set::RemovePositiveCountResult;

// Migrated followers to uppers or uppers to followers depending on the
// aggregation numbers of the nodes involved in the edge. Might increase targets
// aggregation number if they are equal.
pub(super) fn balance_edge<C: AggregationContext>(
    ctx: &C,
    upper_id: &C::NodeRef,
    target_id: &C::NodeRef,
    mut upper_aggregation_number: u32,
    mut target_aggregation_number: u32,
    is_inner: bool,
) {
    // too many uppers on target
    let mut extra_uppers = 0;
    // too many followers on upper
    let mut extra_followers = 0;
    // The last info about uppers
    let mut uppers_count = (!is_inner).then_some(0);
    // The last info about followers
    let mut followers_count = is_inner.then_some(0);

    loop {
        let root = upper_aggregation_number == u32::MAX || target_aggregation_number == u32::MAX;
        let order = if root {
            Ordering::Greater
        } else {
            upper_aggregation_number.cmp(&target_aggregation_number)
        };
        match order {
            Ordering::Equal => {
                // we probably want to increase the aggregation number of target
                let upper = ctx.node(&upper_id);
                upper_aggregation_number = upper.aggregation_number();
                drop(upper);
                if upper_aggregation_number != u32::MAX
                    && upper_aggregation_number == target_aggregation_number
                {
                    let target = ctx.node(&target_id);
                    target_aggregation_number = target.aggregation_number();
                    if upper_aggregation_number == target_aggregation_number {
                        // increase target aggregation number
                        increase_aggregation_number(
                            ctx,
                            target,
                            &target_id,
                            target_aggregation_number + 1,
                        );
                        continue;
                    }
                }
            }
            Ordering::Less => {
                // target should probably be a follower of upper
                if uppers_count.map_or(false, |count| count <= 0) {
                    // We already removed all uppers, maybe too many
                    break;
                } else if extra_followers == 0 {
                    let upper = ctx.node(&upper_id);
                    upper_aggregation_number = upper.aggregation_number();
                    if upper_aggregation_number < target_aggregation_number {
                        // target should be a follower of upper
                        // add some extra followers
                        let count = uppers_count.unwrap_or(1) as usize;
                        extra_followers += count;
                        followers_count = Some(add_follower_count(ctx, upper, &target_id, count));
                        continue;
                    }
                } else {
                    // we already have extra followers, remove some uppers to balance
                    let count = extra_followers + extra_uppers;
                    let target = ctx.node(&target_id);
                    let RemovePositiveUpperCountResult {
                        removed_count,
                        remaining_count,
                    } = remove_positive_upper_count(ctx, target, &upper_id, count);
                    decrease_numbers(removed_count, &mut extra_uppers, &mut extra_followers);
                    uppers_count = Some(remaining_count);
                }
            }
            Ordering::Greater => {
                // target should probably be an inner node of upper
                if followers_count.map_or(false, |count| count <= 0) {
                    // We already removed all followers, maybe too many
                    break;
                } else if extra_uppers == 0 {
                    let target = ctx.node(&target_id);
                    target_aggregation_number = target.aggregation_number();
                    if root || target_aggregation_number < upper_aggregation_number {
                        // target should be a inner node of upper
                        // add some extra uppers
                        let count = followers_count.unwrap_or(1) as usize;
                        extra_uppers += count;
                        uppers_count =
                            Some(add_upper_count(ctx, target, &target_id, &upper_id, count));
                        continue;
                    }
                } else {
                    // we already have extra uppers, try to remove some followers to balance
                    let count = extra_followers + extra_uppers;
                    let upper = ctx.node(&upper_id);
                    let RemovePositveFollowerCountResult {
                        removed_count,
                        remaining_count,
                    } = remove_positive_follower_count(ctx, upper, &target_id, count);
                    decrease_numbers(removed_count, &mut extra_followers, &mut extra_uppers);
                    followers_count = Some(remaining_count);
                }
            }
        }
    }
    if extra_followers > 0 {
        let upper = ctx.node(&upper_id);
        remove_follower_count(ctx, upper, &target_id, extra_followers);
    }
    if extra_uppers > 0 {
        let target = ctx.node(&target_id);
        remove_upper_count(ctx, target, &upper_id, extra_uppers);
    }
}

fn decrease_numbers(amount: usize, a: &mut usize, b: &mut usize) {
    if *a >= amount {
        *a -= amount;
    } else {
        *b -= amount - *a;
        *a = 0;
    }
}
