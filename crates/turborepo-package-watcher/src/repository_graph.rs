#[cfg(test)]
use std::io::ErrorKind;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
#[cfg(test)]
use turborepo_repository::package_json;
use turborepo_repository::{
    bootstrap::{
        ContributorPlan, Registry, RepositoryBootstrap, RootObservation, RootObservationError,
    },
    package_graph::{self, PackageGraphBuilder},
    package_json::PackageJson,
    toolchain::ToolchainId,
};

#[derive(Clone, Default)]
pub struct RepositoryGraphFeatures {
    registry: Registry,
    // Compatibility for load_root_package_json -> configure callers. Explicit
    // run/watch paths move the observation directly instead. Each load revalidates.
    loaded_plans: Arc<Mutex<HashMap<AbsoluteSystemPathBuf, ContributorPlan>>>,
}

impl std::fmt::Debug for RepositoryGraphFeatures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepositoryGraphFeatures")
            .field(
                "enabled_ids",
                &self.registry.enabled_ids().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl RepositoryGraphFeatures {
    pub fn new(future_flags: &turborepo_turbo_json::FutureFlags) -> Self {
        let Ok(serde_json::Value::Object(flags)) = serde_json::to_value(future_flags) else {
            unreachable!("FutureFlags serializes to a JSON object");
        };
        Self {
            registry: Registry::from_flags(&flags),
            loaded_plans: Default::default(),
        }
    }

    pub fn cargo_enabled(&self) -> bool {
        self.registry
            .enabled_ids()
            .any(|id| id == ToolchainId::RUST)
    }

    pub fn python_enabled(&self) -> bool {
        self.registry
            .enabled_ids()
            .any(|id| id == ToolchainId::PYTHON)
    }

    pub fn go_enabled(&self) -> bool {
        self.registry.enabled_ids().any(|id| id == ToolchainId::GO)
    }

    /// Load the JavaScript root, validating every enabled native root as well.
    /// A missing package.json is valid only for a recognized native workspace;
    /// merely having a native manifest (or enabling its flag) is insufficient.
    #[allow(
        clippy::result_large_err,
        reason = "preserve the repository's typed package graph diagnostics"
    )]
    pub fn load_root_package_json(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<Option<PackageJson>, package_graph::Error> {
        // Invalidate before observing so failed revalidation cannot leave a
        // stale successful plan available to configure.
        self.loaded_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(repo_root);
        let (package_json, plan) = self.observe_root(repo_root)?.into_graph_parts()?;
        self.loaded_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(repo_root.to_owned(), plan);
        Ok(package_json)
    }

    /// Explicit generation boundary: revalidate roots using this feature set.
    /// Recreate the feature set when configuration flags change. The caller
    /// must use the returned plan, not independently probe again.
    #[allow(
        clippy::result_large_err,
        reason = "preserve the repository's typed package graph diagnostics"
    )]
    pub fn observe_root(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<RootObservation, package_graph::Error> {
        RepositoryBootstrap::new(self.registry.clone())
            .observe(repo_root)
            .map_err(|error| match error {
                RootObservationError::Bootstrap(error) => error.into(),
                RootObservationError::PackageJson(error) => error.into(),
            })
    }

    pub fn configure_with_plan<'a, T>(
        &self,
        builder: PackageGraphBuilder<'a, T>,
        plan: &ContributorPlan,
    ) -> PackageGraphBuilder<'a, T> {
        builder.with_bootstrap_plan(plan)
    }

    pub fn configure<'a, T>(
        &self,
        builder: PackageGraphBuilder<'a, T>,
    ) -> PackageGraphBuilder<'a, T> {
        let plans = self
            .loaded_plans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match plans.get(builder.repository_root()) {
            Some(plan) => builder.with_bootstrap_plan(plan),
            None => builder.with_bootstrap_registry(&self.registry),
        }
    }
}

#[cfg(test)]
mod tests {
    use turborepo_turbo_json::FutureFlags;

    use super::*;

    fn all_native_features() -> RepositoryGraphFeatures {
        RepositoryGraphFeatures::new(&FutureFlags {
            experimental_cargo_workspaces: true,
            experimental_python_workspaces: true,
            experimental_go_workspaces: true,
            ..FutureFlags::default()
        })
    }

    struct CountingBootstrap {
        probes: Arc<std::sync::atomic::AtomicUsize>,
        root: turbopath::AbsoluteSystemPathBuf,
    }

    impl turborepo_repository::bootstrap::ToolchainBootstrap for CountingBootstrap {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("fake")
        }
        fn probe(
            &self,
            root: &AbsoluteSystemPath,
        ) -> Result<
            Option<turborepo_repository::bootstrap::BootstrapWorkspace>,
            turborepo_repository::bootstrap::BootstrapError,
        > {
            self.probes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Some(
                turborepo_repository::bootstrap::BootstrapWorkspace::new(
                    root.to_owned(),
                    root.join_component("fake.workspace"),
                    Arc::new(EmptyContributor(self.root.clone())),
                ),
            ))
        }
        fn contributor(
            &self,
            _: &AbsoluteSystemPath,
            _: turborepo_repository::bootstrap::ContributorOptions,
        ) -> Option<Arc<dyn turborepo_repository::toolchain::RepositoryContributor>> {
            Some(Arc::new(EmptyContributor(self.root.clone())))
        }
    }

    struct EmptyContributor(turbopath::AbsoluteSystemPathBuf);
    impl turborepo_repository::toolchain::RepositoryContributor for EmptyContributor {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("fake")
        }
        fn discover_packages(&self) -> turborepo_repository::toolchain::DiscoverPackagesFuture<'_> {
            panic!("lazy construction must not invoke full discovery")
        }
        fn discover_package_scopes(
            &self,
        ) -> turborepo_repository::toolchain::DiscoverPackageScopesFuture<'_> {
            Box::pin(async {
                Ok(
                    turborepo_repository::toolchain::DiscoveredPackageScopes::new(
                        Vec::new(),
                        vec![turborepo_repository::toolchain::WorkspaceRoot::new(
                            "fake",
                            self.0.clone(),
                        )],
                    ),
                )
            })
        }
    }

    #[tokio::test]
    async fn optional_root_loading_and_configuration_share_one_recognition() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        let probes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let features = RepositoryGraphFeatures {
            registry: Registry::default().with_adapter(Arc::new(CountingBootstrap {
                probes: probes.clone(),
                root: root.to_owned(),
            })),
            ..RepositoryGraphFeatures::default()
        };
        // Neither a package.json nor a real native manifest is a prerequisite.
        let javascript = features.load_root_package_json(root).unwrap();
        assert!(javascript.is_none());
        features
            .clone()
            .configure(PackageGraphBuilder::new_optional(root, javascript))
            .build_lazy()
            .await
            .unwrap();
        assert_eq!(probes.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Loading again is an explicit generation boundary, not a stale cache hit.
        features.load_root_package_json(root).unwrap();
        assert_eq!(probes.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn typed_flags_enable_registry_ids() {
        let disabled = RepositoryGraphFeatures::default();
        assert!(!disabled.cargo_enabled() && !disabled.python_enabled() && !disabled.go_enabled());
        let enabled = all_native_features();
        assert!(enabled.cargo_enabled() && enabled.python_enabled() && enabled.go_enabled());
    }

    #[test]
    fn optional_root_requires_an_enabled_native_workspace() {
        for (manifest, contents) in [
            ("Cargo.toml", "[workspace]\nmembers = []\n"),
            ("pyproject.toml", "[tool.uv.workspace]\nmembers = []\n"),
            ("go.work", "go 1.22\nuse ()\n"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
            let enabled = all_native_features();
            assert!(matches!(
                enabled.load_root_package_json(root),
                Err(package_graph::Error::PackageJson(package_json::Error::Io(io)))
                    if io.kind() == ErrorKind::NotFound
            ));
            root.join_component(manifest)
                .create_with_contents(contents)
                .unwrap();
            assert!(enabled.load_root_package_json(root).unwrap().is_none());
            assert!(matches!(
                RepositoryGraphFeatures::default().load_root_package_json(root),
                Err(package_graph::Error::PackageJson(_))
            ));
        }
    }

    #[test]
    fn standalone_native_manifests_are_not_workspace_roots() {
        for (manifest, contents) in [
            (
                "Cargo.toml",
                "[package]\nname = \"standalone\"\nversion = \"0.1.0\"\n",
            ),
            (
                "pyproject.toml",
                "[project]\nname = \"standalone\"\nversion = \"0.1.0\"\n",
            ),
            ("go.mod", "module example.com/standalone\ngo 1.22\n"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
            root.join_component(manifest)
                .create_with_contents(contents)
                .unwrap();
            assert!(matches!(
                all_native_features().load_root_package_json(root),
                Err(package_graph::Error::PackageJson(_))
            ));
        }
    }

    #[tokio::test]
    async fn single_package_native_registry_keeps_lazy_inventory() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
        root.join_component("Cargo.toml")
            .create_with_contents(
                "[workspace]\nmembers = [\"member\"]\n[workspace.metadata]\nname = \
                 \"native-workspace\"\n",
            )
            .unwrap();
        let member = root.join_component("member");
        member.create_dir_all().unwrap();
        member
            .join_component("Cargo.toml")
            .create_with_contents("[package]\nname = \"native-lib\"\nversion = \"0.1.0\"\n")
            .unwrap();
        let features = all_native_features();
        let root_package_json = features.load_root_package_json(root).unwrap();
        let graph = features
            .configure(
                PackageGraphBuilder::new_optional(root, root_package_json)
                    .with_single_package_mode(true),
            )
            .build_lazy()
            .await
            .unwrap();
        for name in ["native-workspace", "native-lib"] {
            let package = package_graph::PackageName::from(name);
            assert_eq!(
                graph.graph().unloaded_scope_owner(&package),
                Some(&ToolchainId::RUST)
            );
        }
        assert!(!graph.graph().has_root_javascript_scope());
    }

    #[test]
    fn bootstrap_and_package_json_errors_keep_their_sources() {
        for (manifest, id) in [
            ("Cargo.toml", ToolchainId::RUST),
            ("pyproject.toml", ToolchainId::PYTHON),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = AbsoluteSystemPath::from_std_path(temp.path()).unwrap();
            let path = root.join_component(manifest);
            path.create_with_contents("[broken").unwrap();
            for javascript in [false, true] {
                if javascript {
                    root.join_component("package.json")
                        .create_with_contents("{}")
                        .unwrap();
                }
                let error = all_native_features()
                    .load_root_package_json(root)
                    .unwrap_err();
                let package_graph::Error::Bootstrap(error) = error else {
                    panic!("expected bootstrap error, got {error}");
                };
                assert_eq!(error.toolchain, id);
                assert_eq!(error.path, path);
                assert!(std::error::Error::source(&error).is_some());
            }
            assert!(
                RepositoryGraphFeatures::default()
                    .load_root_package_json(root)
                    .unwrap()
                    .is_some()
            );
            root.join_component("package.json")
                .create_with_contents("{")
                .unwrap();
            assert!(matches!(
                all_native_features().load_root_package_json(root),
                Err(package_graph::Error::PackageJson(_))
            ));
        }
    }
}
