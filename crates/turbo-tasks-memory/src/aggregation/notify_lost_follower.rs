use std::{hash::Hash, thread::yield_now, time::Instant};

use anyhow::{bail, Result};

use super::{
    in_progress::{
        finish_in_progress_without_node, is_in_progress, start_in_progress, start_in_progress_all,
        start_in_progress_count,
    },
    uppers::get_aggregated_remove_change,
    AggegatingNode, AggregationContext, AggregationNode, AggregationNodeGuard, PreparedOperation,
    StackVec,
};
use crate::count_hash_set::RemoveIfEntryResult;

impl<I: Clone + Eq + Hash, D> AggregationNode<I, D> {
    /// Called when a inner node of the upper node has lost a follower
    /// It's expected that the upper node is flagged as "in progress"
    pub(super) fn notify_lost_follower<C: AggregationContext<NodeRef = I, Data = D>>(
        &mut self,
        ctx: &C,
        upper_id: &C::NodeRef,
        follower_id: &C::NodeRef,
    ) -> Option<PreparedNotifyLostFollower<C>> {
        let AggregationNode::Aggegating(aggregating) = self else {
            unreachable!();
        };
        match aggregating.followers.remove_if_entry(follower_id) {
            RemoveIfEntryResult::PartiallyRemoved => {
                self.finish_in_progress(ctx, upper_id);
                None
            }
            RemoveIfEntryResult::Removed => {
                let uppers = aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
                start_in_progress_all(ctx, &uppers);
                self.finish_in_progress(ctx, upper_id);
                Some(PreparedNotifyLostFollower::RemovedFollower {
                    uppers,
                    follower_id: follower_id.clone(),
                })
            }
            RemoveIfEntryResult::NotPresent => Some(PreparedNotifyLostFollower::NotFollower {
                upper_id: upper_id.clone(),
                follower_id: follower_id.clone(),
            }),
        }
    }
}

pub(super) enum PreparedNotifyLostFollower<C: AggregationContext> {
    RemovedFollower {
        uppers: StackVec<C::NodeRef>,
        follower_id: C::NodeRef,
    },
    NotFollower {
        upper_id: C::NodeRef,
        follower_id: C::NodeRef,
    },
}

impl<C: AggregationContext> PreparedOperation<C> for PreparedNotifyLostFollower<C> {
    type Result = ();
    fn apply(self, ctx: &C) {
        match self {
            PreparedNotifyLostFollower::RemovedFollower {
                uppers,
                follower_id,
            } => {
                for upper_id in uppers {
                    notify_lost_follower(ctx, ctx.node(&upper_id), &upper_id, &follower_id);
                }
            }
            PreparedNotifyLostFollower::NotFollower {
                upper_id,
                follower_id,
            } => {
                let mut try_start: Option<Instant> = None;
                loop {
                    let mut follower = ctx.node(&follower_id);
                    match follower.uppers_mut().remove_if_entry(&upper_id) {
                        RemoveIfEntryResult::PartiallyRemoved => {
                            drop(follower);
                            finish_in_progress_without_node(ctx, &upper_id);
                            return;
                        }
                        RemoveIfEntryResult::Removed => {
                            let remove_change = get_aggregated_remove_change(ctx, &follower);
                            let followers = match &*follower {
                                AggregationNode::Leaf { .. } => {
                                    follower.children().collect::<StackVec<_>>()
                                }
                                AggregationNode::Aggegating(aggregating) => {
                                    let AggegatingNode { ref followers, .. } = **aggregating;
                                    followers.iter().cloned().collect::<StackVec<_>>()
                                }
                            };
                            drop(follower);

                            let mut upper = ctx.node(&upper_id);
                            let remove_change = remove_change
                                .map(|remove_change| upper.apply_change(ctx, remove_change));
                            start_in_progress_count(ctx, &upper_id, followers.len() as u32);
                            let prepared = followers
                                .into_iter()
                                .filter_map(|follower_id| {
                                    upper.notify_lost_follower(ctx, &upper_id, &follower_id)
                                })
                                .collect::<StackVec<_>>();
                            upper.finish_in_progress(ctx, &upper_id);
                            drop(upper);
                            prepared.apply(ctx);
                            remove_change.apply(ctx);
                            return;
                        }
                        RemoveIfEntryResult::NotPresent => {
                            drop(follower);
                            let mut upper = ctx.node(&upper_id);
                            let AggregationNode::Aggegating(aggregating) = &mut *upper else {
                                unreachable!();
                            };
                            match aggregating.followers.remove_if_entry(&follower_id) {
                                RemoveIfEntryResult::PartiallyRemoved => {
                                    upper.finish_in_progress(ctx, &upper_id);
                                    return;
                                }
                                RemoveIfEntryResult::Removed => {
                                    let uppers =
                                        aggregating.uppers.iter().cloned().collect::<StackVec<_>>();
                                    start_in_progress_all(ctx, &uppers);
                                    upper.finish_in_progress(ctx, &upper_id);
                                    drop(upper);
                                    for upper_id in uppers {
                                        notify_lost_follower(
                                            ctx,
                                            ctx.node(&upper_id),
                                            &upper_id,
                                            &follower_id,
                                        );
                                    }
                                    return;
                                }
                                RemoveIfEntryResult::NotPresent => {
                                    drop(upper);
                                    if let Some(try_start) = &try_start {
                                        if try_start.elapsed().as_millis() > 10000 {
                                            panic!(
                                                "The graph is malformed, we need to remove either \
                                                 follower or upper but neither exists."
                                            );
                                        }
                                    } else {
                                        try_start = Some(Instant::now());
                                    }
                                    yield_now()
                                    // Retry, concurrency
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn notify_lost_follower<C: AggregationContext>(
    ctx: &C,
    mut upper: C::Guard<'_>,
    upper_id: &C::NodeRef,
    follower_id: &C::NodeRef,
) {
    let p = upper.notify_lost_follower(ctx, upper_id, follower_id);
    drop(upper);
    p.apply(ctx);
}

fn find_path<C: AggregationContext>(
    ctx: &C,
    start_id: &C::NodeRef,
    end_id: &C::NodeRef,
) -> Result<Vec<C::NodeRef>> {
    let mut queue = vec![(start_id.clone(), vec![])];
    while let Some((node_id, mut path)) = queue.pop() {
        let node = ctx.node(&node_id);
        if node_id == *end_id {
            path.push(node_id);
            return Ok(path);
        }
        for child_id in node.children() {
            let mut new_path = path.clone();
            new_path.push(node_id.clone());
            queue.push((child_id, new_path));
        }
    }
    bail!("No path found");
}
