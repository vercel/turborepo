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
