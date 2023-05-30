use std::rc::Rc;

use anyhow::Result;
use turbopath::AbsoluteSystemPathBuf;

use crate::{package_json::PackageJson, run::workspace_catalog::WorkspaceCatalog};

pub struct PackageGraph {
    pub workspace_graph: Rc<petgraph::Graph<String, String>>,
    pub workspace_infos: Rc<WorkspaceCatalog>,
}

impl PackageGraph {
    pub fn build_single_package_graph(_root_package_json: PackageJson) -> Result<PackageGraph> {
        // TODO
        Ok(PackageGraph {
            workspace_graph: Rc::new(petgraph::Graph::new()),
            workspace_infos: Rc::new(WorkspaceCatalog::default()),
        })
    }

    pub fn build_multi_package_graph(
        _repo_root: &AbsoluteSystemPathBuf,
        _root_package_json: &PackageJson,
    ) -> Result<PackageGraph> {
        // TODO
        Ok(PackageGraph {
            workspace_graph: Rc::new(petgraph::Graph::new()),
            workspace_infos: Rc::new(WorkspaceCatalog::default()),
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
