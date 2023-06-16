use std::collections::{HashMap, HashSet};

use anyhow::Result;
use petgraph::graph::{Graph, NodeIndex};
use turbopath::AbsoluteSystemPath;
use turborepo_lockfiles::Lockfile;

use super::{Dependency, Entry, PackageGraph, WorkspaceName, WorkspaceNode};
use crate::{package_json::PackageJson, package_manager::PackageManager};

pub struct PackageGraphBuilder<'a> {
    repo_root: &'a AbsoluteSystemPath,
    root_package_json: PackageJson,
    single: bool,
    package_manager: Option<PackageManager>,
    package_jsons: Option<HashMap<WorkspaceName, PackageJson>>,
    lockfile: Option<Box<dyn Lockfile>>,
}

impl<'a> PackageGraphBuilder<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath, root_package_json: PackageJson) -> Self {
        Self {
            repo_root,
            root_package_json,
            single: false,
            package_manager: None,
            package_jsons: None,
            lockfile: None,
        }
    }

    pub fn with_single_package_mode(mut self, is_single: bool) -> Self {
        self.single = is_single;
        self
    }

    pub fn with_package_manger(mut self, package_manager: Option<PackageManager>) -> Self {
        self.package_manager = package_manager;
        self
    }

    pub fn with_package_jsons(
        mut self,
        package_jsons: Option<HashMap<WorkspaceName, PackageJson>>,
    ) -> Self {
        self.package_jsons = package_jsons;
        self
    }

    pub fn with_lockfile(mut self, lockfile: Option<Box<dyn Lockfile>>) -> Self {
        self.lockfile = lockfile;
        self
    }

    pub fn build(self) -> Result<PackageGraph> {
        let single = self.single;
        let state = BuildState::new(self)?;
        match single {
            true => Ok(state.build_single_package_graph()),
            false => {
                let state = state.parse_package_jsons();
                let state = state.resolve_lockfile();
                Ok(state.build())
            }
        }
    }
}

struct BuildState<'a, S> {
    repo_root: &'a AbsoluteSystemPath,
    single: bool,
    package_manager: PackageManager,
    workspaces: HashMap<WorkspaceName, Entry>,
    workspace_graph: Graph<WorkspaceNode, Dependency>,
    node_lookup: HashMap<WorkspaceNode, NodeIndex>,
    lockfile: Option<Box<dyn Lockfile>>,
    state: std::marker::PhantomData<S>,
}

// Allows us to perform workspace discovery and parse package jsons
enum ResolvedPackageManager {}

// Allows us to build the workspace graph and list over external dependencies
enum ResolvedWorkspaces {}

// Allows us to collect all transitive deps
enum ResolvedLockfile {}

impl<'a, S> BuildState<'a, S> {
    fn add_node(&mut self, node: WorkspaceNode) -> NodeIndex {
        let idx = self.workspace_graph.add_node(node.clone());
        self.node_lookup.insert(node, idx);
        idx
    }

    fn add_root_workspace(&mut self) {
        let root_index = self.add_node(WorkspaceNode::Root);
        let root_workspace = self.add_node(WorkspaceNode::Workspace(WorkspaceName::Root));
        self.workspace_graph
            .add_edge(root_workspace, root_index, Dependency::Root);
    }
}

impl<'a> BuildState<'a, ResolvedPackageManager> {
    fn new(
        builder: PackageGraphBuilder<'a>,
    ) -> Result<BuildState<'a, ResolvedPackageManager>, crate::package_manager::Error> {
        let PackageGraphBuilder {
            repo_root,
            root_package_json,
            single,
            package_manager,
            package_jsons,
            lockfile,
        } = builder;
        let package_manager = package_manager.map_or_else(
            || PackageManager::get_package_manager(repo_root, Some(&root_package_json)),
            Result::Ok,
        )?;
        let mut workspaces = HashMap::new();
        for (name, json) in package_jsons.into_iter().flatten() {
            workspaces.insert(
                name,
                Entry {
                    json,
                    ..Default::default()
                },
            );
        }
        workspaces.insert(
            WorkspaceName::Root,
            Entry {
                json: root_package_json,
                ..Default::default()
            },
        );

        Ok(BuildState {
            repo_root,
            single,
            package_manager,
            workspaces,
            lockfile,
            workspace_graph: Graph::new(),
            node_lookup: HashMap::new(),
            state: std::marker::PhantomData,
        })
    }

    fn parse_package_jsons(self) -> BuildState<'a, ResolvedWorkspaces> {
        // TODO actually parse the package.json
        let Self {
            repo_root,
            single,
            package_manager,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            ..
        } = self;
        BuildState {
            repo_root,
            single,
            package_manager,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            state: std::marker::PhantomData,
        }
    }

    fn build_single_package_graph(mut self) -> PackageGraph {
        self.add_root_workspace();
        let Self {
            single,
            package_manager,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            ..
        } = self;
        debug_assert!(single, "expected single package graph");
        PackageGraph {
            workspace_graph,
            node_lookup,
            workspaces,
            package_manager,
            lockfile,
        }
    }
}

impl<'a> BuildState<'a, ResolvedWorkspaces> {
    fn resolve_lockfile(self) -> BuildState<'a, ResolvedLockfile> {
        // TODO actually parse lockfile
        let Self {
            repo_root,
            single,
            package_manager,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            ..
        } = self;
        BuildState {
            repo_root,
            single,
            package_manager,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            state: std::marker::PhantomData,
        }
    }
}

impl<'a> BuildState<'a, ResolvedLockfile> {
    fn build(self) -> PackageGraph {
        PackageGraph {
            workspace_graph: Graph::new(),
            node_lookup: HashMap::new(),
            workspaces: HashMap::new(),
            package_manager: PackageManager::Npm,
            lockfile: None,
        }
    }
}
