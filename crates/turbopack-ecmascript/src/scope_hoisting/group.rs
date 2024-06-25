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

struct Workspace {
    dep_graph: Vc<Box<dyn DepGraph>>,

    scopes: Vec<Vc<ModuleScope>>,
    done: FxHashSet<Item>,
    entries: IndexSet<Item>,
}

impl Workspace {
    #[async_recursion]
    async fn fill_entries_lazy(
        &mut self,
        entry: Vc<Box<dyn Module>>,
        is_entry: bool,
    ) -> Result<()> {
        let entry = entry.resolve_strongly_consistent().await?;
        if !self.done.insert(entry) {
            return Ok(());
        }
        if is_entry {
            self.entries.insert(entry);
        }

        let deps = self.dep_graph.deps(entry);

        for &dep in deps.await?.iter() {
            let dep = dep.resolve_strongly_consistent().await?;

            if self.dep_graph.get_edge(entry, dep).await?.is_lazy {
                self.fill_entries_lazy(dep, true).await?;
            } else {
                self.fill_entries_lazy(dep, false).await?;
            }
        }

        Ok(())
    }

    #[async_recursion]
    async fn fill_entries_multi(
        &mut self,
        entry: Vc<Box<dyn Module>>,
        is_init: bool,
        is_entry: bool,
    ) -> Result<()> {
        let entry = entry.resolve_strongly_consistent().await?;

        if !is_init && !self.done.insert(entry) {
            return Ok(());
        }

        if is_entry {
            self.entries.insert(entry);
        }

        let deps = self.dep_graph.deps(entry);

        for &dep in deps.await?.iter() {
            let dep = dep.resolve_strongly_consistent().await?;

            let dependants = self.dep_graph.depandants(dep);

            let mut count = 0;

            for dependant in dependants.await?.iter() {
                let dependant = dependant.resolve_strongly_consistent().await?;
                if self.done.contains(&dependant) {
                    continue;
                }

                count += 1;
            }

            if count > 1 {
                self.fill_entries_multi(dep, false, true).await?;
            } else {
                self.fill_entries_multi(dep, false, false).await?;
            }
        }

        Ok(())
    }

    #[async_recursion]
    async fn start_scope(&mut self, entry: Vc<Box<dyn Module>>) -> Result<()> {
        let entry = entry.resolve_strongly_consistent().await?;

        let modules = self.collect(entry, true).await?;
        if modules.is_empty() {
            return Ok(());
        }

        let module_scope = ModuleScope {
            modules: Vc::cell(modules),
        }
        .cell();

        self.scopes.push(module_scope);

        Ok(())
    }

    #[async_recursion]
    async fn collect(&mut self, from: Item, is_start: bool) -> Result<Vec<Item>> {
        let from = from.resolve_strongly_consistent().await?;
        if !is_start && !self.done.insert(from) {
            return Ok(vec![]);
        }

        let deps = self.dep_graph.deps(from);
        let mut modules = vec![from];

        for &dep in deps.await?.iter() {
            let dep = dep.resolve_strongly_consistent().await?;

            // Collect dependencies in the same scope.
            let v = self.collect(dep, false).await?;

            for vc in v.iter() {
                let vc = vc.resolve_strongly_consistent().await?;
                modules.push(vc);
            }
        }

        Ok(modules)
    }
}

#[turbo_tasks::function]
pub async fn split_scopes(
    dep_graph: Vc<Box<dyn DepGraph>>,
    entry: Vc<Box<dyn Module>>,
) -> Result<Vc<ModuleScopeGroup>> {
    // If a module is imported only as lazy, it should be in a separate scope

    let entries = determine_entries(dep_graph.clone(), entry.clone()).await?;

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

    let mut done = FxHashSet::default();
    let mut queue = VecDeque::<Item>::new();
    queue.push_back(entry);

    while let Some(c) = queue.pop_front() {
        let cur = c.resolve_strongly_consistent().await?;

        if !entries.insert(cur) {
            continue;
        }

        let deps = dep_graph.deps(cur).await?;

        for &dep in deps {
            // If lazy, it should be in a separate scope.
            if dep_graph.get_edge(cur, dep).await?.is_lazy {
                entries.insert(dep);
                continue;
            }

            done.extend(follow_single_edge(dep_graph, dep).await?);

            let dependants = dep_graph.depandants(dep).await?;
            let mut filtered = vec![];
            for &dependant in dependants.iter() {
                if done.contains(&dependant) {
                    continue;
                }
                filtered.push(dependant);
            }

            // If there are multiple dependants, it's an entry.
            if filtered.len() > 1 {
                entries.insert(dep);
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

    let mut queue = VecDeque::<Item>::new();
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
