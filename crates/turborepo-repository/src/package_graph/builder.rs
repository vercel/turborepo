use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use miette::{Diagnostic, Report};
use petgraph::graph::{Graph, NodeIndex};
use tracing::{Instrument, warn};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
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
    package_json::{DependencyKind, PackageJson},
    package_manager::{PackageManager, pnpm::PnpmCatalogs},
    toolchain::{
        DiscoveredPackage, JavaScriptToolchain, Toolchain, ToolchainId, ToolchainRegistry,
    },
};

pub struct PackageGraphBuilder<'a, T> {
    repo_root: &'a AbsoluteSystemPath,
    /// The root `package.json`, when the repository has one. Absent for a
    /// pure Cargo workspace (`futureFlags.experimentalCargoWorkspaces` with
    /// no root `package.json`): there is no JavaScript project, so no
    /// package manager, lockfile, or root manifest to resolve.
    root_package_json: Option<PackageJson>,
    is_single_package: bool,
    package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    lockfile: Option<Box<dyn Lockfile>>,
    package_discovery: T,
    package_manager: Option<PackageManager>,
    defer_closures: bool,
    closure_hasher: Option<ClosureHasher>,
    /// Toolchains registered in addition to JavaScript (e.g. Cargo when
    /// `futureFlags.experimentalCargoWorkspaces` is enabled). Their packages
    /// are discovered alongside JavaScript packages; name collisions across
    /// toolchains are a hard error.
    extra_toolchains: Vec<Arc<dyn Toolchain>>,
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error("Could not resolve workspace.")]
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
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error(transparent)]
    Discovery(#[from] crate::discovery::Error),
    #[error(transparent)]
    Toolchain(Box<dyn std::error::Error + Send + Sync>),
}

// JavaScript toolchain errors map onto the pre-existing variants rather than
// new ones: consumers match on `Error::PackageJson` (diagnostic rendering,
// io-NotFound telemetry in the run builder), and those contracts must not
// depend on whether the error surfaced through a toolchain.
impl From<crate::toolchain::Error> for Error {
    fn from(err: crate::toolchain::Error) -> Self {
        match err {
            crate::toolchain::Error::Discovery(err) => Error::Discovery(err),
            crate::toolchain::Error::Descriptor(err) => Error::PackageJson(err),
            crate::toolchain::Error::Failed(err) => Error::Toolchain(err),
        }
    }
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
        Self::new_optional(repo_root, Some(root_package_json))
    }

    /// Build over a repository that may have no root `package.json`. When
    /// `root_package_json` is `None`, the JavaScript toolchain contributes
    /// nothing (no package manager, no lockfile); the graph is populated
    /// entirely by the extra toolchains registered via
    /// [`PackageGraphBuilder::with_toolchain`] (Cargo). When it is `Some`,
    /// this behaves exactly like [`PackageGraphBuilder::new`].
    pub fn new_optional(
        repo_root: &'a AbsoluteSystemPath,
        root_package_json: Option<PackageJson>,
    ) -> Self {
        Self {
            package_discovery: LocalPackageDiscoveryBuilder::new(
                repo_root.to_owned(),
                None,
                root_package_json.clone(),
            ),
            repo_root,
            root_package_json,
            is_single_package: false,
            package_jsons: None,
            lockfile: None,
            package_manager: None,
            defer_closures: false,
            closure_hasher: None,
            extra_toolchains: Vec::new(),
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

    /// Defer transitive-closure computation to a background thread. The
    /// resulting graph's `transitive_dependencies` are absent until
    /// [`PackageGraph::ensure_transitive_closures`] is called; callers that
    /// enable this own calling it before any closure consumer runs.
    pub fn defer_transitive_closures(mut self, defer: bool) -> Self {
        self.defer_closures = defer;
        self
    }

    /// Provide a function that hashes each workspace's sorted external
    /// dependency closure. When set, `PackageInfo::external_deps_hash` is
    /// populated wherever closures are computed (inline or deferred).
    /// Injected because the capnp-based hasher lives in `turborepo-hash`,
    /// which transitively depends on this crate.
    pub fn with_closure_hasher(mut self, hasher: ClosureHasher) -> Self {
        self.closure_hasher = Some(hasher);
        self
    }

    pub fn with_lockfile(mut self, lockfile: Option<Box<dyn Lockfile>>) -> Self {
        self.lockfile = lockfile;
        self
    }

    /// Register a toolchain in addition to JavaScript. Its packages are
    /// discovered alongside JavaScript packages; a package name collision
    /// across toolchains is a hard error, like any duplicate package name.
    pub fn with_toolchain(mut self, toolchain: Arc<dyn Toolchain>) -> Self {
        self.extra_toolchains.push(toolchain);
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
            defer_closures: self.defer_closures,
            closure_hasher: self.closure_hasher,
            extra_toolchains: self.extra_toolchains,
        }
    }
}

impl<T> PackageGraphBuilder<'_, T>
where
    T: PackageDiscoveryBuilder,
    T::Output: Send + Sync + 'static,
    T::Error: Into<crate::package_manager::Error>,
{
    /// Build the `PackageGraph`.
    #[tracing::instrument(skip(self))]
    pub async fn build(mut self) -> Result<PackageGraph, Error> {
        let is_single_package = self.is_single_package;

        // If no pre-supplied lockfile, start reading it on a blocking thread
        // concurrently with package discovery + JSON parsing. A pure Cargo
        // workspace has no root package.json and therefore no JavaScript
        // package manager or lockfile to read.
        let known_pm = self
            .package_manager
            .take()
            .or_else(|| {
                self.root_package_json
                    .as_ref()
                    .and_then(|root_package_json| {
                        PackageManager::get_package_manager(self.repo_root, root_package_json).ok()
                    })
            })
            .map(|pm| pm.with_resolved_nub_lockfile(self.repo_root));
        let lockfile_future = if !is_single_package && self.lockfile.is_none() {
            if let (Some(pm), Some(root_package_json)) = (known_pm, self.root_package_json.clone())
            {
                let repo_root = self.repo_root.to_owned();
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
                let state = state.resolve_lockfile(lockfile_future).await?;
                Ok(state.build_inner().await?)
            }
        }
    }
}

struct BuildState<'a, S, T> {
    repo_root: &'a AbsoluteSystemPath,
    single: bool,
    workspaces: HashMap<PackageName, PackageInfo>,
    workspace_graph: Graph<PackageNode, DependencyKind>,
    root_node_index: NodeIndex,
    root_workspace_index: NodeIndex,
    node_lookup: HashMap<PackageNode, NodeIndex>,
    /// The root `package.json`, absent for a pure Cargo workspace. See
    /// [`PackageGraphBuilder::root_package_json`].
    root_package_json: Option<PackageJson>,
    lockfile: Option<Box<dyn Lockfile>>,
    package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    defer_closures: bool,
    closure_hasher: Option<ClosureHasher>,
    state: std::marker::PhantomData<S>,
    /// The JavaScript toolchain, typed. Package-manager resolution for
    /// dependency splitting and lockfile handling reaches through this —
    /// documented debt, see `crate::toolchain` module docs. Absent for a
    /// pure Cargo workspace, where there is no JavaScript project to resolve
    /// a package manager or lockfile from.
    javascript: Option<Arc<JavaScriptToolchain<T>>>,
    /// Every toolchain contributing packages, JavaScript included. Package
    /// discovery goes through this and only this.
    toolchains: ToolchainRegistry,
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
}

impl<'a, T> BuildState<'a, ResolvedPackageManager, T>
where
    T: PackageDiscoveryBuilder,
    T::Output: Send + Sync + 'static,
    T::Error: Into<crate::package_manager::Error>,
{
    fn new(
        builder: PackageGraphBuilder<'a, T>,
    ) -> Result<BuildState<'a, ResolvedPackageManager, CachingPackageDiscovery<T::Output>>, Error>
    {
        let PackageGraphBuilder {
            repo_root,
            root_package_json,
            defer_closures,
            closure_hasher,
            is_single_package: single,

            package_jsons,
            lockfile,
            package_discovery,
            package_manager: _,
            extra_toolchains,
        } = builder;
        // Pure Cargo workspace: with no root package.json there is no
        // JavaScript project, so the JavaScript toolchain is neither
        // registered nor queried for a package manager. The graph is built
        // entirely from the extra toolchains (Cargo).
        let no_javascript = root_package_json.is_none();
        let mut workspaces = HashMap::new();
        let root_package_info = PackageInfo {
            // The root node always needs a descriptor; a pure Cargo workspace
            // has none, so it gets an empty one. The graph's public
            // `root_package_json()` still reports `None` (see below).
            package_json: root_package_json.clone().unwrap_or_default(),
            package_json_path: AnchoredSystemPathBuf::from_raw("package.json")?,
            ..Default::default()
        };
        workspaces.insert(PackageName::Root, root_package_info);

        let mut workspace_graph = Graph::new();
        let root_node_index = workspace_graph.add_node(PackageNode::Root);
        let root_workspace = PackageNode::Workspace(PackageName::Root);
        let root_workspace_index = workspace_graph.add_node(root_workspace.clone());
        workspace_graph.add_edge(
            root_workspace_index,
            root_node_index,
            DependencyKind::Production,
        );

        let mut node_lookup = HashMap::new();
        node_lookup.insert(PackageNode::Root, root_node_index);
        node_lookup.insert(root_workspace, root_workspace_index);

        // The discovery strategy is shared (via the JavaScript toolchain)
        // between package discovery and package-manager resolution; the
        // caching wrapper guarantees the underlying strategy runs once. For a
        // pure Cargo workspace there is no JavaScript project, so discovery is
        // not built and the toolchain is left unregistered.
        let mut toolchains = ToolchainRegistry::new();
        let javascript = if no_javascript {
            None
        } else {
            let javascript = Arc::new(JavaScriptToolchain::new(CachingPackageDiscovery::new(
                package_discovery.build().map_err(Into::into)?,
            )));
            // JavaScript registers first: its packages claim names before any
            // other toolchain's, so a cross-toolchain collision surfaces as the
            // non-JS package failing to add.
            toolchains.register(javascript.clone());
            Some(javascript)
        };
        for toolchain in extra_toolchains {
            toolchains.register(toolchain);
        }

        Ok(BuildState {
            repo_root,
            single,

            workspaces,
            lockfile,
            package_jsons,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            defer_closures,
            closure_hasher,
            state: std::marker::PhantomData,
            javascript,
            toolchains,
        })
    }
}

impl<'a, T: PackageDiscovery + Send + Sync> BuildState<'a, ResolvedPackageManager, T> {
    fn add_package(
        &mut self,
        toolchain: ToolchainId,
        package: DiscoveredPackage,
    ) -> Result<(), Error> {
        let DiscoveredPackage {
            descriptor: json,
            manifest_path,
            external_dependencies,
        } = package;
        let relative_json_path =
            AnchoredSystemPathBuf::relative_path_between(self.repo_root, &manifest_path);
        let name = PackageName::Other(
            json.name
                .clone()
                .ok_or(Error::PackageJsonMissingName(manifest_path))?
                .into_inner(),
        );
        // Toolchain-resolved external identities (e.g. Cargo's per-crate
        // lockfile closures), in the sorted representation the JS lockfile
        // phase produces. That phase later fills this for JavaScript
        // packages and never touches non-JS ones; the external-dependency
        // hash is computed on demand from the sorted closure.
        let transitive_dependencies = external_dependencies.map(|externals| {
            let mut sorted: Vec<std::sync::Arc<turborepo_lockfiles::Package>> =
                externals.into_iter().map(std::sync::Arc::new).collect();
            sorted.sort_by(|a, b| (&a.key, &a.version).cmp(&(&b.key, &b.version)));
            sorted
        });
        let entry = PackageInfo {
            package_json: json,
            package_json_path: relative_json_path,
            toolchain,
            transitive_dependencies,
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
        // A pre-supplied set of parsed package.json files (used by the
        // package-change watcher and tests) stands in for JavaScript
        // discovery only; other toolchains always discover for themselves.
        let mut pre_supplied = self.package_jsons.take();
        let mut discovered: Vec<(ToolchainId, DiscoveredPackage)> = Vec::new();
        for toolchain in self.toolchains.iter() {
            let id = toolchain.id();
            if id == ToolchainId::JAVASCRIPT
                && let Some(jsons) = pre_supplied.take()
            {
                discovered.extend(jsons.into_iter().map(|(path, json)| {
                    (
                        ToolchainId::JAVASCRIPT,
                        DiscoveredPackage {
                            descriptor: json,
                            manifest_path: path,
                            external_dependencies: None,
                        },
                    )
                }));
                continue;
            }
            let packages = toolchain.discover_packages().await?;
            discovered.extend(packages.into_iter().map(|package| (id.clone(), package)));
        }

        self.workspaces.reserve(discovered.len());
        self.node_lookup.reserve(discovered.len());

        let _span = tracing::info_span!("add_packages").entered();
        for (toolchain, package) in discovered {
            match self.add_package(toolchain, package) {
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
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            lockfile,
            javascript,
            toolchains,
            defer_closures,
            closure_hasher,
            ..
        } = self;
        Ok(BuildState {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            lockfile,
            javascript,
            toolchains,
            defer_closures,
            closure_hasher,
            package_jsons: None,
            state: std::marker::PhantomData,
        })
    }

    async fn build_single_package_graph(self) -> Result<PackageGraph, discovery::Error> {
        let Self {
            single,
            workspaces,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            lockfile,
            javascript,
            toolchains,
            repo_root,
            ..
        } = self;

        let package_manager = match &javascript {
            Some(javascript) => {
                let package_manager = javascript
                    .package_manager()
                    .await?
                    .with_resolved_nub_lockfile(repo_root);
                // Command resolution is synchronous; record the resolved
                // package manager on the toolchain so it does not re-run
                // discovery.
                javascript.set_resolved_package_manager(package_manager.clone());
                Some(package_manager)
            }
            None => None,
        };

        debug_assert!(single, "expected single package graph");
        Ok(PackageGraph {
            graph: workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            packages: workspaces,
            lockfile: lockfile.map(Arc::from),
            package_manager,
            repo_root: repo_root.to_owned(),
            deferred_closures: std::sync::Mutex::new(None),
            external_dep_to_internal_dependents: std::sync::OnceLock::new(),
            root_internal_dependencies: std::sync::OnceLock::new(),
            toolchains,
        })
    }
}

impl<'a, T: PackageDiscovery + Send + Sync> BuildState<'a, ResolvedWorkspaces, T> {
    #[tracing::instrument(skip(self))]
    fn connect_internal_dependencies(
        &mut self,
        package_manager: Option<&PackageManager>,
    ) -> Result<(), Error> {
        let path_index = WorkspacePathIndex::new(&self.workspaces);
        // Compute once — for pnpm/Berry this reads a config file from disk.
        // Without hoisting, the par_iter below would redundantly read the
        // same file N times (once per workspace). A pure Cargo workspace has
        // no package manager: crate edges use the `workspace:*` protocol,
        // which the splitter always resolves internally regardless of
        // workspace linking, and there are no pnpm catalogs.
        let link_workspace_packages =
            package_manager.is_some_and(|pm| pm.link_workspace_packages(self.repo_root));
        let catalogs = package_manager.and_then(|pm| pm.read_catalogs(self.repo_root));
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
                            entry.package_json.dependencies_with_kind(),
                            &path_index,
                            catalogs.as_ref(),
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
                self.workspace_graph
                    .add_edge(*node_idx, *root_idx, DependencyKind::Production);
            }
            for (dependency, kind) in internal {
                let dependency_idx = self
                    .node_lookup
                    .get(&PackageNode::Workspace(dependency))
                    .expect("unable to find workspace node index");
                self.workspace_graph
                    .add_edge(*node_idx, *dependency_idx, kind);
            }
            entry.unresolved_external_dependencies = Some(external);
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, package_manager))]
    async fn populate_lockfile(
        &mut self,
        package_manager: &PackageManager,
    ) -> Result<Box<dyn Lockfile>, Error> {
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

    #[tracing::instrument(skip(self, lockfile_future))]
    async fn resolve_lockfile(
        mut self,
        lockfile_future: Option<tokio::task::JoinHandle<Option<Box<dyn Lockfile>>>>,
    ) -> Result<BuildState<'a, ResolvedLockfile, T>, Error> {
        // Since we've already performed package discovery, this should just be
        // a cache hit. A pure Cargo workspace has no JavaScript toolchain and
        // therefore no package manager or lockfile.
        let package_manager = match &self.javascript {
            Some(javascript) => Some(
                javascript
                    .package_manager()
                    .await?
                    .with_resolved_nub_lockfile(self.repo_root),
            ),
            None => None,
        };
        turborepo_rayon_compat::block_in_place(|| {
            self.connect_internal_dependencies(package_manager.as_ref())
        })?;

        if let Some(handle) = lockfile_future
            && let Ok(Some(lockfile)) = handle.await
        {
            self.lockfile = Some(lockfile);
        }

        let lockfile = match package_manager.as_ref() {
            // No JavaScript package manager (pure Cargo): no JS lockfile to
            // parse. Cargo's own lockfile is handled by the Cargo toolchain.
            None => None,
            Some(package_manager) => match self.populate_lockfile(package_manager).await {
                Ok(lockfile) => Some(lockfile),
                Err(e) => {
                    let problematic_file_path =
                        extract_file_path_from_error(&e, package_manager, self.repo_root);

                    warn!(
                        "An issue occurred while attempting to parse {}. Turborepo will still \
                         function, but some features may not be available:\n {:?}",
                        problematic_file_path,
                        Report::new(e)
                    );
                    None
                }
            },
        };

        let Self {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            javascript,
            toolchains,
            defer_closures,
            closure_hasher,
            ..
        } = self;
        Ok(BuildState {
            repo_root,
            single,
            workspaces,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            lockfile,
            defer_closures,
            closure_hasher,
            package_jsons: None,
            state: std::marker::PhantomData,
            javascript,
            toolchains,
        })
    }
}

/// Computes per-workspace external dependency hashes from sorted closures,
/// keyed by workspace unix directory. See
/// [`PackageGraphBuilder::with_closure_hasher`].
pub type ClosureHasher = Arc<
    dyn Fn(&HashMap<String, Vec<Arc<turborepo_lockfiles::Package>>>) -> HashMap<String, String>
        + Send
        + Sync,
>;

impl<T: PackageDiscovery + Send + Sync> BuildState<'_, ResolvedLockfile, T> {
    fn all_external_dependencies(
        &self,
    ) -> Result<HashMap<String, BTreeMap<String, String>>, Error> {
        self.workspaces
            .values()
            // Only JavaScript packages participate in the JS lockfile's
            // external-dependency closures. This map is keyed by directory,
            // and a non-JS package can share a directory with a JS one (the
            // synthetic Cargo workspace package lives at the repo root, like
            // the root package) — including both would let HashMap iteration
            // order decide which entry survives, flipping the root's
            // external-dependency hash run to run.
            .filter(|entry| entry.toolchain == ToolchainId::JAVASCRIPT)
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
        let mut closures = turborepo_lockfiles::all_transitive_closures_sorted(
            lockfile,
            self.all_external_dependencies()?,
            false,
        )?;
        let mut hashes = self
            .closure_hasher
            .as_ref()
            .map(|hasher| hasher(&closures))
            .unwrap_or_default();
        for entry in self.workspaces.values_mut() {
            // Mirror of the filter in all_external_dependencies: a non-JS
            // package sharing a directory with a JS package must not steal
            // its closure.
            if entry.toolchain != ToolchainId::JAVASCRIPT {
                continue;
            }
            let dir = entry.unix_dir_str()?;
            entry.transitive_dependencies = closures.remove(&dir);
            entry.external_deps_hash = hashes.remove(&dir);
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn build_inner(mut self) -> Result<PackageGraph, discovery::Error> {
        // Transitive closures are only consumed by task hashing and
        // change-detection, well after graph construction. When deferral is
        // requested, compute them on a background thread so package-list
        // consumers (microfrontends config, turbo.json preloading, engine
        // construction) overlap with the closure work instead of waiting
        // behind it. `PackageGraph::ensure_transitive_closures` joins.
        let mut deferred_closures = None;
        let arc_lockfile: Option<Arc<dyn Lockfile>> = if self.defer_closures {
            let lockfile: Option<Arc<dyn Lockfile>> = self.lockfile.take().map(Arc::from);
            if let Some(lockfile) = lockfile.clone() {
                match self.all_external_dependencies() {
                    Ok(external_deps) => {
                        let (tx, rx) = std::sync::mpsc::sync_channel(1);
                        let hasher = self.closure_hasher.clone();
                        let spawned = std::thread::Builder::new()
                            .name("turbo-closures".into())
                            .spawn(move || {
                                let result = turborepo_lockfiles::all_transitive_closures_sorted(
                                    lockfile.as_ref(),
                                    external_deps,
                                    false,
                                )
                                .map(|closures| {
                                    let hashes = hasher
                                        .as_ref()
                                        .map(|hasher| hasher(&closures))
                                        .unwrap_or_default();
                                    super::DeferredClosures { closures, hashes }
                                });
                                let _ = tx.send(result.map_err(|e| e.to_string()));
                            });
                        match spawned {
                            Ok(_) => deferred_closures = Some(rx),
                            Err(e) => {
                                warn!("Unable to spawn transitive closure thread: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Unable to calculate transitive closures: {}", e);
                    }
                }
            }
            lockfile
        } else {
            if let Err(e) =
                turborepo_rayon_compat::block_in_place(|| self.populate_transitive_dependencies())
            {
                warn!("Unable to calculate transitive closures: {}", e);
            }
            self.lockfile.take().map(Arc::from)
        };
        // A pure Cargo workspace has no JavaScript toolchain, hence no package
        // manager to resolve.
        let package_manager = match &self.javascript {
            Some(javascript) => {
                let package_manager = javascript
                    .package_manager()
                    .instrument(tracing::debug_span!("package discovery"))
                    .await?
                    .with_resolved_nub_lockfile(self.repo_root);
                // Command resolution is synchronous; record the resolved
                // package manager on the toolchain so it does not re-run
                // discovery.
                javascript.set_resolved_package_manager(package_manager.clone());
                Some(package_manager)
            }
            None => None,
        };
        let Self {
            workspaces,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            toolchains,
            repo_root,
            ..
        } = self;
        Ok(PackageGraph {
            graph: workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            packages: workspaces,
            package_manager,
            lockfile: arc_lockfile,
            repo_root: repo_root.to_owned(),
            deferred_closures: std::sync::Mutex::new(deferred_closures),
            external_dep_to_internal_dependents: std::sync::OnceLock::new(),
            root_internal_dependencies: std::sync::OnceLock::new(),
            toolchains,
        })
    }
}

struct Dependencies {
    internal: HashMap<PackageName, DependencyKind>,
    external: BTreeMap<String, String>, // Package name and version
}

impl Dependencies {
    pub fn new<'a, I: IntoIterator<Item = (&'a String, &'a String, DependencyKind)>>(
        repo_root: &AbsoluteSystemPath,
        workspace_json_path: &AnchoredSystemPathBuf,
        workspaces: &HashMap<PackageName, PackageInfo>,
        link_workspace_packages: bool,
        dependencies: I,
        path_index: &WorkspacePathIndex<'_>,
        catalogs: Option<&PnpmCatalogs>,
    ) -> Self {
        let resolved_workspace_json_path = repo_root.resolve(workspace_json_path);
        let workspace_dir = resolved_workspace_json_path
            .parent()
            .expect("package.json path should have parent");
        let mut internal = HashMap::new();
        let mut external = BTreeMap::new();
        let mut seen = HashSet::new();
        let splitter = DependencySplitter::new(
            repo_root,
            workspace_dir,
            workspaces,
            link_workspace_packages,
            path_index,
            catalogs,
        );
        for (name, version, kind) in dependencies.into_iter() {
            if !seen.insert(name.clone()) {
                continue;
            }

            match kind {
                // Peers are provided by consumers and are not package graph inputs.
                DependencyKind::Peer { .. } => {}
                DependencyKind::Production | DependencyKind::Development => {
                    if let Some(workspace) = splitter.is_internal(name, version) {
                        internal.entry(workspace).or_insert(kind);
                    } else {
                        external.insert(name.clone(), version.clone());
                    }
                }
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
    use std::{assert_matches, collections::HashMap};

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
    async fn test_dev_dependency_edge_kind() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["apps", "web", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("web".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("lib".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    dev_dependencies: Some(
                        [("tooling".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "lib", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("lib".into())),
                    version: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "tooling", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("tooling".into())),
                    version: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let web = PackageNode::Workspace(PackageName::from("web"));
        let lib = PackageNode::Workspace(PackageName::from("lib"));
        let tooling = PackageNode::Workspace(PackageName::from("tooling"));

        assert_eq!(
            graph.dependency_kind(&web, &lib),
            Some(DependencyKind::Production)
        );
        assert_eq!(
            graph.dependency_kind(&web, &tooling),
            Some(DependencyKind::Development)
        );

        let web_closure = graph.production_transitive_closure([&web]);
        assert!(web_closure.contains(&web));
        assert!(web_closure.contains(&lib));
        assert!(!web_closure.contains(&tooling));
    }

    #[tokio::test]
    async fn test_duplicate_dependency_prefers_production_kind() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["apps", "web", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("web".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("shared".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    dev_dependencies: Some(
                        [("shared".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "shared", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("shared".into())),
                    version: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let web = PackageNode::Workspace(PackageName::from("web"));
        let shared = PackageNode::Workspace(PackageName::from("shared"));

        assert_eq!(
            graph.dependency_kind(&web, &shared),
            Some(DependencyKind::Production)
        );
    }

    #[tokio::test]
    async fn test_peer_workspace_dep_does_not_override_concrete_external_dep() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("b".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "b", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("b".into())),
                    version: Some("1.0.0".to_string()),
                    dev_dependencies: Some(
                        [("buffer".to_string(), "npm:buffer@6.0.3".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    peer_dependencies: Some(
                        [("buffer".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "buffer", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("buffer".into())),
                    version: Some("6.0.3".to_string()),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let b = PackageName::from("b");
        let buffer = PackageName::from("buffer");
        let b_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(b.clone()))
            .unwrap();
        assert!(
            !b_deps.contains(&PackageNode::Workspace(buffer)),
            "peer workspace specifier should not create an internal edge when a concrete external \
             dependency exists, got: {:?}",
            b_deps
        );

        let b_external = graph
            .package_info(&b)
            .unwrap()
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();
        assert_eq!(
            b_external.get("buffer").map(|v| v.as_str()),
            Some("npm:buffer@6.0.3")
        );
    }

    #[tokio::test]
    async fn test_pure_peer_workspace_dep_does_not_create_edge() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("b".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "b", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("b".into())),
                    version: Some("1.0.0".to_string()),
                    peer_dependencies: Some(
                        [("a".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let a = PackageName::from("a");
        let b = PackageName::from("b");

        let a_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(a.clone()))
            .unwrap();
        assert!(
            a_deps.contains(&PackageNode::Workspace(b.clone())),
            "a should depend on b, got: {:?}",
            a_deps
        );

        let b_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(b.clone()))
            .unwrap();
        assert!(
            !b_deps.contains(&PackageNode::Workspace(a)),
            "pure peer workspace specifier should not create an internal edge, got: {:?}",
            b_deps
        );

        assert!(
            graph.find_cycles().is_empty(),
            "package graph should be acyclic once the pure peer edge is dropped"
        );
    }

    #[tokio::test]
    async fn test_external_peer_dep_is_not_retained_as_external() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    version: Some("1.0.0".to_string()),
                    peer_dependencies: Some(
                        [("react".to_string(), "*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let a = PackageName::from("a");
        let a_external = graph
            .package_info(&a)
            .unwrap()
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();
        assert!(
            !a_external.contains_key("react"),
            "external peer dependency should not be retained as an external dep, got: {:?}",
            a_external
        );
    }

    #[tokio::test]
    async fn test_optional_external_peer_is_not_retained() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson::from_value(serde_json::json!({
                    "name": "a",
                    "version": "1.0.0",
                    "peerDependencies": {
                        "react": "*",
                        "lodash": "*"
                    },
                    "peerDependenciesMeta": {
                        "react": { "optional": true }
                    }
                }))
                .unwrap(),
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let a = PackageName::from("a");
        let a_external = graph
            .package_info(&a)
            .unwrap()
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();
        assert!(
            !a_external.contains_key("react"),
            "optional peer should not be retained, got: {:?}",
            a_external
        );
        assert!(
            !a_external.contains_key("lodash"),
            "required peer should not be retained, got: {:?}",
            a_external
        );
    }

    #[tokio::test]
    async fn test_peer_dependencies_do_not_create_internal_edges() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "app", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("app".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("lib".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "lib", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("lib".into())),
                    version: Some("1.0.0".to_string()),
                    peer_dependencies: Some(
                        [
                            ("app".to_string(), "workspace:*".to_string()),
                            ("react".to_string(), "*".to_string()),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let app = PackageNode::Workspace(PackageName::from("app"));
        let lib = PackageNode::Workspace(PackageName::from("lib"));

        let lib_closure = graph.transitive_closure([&lib]);
        assert!(
            !lib_closure.contains(&app),
            "package graph closure for lib should exclude pure-peer workspace app, got: \
             {lib_closure:?}"
        );
        assert!(
            graph.transitive_closure([&app]).contains(&lib),
            "prune closure for app should include its regular dependency lib"
        );

        let lib_external = graph
            .package_info(&PackageName::from("lib"))
            .unwrap()
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();
        assert!(
            !lib_external.contains_key("react"),
            "external peer should not be retained by package graph, got: {:?}",
            lib_external
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
