use std::collections::{BTreeMap, HashMap, HashSet};

use miette::{Diagnostic, Report};
use petgraph::graph::{Graph, NodeIndex};
use tracing::{Instrument, warn};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_graph_utils as graph;
use turborepo_lockfiles::Lockfile;

use super::{
    PackageGraph, PackageInfo, PackageName, PackageNode,
    dep_splitter::{DependencySplitter, WorkspacePathIndex},
};
use crate::{
    discovery::{
        self, CachingPackageDiscovery, LocalPackageDiscoveryBuilder, PackageDiscovery,
        PackageDiscoveryBuilder,
    },
    package_json::PackageJson,
    package_manager::PackageManager,
};

pub struct PackageGraphBuilder<'a, T> {
    repo_root: &'a AbsoluteSystemPath,
    root_package_json: PackageJson,
    is_single_package: bool,
    package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    lockfile: Option<Box<dyn Lockfile>>,
    package_discovery: T,
    package_manager: Option<PackageManager>,
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error("Could not resolve workspaces.")]
    #[diagnostic(transparent)]
    PackageManager(#[from] crate::package_manager::Error),
    #[error(
        "Failed to add workspace \"{name}\" from \"{path}\", it already exists at \
         \"{existing_path}\""
    )]
    DuplicateWorkspace {
        name: String,
        path: String,
        existing_path: String,
    },
    #[error("Path error: {0}")]
    Path(#[from] turbopath::PathError),
    #[diagnostic(transparent)]
    #[error(transparent)]
    PackageJson(#[from] crate::package_json::Error),
    #[error("package.json must have a name field:\n{0}")]
    PackageJsonMissingName(AbsoluteSystemPathBuf),
    #[error("Invalid package dependency graph:")]
    InvalidPackageGraph(#[source] graph::Error),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error(transparent)]
    Discovery(#[from] crate::discovery::Error),
}

/// Attempts to extract the file path that caused the error from the error chain
/// Falls back to the lockfile path if no specific file can be determined
fn extract_file_path_from_error(
    error: &Error,
    package_manager: &crate::package_manager::PackageManager,
    repo_root: &AbsoluteSystemPath,
) -> AbsoluteSystemPathBuf {
    match error {
        Error::PackageJsonMissingName(path) => path.clone(),
        // TODO: We're handling every other error here. We could handle situations where the
        // lockfile isn't the issue better.
        _ => package_manager.lockfile_path(repo_root),
    }
}

impl<'a> PackageGraphBuilder<'a, LocalPackageDiscoveryBuilder> {
    pub fn new(repo_root: &'a AbsoluteSystemPath, root_package_json: PackageJson) -> Self {
        Self {
            package_discovery: LocalPackageDiscoveryBuilder::new(
                repo_root.to_owned(),
                None,
                Some(root_package_json.clone()),
            ),
            repo_root,
            root_package_json,
            is_single_package: false,
            package_jsons: None,
            lockfile: None,
            package_manager: None,
        }
    }

    pub fn with_allow_no_package_manager(mut self, allow_no_package_manager: bool) -> Self {
        self.package_discovery
            .with_allow_no_package_manager(allow_no_package_manager);
        self
    }

    pub fn with_package_manager(mut self, package_manager: PackageManager) -> Self {
        self.package_manager = Some(package_manager.clone());
        self.package_discovery
            .with_package_manager(Some(package_manager));
        self
    }
}

impl<'a, P> PackageGraphBuilder<'a, P> {
    pub fn with_single_package_mode(mut self, is_single: bool) -> Self {
        self.is_single_package = is_single;
        self
    }

    pub fn with_package_jsons(
        mut self,
        package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    ) -> Self {
        self.package_jsons = package_jsons;
        self
    }

    pub fn with_lockfile(mut self, lockfile: Option<Box<dyn Lockfile>>) -> Self {
        self.lockfile = lockfile;
        self
    }

    /// Set the package discovery strategy to use. Note that whatever strategy
    /// selected here will be wrapped in a `CachingPackageDiscovery` to
    /// prevent unnecessary work during building.
    pub fn with_package_discovery<P2: PackageDiscoveryBuilder>(
        self,
        discovery: P2,
    ) -> PackageGraphBuilder<'a, P2> {
        PackageGraphBuilder {
            repo_root: self.repo_root,
            root_package_json: self.root_package_json,
            is_single_package: self.is_single_package,
            package_jsons: self.package_jsons,
            lockfile: self.lockfile,
            package_discovery: discovery,
            package_manager: self.package_manager,
        }
    }
}

impl<T> PackageGraphBuilder<'_, T>
where
    T: PackageDiscoveryBuilder,
    T::Output: Send + Sync,
    T::Error: Into<crate::package_manager::Error>,
{
    /// Build the `PackageGraph`.
    #[tracing::instrument(skip(self))]
    pub async fn build(mut self) -> Result<PackageGraph, Error> {
        let is_single_package = self.is_single_package;

        // If no pre-supplied lockfile, start reading it on a blocking thread
        // concurrently with package discovery + JSON parsing.
        let known_pm = self.package_manager.take().or_else(|| {
            PackageManager::get_package_manager(self.repo_root, &self.root_package_json).ok()
        });
        let lockfile_future = if !is_single_package && self.lockfile.is_none() {
            if let Some(pm) = known_pm {
                let repo_root = self.repo_root.to_owned();
                let root_package_json = self.root_package_json.clone();
                Some(tokio::task::spawn_blocking(
                    move || -> Option<Box<dyn Lockfile>> {
                        pm.read_lockfile(&repo_root, &root_package_json).ok()
                    },
                ))
            } else {
                None
            }
        } else {
            None
        };

        let state = BuildState::new(self)?;

        match is_single_package {
            true => Ok(state.build_single_package_graph().await?),
            false => {
                let state = state.parse_package_jsons().await?;

                // If we started a lockfile read, collect the result before
                // entering resolve_lockfile so it becomes a cache hit.
                let state = if let Some(handle) = lockfile_future {
                    if let Ok(Some(lockfile)) = handle.await {
                        state.with_lockfile(lockfile)
                    } else {
                        state
                    }
                } else {
                    state
                };

                let state = state.resolve_lockfile().await?;
                Ok(state.build_inner().await?)
            }
        }
    }
}

struct BuildState<'a, S, T> {
    repo_root: &'a AbsoluteSystemPath,
    single: bool,
    workspaces: HashMap<PackageName, PackageInfo>,
    workspace_graph: Graph<PackageNode, ()>,
    node_lookup: HashMap<PackageNode, NodeIndex>,
    lockfile: Option<Box<dyn Lockfile>>,
    package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    state: std::marker::PhantomData<S>,
    package_discovery: T,
}

// Allows us to perform workspace discovery and parse package jsons
enum ResolvedPackageManager {}

// Allows us to build the workspace graph and list over external dependencies
enum ResolvedWorkspaces {}

// Allows us to collect all transitive deps
enum ResolvedLockfile {}

impl<S, T> BuildState<'_, S, T> {
    fn add_node(&mut self, node: PackageNode) -> NodeIndex {
        let idx = self.workspace_graph.add_node(node.clone());
        self.node_lookup.insert(node, idx);
        idx
    }

    fn add_root_workspace(&mut self) {
        let root_index = self.add_node(PackageNode::Root);
        let root_workspace = self.add_node(PackageNode::Workspace(PackageName::Root));
        self.workspace_graph
            .add_edge(root_workspace, root_index, ());
    }
}

impl<'a, T> BuildState<'a, ResolvedPackageManager, T>
where
    T: PackageDiscoveryBuilder,
    T::Output: Send,
    T::Error: Into<crate::package_manager::Error>,
{
    fn new(
        builder: PackageGraphBuilder<'a, T>,
    ) -> Result<
        BuildState<'a, ResolvedPackageManager, CachingPackageDiscovery<T::Output>>,
        crate::package_manager::Error,
    > {
        let PackageGraphBuilder {
            repo_root,
            root_package_json,
            is_single_package: single,

            package_jsons,
            lockfile,
            package_discovery,
            package_manager: _,
        } = builder;
        let mut workspaces = HashMap::new();
        workspaces.insert(
            PackageName::Root,
            PackageInfo {
                package_json: root_package_json,
                package_json_path: AnchoredSystemPathBuf::from_raw("package.json").unwrap(),
                ..Default::default()
            },
        );

        Ok(BuildState {
            repo_root,
            single,

            workspaces,
            lockfile,
            package_jsons,
            workspace_graph: Graph::new(),
            node_lookup: HashMap::new(),
            state: std::marker::PhantomData,
            package_discovery: CachingPackageDiscovery::new(
                package_discovery.build().map_err(Into::into)?,
            ),
        })
    }
}

impl<'a, T: PackageDiscovery> BuildState<'a, ResolvedPackageManager, T> {
    fn add_json(
        &mut self,
        package_json_path: AbsoluteSystemPathBuf,
        json: PackageJson,
    ) -> Result<(), Error> {
        let relative_json_path =
            AnchoredSystemPathBuf::relative_path_between(self.repo_root, &package_json_path);
        let name = PackageName::Other(
            json.name
                .clone()
                .ok_or(Error::PackageJsonMissingName(package_json_path))?
                .into_inner(),
        );
        let entry = PackageInfo {
            package_json: json,
            package_json_path: relative_json_path,
            ..Default::default()
        };
        match self.workspaces.entry(name) {
            std::collections::hash_map::Entry::Vacant(vacant) => {
                let name = vacant.key().clone();
                vacant.insert(entry);
                self.add_node(PackageNode::Workspace(name));
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(occupied) => {
                let existing_path = occupied.get().package_json_path.to_string();
                let name = occupied.key().to_string();
                Err(Error::DuplicateWorkspace {
                    name,
                    path: entry.package_json_path.to_string(),
                    existing_path,
                })
            }
        }
    }

    // need our own type
    #[tracing::instrument(skip(self))]
    async fn parse_package_jsons(mut self) -> Result<BuildState<'a, ResolvedWorkspaces, T>, Error> {
        // The root workspace will be present
        // we either read from disk or just read the map
        self.add_root_workspace();

        let package_jsons = match self.package_jsons.take() {
            Some(jsons) => Ok(jsons),
            None => {
                let workspace_paths: Vec<_> =
                    self.package_discovery.discover_packages().await?.workspaces;

                let results: Vec<_> = {
                    use rayon::prelude::*;
                    workspace_paths
                        .into_par_iter()
                        .map(|path| {
                            let json = PackageJson::load(&path.package_json)?;
                            Ok((path.package_json, json))
                        })
                        .collect::<Result<Vec<_>, Error>>()?
                };

                let mut jsons = HashMap::with_capacity(results.len());
                for (path, json) in results {
                    jsons.insert(path, json);
                }
                Ok::<_, Error>(jsons)
            }
        }?;

        self.workspaces.reserve(package_jsons.len());
        self.node_lookup.reserve(package_jsons.len());

        for (path, json) in package_jsons {
            match self.add_json(path, json) {
                Ok(()) => {}
                Err(Error::PackageJsonMissingName(path)) => {
                    // previous implementations of turbo would silently ignore package.json files
                    // that didn't have a name field (well, actually, if two or more had the same
                    // name, it would throw a 'name clash' error, but that's a different story)
                    //
                    // let's try to match that behavior, but log a debug message
                    tracing::debug!("ignoring package.json at {} since it has no name", path);
                }
                Err(err) => return Err(err),
            }
        }

        let Self {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            package_discovery,
            ..
        } = self;
        Ok(BuildState {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            package_discovery,
            package_jsons: None,
            state: std::marker::PhantomData,
        })
    }

    async fn build_single_package_graph(mut self) -> Result<PackageGraph, discovery::Error> {
        self.add_root_workspace();
        let Self {
            single,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            package_discovery,
            repo_root,
            ..
        } = self;

        let package_manager = package_discovery.discover_packages().await?.package_manager;

        debug_assert!(single, "expected single package graph");
        Ok(PackageGraph {
            graph: workspace_graph,
            node_lookup,
            packages: workspaces,
            lockfile,
            package_manager,
            repo_root: repo_root.to_owned(),
            external_dep_to_internal_dependents: std::sync::OnceLock::new(),
        })
    }
}

impl<'a, T: PackageDiscovery> BuildState<'a, ResolvedWorkspaces, T> {
    fn with_lockfile(mut self, lockfile: Box<dyn Lockfile>) -> Self {
        self.lockfile = Some(lockfile);
        self
    }

    #[tracing::instrument(skip(self))]
    fn connect_internal_dependencies(
        &mut self,
        package_manager: &PackageManager,
    ) -> Result<(), Error> {
        let path_index = WorkspacePathIndex::new(&self.workspaces);
        // Compute once — for pnpm/Berry this reads a config file from disk.
        // Without hoisting, the par_iter below would redundantly read the
        // same file N times (once per workspace).
        let link_workspace_packages = package_manager.link_workspace_packages(self.repo_root);
        // Resolve internal vs external dependencies in parallel. Each
        // Dependencies::new call is read-only on the workspaces map
        // so this is safe. Graph mutation stays sequential below.
        let split_deps = {
            use rayon::prelude::*;
            self.workspaces
                .par_iter()
                .map(|(name, entry)| {
                    (
                        name.clone(),
                        Dependencies::new(
                            self.repo_root,
                            &entry.package_json_path,
                            &self.workspaces,
                            link_workspace_packages,
                            entry.package_json.all_dependencies(),
                            &path_index,
                        ),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (name, deps) in split_deps {
            let entry = self
                .workspaces
                .get_mut(&name)
                .expect("workspace present in ");
            let Dependencies { internal, external } = deps;
            let node_idx = self
                .node_lookup
                .get(&PackageNode::Workspace(name))
                .expect("unable to find workspace node index");
            if internal.is_empty() {
                let root_idx = self
                    .node_lookup
                    .get(&PackageNode::Root)
                    .expect("root node should have index");
                self.workspace_graph.add_edge(*node_idx, *root_idx, ());
            }
            for dependency in internal {
                let dependency_idx = self
                    .node_lookup
                    .get(&PackageNode::Workspace(dependency))
                    .expect("unable to find workspace node index");
                self.workspace_graph
                    .add_edge(*node_idx, *dependency_idx, ());
            }
            entry.unresolved_external_dependencies = Some(external);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn populate_lockfile(&mut self) -> Result<Box<dyn Lockfile>, Error> {
        let package_manager = self
            .package_discovery
            .discover_packages()
            .await?
            .package_manager;

        match self.lockfile.take() {
            Some(lockfile) => Ok(lockfile),
            None => {
                let lockfile = package_manager.read_lockfile(
                    self.repo_root,
                    self.workspaces
                        .get(&PackageName::Root)
                        .as_ref()
                        .map(|e| &e.package_json)
                        .expect("root workspace should have json"),
                )?;
                Ok(lockfile)
            }
        }
    }

    #[tracing::instrument(skip(self))]
    async fn resolve_lockfile(mut self) -> Result<BuildState<'a, ResolvedLockfile, T>, Error> {
        // Since we've already performed package discovery, this should just be a cache
        // hit
        let package_manager = self
            .package_discovery
            .discover_packages()
            .await?
            .package_manager;
        self.connect_internal_dependencies(&package_manager)?;

        let lockfile = match self.populate_lockfile().await {
            Ok(lockfile) => Some(lockfile),
            Err(e) => {
                let problematic_file_path =
                    extract_file_path_from_error(&e, &package_manager, self.repo_root);

                warn!(
                    "An issue occurred while attempting to parse {}. Turborepo will still \
                     function, but some features may not be available:\n {:?}",
                    problematic_file_path,
                    Report::new(e)
                );
                None
            }
        };

        let Self {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            node_lookup,
            package_discovery,
            ..
        } = self;
        Ok(BuildState {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            package_jsons: None,
            state: std::marker::PhantomData,
            package_discovery,
        })
    }
}

impl<T: PackageDiscovery> BuildState<'_, ResolvedLockfile, T> {
    fn all_external_dependencies(&self) -> Result<HashMap<String, HashMap<String, String>>, Error> {
        self.workspaces
            .values()
            .map(|entry| {
                let workspace_path = entry
                    .package_json_path
                    .parent()
                    .unwrap_or(AnchoredSystemPath::new("")?)
                    .to_unix();
                let workspace_string = workspace_path.as_str();
                let external_deps = entry
                    .unresolved_external_dependencies
                    .as_ref()
                    .map(|deps| {
                        deps.iter()
                            .map(|(name, version)| (name.to_string(), version.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok((workspace_string.to_string(), external_deps))
            })
            .collect()
    }

    #[tracing::instrument(skip_all)]
    fn populate_transitive_dependencies(&mut self) -> Result<(), Error> {
        let Some(lockfile) = self.lockfile.as_deref() else {
            return Ok(());
        };

        // We cannot ignore missing packages in this context, it would indicate a
        // malformed or stale lockfile.
        let mut closures = turborepo_lockfiles::all_transitive_closures(
            lockfile,
            self.all_external_dependencies()?,
            false,
        )?;
        for (_, entry) in self.workspaces.iter_mut() {
            entry.transitive_dependencies = closures.remove(&entry.unix_dir_str()?);
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn build_inner(mut self) -> Result<PackageGraph, discovery::Error> {
        if let Err(e) = self.populate_transitive_dependencies() {
            warn!("Unable to calculate transitive closures: {}", e);
        }
        let package_manager = self
            .package_discovery
            .discover_packages()
            .instrument(tracing::debug_span!("package discovery"))
            .await?
            .package_manager;
        let Self {
            workspaces,
            workspace_graph,
            node_lookup,
            lockfile,
            repo_root,
            ..
        } = self;
        Ok(PackageGraph {
            graph: workspace_graph,
            node_lookup,
            packages: workspaces,
            package_manager,
            lockfile,
            repo_root: repo_root.to_owned(),
            external_dep_to_internal_dependents: std::sync::OnceLock::new(),
        })
    }
}

struct Dependencies {
    internal: HashSet<PackageName>,
    external: BTreeMap<String, String>, // Package name and version
}

impl Dependencies {
    pub fn new<'a, I: IntoIterator<Item = (&'a String, &'a String)>>(
        repo_root: &AbsoluteSystemPath,
        workspace_json_path: &AnchoredSystemPathBuf,
        workspaces: &HashMap<PackageName, PackageInfo>,
        link_workspace_packages: bool,
        dependencies: I,
        path_index: &WorkspacePathIndex<'_>,
    ) -> Self {
        let resolved_workspace_json_path = repo_root.resolve(workspace_json_path);
        let workspace_dir = resolved_workspace_json_path
            .parent()
            .expect("package.json path should have parent");
        let mut internal = HashSet::new();
        let mut external = BTreeMap::new();
        let splitter = DependencySplitter::new(
            repo_root,
            workspace_dir,
            workspaces,
            link_workspace_packages,
            path_index,
        );
        for (name, version) in dependencies.into_iter() {
            if let Some(workspace) = splitter.is_internal(name, version) {
                internal.insert(workspace);
            } else {
                external.insert(name.clone(), version.clone());
            }
        }
        Self { internal, external }
    }
}

impl PackageInfo {
    fn unix_dir_str(&self) -> Result<String, Error> {
        let unix = self
            .package_json_path
            .parent()
            .unwrap_or_else(|| AnchoredSystemPath::new("").expect("empty path is anchored"))
            .to_unix();
        Ok(unix.to_string())
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, collections::HashMap};

    use turborepo_errors::Spanned;

    use super::*;

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            Ok(crate::discovery::DiscoveryResponse {
                package_manager: crate::package_manager::PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            self.discover_packages().await
        }
    }

    // Regression test: connect_internal_dependencies must produce correct
    // graph edges and external deps regardless of iteration order or
    // parallelism. This captures the exact edges and
    // unresolved_external_dependencies so any refactor of the collection phase
    // (e.g. rayon parallelization) is safe.
    #[tokio::test]
    async fn test_connect_internal_dependencies_produces_correct_edges() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let mut package_jsons = HashMap::new();
        // "web" depends on "ui" (workspace:*) and "react" (external)
        package_jsons.insert(
            root.join_components(&["apps", "web", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("web".into())),
                version: Some("1.0.0".to_string()),
                dependencies: Some(
                    [
                        ("ui".to_string(), "workspace:*".to_string()),
                        ("react".to_string(), "^18.0.0".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
        );
        // "api" depends on "utils" (workspace:*) and "express" (external)
        package_jsons.insert(
            root.join_components(&["apps", "api", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("api".into())),
                version: Some("1.0.0".to_string()),
                dependencies: Some(
                    [
                        ("utils".to_string(), "workspace:*".to_string()),
                        ("express".to_string(), "^4.0.0".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
        );
        // "ui" has no workspace deps, only "csstype" (external)
        package_jsons.insert(
            root.join_components(&["packages", "ui", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("ui".into())),
                version: Some("1.0.0".to_string()),
                dependencies: Some(
                    [("csstype".to_string(), "^3.0.0".to_string())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        );
        // "utils" has no deps at all
        package_jsons.insert(
            root.join_components(&["packages", "utils", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("utils".into())),
                version: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(package_jsons))
        .build()
        .await
        .unwrap();

        // Verify internal dependency edges via the package graph API
        let web_name = PackageName::from("web");
        let api_name = PackageName::from("api");
        let ui_name = PackageName::from("ui");
        let utils_name = PackageName::from("utils");

        // web -> ui (internal)
        let web_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(web_name.clone()))
            .unwrap();
        assert!(
            web_deps.contains(&PackageNode::Workspace(ui_name.clone())),
            "web should depend on ui, got: {:?}",
            web_deps
        );

        // api -> utils (internal)
        let api_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(api_name.clone()))
            .unwrap();
        assert!(
            api_deps.contains(&PackageNode::Workspace(utils_name.clone())),
            "api should depend on utils, got: {:?}",
            api_deps
        );

        // ui has no internal deps -> should connect to root
        let ui_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(ui_name.clone()))
            .unwrap();
        assert!(
            ui_deps.contains(&PackageNode::Root),
            "ui should depend on root (no internal deps), got: {:?}",
            ui_deps
        );

        // utils has no internal deps -> should connect to root
        let utils_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(utils_name.clone()))
            .unwrap();
        assert!(
            utils_deps.contains(&PackageNode::Root),
            "utils should depend on root (no internal deps), got: {:?}",
            utils_deps
        );

        // Verify external deps are recorded correctly
        let web_info = graph.package_info(&web_name).unwrap();
        let web_ext = web_info.unresolved_external_dependencies.as_ref().unwrap();
        assert_eq!(web_ext.get("react").map(|v| v.as_str()), Some("^18.0.0"));
        assert!(
            !web_ext.contains_key("ui"),
            "ui should be internal, not external"
        );

        let api_info = graph.package_info(&api_name).unwrap();
        let api_ext = api_info.unresolved_external_dependencies.as_ref().unwrap();
        assert_eq!(api_ext.get("express").map(|v| v.as_str()), Some("^4.0.0"));
        assert!(
            !api_ext.contains_key("utils"),
            "utils should be internal, not external"
        );

        let ui_info = graph.package_info(&ui_name).unwrap();
        let ui_ext = ui_info.unresolved_external_dependencies.as_ref().unwrap();
        assert_eq!(ui_ext.get("csstype").map(|v| v.as_str()), Some("^3.0.0"));

        let utils_info = graph.package_info(&utils_name).unwrap();
        let utils_ext = utils_info
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();
        assert!(
            utils_ext.is_empty(),
            "utils should have no external deps, got: {:?}",
            utils_ext
        );
    }

    #[tokio::test]
    async fn test_duplicate_package_names() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let builder = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("a"),
                PackageJson {
                    name: Some(Spanned::new("foo".into())),
                    ..Default::default()
                },
            );
            map.insert(
                root.join_component("b"),
                PackageJson {
                    name: Some(Spanned::new("foo".into())),
                    ..Default::default()
                },
            );
            map
        }));
        assert_matches!(builder.build().await, Err(Error::DuplicateWorkspace { .. }));
    }

    #[test]
    #[cfg(unix)]
    fn test_missing_name_field_warning_message() {
        let package_json_path =
            AbsoluteSystemPathBuf::new("/my-project/packages/app/package.json").unwrap();
        let missing_name_error = Error::PackageJsonMissingName(package_json_path.clone());

        let fake_repo_root = AbsoluteSystemPathBuf::new("/my-project").unwrap();
        let fake_package_manager = crate::package_manager::PackageManager::Npm;
        let extracted_path = extract_file_path_from_error(
            &missing_name_error,
            &fake_package_manager,
            &fake_repo_root,
        );
        assert_eq!(extracted_path, package_json_path);

        let warning_message = format!(
            "An issue occurred while attempting to parse {}. Turborepo will still function, but \
             some features may not be available:\n {:?}",
            package_json_path,
            miette::Report::new(missing_name_error)
        );

        insta::assert_snapshot!("missing_name_field_warning_message", warning_message);
    }
}
