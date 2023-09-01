use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use auto_hash_map::AutoSet;
use parking_lot::{Mutex, MutexGuard};

use super::{
    inner_refs::{BottomRef, ChildLocation, TopRef},
    leaf::bottom_tree,
    top_tree::TopTree,
    AggregationContext, AggregationItemLock,
};

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
pub struct BottomTree<T, I> {
    height: u8,
    state: Mutex<BottomTreeState<T, I>>,
}

enum BottomTreeParent<T, I> {
    Top(TopRef<T>),
    Bottom(BottomRef<T, I>),
}

impl<T, I> PartialEq for BottomTreeParent<T, I> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Top(left), Self::Top(right)) => left == right,
            (Self::Bottom(left), Self::Bottom(right)) => left == right,
            _ => false,
        }
    }
}

impl<T, I> Eq for BottomTreeParent<T, I> {}

impl<T, I> Hash for BottomTreeParent<T, I> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Top(top) => top.hash(state),
            Self::Bottom(bottom) => bottom.hash(state),
        }
    }
}

pub struct BottomTreeState<T, I> {
    data: T,
    upper: AutoSet<BottomTreeParent<T, I>>,
    /// Items that are referenced by right children of this node.
    following: AutoSet<I>,
}

impl<T: Default, I> BottomTree<T, I> {
    pub fn new(height: u8) -> Self {
        Self {
            height,
            state: Mutex::new(BottomTreeState {
                data: T::default(),
                upper: AutoSet::new(),
                following: AutoSet::new(),
            }),
        }
    }
}

impl<T, I: Clone + Eq + Hash> BottomTree<T, I> {
    pub fn add_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_is_blue: bool,
        child_of_child: I,
    ) {
        match (child_location, child_is_blue) {
            (ChildLocation::Left, false) | (ChildLocation::Middle, _) => {
                // the left/middle child has a new child
                // this means it's a right child of this node
                let mut item = context.item(child_of_child);
                if self.height == 0 {
                    add_parent_to_item(context, &mut item, &self, ChildLocation::Right);
                } else {
                    bottom_tree(context, &mut item, self.height - 1).add_bottom_tree_parent(
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
                let mut item = context.item(child_of_child);
                if self.height == 0 {
                    add_parent_to_item(context, &mut item, &self, ChildLocation::Middle);
                } else {
                    bottom_tree(context, &mut item, self.height - 1).add_bottom_tree_parent(
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
                for parent in state.upper.iter() {
                    match parent {
                        BottomTreeParent::Top(TopRef { parent }) => {
                            parent.add_child_of_child(context, child_of_child.clone());
                        }
                        BottomTreeParent::Bottom(BottomRef { parent, location }) => {
                            parent.add_child_of_child(
                                context,
                                *location,
                                context.is_blue(&child_of_child),
                                child_of_child.clone(),
                            );
                        }
                    }
                }
                state.following.insert(child_of_child);
            }
        }
    }

    pub fn remove_child_of_child<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_location: ChildLocation,
        child_is_blue: bool,
        child_of_child: I,
    ) {
        match (child_location, child_is_blue) {
            (ChildLocation::Left, false) | (ChildLocation::Middle, _) => {
                // the left/middle child has lost a child
                // this means this node has lost a right child
                if self.height == 0 {
                    remove_parent_from_item(
                        context,
                        &mut context.item(child_of_child),
                        &self,
                        ChildLocation::Right,
                    );
                } else {
                    bottom_tree(context, &mut context.item(child_of_child), self.height - 1)
                        .remove_bottom_tree_parent(context, &self, ChildLocation::Right);
                }
            }
            (ChildLocation::Left, true) => {
                // the left child has lost a child
                // and the left child is a blue node
                // this means this node has lost a middle child
                if self.height == 0 {
                    remove_parent_from_item(
                        context,
                        &mut context.item(child_of_child),
                        &self,
                        ChildLocation::Middle,
                    );
                } else {
                    bottom_tree(context, &mut context.item(child_of_child), self.height - 1)
                        .remove_bottom_tree_parent(context, &self, ChildLocation::Middle);
                }
            }
            (ChildLocation::Right, _) => {
                // the right child has lost a child
                // this means we need to propagate the change up
                // and remove them from our own list
                let mut state = self.state.lock();
                for parent in state.upper.iter() {
                    match parent {
                        BottomTreeParent::Top(TopRef { parent }) => {
                            parent.remove_child_of_child(context, child_of_child.clone());
                        }
                        BottomTreeParent::Bottom(BottomRef { parent, location }) => {
                            parent.remove_child_of_child(
                                context,
                                *location,
                                context.is_blue(&child_of_child),
                                child_of_child.clone(),
                            );
                        }
                    }
                }
                state.following.remove(&child_of_child);
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
        if state.upper.insert(BottomTreeParent::Bottom(BottomRef {
            parent: parent.clone(),
            location,
        })) {
            if let Some(change) = context.info_to_add_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.add_child_of_child(
                    context,
                    location,
                    context.is_blue(&following),
                    following.clone(),
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
        if state.upper.remove(&BottomTreeParent::Bottom(BottomRef {
            parent: parent.clone(),
            location,
        })) {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.remove_child_of_child(
                    context,
                    location,
                    context.is_blue(&following),
                    following.clone(),
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
        if state.upper.insert(BottomTreeParent::Top(TopRef {
            parent: parent.clone(),
        })) {
            if let Some(change) = context.info_to_add_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.add_child_of_child(context, following.clone());
            }
        }
    }

    pub(super) fn remove_top_tree_parent<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        parent: &Arc<TopTree<T>>,
    ) {
        let mut state = self.state.lock();
        if state.upper.remove(&BottomTreeParent::Top(TopRef {
            parent: parent.clone(),
        })) {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent.remove_child_of_child(context, following.clone());
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
        for parent in state.upper.iter() {
            match parent {
                BottomTreeParent::Top(TopRef { parent }) => {
                    let info = parent.get_root_info(context, root_info_type);
                    if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                        break;
                    }
                }
                BottomTreeParent::Bottom(BottomRef {
                    parent,
                    location: _,
                }) => {
                    let info = parent.get_root_info(context, root_info_type);
                    if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                        break;
                    }
                }
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
    for parent in state.upper.iter() {
        match parent {
            BottomTreeParent::Top(TopRef { parent }) => {
                parent.child_change(context, &change);
            }
            BottomTreeParent::Bottom(BottomRef {
                parent,
                location: _,
            }) => {
                parent.child_change(context, &change);
            }
        }
    }
}

pub fn add_parent_to_item<C: AggregationContext>(
    context: &C,
    item: &mut C::ItemLock<'_>,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) {
    if item.leaf().add_upper(parent.clone(), location) {
        if let Some(change) = item.get_add_change() {
            context.on_add_change(&change);
            let mut state = parent.state.lock();
            let change = context.apply_change(&mut state.data, &change);
            propagate_change_to_upper(&mut state, context, change);
        }
        let child_is_blue = item.is_blue();
        for child in item.children() {
            parent.add_child_of_child(context, location, child_is_blue, child)
        }
    }
}

pub fn remove_parent_from_item<C: AggregationContext>(
    context: &C,
    item: &mut C::ItemLock<'_>,
    parent: &Arc<BottomTree<C::Info, C::ItemRef>>,
    location: ChildLocation,
) {
    if item.leaf().remove_upper(parent.clone(), location) {
        if let Some(change) = item.get_remove_change() {
            context.on_remove_change(&change);
            let mut state = parent.state.lock();
            let change = context.apply_change(&mut state.data, &change);
            propagate_change_to_upper(&mut state, context, change);
        }
        let child_is_blue = item.is_blue();
        for child in item.children() {
            parent.remove_child_of_child(context, location, child_is_blue, child)
        }
    }
}
