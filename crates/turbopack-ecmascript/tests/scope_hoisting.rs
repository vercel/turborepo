#![feature(arbitrary_self_types)]
use std::hash::{BuildHasherDefault, Hash};

use anyhow::Result;
use indexmap::IndexSet;
use petgraph::{algo::has_path_connecting, graphmap::DiGraphMap};
use rustc_hash::FxHasher;
use turbo_tasks::{TurboTasks, Vc};
use turbo_tasks_fs::{DiskFileSystem, FileContent, FileSystem, FileSystemPath};
use turbo_tasks_memory::MemoryBackend;
use turbopack_core::{
    asset::{Asset, AssetContent},
    ident::AssetIdent,
    module::Module,
};
use turbopack_ecmascript::scope_hoisting::group::{split_scopes, DepGraph, EdgeData};

fn register() {
    turbo_tasks::register();
    turbo_tasks_fs::register();
    turbopack_ecmascript::register();

    include!(concat!(env!("OUT_DIR"), "/register_test_scope_hoisting.rs"));
}

#[tokio::test]
async fn test_1() -> Result<()> {
    let result = split(to_num_deps(vec![
        ("example", vec![("a", false), ("b", false), ("lazy", true)]),
        ("lazy", vec![("c", false), ("d", false)]),
        ("a", vec![("shared", false)]),
        ("c", vec![("shared", false), ("cjs", false)]),
        ("shared", vec![("shared2", false)]),
    ]))
    .await?;

    assert_eq!(result, vec![vec![6, 8], vec![3, 4, 7, 5], vec![0, 1, 2]]);

    Ok(())
}

#[tokio::test]
async fn test_2() -> Result<()> {
    // a => b
    // a => c
    // b => d
    // c => d
    let result = split(to_num_deps(vec![
        ("example", vec![("a", false), ("b", false), ("lazy", true)]),
        ("lazy", vec![("shared", false)]),
        ("a", vec![("shared", false), ("b", false), ("c", false)]),
        ("b", vec![("shared", false), ("d", false)]),
        ("c", vec![("shared", false), ("d", false)]),
        ("d", vec![("shared", false)]),
        ("shared", vec![("shared2", false)]),
    ]))
    .await?;

    assert_eq!(result, vec![vec![0, 1, 2, 5, 6], vec![3], vec![4, 7]]);

    Ok(())
}

fn to_num_deps(deps: Vec<(&str, Vec<(&str, bool)>)>) -> Deps {
    let mut map = IndexSet::new();

    for (from, to) in deps.iter() {
        if map.insert(*from) {
            eprintln!("Inserted {from} as {}", map.get_full(from).unwrap().0);
        }

        for (to, _) in to {
            if map.insert(to) {
                eprintln!("Inserted {to} as {}", map.get_full(to).unwrap().0);
            }
        }
    }

    deps.into_iter()
        .map(|(from, to)| {
            (
                map.get_full(from).unwrap().0,
                to.into_iter()
                    .map(|(to, is_lazy)| (map.get_full(to).unwrap().0, is_lazy))
                    .collect(),
            )
        })
        .collect()
}

type Deps = Vec<(usize, Vec<(usize, bool)>)>;

async fn split(deps: Deps) -> Result<Vec<Vec<usize>>> {
    register();

    let tt = TurboTasks::new(MemoryBackend::default());
    tt.run_once(async move {
        let fs = DiskFileSystem::new("test".into(), "test".into(), Default::default());

        let graph = test_dep_graph(fs, deps);

        let group = split_scopes(graph, Vc::upcast(TestModule::new_from(fs, 0)));

        let group = group.await?;

        let mut data = vec![];

        for scope in group.scopes.await?.iter() {
            let mut scope_data = vec![];

            for &module in scope.await?.modules.await?.iter() {
                let module = from_module(module).await?;
                scope_data.push(module);
            }

            data.push(scope_data);
        }

        Ok(data)
    })
    .await
}

fn test_dep_graph(fs: Vc<DiskFileSystem>, deps: Deps) -> Vc<Box<dyn DepGraph>> {
    let mut g = InternedGraph::default();

    for (from, to) in deps {
        let from = g.node(&from);

        for (to, is_lazy) in to {
            let to = g.node(&to);

            g.idx_graph.add_edge(from, to, TestEdgeData { is_lazy });
        }
    }

    Vc::upcast(TestDepGraph { fs, graph: g }.cell())
}

#[turbo_tasks::value(serialization = "none")]
pub struct TestDepGraph {
    fs: Vc<DiskFileSystem>,
    #[turbo_tasks(trace_ignore)]
    graph: InternedGraph<usize>,
}

async fn from_module(module: Vc<Box<dyn Module>>) -> Result<usize> {
    let module: Vc<TestModule> = Vc::try_resolve_downcast_type(module).await?.unwrap();
    let path = module.await?.path.await?;
    path.to_string()
        .split('/')
        .last()
        .unwrap()
        .parse()
        .map_err(Into::into)
}

#[turbo_tasks::value_impl]
impl DepGraph for TestDepGraph {
    #[turbo_tasks::function]
    async fn deps(&self, module: Vc<Box<dyn Module>>) -> Result<Vc<Vec<Vc<Box<dyn Module>>>>> {
        let from = self.graph.get_node(&from_module(module).await?);

        let dependencies = self
            .graph
            .idx_graph
            .neighbors_directed(from, petgraph::Direction::Outgoing)
            .map(|id| Vc::upcast(TestModule::new_from(self.fs, id as usize)))
            .collect();

        Ok(Vc::cell(dependencies))
    }

    #[turbo_tasks::function]
    async fn depandants(
        &self,
        module: Vc<Box<dyn Module>>,
    ) -> Result<Vc<Vec<Vc<Box<dyn Module>>>>> {
        let from = self.graph.get_node(&from_module(module).await?);

        let dependants = self
            .graph
            .idx_graph
            .neighbors_directed(from, petgraph::Direction::Incoming)
            .map(|id| Vc::upcast(TestModule::new_from(self.fs, id as usize)))
            .collect();

        Ok(Vc::cell(dependants))
    }

    #[turbo_tasks::function]
    async fn get_edge(
        &self,
        from: Vc<Box<dyn Module>>,
        to: Vc<Box<dyn Module>>,
    ) -> Result<Vc<EdgeData>> {
        let from = self.graph.get_node(&from_module(from).await?);
        let to = self.graph.get_node(&from_module(to).await?);

        let edge_data = self.graph.idx_graph.edge_weight(from, to).unwrap();

        let is_lazy = edge_data.is_lazy;

        Ok(EdgeData { is_lazy }.cell())
    }

    #[turbo_tasks::function]
    async fn has_path_connecting(
        &self,
        from: Vc<Box<dyn Module>>,
        to: Vc<Box<dyn Module>>,
    ) -> Result<Vc<bool>> {
        let from = self.graph.get_node(&from_module(from).await?);
        let to = self.graph.get_node(&from_module(to).await?);

        Ok(Vc::cell(has_path_connecting(
            &self.graph.idx_graph,
            from,
            to,
            None,
        )))
    }
}

#[turbo_tasks::value(shared)]
struct TestModule {
    path: Vc<FileSystemPath>,
}

#[turbo_tasks::value_impl]
impl TestModule {
    #[turbo_tasks::function]
    fn new(path: Vc<FileSystemPath>) -> Vc<Self> {
        Self { path }.cell()
    }

    #[turbo_tasks::function]
    fn new_from(fs: Vc<DiskFileSystem>, id: usize) -> Vc<Self> {
        Self {
            path: fs.root().join(format!("{}", id).into()),
        }
        .cell()
    }
}

#[turbo_tasks::value_impl]
impl Asset for TestModule {
    #[turbo_tasks::function]
    fn content(self: Vc<Self>) -> Vc<AssetContent> {
        AssetContent::File(FileContent::NotFound.cell()).cell()
    }
}

#[turbo_tasks::value_impl]
impl Module for TestModule {
    #[turbo_tasks::function]
    fn ident(&self) -> Vc<AssetIdent> {
        AssetIdent::from_path(self.path)
    }
}

#[derive(Debug)]
struct TestEdgeData {
    is_lazy: bool,
}

impl<T> PartialEq for InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl<T> Eq for InternedGraph<T> where T: Eq + Hash + Clone {}

#[derive(Debug)]
pub struct InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    idx_graph: DiGraphMap<u32, TestEdgeData>,
    graph_ix: IndexSet<T, BuildHasherDefault<FxHasher>>,
}

impl<T> Default for InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self {
            idx_graph: Default::default(),
            graph_ix: Default::default(),
        }
    }
}

impl<T> InternedGraph<T>
where
    T: Eq + Hash + Clone,
{
    fn node(&mut self, id: &T) -> u32 {
        self.graph_ix.get_index_of(id).unwrap_or_else(|| {
            let ix = self.graph_ix.len();
            self.graph_ix.insert_full(id.clone());
            ix
        }) as _
    }

    /// Panics if `id` is not found.
    fn get_node(&self, id: &T) -> u32 {
        self.graph_ix.get_index_of(id).unwrap() as _
    }
}
