use std::collections::VecDeque;

use indexmap::{IndexMap, IndexSet};
use rustc_hash::FxHashSet;

/// Counterpart of `Chunk` in webpack scope hoisting
pub struct ModuleScopeGroup {
    pub scopes: Vec<ModuleScope>,
}

/// Counterpart of `Scope` in webpack scope hoisting
pub struct ModuleScope {
    /// The modules in this scope.
    pub modules: IndexSet<Item>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Item(pub usize);

pub fn split_scopes(dep_graph: &mut dyn DepGraph, entry: Item) -> ModuleScopeGroup {
    // If a module is imported only as lazy, it should be in a separate scope

    let entries = determine_entries(dep_graph, entry);

    let items = entries
        .iter()
        .map(|entry| {
            follow_single_edge(dep_graph, entry, &entries)
                .into_iter()
                .collect()
        })
        .collect::<Vec<_>>();

    let initial = entries.into_iter().zip(items).collect();

    let entries = merge_entries(dep_graph, initial);

    let mut scopes = vec![];

    for (_, modules) in entries {
        scopes.push(ModuleScope { modules });
    }

    ModuleScopeGroup { scopes }
}

pub trait DepGraph {
    fn deps(&self, module: Item) -> Vec<Item>;

    fn depandants(&self, module: Item) -> Vec<Item>;

    fn get_edge(&self, from: Item, to: Item) -> EdgeData;

    fn has_path_connecting(&self, from: Item, to: Item) -> bool;

    fn remove_edge(&mut self, from: Item, to: Item);
}

#[derive(Debug)]
pub struct EdgeData {
    pub is_lazy: bool,
}

fn determine_entries(dep_graph: &mut dyn DepGraph, entry: Item) -> IndexSet<Item> {
    let mut entries = IndexSet::default();

    let mut queue = VecDeque::default();
    queue.push_back((entry, true));

    while let Some((cur, is_entry)) = queue.pop_front() {
        if is_entry && !entries.insert(cur) {
            continue;
        }

        let deps = dep_graph.deps(cur);

        for dep in deps {
            // If lazy, it should be in a separate scope.
            if dep_graph.get_edge(cur, dep).is_lazy {
                queue.push_back((dep, true));
                dep_graph.remove_edge(cur, dep);
                continue;
            }

            let dependants = dep_graph.depandants(dep);

            // If there are multiple dependants, it's an entry.
            if dependants.len() >= 2 {
                // TODO: Optimization: If all of the dependants are from the same entrypoint, it
                // should be in the same scope.
                queue.push_back((dep, true));
                continue;
            }

            queue.push_back((dep, false))
        }
    }

    entries
}

fn merge_entries(
    dep_graph: &dyn DepGraph,
    entries: IndexMap<Item, IndexSet<Item>>,
) -> IndexMap<Item, IndexSet<Item>> {
    let mut new = IndexMap::<Item, IndexSet<Item>>::default();

    let keys = entries
        .keys()
        .filter(|entry| {
            let dependants = dep_graph.depandants(**entry);
            dependants.len() < 2
        })
        .cloned()
        .collect::<Vec<_>>();

    for (entry, items) in entries.into_iter() {
        // An entry is a target of check if there are two or more dependant nodes.

        let dependants = dep_graph.depandants(entry);
        if dependants.len() < 2 {
            new.entry(entry).or_default().extend(items);
            continue;
        }
        dbg!(&entry);
        dbg!(&keys);
        dbg!(&dependants);

        // If an entry is

        let real_start = keys.iter().find(|start| {
            dependants
                .iter()
                .all(|dep| dep_graph.has_path_connecting(**start, *dep))
        });

        dbg!(real_start);

        if let Some(real_start) = real_start {
            new.entry(*real_start).or_default().extend(items);
        } else {
            new.entry(entry).or_default().extend(items);
        }
    }

    new
}

fn follow_single_edge(
    dep_graph: &dyn DepGraph,
    entry: &Item,
    exclude: &IndexSet<Item>,
) -> FxHashSet<Item> {
    let mut done = FxHashSet::default();

    let mut queue = VecDeque::<Item>::default();
    queue.push_back(*entry);

    while let Some(cur) = queue.pop_front() {
        if !done.insert(cur) {
            continue;
        }

        let deps = dep_graph.deps(cur);

        for dep in deps {
            if exclude.contains(&dep) {
                continue;
            }

            queue.push_back(dep);
        }
    }

    done
}
