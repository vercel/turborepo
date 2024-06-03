use anyhow::Result;
use rustc_hash::{FxHashMap, FxHashSet};
use turbo_tasks::Vc;
use turbopack_core::{
    chunk::{ModuleId, ModuleIds},
    ident::AssetIdent,
    module::Module,
};

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
}

impl Workspace {
    async fn start_scope(&mut self, entry: Vc<Box<dyn Module>>) -> Result<()> {
        let deps = self.dep_graph.deps(entry);
        let mut modules = vec![entry];

        for &dep in deps.await?.iter() {
            let dependants = self.dep_graph.depandants(dep);

            if dependants.await?.len() == 1 {
                modules.push(dep);
            } else {
                self.start_scope(dep).await?;
            }
        }

        let module_scope = ModuleScope {
            modules: Vc::cell(modules),
        }
        .cell();

        self.scopes.push(module_scope);

        Ok(())
    }
}

#[turbo_tasks::function]
pub async fn split_scopes(
    entry: Vc<Box<dyn Module>>,
    dep_graph: Vc<Box<dyn DepGraph>>,
) -> Result<Vc<Vec<Vc<ModuleScopeGroup>>>> {
    // If a module is imported only as lazy, it should be in a separate scope

    let mut workspace = Workspace {
        dep_graph,
        scopes: Default::default(),
    };

    workspace.start_scope(entry).await?;

    todo!()
}

#[turbo_tasks::value_trait]
pub trait DepGraph {
    fn deps(&self, id: Vc<Box<dyn Module>>) -> Vc<Vec<Vc<Box<dyn Module>>>>;

    fn depandants(&self, id: Vc<Box<dyn Module>>) -> Vc<Vec<Vc<Box<dyn Module>>>>;

    fn get_edge(&self, from: Vc<Box<dyn Module>>, to: Vc<Box<dyn Module>>) -> Vc<Option<EdgeData>>;
}

#[turbo_tasks::value]
pub struct EdgeData {
    pub is_lazy: bool,
}
