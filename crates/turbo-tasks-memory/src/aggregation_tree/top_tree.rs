use std::{mem::transmute, sync::Arc};

use parking_lot::{Mutex, MutexGuard};

use super::{inner_refs::TopRef, leaf::top_tree, AggregationContext};
use crate::count_hash_set::CountHashSet;

pub struct TopTree<T: AggregationContext> {
    depth: u8,
    state: Mutex<TopTreeState<T>>,
}

struct TopTreeState<T: AggregationContext> {
    data: T::Info,
    upper: CountHashSet<TopRef<T>>,
}

impl<T: AggregationContext> TopTree<T> {
    pub fn new(depth: u8) -> Self {
        Self {
            depth,
            state: Mutex::new(TopTreeState {
                data: T::new_info(),
                upper: CountHashSet::new(),
            }),
        }
    }

    pub(super) fn add_child_of_child(self: Arc<Self>, context: &T, child_of_child: T::ItemRef) {
        top_tree(context, &mut context.item(child_of_child), self.depth + 1)
            .add_parent(context, self);
    }

    pub(super) fn remove_child_of_child(self: Arc<Self>, context: &T, child_of_child: T::ItemRef) {
        top_tree(context, &mut context.item(child_of_child), self.depth + 1)
            .remove_parent(context, self);
    }

    pub(super) fn add_parent(&self, context: &T, parent: Arc<TopTree<T>>) {
        let mut state = self.state.lock();
        if let Some(change) = context.info_to_add_change(&state.data) {
            parent.child_change(context, &change);
        }
        state.upper.add(TopRef { parent });
    }

    pub(super) fn remove_parent(&self, context: &T, parent: Arc<TopTree<T>>) {
        let mut state = self.state.lock();
        if let Some(change) = context.info_to_remove_change(&state.data) {
            parent.child_change(context, &change);
        }
        state.upper.remove(TopRef { parent });
    }

    pub(super) fn info(self: Arc<Self>) -> AggregationInfoGuard<T> {
        AggregationInfoGuard {
            // SAFETY: We can cast the lifetime as we keep a strong reference to the tree.
            // The order of the field in the struct is important to drop guard before tree.
            guard: unsafe { transmute(self.state.lock()) },
            tree: self.clone(),
        }
    }

    pub(super) fn child_change(&self, context: &T, change: &T::ItemChange) {
        let mut state = self.state.lock();
        let change = context.apply_change(&mut state.data, change);
        propagate_change_to_upper(&mut state, context, change);
    }
}

fn propagate_change_to_upper<T: AggregationContext>(
    state: &mut MutexGuard<TopTreeState<T>>,
    context: &T,
    change: Option<T::ItemChange>,
) {
    let Some(change) = change else {
        return;
    };
    for TopRef { parent } in state.upper.iter() {
        parent.child_change(context, &change);
    }
}

pub struct AggregationInfoGuard<T: AggregationContext + 'static> {
    guard: MutexGuard<'static, TopTreeState<T>>,
    tree: Arc<TopTree<T>>,
}

impl<T: AggregationContext> std::ops::Deref for AggregationInfoGuard<T> {
    type Target = T::Info;

    fn deref(&self) -> &Self::Target {
        &self.guard.data
    }
}
