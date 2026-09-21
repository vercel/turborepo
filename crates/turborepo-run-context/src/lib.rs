//! Shared context describing the repository for a Turborepo run.

use std::sync::Arc;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_microfrontends_config::UnifiedTurboJsonLoader;
use turborepo_repository::package_graph::PackageGraph;
use turborepo_scm::SCM;
use turborepo_turbo_json::TurboJson;
use turborepo_ui::ColorConfig;

/// Repository-scoped data shared by all phases of a run.
#[derive(Clone)]
pub struct RepoContext {
    repo_root: AbsoluteSystemPathBuf,
    color_config: ColorConfig,
    version: &'static str,
    scm: SCM,
    pkg_dep_graph: Arc<PackageGraph>,
    turbo_json_loader: UnifiedTurboJsonLoader,
    root_turbo_json: TurboJson,
}

impl RepoContext {
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        color_config: ColorConfig,
        version: &'static str,
        scm: SCM,
        pkg_dep_graph: Arc<PackageGraph>,
        turbo_json_loader: UnifiedTurboJsonLoader,
        root_turbo_json: TurboJson,
    ) -> Self {
        Self {
            repo_root,
            color_config,
            version,
            scm,
            pkg_dep_graph,
            turbo_json_loader,
            root_turbo_json,
        }
    }

    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        &self.repo_root
    }

    pub fn color_config(&self) -> ColorConfig {
        self.color_config
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn scm(&self) -> &SCM {
        &self.scm
    }

    pub fn pkg_dep_graph(&self) -> &PackageGraph {
        &self.pkg_dep_graph
    }

    pub fn pkg_dep_graph_handle(&self) -> Arc<PackageGraph> {
        self.pkg_dep_graph.clone()
    }

    pub fn turbo_json_loader(&self) -> &UnifiedTurboJsonLoader {
        &self.turbo_json_loader
    }

    pub fn root_turbo_json(&self) -> &TurboJson {
        &self.root_turbo_json
    }
}
