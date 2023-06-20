use std::rc::Rc;

use anyhow::Result;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_lockfiles::{Lockfile, NpmLockfile};

use crate::{package_json::PackageJson, package_manager::PackageManager};

#[derive(Default)]
pub struct WorkspaceCatalog {}

pub struct PackageGraph {
    pub workspace_graph: Rc<petgraph::Graph<String, String>>,
    pub workspace_infos: Rc<WorkspaceCatalog>,
    pub package_manager: PackageManager,
    pub lockfile: Box<dyn Lockfile>,
}

impl PackageGraph {
    pub fn build_single_package_graph(_root_package_json: &PackageJson) -> Result<PackageGraph> {
        // TODO
        Ok(PackageGraph {
            workspace_graph: Rc::new(petgraph::Graph::new()),
            workspace_infos: Rc::new(WorkspaceCatalog::default()),
            package_manager: PackageManager::Npm,
            lockfile: Box::<NpmLockfile>::default(),
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
            package_manager: PackageManager::Npm,
            lockfile: Box::<NpmLockfile>::default(),
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
