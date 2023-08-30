use std::{hash::Hash, sync::Arc};

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
/// The first level aggregates an item with all of its children plus potentially
/// all of their children. This depends on a flag called "is_blue" of the
/// children. A "blue" child will aggregate one additional layer of
/// connectivity, but this is not applies recursively. So the first lever of the
/// tree will aggregate a connectivity of 2 to 3.
///
/// The higher levels will only aggregate a connectivity of 2. "Blue" nodes
/// doesn't exist on this level, but that might be something to consider for the
/// future.
///
/// The concept of "blue" nodes will improve the sharing of graphs as
/// aggregation will eventually propagate to use the same items, even if they
/// start on different depths of the graph.
pub struct BottomTree<T: AggregationContext> {
    height: u8,
    state: Mutex<BottomTreeState<T>>,
}

enum BottomTreeParent<T: AggregationContext> {
    Top(TopRef<T>),
    Bottom(BottomRef<T>),
}

impl<T: AggregationContext> PartialEq for BottomTreeParent<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Top(left), Self::Top(right)) => left == right,
            (Self::Bottom(left), Self::Bottom(right)) => left == right,
            _ => false,
        }
    }
}

impl<T: AggregationContext> Eq for BottomTreeParent<T> {}

impl<T: AggregationContext> Hash for BottomTreeParent<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Top(top) => top.hash(state),
            Self::Bottom(bottom) => bottom.hash(state),
        }
    }
}

pub struct BottomTreeState<T: AggregationContext> {
    data: T::Info,
    upper: CountHashSet<BottomTreeParent<T>>,
    /// Items that are referenced by right children of this node.
    following: CountHashSet<T::ItemRef>,
}

impl<T: AggregationContext> BottomTree<T> {
    pub fn new(height: u8) -> Self {
        Self {
            height,
            state: Mutex::new(BottomTreeState {
                data: T::new_info(),
                upper: CountHashSet::new(),
                following: CountHashSet::new(),
            }),
        }
    }

    pub fn add_child_of_child(
        self: Arc<Self>,
        context: &T,
        child_location: ChildLocation,
        child_is_blue: bool,
        child_of_child: T::ItemRef,
    ) {
        match (child_location, child_is_blue) {
            (ChildLocation::Left, false) | (ChildLocation::Middle, _) => {
                // the left/middle child has a new child
                // this means it's a right child of this node
                let mut item = context.item(child_of_child);
                if self.height == 0 {
                    add_parent_to_item(context, &mut item, self.clone(), ChildLocation::Right);
                } else {
                    bottom_tree(context, &mut item, self.height - 1).add_bottom_tree_parent(
                        context,
                        self.clone(),
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
                    add_parent_to_item(context, &mut item, self.clone(), ChildLocation::Middle);
                } else {
                    bottom_tree(context, &mut item, self.height - 1).add_bottom_tree_parent(
                        context,
                        self.clone(),
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
                            parent
                                .clone()
                                .add_child_of_child(context, child_of_child.clone());
                        }
                        BottomTreeParent::Bottom(BottomRef { parent, location }) => {
                            parent.clone().add_child_of_child(
                                context,
                                *location,
                                false,
                                child_of_child.clone(),
                            );
                        }
                    }
                }
                state.following.add(child_of_child);
            }
        }
    }

    pub fn remove_child_of_child(
        self: &Arc<Self>,
        context: &T,
        child_location: ChildLocation,
        child_is_blue: bool,
        child_of_child: T::ItemRef,
    ) {
        match (child_location, child_is_blue) {
            (ChildLocation::Left, false) | (ChildLocation::Middle, _) => {
                // the left/middle child has lost a child
                // this means this node has lost a right child
                if self.height == 0 {
                    remove_parent_from_item(
                        context,
                        &mut context.item(child_of_child),
                        self.clone(),
                        ChildLocation::Right,
                    );
                } else {
                    bottom_tree(context, &mut context.item(child_of_child), self.height - 1)
                        .remove_bottom_tree_parent(context, self.clone(), ChildLocation::Right);
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
                        self.clone(),
                        ChildLocation::Middle,
                    );
                } else {
                    bottom_tree(context, &mut context.item(child_of_child), self.height - 1)
                        .remove_bottom_tree_parent(context, self.clone(), ChildLocation::Middle);
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
                            parent
                                .clone()
                                .remove_child_of_child(context, child_of_child.clone());
                        }
                        BottomTreeParent::Bottom(BottomRef { parent, location }) => {
                            parent.clone().remove_child_of_child(
                                context,
                                *location,
                                false,
                                child_of_child.clone(),
                            );
                        }
                    }
                }
                state.following.remove(child_of_child);
            }
        }
    }

    pub(super) fn add_bottom_tree_parent(
        &self,
        context: &T,
        parent: Arc<BottomTree<T>>,
        location: ChildLocation,
    ) {
        let mut state = self.state.lock();
        if state.upper.add(BottomTreeParent::Bottom(BottomRef {
            parent: parent.clone(),
            location,
        })) {
            if let Some(change) = context.info_to_add_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent
                    .clone()
                    .add_child_of_child(context, location, false, following.clone());
            }
        }
    }

    pub(super) fn remove_bottom_tree_parent(
        &self,
        context: &T,
        parent: Arc<BottomTree<T>>,
        location: ChildLocation,
    ) {
        let mut state = self.state.lock();
        if state.upper.remove(BottomTreeParent::Bottom(BottomRef {
            parent: parent.clone(),
            location,
        })) {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent
                    .clone()
                    .remove_child_of_child(context, location, false, following.clone());
            }
        }
    }

    pub(super) fn add_top_tree_parent(&self, context: &T, parent: Arc<TopTree<T>>) {
        let mut state = self.state.lock();
        if state.upper.add(BottomTreeParent::Top(TopRef {
            parent: parent.clone(),
        })) {
            if let Some(change) = context.info_to_add_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent
                    .clone()
                    .add_child_of_child(context, following.clone());
            }
        }
    }

    pub(super) fn remove_top_tree_parent(&self, context: &T, parent: Arc<TopTree<T>>) {
        let mut state = self.state.lock();
        if state.upper.remove(BottomTreeParent::Top(TopRef {
            parent: parent.clone(),
        })) {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                parent.child_change(context, &change);
            }
            for following in state.following.iter() {
                parent
                    .clone()
                    .remove_child_of_child(context, following.clone());
            }
        }
    }

    pub(super) fn child_change(&self, context: &T, change: &T::ItemChange) {
        let mut state = self.state.lock();
        let change = context.apply_change(&mut state.data, change);
        propagate_change_to_upper(&mut state, context, change);
    }
}

fn propagate_change_to_upper<T: AggregationContext>(
    state: &mut MutexGuard<BottomTreeState<T>>,
    context: &T,
    change: Option<T::ItemChange>,
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

pub fn add_parent_to_item<T: AggregationContext>(
    context: &T,
    item: &mut T::ItemLock,
    parent: Arc<BottomTree<T>>,
    location: ChildLocation,
) {
    if item.leaf().add_upper(parent.clone(), location) {
        if let Some(change) = item.get_add_change() {
            let mut state = parent.state.lock();
            let change = context.apply_change(&mut state.data, &change);
            propagate_change_to_upper(&mut state, context, change);
        }
        let child_is_blue = item.is_blue();
        for child in item.children() {
            parent
                .clone()
                .add_child_of_child(context, location, child_is_blue, child)
        }
    }
}

pub fn remove_parent_from_item<T: AggregationContext>(
    context: &T,
    item: &mut T::ItemLock,
    parent: Arc<BottomTree<T>>,
    location: ChildLocation,
) {
    if item.leaf().remove_upper(parent.clone(), location) {
        if let Some(change) = item.get_remove_change() {
            let mut state = parent.state.lock();
            let change = context.apply_change(&mut state.data, &change);
            propagate_change_to_upper(&mut state, context, change);
        }
        let child_is_blue = item.is_blue();
        for child in item.children() {
            parent
                .clone()
                .remove_child_of_child(context, location, child_is_blue, child)
        }
    }
}
