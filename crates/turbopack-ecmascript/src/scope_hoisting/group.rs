use anyhow::Result;
use async_recursion::async_recursion;
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

struct Workspace {
    dep_graph: Vc<Box<dyn DepGraph>>,

    scopes: Vec<Vc<ModuleScope>>,
    // TODO: Vc does not work here
    done: FxHashSet<Vc<Box<dyn Module>>>,
}

impl Workspace {
    #[async_recursion]
    async fn start_scope(&mut self, entry: Vc<Box<dyn Module>>) -> Result<()> {
        let entry = entry.resolve().await?;
        if !self.done.insert(entry) {
            return Ok(());
        }

        let modules = self.walk(entry).await?;

        let module_scope = ModuleScope {
            modules: Vc::cell(modules),
        }
        .cell();

        self.scopes.push(module_scope);

        Ok(())
    }

    #[async_recursion]
    async fn walk(&mut self, from: Vc<Box<dyn Module>>) -> Result<Vec<Vc<Box<dyn Module>>>> {
        let deps = self.dep_graph.deps(from);
        let mut modules = vec![from];

        for &dep in deps.await?.iter() {
            let dependants = self.dep_graph.depandants(dep);

            let should_start_scope = if dependants.await?.len() == 1 {
                if self.dep_graph.get_edge(from, dep).await?.is_lazy {
                    true
                } else {
                    false
                }
            } else {
                // TODO: If all dependants start from the same scope, we can merge them
                true
            };

            if should_start_scope {
                self.start_scope(dep).await?;
            } else {
                modules.extend(self.walk(dep).await?);
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
}

#[turbo_tasks::value(shared)]
pub struct EdgeData {
    pub is_lazy: bool,
}
