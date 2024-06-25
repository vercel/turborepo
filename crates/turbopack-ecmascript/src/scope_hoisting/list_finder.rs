use std::collections::VecDeque;

use anyhow::Result;
use rustc_hash::{FxHashMap, FxHashSet};
use turbo_tasks::Vc;
use turbopack_core::module::Module;

#[turbo_tasks::value]
pub struct Item {
    pub shallow_list: Vc<Vec<Vc<Item>>>,
    pub cond_loaded_modules: Vc<FxHashSet<Vc<Item>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Enter,
    Leave,
}

pub async fn find_list(entrypoints: Vc<Vec<Vc<Item>>>) -> Result<Vc<Vec<Vc<Item>>>> {
    // Create an empty set X for items that have been processed.
    let mut done = FxHashSet::default();
    let mut queue = VecDeque::<(_, ItemKind)>::new();
    queue.extend(entrypoints.await?.iter().map(|vc| (*vc, ItemKind::Enter)));

    // Create an empty list L.
    let mut list = vec![];

    // Create an empty set S.
    let mut set = FxHashSet::default();

    while let Some((item, kind)) = queue.pop_front() {
        // Add item to X.
        if !done.insert(item) {
            continue;
        }

        match kind {
            ItemKind::Enter => {
                // Put all items from the shallow list of the dequeued item at the front of the
                // queue Q with type ENTER
                for shallow_item in item.shallow_list.iter() {
                    queue.push_front((shallow_item, ItemKind::Enter));
                }

                // Put all items from conditional loaded modules into set S.
                for cond_loaded_module in item.cond_loaded_modules.iter() {
                    set.insert(cond_loaded_module);
                }

                // Put the current item into the queue with type LEAVE
                queue.push_back((item, ItemKind::Leave));
            }
            ItemKind::Leave => {
                // Put the current item into the list L.
                list.push(item);
            }
        }

        // Remove all items of L from S.
        //
        // An item is already loaded by an import and conditionally loading won’t have
        // an evaluation effect anymore
        for item in list {
            set.remove(&item);
        }

        // Set the item’s shallow list to L and the conditionally loaded modules
        // to S.
        let item = Item {
            shallow_list: list,
            cond_loaded_modules: set,
        };

        // Enqueue all items from S for processing
        queue.extend(set.iter().map(|vc| (*vc, ItemKind::Enter)));

        // Gather all lists from X.
        let mut lists = vec![];
        for &item in done.iter() {
            lists.push(item.await?.shallow_list.clone());
        }

        // Create a reverse mapping from item → (list, index)[] (sorted by
        // smallest index first)
        //
        // So we can cheaply determine in which lists an item is and at which
        // index.
        let mut reverse_mapping = FxHashMap::default();
        for (i, list) in lists.iter().enumerate() {
            for (j, item) in list.await?.iter().enumerate() {
                reverse_mapping
                    .entry(*item)
                    .or_insert_with(Vec::new)
                    .push((i, j));
            }
        }

        // Remove all mappings that only point to a single list-index tuple.
        //
        // They are already done and we don’t want to sort them.*
        for (item, list_indices) in reverse_mapping.iter_mut() {
            if list_indices.len() == 1 {
                reverse_mapping.remove(item);
            }
        }

        // Sort the mapping by smallest index
        //
        // This is important to be able to find the longest common slice in the
        // following*
        for (_, list_indices) in reverse_mapping.iter_mut() {
            list_indices.sort_by_key(|(list, index)| (*list, *index));
        }

        // For each item in the mappings:

        for mapping in reverse_mapping {
            // We need to split the item into a separate list. We want the list
            // to be as long as possible.

            // Create a new list L.
            let mut list = vec![];

            // TODO
            // Walk all lists at the same item as long as the items in these
            // lists are common (starting by current index):
            {
                // TODO
                // Put item into L

                // TODO
                // Remove item from the mapping.
            }

            // Put L into the set of all lists.
            lists.push(list);

            // TODO
            // Put all remaining items in the lists (starting by current index)
            // into new lists and update mappings correctly. (No need to
            // updating sorting, Items might not have a mapping)
        }

        // At this point every item is exactly in one list.
    }

    Ok(Vc::cell(list))
}
