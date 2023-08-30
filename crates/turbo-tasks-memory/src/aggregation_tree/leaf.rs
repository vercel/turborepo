use std::sync::Arc;

use auto_hash_map::AutoMap;
use nohash_hasher::BuildNoHashHasher;

use super::{
    bottom_tree::{add_parent_to_item, BottomTree},
    inner_refs::{BottomRef, ChildLocation},
    top_tree::TopTree,
    AggregationContext, AggregationItemLock,
};
use crate::count_hash_set::CountHashSet;

pub struct AggregationTreeLeaf<T: AggregationContext> {
    top_trees: AutoMap<u8, Arc<TopTree<T>>, BuildNoHashHasher<u8>>,
    bottom_trees: AutoMap<u8, Arc<BottomTree<T>>, BuildNoHashHasher<u8>>,
    upper: CountHashSet<BottomRef<T>>,
}

impl<T: AggregationContext> AggregationTreeLeaf<T> {
    pub fn new() -> Self {
        Self {
            top_trees: AutoMap::with_hasher(),
            bottom_trees: AutoMap::with_hasher(),
            upper: CountHashSet::new(),
        }
    }

    pub fn add_child(&self, self_is_blue: bool, context: &T, child: &T::ItemRef) {
        for BottomRef { parent, location } in self.upper.iter() {
            parent
                .clone()
                .add_child_of_child(context, *location, self_is_blue, child.clone());
        }
    }

    pub fn remove_child(&self, self_is_blue: bool, context: &T, child: &T::ItemRef) {
        for BottomRef { parent, location } in self.upper.iter() {
            parent.remove_child_of_child(context, *location, self_is_blue, child.clone());
        }
    }

    pub fn change(&self, context: &T, change: &T::ItemChange) {
        for BottomRef {
            parent,
            location: _,
        } in self.upper.iter()
        {
            parent.child_change(context, change);
        }
    }

    #[must_use]
    pub(super) fn add_upper(
        &mut self,
        parent: Arc<BottomTree<T>>,
        location: ChildLocation,
    ) -> bool {
        self.upper.add(BottomRef { parent, location })
    }

    #[must_use]
    pub(super) fn remove_upper(
        &mut self,
        parent: Arc<BottomTree<T>>,
        location: ChildLocation,
    ) -> bool {
        self.upper.remove(BottomRef { parent, location })
    }
}

pub fn top_tree<T: AggregationContext>(
    context: &T,
    item: &mut T::ItemLock,
    depth: u8,
) -> Arc<TopTree<T>> {
    if let Some(top_tree) = item.leaf().top_trees.get(&depth) {
        return top_tree.clone();
    }
    let bottom_tree = bottom_tree(context, item, depth);
    let top_tree = Arc::new(TopTree::new(depth));
    bottom_tree.add_top_tree_parent(context, top_tree.clone());
    item.leaf().top_trees.insert(depth, top_tree.clone());
    top_tree
}

pub fn bottom_tree<T: AggregationContext>(
    context: &T,
    item: &mut T::ItemLock,
    height: u8,
) -> Arc<BottomTree<T>> {
    if let Some(bottom_tree) = item.leaf().bottom_trees.get(&height) {
        return bottom_tree.clone();
    }
    let new_bottom_tree = Arc::new(BottomTree::new(height));
    if height == 0 {
        add_parent_to_item(context, item, new_bottom_tree.clone(), ChildLocation::Left);
    } else {
        bottom_tree(context, item, height - 1).add_bottom_tree_parent(
            context,
            new_bottom_tree.clone(),
            ChildLocation::Left,
        );
    }
    item.leaf()
        .bottom_trees
        .insert(height, new_bottom_tree.clone());
    new_bottom_tree
}
