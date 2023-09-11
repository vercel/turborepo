use std::{hash::Hash, sync::Arc};

use nohash_hasher::{BuildNoHashHasher, IsEnabled};

use super::{
    bottom_tree::BottomTree,
    inner_refs::{BottomRef, ChildLocation},
    AggregationContext,
};
use crate::count_hash_set::CountHashSet;

pub enum BottomConnection<T, I: IsEnabled> {
    Left(Arc<BottomTree<T, I>>),
    Inner(CountHashSet<BottomRef<T, I>, BuildNoHashHasher<BottomRef<T, I>>>),
}

impl<T, I: IsEnabled> BottomConnection<T, I> {
    pub(super) fn as_cloned_uppers(&self) -> BottomUppers<T, I> {
        match self {
            Self::Left(upper) => BottomUppers::Left(upper.clone()),
            Self::Inner(upper) => BottomUppers::Inner(upper.iter().cloned().collect()),
        }
    }

    pub(super) fn set_left_upper(
        &mut self,
        upper: &Arc<BottomTree<T, I>>,
    ) -> CountHashSet<BottomRef<T, I>, BuildNoHashHasher<BottomRef<T, I>>> {
        match std::mem::replace(self, BottomConnection::Left(upper.clone())) {
            BottomConnection::Left(_) => unreachable!("Can't have two left children"),
            BottomConnection::Inner(old_inner) => old_inner,
        }
    }

    pub(super) fn unset_left_upper(&mut self, upper: &Arc<BottomTree<T, I>>) {
        match std::mem::replace(self, BottomConnection::Inner(CountHashSet::new())) {
            BottomConnection::Left(old_upper) => {
                debug_assert!(Arc::ptr_eq(&old_upper, upper));
            }
            BottomConnection::Inner(_) => unreachable!("Must that a left child"),
        }
    }
}

pub enum BottomUppers<T, I: IsEnabled> {
    Left(Arc<BottomTree<T, I>>),
    Inner(Vec<BottomRef<T, I>>),
}

impl<T, I: IsEnabled + Eq + Hash + Clone> BottomUppers<T, I> {
    pub(super) fn add_children_of_child<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        children: impl IntoIterator<Item = (u32, &'a I)> + Clone,
        nesting_level: u8,
    ) where
        I: 'a,
    {
        match self {
            BottomUppers::Left(upper) => {
                upper.add_children_of_child(context, ChildLocation::Left, children, nesting_level);
            }
            BottomUppers::Inner(list) => {
                for BottomRef { upper } in list {
                    upper.add_children_of_child(
                        context,
                        ChildLocation::Inner,
                        children.clone(),
                        nesting_level,
                    );
                }
            }
        }
    }

    pub(super) fn add_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        child_of_child: &I,
        child_of_child_hash: u32,
        nesting_level: u8,
    ) {
        match self {
            BottomUppers::Left(upper) => {
                upper.add_child_of_child(
                    context,
                    ChildLocation::Left,
                    child_of_child,
                    child_of_child_hash,
                    nesting_level,
                );
            }
            BottomUppers::Inner(list) => {
                for BottomRef { upper } in list {
                    upper.add_child_of_child(
                        context,
                        ChildLocation::Inner,
                        child_of_child,
                        child_of_child_hash,
                        nesting_level,
                    );
                }
            }
        }
    }

    pub(super) fn remove_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        child_of_child: &I,
    ) {
        match self {
            BottomUppers::Left(upper) => {
                upper.remove_child_of_child(context, child_of_child);
            }
            BottomUppers::Inner(list) => {
                for BottomRef { upper } in list {
                    upper.remove_child_of_child(context, child_of_child);
                }
            }
        }
    }
}
