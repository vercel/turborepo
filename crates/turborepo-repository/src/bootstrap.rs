//! Shared, construction-scoped repository observation and contributor planning.
//!
//! Root recognition reads each enabled native adapter once; membership uses its
//! lazy scope inventory, never full discovery. Optional JavaScript metadata is
//! owned once by the root observation, not cloned into native contributor
//! plans. Plans are immutable snapshots, not filesystem caches. Rebuild
//! explicitly at process/config boundaries (notably shim -> CLI), and after
//! graph-defining changes in watch mode; do not re-probe between root loading
//! and graph building.

pub mod javascript;

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::toolchain::{
    self, DiscoveredPackageScopes, DiscoveredScopeKind, RepositoryContributor, ToolchainId,
};

/// A bootstrap failure, retaining the native diagnostic and its source.
#[derive(Debug, thiserror::Error)]
#[error("{toolchain} workspace at {path}: {source}")]
pub struct BootstrapError {
    pub toolchain: ToolchainId,
    pub path: AbsoluteSystemPathBuf,
    #[source]
    pub source: Arc<dyn std::error::Error + Send + Sync>,
}

impl BootstrapError {
    pub fn new(
        toolchain: ToolchainId,
        path: AbsoluteSystemPathBuf,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            toolchain,
            path,
            source: Arc::new(source),
        }
    }
}

/// Graph-construction options understood by contributor factories.
#[derive(Debug, Clone, Copy)]
pub struct ContributorOptions {
    pub resolve_external_dependencies: bool,
}

impl Default for ContributorOptions {
    fn default() -> Self {
        Self {
            resolve_external_dependencies: true,
        }
    }
}

/// An open-ended, subprocess-free root adapter.
///
/// `probe` must read only root definitions, not enumerate members or invoke a
/// toolchain. A contributor factory is optional for recognition-only adapters.
/// Contributors attached to a workspace must honor the subprocess-free
/// [`RepositoryContributor::discover_package_scopes`] contract and complete
/// their inventories without requiring an async runtime.
pub trait ToolchainBootstrap: Send + Sync {
    fn id(&self) -> ToolchainId;

    fn probe(&self, dir: &AbsoluteSystemPath)
    -> Result<Option<BootstrapWorkspace>, BootstrapError>;

    fn contributor(
        &self,
        _dir: &AbsoluteSystemPath,
        _options: ContributorOptions,
    ) -> Option<Arc<dyn RepositoryContributor>> {
        None
    }
}

type ScopeInventory = Result<DiscoveredPackageScopes, Arc<toolchain::Error>>;

/// A recognized root and the contributor that can answer membership queries.
///
/// Clones share their lazily initialized inventory, including failures. The
/// snapshot is construction-scoped: probe again after changing manifests.
#[derive(Clone)]
pub struct BootstrapWorkspace {
    id: ToolchainId,
    root: AbsoluteSystemPathBuf,
    manifest_path: AbsoluteSystemPathBuf,
    contributor: Arc<dyn RepositoryContributor>,
    inventory: Arc<OnceLock<ScopeInventory>>,
}

impl std::fmt::Debug for BootstrapWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapWorkspace")
            .field("id", &self.id)
            .field("root", &self.root)
            .field("manifest_path", &self.manifest_path)
            .finish_non_exhaustive()
    }
}

impl BootstrapWorkspace {
    pub fn new(
        root: AbsoluteSystemPathBuf,
        manifest_path: AbsoluteSystemPathBuf,
        contributor: Arc<dyn RepositoryContributor>,
    ) -> Self {
        Self {
            id: contributor.id(),
            root,
            manifest_path,
            contributor,
            inventory: Arc::new(OnceLock::new()),
        }
    }

    pub fn id(&self) -> ToolchainId {
        self.id.clone()
    }

    pub fn root(&self) -> &AbsoluteSystemPath {
        &self.root
    }

    pub fn manifest_path(&self) -> &AbsoluteSystemPath {
        &self.manifest_path
    }

    pub fn contributor(&self) -> Arc<dyn RepositoryContributor> {
        self.contributor.clone()
    }

    /// Whether `target` is this root or lies inside a declared package scope.
    ///
    /// Root/outside-root queries need no inventory. All other queries reuse the
    /// native inventory, including glob expansion, exclusions, implicit Cargo
    /// path members, and native errors. An aggregate scope must not claim every
    /// descendant of the root: unrelated directories are not workspace members.
    pub fn owns(&self, target: &AbsoluteSystemPath) -> Result<bool, BootstrapError> {
        if target == self.root() {
            return Ok(true);
        }
        if !self.root.contains(target) {
            return Ok(false);
        }
        let inventory = self.inventory.get_or_init(|| {
            futures::executor::block_on(self.contributor.discover_package_scopes())
                .map_err(Arc::new)
        });
        let inventory = inventory.as_ref().map_err(|source| BootstrapError {
            toolchain: self.id(),
            path: self.manifest_path.clone(),
            source: source.clone(),
        })?;
        Ok(inventory.scopes().iter().any(|scope| {
            scope.scope_kind() == DiscoveredScopeKind::Package
                && scope
                    .manifest_path()
                    .parent()
                    .is_some_and(|dir| self.root.contains(dir) && dir.contains(target))
        }))
    }
}

/// Failures during the common root observation, preserving original sources.
#[derive(Debug, thiserror::Error)]
pub enum RootObservationError {
    #[error(transparent)]
    Bootstrap(#[from] BootstrapError),
    #[error(transparent)]
    PackageJson(#[from] crate::package_json::Error),
}

/// The enabled registry and optional invocation-resolved config provenance.
#[derive(Clone, Default)]
pub struct RepositoryBootstrap {
    registry: Registry,
    config_path: Option<AbsoluteSystemPathBuf>,
}

impl RepositoryBootstrap {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            config_path: None,
        }
    }

    pub fn with_config_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Observe JavaScript and all enabled native root definitions exactly once.
    /// Missing JavaScript is optional; malformed JavaScript remains an error.
    pub fn observe(
        &self,
        root: &AbsoluteSystemPath,
    ) -> Result<RootObservation, RootObservationError> {
        let (javascript, missing_javascript) = match javascript::JavaScriptBootstrap::observe(root)
        {
            Ok(workspace) => (Some(workspace), None),
            Err(crate::package_json::Error::Io(io))
                if io.kind() == std::io::ErrorKind::NotFound =>
            {
                (None, Some(crate::package_json::Error::Io(io)))
            }
            Err(error) => return Err(error.into()),
        };
        let mut plan = self.registry.contributor_plan(root)?;
        plan.config_path = self.config_path.clone();
        Ok(RootObservation {
            plan,
            javascript,
            missing_javascript,
        })
    }
}

/// One root decision shared by inference and optional graph root loading.
/// JavaScript metadata is moved out once; only the native plan is cloned.
#[derive(Debug)]
pub struct RootObservation {
    plan: ContributorPlan,
    javascript: Option<javascript::JavaScriptWorkspace>,
    missing_javascript: Option<crate::package_json::Error>,
}

impl RootObservation {
    pub fn root(&self) -> &AbsoluteSystemPath {
        self.plan.root()
    }
    pub fn contributor_plan(&self) -> ContributorPlan {
        self.plan.clone()
    }
    pub fn is_repository(&self) -> bool {
        self.javascript.is_some() || !self.plan.native_workspaces.is_empty()
    }
    pub fn is_workspace(&self) -> bool {
        self.javascript
            .as_ref()
            .is_some_and(javascript::JavaScriptWorkspace::is_workspace)
            || !self.plan.native_workspaces.is_empty()
    }
    pub fn owns(&self, target: &AbsoluteSystemPath) -> Result<bool, BootstrapError> {
        if self
            .javascript
            .as_ref()
            .is_some_and(|workspace| workspace.owns(target))
        {
            return Ok(true);
        }
        for workspace in &self.plan.native_workspaces {
            if workspace.owns(target)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn into_javascript(self) -> Option<javascript::JavaScriptRoot> {
        self.javascript
            .map(javascript::JavaScriptWorkspace::into_root)
    }
    /// Missing package.json is valid only for a recognized native workspace.
    /// Preserve the original I/O error when no ecosystem recognizes this root.
    pub fn into_graph_parts(
        self,
    ) -> Result<
        (Option<crate::package_json::PackageJson>, ContributorPlan),
        crate::package_json::Error,
    > {
        if self.plan.native_workspaces.is_empty()
            && let Some(error) = self.missing_javascript
        {
            return Err(error);
        }
        // A native workspace may contain package.json solely for repository
        // metadata. Keep JS-only validation unchanged, but do not manufacture a
        // JavaScript graph participant (and require a JS package manager) for an
        // otherwise native-only root.
        let has_native_workspace = !self.plan.native_workspaces.is_empty();
        Ok((
            self.javascript
                .filter(|workspace| !has_native_workspace || workspace.participates_in_graph())
                .map(|workspace| workspace.into_root().package_json),
            self.plan,
        ))
    }
}

/// Recognized native observations plus their enabled factories. Clones share
/// membership inventories. Creating contributors applies final graph options
/// without re-reading root definitions or JavaScript metadata.
#[derive(Clone)]
pub struct ContributorPlan {
    root: AbsoluteSystemPathBuf,
    registry: Registry,
    native_workspaces: Vec<BootstrapWorkspace>,
    config_path: Option<AbsoluteSystemPathBuf>,
}

impl std::fmt::Debug for ContributorPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContributorPlan")
            .field("root", &self.root)
            .field(
                "enabled_ids",
                &self.registry.enabled_ids().collect::<Vec<_>>(),
            )
            .field("native_workspaces", &self.native_workspaces)
            .field("config_path", &self.config_path)
            .finish()
    }
}

impl ContributorPlan {
    pub fn root(&self) -> &AbsoluteSystemPath {
        &self.root
    }
    pub fn config_path(&self) -> Option<&AbsoluteSystemPath> {
        self.config_path.as_deref()
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn native_workspaces(&self) -> &[BootstrapWorkspace] {
        &self.native_workspaces
    }
    pub fn contributors(&self, options: ContributorOptions) -> Vec<Arc<dyn RepositoryContributor>> {
        self.native_workspaces
            .iter()
            .filter_map(|workspace| {
                self.registry
                    .contributor(&workspace.id(), &self.root, options)
            })
            .collect()
    }
}

/// Enabled adapters, in deterministic registration order.
///
/// Disabled adapters are absent, so neither probing nor factory lookup reads
/// their manifests. Register another ecosystem with `with_adapter`; inference
/// and graph construction need no additional toolchain switches.
#[derive(Default, Clone)]
pub struct Registry {
    adapters: Vec<Arc<dyn ToolchainBootstrap>>,
}

impl Registry {
    /// Enable the built-in native adapters identified by `enabled`.
    /// Unknown IDs are ignored; register their implementation with
    /// `with_adapter`. JavaScript is intentionally not a built-in bootstrap
    /// adapter.
    pub fn new(enabled: impl IntoIterator<Item = ToolchainId>) -> Self {
        let enabled = enabled.into_iter().collect::<Vec<_>>();
        Self {
            adapters: native_adapters()
                .into_iter()
                .filter(|(_, adapter)| enabled.contains(&adapter.id()))
                .map(|(_, adapter)| adapter)
                .collect(),
        }
    }

    /// Interpret the `futureFlags` object. Only boolean `true` enables an
    /// adapter; flag names and factories are registered together in one place.
    pub fn from_flags(flags: &Map<String, Value>) -> Self {
        Self {
            adapters: native_adapters()
                .into_iter()
                .filter(|(flag, _)| flags.get(*flag).and_then(Value::as_bool) == Some(true))
                .map(|(_, adapter)| adapter)
                .collect(),
        }
    }

    /// Enable an adapter, replacing an existing registration of the same ID.
    pub fn with_adapter(mut self, adapter: Arc<dyn ToolchainBootstrap>) -> Self {
        if let Some(existing) = self
            .adapters
            .iter_mut()
            .find(|existing| existing.id() == adapter.id())
        {
            *existing = adapter;
        } else {
            self.adapters.push(adapter);
        }
        self
    }

    pub fn enabled_ids(&self) -> impl Iterator<Item = ToolchainId> + '_ {
        self.adapters.iter().map(|adapter| adapter.id())
    }

    /// Recognize all enabled native roots at `dir`, without member discovery.
    pub fn probe(
        &self,
        dir: &AbsoluteSystemPath,
    ) -> Result<Vec<BootstrapWorkspace>, BootstrapError> {
        self.adapters
            .iter()
            .filter_map(|adapter| adapter.probe(dir).transpose())
            .collect()
    }

    /// Observe native roots for callers that already own JavaScript metadata.
    pub fn contributor_plan(
        &self,
        root: &AbsoluteSystemPath,
    ) -> Result<ContributorPlan, BootstrapError> {
        Ok(ContributorPlan {
            root: root.to_owned(),
            registry: self.clone(),
            native_workspaces: self.probe(root)?,
            config_path: None,
        })
    }

    /// Create a graph contributor without re-reading flags or probing members.
    /// Returns `None` for disabled, unknown, or recognition-only adapters.
    pub fn contributor(
        &self,
        id: &ToolchainId,
        dir: &AbsoluteSystemPath,
        options: ContributorOptions,
    ) -> Option<Arc<dyn RepositoryContributor>> {
        self.adapters
            .iter()
            .find(|adapter| adapter.id() == *id)
            .and_then(|adapter| adapter.contributor(dir, options))
    }

    /// Synchronous inference convenience: recognize roots, then query
    /// membership.
    pub fn probe_member(
        &self,
        dir: &AbsoluteSystemPath,
        target: &AbsoluteSystemPath,
    ) -> Result<bool, BootstrapError> {
        for workspace in self.probe(dir)? {
            if workspace.owns(target)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

// The only built-in registration list. Factories and root parsing travel with
// each adapter rather than being duplicated by inference and graph callers.
fn native_adapters() -> [(&'static str, Arc<dyn ToolchainBootstrap>); 3] {
    [
        ("experimentalCargoWorkspaces", Arc::new(RustBootstrap)),
        ("experimentalPythonWorkspaces", Arc::new(PythonBootstrap)),
        ("experimentalGoWorkspaces", Arc::new(GoBootstrap)),
    ]
}

struct RustBootstrap;

impl ToolchainBootstrap for RustBootstrap {
    fn id(&self) -> ToolchainId {
        ToolchainId::RUST
    }

    fn probe(
        &self,
        dir: &AbsoluteSystemPath,
    ) -> Result<Option<BootstrapWorkspace>, BootstrapError> {
        let path = dir.join_component(crate::cargo::CARGO_TOML);
        let error = |source| BootstrapError::new(self.id(), path.clone(), source);
        let Some(contents) = path
            .read_existing_to_string()
            .map_err(|source| error(crate::cargo::Error::WorkspaceFileRead(source)))?
        else {
            return Ok(None);
        };
        // Match Cargo's in-process inventory parser, without expanding members.
        let document: toml_edit::DocumentMut = contents
            .parse()
            .map_err(|source| error(crate::cargo::Error::ManifestParse(Box::new(source))))?;
        let Some(workspace) = document.get("workspace") else {
            return Ok(None);
        };
        if workspace.as_table().is_none() {
            return Err(error(crate::cargo::Error::NotAWorkspace));
        }
        Ok(Some(BootstrapWorkspace::new(
            dir.to_owned(),
            path,
            crate::cargo::CargoContributor::new(dir.to_owned()),
        )))
    }

    fn contributor(
        &self,
        dir: &AbsoluteSystemPath,
        options: ContributorOptions,
    ) -> Option<Arc<dyn RepositoryContributor>> {
        Some(if options.resolve_external_dependencies {
            crate::cargo::CargoContributor::new(dir.to_owned())
        } else {
            crate::cargo::CargoContributor::new_without_external_dependencies(dir.to_owned())
        })
    }
}

struct PythonBootstrap;

impl ToolchainBootstrap for PythonBootstrap {
    fn id(&self) -> ToolchainId {
        ToolchainId::PYTHON
    }

    fn probe(
        &self,
        dir: &AbsoluteSystemPath,
    ) -> Result<Option<BootstrapWorkspace>, BootstrapError> {
        let path = dir.join_component(crate::uv::PYPROJECT_TOML);
        if !crate::uv::is_workspace_root(dir)
            .map_err(|source| BootstrapError::new(self.id(), path.clone(), source))?
        {
            return Ok(None);
        }
        Ok(Some(BootstrapWorkspace::new(
            dir.to_owned(),
            path,
            crate::uv::UvContributor::new(dir.to_owned()),
        )))
    }

    fn contributor(
        &self,
        dir: &AbsoluteSystemPath,
        _options: ContributorOptions,
    ) -> Option<Arc<dyn RepositoryContributor>> {
        Some(crate::uv::UvContributor::new(dir.to_owned()))
    }
}

struct GoBootstrap;

impl ToolchainBootstrap for GoBootstrap {
    fn id(&self) -> ToolchainId {
        ToolchainId::GO
    }

    fn probe(
        &self,
        dir: &AbsoluteSystemPath,
    ) -> Result<Option<BootstrapWorkspace>, BootstrapError> {
        let path = dir.join_component(crate::go::GO_WORK);
        let Some(contents) = path.read_existing_to_string().map_err(|source| {
            BootstrapError::new(
                self.id(),
                path.clone(),
                crate::go::Error::ManifestRead {
                    path: path.to_string(),
                    source,
                },
            )
        })?
        else {
            return Ok(None);
        };
        // Share the inventory parser without checking members, their paths,
        // secondary workspaces, or whether the member list is empty.
        crate::go::validate_workspace_root(&contents, &path)
            .map_err(|source| BootstrapError::new(self.id(), path.clone(), source))?;
        Ok(Some(BootstrapWorkspace::new(
            dir.to_owned(),
            path,
            crate::go::GoContributor::new(dir.to_owned()),
        )))
    }

    fn contributor(
        &self,
        dir: &AbsoluteSystemPath,
        _options: ContributorOptions,
    ) -> Option<Arc<dyn RepositoryContributor>> {
        Some(crate::go::GoContributor::new(dir.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use test_case::test_case;

    use super::*;
    use crate::toolchain::{
        DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackageScope, WorkspaceRoot,
    };

    fn write(root: &AbsoluteSystemPath, path: &str, contents: &str) {
        let path = AbsoluteSystemPathBuf::from_unknown(root, path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path.create_with_contents(contents).unwrap();
    }

    fn fixture(root: &AbsoluteSystemPath, id: &ToolchainId) -> &'static str {
        if *id == ToolchainId::RUST {
            write(
                root,
                "Cargo.toml",
                "[workspace]\nmembers = [\"members/*\"]\nexclude = \
                 [\"members/excluded\"]\n[workspace.metadata]\nname = \"workspace\"\n",
            );
            for member in ["owned", "excluded"] {
                write(
                    root,
                    &format!("members/{member}/Cargo.toml"),
                    &format!("[package]\nname = \"{member}\"\nversion = \"0.1.0\"\n"),
                );
            }
            "Cargo.toml"
        } else if *id == ToolchainId::PYTHON {
            write(
                root,
                "pyproject.toml",
                "[tool.uv.workspace]\nmembers = [\"members/*\"]\nexclude = \
                 [\"members/excluded\"]\n[tool.turbo]\nname = \"workspace\"\n",
            );
            for member in ["owned", "excluded"] {
                write(
                    root,
                    &format!("members/{member}/pyproject.toml"),
                    &format!("[project]\nname = \"{member}\"\nversion = \"0.1.0\"\n"),
                );
            }
            "pyproject.toml"
        } else {
            write(root, "go.work", "go 1.22\nuse (\n ./members/owned\n)\n");
            for member in ["owned", "excluded"] {
                write(
                    root,
                    &format!("members/{member}/go.mod"),
                    &format!("module example.com/{member}\ngo 1.22\n"),
                );
            }
            "go.work"
        }
    }

    #[test_case(ToolchainId::RUST, None; "rust_without_package_json")]
    #[test_case(ToolchainId::PYTHON, None; "python_without_package_json")]
    #[test_case(ToolchainId::GO, None; "go_without_package_json")]
    #[test_case(ToolchainId::RUST, Some(r#"{"name":"unrelated"}"#); "rust_with_irrelevant_package_json")]
    #[test_case(ToolchainId::PYTHON, Some(r#"{"name":"unrelated"}"#); "python_with_irrelevant_package_json")]
    #[test_case(ToolchainId::GO, Some(r#"{"name":"unrelated"}"#); "go_with_irrelevant_package_json")]
    #[test_case(ToolchainId::RUST, Some("not JSON"); "rust_with_malformed_package_json")]
    #[test_case(ToolchainId::PYTHON, Some("not JSON"); "python_with_malformed_package_json")]
    #[test_case(ToolchainId::GO, Some("not JSON"); "go_with_malformed_package_json")]
    fn native_membership(id: ToolchainId, package_json: Option<&str>) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let manifest = fixture(&root, &id);
        if let Some(contents) = package_json {
            // Even malformed JavaScript is irrelevant to native root eligibility.
            write(&root, "package.json", contents);
        }
        let registry = Registry::new([id.clone()]);
        let workspaces = registry.probe(&root).unwrap();
        assert_eq!(workspaces.len(), 1);
        let workspace = &workspaces[0];
        assert_eq!(workspace.id(), id);
        assert_eq!(workspace.root(), &*root);
        assert_eq!(workspace.manifest_path(), &*root.join_component(manifest));
        assert!(workspace.owns(&root).unwrap());
        for path in [
            "members/owned",
            "members/owned/src/deep/file",
            "members/owned/package.json",
        ] {
            assert!(
                workspace
                    .owns(&AbsoluteSystemPathBuf::from_unknown(&root, path))
                    .unwrap(),
                "{path}"
            );
        }
        for path in [
            "members/excluded",
            "members/excluded/src",
            "members/owned-other",
            "unrelated/src",
        ] {
            assert!(
                !workspace
                    .owns(&AbsoluteSystemPathBuf::from_unknown(&root, path))
                    .unwrap(),
                "{path}"
            );
        }
        assert!(!workspace.owns(root.parent().unwrap()).unwrap());
        assert_eq!(
            registry
                .contributor(&id, &root, ContributorOptions::default())
                .unwrap()
                .id(),
            id
        );
        assert!(
            registry
                .contributor(
                    &ToolchainId::new("disabled"),
                    &root,
                    ContributorOptions::default()
                )
                .is_none()
        );
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn native_probe_does_not_read_members(id: ToolchainId) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        fixture(&root, &id);
        let member_manifest = if id == ToolchainId::RUST {
            "members/owned/Cargo.toml"
        } else if id == ToolchainId::PYTHON {
            "members/owned/pyproject.toml"
        } else {
            "members/owned/go.mod"
        };
        write(&root, member_manifest, "[");
        let workspaces = Registry::new([id.clone()]).probe(&root).unwrap();
        assert!(workspaces[0].owns(&root).unwrap());
        let target = AbsoluteSystemPathBuf::from_unknown(&root, "members/owned/src");
        let error = workspaces[0].owns(&target).unwrap_err();
        assert_eq!(error.toolchain, id);
        assert!(error.source.downcast_ref::<toolchain::Error>().is_some());
    }

    #[test_case(ToolchainId::RUST, "Cargo.toml"; "rust")]
    #[test_case(ToolchainId::PYTHON, "pyproject.toml"; "python")]
    #[test_case(ToolchainId::GO, "go.work"; "go")]
    fn malformed_native_manifests_preserve_errors(id: ToolchainId, manifest: &str) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        write(&root, manifest, "[");
        let registry = Registry::new([id.clone()]);
        let error = registry.probe(&root).unwrap_err();
        assert_eq!(error.toolchain, id);
        assert_eq!(error.path, root.join_component(manifest));
        let RootObservationError::Bootstrap(observed_error) = RepositoryBootstrap::new(registry)
            .observe(&root)
            .unwrap_err()
        else {
            panic!("common observation must preserve native bootstrap errors");
        };
        assert_eq!(observed_error.toolchain, id);
        assert_eq!(observed_error.path, error.path);
        if id == ToolchainId::RUST {
            assert!(error.source.downcast_ref::<crate::cargo::Error>().is_some());
        } else if id == ToolchainId::PYTHON {
            assert!(error.source.downcast_ref::<crate::uv::Error>().is_some());
        } else {
            assert!(error.source.downcast_ref::<crate::go::Error>().is_some());
            assert!(error.to_string().contains("go.work"));
        }
    }

    #[test_case("unsupported ./member\n"; "unknown_directive")]
    #[test_case("use (\n ./member\n"; "unterminated_block")]
    #[test_case("use ./member ./other\n"; "extra_use_argument")]
    #[test_case("go 1.23\n"; "duplicate_go_directive")]
    #[test_case("use \"./member\n"; "unterminated_quote")]
    fn go_probe_rejects_malformed_root_without_reading_members(directive: &str) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let manifest = root.join_component(crate::go::GO_WORK);
        // The missing member must not hide a subsequent root syntax error.
        write(
            &root,
            crate::go::GO_WORK,
            &format!("go 1.22\nuse ./missing\n{directive}"),
        );
        let error = Registry::new([ToolchainId::GO]).probe(&root).unwrap_err();
        assert_eq!(error.toolchain, ToolchainId::GO);
        assert_eq!(error.path, manifest);
        assert!(matches!(
            error.source.downcast_ref::<crate::go::Error>(),
            Some(
                crate::go::Error::MalformedGoWork { .. }
                    | crate::go::Error::UnknownGoWorkDirective { .. }
            )
        ));
        assert!(Registry::default().probe(&root).unwrap().is_empty());
    }

    #[test_case("go 1.22\n"; "no_members")]
    #[test_case("go 1.22\nuse (\n)\n"; "empty_use_block")]
    #[test_case("go 1.22\nuse ./missing\n"; "missing_member")]
    fn go_probe_recognizes_root_before_member_validation(contents: &str) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        write(&root, crate::go::GO_WORK, contents);
        root.join_component("missing").create_dir_all().unwrap();
        let workspaces = Registry::new([ToolchainId::GO]).probe(&root).unwrap();
        assert_eq!(workspaces.len(), 1);
        assert!(workspaces[0].owns(&root).unwrap());
        let error = workspaces[0]
            .owns(&root.join_component("missing"))
            .unwrap_err();
        let toolchain::Error::Failed(native) =
            error.source.downcast_ref::<toolchain::Error>().unwrap()
        else {
            panic!("expected a retained native inventory error");
        };
        let native = native.downcast_ref::<crate::go::Error>().unwrap();
        assert!(matches!(
            native,
            crate::go::Error::EmptyWorkspace | crate::go::Error::MissingGoMod { .. }
        ));
    }

    #[test_case("[project]\nname = 42\n"; "project_type_error")]
    #[test_case("[tool.uv.workspace]\nmembers = [42]\n"; "member_type_error")]
    fn python_probe_preserves_native_root_parser_type_errors(contents: &str) {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        write(&root, crate::uv::PYPROJECT_TOML, contents);
        let error = Registry::new([ToolchainId::PYTHON])
            .probe(&root)
            .unwrap_err();
        assert_eq!(error.toolchain, ToolchainId::PYTHON);
        assert_eq!(error.path, root.join_component(crate::uv::PYPROJECT_TOML));
        assert!(matches!(
            error.source.downcast_ref::<crate::uv::Error>(),
            Some(crate::uv::Error::ManifestParse { .. })
        ));
    }

    #[test]
    fn flags_are_boolean_and_disabled_manifests_are_not_read() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        // Reading any disabled manifest would fail (they are directories).
        for manifest in ["Cargo.toml", "pyproject.toml", "go.work"] {
            std::fs::create_dir(root.join_component(manifest)).unwrap();
        }
        for value in [
            Value::Bool(false),
            Value::Null,
            Value::String("true".into()),
            Value::Number(1.into()),
        ] {
            let flags = [
                "experimentalCargoWorkspaces",
                "experimentalPythonWorkspaces",
                "experimentalGoWorkspaces",
            ]
            .into_iter()
            .map(|flag| (flag.to_string(), value.clone()))
            .collect();
            let registry = Registry::from_flags(&flags);
            assert_eq!(registry.enabled_ids().count(), 0);
            assert!(registry.probe(&root).unwrap().is_empty());
        }
        assert!(Registry::default().probe(&root).unwrap().is_empty());
        for (flag, id) in [
            ("experimentalCargoWorkspaces", ToolchainId::RUST),
            ("experimentalPythonWorkspaces", ToolchainId::PYTHON),
            ("experimentalGoWorkspaces", ToolchainId::GO),
        ] {
            let flags = [(flag.to_string(), Value::Bool(true))]
                .into_iter()
                .collect();
            let registry = Registry::from_flags(&flags);
            assert_eq!(registry.enabled_ids().collect::<Vec<_>>(), [id.clone()]);
            assert_eq!(registry.probe(&root).unwrap_err().toolchain, id);
        }
    }

    #[test]
    fn absent_and_non_workspace_manifests_do_not_recognize_a_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let registry = Registry::new([ToolchainId::RUST, ToolchainId::PYTHON, ToolchainId::GO]);
        assert!(registry.probe(&root).unwrap().is_empty());
        write(&root, "Cargo.toml", "[package]\nname = \"not-workspace\"\n");
        write(
            &root,
            "pyproject.toml",
            "[project]\nname = \"not-workspace\"\n",
        );
        assert!(registry.probe(&root).unwrap().is_empty());
    }

    #[test]
    fn all_enabled_native_roots_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let ids = [ToolchainId::RUST, ToolchainId::PYTHON, ToolchainId::GO];
        for id in &ids {
            fixture(&root, id);
        }
        let registry = Registry::new(ids.clone());
        assert_eq!(
            registry
                .probe(&root)
                .unwrap()
                .iter()
                .map(BootstrapWorkspace::id)
                .collect::<Vec<_>>(),
            ids
        );
    }

    struct FourthAdapter {
        inventories: Arc<AtomicUsize>,
    }

    impl ToolchainBootstrap for FourthAdapter {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("fourth")
        }

        fn probe(
            &self,
            dir: &AbsoluteSystemPath,
        ) -> Result<Option<BootstrapWorkspace>, BootstrapError> {
            Ok(Some(BootstrapWorkspace::new(
                dir.to_owned(),
                dir.join_component("fourth.workspace"),
                self.contributor(dir, ContributorOptions::default())
                    .unwrap(),
            )))
        }

        fn contributor(
            &self,
            dir: &AbsoluteSystemPath,
            _options: ContributorOptions,
        ) -> Option<Arc<dyn RepositoryContributor>> {
            Some(Arc::new(FourthContributor {
                root: dir.to_owned(),
                inventories: self.inventories.clone(),
            }))
        }
    }

    struct FourthContributor {
        root: AbsoluteSystemPathBuf,
        inventories: Arc<AtomicUsize>,
    }

    impl RepositoryContributor for FourthContributor {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("fourth")
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            panic!("bootstrap must never invoke full discovery")
        }

        fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
            self.inventories.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(DiscoveredPackageScopes::new(
                    vec![
                        DiscoveredPackageScope::new(
                            Some("member".into()),
                            AbsoluteSystemPathBuf::from_unknown(
                                &self.root,
                                "member/fourth.manifest",
                            ),
                        ),
                        DiscoveredPackageScope::new(
                            Some("aggregate".into()),
                            self.root.join_component("fourth.workspace"),
                        )
                        .into_aggregate(),
                    ],
                    vec![WorkspaceRoot::new("fourth", self.root.clone())],
                ))
            })
        }
    }

    struct GraphFourthAdapter {
        probes: Arc<AtomicUsize>,
        contributor: Arc<GraphFourthContributor>,
        factory_options: Arc<std::sync::Mutex<Vec<bool>>>,
    }

    impl ToolchainBootstrap for GraphFourthAdapter {
        fn id(&self) -> ToolchainId {
            self.contributor.id()
        }

        fn probe(
            &self,
            dir: &AbsoluteSystemPath,
        ) -> Result<Option<BootstrapWorkspace>, BootstrapError> {
            self.probes.fetch_add(1, Ordering::SeqCst);
            let path = dir.join_component("fourth.workspace");
            Ok(path
                .exists()
                .then(|| BootstrapWorkspace::new(dir.to_owned(), path, self.contributor.clone())))
        }

        fn contributor(
            &self,
            _dir: &AbsoluteSystemPath,
            options: ContributorOptions,
        ) -> Option<Arc<dyn RepositoryContributor>> {
            self.factory_options
                .lock()
                .unwrap()
                .push(options.resolve_external_dependencies);
            Some(self.contributor.clone())
        }
    }

    struct GraphFourthContributor {
        inventory: FourthContributor,
        full_calls: Arc<AtomicUsize>,
    }

    impl RepositoryContributor for GraphFourthContributor {
        fn id(&self) -> ToolchainId {
            self.inventory.id()
        }

        fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
            self.inventory.discover_package_scopes()
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            self.full_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                use crate::{
                    package_json::PackageJson,
                    toolchain::{DiscoveredPackage, DiscoveredPackages},
                };
                let root = &self.inventory.root;
                Ok(DiscoveredPackages::new(
                    vec![
                        DiscoveredPackage::package(
                            Some("member".into()),
                            PackageJson::default(),
                            AbsoluteSystemPathBuf::from_unknown(root, "member/fourth.manifest"),
                        )
                        .with_native_relationships(Vec::new())
                        .with_native_tasks(Vec::new()),
                        DiscoveredPackage::aggregate(
                            "aggregate".into(),
                            PackageJson::default(),
                            root.join_component("fourth.workspace"),
                        )
                        .with_native_relationships(Vec::new())
                        .with_native_tasks(Vec::new()),
                    ],
                    vec![WorkspaceRoot::new("fourth", root.clone())],
                ))
            })
        }
    }

    #[tokio::test]
    async fn shared_root_observation_builds_graph_once_without_javascript() {
        use crate::package_graph::{PackageGraphBuilder, PackageName};

        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        write(&root, "fourth.workspace", "member\n");
        write(&root, "member/fourth.manifest", "member\n");
        assert!(!root.join_component("package.json").exists());
        let inventories = Arc::new(AtomicUsize::new(0));
        let full_calls = Arc::new(AtomicUsize::new(0));
        let factory_options = Arc::new(std::sync::Mutex::new(Vec::new()));
        let probes = Arc::new(AtomicUsize::new(0));
        let registry = Registry::default().with_adapter(Arc::new(GraphFourthAdapter {
            probes: probes.clone(),
            contributor: Arc::new(GraphFourthContributor {
                inventory: FourthContributor {
                    root: root.clone(),
                    inventories: inventories.clone(),
                },
                full_calls: full_calls.clone(),
            }),
            factory_options: factory_options.clone(),
        }));
        let config_path = root.join_component("custom.jsonc");
        let observation = RepositoryBootstrap::new(registry)
            .with_config_path(config_path.clone())
            .observe(&root)
            .unwrap();
        assert!(observation.is_repository());
        assert!(observation.is_workspace());
        let contributor_plan = observation.contributor_plan();
        assert_eq!(contributor_plan.config_path(), Some(&*config_path));
        let (javascript, consumed_plan) = observation.into_graph_parts().unwrap();
        assert!(javascript.is_none());
        assert_eq!(consumed_plan.root(), contributor_plan.root());
        let workspace = &contributor_plan.native_workspaces()[0];
        assert!(
            workspace
                .owns(&AbsoluteSystemPathBuf::from_unknown(&root, "member/src"))
                .unwrap()
        );
        assert!(!workspace.owns(&root.join_component("unrelated")).unwrap());
        assert_eq!(inventories.load(Ordering::SeqCst), 1);
        assert_eq!(full_calls.load(Ordering::SeqCst), 0);
        assert!(factory_options.lock().unwrap().is_empty());

        // A plan must not recognize again, even if the root definition changes.
        std::fs::remove_file(root.join_component("fourth.workspace")).unwrap();
        let graph = PackageGraphBuilder::new_optional(&root, None)
            .without_external_dependencies()
            .with_bootstrap_plan(&contributor_plan)
            .build()
            .await
            .unwrap();
        let id = ToolchainId::new("fourth");
        for name in ["member", "aggregate"] {
            assert_eq!(graph.package_toolchain(&PackageName::from(name)), Some(&id));
        }
        assert!(!graph.has_unloaded_scopes());
        assert_eq!(full_calls.load(Ordering::SeqCst), 1);

        let (inventory_graph, mut plan) = PackageGraphBuilder::new_optional(&root, None)
            .with_bootstrap_plan(&consumed_plan)
            .without_external_dependencies()
            .build_lazy()
            .await
            .unwrap()
            .into_parts();
        for name in ["member", "aggregate"] {
            assert_eq!(
                inventory_graph.unloaded_scope_owner(&PackageName::from(name)),
                Some(&id)
            );
        }
        assert_eq!(full_calls.load(Ordering::SeqCst), 1);
        assert_eq!(inventories.load(Ordering::SeqCst), 2);
        let loaded = plan
            .load(&std::collections::HashSet::from([id.clone()]))
            .await
            .unwrap();
        assert_eq!(
            loaded.package_toolchain(&PackageName::from("member")),
            Some(&id)
        );
        assert!(!loaded.has_unloaded_scopes());
        assert_eq!(full_calls.load(Ordering::SeqCst), 2);
        // Reusing the snapshot must not retain a previous builder's options.
        PackageGraphBuilder::new_optional(&root, None)
            .with_bootstrap_plan(&contributor_plan)
            .build_lazy()
            .await
            .unwrap();
        assert_eq!(*factory_options.lock().unwrap(), [false, false, true]);
        assert_eq!(probes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fourth_adapter_needs_no_core_changes_and_inventory_is_lazy_and_shared() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp.path()).unwrap();
        let inventories = Arc::new(AtomicUsize::new(0));
        let registry = Registry::default().with_adapter(Arc::new(FourthAdapter {
            inventories: inventories.clone(),
        }));
        let workspaces = registry.probe(&root).unwrap();
        let workspace = &workspaces[0];
        assert_eq!(workspace.id(), ToolchainId::new("fourth"));
        assert!(workspace.owns(&root).unwrap());
        assert!(!workspace.owns(root.parent().unwrap()).unwrap());
        assert_eq!(inventories.load(Ordering::SeqCst), 0);
        assert!(workspace.owns(&root.join_component("member")).unwrap());
        assert!(
            workspace
                .clone()
                .owns(&AbsoluteSystemPathBuf::from_unknown(
                    &root,
                    "member/deep/src"
                ))
                .unwrap()
        );
        assert!(!workspace.owns(&root.join_component("unrelated")).unwrap());
        assert_eq!(inventories.load(Ordering::SeqCst), 1);
        assert_eq!(
            registry
                .contributor(
                    &ToolchainId::new("fourth"),
                    &root,
                    ContributorOptions::default()
                )
                .unwrap()
                .id(),
            ToolchainId::new("fourth")
        );
        let registry = registry.with_adapter(Arc::new(FourthAdapter { inventories }));
        assert_eq!(registry.enabled_ids().count(), 1);
    }
}
