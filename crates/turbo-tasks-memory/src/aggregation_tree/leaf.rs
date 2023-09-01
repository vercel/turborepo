use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use auto_hash_map::{AutoMap, AutoSet};
use nohash_hasher::BuildNoHashHasher;

use super::{
    bottom_tree::{add_parent_to_item, BottomTree},
    inner_refs::{BottomRef, ChildLocation},
    top_tree::TopTree,
    AggregationContext, AggregationItemLock,
};

pub struct AggregationTreeLeaf<T, I> {
    top_trees: AutoMap<u8, Arc<TopTree<T>>, BuildNoHashHasher<u8>>,
    bottom_trees: AutoMap<u8, Arc<BottomTree<T, I>>, BuildNoHashHasher<u8>>,
    upper: AutoSet<BottomRef<T, I>>,
}

impl<T, I: Clone + Eq + Hash> AggregationTreeLeaf<T, I> {
    pub fn new() -> Self {
        Self {
            top_trees: AutoMap::with_hasher(),
            bottom_trees: AutoMap::with_hasher(),
            upper: AutoSet::new(),
        }
    }

    pub fn add_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        self_is_blue: bool,
        context: &C,
        child: &I,
    ) {
        for BottomRef { parent, location } in self.upper.iter() {
            parent.add_child_of_child(context, *location, self_is_blue, child.clone());
        }
    }

    pub fn remove_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        self_is_blue: bool,
        context: &C,
        child: &I,
    ) {
        for BottomRef { parent, location } in self.upper.iter() {
            parent.remove_child_of_child(context, *location, self_is_blue, child.clone());
        }
    }

    pub fn change<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        change: &C::ItemChange,
    ) {
        context.on_change(change);
        for BottomRef {
            parent,
            location: _,
        } in self.upper.iter()
        {
            parent.child_change(context, change);
        }
    }

    pub fn get_root_info<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        root_info_type: &C::RootInfoType,
    ) -> C::RootInfo {
        let mut result = context.new_root_info(root_info_type);
        for BottomRef {
            parent,
            location: _,
        } in self.upper.iter()
        {
            let info = parent.get_root_info(context, root_info_type);
            if context.merge_root_info(&mut result, info) == ControlFlow::Break(()) {
                break;
            }
        }
        result
    }

    pub fn has_upper(&self) -> bool {
        !self.upper.is_empty()
    }

    #[must_use]
    pub(super) fn add_upper(
        &mut self,
        parent: Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) -> bool {
        self.upper.insert(BottomRef { parent, location })
    }

    #[must_use]
    pub(super) fn remove_upper(
        &mut self,
        parent: Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) -> bool {
        self.upper.remove(&BottomRef { parent, location })
    }
}

pub fn top_tree<C: AggregationContext>(
    context: &C,
    item: &mut C::ItemLock<'_>,
    depth: u8,
) -> Arc<TopTree<C::Info>> {
    if let Some(top_tree) = item.leaf().top_trees.get(&depth) {
        return top_tree.clone();
    }
    let bottom_tree = bottom_tree(context, item, depth);
    let top_tree = Arc::new(TopTree::new(depth));
    bottom_tree.add_top_tree_parent(context, &top_tree);
    item.leaf().top_trees.insert(depth, top_tree.clone());
    top_tree
}

pub fn bottom_tree<C: AggregationContext>(
    context: &C,
    item: &mut C::ItemLock<'_>,
    height: u8,
) -> Arc<BottomTree<C::Info, C::ItemRef>> {
    if let Some(bottom_tree) = item.leaf().bottom_trees.get(&height) {
        return bottom_tree.clone();
    }
    let new_bottom_tree = Arc::new(BottomTree::new(height));
    if height == 0 {
        add_parent_to_item(context, item, &new_bottom_tree, ChildLocation::Left);
    } else {
        bottom_tree(context, item, height - 1).add_bottom_tree_parent(
            context,
            &new_bottom_tree,
            ChildLocation::Left,
        );
    }
    item.leaf()
        .bottom_trees
        .insert(height, new_bottom_tree.clone());
    new_bottom_tree
}
