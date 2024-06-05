use std::collections::HashMap;

use anyhow::Result;
use turbo_tasks::Vc;
use turbo_tasks_fs::{DiskFileSystem, FileContent, FileSystem, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    ident::AssetIdent,
    module::Module,
};

use super::group::{DepGraph, EdgeData};

#[test]
fn test_1() {}

fn test_dep_graph(deps: Vec<(usize, Vec<usize>)>) -> Vc<TestDepGraph> {
    let fs = DiskFileSystem::new("test".to_owned(), "test".to_owned(), Default::default());

    let mut dependants = HashMap::new();

    for (from, to) in &deps {
        for &to in to {
            dependants.entry(to).or_insert_with(Vec::new).push(*from);
        }
    }

    TestDepGraph {
        fs,
        deps: deps.into_iter().collect(),
        dependants,
    }
    .cell()
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
        Ok(Vc::cell(None))
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
