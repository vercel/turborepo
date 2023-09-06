use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use nohash_hasher::{BuildNoHashHasher, IsEnabled};
use parking_lot::{Mutex, MutexGuard};

use super::{
    inner_refs::{BottomRef, ChildLocation, TopRef},
    leaf::bottom_tree,
    top_tree::TopTree,
    AggregationContext, AggregationItemLock,
};
use crate::count_hash_set::CountHashSet;

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
    is_blue: bool,
    state: Mutex<BottomTreeState<T, I>>,
}

pub struct BottomTreeState<T, I: IsEnabled> {
    data: T,
    left_bottom_upper: Option<Arc<BottomTree<T, I>>>,
    inner_bottom_upper: CountHashSet<BottomRef<T, I>, BuildNoHashHasher<BottomRef<T, I>>>,
    top_upper: CountHashSet<TopRef<T>, BuildNoHashHasher<TopRef<T>>>,
    /// Items that are referenced by right children of this node.
    following: CountHashSet<I, BuildNoHashHasher<I>>,
}

impl<T: Default, I: IsEnabled> BottomTree<T, I> {
    pub fn new(height: u8, is_blue: bool) -> Self {
        Self {
            height,
            is_blue,
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

impl<T, I: Clone + Eq + Hash + IsEnabled> BottomTree<T, I> {
    fn is_blue(self: &Arc<Self>, hash: u32) -> bool {
        hash >> self.height & 1 == 0
    }

    pub fn add_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_of_child: &I,
        child_of_child_hash: u32,
    ) {
        let is_blue = self.is_blue(child_of_child_hash);
        match (child_location, is_blue) {
            (ChildLocation::Left, _) | (ChildLocation::Inner, false) => {
                // the left child has a new child
                // or the inner child has a new child
                // but it's not a blue node
                // this means it's a inner child of this node
                self.add_child_of_child_inner(context, child_of_child);
            }
            (ChildLocation::Inner, true) => {
                // the inner child has a new child
                // this means we need to propagate the change up
                // and store them in our own list
                self.add_child_of_child_following(context, child_of_child, child_of_child_hash);
            }
        }
    }

    fn add_child_of_child_following<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
        child_of_child_hash: u32,
    ) {
        let mut left_bottom_upper = None;
        let mut inner_bottom_uppers = Vec::new();
        let mut state = self.state.lock();
        if state.following.add(child_of_child.clone()) {
            left_bottom_upper = state.left_bottom_upper.clone();
            inner_bottom_uppers.extend(state.inner_bottom_upper.iter().cloned());
            for TopRef { upper } in state.top_upper.iter() {
                upper.add_child_of_child(context, child_of_child);
            }
        }
        drop(state);
        if let Some(upper) = left_bottom_upper {
            upper.add_child_of_child(
                context,
                ChildLocation::Left,
                child_of_child,
                child_of_child_hash,
            );
        }
        for BottomRef { upper } in inner_bottom_uppers {
            upper.add_child_of_child(
                context,
                ChildLocation::Inner,
                child_of_child,
                child_of_child_hash,
            );
        }
    }

    fn add_child_of_child_inner<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
    ) {
        if self.height == 0 {
            add_upper_to_item_ref(context, child_of_child, &self, ChildLocation::Inner);
        } else {
            bottom_tree(context, child_of_child, self.height - 1).add_bottom_tree_upper(
                context,
                &self,
                ChildLocation::Inner,
            );
        }
    }

    pub fn remove_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_of_child: &I,
        child_of_child_hash: u32,
    ) {
        let is_blue = self.is_blue(child_of_child_hash);
        match (child_location, is_blue) {
            (ChildLocation::Left, _) | (ChildLocation::Inner, false) => {
                // the left/inner child has lost a child
                // this means this node has lost a inner child
                self.remove_child_of_child_inner(context, child_of_child, child_of_child_hash);
            }
            (ChildLocation::Inner, true) => {
                // the inner blue child has lost a child
                // this means we need to propagate the change up
                // and remove them from our own list
                self.remove_child_of_child_following(context, child_of_child, child_of_child_hash);
            }
        }
    }

    fn remove_child_of_child_following<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
        child_of_child_hash: u32,
    ) {
        let mut left_bottom_upper = None;
        let mut inner_bottom_upper = Vec::new();
        let mut state = self.state.lock();
        if state.following.remove(child_of_child.clone()) {
            left_bottom_upper = state.left_bottom_upper.clone();
            inner_bottom_upper.extend(state.inner_bottom_upper.iter().cloned());
            for TopRef { upper } in state.top_upper.iter() {
                upper.remove_child_of_child(context, child_of_child);
            }
        }
        drop(state);
        if let Some(upper) = left_bottom_upper {
            upper.remove_child_of_child(
                context,
                ChildLocation::Left,
                child_of_child,
                child_of_child_hash,
            );
        }
        for BottomRef { upper } in inner_bottom_upper {
            upper.remove_child_of_child(
                context,
                ChildLocation::Inner,
                child_of_child,
                child_of_child_hash,
            );
        }
    }

    fn remove_child_of_child_inner<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
        child_of_child_hash: u32,
    ) {
        if self.height == 0 {
            remove_upper_from_item_ref(context, child_of_child, &self, ChildLocation::Inner);
        } else {
            bottom_tree(context, child_of_child, self.height - 1).remove_bottom_tree_parent(
                context,
                &self,
                ChildLocation::Inner,
            );
        }
    }

    pub(super) fn add_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) {
        let mut state = self.state.lock();
        let new = match location {
            ChildLocation::Left => {
                state.left_bottom_upper = Some(upper.clone());
                true
            }
            ChildLocation::Inner => state.inner_bottom_upper.add(BottomRef {
                upper: upper.clone(),
            }),
        };
        if new {
            if let Some(change) = context.info_to_add_change(&state.data) {
                upper.child_change(context, &change);
            }
            for following in state.following.iter() {
                upper.add_child_of_child(context, location, following, context.hash(following));
            }
        }
    }

    pub(super) fn remove_bottom_tree_parent<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) {
        let mut state = self.state.lock();
        debug_assert!(matches!(location, ChildLocation::Inner));
        let removed = state.inner_bottom_upper.remove(BottomRef {
            upper: upper.clone(),
        });
        if removed {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                upper.child_change(context, &change);
            }
            for following in state.following.iter() {
                upper.remove_child_of_child(context, location, following, context.hash(following));
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

pub fn add_upper_to_item_ref<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) {
    let result = {
        let mut item = context.item(reference);
        add_upper_to_item_step_1::<C>(&mut item, upper, location)
    };
    add_upper_to_item_step_2(context, upper, result);
}

#[must_use]
pub fn add_upper_to_item_step_1<C: AggregationContext>(
    item: &mut C::ItemLock<'_>,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) -> (Option<C::ItemChange>, ChildLocation, Vec<C::ItemRef>) {
    if item.leaf().add_upper(upper, location) {
        let change = item.get_add_change();
        (
            change,
            location,
            item.children().map(|r| r.into_owned()).collect(),
        )
    } else {
        (None, location, Vec::new())
    }
}

pub fn add_upper_to_item_step_2<C: AggregationContext>(
    context: &C,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    step_1_result: (Option<C::ItemChange>, ChildLocation, Vec<C::ItemRef>),
) {
    let (change, location, children) = step_1_result;
    if let Some(change) = change {
        context.on_add_change(&change);
        upper.child_change(context, &change);
    }
    for child in children {
        upper.add_child_of_child(context, location, &child, context.hash(&child))
    }
}

pub fn remove_upper_from_item_ref<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) {
    let result = {
        let mut item = context.item(reference);
        remove_upper_from_item_step_1::<C>(&mut item, upper, location)
    };
    remove_upper_from_item_step_2(context, upper, result);
}

#[must_use]
pub fn remove_upper_from_item_step_1<C: AggregationContext>(
    item: &mut C::ItemLock<'_>,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) -> (Option<C::ItemChange>, Vec<C::ItemRef>) {
    if item.leaf().remove_upper(upper, location) {
        let change = item.get_remove_change();
        (change, item.children().map(|r| r.into_owned()).collect())
    } else {
        (None, Vec::new())
    }
}

pub fn remove_upper_from_item_step_2<C: AggregationContext>(
    context: &C,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    step_1_result: (Option<C::ItemChange>, Vec<C::ItemRef>),
) {
    let (change, children) = step_1_result;
    if let Some(change) = change {
        context.on_remove_change(&change);
        upper.child_change(context, &change);
    }
    for child in children {
        upper.remove_child_of_child(context, ChildLocation::Inner, &child, context.hash(&child))
    }
}
