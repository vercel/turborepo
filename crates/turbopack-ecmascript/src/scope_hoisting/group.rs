use std::collections::VecDeque;

use anyhow::Result;
use async_recursion::async_recursion;
use indexmap::IndexSet;
use rustc_hash::FxHashSet;
use turbo_tasks::{vdbg, Vc};
use turbopack_core::module::Module;

/// Counterpart of `Chunk` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleScopeGroup {
    pub scopes: Vc<Vec<Vc<ModuleScope>>>,
}

/// Counterpart of `Scope` in webpack scope hoisting
#[turbo_tasks::value]
pub struct ModuleScope {
    /// The modules in this scope.
    pub modules: Vc<Vec<Vc<Box<dyn Module>>>>,
}

type Item = Vc<Box<dyn Module>>;

#[turbo_tasks::function]
pub async fn split_scopes(
    dep_graph: Vc<Box<dyn DepGraph>>,
    entry: Vc<Box<dyn Module>>,
) -> Result<Vc<ModuleScopeGroup>> {
    // If a module is imported only as lazy, it should be in a separate scope

    let entries = determine_entries(dep_graph, entry).await?;

    let mut scopes = vec![];

    for &entry in entries.iter() {
        let modules = follow_single_edge(dep_graph, entry)
            .await?
            .into_iter()
            .collect::<Vec<_>>();

        scopes.push(
            ModuleScope {
                modules: Vc::cell(modules),
            }
            .cell(),
        );
    }

    Ok(ModuleScopeGroup {
        scopes: Vc::cell(scopes),
    }
    .cell())
}

#[turbo_tasks::value_trait]
pub trait DepGraph {
    fn deps(&self, module: Vc<Box<dyn Module>>) -> Vc<Vec<Vc<Box<dyn Module>>>>;

    fn depandants(&self, module: Vc<Box<dyn Module>>) -> Vc<Vec<Vc<Box<dyn Module>>>>;

    fn get_edge(&self, from: Vc<Box<dyn Module>>, to: Vc<Box<dyn Module>>) -> Vc<EdgeData>;

    fn has_path_connecting(&self, from: Vc<Box<dyn Module>>, to: Vc<Box<dyn Module>>) -> Vc<bool>;
}

#[turbo_tasks::value(shared)]
pub struct EdgeData {
    pub is_lazy: bool,
}

async fn determine_entries(
    dep_graph: Vc<Box<dyn DepGraph>>,
    entry: Item,
) -> Result<FxHashSet<Item>> {
    let mut entries = FxHashSet::default();

    let mut done = FxHashSet::<Item>::default();
    let mut queue = VecDeque::<Item>::default();
    queue.push_back(entry);

    while let Some(c) = queue.pop_front() {
        let cur = c.resolve_strongly_consistent().await?;

        vdbg!(cur);
        if !entries.insert(cur) {
            continue;
        }

        let group = follow_single_edge(dep_graph, cur).await?;
        for &group_item in group.iter() {
            vdbg!(group_item);
        }
        done.extend(group.iter().copied());

        let deps = dep_graph.deps(cur).await?;

        for &dep in deps {
            vdbg!(dep);

            // If lazy, it should be in a separate scope.
            if dep_graph.get_edge(cur, dep).await?.is_lazy {
                queue.push_back(dep);
                continue;
            }

            let dependants = dep_graph.depandants(dep).await?;
            let mut filtered = vec![];
            for &dependant in dependants.iter() {
                let dependant = dependant.resolve_strongly_consistent().await?;
                if group.contains(&dependant) {
                    continue;
                }
                filtered.push(dependant);
            }

            // If there are multiple dependants, it's an entry.
            if filtered.len() > 1 {
                queue.push_back(dep);
            }
        }
    }

    Ok(entries)
}

async fn follow_single_edge(
    dep_graph: Vc<Box<dyn DepGraph>>,
    entry: Item,
) -> Result<FxHashSet<Item>> {
    let mut done = FxHashSet::default();

    let mut queue = VecDeque::<Item>::default();
    queue.push_back(entry);

    while let Some(c) = queue.pop_front() {
        let cur = c.resolve_strongly_consistent().await?;

        if !done.insert(cur) {
            continue;
        }

        let deps = dep_graph.deps(cur).await?;

        if deps.is_empty() {
            break;
        }

        for &dep in deps {
            if dep_graph.get_edge(cur, dep).await?.is_lazy {
                continue;
            }

            // If there are multiple dependeants, ignore.
            let dependants = dep_graph.depandants(dep).await?;

            if dependants.len() > 1 {
                continue;
            }

            queue.push_back(dep);
        }
    }

    Ok(done)
}
