use std::{collections::VecDeque, mem::take};

use anyhow::Result;
use indexmap::IndexSet;
use rustc_hash::FxHashMap;
use turbo_tasks::Vc;

#[turbo_tasks::value]
pub struct Item {
    pub shallow_list: Vc<Vec<Vc<Item>>>,
    pub cond_loaded_modules: Vc<IndexSet<Vc<Item>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Enter,
    Leave,
}

pub async fn find_list(entrypoints: Vc<Vec<Vc<Item>>>) -> Result<Vc<Vec<Vc<Item>>>> {
    // Create an empty set X for items that have been processed.
    let mut done = IndexSet::new();
    let mut queue = VecDeque::<(_, ItemKind)>::new();
    queue.extend(entrypoints.await?.iter().map(|vc| (*vc, ItemKind::Enter)));

    // Create an empty list L.
    let mut list = vec![];

    // Create an empty set S.
    let mut set = IndexSet::default();

    while let Some((item, kind)) = queue.pop_front() {
        // Add item to X.
        if !done.insert(item) {
            continue;
        }

        match kind {
            ItemKind::Enter => {
                // Put all items from the shallow list of the dequeued item at the front of the
                // queue Q with type ENTER
                for shallow_item in item.await?.shallow_list.await?.iter() {
                    queue.push_front((*shallow_item, ItemKind::Enter));
                }

                // Put all items from conditional loaded modules into set S.
                for cond_loaded_module in item.await?.cond_loaded_modules.await?.iter() {
                    set.insert(*cond_loaded_module);
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
        for &item in &list {
            set.remove(&item);
        }

        // Set the item’s shallow list to L and the conditionally loaded modules
        // to S.
        let item = Item {
            shallow_list: Vc::cell(take(&mut list)),
            cond_loaded_modules: Vc::cell(take(&mut set)),
        };

        // Enqueue all items from S for processing
        queue.extend(
            item.cond_loaded_modules
                .await?
                .iter()
                .map(|vc| (*vc, ItemKind::Enter)),
        );
    }

    // Create a reverse mapping from item → (list, index)[] (sorted by
    // smallest index first)
    //
    // So we can cheaply determine in which lists an item is and at which
    // index.
    macro_rules! reverse_map {
        ($list:expr) => {{
            let mut reverse_mapping = FxHashMap::default();
            for (i, list) in $list.iter().enumerate() {
                for (j, item) in list.await?.iter().enumerate() {
                    reverse_mapping
                        .entry(*item)
                        .or_insert_with(Vec::new)
                        .push((i, j));
                }
            }

            reverse_mapping
        }};
    }

    // Gather all lists from X.
    let mut lists = vec![];
    for &item in done.iter() {
        lists.push(item.await?.shallow_list.clone());
    }

    let mut reverse_mapping = reverse_map!(lists);

    // Remove all mappings that only point to a single list-index tuple.
    //
    // They are already done and we don’t want to sort them.*
    reverse_mapping.retain(|_item, list_indices| list_indices.len() != 1);

    // Sort the mapping by smallest index
    //
    // This is important to be able to find the longest common slice in the
    // following*
    for (_, list_indices) in reverse_mapping.iter_mut() {
        list_indices.sort_by_key(|(list, index)| (*list, *index));
    }

    // For each item in the mappings:

    for (item, list_indices) in reverse_mapping {
        // We need to split the item into a separate list. We want the list
        // to be as long as possible.

        // Create a new list L.
        let mut list = vec![];

        // TODO
        // Walk all lists at the same item as long as the items in these
        // lists are common (starting by current index):

        let mut current_index = 0;
        {
            // Put item into L
            list.push(item);

            // Remove item from the mapping.
            reverse_mapping.remove(&item);
        }

        // Put L into the set of all lists.
        lists.push(Vc::cell(list));

        // TODO
        // Put all remaining items in the lists (starting by current index)
        // into new lists and update mappings correctly. (No need to
        // updating sorting, Items might not have a mapping)
    }

    // At this point every item is exactly in one list.

    Ok(Vc::cell(list))
}
