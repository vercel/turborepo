#![feature(arbitrary_self_types)]

use std::collections::HashMap;

use anyhow::Result;
use indexmap::IndexSet;
use rustc_hash::FxHashMap;
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

fn to_num_deps(deps: Vec<(&str, Vec<&str>)>) -> Deps {
    let mut map = IndexSet::new();

    for (from, to) in deps.iter() {
        map.insert(*from);

        for &to in to {
            map.insert(to);
        }
    }

    deps.into_iter()
        .map(|(from, to)| {
            (
                map.get_full(from).unwrap().0,
                to.into_iter()
                    .map(|to| map.get_full(to).unwrap().0)
                    .collect(),
            )
        })
        .collect()
}

#[tokio::test]
async fn test_1() -> Result<()> {
    let result = split(to_num_deps(vec![
        ("example", vec!["a", "b", "lazy"]),
        ("lazy", vec!["c", "d"]),
        ("a", vec!["shared"]),
        ("c", vec!["shared", "cjs"]),
        ("shared", vec!["shared2"]),
    ]))
    .await?;

    assert_eq!(result, vec![vec![3], vec![1, 2], vec![0]]);

    Ok(())
}

type Deps = Vec<(usize, Vec<usize>)>;

async fn split(deps: Deps) -> Result<Vec<Vec<usize>>> {
    register();

    let tt = TurboTasks::new(MemoryBackend::default());
    tt.run_once(async move {
        let fs = DiskFileSystem::new("test".to_owned(), "test".to_owned(), Default::default());

        let graph = test_dep_graph(fs, deps);

        let group = split_scopes(to_module(fs, 0), graph);

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
    let mut dependants = HashMap::new();

    for (from, to) in &deps {
        for &to in to {
            dependants.entry(to).or_insert_with(Vec::new).push(*from);
        }
    }

    Vc::upcast(
        TestDepGraph {
            fs,
            deps: deps.into_iter().collect(),
            dependants,
        }
        .cell(),
    )
}

#[turbo_tasks::value]
pub struct TestDepGraph {
    fs: Vc<DiskFileSystem>,
    deps: HashMap<usize, Vec<usize>>,
    dependants: HashMap<usize, Vec<usize>>,
}

fn to_module(fs: Vc<DiskFileSystem>, id: usize) -> Vc<Box<dyn Module>> {
    let vc = TestModule {
        path: fs.root().join(format!("{}", id)),
    }
    .cell();

    Vc::upcast(vc)
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
        let module = from_module(module).await?;

        Ok(Vc::cell(
            self.deps
                .get(&module)
                .map(|deps| {
                    deps.iter()
                        .map(|&id| Vc::upcast(to_module(self.fs, id)))
                        .collect()
                })
                .unwrap_or_default(),
        ))
    }

    #[turbo_tasks::function]
    async fn depandants(
        &self,
        module: Vc<Box<dyn Module>>,
    ) -> Result<Vc<Vec<Vc<Box<dyn Module>>>>> {
        let module = from_module(module).await?;

        Ok(Vc::cell(
            self.dependants
                .get(&module)
                .map(|deps| {
                    deps.iter()
                        .map(|&id| Vc::upcast(to_module(self.fs, id)))
                        .collect()
                })
                .unwrap_or_default(),
        ))
    }

    #[turbo_tasks::function]
    async fn get_edge(
        &self,
        from: Vc<Box<dyn Module>>,
        to: Vc<Box<dyn Module>>,
    ) -> Result<Vc<Option<EdgeData>>> {
        todo!()
    }
}

#[turbo_tasks::value]
struct TestModule {
    path: Vc<FileSystemPath>,
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
