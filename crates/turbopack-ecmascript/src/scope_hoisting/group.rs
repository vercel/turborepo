use anyhow::Result;
use async_recursion::async_recursion;
use indexmap::IndexSet;
use rustc_hash::FxHashSet;
use turbo_tasks::Vc;
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
        is_entry: bool,
    ) -> Result<()> {
        let entry = entry.resolve_strongly_consistent().await?;

        if is_entry {
            self.entries.insert(entry);
        }

        let deps = self.dep_graph.deps(entry);

        for &dep in deps.await?.iter() {
            let dep = dep.resolve_strongly_consistent().await?;
            let dependants = self.dep_graph.depandants(dep);

            if !self.done.insert(dep) {
                return Ok(());
            }

            // Exclude lazy dependency.
            let mut count = 0;

            for dependant in dependants.await?.iter() {
                let dependant = dependant.resolve_strongly_consistent().await?;
                if self.done.contains(&dependant)
                    || self.dep_graph.get_edge(dependant, dep).await?.is_lazy
                {
                    continue;
                }

                count += 1;
            }

            if count > 1 {
                self.fill_entries_multi(dep, true).await?;
            } else {
                self.fill_entries_multi(dep, false).await?;
            }
        }

        Ok(())
    }

    #[async_recursion]
    async fn start_scope(&mut self, entry: Vc<Box<dyn Module>>) -> Result<()> {
        let entry = entry.resolve_strongly_consistent().await?;

        let modules = self.walk(entry, true).await?;
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
    async fn walk(&mut self, from: Item, is_start: bool) -> Result<Vec<Item>> {
        let from = from.resolve_strongly_consistent().await?;
        if !is_start && !self.done.insert(from) {
            return Ok(vec![]);
        }

        let deps = self.dep_graph.deps(from);
        let mut modules = vec![from];

        for &dep in deps.await?.iter() {
            let dep = dep.resolve_strongly_consistent().await?;

            // Collect dependencies in the same scope.
            let v = self.walk(dep, false).await?;

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

    let mut w = Workspace {
        dep_graph,
        scopes: Default::default(),
        done: Default::default(),
        entries: Default::default(),
    };

    w.fill_entries_lazy(entry, true).await?;
    w.done.clear();

    w.done.extend(w.entries.iter().copied());
    w.fill_entries_multi(entry, true).await?;
    w.done.clear();

    let entries = w.entries.clone();

    w.done.extend(entries.iter().copied());

    for &entry in entries.iter() {
        w.start_scope(entry).await?;
    }

    Ok(ModuleScopeGroup {
        scopes: Vc::cell(w.scopes),
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
