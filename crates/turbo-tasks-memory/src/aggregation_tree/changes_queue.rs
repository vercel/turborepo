use std::{
    borrow::Cow,
    cmp::{max, min},
    collections::{hash_map::RawEntryMut, HashMap},
    hash::Hash,
    mem::{replace, take},
};

use nohash_hasher::IsEnabled;

use super::{
    inner_refs::{BottomRef, TopRef},
    AggregationContext,
};

fn get_in_vec<K, V>(vec: &mut Vec<HashMap<K, V>>, index: usize) -> &mut HashMap<K, V> {
    if vec.len() <= index {
        vec.resize_with(index + 1, || HashMap::new());
    }
    &mut vec[index]
}

pub struct ChangesQueue<T, I: IsEnabled, U> {
    inner: ChangesQueueInner<T, I, U>,
}

enum ChangesQueueInner<T, I: IsEnabled, U> {
    Empty,
    BottomTreeChange {
        bottom_ref: BottomRef<T, I>,
        change: U,
    },
    TopTreeChange {
        top_ref: TopRef<T>,
        change: U,
    },
    Queue {
        bottom_tree_changes: Vec<HashMap<BottomRef<T, I>, U>>,
        first_bottom_tree_height: u8,
        top_tree_changes: Vec<HashMap<TopRef<T>, U>>,
        first_top_tree_depth: u8,
    },
}

fn add_to_bottom_queue<
    C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>,
    T,
    I: IsEnabled,
    U: Clone,
>(
    aggregation_context: &C,
    bottom_tree_changes: &mut Vec<HashMap<BottomRef<T, I>, U>>,
    first_bottom_tree_height: &mut u8,
    bottom_ref: &BottomRef<T, I>,
    change: Cow<'_, U>,
) {
    let height = bottom_ref.upper.height;
    let map = get_in_vec(bottom_tree_changes, height as usize);
    if map.is_empty() {
        *first_bottom_tree_height = min(*first_bottom_tree_height, height);
    }
    match map.raw_entry_mut().from_key(bottom_ref) {
        RawEntryMut::Occupied(mut entry) => {
            let current = entry.get_mut();
            aggregation_context.merge_change(current, change);
        }
        RawEntryMut::Vacant(entry) => {
            entry.insert(bottom_ref.clone(), change.into_owned());
        }
    }
}

fn add_to_top_queue<
    C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>,
    T,
    I: IsEnabled,
    U: Clone,
>(
    aggregation_context: &C,
    top_tree_changes: &mut Vec<HashMap<TopRef<T>, U>>,
    first_top_tree_depth: &mut u8,
    top_ref: &TopRef<T>,
    change: Cow<'_, U>,
) {
    let depth = top_ref.upper.depth;
    let map = get_in_vec(top_tree_changes, depth as usize);
    if map.is_empty() {
        *first_top_tree_depth = max(*first_top_tree_depth, depth);
    }
    match map.raw_entry_mut().from_key(top_ref) {
        RawEntryMut::Occupied(mut entry) => {
            let current = entry.get_mut();
            aggregation_context.merge_change(current, change);
        }
        RawEntryMut::Vacant(entry) => {
            entry.insert(top_ref.clone(), change.into_owned());
        }
    }
}

impl<T, I: Clone + Eq + Hash + IsEnabled, U: Clone> ChangesQueue<T, I, U> {
    pub fn new() -> Self {
        Self {
            inner: ChangesQueueInner::Empty,
        }
    }

    pub fn add_bottom_change<C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>>(
        &mut self,
        aggregation_context: &C,
        bottom_ref: &BottomRef<T, I>,
        change: Cow<'_, U>,
    ) {
        match &mut self.inner {
            ChangesQueueInner::Empty => {
                self.inner = ChangesQueueInner::BottomTreeChange {
                    bottom_ref: bottom_ref.clone(),
                    change: change.into_owned(),
                };
            }
            ChangesQueueInner::BottomTreeChange {
                bottom_ref: old_bottom_ref,
                change: old_change,
            } => {
                let mut bottom_tree_changes = Vec::new();
                let mut first_bottom_tree_height = u8::MAX;
                add_to_bottom_queue(
                    aggregation_context,
                    &mut bottom_tree_changes,
                    &mut first_bottom_tree_height,
                    old_bottom_ref,
                    Cow::Borrowed(&*old_change),
                );
                add_to_bottom_queue(
                    aggregation_context,
                    &mut bottom_tree_changes,
                    &mut first_bottom_tree_height,
                    bottom_ref,
                    change,
                );
                self.inner = ChangesQueueInner::Queue {
                    bottom_tree_changes,
                    first_bottom_tree_height,
                    top_tree_changes: Vec::new(),
                    first_top_tree_depth: 0,
                };
            }
            ChangesQueueInner::TopTreeChange {
                top_ref,
                change: old_change,
            } => {
                let mut bottom_tree_changes = Vec::new();
                let mut first_bottom_tree_height = u8::MAX;
                let mut top_tree_changes = Vec::new();
                let mut first_top_tree_depth = 0;
                add_to_top_queue(
                    aggregation_context,
                    &mut top_tree_changes,
                    &mut first_top_tree_depth,
                    top_ref,
                    Cow::Borrowed(&*old_change),
                );
                add_to_bottom_queue(
                    aggregation_context,
                    &mut bottom_tree_changes,
                    &mut first_bottom_tree_height,
                    bottom_ref,
                    change,
                );
                self.inner = ChangesQueueInner::Queue {
                    bottom_tree_changes,
                    first_bottom_tree_height,
                    top_tree_changes,
                    first_top_tree_depth,
                };
            }
            ChangesQueueInner::Queue {
                bottom_tree_changes,
                first_bottom_tree_height,
                top_tree_changes: _,
                first_top_tree_depth: _,
            } => {
                add_to_bottom_queue(
                    aggregation_context,
                    bottom_tree_changes,
                    first_bottom_tree_height,
                    bottom_ref,
                    change,
                );
            }
        }
    }

    pub fn add_top_change<C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>>(
        &mut self,
        aggregation_context: &C,
        top_ref: &TopRef<T>,
        change: Cow<'_, U>,
    ) {
        match &mut self.inner {
            ChangesQueueInner::Empty => {
                self.inner = ChangesQueueInner::TopTreeChange {
                    top_ref: top_ref.clone(),
                    change: change.into_owned(),
                };
            }
            ChangesQueueInner::BottomTreeChange {
                bottom_ref,
                change: old_change,
            } => {
                let mut bottom_tree_changes = Vec::new();
                let mut first_bottom_tree_height = u8::MAX;
                let mut top_tree_changes = Vec::new();
                let mut first_top_tree_depth = 0;
                add_to_bottom_queue(
                    aggregation_context,
                    &mut bottom_tree_changes,
                    &mut first_bottom_tree_height,
                    bottom_ref,
                    Cow::Borrowed(&*old_change),
                );
                add_to_top_queue(
                    aggregation_context,
                    &mut top_tree_changes,
                    &mut first_top_tree_depth,
                    top_ref,
                    change,
                );
                self.inner = ChangesQueueInner::Queue {
                    bottom_tree_changes,
                    first_bottom_tree_height,
                    top_tree_changes,
                    first_top_tree_depth,
                };
            }
            ChangesQueueInner::TopTreeChange {
                top_ref: old_top_ref,
                change: old_change,
            } => {
                let mut top_tree_changes = Vec::new();
                let mut first_top_tree_depth = 0;
                add_to_top_queue(
                    aggregation_context,
                    &mut top_tree_changes,
                    &mut first_top_tree_depth,
                    old_top_ref,
                    Cow::Borrowed(&*old_change),
                );
                add_to_top_queue(
                    aggregation_context,
                    &mut top_tree_changes,
                    &mut first_top_tree_depth,
                    top_ref,
                    change,
                );
                self.inner = ChangesQueueInner::Queue {
                    bottom_tree_changes: Vec::new(),
                    first_bottom_tree_height: u8::MAX,
                    top_tree_changes,
                    first_top_tree_depth,
                };
            }
            ChangesQueueInner::Queue {
                bottom_tree_changes: _,
                first_bottom_tree_height: _,
                top_tree_changes,
                first_top_tree_depth,
            } => {
                add_to_top_queue(
                    aggregation_context,
                    top_tree_changes,
                    first_top_tree_depth,
                    top_ref,
                    change,
                );
            }
        }
    }

    pub fn apply_changes<C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>>(
        &mut self,
        aggregation_context: &C,
    ) {
        loop {
            if let ChangesQueueInner::Queue {
                bottom_tree_changes,
                first_bottom_tree_height,
                top_tree_changes,
                first_top_tree_depth,
            } = &mut self.inner
            {
                if *first_bottom_tree_height < bottom_tree_changes.len() as u8 {
                    let map = take(&mut bottom_tree_changes[*first_bottom_tree_height as usize]);
                    *first_bottom_tree_height += 1;
                    for (bottom_ref, change) in map {
                        let bottom_tree = &bottom_ref.upper;
                        bottom_tree.apply_change(aggregation_context, self, &change);
                    }
                } else if top_tree_changes.is_empty() {
                    self.inner = ChangesQueueInner::Empty;
                    return;
                } else if *first_top_tree_depth < top_tree_changes.len() as u8 {
                    let map = take(&mut top_tree_changes[*first_top_tree_depth as usize]);
                    if *first_top_tree_depth == 0 {
                        if map.is_empty() {
                            self.inner = ChangesQueueInner::Empty;
                            return;
                        }
                    } else {
                        *first_top_tree_depth -= 1;
                    }

                    for (top_ref, change) in map {
                        let top_tree = &top_ref.upper;
                        top_tree.apply_change(aggregation_context, self, &change);
                    }
                }
            } else {
                match replace(&mut self.inner, ChangesQueueInner::Empty) {
                    ChangesQueueInner::Empty => {
                        return;
                    }
                    ChangesQueueInner::BottomTreeChange { bottom_ref, change } => {
                        bottom_ref
                            .upper
                            .apply_change(aggregation_context, self, &change);
                    }
                    ChangesQueueInner::TopTreeChange { top_ref, change } => {
                        top_ref
                            .upper
                            .apply_change(aggregation_context, self, &change);
                    }
                    ChangesQueueInner::Queue { .. } => unreachable!(),
                }
            }
        }
    }
}

#[cfg(debug_assertions)]
impl<T, I: IsEnabled, U> Drop for ChangesQueue<T, I, U> {
    fn drop(&mut self) {
        if !matches!(self.inner, ChangesQueueInner::Empty) {
            panic!("ChangesQueue::apply_changes was not called");
        }
    }
}
