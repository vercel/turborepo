use std::env;

use tracing::debug;
use turborepo_repository::{inference::RepoState, package_manager::PackageManager};

const TURBO_DOWNLOAD_LOCAL_ENABLED: &str = "TURBO_DOWNLOAD_LOCAL_ENABLED";

/// Struct containing information about the desired local turbo version
/// according to lockfiles, package.jsons, and if all else fails turbo.json
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTurboConfig {
    turbo_version: String,
}

fn is_env_var_truthy(env_var: &str) -> Option<bool> {
    let value = env::var(env_var).ok()?;
    match value.as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

impl LocalTurboConfig {
    pub fn infer(repo_state: &RepoState) -> Option<Self> {
        Self::infer_internal(repo_state, is_env_var_truthy(TURBO_DOWNLOAD_LOCAL_ENABLED))
    }

    // Used for testing when we want to manually set the controlling env var
    fn infer_internal(repo_state: &RepoState, is_enabled: Option<bool>) -> Option<Self> {
        // TODO: once we have properly communicated this functionality we should make
        // this opt-out.
        if !is_enabled.unwrap_or(false) {
            debug!("downloading correct local version not enabled");
            return None;
        }
        let turbo_version = Self::turbo_version_from_lockfile(repo_state)?;
        Some(Self { turbo_version })
    }

    pub fn turbo_version(&self) -> &str {
        &self.turbo_version
    }

    fn turbo_version_from_lockfile(repo_state: &RepoState) -> Option<String> {
        if let Ok(package_manager) = &repo_state.package_manager {
            let lockfile = package_manager
                .read_lockfile(&repo_state.root, &repo_state.root_package_json)
                .ok()?;
            return lockfile.turbo_version();
        }

        // If there isn't a package manager, just try to parse all known lockfiles
        // This isn't the most effecient, but since we'll be hitting network to download
        // the correct binary the unnecessary file reads aren't costly relative to the
        // download.
        PackageManager::supported_managers().iter().find_map(|pm| {
            let lockfile = pm
                .read_lockfile(&repo_state.root, &repo_state.root_package_json)
                .ok()?;
            lockfile.turbo_version()
        })
    }
}

#[cfg(test)]
mod test {
    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPath;
    use turborepo_repository::{
        inference::RepoMode, package_json::PackageJson, package_manager::Error,
    };

    use super::*;

    #[test]
    fn test_package_manager_and_lockfile() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson::default(),
            package_manager: Ok(PackageManager::Npm),
        };
        let lockfile = root.join_component("package-lock.json");
        lockfile
            .create_with_contents(include_bytes!(
                "../../fixtures/local_config/turbov2.package-lock.json"
            ))
            .unwrap();

        assert_eq!(
            LocalTurboConfig::infer_internal(&repo, Some(true)),
            Some(LocalTurboConfig {
                turbo_version: "2.0.3".into()
            })
        );
    }

    #[test]
    fn test_just_lockfile() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson::default(),
            package_manager: Err(Error::MissingPackageManager),
        };
        let lockfile = root.join_component("package-lock.json");
        lockfile
            .create_with_contents(include_bytes!(
                "../../fixtures/local_config/turbov2.package-lock.json"
            ))
            .unwrap();

        assert_eq!(
            LocalTurboConfig::infer_internal(&repo, Some(true)),
            Some(LocalTurboConfig {
                turbo_version: "2.0.3".into()
            })
        );
    }

    #[test]
    fn test_package_json_dep() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson {
                dependencies: Some(
                    vec![("turbo".into(), "^2.0.0".into())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
            package_manager: Err(Error::MissingPackageManager),
        };

        assert_eq!(LocalTurboConfig::infer_internal(&repo, Some(true)), None,);
    }

    #[test]
    fn test_package_json_dev_dep() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson {
                dev_dependencies: Some(
                    vec![("turbo".into(), "^2.0.0".into())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
            package_manager: Err(Error::MissingPackageManager),
        };

        assert_eq!(LocalTurboConfig::infer_internal(&repo, Some(true)), None);
    }

    #[test]
    fn test_v1_schema() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson::default(),
            package_manager: Err(Error::MissingPackageManager),
        };
        let turbo_json = root.join_component("turbo.json");
        turbo_json
            .create_with_contents(include_bytes!("../../fixtures/local_config/turbo.v1.json"))
            .unwrap();
        assert_eq!(LocalTurboConfig::infer_internal(&repo, Some(true)), None);
    }

    #[test]
    fn test_v2_schema() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson::default(),
            package_manager: Err(Error::MissingPackageManager),
        };
        let turbo_json = root.join_component("turbo.json");
        turbo_json
            .create_with_contents(include_bytes!("../../fixtures/local_config/turbo.v2.json"))
            .unwrap();
        assert_eq!(LocalTurboConfig::infer_internal(&repo, Some(true)), None,);
    }

    #[test]
    fn nothing() {
        let tmpdir = TempDir::with_prefix("local_config").unwrap();
        let root = AbsoluteSystemPath::from_std_path(tmpdir.path()).unwrap();
        let repo = RepoState {
            root: root.to_owned(),
            mode: RepoMode::MultiPackage,
            root_package_json: PackageJson::default(),
            package_manager: Err(Error::MissingPackageManager),
        };
        assert_eq!(LocalTurboConfig::infer_internal(&repo, Some(true)), None,);
    }
}
