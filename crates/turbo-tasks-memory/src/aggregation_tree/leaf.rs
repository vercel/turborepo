use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use auto_hash_map::AutoMap;
use nohash_hasher::{BuildNoHashHasher, IsEnabled};

use super::{
    bottom_tree::{add_upper_to_item_step_1, add_upper_to_item_step_2, BottomTree},
    inner_refs::{BottomRef, ChildLocation},
    top_tree::TopTree,
    AggregationContext, AggregationItemLock,
};
use crate::count_hash_set::CountHashSet;

pub struct AggregationTreeLeaf<T, I: IsEnabled> {
    top_trees: AutoMap<u8, Arc<TopTree<T>>, BuildNoHashHasher<u8>>,
    bottom_trees: AutoMap<u8, Arc<BottomTree<T, I>>, BuildNoHashHasher<u8>>,
    left_upper: Option<Arc<BottomTree<T, I>>>,
    inner_upper: CountHashSet<BottomRef<T, I>>,
}

impl<T, I: Clone + Eq + Hash + IsEnabled> AggregationTreeLeaf<T, I> {
    pub fn new() -> Self {
        Self {
            top_trees: AutoMap::with_hasher(),
            bottom_trees: AutoMap::with_hasher(),
            left_upper: None,
            inner_upper: CountHashSet::new(),
        }
    }

    pub fn add_child<C: AggregationContext<Info = T, ItemRef = I>>(&self, context: &C, child: &I) {
        let hash = context.hash(child);
        if let Some(upper) = self.left_upper.as_ref() {
            upper.add_child_of_child(context, ChildLocation::Left, child, hash);
        }
        for BottomRef { upper } in self.inner_upper.iter() {
            upper.add_child_of_child(context, ChildLocation::Inner, child, hash);
        }
    }

    pub fn add_child_job<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &'a C,
        child: &'a I,
    ) -> impl FnOnce() + 'a
    where
        T: 'a,
    {
        let left_upper = self.left_upper.clone();
        let inner_upper = self.inner_upper.iter().cloned().collect::<Vec<_>>();
        move || {
            let hash = context.hash(child);
            if let Some(upper) = left_upper {
                upper.add_child_of_child(context, ChildLocation::Left, child, hash);
            }
            for BottomRef { upper } in inner_upper {
                upper.add_child_of_child(context, ChildLocation::Inner, child, hash);
            }
        }
    }

    pub fn remove_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        child: &I,
    ) {
        let hash = context.hash(child);
        if let Some(upper) = self.left_upper.as_ref() {
            upper.remove_child_of_child(context, ChildLocation::Left, child, hash);
        }
        for BottomRef { upper } in self.inner_upper.iter() {
            upper.remove_child_of_child(context, ChildLocation::Inner, child, hash);
        }
    }

    pub fn change<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        change: &C::ItemChange,
    ) {
        context.on_change(change);
        if let Some(upper) = self.left_upper.as_ref() {
            upper.child_change(context, change);
        }
        for BottomRef { upper } in self.inner_upper.iter() {
            upper.child_change(context, change);
        }
    }

    pub fn get_root_info<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        root_info_type: &C::RootInfoType,
    ) -> C::RootInfo {
        let mut result = context.new_root_info(root_info_type);
        if let Some(upper) = self.left_upper.as_ref() {
            let info = upper.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                return result;
            }
        }
        for BottomRef { upper } in self.inner_upper.iter() {
            let info = upper.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                break;
            }
        }
        result
    }

    pub fn has_upper(&self) -> bool {
        self.left_upper.is_some() || !self.inner_upper.is_empty()
    }

    #[must_use]
    pub(super) fn add_upper(
        &mut self,
        upper: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) -> bool {
        match location {
            ChildLocation::Left => {
                self.left_upper = Some(upper.clone());
                true
            }
            ChildLocation::Inner => self.inner_upper.add(BottomRef {
                upper: upper.clone(),
            }),
        }
    }

    #[must_use]
    pub(super) fn remove_upper(
        &mut self,
        upper: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) -> bool {
        debug_assert!(matches!(location, ChildLocation::Inner));
        self.inner_upper.remove(BottomRef {
            upper: upper.clone(),
        })
    }
}

pub fn top_tree<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    depth: u8,
) -> Arc<TopTree<C::Info>> {
    let new_top_tree = {
        let mut item = context.item(reference);
        let leaf = item.leaf();
        if let Some(top_tree) = leaf.top_trees.get(&depth) {
            return top_tree.clone();
        }
        let new_top_tree = Arc::new(TopTree::new(depth));
        leaf.top_trees.insert(depth, new_top_tree.clone());
        new_top_tree
    };
    let bottom_tree = bottom_tree(context, reference, (depth + 1) * 4);
    bottom_tree.add_top_tree_upper(context, &new_top_tree);
    new_top_tree
}

pub fn bottom_tree<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    height: u8,
) -> Arc<BottomTree<C::Info, C::ItemRef>> {
    let new_bottom_tree;
    let mut result = None;
    {
        let mut item = context.item(reference);
        let is_blue = ((item.hash() >> height >> 1) & 1) == 0;
        let leaf = item.leaf();
        if let Some(bottom_tree) = leaf.bottom_trees.get(&height) {
            return bottom_tree.clone();
        }
        new_bottom_tree = Arc::new(BottomTree::new(height, is_blue));
        leaf.bottom_trees.insert(height, new_bottom_tree.clone());
        if height == 0 {
            result = Some(add_upper_to_item_step_1::<C>(
                &mut item,
                &new_bottom_tree,
                ChildLocation::Left,
            ));
        }
    }
    if let Some(result) = result {
        add_upper_to_item_step_2(context, &new_bottom_tree, result);
    }
    if height != 0 {
        bottom_tree(context, reference, height - 1).add_bottom_tree_upper(
            context,
            &new_bottom_tree,
            ChildLocation::Left,
        );
    }
    new_bottom_tree
}
