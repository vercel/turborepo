use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use auto_hash_map::AutoMap;
use nohash_hasher::BuildNoHashHasher;

use super::{
    bottom_tree::{add_parent_to_item, BottomTree},
    inner_refs::ChildLocation,
    top_tree::TopTree,
    upper_map::UpperMap,
    AggregationContext, AggregationItemLock,
};

pub struct AggregationTreeLeaf<T, I> {
    top_trees: AutoMap<u8, Arc<TopTree<T>>, BuildNoHashHasher<u8>>,
    bottom_trees: AutoMap<u8, Arc<BottomTree<T, I>>, BuildNoHashHasher<u8>>,
    upper: UpperMap<BottomTree<T, I>>,
}

impl<T, I: Clone + Eq + Hash> AggregationTreeLeaf<T, I> {
    pub fn new() -> Self {
        Self {
            top_trees: AutoMap::with_hasher(),
            bottom_trees: AutoMap::with_hasher(),
            upper: UpperMap::new(),
        }
    }

    pub fn add_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        self_is_blue: bool,
        context: &C,
        child: &I,
    ) {
        for (parent, location) in self.upper.iter() {
            parent.add_child_of_child(context, location, self_is_blue, child);
        }
    }

    pub fn remove_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        self_is_blue: bool,
        context: &C,
        child: &I,
    ) {
        for (parent, location) in self.upper.iter() {
            parent.remove_child_of_child(context, location, self_is_blue, child);
        }
    }

    pub fn change<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        change: &C::ItemChange,
    ) {
        context.on_change(change);
        for parent in self.upper.keys() {
            parent.child_change(context, change);
        }
    }

    pub fn get_root_info<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        root_info_type: &C::RootInfoType,
    ) -> C::RootInfo {
        let mut result = context.new_root_info(root_info_type);
        for parent in self.upper.keys() {
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
        parent: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) -> bool {
        match location {
            ChildLocation::Left => {
                self.upper.init_left(parent.clone());
                true
            }
            ChildLocation::Middle => self.upper.add_middle(parent.clone()),
            ChildLocation::Right => self.upper.add_right(parent.clone()),
        }
    }

    #[must_use]
    pub(super) fn remove_upper(
        &mut self,
        parent: &Arc<BottomTree<T, I>>,
        location: ChildLocation,
    ) -> Option<ChildLocation> {
        match location {
            ChildLocation::Left => unreachable!(),
            ChildLocation::Middle => self.upper.remove_middle(parent.clone()),
            ChildLocation::Right => self.upper.remove_right(parent.clone()),
        }
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
    let bottom_tree = bottom_tree(context, reference, depth);
    bottom_tree.add_top_tree_parent(context, &new_top_tree);
    new_top_tree
}

pub fn bottom_tree<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    height: u8,
) -> Arc<BottomTree<C::Info, C::ItemRef>> {
    let new_bottom_tree = {
        let mut item = context.item(reference);
        let leaf = item.leaf();
        if let Some(bottom_tree) = leaf.bottom_trees.get(&height) {
            return bottom_tree.clone();
        }
        let new_bottom_tree = Arc::new(BottomTree::new(height));
        leaf.bottom_trees.insert(height, new_bottom_tree.clone());
        if height == 0 {
            add_parent_to_item(context, &mut item, &new_bottom_tree, ChildLocation::Left);
        }
        new_bottom_tree
    };
    if height != 0 {
        bottom_tree(context, reference, height - 1).add_bottom_tree_parent(
            context,
            &new_bottom_tree,
            ChildLocation::Left,
        );
    }
    new_bottom_tree
}
