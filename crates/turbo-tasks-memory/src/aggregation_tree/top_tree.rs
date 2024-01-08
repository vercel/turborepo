use std::{mem::transmute, sync::Arc};

use parking_lot::{Mutex, MutexGuard};

use super::{leaf::bottom_tree, AggregationContext};

/// The top half of the aggregation tree. It can aggregate all nodes of a
/// subgraph. To do that it starts with a [BottomTree] of height 1 of the root
/// node and for every child of that [BottomTree] it connects a [BottomTree] of
/// height 2. Continuing with height 3, 4, etc. until the whole subgraph is
/// covered.
pub struct TopTree<T> {
    state: Mutex<TopTreeState<T>>,
}

struct TopTreeState<T> {
    data: T,
}

impl<T: Default> TopTree<T> {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(TopTreeState { data: T::default() }),
        }
    }
}

impl<T> TopTree<T> {
    pub fn add_children_of_child<'a, C: AggregationContext<Info = T>>(
        self: &Arc<Self>,
        aggregation_context: &C,
        children: impl IntoIterator<Item = &'a C::ItemRef>,
        height: u8,
    ) where
        C::ItemRef: 'a,
    {
        for child in children {
            bottom_tree(aggregation_context, child, height + 1)
                .add_top_tree_upper(aggregation_context, self);
        }
    }

    pub fn add_child_of_child<C: AggregationContext<Info = T>>(
        self: &Arc<Self>,
        aggregation_context: &C,
        child_of_child: &C::ItemRef,
        height: u8,
    ) {
        bottom_tree(aggregation_context, child_of_child, height + 1)
            .add_top_tree_upper(aggregation_context, self);
    }

    pub fn remove_child_of_child<C: AggregationContext<Info = T>>(
        self: &Arc<Self>,
        aggregation_context: &C,
        child_of_child: &C::ItemRef,
        height: u8,
    ) {
        bottom_tree(aggregation_context, child_of_child, height + 1)
            .remove_top_tree_upper(aggregation_context, self);
    }

    pub fn remove_children_of_child<'a, C: AggregationContext<Info = T>>(
        self: &Arc<Self>,
        aggregation_context: &C,
        children: impl IntoIterator<Item = &'a C::ItemRef>,
        height: u8,
    ) where
        C::ItemRef: 'a,
    {
        for child in children {
            bottom_tree(aggregation_context, child, height + 1)
                .remove_top_tree_upper(aggregation_context, self);
        }
    }

    pub fn child_change<C: AggregationContext<Info = T>>(
        &self,
        aggregation_context: &C,
        change: &C::ItemChange,
    ) {
        let mut state = self.state.lock();
        aggregation_context.apply_change(&mut state.data, change);
    }

    pub fn get_root_info<C: AggregationContext<Info = T>>(
        &self,
        aggregation_context: &C,
        root_info_type: &C::RootInfoType,
    ) -> C::RootInfo {
        let state = self.state.lock();
        // This is the root
        aggregation_context.info_to_root_info(&state.data, root_info_type)
    }

    pub fn lock_info(self: &Arc<Self>) -> AggregationInfoGuard<T> {
        AggregationInfoGuard {
            // SAFETY: We can cast the lifetime as we keep a strong reference to the tree.
            // The order of the field in the struct is important to drop guard before tree.
            guard: unsafe { transmute(self.state.lock()) },
            tree: self.clone(),
        }
    }
}

pub struct AggregationInfoGuard<T: 'static> {
    guard: MutexGuard<'static, TopTreeState<T>>,
    #[allow(dead_code, reason = "need to stay alive until the guard is dropped")]
    tree: Arc<TopTree<T>>,
}

impl<T> std::ops::Deref for AggregationInfoGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.guard.data
    }
}

impl<T> std::ops::DerefMut for AggregationInfoGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard.data
    }
}
