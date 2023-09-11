use std::{hash::Hash, ops::ControlFlow, sync::Arc};

use nohash_hasher::IsEnabled;
use ref_cast::RefCast;

use super::{
    bottom_tree::BottomTree,
    inner_refs::{BottomRef, ChildLocation},
    top_tree::TopTree,
    AggregationContext, AggregationItemLock, MAX_INNER_UPPERS,
};
use crate::count_hash_set::CountHashSet;

pub struct AggregationTreeLeaf<T, I: IsEnabled> {
    top_trees: Vec<Option<Arc<TopTree<T>>>>,
    bottom_trees: Vec<Option<Arc<BottomTree<T, I>>>>,
    left_upper: Option<Arc<BottomTree<T, I>>>,
    inner_upper: CountHashSet<BottomRef<T, I>>,
}

impl<T, I: Clone + Eq + Hash + IsEnabled> AggregationTreeLeaf<T, I> {
    pub fn new() -> Self {
        Self {
            top_trees: Vec::new(),
            bottom_trees: Vec::new(),
            left_upper: None,
            inner_upper: CountHashSet::new(),
        }
    }

    pub fn add_children<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        children: impl IntoIterator<Item = &'a I>,
    ) where
        I: 'a,
    {
        let children = children
            .into_iter()
            .map(|child| (context.hash(child), child));
        // Only collect the children into a Vec when neccessary
        if self.inner_upper.is_empty() {
            if let Some(upper) = self.left_upper.as_ref() {
                upper.add_children_of_child(context, ChildLocation::Left, children, 0);
            }
        } else {
            if let Some(upper) = self.left_upper.as_ref() {
                let children = children.collect::<Vec<_>>();
                upper.add_children_of_child(
                    context,
                    ChildLocation::Left,
                    children.iter().copied(),
                    0,
                );
                for BottomRef { upper } in self.inner_upper.iter() {
                    upper.add_children_of_child(
                        context,
                        ChildLocation::Inner,
                        children.iter().copied(),
                        0,
                    );
                }
            } else if self.inner_upper.len() == 1 {
                let BottomRef { upper } = self.inner_upper.iter().next().unwrap();
                upper.add_children_of_child(context, ChildLocation::Inner, children, 0);
            } else {
                let children = children.collect::<Vec<_>>();
                for BottomRef { upper } in self.inner_upper.iter() {
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

    pub fn add_child<C: AggregationContext<Info = T, ItemRef = I>>(&self, context: &C, child: &I) {
        let hash = context.hash(child);
        if let Some(upper) = self.left_upper.as_ref() {
            upper.add_child_of_child(context, ChildLocation::Left, child, hash, 0);
        }
        for BottomRef { upper } in self.inner_upper.iter() {
            upper.add_child_of_child(context, ChildLocation::Inner, child, hash, 0);
        }
    }

    pub fn add_children_job<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &'a C,
        children: Vec<I>,
    ) -> impl FnOnce() + 'a
    where
        I: 'a,
        T: 'a,
    {
        let left_upper = self.left_upper.clone();
        let inner_upper = self.inner_upper.iter().cloned().collect::<Vec<_>>();
        move || {
            let children = children
                .iter()
                .map(|child| (context.hash(child), child))
                .collect::<Vec<_>>();
            if let Some(upper) = left_upper {
                upper.add_children_of_child(
                    context,
                    ChildLocation::Left,
                    children.iter().copied(),
                    0,
                );
            }
            for BottomRef { upper } in inner_upper {
                upper.add_children_of_child(
                    context,
                    ChildLocation::Inner,
                    children.iter().copied(),
                    0,
                );
            }
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
                upper.add_child_of_child(context, ChildLocation::Left, child, hash, 0);
            }
            for BottomRef { upper } in inner_upper {
                upper.add_child_of_child(context, ChildLocation::Inner, child, hash, 0);
            }
        }
    }

    pub fn remove_child<C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &C,
        child: &I,
    ) {
        if let Some(upper) = self.left_upper.as_ref() {
            upper.remove_child_of_child(context, child);
        }
        for BottomRef { upper } in self.inner_upper.iter() {
            upper.remove_child_of_child(context, child);
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

    pub fn change_job<'a, C: AggregationContext<Info = T, ItemRef = I>>(
        &self,
        context: &'a C,
        change: C::ItemChange,
    ) -> impl FnOnce() + 'a
    where
        I: 'a,
        T: 'a,
    {
        let left_upper = self.left_upper.clone();
        let inner_upper = self.inner_upper.iter().cloned().collect::<Vec<_>>();
        move || {
            context.on_change(&change);
            if let Some(upper) = left_upper {
                upper.child_change(context, &change);
            }
            for BottomRef { upper } in inner_upper {
                upper.child_change(context, &change);
            }
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
        self.left_upper.is_some() || !self.inner_upper.is_unset()
    }
}

fn get_or_create_in_vec<T>(
    vec: &mut Vec<Option<T>>,
    index: usize,
    create: impl FnOnce() -> T,
) -> (&mut T, bool) {
    if vec.len() <= index {
        vec.resize_with(index + 1, || None);
    }
    let item = &mut vec[index];
    if item.is_none() {
        *item = Some(create());
        (item.as_mut().unwrap(), true)
    } else {
        (item.as_mut().unwrap(), false)
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
        let (tree, new) = get_or_create_in_vec(&mut leaf.top_trees, depth as usize, || {
            Arc::new(TopTree::new(depth))
        });
        if !new {
            return tree.clone();
        }
        tree.clone()
    };
    let bottom_tree = bottom_tree(context, reference, depth + 4);
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
        let leaf = item.leaf();
        let (tree, new) = get_or_create_in_vec(&mut leaf.bottom_trees, height as usize, || {
            Arc::new(BottomTree::new(reference.clone(), height))
        });
        if !new {
            return tree.clone();
        }
        new_bottom_tree = tree.clone();
        if height == 0 {
            result = Some(add_left_upper_to_item_step_1::<C>(
                &mut item,
                &new_bottom_tree,
            ));
        }
    }
    if let Some(result) = result {
        add_left_upper_to_item_step_2(context, &new_bottom_tree, result);
    }
    if height != 0 {
        bottom_tree(context, reference, height - 1)
            .add_left_bottom_tree_upper(context, &new_bottom_tree);
    }
    new_bottom_tree
}

#[must_use]
pub fn add_inner_upper_to_item<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    force_inner: bool,
    nesting_level: u8,
) -> bool {
    let (change, children) = {
        let mut item = context.item(reference);
        let leaf = item.leaf();
        if !force_inner && (leaf.inner_upper.len() >= MAX_INNER_UPPERS || leaf.left_upper.is_some())
        {
            return leaf.inner_upper.add_if_entry(BottomRef::ref_cast(upper));
        }
        let new = leaf.inner_upper.add(BottomRef {
            upper: upper.clone(),
        });
        if new {
            let change = item.get_add_change();
            (
                change,
                item.children().map(|r| r.into_owned()).collect::<Vec<_>>(),
            )
        } else {
            return true;
        }
    };
    if let Some(change) = change {
        context.on_add_change(&change);
        upper.child_change(context, &change);
    }
    if !children.is_empty() {
        upper.add_children_of_child(
            context,
            ChildLocation::Inner,
            children.iter().map(|child| (context.hash(&child), child)),
            nesting_level + 1,
        )
    }
    true
}

#[must_use]
fn add_left_upper_to_item_step_1<C: AggregationContext>(
    item: &mut C::ItemLock<'_>,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
) -> (Option<C::ItemChange>, Vec<C::ItemRef>) {
    item.leaf().left_upper = Some(upper.clone());
    (
        item.get_add_change(),
        item.children().map(|r| r.into_owned()).collect(),
    )
}

fn add_left_upper_to_item_step_2<C: AggregationContext>(
    context: &C,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
    step_1_result: (Option<C::ItemChange>, Vec<C::ItemRef>),
) {
    let (change, children) = step_1_result;
    if let Some(change) = change {
        context.on_add_change(&change);
        upper.child_change(context, &change);
    }
    if !children.is_empty() {
        upper.add_children_of_child(
            context,
            ChildLocation::Left,
            children.iter().map(|child| (context.hash(&child), child)),
            1,
        )
    }
}

pub fn remove_left_upper_from_item<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
) {
    let mut item = context.item(reference);
    let leaf = &mut item.leaf();
    debug_assert!(if let Some(left_upper) = leaf.left_upper.as_ref() {
        Arc::ptr_eq(left_upper, upper)
    } else {
        false
    });
    leaf.left_upper = None;
    let change = item.get_remove_change();
    let children = item.children().map(|r| r.into_owned()).collect::<Vec<_>>();
    drop(item);
    if let Some(change) = change {
        context.on_remove_change(&change);
        upper.child_change(context, &change);
    }
    for child in children {
        upper.remove_child_of_child(context, &child)
    }
}

pub fn remove_inner_upper_from_item<C: AggregationContext>(
    context: &C,
    reference: &C::ItemRef,
    upper: &Arc<BottomTree<C::Info, C::ItemRef>>,
) {
    let (change, children) = {
        let mut item = context.item(reference);
        let leaf = &mut item.leaf();
        let removed = leaf.inner_upper.remove(BottomRef {
            upper: upper.clone(),
        });
        if removed {
            let change = item.get_remove_change();
            (
                change,
                item.children().map(|r| r.into_owned()).collect::<Vec<_>>(),
            )
        } else {
            return;
        }
    };
    if let Some(change) = change {
        context.on_remove_change(&change);
        upper.child_change(context, &change);
    }
    for child in children {
        upper.remove_child_of_child(context, &child)
    }
}
