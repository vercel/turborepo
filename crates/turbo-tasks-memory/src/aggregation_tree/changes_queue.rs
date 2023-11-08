use std::{
    borrow::Cow,
    cmp::{max, min},
    collections::{hash_map::RawEntryMut, HashMap},
    hash::Hash,
    mem::take,
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
    bottom_tree_changes: Vec<HashMap<BottomRef<T, I>, U>>,
    first_bottom_tree_height: u8,
    top_tree_changes: Vec<HashMap<TopRef<T>, U>>,
    first_top_tree_depth: u8,
}

impl<T, I: Clone + Eq + Hash + IsEnabled, U: Clone> ChangesQueue<T, I, U> {
    pub fn new() -> Self {
        Self {
            bottom_tree_changes: Vec::new(),
            first_bottom_tree_height: u8::MAX,
            top_tree_changes: Vec::new(),
            first_top_tree_depth: 0,
        }
    }

    pub fn add_bottom_change<C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>>(
        &mut self,
        aggregation_context: &C,
        bottom_ref: &BottomRef<T, I>,
        change: Cow<'_, U>,
    ) {
        let height = bottom_ref.upper.height;
        let map = get_in_vec(&mut self.bottom_tree_changes, height as usize);
        if map.is_empty() {
            self.first_bottom_tree_height = min(self.first_bottom_tree_height, height);
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

    pub fn add_top_change<C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>>(
        &mut self,
        aggregation_context: &C,
        top_ref: &TopRef<T>,
        change: Cow<'_, U>,
    ) {
        let depth = top_ref.upper.depth;
        let map = get_in_vec(&mut self.top_tree_changes, depth as usize);
        if map.is_empty() {
            self.first_top_tree_depth = max(self.first_top_tree_depth, depth);
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

    pub fn apply_changes<C: AggregationContext<Info = T, ItemRef = I, ItemChange = U>>(
        &mut self,
        aggregation_context: &C,
    ) {
        while self.first_bottom_tree_height < self.bottom_tree_changes.len() as u8 {
            let map = &mut self.bottom_tree_changes[self.first_bottom_tree_height as usize];
            self.first_bottom_tree_height += 1;
            for (bottom_ref, change) in take(map) {
                let bottom_tree = &bottom_ref.upper;
                bottom_tree.apply_change(aggregation_context, self, &change);
            }
        }
        if !self.top_tree_changes.is_empty() {
            loop {
                let map = &mut self.top_tree_changes[self.first_top_tree_depth as usize];
                if self.first_top_tree_depth == 0 {
                    if map.is_empty() {
                        break;
                    }
                } else {
                    self.first_top_tree_depth -= 1;
                }
                for (top_ref, change) in take(map) {
                    let top_tree = &top_ref.upper;
                    top_tree.apply_change(aggregation_context, self, &change);
                }
            }
        }
    }
}

#[cfg(debug_assertions)]
impl<T, I: IsEnabled, U> Drop for ChangesQueue<T, I, U> {
    fn drop(&mut self) {
        if self.first_bottom_tree_height < self.bottom_tree_changes.len() as u8 {
            panic!("ChangesQueue was dropped without applying all changes");
        }
        if self.first_top_tree_depth != 0 {
            panic!("ChangesQueue was dropped without applying all changes");
        }
    }
}
