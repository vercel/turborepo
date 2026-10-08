use std::cell::OnceCell;

use biome_json_parser::JsonParserOptions;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::json::deserialize_from_json_str;

use crate::{
    discovery::select_turbo_config_path,
    package_json::PackageJson,
    package_manager::{self, PackageManager},
    workspaces::WorkspaceGlobs,
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
    pub root_package_json: PackageJson,
    pub package_manager: Result<PackageManager, package_manager::Error>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to find repository root containing {0}.")]
    NotFound(AbsoluteSystemPathBuf),
}

#[derive(Debug)]
struct InferInfo {
    path: AbsoluteSystemPathBuf,
    workspace_globs: Option<WorkspaceGlobs>,
    package_manager: Result<PackageManager, package_manager::Error>,
    package_json: PackageJson,
    // Computed on first use: a candidate without JS workspaces reads its turbo
    // config to decide, which candidates that are never selected don't need.
    mode: OnceCell<RepoMode>,
}

impl InferInfo {
    fn repo_mode(&self) -> RepoMode {
        *self.mode.get_or_init(|| {
            if self.workspace_globs.is_some() || has_enabled_native_workspace(&self.path) {
                RepoMode::MultiPackage
            } else {
                RepoMode::SinglePackage
            }
        })
    }

    pub fn is_workspace_root_of(&self, target_path: &AbsoluteSystemPath) -> bool {
        match &self.workspace_globs {
            Some(globs) => globs
                .target_is_workspace(&self.path, target_path)
                .unwrap_or(false),
            None => false,
        }
    }
}

impl From<InferInfo> for RepoState {
    fn from(root: InferInfo) -> Self {
        Self {
            mode: root.repo_mode(),
            package_manager: root.package_manager,
            root: root.path,
            root_package_json: root.package_json,
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
        let candidates = reference_dir.ancestors().filter_map(|path| {
            PackageJson::load(&path.join_component("package.json"))
                .ok()
                .map(|package_json| {
                    let package_manager =
                        PackageManager::read_or_detect_package_manager(&package_json, path);
                    let workspace_globs = package_manager
                        .as_ref()
                        .ok()
                        .and_then(|mgr| mgr.get_workspace_globs(path).ok());

                    InferInfo {
                        path: path.to_owned(),
                        workspace_globs,
                        package_manager,
                        package_json,
                        mode: OnceCell::new(),
                    }
                })
        });
        let mut root: Option<InferInfo> = None;
        for candidate in candidates {
            let selected = match root {
                Some(current) if !candidate.is_workspace_root_of(&current.path) => current,
                _ => candidate,
            };
            // Once a multi-package root is selected, no ancestor can replace
            // it. Stop here rather than loading manifests and compiling globs
            // for outer repositories whose results would be discarded.
            if selected.repo_mode() == RepoMode::MultiPackage {
                return Ok(selected.into());
            }
            root = Some(selected);
        }
        root.map(Into::into)
            .ok_or_else(|| Error::NotFound(reference_dir.to_owned()))
    }
}

/// Which experimental native workspace toolchains a root turbo config enables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeWorkspaceFlags {
    pub cargo: bool,
    pub python: bool,
    pub go: bool,
}

/// A native workspace manifest that could not be read or parsed.
#[derive(Debug, Error)]
#[error("{reason}")]
pub struct NativeWorkspaceManifestError {
    pub path: AbsoluteSystemPathBuf,
    pub reason: String,
}

/// The manifest in `dir` that roots a native workspace whose toolchain is
/// enabled: a `Cargo.toml` with a `workspace` table, a `pyproject.toml` with
/// `tool.uv.workspace`, or a `go.work`. Manifests are only read for enabled
/// toolchains. Callers decide whether an unreadable manifest is an error.
pub fn native_workspace_manifest(
    dir: &AbsoluteSystemPath,
    flags: NativeWorkspaceFlags,
) -> Result<Option<&'static str>, NativeWorkspaceManifestError> {
    let toml_manifest = |name| -> Result<Option<toml::Value>, NativeWorkspaceManifestError> {
        let path = dir.join_component(name);
        let error = |reason: String| NativeWorkspaceManifestError {
            path: path.clone(),
            reason,
        };
        path.read_existing_to_string()
            .map_err(|e| error(e.to_string()))?
            .map(|contents| toml::from_str(&contents).map_err(|e| error(e.to_string())))
            .transpose()
    };
    if flags.cargo
        && toml_manifest("Cargo.toml")?.is_some_and(|cargo| cargo.get("workspace").is_some())
    {
        return Ok(Some("Cargo.toml"));
    }
    if flags.python
        && toml_manifest("pyproject.toml")?.is_some_and(|python| {
            python
                .get("tool")
                .and_then(|tool| tool.get("uv"))
                .and_then(|uv| uv.get("workspace"))
                .is_some()
        })
    {
        return Ok(Some("pyproject.toml"));
    }
    if flags.go && dir.join_component("go.work").exists() {
        return Ok(Some("go.work"));
    }
    Ok(None)
}

/// Whether `dir` holds a native workspace manifest whose toolchain is enabled
/// by the future flags of the turbo config in `dir`.
///
/// A missing, unreadable, or malformed file counts as not enabled; the
/// turbo.json loader reports config errors once the run starts.
fn has_enabled_native_workspace(dir: &AbsoluteSystemPath) -> bool {
    let Some(flags) = future_flags(dir) else {
        return false;
    };
    let enabled = |flag| {
        flags
            .get(flag)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    // Ask per toolchain so one malformed manifest doesn't hide another.
    [
        NativeWorkspaceFlags {
            cargo: enabled("experimentalCargoWorkspaces"),
            ..Default::default()
        },
        NativeWorkspaceFlags {
            python: enabled("experimentalPythonWorkspaces"),
            ..Default::default()
        },
        NativeWorkspaceFlags {
            go: enabled("experimentalGoWorkspaces"),
            ..Default::default()
        },
    ]
    .into_iter()
    .any(|flags| matches!(native_workspace_manifest(dir, flags), Ok(Some(_))))
}

/// Reads only the `futureFlags` object of the turbo config in `dir`, without
/// depending on the full turbo.json schema.
fn future_flags(dir: &AbsoluteSystemPath) -> Option<serde_json::Map<String, serde_json::Value>> {
    let path = select_turbo_config_path(
        dir,
        dir.join_component("turbo.json").exists(),
        dir.join_component("turbo.jsonc").exists(),
    )
    .ok()??;
    let contents = path.read_existing_to_string().ok()??;
    let (config, _) = deserialize_from_json_str::<serde_json::Value>(
        &contents,
        JsonParserOptions::default()
            .with_allow_comments()
            .with_allow_trailing_commas(),
        path.as_str(),
    );
    match config?.get_mut("futureFlags")?.take() {
        serde_json::Value::Object(flags) => Some(flags),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{NativeWorkspaceFlags, RepoMode, RepoState, native_workspace_manifest};
    use crate::{package_json::PackageJson, package_manager, package_manager::PackageManager};

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
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

        let pnpm = PackageManager::Pnpm;
        let tests = [
            (&irrelevant, None),
            (
                &monorepo_root,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&monorepo_pkg_json).unwrap(),
                }),
            ),
            (
                &app_1,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&monorepo_pkg_json).unwrap(),
                }),
            ),
            (
                &app_1_src,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&monorepo_pkg_json).unwrap(),
                }),
            ),
            (
                &single_root,
                Some(RepoState {
                    root: single_root.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&single_root_package_json).unwrap(),
                }),
            ),
            (
                &single_root_src,
                Some(RepoState {
                    root: single_root.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&single_root_package_json).unwrap(),
                }),
            ),
            // Nested, technically not supported
            (
                &standalone,
                Some(RepoState {
                    root: standalone.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&standalone_pkg_json).unwrap(),
                }),
            ),
            (
                &standalone_monorepo,
                Some(RepoState {
                    root: standalone_monorepo.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm.clone()),
                    root_package_json: PackageJson::load(&standalone_monorepo_package_json)
                        .unwrap(),
                }),
            ),
            (
                &app_2,
                Some(RepoState {
                    root: standalone_monorepo.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&standalone_monorepo_package_json)
                        .unwrap(),
                }),
            ),
        ];
        for (reference_path, expected) in tests {
            let repo_state = RepoState::infer(reference_path);
            if let Some(expected) = expected {
                let repo_state = repo_state.expect("infer a repo");
                assert_eq!(repo_state.root, expected.root);
                assert_eq!(repo_state.mode, expected.mode);
            } else {
                assert!(repo_state.is_err(), "Expected to fail inference");
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
            repo_state_from_root.package_manager.unwrap_err(),
            package_manager::Error::MissingPackageManager
        ));
        assert!(matches!(
            repo_state_from_app.package_manager.unwrap_err(),
            package_manager::Error::MissingPackageManager
        ));
    }

    const CARGO_WORKSPACE: &str = "[workspace]\nmembers = [\"crates/*\"]\n";
    const UV_WORKSPACE: &str =
        "[project]\nname = \"root\"\n\n[tool.uv.workspace]\nmembers = [\"packages/*\"]\n";

    /// Infers from a nested directory of a root whose package.json declares no
    /// JS workspaces, with the given root files.
    fn infer_native_root(files: &[(&str, &str)]) -> RepoMode {
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
        let inferred = RepoState::infer(&nested).unwrap();
        assert_eq!(inferred.root, root);
        inferred.mode
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
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
    ], RepoMode::SinglePackage ; "malformed turbo json")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n"),
    ], RepoMode::SinglePackage ; "cargo package without workspace")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", "[workspace\n"),
    ], RepoMode::SinglePackage ; "malformed cargo toml")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalPythonWorkspaces":true}}"#),
        ("pyproject.toml", "[project]\nname = \"root\"\n"),
    ], RepoMode::SinglePackage ; "pyproject without uv workspace")]
    #[test_case(&[
        ("turbo.json", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("turbo.jsonc", r#"{"futureFlags":{"experimentalCargoWorkspaces":true}}"#),
        ("Cargo.toml", CARGO_WORKSPACE),
    ], RepoMode::SinglePackage ; "ambiguous turbo config")]
    fn native_workspace_root_mode(files: &[(&str, &str)], expected: RepoMode) {
        assert_eq!(infer_native_root(files), expected);
    }

    #[test]
    fn native_workspace_manifest_is_gated_by_flags_and_strict_about_enabled_manifests() {
        let (_tmp, dir) = tmp_dir();
        dir.join_component("Cargo.toml")
            .create_with_contents(CARGO_WORKSPACE)
            .unwrap();
        dir.join_component("pyproject.toml")
            .create_with_contents(UV_WORKSPACE)
            .unwrap();
        dir.join_component("go.work")
            .create_with_contents("go 1.22\n")
            .unwrap();
        let only = |cargo, python, go| NativeWorkspaceFlags { cargo, python, go };

        assert_eq!(
            native_workspace_manifest(&dir, only(false, false, false)).unwrap(),
            None
        );
        assert_eq!(
            native_workspace_manifest(&dir, only(true, false, false)).unwrap(),
            Some("Cargo.toml")
        );
        assert_eq!(
            native_workspace_manifest(&dir, only(false, true, false)).unwrap(),
            Some("pyproject.toml")
        );
        assert_eq!(
            native_workspace_manifest(&dir, only(false, false, true)).unwrap(),
            Some("go.work")
        );

        // A malformed manifest is only read, and only an error, when enabled.
        dir.join_component("Cargo.toml")
            .create_with_contents("[workspace")
            .unwrap();
        assert_eq!(
            native_workspace_manifest(&dir, only(false, false, true)).unwrap(),
            Some("go.work")
        );
        let error = native_workspace_manifest(&dir, only(true, false, true)).unwrap_err();
        assert_eq!(error.path, dir.join_component("Cargo.toml"));
    }

    #[test]
    fn malformed_manifest_does_not_hide_another_enabled_native_workspace() {
        assert_eq!(
            infer_native_root(&[
                (
                    "turbo.json",
                    r#"{"futureFlags":{"experimentalCargoWorkspaces":true,"experimentalGoWorkspaces":true}}"#,
                ),
                ("Cargo.toml", "[workspace"),
                ("go.work", "go 1.22\n"),
            ]),
            RepoMode::MultiPackage
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
