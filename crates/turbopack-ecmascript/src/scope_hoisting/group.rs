use std::collections::VecDeque;

use rustc_hash::FxHashSet;

/// Counterpart of `Chunk` in webpack scope hoisting
pub struct ModuleScopeGroup {
    pub scopes: Vec<ModuleScope>,
}

/// Counterpart of `Scope` in webpack scope hoisting
pub struct ModuleScope {
    /// The modules in this scope.
    pub modules: Vec<Item>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Item(pub usize);

pub fn split_scopes(dep_graph: &dyn DepGraph, entry: Item) -> ModuleScopeGroup {
    // If a module is imported only as lazy, it should be in a separate scope

    let entries = determine_entries(dep_graph, entry);
    dbg!(&entries);

    let mut scopes = vec![];

    let mut done = FxHashSet::default();
    for &entry in entries.iter() {
        let modules = follow_single_edge(dep_graph, entry, &done)
            .into_iter()
            .collect::<Vec<_>>();

        done.extend(modules.iter().copied());

        scopes.push(ModuleScope { modules });
    }

    ModuleScopeGroup { scopes }
}

pub trait DepGraph {
    fn deps(&self, module: Item) -> Vec<Item>;

    fn depandants(&self, module: Item) -> Vec<Item>;

    fn get_edge(&self, from: Item, to: Item) -> EdgeData;

    fn has_path_connecting(&self, from: Item, to: Item) -> bool;
}

#[derive(Debug)]
pub struct EdgeData {
    pub is_lazy: bool,
}

fn determine_entries(dep_graph: &dyn DepGraph, entry: Item) -> FxHashSet<Item> {
    let mut entries = FxHashSet::default();

    let mut done = FxHashSet::<Item>::default();
    let mut queue = VecDeque::default();
    queue.push_back((entry, true));

    while let Some((cur, is_entry)) = queue.pop_front() {
        if is_entry && !entries.insert(cur) {
            continue;
        }

        if is_entry {
            let group = follow_single_edge(dep_graph, cur, &done);
            done.extend(group.iter().copied());
        }

        let deps = dep_graph.deps(cur);

        for dep in deps {
            // If lazy, it should be in a separate scope.
            if dep_graph.get_edge(cur, dep).is_lazy {
                queue.push_back((dep, true));
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

fn follow_single_edge(
    dep_graph: &dyn DepGraph,
    entry: Item,
    exclude: &FxHashSet<Item>,
) -> FxHashSet<Item> {
    let mut done = FxHashSet::default();

    let mut queue = VecDeque::<Item>::default();
    queue.push_back(entry);

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
