use std::{hash::Hash, ops::DerefMut};

use smallvec::SmallVec;

use crate::count_hash_set::CountHashSet;

mod add_followers;
mod aggregation_data;
mod change;
mod increase;
mod lost_edge;
mod new_edge;
mod notify_aggregation_number_changed;
mod notify_lost_follower;
mod notify_new_follower;
mod remove_followers;
mod root_query;
#[cfg(test)]
mod tests;
mod uppers;

pub use aggregation_data::{aggregation_data, prepare_aggregation_data, AggregationDataGuard};
pub use change::apply_change;
pub use increase::increase_aggregation_number;
pub(self) use notify_aggregation_number_changed::notify_aggregation_number_changed;
pub(self) use notify_lost_follower::notify_lost_follower;
pub(self) use notify_new_follower::notify_new_follower;
pub use root_query::{query_root_info, RootQuery};

type StackVec<I> = SmallVec<[I; 16]>;

pub enum AggregationNode<I, D> {
    Leaf {
        aggregation_number: u8,
        uppers: CountHashSet<I>,
    },
    Aggegating(Box<AggegatingNode<I, D>>),
}

impl<I, D> AggregationNode<I, D> {
    pub fn new() -> Self {
        Self::Leaf {
            aggregation_number: 0,
            uppers: CountHashSet::new(),
        }
    }
}

pub struct AggegatingNode<I, D> {
    aggregation_number: u32,
    uppers: CountHashSet<I>,
    followers: CountHashSet<I>,
    data: D,
}

impl<I, A> AggregationNode<I, A> {
    fn aggregation_number(&self) -> u32 {
        match self {
            AggregationNode::Leaf {
                aggregation_number, ..
            } => *aggregation_number as u32,
            AggregationNode::Aggegating(aggegating) => aggegating.aggregation_number,
        }
    }

    fn uppers(&self) -> &CountHashSet<I> {
        match self {
            AggregationNode::Leaf { uppers, .. } => uppers,
            AggregationNode::Aggegating(aggegating) => &aggegating.uppers,
        }
    }

    fn uppers_mut(&mut self) -> &mut CountHashSet<I> {
        match self {
            AggregationNode::Leaf { uppers, .. } => uppers,
            AggregationNode::Aggegating(aggegating) => &mut aggegating.uppers,
        }
    }
}

#[must_use]
pub trait PreparedOperation<C: AggregationContext> {
    fn apply(self, ctx: &C);
}

impl<C: AggregationContext, T: PreparedOperation<C>> PreparedOperation<C> for Option<T> {
    fn apply(self, ctx: &C) {
        if let Some(prepared) = self {
            prepared.apply(ctx);
        }
    }
}

impl<C: AggregationContext, T: PreparedOperation<C>> PreparedOperation<C> for Vec<T> {
    fn apply(self, ctx: &C) {
        for prepared in self {
            prepared.apply(ctx);
        }
    }
}

impl<C: AggregationContext, T: PreparedOperation<C>, const N: usize> PreparedOperation<C>
    for SmallVec<[T; N]>
{
    fn apply(self, ctx: &C) {
        for prepared in self {
            prepared.apply(ctx);
        }
    }
}

pub trait AggregationContext {
    type NodeRef: Clone + Eq + Hash;
    type Guard<'l>: AggregationNodeGuard<
        NodeRef = Self::NodeRef,
        Data = Self::Data,
        DataChange = Self::DataChange,
    >
    where
        Self: 'l;
    type Data;
    type DataChange;

    /// Gets mutable access to an item.
    fn node<'l>(&'l self, id: &Self::NodeRef) -> Self::Guard<'l>;

    /// Apply a changeset to an aggregated info object. Returns a new changeset
    /// that should be applied to the next aggregation level. Might return None,
    /// if no change should be applied to the next level.
    fn apply_change(
        &self,
        info: &mut Self::Data,
        change: &Self::DataChange,
    ) -> Option<Self::DataChange>;

    /// Creates a changeset from an aggregated info object, that represents
    /// adding the aggregated node to an aggregated node of the next level.
    fn data_to_add_change(&self, data: &Self::Data) -> Option<Self::DataChange>;
    /// Creates a changeset from an aggregated info object, that represents
    /// removing the aggregated node from an aggregated node of the next level.
    fn data_to_remove_change(&self, data: &Self::Data) -> Option<Self::DataChange>;
}

pub trait AggregationNodeGuard:
    DerefMut<Target = AggregationNode<Self::NodeRef, Self::Data>>
{
    type NodeRef: Clone + Eq + Hash;
    type Data;
    type DataChange;

    type ChildrenIter<'a>: Iterator<Item = Self::NodeRef> + 'a
    where
        Self: 'a;

    /// Returns the number of children.
    fn number_of_children(&self) -> usize;
    /// Returns an iterator over the children.
    fn children(&self) -> Self::ChildrenIter<'_>;
    /// Returns a changeset that represents the addition of the node.
    fn get_add_change(&self) -> Option<Self::DataChange>;
    /// Returns a changeset that represents the removal of the node.
    fn get_remove_change(&self) -> Option<Self::DataChange>;
    /// Returns the aggregated data which contains only that node
    fn get_initial_data(&self) -> Self::Data;
}
