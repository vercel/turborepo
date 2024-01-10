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
                            PackageManager::get_package_manager(path, Some(&package_json));
                        let workspace_globs = package_manager
                            .as_ref()
                            .ok()
                            .and_then(|mgr| mgr.get_workspace_globs(path).ok());

                        InferInfo {
                            path: path.to_owned(),
                            workspace_globs,
                            package_manager,
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
    use crate::package_manager::PackageManager;

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
        monorepo_pkg_json.ensure_dir().unwrap();
        monorepo_pkg_json
            .create_with_contents("{\"workspaces\": [\"packages/*\"]}")
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
        standalone_pkg_json.ensure_dir().unwrap();
        standalone_pkg_json
            .create_with_contents("{\"name\":\"standalone\"}")
            .unwrap();
        standalone
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let standalone_monorepo = monorepo_root.join_component("standalone_monorepo");
        let app_2 = standalone_monorepo.join_components(&["packages", "app-2"]);
        app_2.create_dir_all().unwrap();
        app_2
            .join_component("package.json")
            .create_with_contents("{\"name\":\"app-2\"}")
            .unwrap();
        standalone_monorepo
            .join_component("package.json")
            .create_with_contents("{\"workspaces\": [\"packages/*\"]}")
            .unwrap();
        standalone_monorepo
            .join_component("package-lock.json")
            .create_with_contents("")
            .unwrap();

        let single_root = tmp_dir.join_component("single_root");
        let single_root_src = single_root.join_component("src");
        single_root_src.create_dir_all().unwrap();
        single_root
            .join_component("package.json")
            .create_with_contents("{\"name\": \"single-root\"}")
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
                }),
            ),
            (
                &app_1,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
                }),
            ),
            (
                &app_1_src,
                Some(RepoState {
                    root: monorepo_root.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
                }),
            ),
            (
                &single_root,
                Some(RepoState {
                    root: single_root.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm),
                }),
            ),
            (
                &single_root_src,
                Some(RepoState {
                    root: single_root.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm),
                }),
            ),
            // Nested, technically not supported
            (
                &standalone,
                Some(RepoState {
                    root: standalone.clone(),
                    mode: RepoMode::SinglePackage,
                    package_manager: Ok(pnpm),
                }),
            ),
            (
                &standalone_monorepo,
                Some(RepoState {
                    root: standalone_monorepo.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
                }),
            ),
            (
                &app_2,
                Some(RepoState {
                    root: standalone_monorepo.clone(),
                    mode: RepoMode::MultiPackage,
                    package_manager: Ok(pnpm),
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
}
