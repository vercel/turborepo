use anyhow::Result;
use async_recursion::async_recursion;
use rustc_hash::{FxHashMap, FxHashSet};
use turbo_tasks::{debug::ValueDebugFormat, vdbg, Vc};
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
}

async fn hashed(item: Item) -> Result<String> {
    item.value_debug_format(10).try_to_string().await
}

impl Workspace {
    #[async_recursion]
    async fn start_scope(&mut self, entry: Vc<Box<dyn Module>>) -> Result<()> {
        let entry = entry.resolve_strongly_consistent().await?;

        let modules = self.walk(entry, entry).await?;
        if modules.is_empty() {
            return Ok(());
        }

        vdbg!(entry);

        let module_scope = ModuleScope {
            modules: Vc::cell(modules),
        }
        .cell();

        self.scopes.push(module_scope);

        Ok(())
    }

    #[async_recursion]
    async fn walk(&mut self, from: Item, start: Item) -> Result<Vec<Item>> {
        let from = from.resolve_strongly_consistent().await?;
        if !self.done.insert(from) {
            return Ok(vec![]);
        }

        let start = start.resolve_strongly_consistent().await?;
        let deps = self.dep_graph.deps(from);
        let mut modules = vec![from];

        for &dep in deps.await?.iter() {
            let dep = dep.resolve_strongly_consistent().await?;
            let dependants = self.dep_graph.depandants(dep);

            let dependants = {
                let mut buf = vec![];
                for dep in dependants.await?.iter() {
                    buf.push(dep.resolve_strongly_consistent().await?);
                }
                buf
            };

            let should_start_scope = if dependants.len() == 1 {
                self.dep_graph.get_edge(from, dep).await?.is_lazy
            } else {
                // If all dependants start from the same scope, we can merge them
                let mut all = true;
                for &dep in dependants.iter() {
                    if !*self.dep_graph.has_path_connecting(dep, start).await? {
                        all = false;
                        break;
                    }
                }
                !all
            };

            if should_start_scope {
                self.start_scope(dep).await?;
            } else {
                let v = self.walk(dep, start).await?;

                for vc in v.iter() {
                    let vc = vc.resolve_strongly_consistent().await?;
                    modules.push(vc);
                }
            }
        }

        Ok(modules)
    }
}

#[turbo_tasks::function]
pub async fn split_scopes(
    entry: Vc<Box<dyn Module>>,
    dep_graph: Vc<Box<dyn DepGraph>>,
) -> Result<Vc<ModuleScopeGroup>> {
    // If a module is imported only as lazy, it should be in a separate scope

    let mut workspace = Workspace {
        dep_graph,
        scopes: Default::default(),
        done: Default::default(),
    };

    workspace.start_scope(entry).await?;

    Ok(ModuleScopeGroup {
        scopes: Vc::cell(workspace.scopes),
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
