use anyhow::Result;
use turbopath::AbsoluteSystemPathBuf;

use crate::{package_json::PackageJson, run::graph::WorkspaceCatalog};

pub struct Context {
    pub workspace_graph: petgraph::Graph<String, String>,
    pub workspace_infos: WorkspaceCatalog,
}

impl Context {
    pub fn build_single_package_graph(_root_package_json: PackageJson) -> Result<Context> {
        // TODO
        Ok(Context {
            workspace_graph: petgraph::Graph::new(),
            workspace_infos: WorkspaceCatalog::default(),
        })
    }

    pub fn build_multi_package_graph(
        _repo_root: &AbsoluteSystemPathBuf,
        _root_package_json: &PackageJson,
    ) -> Result<Context> {
        // TODO
        Ok(Context {
            workspace_graph: petgraph::Graph::new(),
            workspace_infos: WorkspaceCatalog::default(),
        })
    }

    pub fn validate(&self) -> Result<()> {
        // TODO
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.workspace_graph.node_count()
    }
}
