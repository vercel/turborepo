use std::{
    collections::{HashSet, VecDeque},
    hash::Hash,
    ops::ControlFlow,
    sync::Arc,
};

use nohash_hasher::{BuildNoHashHasher, IsEnabled};
use parking_lot::{Mutex, MutexGuard};
use ref_cast::RefCast;

use super::{
    inner_refs::{BottomRef, ChildLocation, TopRef},
    leaf::{add_inner_upper_to_item, bottom_tree, remove_inner_upper_from_item},
    top_tree::TopTree,
    AggregationContext, FORCE_LEFT_CHILD_CHILD_AS_INNER, LEFT_CHILD_CHILD_USE_BLUE,
    MAX_INNER_UPPERS, MAX_NESTING_LEVEL, USE_BLUE_NODES,
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
        let height = self.height;
        is_blue(hash, height)
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
                if LEFT_CHILD_CHILD_USE_BLUE {
                    let SplitChildren { blue, mut white } = self.split_children(children);
                    if !blue.is_empty() {
                        self.add_children_of_child_following(context, blue);
                    }
                    if !white.is_empty() {
                        self.add_children_of_child_if_following(&mut white);
                        self.add_children_of_child_inner(
                            context,
                            white.iter().map(|&(_, c)| c),
                            FORCE_LEFT_CHILD_CHILD_AS_INNER,
                            nesting_level,
                        );
                    }
                } else {
                    // the left child has new children
                    // this means it's a inner child of this node
                    // We always want to aggregate over at least connectivity 1
                    self.add_children_of_child_inner(
                        context,
                        children.into_iter().map(|(_, c)| c),
                        FORCE_LEFT_CHILD_CHILD_AS_INNER,
                        nesting_level,
                    );
                }
            }
            ChildLocation::Inner => {
                // the inner child has new children
                // this means white children are inner children of this node
                // and blue children need to propagate up
                if nesting_level > MAX_NESTING_LEVEL {
                    self.add_children_of_child_following(context, children.into_iter().collect());
                    return;
                }
                let SplitChildren { blue, mut white } = self.split_children(children);
                if !blue.is_empty() {
                    self.add_children_of_child_following(context, blue);
                }
                if !white.is_empty() {
                    self.add_children_of_child_if_following(&mut white);
                    self.add_children_of_child_inner(
                        context,
                        white.iter().map(|&(_, c)| c),
                        false,
                        nesting_level,
                    );
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
        let mut state = self.state.lock();
        children.retain(|&(_, child)| state.following.add(child.clone()));
        if children.is_empty() {
            return;
        }
        let left_bottom_upper = state.left_bottom_upper.clone();
        let inner_bottom_uppers = state.inner_bottom_upper.iter().cloned().collect::<Vec<_>>();
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
        force_inner: bool,
        nesting_level: u8,
    ) where
        I: 'a,
    {
        let mut following = Vec::new();
        if self.height == 0 {
            for child in children {
                let can_be_inner =
                    add_inner_upper_to_item(context, child, &self, force_inner, nesting_level);
                if !can_be_inner {
                    following.push((context.hash(child), child));
                }
            }
        } else {
            for child in children {
                let can_be_inner = bottom_tree(context, child, self.height - 1)
                    .add_inner_bottom_tree_upper(context, &self, force_inner, nesting_level);
                if !can_be_inner {
                    following.push((context.hash(child), child));
                }
            }
        }
        if !following.is_empty() {
            self.add_children_of_child_following(context, following);
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
        match child_location {
            ChildLocation::Left => {
                if LEFT_CHILD_CHILD_USE_BLUE && self.is_blue(child_of_child_hash) {
                    // the left child has a new child
                    // and it's a blue node
                    // this means it's a following child of this node
                    self.add_child_of_child_following(context, child_of_child, child_of_child_hash);
                    return;
                }
                // the left child has a new child
                // this means it's a inner child of this node
                // We always want to aggregate over at least connectivity 1
                self.add_child_of_child_inner(
                    context,
                    child_of_child,
                    child_of_child_hash,
                    FORCE_LEFT_CHILD_CHILD_AS_INNER,
                    nesting_level,
                );
            }
            ChildLocation::Inner => {
                if nesting_level <= 4 && !self.is_blue(child_of_child_hash) {
                    // the inner child has a new child
                    // but it's not a blue node and we are not too deep
                    // this means it's a inner child of this node
                    // if it's not already a following child
                    if !self.add_child_of_child_if_following(child_of_child) {
                        self.add_child_of_child_inner(
                            context,
                            child_of_child,
                            child_of_child_hash,
                            false,
                            nesting_level,
                        );
                    }
                } else {
                    // the inner child has a new child
                    // this means we need to propagate the change up
                    // and store them in our own list
                    self.add_child_of_child_following(context, child_of_child, child_of_child_hash);
                }
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
        child_of_child_hash: u32,
        force_inner: bool,
        nesting_level: u8,
    ) {
        let can_be_inner;
        if self.height == 0 {
            can_be_inner =
                add_inner_upper_to_item(context, child_of_child, &self, force_inner, nesting_level);
        } else {
            can_be_inner = bottom_tree(context, child_of_child, self.height - 1)
                .add_inner_bottom_tree_upper(context, &self, force_inner, nesting_level);
        }
        if !can_be_inner {
            self.add_child_of_child_following(context, child_of_child, child_of_child_hash);
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
        let left_bottom_upper;
        let inner_bottom_upper;
        let mut state = self.state.lock();
        match state.following.remove_if_entry(child_of_child) {
            RemoveIfEntryResult::PartiallyRemoved => return true,
            RemoveIfEntryResult::NotPresent => return false,
            RemoveIfEntryResult::Removed => {
                left_bottom_upper = state.left_bottom_upper.clone();
                inner_bottom_upper = state.inner_bottom_upper.iter().cloned().collect::<Vec<_>>();
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

    #[must_use]
    pub(super) fn add_inner_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
        force_inner: bool,
        nesting_level: u8,
    ) -> bool {
        let mut state = self.state.lock();
        if !force_inner
            && (state.inner_bottom_upper.len() >= MAX_INNER_UPPERS
                || state.left_bottom_upper.is_some())
        {
            return state
                .inner_bottom_upper
                .add_if_entry(&BottomRef::ref_cast(upper));
        }
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
        true
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

fn is_blue(hash: u32, height: u8) -> bool {
    USE_BLUE_NODES && (hash.rotate_right(height as u32) & 1) == 0
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

pub fn print_graph<C: AggregationContext>(
    context: &C,
    entry: &C::ItemRef,
    height: u8,
    color_upper: bool,
    name_fn: impl Fn(&C::ItemRef) -> String,
) {
    use std::fmt::Write;
    if !color_upper {
        print!("subgraph cluster_{} {{", height);
        print!("label = \"Level {}\";", height);
        print!("color = \"black\";");
    }
    let mut edges = String::new();
    let tree = bottom_tree(context, entry, height);
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    visited.insert(entry.clone());
    queue.push_back((entry.clone(), tree));
    while let Some((item, tree)) = queue.pop_front() {
        let name = name_fn(&item);
        if color_upper {
            print!(r#""{} {}" [color=red];"#, height - 1, name);
        } else if is_blue(context.hash(&item), height + 1) {
            print!(
                r#""{} {}" [label="{}", style=filled, fillcolor=lightblue];"#,
                height, name, name
            );
        } else {
            print!(r#""{} {}" [label="{}"];"#, height, name, name);
        }
        let state = tree.state.lock();
        for next in state.following.iter() {
            if !color_upper {
                write!(
                    edges,
                    r#""{} {}" -> "{} {}";"#,
                    height,
                    name,
                    height,
                    name_fn(next)
                )
                .unwrap();
            }
            if visited.insert(next.clone()) {
                queue.push_back((next.clone(), bottom_tree(context, next, height)));
            }
        }
    }
    if !color_upper {
        println!("}}");
        println!("{}", edges);
    }
}
