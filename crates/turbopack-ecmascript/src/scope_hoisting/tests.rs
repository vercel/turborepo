use super::group::DepGraph;

#[turbo_tasks::value]
pub struct TestDepGraph {}

#[turbo_tasks::value_impl]
impl DepGraph for TestDepGraph {}
