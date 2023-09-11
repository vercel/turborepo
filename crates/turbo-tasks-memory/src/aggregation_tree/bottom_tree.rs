use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use nohash_hasher::{BuildNoHashHasher, IsEnabled};
use parking_lot::{Mutex, MutexGuard};
use ref_cast::RefCast;

use super::{
    inner_refs::{BottomRef, ChildLocation, TopRef},
    leaf::{
        add_inner_upper_to_item, bottom_tree, remove_inner_upper_from_item,
        remove_left_upper_from_item,
    },
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
    item: I,
    state: Mutex<BottomTreeState<T, I>>,
}

enum BottomTreeMode<T, I: IsEnabled> {
    Left(Arc<BottomTree<T, I>>),
    Inner(CountHashSet<BottomRef<T, I>, BuildNoHashHasher<BottomRef<T, I>>>),
}

enum BottomUppers<T, I: IsEnabled> {
    Left(Arc<BottomTree<T, I>>),
    Inner(Vec<BottomRef<T, I>>),
}

impl<T, I: IsEnabled> BottomTreeMode<T, I> {
    fn as_cloned_uppers(&self) -> BottomUppers<T, I> {
        match self {
            Self::Left(upper) => BottomUppers::Left(upper.clone()),
            Self::Inner(upper) => BottomUppers::Inner(upper.iter().cloned().collect()),
        }
    }
}
pub struct BottomTreeState<T, I: IsEnabled> {
    data: T,
    bottom_upper: BottomTreeMode<T, I>,
    top_upper: CountHashSet<TopRef<T>, BuildNoHashHasher<TopRef<T>>>,
    // TODO this can't become negative
    following: CountHashSet<I, BuildNoHashHasher<I>>,
}

impl<T: Default, I: IsEnabled> BottomTree<T, I> {
    pub fn new(item: I, height: u8) -> Self {
        Self {
            height,
            item,
            state: Mutex::new(BottomTreeState {
                data: T::default(),
                bottom_upper: BottomTreeMode::Inner(CountHashSet::new()),
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
                    debug_assert!(!blue.iter().any(|&(_, c)| c == &self.item));
                    debug_assert!(!white.iter().any(|&(_, c)| c == &self.item));
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
                        children.into_iter().map(|(_, c)| {
                            debug_assert!(c != &self.item);
                            c
                        }),
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
                debug_assert!(!blue.iter().any(|&(_, c)| c == &self.item));
                debug_assert!(!white.iter().any(|&(_, c)| c == &self.item));
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
        let buttom_upper = state.bottom_upper.as_cloned_uppers();
        let top_upper = state.top_upper.iter().cloned().collect::<Vec<_>>();
        drop(state);
        for TopRef { upper } in top_upper {
            upper.add_children_of_child(context, children.iter().map(|&(_, c)| c));
        }
        match buttom_upper {
            BottomUppers::Left(upper) => {
                upper.add_children_of_child(
                    context,
                    ChildLocation::Left,
                    children.iter().copied(),
                    0,
                );
            }
            BottomUppers::Inner(list) => {
                for BottomRef { upper } in list {
                    upper.add_children_of_child(
                        context,
                        ChildLocation::Inner,
                        children.iter().copied(),
                        0,
                    );
                }
            }
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
        debug_assert!(child_of_child != &self.item);
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

        propagate_new_following_to_uppers(state, context, child_of_child, child_of_child_hash);
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
        let mut state = self.state.lock();
        match state.following.remove_if_entry(child_of_child) {
            RemoveIfEntryResult::PartiallyRemoved => return true,
            RemoveIfEntryResult::NotPresent => return false,
            RemoveIfEntryResult::Removed => {}
        }
        propagate_lost_following_to_uppers(state, context, child_of_child);
        true
    }

    fn remove_child_of_child_following<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
    ) -> bool {
        let mut state = self.state.lock();
        if !state.following.remove(child_of_child.clone()) {
            // no present, nothing to do
            return false;
        }
        propagate_lost_following_to_uppers(state, context, child_of_child);
        true
    }

    fn remove_child_of_child_inner<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        child_of_child: &I,
    ) {
        let can_remove_inner = if self.height == 0 {
            remove_inner_upper_from_item(context, child_of_child, &self);
            true
        } else {
            bottom_tree(context, child_of_child, self.height - 1)
                .remove_inner_bottom_tree_upper(context, &self)
        };
        if !can_remove_inner {
            self.remove_child_of_child_following(context, child_of_child);
        }
    }

    pub(super) fn add_left_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
    ) {
        let mut state = self.state.lock();
        let old_inner =
            match std::mem::replace(&mut state.bottom_upper, BottomTreeMode::Left(upper.clone())) {
                BottomTreeMode::Left(_) => unreachable!("Can't have two left children"),
                BottomTreeMode::Inner(old_inner) => old_inner,
            };
        if let Some(change) = context.info_to_add_change(&state.data) {
            upper.child_change(context, &change);
        }
        let children = state.following.iter().cloned().collect::<Vec<_>>();
        let following_for_old_uppers = (!old_inner.is_empty())
            .then(|| state.following.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let remove_change = (!old_inner.is_empty())
            .then(|| context.info_to_remove_change(&state.data))
            .flatten();

        drop(state);
        let children = children.iter().map(|item| (context.hash(item), item));
        upper.add_children_of_child(context, ChildLocation::Left, children, 1);

        // Convert this node into a following node for all old (inner) uppers
        //
        // Old state:
        // I1, I2
        //      \
        //       self
        // Adding L as new left upper:
        // I1, I2     L
        //      \    /
        //       self
        // Final state: (I1 and I2 have L as following instead)
        // I1, I2 ----> L
        //             /
        //         self
        // I1 and I2 have "self" change removed since it's now part of L instead.
        // L = upper, I1, I2 = old_inner
        //
        for (BottomRef { upper: old_upper }, count) in old_inner.into_counts() {
            let item = &self.item;
            let mut upper_state = old_upper.state.lock();
            if count > 0 {
                // add as following
                if upper_state
                    .following
                    .add_count(item.clone(), count as usize)
                {
                    propagate_new_following_to_uppers(
                        upper_state,
                        context,
                        item,
                        context.hash(item),
                    );
                } else {
                    drop(upper_state);
                }
                // remove from self
                if let Some(change) = remove_change.as_ref() {
                    old_upper.child_change(context, change);
                }
                for following in following_for_old_uppers.iter() {
                    // TODO use children of child method
                    old_upper.remove_child_of_child(context, following);
                }
            } else {
                // remove count from following instead
                if upper_state
                    .following
                    .remove_count(item.clone(), -count as usize)
                {
                    propagate_lost_following_to_uppers(upper_state, context, item);
                }
            }
        }
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
        let BottomTreeMode::Inner(inner) = &mut state.bottom_upper else {
            return false;
        };
        if !force_inner && inner.len() >= MAX_INNER_UPPERS {
            return inner.add_if_entry(&BottomRef::ref_cast(upper));
        }
        let new = inner.add(BottomRef {
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

    pub(super) fn remove_left_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        self: &Arc<Self>,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
    ) {
        let mut state = self.state.lock();
        match std::mem::replace(
            &mut state.bottom_upper,
            BottomTreeMode::Inner(CountHashSet::new()),
        ) {
            BottomTreeMode::Left(old_upper) => {
                debug_assert!(Arc::ptr_eq(&old_upper, upper));
            }
            BottomTreeMode::Inner(_) => unreachable!("Must that a left child"),
        }
        if let Some(change) = context.info_to_remove_change(&state.data) {
            upper.child_change(context, &change);
        }
        for following in state.following.iter() {
            // TODO use children of child method
            // TODO move this out of the state lock
            upper.remove_child_of_child(context, following);
        }
        if state.top_upper.is_empty() {
            drop(state);
            self.remove_self_from_lower(context);
        }
    }

    #[must_use]
    pub(super) fn remove_inner_bottom_tree_upper<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        upper: &Arc<BottomTree<T, I>>,
    ) -> bool {
        let mut state = self.state.lock();
        let BottomTreeMode::Inner(inner) = &mut state.bottom_upper else {
            return false;
        };
        let removed = inner.remove(BottomRef {
            upper: upper.clone(),
        });
        if removed {
            if let Some(change) = context.info_to_remove_change(&state.data) {
                // TODO move this out of the state lock
                upper.child_change(context, &change);
            }
            for following in state.following.iter() {
                // TODO use children of child method
                // TODO move this out of the state lock
                upper.remove_child_of_child(context, following);
            }
        }
        true
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
        self: &Arc<Self>,
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
            if state.top_upper.is_empty() && !matches!(state.bottom_upper, BottomTreeMode::Left(_))
            {
                drop(state);
                self.remove_self_from_lower(context);
            }
        }
    }

    fn remove_self_from_lower(
        self: &Arc<Self>,
        context: &impl AggregationContext<Info = T, ItemRef = I>,
    ) {
        if self.height == 0 {
            remove_left_upper_from_item(context, &self.item, self);
        } else {
            bottom_tree(context, &self.item, self.height - 1)
                .remove_left_bottom_tree_upper(context, self);
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
        match &state.bottom_upper {
            BottomTreeMode::Left(upper) => {
                let info = upper.get_root_info(context, root_info_type);
                if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                    return result;
                }
            }
            BottomTreeMode::Inner(list) => {
                for BottomRef { upper } in list.iter() {
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

fn propagate_lost_following_to_uppers<C: AggregationContext>(
    state: MutexGuard<'_, BottomTreeState<C::Info, C::ItemRef>>,
    context: &C,
    child_of_child: &C::ItemRef,
) {
    let bottom_uppers = state.bottom_upper.as_cloned_uppers();
    let top_upper = state.top_upper.iter().cloned().collect::<Vec<_>>();
    drop(state);
    for TopRef { upper } in top_upper {
        upper.remove_child_of_child(context, child_of_child);
    }
    match bottom_uppers {
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

fn propagate_new_following_to_uppers<C: AggregationContext>(
    state: MutexGuard<'_, BottomTreeState<C::Info, C::ItemRef>>,
    context: &C,
    child_of_child: &C::ItemRef,
    child_of_child_hash: u32,
) {
    // TODO we want to check if child_of_child is already connected as inner child
    // and convert that that
    let bottom_uppers = state.bottom_upper.as_cloned_uppers();
    let top_upper = state.top_upper.iter().cloned().collect::<Vec<_>>();
    drop(state);
    for TopRef { upper } in top_upper {
        upper.add_child_of_child(context, child_of_child);
    }
    match bottom_uppers {
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
            for BottomRef { upper } in list {
                upper.add_child_of_child(
                    context,
                    ChildLocation::Inner,
                    child_of_child,
                    child_of_child_hash,
                    0,
                );
            }
        }
    }
}

fn is_blue(hash: u32, height: u8) -> bool {
    USE_BLUE_NODES && (hash.rotate_right(height as u32 * 2) & 3) == 0
}

fn propagate_change_to_upper<C: AggregationContext>(
    state: &mut MutexGuard<BottomTreeState<C::Info, C::ItemRef>>,
    context: &C,
    change: Option<C::ItemChange>,
) {
    let Some(change) = change else {
        return;
    };
    match &state.bottom_upper {
        BottomTreeMode::Left(upper) => {
            upper.child_change(context, &change);
        }
        BottomTreeMode::Inner(list) => {
            for BottomRef { upper } in list.iter() {
                upper.child_change(context, &change);
            }
        }
    }
    for TopRef { upper } in state.top_upper.iter() {
        upper.child_change(context, &change);
    }
}

#[cfg(test)]
fn visit_graph<C: AggregationContext>(
    context: &C,
    entry: &C::ItemRef,
    height: u8,
) -> (usize, usize) {
    use std::collections::{HashSet, VecDeque};
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    visited.insert(entry.clone());
    queue.push_back(entry.clone());
    let mut edges = 0;
    while let Some(item) = queue.pop_front() {
        let tree = bottom_tree(context, &item, height);
        let state = tree.state.lock();
        for next in state.following.iter() {
            edges += 1;
            if visited.insert(next.clone()) {
                queue.push_back(next.clone());
            }
        }
    }
    (visited.len(), edges)
}

#[cfg(test)]
pub fn print_graph<C: AggregationContext>(
    context: &C,
    entry: &C::ItemRef,
    height: u8,
    color_upper: bool,
    name_fn: impl Fn(&C::ItemRef) -> String,
) {
    use std::{
        collections::{HashSet, VecDeque},
        fmt::Write,
    };
    let (nodes, edges) = visit_graph(context, entry, height);
    if !color_upper {
        print!("subgraph cluster_{} {{", height);
        print!(
            "label = \"Level {}\\n{} nodes, {} edges\";",
            height, nodes, edges
        );
        print!("color = \"black\";");
    }
    let mut edges = String::new();
    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    visited.insert(entry.clone());
    queue.push_back(entry.clone());
    while let Some(item) = queue.pop_front() {
        let tree = bottom_tree(context, &item, height);
        let name = name_fn(&item);
        let mut label = format!("{}", name);
        let state = tree.state.lock();
        for (item, count) in state.following.counts() {
            if *count < 0 {
                label += "\\n";
                label += &name_fn(item);
            }
        }
        if color_upper {
            print!(r#""{} {}" [color=red];"#, height - 1, name);
        } else if is_blue(context.hash(&item), height + 1) {
            print!(
                r#""{} {}" [label="{}", style=filled, fillcolor=lightblue];"#,
                height, name, label
            );
        } else {
            print!(r#""{} {}" [label="{}"];"#, height, name, label);
        }
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
                queue.push_back(next.clone());
            }
        }
    }
    if !color_upper {
        println!("}}");
        println!("{}", edges);
    }
}
