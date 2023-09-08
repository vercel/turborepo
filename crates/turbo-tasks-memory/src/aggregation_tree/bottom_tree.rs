use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use nohash_hasher::{BuildNoHashHasher, IsEnabled};
use parking_lot::{Mutex, MutexGuard};

use super::{
    inner_refs::{BottomRef, ChildLocation, TopRef},
    leaf::bottom_tree,
    top_tree::TopTree,
    AggregationContext, AggregationItemLock,
};
use crate::count_hash_set::{CountHashSet, RemoveIfEntryResult};

/// The bottom half of the aggregation tree. It aggregates items up the a
/// certain connectivity depending on the "height". Every level of the tree
/// aggregates the previous level.
///
/// The every level aggregates an item with all of its children plus potentially
/// all of their children. This depends on a flag called "is_blue" of the
/// child. A "blue" child will aggregate one additional layer of
/// connectivity, but this is not applies recursively. So every level of the
/// tree will aggregate a connectivity of 2 to 3.
///
/// It's assumed that the "is_blue" flag of an item is randomly distributed, but
/// deterministic.
///
/// The concept of "blue" nodes will improve the sharing of graphs as
/// aggregation will eventually propagate to use the same items, even if they
/// start on different depths of the graph.
pub struct BottomTree<T, I: IsEnabled> {
    height: u8,
    state: Mutex<BottomTreeState<T, I>>,
}

pub struct BottomTreeState<T, I: IsEnabled> {
    data: T,
    left_bottom_upper: Option<Arc<BottomTree<T, I>>>,
    inner_bottom_upper: CountHashSet<BottomRef<T, I>, BuildNoHashHasher<BottomRef<T, I>>>,
    top_upper: CountHashSet<TopRef<T>, BuildNoHashHasher<TopRef<T>>>,
    // TODO this can't become negative
    following: CountHashSet<I, BuildNoHashHasher<I>>,
}

impl<T: Default, I: IsEnabled> BottomTree<T, I> {
    pub fn new(height: u8) -> Self {
        Self {
            height,
            state: Mutex::new(BottomTreeState {
                data: T::default(),
                left_bottom_upper: None,
                inner_bottom_upper: CountHashSet::new(),
                top_upper: CountHashSet::new(),
                following: CountHashSet::new(),
            }),
        }
    }
}

struct SplitChildren<'a, I> {
    blue: Vec<(u32, &'a I)>,
    white: Vec<(u32, &'a I)>,
}

impl<T, I: Clone + Eq + Hash + IsEnabled> BottomTree<T, I> {
    fn is_blue(&self, hash: u32) -> bool {
        (hash.rotate_right(self.height as u32 * 2) & 3) == 0
    }

    fn split_children<'a>(
        &self,
        children: impl IntoIterator<Item = (u32, &'a I)>,
    ) -> SplitChildren<'a, I> {
        let children = children.into_iter();
        let size_hint = children.size_hint();
        let cap = size_hint.1.unwrap_or(size_hint.0);
        let mut blue = Vec::with_capacity(cap);
        let mut white = Vec::with_capacity(cap);
        for (hash, child) in children {
            if self.is_blue(hash) {
                blue.push((hash, child));
            } else {
                white.push((hash, child));
            }
        }
        if !white.is_empty() {
            let state = self.state.lock();
            white.retain(|&(hash, child)| {
                if state.following.get(child) > 0 {
                    blue.push((hash, child));
                    false
                } else {
                    true
                }
            });
        }
        SplitChildren { blue, white }
    }

    pub fn add_children_of_child<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        children: impl IntoIterator<Item = (u32, &'a I)>,
        nesting_level: u8,
    ) where
        I: 'a,
    {
        match child_location {
            ChildLocation::Left => {
                // the left child has new children
                // this means it's a inner child of this node
                // We always want to aggregate over at least connectivity 1
                self.add_children_of_child_inner(
                    context,
                    children.into_iter().map(|(_, c)| c),
                    nesting_level,
                );
            }
            ChildLocation::Inner => {
                // the inner child has new children
                // this means white children are inner children of this node
                // and blue children need to propagate up
                if nesting_level > 4 {
                    self.add_children_of_child_following(context, children.into_iter().collect());
                    return;
                }
                let SplitChildren { blue, mut white } = self.split_children(children);
                if !white.is_empty() {
                    self.add_children_of_child_if_following(&mut white);
                    self.add_children_of_child_inner(
                        context,
                        white.iter().map(|&(_, c)| c),
                        nesting_level,
                    );
                }
                if !blue.is_empty() {
                    self.add_children_of_child_following(context, blue);
                }
            }
        }
    }

    fn add_children_of_child_if_following(&self, children: &mut Vec<(u32, &I)>) {
        let mut state = self.state.lock();
        children.retain(|&(_, child)| !state.following.add_if_entry(child));
    }

    fn add_children_of_child_following<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        mut children: Vec<(u32, &I)>,
    ) {
        let mut left_bottom_upper = None;
        let mut inner_bottom_uppers = Vec::new();
        let mut state = self.state.lock();
        children.retain(|&(_, child)| state.following.add(child.clone()));
        if children.is_empty() {
            return;
        }
        left_bottom_upper = state.left_bottom_upper.clone();
        inner_bottom_uppers.extend(state.inner_bottom_upper.iter().cloned());
        for TopRef { upper } in state.top_upper.iter() {
            upper.add_children_of_child(context, children.iter().map(|&(_, c)| c));
        }
        drop(state);
        if let Some(upper) = left_bottom_upper {
            upper.add_children_of_child(context, ChildLocation::Left, children.iter().copied(), 0);
        }
        for BottomRef { upper } in inner_bottom_uppers {
            upper.add_children_of_child(context, ChildLocation::Inner, children.iter().copied(), 0);
        }
    }

    fn add_children_of_child_inner<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        children: impl IntoIterator<Item = &'a I>,
        nesting_level: u8,
    ) where
        I: 'a,
    {
        if self.height == 0 {
            for child in children {
                add_inner_upper_to_item(context, child, &self, nesting_level);
            }
        } else {
            for child in children {
                bottom_tree(context, child, self.height - 1).add_inner_bottom_tree_upper(
                    context,
                    &self,
                    nesting_level,
                );
            }
        }
    }

    pub fn add_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_of_child: &I,
        child_of_child_hash: u32,
        nesting_level: u8,
    ) {
        let is_blue = self.is_blue(child_of_child_hash);
        match (child_location, is_blue) {
            (ChildLocation::Left, _) => {
                // the left child has a new child
                // this means it's a inner child of this node
                // We always want to aggregate over at least connectivity 1
                self.add_child_of_child_inner(context, child_of_child, nesting_level);
            }
            (ChildLocation::Inner, false) if nesting_level <= 4 => {
                // the inner child has a new child
                // but it's not a blue node and we are not too deep
                // this means it's a inner child of this node
                // if it's not already a following child
                if !self.add_child_of_child_if_following(child_of_child) {
                    self.add_child_of_child_inner(context, child_of_child, nesting_level);
                }
            }
            (ChildLocation::Inner, _) => {
                // the inner child has a new child
                // this means we need to propagate the change up
                // and store them in our own list
                self.add_child_of_child_following(context, child_of_child, child_of_child_hash);
            }
        }
    }

    fn add_child_of_child_if_following(&self, child_of_child: &I) -> bool {
        let mut state = self.state.lock();
        state.following.add_if_entry(child_of_child)
    }

    fn add_child_of_child_following<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
        child_of_child_hash: u32,
    ) {
        let mut state = self.state.lock();
        if !state.following.add(child_of_child.clone()) {
            // Already connect, nothing more to do
            return;
        }

        // TODO we want to check if child_of_child is already connected as inner child
        // and convert that that
        let left_bottom_upper = state.left_bottom_upper.clone();
        let inner_bottom_uppers = state.inner_bottom_upper.iter().cloned().collect::<Vec<_>>();
        for TopRef { upper } in state.top_upper.iter() {
            upper.add_child_of_child(context, child_of_child);
        }
        drop(state);
        if let Some(upper) = left_bottom_upper {
            upper.add_child_of_child(
                context,
                ChildLocation::Left,
                child_of_child,
                child_of_child_hash,
                0,
            );
        }
        for BottomRef { upper } in inner_bottom_uppers {
            upper.add_child_of_child(
                context,
                ChildLocation::Inner,
                child_of_child,
                child_of_child_hash,
                0,
            );
        }
    }

    fn add_child_of_child_inner<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
        nesting_level: u8,
    ) {
        if self.height == 0 {
            add_inner_upper_to_item(context, child_of_child, &self, nesting_level);
        } else {
            bottom_tree(context, child_of_child, self.height - 1).add_inner_bottom_tree_upper(
                context,
                &self,
                nesting_level,
            );
        }
    }

    pub fn remove_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
    ) {
        if !self.remove_child_of_child_if_following(context, child_of_child) {
            self.remove_child_of_child_inner(context, child_of_child);
        }
    }

    fn remove_child_of_child_if_following<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
    ) -> bool {
        let mut left_bottom_upper = None;
        let mut inner_bottom_upper = Vec::new();
        let mut state = self.state.lock();
        match state.following.remove_if_entry(child_of_child) {
            RemoveIfEntryResult::PartiallyRemoved => return true,
            RemoveIfEntryResult::NotPresent => return false,
            RemoveIfEntryResult::Removed => {
                left_bottom_upper = state.left_bottom_upper.clone();
                inner_bottom_upper.extend(state.inner_bottom_upper.iter().cloned());
                for TopRef { upper } in state.top_upper.iter() {
                    upper.remove_child_of_child(context, child_of_child);
                }
            }
        }
        drop(state);
        if let Some(upper) = left_bottom_upper {
            upper.remove_child_of_child(context, child_of_child);
        }
        for BottomRef { upper } in inner_bottom_upper {
            upper.remove_child_of_child(context, child_of_child);
        }
        true
    }

    fn remove_child_of_child_inner<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
    ) {
        if self.height == 0 {
            remove_inner_upper_from_item(context, child_of_child, &self);
        } else {
            bottom_tree(context, child_of_child, self.height - 1)
                .remove_inner_bottom_tree_upper(context, &self);
        }
    }

    pub(super) fn add_left_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
    ) {
        let mut state = self.state.lock();
        state.left_bottom_upper = Some(upper.clone());
        if let Some(change) = context.info_to_add_change(&state.data) {
            upper.child_change(context, &change);
        }
        let children = state.following.iter().cloned().collect::<Vec<_>>();
        drop(state);
        let children = children.iter().map(|item| (context.hash(item), item));
        upper.add_children_of_child(context, ChildLocation::Left, children, 1);
    }

    pub(super) fn add_inner_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
        nesting_level: u8,
    ) {
        let mut state = self.state.lock();
        let new = state.inner_bottom_upper.add(BottomRef {
            upper: upper.clone(),
        });
        if new {
            if let Some(change) = context.info_to_add_change(&state.data) {
                upper.child_change(context, &change);
            }
            let children = state.following.iter().cloned().collect::<Vec<_>>();
            drop(state);
            let children = children.iter().map(|item| (context.hash(item), item));
            upper.add_children_of_child(context, ChildLocation::Inner, children, nesting_level + 1);
        }
    }

    pub(super) fn remove_inner_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
    ) {
        let mut state = self.state.lock();
        let removed = state.inner_bottom_upper.remove(BottomRef {
            upper: upper.clone(),
        });
        if removed {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                upper.child_change(context, &change);
            }
            for following in state.following.iter() {
                upper.remove_child_of_child(context, following);
            }
        }
    }

    pub(super) fn add_top_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<TopTree<T>>,
    ) {
        let mut state = self.state.lock();
        let new = state.top_upper.add(TopRef {
            upper: upper.clone(),
        });
        if new {
            if let Some(change) = context.info_to_add_change(&state.data) {
                upper.child_change(context, &change);
            }
            for following in state.following.iter() {
                upper.add_child_of_child(context, following);
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn remove_top_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<TopTree<T>>,
    ) {
        let mut state = self.state.lock();
        let removed = state.top_upper.remove(TopRef {
            upper: upper.clone(),
        });
        if removed {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                upper.child_change(context, &change);
            }
            for following in state.following.iter() {
                upper.remove_child_of_child(context, following);
            }
        }
    }

    pub(super) fn child_change<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        change: &C::ItemChange,
    ) {
        let mut state = self.state.lock();
        let change = context.apply_change(&mut state.data, change);
        propagate_change_to_upper(&mut state, context, change);
    }

    pub(super) fn get_root_info<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        root_info_type: &C::RootInfoType,
    ) -> C::RootInfo {
        let mut result = context.new_root_info(root_info_type);
        let state = self.state.lock();
        for TopRef { upper } in state.top_upper.iter() {
            let info = upper.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                break;
            }
        }
        if let Some(upper) = state.left_bottom_upper.as_ref() {
            let info = upper.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                return result;
            }
        }
        for BottomRef { upper } in state.inner_bottom_upper.iter() {
            let info = upper.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                break;
            }
        }
        result
    }
}

fn propagate_change_to_upper<C: AggregationContext>(
    state: &mut MutexGuard<BottomTreeState<C::Info, C::ItemRef>>,
    context: &C,
    change: Option<C::ItemChange>,
) {
    let Some(change) = change else {
        return;
    };
    for BottomRef { upper } in state.inner_bottom_upper.iter() {
        upper.child_change(context, &change);
    }
    if let Some(upper) = state.left_bottom_upper.as_ref() {
        upper.child_change(context, &change);
    }
    for TopRef { upper } in state.top_upper.iter() {
        upper.child_change(context, &change);
    }
}

pub fn add_inner_upper_to_item<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    nesting_level: u8,
) {
    let (change, children) = {
        let mut item = context.item(reference);
        if item.leaf().add_upper(upper, ChildLocation::Inner) {
            let change = item.get_add_change();
            (
                change,
                item.children().map(|r| r.into_owned()).collect::<Vec<_>>(),
            )
        } else {
            return;
        }
    };
    if let Some(change) = change {
        context.on_add_change(&change);
        upper.child_change(context, &change);
    }
    if !children.is_empty() {
        upper.add_children_of_child(
            context,
            ChildLocation::Inner,
            children.iter().map(|child| (context.hash(&child), child)),
            nesting_level + 1,
        )
    }
}

#[must_use]
pub fn add_left_upper_to_item_step_1<C: AggregationContext>(
    item: &mut C::ItemLock<'_>,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) -> (Option<C::ItemChange>, Vec<C::ItemRef>) {
    if item.leaf().add_upper(upper, ChildLocation::Left) {
        let change = item.get_add_change();
        (change, item.children().map(|r| r.into_owned()).collect())
    } else {
        (None, Vec::new())
    }
}

pub fn add_left_upper_to_item_step_2<C: AggregationContext>(
    context: &C,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    step_1_result: (Option<C::ItemChange>, Vec<C::ItemRef>),
) {
    let (change, children) = step_1_result;
    if let Some(change) = change {
        context.on_add_change(&change);
        upper.child_change(context, &change);
    }
    if !children.is_empty() {
        upper.add_children_of_child(
            context,
            ChildLocation::Left,
            children.iter().map(|child| (context.hash(&child), child)),
            1,
        )
    }
}

pub fn remove_inner_upper_from_item<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
) {
    let (change, children) = {
        let mut item = context.item(reference);
        if item.leaf().remove_inner_upper(upper) {
            let change = item.get_remove_change();
            (
                change,
                item.children().map(|r| r.into_owned()).collect::<Vec<_>>(),
            )
        } else {
            return;
        }
    };
    if let Some(change) = change {
        context.on_remove_change(&change);
        upper.child_change(context, &change);
    }
    for child in children {
        upper.remove_child_of_child(context, &child)
    }
}
