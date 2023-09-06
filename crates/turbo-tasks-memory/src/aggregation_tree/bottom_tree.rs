use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use nohash_hasher::{BuildNoHashHasher, IsEnabled};
use parking_lot::{Mutex, MutexGuard};

use super::{
    inner_refs::{ChildLocation, TopRef},
    leaf::bottom_tree,
    top_tree::TopTree,
    upper_map::UpperMap,
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
    state: Mutex<BottomTreeState<T, I>>,
}

pub struct BottomTreeState<T, I: IsEnabled> {
    data: T,
    bottom_upper: UpperMap<BottomTree<T, I>>,
    top_upper: CountHashSet<TopRef<T>>,
    /// Items that are referenced by right children of this node.
    following: CountHashSet<I, BuildNoHashHasher<I>>,
}

impl<T: Default, I: IsEnabled> BottomTree<T, I> {
    pub fn new(height: u8) -> Self {
        Self {
            height,
            state: Mutex::new(BottomTreeState {
                data: T::default(),
                bottom_upper: UpperMap::new(),
                top_upper: CountHashSet::new(),
                following: CountHashSet::new(),
            }),
        }
    }
}

impl<T, I: Clone + Eq + Hash + IsEnabled> BottomTree<T, I> {
    pub fn add_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_is_blue: bool,
        child_of_child: &I,
    ) {
        match (child_location, child_is_blue) {
            (ChildLocation::Left, false) | (ChildLocation::Middle, _) => {
                // the left/middle child has a new child
                // this means it's a right child of this node
                {
                    let mut state = self.state.lock();
                    if state.following.remove(child_of_child.clone()) {
                        for (parent, location) in state.bottom_upper.iter() {
                            parent.remove_child_of_child(
                                context,
                                location,
                                context.is_blue(&child_of_child),
                                child_of_child,
                            );
                        }
                        for TopRef { parent } in state.top_upper.iter() {
                            parent.remove_child_of_child(context, child_of_child);
                        }
                    }
                }
                if self.height == 0 {
                    add_parent_to_item_ref(context, child_of_child, &self, ChildLocation::Right);
                } else {
                    bottom_tree(context, child_of_child, self.height - 1).add_bottom_tree_parent(
                        context,
                        &self,
                        ChildLocation::Right,
                    );
                }
            }
            (ChildLocation::Left, true) => {
                // the left child has a new child
                // and the left child is a blue node
                // this means it's a middle child of this node
                {
                    let mut state = self.state.lock();
                    if state.following.remove(child_of_child.clone()) {
                        for (parent, location) in state.bottom_upper.iter() {
                            parent.remove_child_of_child(
                                context,
                                location,
                                context.is_blue(&child_of_child),
                                child_of_child,
                            );
                        }
                        for TopRef { parent } in state.top_upper.iter() {
                            parent.remove_child_of_child(context, child_of_child);
                        }
                    }
                }
                if self.height == 0 {
                    add_parent_to_item_ref(context, child_of_child, &self, ChildLocation::Middle);
                } else {
                    bottom_tree(context, child_of_child, self.height - 1).add_bottom_tree_parent(
                        context,
                        &self,
                        ChildLocation::Middle,
                    );
                }
            }
            (ChildLocation::Right, _) => {
                // the right child has a new child
                // this means we need to propagate the change up
                // and store them in our own list
                let mut state = self.state.lock();
                if state.following.add(child_of_child.clone()) {
                    for (parent, location) in state.bottom_upper.iter() {
                        parent.add_child_of_child(
                            context,
                            location,
                            context.is_blue(&child_of_child),
                            child_of_child,
                        );
                    }
                    for TopRef { parent } in state.top_upper.iter() {
                        parent.add_child_of_child(context, child_of_child);
                    }
                }
            }
        }
    }

    pub fn remove_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_is_blue: bool,
        child_of_child: &I,
    ) {
        match (child_location, child_is_blue) {
            (ChildLocation::Left, false) | (ChildLocation::Middle, _) => {
                // the left/middle child has lost a child
                // this means this node has lost a right child
                if self.height == 0 {
                    remove_parent_from_item_ref(
                        context,
                        child_of_child,
                        &self,
                        ChildLocation::Right,
                    );
                } else {
                    bottom_tree(context, child_of_child, self.height - 1)
                        .remove_bottom_tree_parent(context, &self, ChildLocation::Right);
                }
                {
                    let mut state = self.state.lock();
                    if state.following.add(child_of_child.clone()) {
                        for (parent, location) in state.bottom_upper.iter() {
                            parent.add_child_of_child(
                                context,
                                location,
                                context.is_blue(&child_of_child),
                                child_of_child,
                            );
                        }
                        for TopRef { parent } in state.top_upper.iter() {
                            parent.add_child_of_child(context, child_of_child);
                        }
                    }
                }
            }
            (ChildLocation::Left, true) => {
                // the left child has lost a child
                // and the left child is a blue node
                // this means this node has lost a middle child
                if self.height == 0 {
                    remove_parent_from_item_ref(
                        context,
                        child_of_child,
                        &self,
                        ChildLocation::Middle,
                    );
                } else {
                    bottom_tree(context, child_of_child, self.height - 1)
                        .remove_bottom_tree_parent(context, &self, ChildLocation::Middle);
                }
                {
                    let mut state = self.state.lock();
                    if state.following.add(child_of_child.clone()) {
                        for (parent, location) in state.bottom_upper.iter() {
                            parent.add_child_of_child(
                                context,
                                location,
                                context.is_blue(&child_of_child),
                                child_of_child,
                            );
                        }
                        for TopRef { parent } in state.top_upper.iter() {
                            parent.add_child_of_child(context, child_of_child);
                        }
                    }
                }
            }
            (ChildLocation::Right, _) => {
                // the right child has lost a child
                // this means we need to propagate the change up
                // and remove them from our own list
                let mut state = self.state.lock();
                if state.following.remove(child_of_child.clone()) {
                    for (parent, location) in state.bottom_upper.iter() {
                        parent.remove_child_of_child(
                            context,
                            location,
                            context.is_blue(&child_of_child),
                            child_of_child,
                        );
                    }
                    for TopRef { parent } in state.top_upper.iter() {
                        parent.remove_child_of_child(context, child_of_child);
                    }
                }
            }
        }
    }

    pub(super) fn add_bottom_tree_parent<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        parent: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) {
        let mut state = self.state.lock();
        let new = match location {
            ChildLocation::Left => {
                state.bottom_upper.init_left(parent.clone());
                true
            }
            ChildLocation::Middle => state.bottom_upper.add_middle(parent.clone()),
            ChildLocation::Right => state.bottom_upper.add_right(parent.clone()),
        };
        if new {
            if let Some(change) = context.info_to_add_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.add_child_of_child(
                    context,
                    location,
                    context.is_blue(&following),
                    following,
                );
            }
        }
    }

    pub(super) fn remove_bottom_tree_parent<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        parent: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) {
        let mut state = self.state.lock();
        let old_location = match location {
            ChildLocation::Left => unreachable!(),
            ChildLocation::Middle => state.bottom_upper.remove_middle(parent.clone()),
            ChildLocation::Right => state.bottom_upper.remove_right(parent.clone()),
        };
        if let Some(location) = old_location {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.remove_child_of_child(
                    context,
                    location,
                    context.is_blue(&following),
                    following,
                );
            }
        }
    }

    pub(super) fn add_top_tree_parent<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        parent: &Arc<TopTree<T>>,
    ) {
        let mut state = self.state.lock();
        let new = state.top_upper.add(TopRef {
            parent: parent.clone(),
        });
        if new {
            if let Some(change) = context.info_to_add_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.add_child_of_child(context, following);
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn remove_top_tree_parent<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        parent: &Arc<TopTree<T>>,
    ) {
        let mut state = self.state.lock();
        let removed = state.top_upper.remove(TopRef {
            parent: parent.clone(),
        });
        if removed {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.remove_child_of_child(context, following);
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
        for TopRef { parent } in state.top_upper.iter() {
            let info = parent.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                break;
            }
        }
        for parent in state.bottom_upper.keys() {
            let info = parent.get_root_info(context, root_info_type);
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
    for parent in state.bottom_upper.keys() {
        parent.child_change(context, &change);
    }
    for TopRef { parent } in state.top_upper.iter() {
        parent.child_change(context, &change);
    }
}

pub fn add_parent_to_item_ref<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) {
    let result = {
        let mut item = context.item(reference);
        add_parent_to_item_step_1::<C>(&mut item, parent, location)
    };
    add_parent_to_item_step_2(context, parent, result);
}

#[must_use]
pub fn add_parent_to_item_step_1<C: AggregationContext>(
    item: &mut C::ItemLock<'_>,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) -> (Option<C::ItemChange>, ChildLocation, bool, Vec<C::ItemRef>) {
    if item.leaf().add_upper(parent, location) {
        let change = item.get_add_change();
        let child_is_blue = item.is_blue();
        (
            change,
            location,
            child_is_blue,
            item.children().map(|r| r.into_owned()).collect(),
        )
    } else {
        (None, location, false, Vec::new())
    }
}

pub fn add_parent_to_item_step_2<C: AggregationContext>(
    context: &C,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    step_1_result: (Option<C::ItemChange>, ChildLocation, bool, Vec<C::ItemRef>),
) {
    let (change, location, is_blue, children) = step_1_result;
    if let Some(change) = change {
        context.on_add_change(&change);
        parent.child_change(context, &change);
    }
    for child in children {
        parent.add_child_of_child(context, location, is_blue, &child)
    }
}

pub fn remove_parent_from_item_ref<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) {
    let result = {
        let mut item = context.item(reference);
        remove_parent_from_item_step_1::<C>(&mut item, parent, location)
    };
    remove_parent_from_item_step_2(context, parent, result);
}

#[must_use]
pub fn remove_parent_from_item_step_1<C: AggregationContext>(
    item: &mut C::ItemLock<'_>,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) -> (Option<C::ItemChange>, ChildLocation, bool, Vec<C::ItemRef>) {
    if let Some(location) = item.leaf().remove_upper(parent, location) {
        let change = item.get_remove_change();
        let child_is_blue = item.is_blue();
        (
            change,
            location,
            child_is_blue,
            item.children().map(|r| r.into_owned()).collect(),
        )
    } else {
        (None, ChildLocation::Left, false, Vec::new())
    }
}

pub fn remove_parent_from_item_step_2<C: AggregationContext>(
    context: &C,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    step_1_result: (Option<C::ItemChange>, ChildLocation, bool, Vec<C::ItemRef>),
) {
    let (change, location, is_blue, children) = step_1_result;
    if let Some(change) = change {
        context.on_remove_change(&change);
        parent.child_change(context, &change);
    }
    for child in children {
        parent.remove_child_of_child(context, location, is_blue, &child)
    }
}
