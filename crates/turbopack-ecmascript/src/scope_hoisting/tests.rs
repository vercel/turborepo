use turbo_tasks::Vc;
use turbo_tasks_fs::{FileContent, FileSystemPath};
use turbopack_core::{
    asset::{Asset, AssetContent},
    ident::AssetIdent,
    module::Module,
};

use super::group::DepGraph;

#[turbo_tasks::value]
pub struct TestDepGraph {
    deps: Vec<()>,
}

#[turbo_tasks::value_impl]
impl DepGraph for TestDepGraph {
    fn deps(&self, id: Vc<Box<dyn Module>>) -> Vc<Vec<Vc<Box<dyn Module>>>> {}

    fn depandants(&self, id: Vc<Box<dyn Module>>) -> Vc<Vec<Vc<Box<dyn Module>>>> {}

    fn get_edge(&self, from: Vc<Box<dyn Module>>, to: Vc<Box<dyn Module>>) -> Vc<Option<EdgeData>> {
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
