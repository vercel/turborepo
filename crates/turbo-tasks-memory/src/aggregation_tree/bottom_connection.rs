use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use auto_hash_map::{map::RawEntry, AutoMap};
use nohash_hasher::{BuildNoHashHasher, IsEnabled};

use super::{
    bottom_tree::BottomTree,
    inner_refs::{BottomRef, ChildLocation},
    AggregationContext,
};

struct BottomRefInfo {
    count: isize,
    distance: u8,
}

pub struct DistanceCountMap<T: IsEnabled> {
    map: AutoMap<T, BottomRefInfo, BuildNoHashHasher<T>>,
}

impl<T: IsEnabled + Eq + Hash + Clone> DistanceCountMap<T> {
    pub fn new() -> Self {
        Self {
            map: AutoMap::with_hasher(),
        }
    }

    pub fn is_unset(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&T, u8)> {
        self.map
            .iter()
            .filter(|(_, info)| info.count > 0)
            .map(|(item, &BottomRefInfo { distance, .. })| (item, distance))
    }

    pub fn add_clonable(&mut self, item: &T, distance: u8) -> bool {
        match self.map.raw_entry_mut(item) {
            RawEntry::Occupied(e) => {
                let info = e.into_mut();
                info.count += 1;
                if distance < info.distance {
                    info.distance = distance;
                }
                false
            }
            RawEntry::Vacant(e) => {
                e.insert(item.clone(), BottomRefInfo { count: 1, distance });
                true
            }
        }
    }

    pub fn remove_clonable(&mut self, item: &T) -> bool {
        match self.map.raw_entry_mut(item) {
            RawEntry::Occupied(mut e) => {
                let info = e.get_mut();
                info.count -= 1;
                if info.count == 0 {
                    e.remove();
                    true
                } else {
                    false
                }
            }
            RawEntry::Vacant(e) => {
                e.insert(
                    item.clone(),
                    BottomRefInfo {
                        count: -1,
                        distance: 0,
                    },
                );
                false
            }
        }
    }

    pub fn into_counts(self) -> impl Iterator<Item = (T, isize)> {
        self.map.into_iter().map(|(item, info)| (item, info.count))
    }
}

pub enum BottomConnection<T, I: IsEnabled> {
    Left(Arc<BottomTree<T, I>>),
    Inner(DistanceCountMap<BottomRef<T, I>>),
}

impl<T, I: IsEnabled> BottomConnection<T, I> {
    pub(super) fn new() -> Self {
        Self::Inner(DistanceCountMap::new())
    }

    pub(super) fn is_unset(&self) -> bool {
        match self {
            Self::Left(_) => false,
            Self::Inner(list) => list.is_unset(),
        }
    }

    pub(super) fn as_cloned_uppers(&self) -> BottomUppers<T, I> {
        match self {
            Self::Left(upper) => BottomUppers::Left(upper.clone()),
            Self::Inner(upper) => BottomUppers::Inner(
                upper
                    .iter()
                    .map(|(item, distance)| (item.clone(), distance))
                    .collect(),
            ),
        }
    }

    #[must_use]
    pub(super) fn set_left_upper(
        &mut self,
        upper: &Arc<BottomTree<T, I>>,
    ) -> DistanceCountMap<BottomRef<T, I>> {
        match std::mem::replace(self, BottomConnection::Left(upper.clone())) {
            BottomConnection::Left(_) => unreachable!("Can't have two left children"),
            BottomConnection::Inner(old_inner) => old_inner,
        }
    }

    pub(super) fn unset_left_upper(&mut self, upper: &Arc<BottomTree<T, I>>) {
        match std::mem::replace(self, BottomConnection::Inner(DistanceCountMap::new())) {
            BottomConnection::Left(old_upper) => {
                debug_assert!(Arc::ptr_eq(&old_upper, upper));
            }
            BottomConnection::Inner(_) => unreachable!("Must that a left child"),
        }
    }
}

impl<T, I: IsEnabled + Eq + Hash + Clone> BottomConnection<T, I> {
    pub(super) fn child_change<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        change: &C::ItemChange,
    ) {
        match self {
            BottomConnection::Left(upper) => {
                upper.child_change(context, change);
            }
            BottomConnection::Inner(list) => {
                for (BottomRef { upper }, _) in list.iter() {
                    upper.child_change(context, change);
                }
            }
        }
    }

    pub(super) fn get_root_info<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        root_info_type: &C::RootInfoType,
        mut result: C::RootInfo,
    ) -> C::RootInfo {
        match &self {
            BottomConnection::Left(upper) => {
                let info = upper.get_root_info(context, root_info_type);
                if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                    return result;
                }
            }
            BottomConnection::Inner(list) => {
                for (BottomRef { upper }, _) in list.iter() {
                    let info = upper.get_root_info(context, root_info_type);
                    if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                        return result;
                    }
                }
            }
        }
        result
    }
}

pub enum BottomUppers<T, I: IsEnabled> {
    Left(Arc<BottomTree<T, I>>),
    Inner(Vec<(BottomRef<T, I>, u8)>),
}

impl<T, I: IsEnabled + Eq + Hash + Clone> BottomUppers<T, I> {
    pub(super) fn add_children_of_child<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        children: impl IntoIterator<Item = (u32, &'a I)> + Clone,
    ) where
        I: 'a,
    {
        match self {
            BottomUppers::Left(upper) => {
                upper.add_children_of_child(context, ChildLocation::Left, children, 0);
            }
            BottomUppers::Inner(list) => {
                for &(BottomRef { ref upper }, nesting_level) in list {
                    upper.add_children_of_child(
                        context,
                        ChildLocation::Inner,
                        children.clone(),
                        nesting_level + 1,
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
    ) {
        match self {
            BottomUppers::Left(upper) => {
                upper.add_child_of_child(
                    context,
                    ChildLocation::Left,
                    child_of_child,
                    child_of_child_hash,
                    0,
                );
            }
            BottomUppers::Inner(list) => {
                for &(BottomRef { ref upper }, nesting_level) in list.iter() {
                    upper.add_child_of_child(
                        context,
                        ChildLocation::Inner,
                        child_of_child,
                        child_of_child_hash,
                        nesting_level + 1,
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
                for (BottomRef { upper }, _) in list {
                    upper.remove_child_of_child(context, child_of_child);
                }
            }
        }
    }

    pub(super) fn child_change<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        change: &C::ItemChange,
    ) {
        match self {
            BottomUppers::Left(upper) => {
                upper.child_change(context, change);
            }
            BottomUppers::Inner(list) => {
                for (BottomRef { upper }, _) in list {
                    upper.child_change(context, change);
                }
            }
        }
    }
}
