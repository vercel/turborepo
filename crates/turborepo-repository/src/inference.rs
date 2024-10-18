use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    package_json::PackageJson,
    package_manager::{self, PackageManager, WorkspaceGlobs},
};

#[derive(Debug, PartialEq)]
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
    #[error("Failed to find repository root containing {0}")]
    NotFound(AbsoluteSystemPathBuf),
}

#[derive(Debug)]
struct InferInfo {
    path: AbsoluteSystemPathBuf,
    workspace_globs: Option<WorkspaceGlobs>,
    package_manager: Result<PackageManager, package_manager::Error>,
    package_json: PackageJson,
}

impl InferInfo {
    fn repo_mode(&self) -> RepoMode {
        if self.workspace_globs.is_some() {
            RepoMode::MultiPackage
        } else {
            RepoMode::SinglePackage
        }
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
    pub fn infer(reference_dir: &AbsoluteSystemPath) -> Result<Self, Error> {
        reference_dir
            .ancestors()
            .filter_map(|path| {
                PackageJson::load(&path.join_component("package.json"))
                    .ok()
                    .map(|package_json| {
                        // FIXME: We should save this package manager that we detected
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
                        }
                    })
            })
            .reduce(|current, candidate| {
                if current.repo_mode() == RepoMode::MultiPackage {
                    // We already have a multi-package root, go with that
                    current
                } else if candidate.is_workspace_root_of(&current.path) {
                    // The next candidate is a multipackage root, and it contains current so it's
                    // our root.
                    candidate
                } else {
                    // keep the current single package, it's the closest in
                    current
                }
            })
            .map(|root| root.into())
            .ok_or_else(|| Error::NotFound(reference_dir.to_owned()))
    }
}

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPathBuf;

    use super::{RepoMode, RepoState};
    use crate::{package_json::PackageJson, package_manager::PackageManager};

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
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
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&monorepo_pkg_json).unwrap(),
                }),
            ),
            (
                &app_1,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&monorepo_pkg_json).unwrap(),
                }),
            ),
            (
                &app_1_src,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&monorepo_pkg_json).unwrap(),
                }),
            ),
            (
                &single_root,
                Some(RepoState {
                    root: single_root.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&single_root_package_json).unwrap(),
                }),
            ),
            (
                &single_root_src,
                Some(RepoState {
                    root: single_root.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&single_root_package_json).unwrap(),
                }),
            ),
            // Nested, technically not supported
            (
                &standalone,
                Some(RepoState {
                    root: standalone.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm),
                    root_package_json: PackageJson::load(&standalone_pkg_json).unwrap(),
                }),
            ),
            (
                &standalone_monorepo,
                Some(RepoState {
                    root: standalone_monorepo.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
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
    fn test_allows_missing_package_manager() {
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
        assert_eq!(&repo_state_from_app.root, &monorepo_root);
        assert_eq!(repo_state_from_root.mode, RepoMode::MultiPackage);
        assert_eq!(repo_state_from_app.mode, RepoMode::MultiPackage);
        assert_eq!(
            repo_state_from_root.package_manager.unwrap(),
            PackageManager::Npm
        );
        assert_eq!(
            repo_state_from_app.package_manager.unwrap(),
            PackageManager::Npm
        );
    }

    #[test]
    fn test_gh_8599() {
        // TODO: this test documents existing broken behavior, when we have time we
        // should fix this and update the assertions
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
        // These assertions are the buggy behavior
        assert_eq!(repo_state.root, package_foo);
        assert_eq!(repo_state.mode, RepoMode::SinglePackage);
        // TODO: the following assertions are the correct behavior
        // assert_eq!(repo_state.root, monorepo_root);
        // assert_eq!(repo_state.mode, RepoMode::MultiPackage);
    }
}
