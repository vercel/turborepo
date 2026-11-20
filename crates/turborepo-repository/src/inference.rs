use biome_json_parser::JsonParserOptions;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::json::deserialize_from_json_str;

#[cfg(test)]
use crate::bootstrap::javascript::JavaScriptBootstrap;
pub use crate::bootstrap::javascript::JavaScriptRoot;
use crate::{
    bootstrap::{BootstrapError, Registry, RepositoryBootstrap, RootObservation},
    discovery::select_turbo_config_path,
    package_json,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepoMode {
    SinglePackage,
    MultiPackage,
}

#[derive(Debug)]
pub struct RepoState {
    pub root: AbsoluteSystemPathBuf,
    pub mode: RepoMode,
    pub javascript: Option<JavaScriptRoot>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to find repository root containing {0}.")]
    NotFound(AbsoluteSystemPathBuf),
    #[error(transparent)]
    Bootstrap(#[from] BootstrapError),
    #[error(transparent)]
    PackageJson(#[from] package_json::Error),
    #[error("Unable to read repository configuration {path}: {reason}")]
    Config {
        path: AbsoluteSystemPathBuf,
        reason: String,
    },
    #[error("Invocation directory {cwd} is outside the selected root configuration {config}")]
    Outside {
        cwd: AbsoluteSystemPathBuf,
        config: AbsoluteSystemPathBuf,
    },
}

struct InferInfo(RootObservation);

impl InferInfo {
    fn repo_mode(&self) -> RepoMode {
        if self.0.is_workspace() {
            RepoMode::MultiPackage
        } else {
            RepoMode::SinglePackage
        }
    }

    fn is_workspace_root_of(&self, target: &AbsoluteSystemPath) -> Result<bool, Error> {
        Ok(self.0.owns(target)?)
    }

    fn at(
        path: &AbsoluteSystemPath,
        registry: &Registry,
        config_path: Option<&AbsoluteSystemPath>,
    ) -> Result<Option<Self>, Error> {
        let mut bootstrap = RepositoryBootstrap::new(registry.clone());
        if let Some(config_path) = config_path {
            bootstrap = bootstrap.with_config_path(config_path.to_owned());
        }
        let observation = bootstrap.observe(path).map_err(|error| match error {
            crate::bootstrap::RootObservationError::Bootstrap(error) => Error::Bootstrap(error),
            crate::bootstrap::RootObservationError::PackageJson(error) => Error::PackageJson(error),
        })?;
        Ok(observation.is_repository().then_some(Self(observation)))
    }
}

impl From<InferInfo> for RepoState {
    fn from(root: InferInfo) -> Self {
        let mode = root.repo_mode();
        Self {
            mode,
            root: root.0.root().to_owned(),
            javascript: root.0.into_javascript(),
        }
    }
}

impl RepoState {
    /// Infers `RepoState` from a reference path
    ///
    /// # Arguments
    ///
    /// * `reference_dir`: Turbo's invocation directory
    ///
    /// returns: Result<RepoState, Error>
    #[tracing::instrument(skip_all)]
    pub fn infer(reference_dir: &AbsoluteSystemPath) -> Result<Self, Error> {
        Self::infer_with_config(reference_dir, None)
    }

    /// Use the invocation-resolved configuration path, including custom
    /// filenames. Explicit configuration selects content and native opt-ins,
    /// not a root directory: candidates still follow ancestor membership and
    /// Git boundaries, regardless of where the configuration file lives.
    pub fn infer_with_config(
        reference_dir: &AbsoluteSystemPath,
        config_path: Option<&AbsoluteSystemPath>,
    ) -> Result<Self, Error> {
        Self::infer_with_registry(reference_dir, config_path, |_, flags| {
            Registry::from_flags(flags)
        })
    }

    fn infer_with_registry(
        reference_dir: &AbsoluteSystemPath,
        config_path: Option<&AbsoluteSystemPath>,
        registry_for: impl Fn(
            &AbsoluteSystemPath,
            &serde_json::Map<String, serde_json::Value>,
        ) -> Registry,
    ) -> Result<Self, Error> {
        // Explicit content is validated once and applies to every genuine
        // candidate. Never borrow flags from a candidate's default config.
        let selected_flags = config_path
            .map(|config| future_flags(reference_dir, Some(config)))
            .transpose()?;
        let mut root: Option<InferInfo> = None;
        for path in reference_dir.ancestors() {
            let flags = selected_flags
                .as_ref()
                .map_or_else(|| future_flags(path, None), |flags| Ok(flags.clone()));
            let candidate = match flags {
                Ok(flags) => {
                    let registry = registry_for(path, &flags);
                    InferInfo::at(path, &registry, config_path)?
                }
                Err(error) => {
                    let Some(current) = root.as_ref() else {
                        return Err(error);
                    };
                    // An invalid default config cannot enable native adapters.
                    // Determine JS ownership without borrowing any native flags:
                    // discarded ancestors must not poison a nearer package, but
                    // an owning workspace must retain its config diagnostic.
                    let javascript = InferInfo::at(path, &Registry::default(), None)?;
                    if let Some(candidate) = javascript
                        && candidate.is_workspace_root_of(current.0.root())?
                    {
                        return Err(error);
                    }
                    None
                }
            };
            if let Some(candidate) = candidate {
                let selected = match root {
                    Some(current) if !candidate.is_workspace_root_of(current.0.root())? => current,
                    _ => candidate,
                };
                // Preserve nearest independent-workspace behavior. An ancestor
                // may replace a standalone package only when it actually owns it.
                if selected.repo_mode() == RepoMode::MultiPackage {
                    return Ok(selected.into());
                }
                root = Some(selected);
            }
            // Never borrow a repository from outside a nested clone/worktree.
            if path.join_component(".git").exists() {
                break;
            }
        }
        root.map(Into::into)
            .ok_or_else(|| Error::NotFound(reference_dir.to_owned()))
    }
}

/// Read bootstrap flags from the selected configuration. Missing default config
/// and package-level `extends` configs make no native root claim. Invalid
/// config is an error, not evidence that native support was disabled.
fn future_flags(
    dir: &AbsoluteSystemPath,
    explicit: Option<&AbsoluteSystemPath>,
) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
    let path = match explicit {
        Some(path) => path.to_owned(),
        None => match select_turbo_config_path(
            dir,
            dir.join_component("turbo.json").exists(),
            dir.join_component("turbo.jsonc").exists(),
        )
        .map_err(|error| Error::Config {
            path: dir.to_owned(),
            reason: error.to_string(),
        })? {
            Some(path) => path,
            None => return Ok(Default::default()),
        },
    };
    let contents = path
        .read_existing_to_string()
        .map_err(|error| Error::Config {
            path: path.clone(),
            reason: error.to_string(),
        })?
        .ok_or_else(|| Error::Config {
            path: path.clone(),
            reason: "configuration does not exist".into(),
        })?;
    let (config, diagnostics) = deserialize_from_json_str::<serde_json::Value>(
        &contents,
        JsonParserOptions::default()
            .with_allow_comments()
            .with_allow_trailing_commas(),
        path.as_str(),
    );
    let mut config = config.ok_or_else(|| Error::Config {
        path: path.clone(),
        reason: format!("invalid JSON: {diagnostics:?}"),
    })?;
    if !config.is_object() {
        return Err(Error::Config {
            path,
            reason: "expected an object".into(),
        });
    }
    if explicit.is_none() && config.get("extends").is_some() {
        return Ok(Default::default());
    }
    match config.get_mut("futureFlags").map(serde_json::Value::take) {
        Some(serde_json::Value::Object(flags)) => Ok(flags),
        None => Ok(Default::default()),
        Some(_) => Err(Error::Config {
            path,
            reason: "futureFlags must be an object".into(),
        }),
    }
}

#[cfg(test)]
mod test {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use super::{Error, JavaScriptRoot, RepoMode, RepoState};
    use crate::{
        bootstrap::{BootstrapError, BootstrapWorkspace, Registry, ToolchainBootstrap},
        package_json::PackageJson,
        package_manager::{self, PackageManager},
        toolchain::{
            DiscoverPackageScopesFuture, DiscoverPackagesFuture, DiscoveredPackageScope,
            DiscoveredPackageScopes, RepositoryContributor, ToolchainId, WorkspaceRoot,
        },
    };

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
    }

    #[test]
    fn javascript_probe_distinguishes_absent_malformed_and_unreadable_manifests() {
        let (_tmp, root) = tmp_dir();
        assert!(super::JavaScriptBootstrap::probe(&root).unwrap().is_none());

        write(&root, "package.json", r#"{"name": "#);
        assert!(super::JavaScriptBootstrap::probe(&root).is_err());
        assert!(matches!(
            RepoState::infer(&root),
            Err(Error::PackageJson(_))
        ));

        std::fs::remove_file(root.join_component("package.json")).unwrap();
        root.join_component("package.json")
            .create_dir_all()
            .unwrap();
        assert!(super::JavaScriptBootstrap::probe(&root).is_err());
    }

    #[test]
    fn javascript_probe_retains_metadata_without_a_package_manager() {
        let (_tmp, root) = tmp_dir();
        write(
            &root,
            "package.json",
            r#"{"name":"standalone","workspaces":["packages/*"]}"#,
        );
        write(&root, "package-lock.json", "");
        let observation = super::JavaScriptBootstrap::probe(&root).unwrap().unwrap();
        assert_eq!(observation.root(), &*root);
        assert!(!observation.is_workspace());
        assert!(!observation.owns(&root.join_components(&["packages", "app"])));
        let metadata = observation.into_root();
        assert_eq!(
            metadata
                .package_json
                .name
                .as_ref()
                .map(|name| name.as_str()),
            Some("standalone")
        );
        assert!(matches!(
            metadata.package_manager,
            Err(package_manager::Error::MissingPackageManager)
        ));
    }

    #[test_case(r#"{"packageManager":"npm@10.0.0"}"#; "no workspaces")]
    #[test_case(r#"{"packageManager":"npm@10.0.0","workspaces":["["]}"#; "invalid workspace glob")]
    #[test_case(r#"{"packageManager":"pnpm@9.0.0"}"#; "missing pnpm workspace file")]
    fn javascript_probe_keeps_standalone_fallback_when_globs_are_unavailable(manifest: &str) {
        let (_tmp, root) = tmp_dir();
        write(&root, "package.json", manifest);
        let observation = super::JavaScriptBootstrap::probe(&root).unwrap().unwrap();
        assert!(!observation.is_workspace());
        assert!(!observation.owns(&root.join_components(&["packages", "app"])));
        assert!(observation.into_root().package_manager.is_ok());
        assert_eq!(
            RepoState::infer(&root).unwrap().mode,
            RepoMode::SinglePackage
        );
    }

    #[test_case("npm@10.0.0", false; "package json globs")]
    #[test_case("pnpm@9.0.0", true; "pnpm yaml globs")]
    fn javascript_observation_owns_only_declared_members(manager: &str, pnpm: bool) {
        let (_tmp, root) = tmp_dir();
        write(
            &root,
            "package.json",
            &format!(
                r#"{{"name":"root","packageManager":"{manager}","workspaces":["packages/*","!packages/excluded"]}}"#
            ),
        );
        if pnpm {
            write(
                &root,
                "pnpm-workspace.yaml",
                "packages:\n  - packages/*\n  - '!packages/excluded'\n",
            );
        }
        let observation = super::JavaScriptBootstrap::probe(&root).unwrap().unwrap();
        assert!(observation.is_workspace());
        assert!(observation.owns(&root.join_components(&["packages", "app"])));
        assert!(!observation.owns(&root.join_components(&["packages", "excluded"])));
        assert!(!observation.owns(&root.join_component("unrelated")));
        assert!(!observation.owns(&root.join_components(&["packages", "node_modules", "dep"])));
        assert!(!observation.owns(root.parent().unwrap()));
        assert_eq!(
            observation
                .into_root()
                .package_json
                .name
                .as_ref()
                .map(|name| name.as_str()),
            Some("root")
        );

        let member = root.join_components(&["packages", "app"]);
        write(&member, "package.json", r#"{"name":"app"}"#);
        let inferred = RepoState::infer(&member).unwrap();
        assert_eq!(inferred.root, root);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);
        let excluded = root.join_components(&["packages", "excluded"]);
        write(&excluded, "package.json", r#"{"name":"excluded"}"#);
        assert_eq!(RepoState::infer(&excluded).unwrap().root, excluded);
    }

    #[test]
    fn nested_workspace_keeps_nearest_selected_root() {
        let (_tmp, root) = tmp_dir();
        root.join_component("package.json")
            .create_with_contents(
                r#"{"name":"outer","packageManager":"npm@10.0.0","workspaces":["**"]}"#,
            )
            .unwrap();
        let inner = root.join_component("inner");
        inner.create_dir_all().unwrap();
        inner
            .join_component("package.json")
            .create_with_contents(
                r#"{"name":"inner","packageManager":"npm@10.0.0","workspaces":["packages/*"]}"#,
            )
            .unwrap();
        let member = inner.join_components(&["packages", "app"]);
        member.create_dir_all().unwrap();
        member
            .join_component("package.json")
            .create_with_contents(r#"{"name":"app"}"#)
            .unwrap();
        let src = member.join_component("src");
        src.create_dir_all().unwrap();
        for invocation in [&inner, &member, &src] {
            let inferred = RepoState::infer(invocation).unwrap();
            assert_eq!(inferred.root, inner);
            assert_eq!(inferred.mode, RepoMode::MultiPackage);
        }

        // Merely encountering a workspace isn't enough to stop: a standalone
        // package excluded from the inner workspace may belong to the outer one.
        let standalone = inner.join_component("standalone");
        standalone.create_dir_all().unwrap();
        standalone
            .join_component("package.json")
            .create_with_contents(r#"{"name":"standalone"}"#)
            .unwrap();
        assert_eq!(RepoState::infer(&standalone).unwrap().root, root);
    }

    #[test]
    fn test_repo_state_infer() {
        // Directory layout:
        // <tmp_dir>
        //   irrelevant/
        //   monorepo_root/
        //     package.json
        //     standalone/
        //       package.json
        //     standalone_monorepo/
        //       package.json
        //       packages/
        //         app-2/
        //     packages/
        //       app-1/
        //         package.json
        //         src/
        //   single_root/
        //     package.json
        //     src/
        let (_tmp, tmp_dir) = tmp_dir();
        let irrelevant = tmp_dir.join_component("irrelevant");
        irrelevant.create_dir_all().unwrap();
        let monorepo_root = tmp_dir.join_component("monorepo_root");
        let monorepo_pkg_json = monorepo_root.join_component("package.json");
        let monorepo_contents =
            "{\"workspaces\": [\"packages/*\"], \"packageManager\": \"npm@7.0.0\"}";
        monorepo_pkg_json.ensure_dir().unwrap();
        monorepo_pkg_json
            .create_with_contents(monorepo_contents)
            .unwrap();
        monorepo_root
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let app_1 = monorepo_root.join_components(&["packages", "app-1"]);
        let app_1_pkg_json = app_1.join_component("package.json");
        app_1_pkg_json.ensure_dir().unwrap();
        app_1_pkg_json
            .create_with_contents("{\"name\": \"app_1\"}")
            .unwrap();
        let app_1_src = app_1.join_component("src");
        app_1_src.create_dir_all().unwrap();

        let standalone = monorepo_root.join_component("standalone");
        let standalone_pkg_json = standalone.join_component("package.json");
        let standalone_contents = "{\"name\":\"standalone\"}";
        standalone_pkg_json.ensure_dir().unwrap();
        standalone_pkg_json
            .create_with_contents(standalone_contents)
            .unwrap();
        standalone
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let standalone_monorepo = monorepo_root.join_component("standalone_monorepo");
        let standalone_monorepo_package_json = standalone_monorepo.join_component("package.json");
        let standalone_monorepo_contents =
            "{\"workspaces\": [\"packages/*\"], \"packageManager\": \"npm@7.0.0\"}";
        let app_2 = standalone_monorepo.join_components(&["packages", "app-2"]);
        app_2.create_dir_all().unwrap();
        app_2
            .join_component("package.json")
            .create_with_contents("{\"name\":\"app-2\"}")
            .unwrap();
        standalone_monorepo_package_json
            .create_with_contents(standalone_monorepo_contents)
            .unwrap();
        standalone_monorepo
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let single_root = tmp_dir.join_component("single_root");
        let single_root_src = single_root.join_component("src");
        let single_root_contents = "{\"name\": \"single-root\"}";
        let single_root_package_json = single_root.join_component("package.json");
        single_root_src.create_dir_all().unwrap();
        single_root_package_json
            .create_with_contents(single_root_contents)
            .unwrap();
        single_root
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let javascript_repo = |root: &AbsoluteSystemPathBuf, mode, package_manager| {
            Some(RepoState {
                root: root.clone(),
                mode,
                javascript: Some(JavaScriptRoot {
                    package_json: PackageJson::load(&root.join_component("package.json")).unwrap(),
                    package_manager,
                }),
            })
        };
        let tests = [
            (&irrelevant, None),
            (
                &monorepo_root,
                javascript_repo(
                    &monorepo_root,
                    RepoMode::MultiPackage,
                    Ok(PackageManager::Npm),
                ),
            ),
            (
                &app_1,
                javascript_repo(
                    &monorepo_root,
                    RepoMode::MultiPackage,
                    Ok(PackageManager::Npm),
                ),
            ),
            (
                &app_1_src,
                javascript_repo(
                    &monorepo_root,
                    RepoMode::MultiPackage,
                    Ok(PackageManager::Npm),
                ),
            ),
            (
                &single_root,
                javascript_repo(
                    &single_root,
                    RepoMode::SinglePackage,
                    Err(package_manager::Error::MissingPackageManager),
                ),
            ),
            (
                &single_root_src,
                javascript_repo(
                    &single_root,
                    RepoMode::SinglePackage,
                    Err(package_manager::Error::MissingPackageManager),
                ),
            ),
            // Nested, technically not supported
            (
                &standalone,
                javascript_repo(
                    &standalone,
                    RepoMode::SinglePackage,
                    Err(package_manager::Error::MissingPackageManager),
                ),
            ),
            (
                &standalone_monorepo,
                javascript_repo(
                    &standalone_monorepo,
                    RepoMode::MultiPackage,
                    Ok(PackageManager::Npm),
                ),
            ),
            (
                &app_2,
                javascript_repo(
                    &standalone_monorepo,
                    RepoMode::MultiPackage,
                    Ok(PackageManager::Npm),
                ),
            ),
        ];
        for (reference_path, expected) in tests {
            let repo_state = RepoState::infer(reference_path);
            if let Some(expected) = expected {
                let repo_state = repo_state.expect("infer a repo");
                assert_eq!(repo_state.root, expected.root);
                assert_eq!(repo_state.mode, expected.mode);
                let actual_js = repo_state.javascript.expect("JavaScript root facts");
                let expected_js = expected.javascript.unwrap();
                assert_eq!(actual_js.package_json.name, expected_js.package_json.name);
                match (actual_js.package_manager, expected_js.package_manager) {
                    (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
                    (
                        Err(package_manager::Error::MissingPackageManager),
                        Err(package_manager::Error::MissingPackageManager),
                    ) => {}
                    (actual, expected) => panic!("expected {expected:?}, got {actual:?}"),
                }
            } else {
                assert!(matches!(repo_state, Err(Error::NotFound(_))));
            }
        }
    }

    #[test]
    fn test_missing_package_manager_does_not_infer_from_lockfile() {
        let (_tmp, tmp_dir) = tmp_dir();

        let monorepo_root = tmp_dir.join_component("monorepo_root");
        let monorepo_pkg_json = monorepo_root.join_component("package.json");
        let monorepo_contents = "{\"workspaces\": [\"packages/*\"]}";
        monorepo_pkg_json.ensure_dir().unwrap();
        monorepo_pkg_json
            .create_with_contents(monorepo_contents)
            .unwrap();
        monorepo_root
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let app_1 = monorepo_root.join_components(&["packages", "app-1"]);
        let app_1_pkg_json = app_1.join_component("package.json");
        app_1_pkg_json.ensure_dir().unwrap();
        app_1_pkg_json
            .create_with_contents("{\"name\": \"app_1\"}")
            .unwrap();

        let repo_state_from_root = RepoState::infer(&monorepo_root).unwrap();
        let repo_state_from_app = RepoState::infer(&app_1).unwrap();

        assert_eq!(&repo_state_from_root.root, &monorepo_root);
        assert_eq!(&repo_state_from_app.root, &app_1);
        assert_eq!(repo_state_from_root.mode, RepoMode::SinglePackage);
        assert_eq!(repo_state_from_app.mode, RepoMode::SinglePackage);
        assert!(matches!(
            repo_state_from_root
                .javascript
                .unwrap()
                .package_manager
                .unwrap_err(),
            package_manager::Error::MissingPackageManager
        ));
        assert!(matches!(
            repo_state_from_app
                .javascript
                .unwrap()
                .package_manager
                .unwrap_err(),
            package_manager::Error::MissingPackageManager
        ));
    }

    const CARGO_WORKSPACE: &str = "[workspace]\nmembers = [\"members/*\"]\nexclude = \
                                   [\"members/excluded\"]\n[workspace.metadata]\nname = \
                                   \"workspace\"\n";
    const UV_WORKSPACE: &str = "[tool.uv.workspace]\nmembers = [\"members/*\"]\nexclude = \
                                [\"members/excluded\"]\n[tool.turbo]\nname = \"workspace\"\n";

    /// Infers from a nested directory of a root whose package.json declares no
    /// JS workspaces, with the given root files.
    fn infer_native_root(files: &[(&str, &str)]) -> Result<RepoState, Error> {
        let (_tmp, root) = tmp_dir();
        root.join_component("package.json")
            .create_with_contents(r#"{"name":"root","packageManager":"npm@10.0.0"}"#)
            .unwrap();
        for (name, contents) in files {
            root.join_component(name)
                .create_with_contents(contents)
                .unwrap();
        }
        let nested = root.join_components(&["crates", "app"]);
        nested.create_dir_all().unwrap();
        let inferred = RepoState::infer(&nested)?;
        assert_eq!(inferred.root, root);
        Ok(inferred)
    }

    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
    ], RepoMode::MultiPackage ; "cargo workspace")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalPythonWorkspaces":true}}"#),
        ("pyproject.toml", UV_WORKSPACE),
    ], RepoMode::MultiPackage ; "uv workspace")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalGoWorkspaces":true}}"#),
        ("go.work", "go 1.22\n"),
    ], RepoMode::MultiPackage ; "go workspace")]
    #[test_case(&[
        (
            "turbo.jsonc",
            "{\n  // native crates\n  \"futureFlags\": {\n    /* opt in */\n    \
             \"experimentalCargoWorkspaces\": true,\n  },\n}\n",
        ),
        ("Cargo.toml", CARGO_WORKSPACE),
    ], RepoMode::MultiPackage ; "turbo jsonc with comments")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{}}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
        ("pyproject.toml", UV_WORKSPACE),
        ("go.work", "go 1.22\n"),
    ], RepoMode::SinglePackage ; "markers without flags")]
    #[test_case(&[
        ("Cargo.toml", CARGO_WORKSPACE),
        ("go.work", "go 1.22\n"),
    ], RepoMode::SinglePackage ; "markers without turbo json")]
    #[test_case(&[
        (
            "turbo.json",
            r#"{"futureFlags":{"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true}}"#,
        ),
    ], RepoMode::SinglePackage ; "flags without markers")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n"),
    ], RepoMode::SinglePackage ; "cargo package without workspace")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalPythonWorkspaces":true}}"#),
        ("pyproject.toml", "[project]\nname = \"root\"\n"),
    ], RepoMode::SinglePackage ; "pyproject without uv workspace")]
    fn native_workspace_root_mode(files: &[(&str, &str)], expected: RepoMode) {
        assert_eq!(infer_native_root(files).unwrap().mode, expected);
    }

    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
    ]; "malformed turbo json")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":[]}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
    ]; "invalid future flags")]
    #[test_case(&[("turbo.json", "[]")]; "non object config")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("turbo.jsonc", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
    ]; "ambiguous turbo config")]
    fn invalid_native_configuration_is_an_error(files: &[(&str, &str)]) {
        assert!(matches!(
            infer_native_root(files),
            Err(Error::Config { .. })
        ));
    }

    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", "[workspace\n"),
    ], ToolchainId::RUST, "Cargo.toml"; "malformed cargo toml")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalPythonWorkspaces":true}}"#),
        ("pyproject.toml", "[tool.uv.workspace\n"),
    ], ToolchainId::PYTHON, "pyproject.toml"; "malformed pyproject toml")]
    fn malformed_enabled_native_manifest_is_an_error(
        files: &[(&str, &str)],
        id: ToolchainId,
        manifest: &str,
    ) {
        let Error::Bootstrap(error) = infer_native_root(files).unwrap_err() else {
            panic!("expected a native diagnostic");
        };
        assert_eq!(error.toolchain, id);
        assert_eq!(error.path.file_name(), Some(manifest));
    }

    #[test]
    fn another_enabled_toolchain_does_not_suppress_a_malformed_manifest() {
        let Error::Bootstrap(error) = infer_native_root(&[
            (
                "turbo.json",
                r#"{"futureFlags":{"experimentalCargoWorkspaces":true,"experimentalGoWorkspaces":true}}"#,
            ),
            ("Cargo.toml", "[workspace"),
            ("go.work", "go 1.22\n"),
        ]).unwrap_err() else {
            panic!("expected the malformed Cargo manifest to fail inference");
        };
        assert_eq!(error.toolchain, ToolchainId::RUST);
        assert_eq!(error.path.file_name(), Some("Cargo.toml"));
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn malformed_native_member_is_not_suppressed_by_javascript(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        let member = root.join_components(&["members", "owned"]);
        write(&member, "package.json", r#"{"name":"member"}"#);
        let manifest = match id.as_str() {
            "rust" => "Cargo.toml",
            "python" => "pyproject.toml",
            "go" => "go.mod",
            _ => unreachable!(),
        };
        write(&member, manifest, "[");
        let Error::Bootstrap(error) = RepoState::infer(&member).unwrap_err() else {
            panic!("expected a native inventory diagnostic");
        };
        assert_eq!(error.toolchain, id);
    }

    fn write(root: &AbsoluteSystemPath, path: &str, contents: &str) {
        let path = AbsoluteSystemPathBuf::from_unknown(root, path);
        path.ensure_dir().unwrap();
        path.create_with_contents(contents).unwrap();
    }

    fn native_flag(id: &ToolchainId) -> &'static str {
        match id.as_str() {
            "rust" => "experimentalCargoWorkspaces",
            "python" => "experimentalPythonWorkspaces",
            "go" => "experimentalGoWorkspaces",
            other => panic!("no built-in flag for {other}"),
        }
    }

    fn write_native_config(root: &AbsoluteSystemPath, path: &str, id: &ToolchainId, enabled: bool) {
        write(
            root,
            path,
            &format!(r#"{{"futureFlags":{{"{}":{enabled}}}}}"#, native_flag(id)),
        );
    }

    fn write_native_package(root: &AbsoluteSystemPath, id: &ToolchainId, name: &str) {
        match id.as_str() {
            "rust" => write(
                root,
                "Cargo.toml",
                &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
            ),
            "python" => write(
                root,
                "pyproject.toml",
                &format!("[project]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
            ),
            "go" => write(
                root,
                "go.mod",
                &format!("module example.com/{name}\ngo 1.22\n"),
            ),
            other => panic!("no native fixture for {other}"),
        }
    }

    fn native_fixture(root: &AbsoluteSystemPath, id: &ToolchainId) {
        match id.as_str() {
            "rust" => write(root, "Cargo.toml", CARGO_WORKSPACE),
            "python" => write(root, "pyproject.toml", UV_WORKSPACE),
            "go" => write(root, "go.work", "go 1.22\nuse (\n ./members/owned\n)\n"),
            other => panic!("no native fixture for {other}"),
        }
        for member in ["owned", "excluded"] {
            write_native_package(&root.join_components(&["members", member]), id, member);
        }
        write_native_config(root, "turbo.json", id, true);
    }

    fn assert_native_root(
        invocation: &AbsoluteSystemPath,
        root: &AbsoluteSystemPath,
        javascript: bool,
    ) {
        let inferred = RepoState::infer(invocation).unwrap();
        assert_eq!(&*inferred.root, root);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);
        assert_eq!(inferred.javascript.is_some(), javascript);
        if let Some(js) = inferred.javascript {
            assert_eq!(
                js.package_json.name.as_deref().map(String::as_str),
                Some("irrelevant")
            );
            assert!(matches!(
                js.package_manager,
                Err(package_manager::Error::MissingPackageManager)
            ));
        }
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn native_root_without_package_json(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        let member = root.join_components(&["members", "owned"]);
        let src = member.join_component("src");
        src.create_dir_all().unwrap();
        for invocation in [&root, &member, &src] {
            assert_native_root(invocation, &root, false);
        }
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn irrelevant_root_package_json_does_not_change_native_root_or_scope(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        let owned = root.join_components(&["members", "owned"]);
        let excluded = root.join_components(&["members", "excluded"]);
        let unrelated = root.join_component("unrelated");
        for package in [&owned, &excluded, &unrelated] {
            write(package, "package.json", r#"{"name":"nested"}"#);
        }
        // A JS package forces ancestor membership checks, unlike a bare cwd.
        for with_javascript in [false, true] {
            if with_javascript {
                write(&root, "package.json", r#"{"name":"irrelevant"}"#);
            }
            assert_native_root(&root, &root, with_javascript);
            assert_native_root(&owned, &root, with_javascript);
            for standalone in [&excluded, &unrelated] {
                let inferred = RepoState::infer(standalone).unwrap();
                assert_eq!(&inferred.root, standalone);
                assert_eq!(inferred.mode, RepoMode::SinglePackage);
                assert_eq!(
                    inferred
                        .javascript
                        .unwrap()
                        .package_json
                        .name
                        .as_deref()
                        .map(String::as_str),
                    Some("nested")
                );
            }
        }
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn nested_member_package_json_selects_native_owner(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        let javascript = root.join_components(&["members", "owned", "web"]);
        write(
            &javascript,
            "package.json",
            r#"{"name":"web","packageManager":"npm@10.0.0"}"#,
        );
        let src = javascript.join_components(&["src", "deep"]);
        src.create_dir_all().unwrap();
        for invocation in [&javascript, &src] {
            assert_native_root(invocation, &root, false);
        }
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn explicit_custom_config_enables_native_inference(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        write_native_config(&root, "turbo.json", &id, false);
        write_native_config(&root, "custom.jsonc", &id, true);
        let config = root.join_component("custom.jsonc");
        let member = root.join_components(&["members", "owned"]);
        write(&member, "package.json", r#"{"name":"member"}"#);
        let src = member.join_component("src");
        src.create_dir_all().unwrap();
        assert_eq!(RepoState::infer(&src).unwrap().root, member);
        for invocation in [&root, &member, &src] {
            let inferred = RepoState::infer_with_config(invocation, Some(&config)).unwrap();
            assert_eq!(inferred.root, root);
            assert_eq!(inferred.mode, RepoMode::MultiPackage);
            assert!(inferred.javascript.is_none());
        }
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn custom_config_disabling_default_flag_prevents_native_ancestor_claim(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        write(&root, "package.json", r#"{"name":"irrelevant"}"#);
        write_native_config(&root, "custom.json", &id, false);
        let config = root.join_component("custom.json");
        let member = root.join_components(&["members", "owned"]);
        write(&member, "package.json", r#"{"name":"member"}"#);
        assert_native_root(&member, &root, true);
        for root_package_json in [true, false] {
            if !root_package_json {
                std::fs::remove_file(root.join_component("package.json")).unwrap();
            }
            let inferred = RepoState::infer_with_config(&member, Some(&config)).unwrap();
            assert_eq!(inferred.root, member);
            assert_eq!(inferred.mode, RepoMode::SinglePackage);
            assert_eq!(
                inferred
                    .javascript
                    .unwrap()
                    .package_json
                    .name
                    .as_deref()
                    .map(String::as_str),
                Some("member")
            );
        }
        std::fs::remove_file(member.join_component("package.json")).unwrap();
        assert!(matches!(
            RepoState::infer_with_config(&member, Some(&config)),
            Err(Error::NotFound(_))
        ));
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn displaced_config_preserves_native_membership(id: ToolchainId) {
        let (_tmp, outer) = tmp_dir();
        let root = outer.join_component("repository");
        native_fixture(&root, &id);
        root.join_component(".git").create_dir_all().unwrap();
        // Neither candidate defaults nor the config's location select the root.
        write(&root, "turbo.json", "{");
        let member = root.join_components(&["members", "owned"]);
        let excluded = root.join_components(&["members", "excluded"]);
        for package in [&member, &excluded] {
            write(package, "package.json", r#"{"name":"child"}"#);
        }
        for config in [
            root.join_components(&["configs", "selected.json"]),
            outer.join_component("selected.json"),
        ] {
            write_native_config(&outer, config.as_str(), &id, true);
            for invocation in [&root, &member] {
                let inferred = RepoState::infer_with_config(invocation, Some(&config)).unwrap();
                assert_eq!(inferred.root, root);
                assert_eq!(inferred.mode, RepoMode::MultiPackage);
            }
            let inferred = RepoState::infer_with_config(&excluded, Some(&config)).unwrap();
            assert_eq!(inferred.root, excluded);
            assert_eq!(inferred.mode, RepoMode::SinglePackage);
        }
    }

    #[test]
    fn explicit_config_does_not_cross_git_boundary() {
        let (_tmp, outer) = tmp_dir();
        native_fixture(&outer, &ToolchainId::RUST);
        let clone = outer.join_components(&["members", "owned", "clone"]);
        clone.join_component(".git").create_dir_all().unwrap();
        let config = outer.join_component("turbo.json");
        assert!(matches!(
            RepoState::infer_with_config(&clone, Some(&config)),
            Err(Error::NotFound(_))
        ));
        write(&clone, "package.json", r#"{"name":"clone"}"#);
        let inferred = RepoState::infer_with_config(&clone, Some(&config)).unwrap();
        assert_eq!(inferred.root, clone);
        assert_eq!(inferred.mode, RepoMode::SinglePackage);
    }

    #[test_case("{", false; "malformed_unselected")]
    #[test_case(r#"{"futureFlags":[]}"#, false; "invalid_flags_unselected")]
    #[test_case("[]", false; "non_object_unselected")]
    #[test_case("{", true; "malformed_owning")]
    #[test_case(r#"{"futureFlags":[]}"#, true; "invalid_flags_owning")]
    fn ancestor_config_diagnostic_requires_ownership(config: &str, owns: bool) {
        let (_tmp, outer) = tmp_dir();
        outer.join_component(".git").create_dir_all().unwrap();
        write(
            &outer,
            "package.json",
            if owns {
                r#"{"name":"outer","packageManager":"npm@10.0.0","workspaces":["child"]}"#
            } else {
                r#"{"name":"outer","packageManager":"npm@10.0.0"}"#
            },
        );
        write(&outer, "turbo.json", config);
        let child = outer.join_component("child");
        write(
            &child,
            "package.json",
            r#"{"name":"child","packageManager":"npm@10.0.0"}"#,
        );
        write(&child, "turbo.json", r#"{"tasks":{}}"#);
        let src = child.join_component("src");
        src.create_dir_all().unwrap();
        for invocation in [&child, &src] {
            let inferred = RepoState::infer(invocation);
            if owns {
                assert!(
                    matches!(inferred, Err(Error::Config { path, .. }) if path == outer.join_component("turbo.json"))
                );
            } else {
                let inferred = inferred.unwrap();
                assert_eq!(inferred.root, child);
                assert_eq!(inferred.mode, RepoMode::SinglePackage);
            }
        }
        // Even an unrelated parent's config is a hard error when it is selected.
        assert!(matches!(
            RepoState::infer(&outer),
            Err(Error::Config { .. })
        ));
    }

    #[test]
    fn invalid_explicit_config_is_not_discarded_with_ancestors() {
        let (_tmp, root) = tmp_dir();
        root.join_component(".git").create_dir_all().unwrap();
        write(&root, "package.json", r#"{"name":"root"}"#);
        write(&root, "configs/selected.json", "{");
        assert!(matches!(
            RepoState::infer_with_config(
                &root,
                Some(&root.join_components(&["configs", "selected.json"]))
            ),
            Err(Error::Config { .. })
        ));
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn disabled_adapter_does_not_claim_or_read_native_manifests(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        native_fixture(&root, &id);
        write_native_config(&root, "turbo.json", &id, false);
        write(&root, "package.json", r#"{"name":"irrelevant"}"#);
        let member = root.join_components(&["members", "owned"]);
        write(&member, "package.json", r#"{"name":"member"}"#);
        // If the disabled adapter probes this directory it will fail to read it.
        let manifest = match id.as_str() {
            "rust" => "Cargo.toml",
            "python" => "pyproject.toml",
            "go" => "go.work",
            _ => unreachable!(),
        };
        let manifest = root.join_component(manifest);
        std::fs::remove_file(&manifest).unwrap();
        manifest.create_dir_all().unwrap();
        for (invocation, expected) in [(&root, &root), (&member, &member)] {
            let inferred = RepoState::infer(invocation).unwrap();
            assert_eq!(&inferred.root, expected);
            assert_eq!(inferred.mode, RepoMode::SinglePackage);
            assert!(inferred.javascript.is_some());
        }
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn standalone_native_package_is_not_a_workspace_root(id: ToolchainId) {
        let (_tmp, root) = tmp_dir();
        root.join_component(".git").create_dir_all().unwrap();
        write_native_package(&root, &id, "standalone");
        write_native_config(&root, "turbo.json", &id, true);
        let src = root.join_component("src");
        src.create_dir_all().unwrap();
        for invocation in [&root, &src] {
            assert!(matches!(
                RepoState::infer(invocation),
                Err(Error::NotFound(_))
            ));
        }
        // JavaScript can still identify a single package, not a native workspace.
        write(&root, "package.json", r#"{"name":"irrelevant"}"#);
        let inferred = RepoState::infer(&src).unwrap();
        assert_eq!(inferred.root, root);
        assert_eq!(inferred.mode, RepoMode::SinglePackage);
        assert!(inferred.javascript.is_some());
    }

    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn nested_independent_native_workspace_keeps_nearest_root(id: ToolchainId) {
        let (_tmp, outer) = tmp_dir();
        native_fixture(&outer, &id);
        write(
            &outer,
            "package.json",
            r#"{"name":"outer","packageManager":"npm@10.0.0","workspaces":["**"]}"#,
        );
        let inner = outer.join_components(&["members", "owned", "independent"]);
        native_fixture(&inner, &id);
        let member = inner.join_components(&["members", "owned"]);
        write(&member, "package.json", r#"{"name":"member"}"#);
        let src = member.join_component("src");
        src.create_dir_all().unwrap();
        for invocation in [&inner, &member, &src] {
            assert_native_root(invocation, &inner, false);
        }
    }

    #[test_case(ToolchainId::RUST, false; "rust_clone")]
    #[test_case(ToolchainId::PYTHON, false; "python_clone")]
    #[test_case(ToolchainId::GO, false; "go_clone")]
    #[test_case(ToolchainId::RUST, true; "rust_worktree")]
    #[test_case(ToolchainId::PYTHON, true; "python_worktree")]
    #[test_case(ToolchainId::GO, true; "go_worktree")]
    fn git_boundary_prevents_borrowing_a_native_ancestor(id: ToolchainId, worktree: bool) {
        let (_tmp, outer) = tmp_dir();
        native_fixture(&outer, &id);
        let clone = outer.join_components(&["members", "owned", "clone"]);
        let src = clone.join_component("src");
        src.create_dir_all().unwrap();
        if worktree {
            write(&clone, ".git", "gitdir: /unused/worktree\n");
        } else {
            clone.join_component(".git").create_dir_all().unwrap();
        }
        assert!(matches!(RepoState::infer(&src), Err(Error::NotFound(_))));
        write(&clone, "package.json", r#"{"name":"clone"}"#);
        let inferred = RepoState::infer(&src).unwrap();
        assert_eq!(inferred.root, clone);
        assert_eq!(inferred.mode, RepoMode::SinglePackage);
        assert_eq!(
            inferred
                .javascript
                .unwrap()
                .package_json
                .name
                .as_deref()
                .map(String::as_str),
            Some("clone")
        );
    }

    #[cfg(unix)]
    #[test_case(ToolchainId::RUST; "rust")]
    #[test_case(ToolchainId::PYTHON; "python")]
    #[test_case(ToolchainId::GO; "go")]
    fn native_inference_does_not_spawn_toolchain_processes(id: ToolchainId) {
        use std::os::unix::fs::PermissionsExt;

        const TRAP: &str = "TURBO_INFERENCE_TOOLCHAIN_TRAP";
        if std::env::var_os(TRAP).is_some() {
            // Run in an isolated test process: never mutate the parallel test
            // harness's PATH. Root inference and ownership must both stay cheap.
            let (_tmp, root) = tmp_dir();
            native_fixture(&root, &id);
            assert_native_root(&root, &root, false);
            let member = root.join_components(&["members", "owned"]);
            write(&member, "package.json", r#"{"name":"member"}"#);
            let src = member.join_component("src");
            src.create_dir_all().unwrap();
            assert_native_root(&src, &root, false);
            return;
        }

        let (_tmp, traps) = tmp_dir();
        let marker = traps.join_component("invoked");
        for program in ["cargo", "rustc", "uv", "python", "python3", "go", "which"] {
            write(
                &traps,
                program,
                "#!/bin/sh\nprintf '%s\\n' \"$0\" >> \"$TURBO_INFERENCE_TOOLCHAIN_TRAP\"\nexit \
                 99\n",
            );
            std::fs::set_permissions(
                traps.join_component(program),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                std::thread::current().name().unwrap(),
                "--nocapture",
            ])
            .env("PATH", traps.as_str())
            .env(TRAP, marker.as_str())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child inference failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "child must run the exact regression, not an empty filter"
        );
        assert!(!marker.exists(), "inference invoked a native toolchain");
    }

    struct FourthBootstrap {
        inventories: Arc<AtomicUsize>,
    }

    impl ToolchainBootstrap for FourthBootstrap {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("fourth")
        }

        fn probe(
            &self,
            dir: &AbsoluteSystemPath,
        ) -> Result<Option<BootstrapWorkspace>, BootstrapError> {
            let manifest = dir.join_component("fourth.workspace");
            if !manifest.exists() {
                return Ok(None);
            }
            Ok(Some(BootstrapWorkspace::new(
                dir.to_owned(),
                manifest,
                Arc::new(FourthContributor {
                    root: dir.to_owned(),
                    inventories: self.inventories.clone(),
                }),
            )))
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
            panic!("ancestor inference must not invoke full discovery")
        }

        fn discover_package_scopes(&self) -> DiscoverPackageScopesFuture<'_> {
            self.inventories.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(DiscoveredPackageScopes::new(
                    vec![
                        DiscoveredPackageScope::new(
                            Some("owned".into()),
                            self.root
                                .join_components(&["members", "owned", "fourth.manifest"]),
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

    #[test]
    fn fourth_adapter_participates_in_ancestor_inference_without_core_switches() {
        let (_tmp, root) = tmp_dir();
        root.join_component(".git").create_dir_all().unwrap();
        write(&root, "fourth.workspace", "workspace");
        write(
            &root,
            "turbo.json",
            r#"{"futureFlags":{"experimentalFourthWorkspaces":true}}"#,
        );
        let javascript = root.join_components(&["members", "owned", "web"]);
        write(&javascript, "package.json", r#"{"name":"web"}"#);
        let src = javascript.join_component("src");
        src.create_dir_all().unwrap();
        let inventories = Arc::new(AtomicUsize::new(0));
        let visited = std::cell::RefCell::new(Vec::new());
        let registry_for =
            |dir: &AbsoluteSystemPath, flags: &serde_json::Map<String, serde_json::Value>| {
                visited.borrow_mut().push(dir.to_owned());
                let registry = Registry::from_flags(flags);
                if flags
                    .get("experimentalFourthWorkspaces")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                {
                    registry.with_adapter(Arc::new(FourthBootstrap {
                        inventories: inventories.clone(),
                    }))
                } else {
                    registry
                }
            };

        let inferred = RepoState::infer_with_registry(&root, None, registry_for).unwrap();
        assert_eq!(inferred.root, root);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);
        assert!(inferred.javascript.is_none());
        assert_eq!(
            inventories.load(Ordering::SeqCst),
            0,
            "root probing is lazy"
        );

        visited.borrow_mut().clear();
        let inferred = RepoState::infer_with_registry(&src, None, registry_for).unwrap();
        assert_eq!(inferred.root, root);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);
        assert!(inferred.javascript.is_none());
        assert_eq!(
            inventories.load(Ordering::SeqCst),
            1,
            "membership uses scope inventory"
        );
        assert_eq!(visited.borrow().first(), Some(&src));
        assert_eq!(visited.borrow().last(), Some(&root));
        assert!(visited.borrow().contains(&javascript));

        // An aggregate at the root must not claim arbitrary descendant packages.
        let unrelated = root.join_component("unrelated");
        write(&unrelated, "package.json", r#"{"name":"unrelated"}"#);
        let inferred = RepoState::infer_with_registry(&unrelated, None, registry_for).unwrap();
        assert_eq!(inferred.root, unrelated);
        assert_eq!(inferred.mode, RepoMode::SinglePackage);
        assert_eq!(inventories.load(Ordering::SeqCst), 2);

        write(
            &root,
            "custom.json",
            r#"{"futureFlags":{"experimentalFourthWorkspaces":true}}"#,
        );
        let config = root.join_component("custom.json");
        let inferred = RepoState::infer_with_registry(&src, Some(&config), registry_for).unwrap();
        assert_eq!(inferred.root, root);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);
        assert_eq!(inventories.load(Ordering::SeqCst), 3);

        write(
            &root,
            "turbo.json",
            r#"{"futureFlags":{"experimentalFourthWorkspaces":false}}"#,
        );
        let inferred = RepoState::infer_with_registry(&src, None, registry_for).unwrap();
        assert_eq!(inferred.root, javascript);
        assert_eq!(inferred.mode, RepoMode::SinglePackage);
        assert_eq!(
            inventories.load(Ordering::SeqCst),
            3,
            "disabled adapter stays untouched"
        );
    }

    #[test]
    fn native_workspace_root_stops_ancestor_walk() {
        // A native multi-package root is selected like a JS one: an outer JS
        // workspace that globs over it can't replace it.
        let (_tmp, outer) = tmp_dir();
        outer
            .join_component("package.json")
            .create_with_contents(
                r#"{"name":"outer","packageManager":"npm@10.0.0","workspaces":["**"]}"#,
            )
            .unwrap();
        let inner = outer.join_component("inner");
        inner.create_dir_all().unwrap();
        inner
            .join_component("package.json")
            .create_with_contents(r#"{"name":"inner","packageManager":"npm@10.0.0"}"#)
            .unwrap();
        inner
            .join_component("Cargo.toml")
            .create_with_contents(CARGO_WORKSPACE)
            .unwrap();
        let turbo_json = inner.join_component("turbo.json");
        turbo_json
            .create_with_contents(r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#)
            .unwrap();
        let inferred = RepoState::infer(&inner).unwrap();
        assert_eq!(inferred.root, inner);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);

        // Without the flag, the inner root is a JS package of the outer workspace.
        turbo_json
            .create_with_contents(r#"{"futureFlags":{}}"#)
            .unwrap();
        let inferred = RepoState::infer(&inner).unwrap();
        assert_eq!(inferred.root, outer);
        assert_eq!(inferred.mode, RepoMode::MultiPackage);
    }

    #[test]
    fn test_gh_8599() {
        // Test that workspace globs with leading "./" are properly handled
        // See https://github.com/vercel/turborepo/issues/8599
        let (_tmp, tmp_dir) = tmp_dir();
        let monorepo_root = tmp_dir.join_component("monorepo_root");
        let monorepo_pkg_json = monorepo_root.join_component("package.json");
        monorepo_pkg_json.ensure_dir().unwrap();
        monorepo_pkg_json.create_with_contents(r#"{"name": "mono", "packageManager": "npm@10.2.4", "workspaces": ["./packages/*"]}"#.as_bytes()).unwrap();
        let package_foo = monorepo_root.join_components(&["packages", "foo"]);
        let foo_package_json = package_foo.join_component("package.json");
        foo_package_json.ensure_dir().unwrap();
        foo_package_json
            .create_with_contents(r#"{"name": "foo"}"#.as_bytes())
            .unwrap();

        let repo_state = RepoState::infer(&package_foo).unwrap();
        assert_eq!(repo_state.root, monorepo_root);
        assert_eq!(repo_state.mode, RepoMode::MultiPackage);
    }
}
