use std::env;

use jsonc_parser::{
    ast::{ObjectPropName, Value},
    parse_to_ast,
};
use tracing::debug;
use turborepo_repository::{inference::RepoState, package_manager::PackageManager};

const TURBO_DOWNLOAD_LOCAL_DISABLED: &str = "TURBO_DOWNLOAD_LOCAL_DISABLED";

/// Struct containing information about the desired local turbo version
/// according to lockfiles, package.jsons, and if all else fails turbo.json
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTurboConfig {
    turbo_version: String,
}

impl LocalTurboConfig {
    pub fn infer(repo_state: &RepoState) -> Option<Self> {
        // Don't attempt a download if user has opted out
        if env::var(TURBO_DOWNLOAD_LOCAL_DISABLED)
            .map_or(false, |disable| matches!(disable.as_str(), "1" | "true"))
        {
            debug!("downloading correct local version disabled");
            return None;
        }
        let turbo_version = Self::turbo_version_from_lockfile(repo_state)
            .or_else(|| {
                debug!(
                    "No turbo version found in a lockfile. Attempting to read version from root \
                     package.json"
                );
                Self::turbo_version_from_package_json(repo_state)
            })
            .or_else(|| {
                debug!("No turbo version found in package.json. Checking if turbo.json is for v1");
                Self::turbo_version_from_turbo_json_schema(repo_state)
            })?;
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

    fn turbo_version_from_package_json(repo_state: &RepoState) -> Option<String> {
        let package_json = &repo_state.root_package_json;
        // Look for turbo as a root dependency
        package_json
            .all_dependencies()
            .find_map(|(name, version)| (name == "turbo").then(|| version.clone()))
    }

    fn turbo_version_from_turbo_json_schema(repo_state: &RepoState) -> Option<String> {
        let turbo_json_path = repo_state.root.join_component("turbo.json");
        let turbo_json_contents = turbo_json_path.read_existing_to_string().ok().flatten()?;
        // We explicitly do not use regular path for parsing turbo.json as that will
        // fail if it sees unexpected keys. Future versions of turbo might add
        // keys and we don't want to crash in that situation.
        let turbo_json = parse_to_ast(
            &turbo_json_contents,
            &Default::default(),
            &Default::default(),
        )
        .ok()?;

        if let Value::Object(turbo_json) = turbo_json.value? {
            let has_pipeline = turbo_json.properties.iter().any(|property| {
                let ObjectPropName::String(name) = &property.name else {
                    return false;
                };
                name.value == "pipeline"
            });
            if has_pipeline {
                // All we can determine is that the turbo.json is meant for a turbo v1
                return Some("^1".to_owned());
            }
        }
        // We do not check for the existence of `tasks` as it provides us no beneficial
        // information. We're already a turbo 2 binary so we'll continue
        // execution.
        None
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
            LocalTurboConfig::infer(&repo),
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
            LocalTurboConfig::infer(&repo),
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

        assert_eq!(
            LocalTurboConfig::infer(&repo),
            Some(LocalTurboConfig {
                turbo_version: "^2.0.0".into()
            })
        );
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

        assert_eq!(
            LocalTurboConfig::infer(&repo),
            Some(LocalTurboConfig {
                turbo_version: "^2.0.0".into()
            })
        );
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
        assert_eq!(
            LocalTurboConfig::infer(&repo),
            Some(LocalTurboConfig {
                turbo_version: "^1".into()
            })
        );
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
        assert_eq!(LocalTurboConfig::infer(&repo), None,);
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
        assert_eq!(LocalTurboConfig::infer(&repo), None,);
    }
}
