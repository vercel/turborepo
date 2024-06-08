use std::env;

use jsonc_parser::{
    ast::{ObjectPropName, Value},
    parse_to_ast,
};
use turborepo_repository::{inference::RepoState, package_manager::PackageManager};

const TURBO_DOWNLOAD_LOCAL_DISABLED: &str = "TURBO_DOWNLOAD_LOCAL_DISABLED";

/// Struct containing information about the desired local turbo version
/// according to lockfiles, package.jsons, and if all else fails turbo.json
pub struct LocalTurboConfig {
    turbo_version: String,
}

impl LocalTurboConfig {
    pub fn infer(repo_state: &RepoState) -> Option<Self> {
        // Don't attempt a download if user has opted out
        if env::var(TURBO_DOWNLOAD_LOCAL_DISABLED)
            .map_or(false, |disable| matches!(disable.as_str(), "1" | "true"))
        {
            return None;
        }
        let turbo_version = Self::turbo_version_from_lockfile(repo_state)
            .or_else(|| Self::turbo_version_from_package_json(repo_state))
            .or_else(|| Self::turbo_version_from_turbo_json_schema(repo_state))?;
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
        // the correct binary the unnecessary file reads.
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
