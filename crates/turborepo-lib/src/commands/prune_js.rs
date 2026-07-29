//! Pure JavaScript prune rendering helpers extracted from `prune`
//! orchestration.
//!
//! These functions rewrite manifests/workspaces/patches without performing
//! filesystem layout. `commands/prune.rs` remains responsible for copying and
//! path safety until later Phase 7 leaves migrate orchestration.

use std::collections::{BTreeSet, HashSet};

use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

use super::Error;

pub(crate) fn workspace_dependency_target<'a>(name: &'a str, version: &'a str) -> Option<&'a str> {
    let specifier = version.strip_prefix("workspace:")?;
    match specifier.rsplit_once('@') {
        Some((target, "*" | "^" | "~")) if !target.is_empty() => Some(target),
        _ => Some(name),
    }
}

pub(crate) fn prune_package_json_dev_dependencies(
    package_json: &mut serde_json::Value,
    excluded_workspaces: &HashSet<String>,
) -> bool {
    let Some(dev_dependencies) = package_json
        .get_mut("devDependencies")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };

    let original_len = dev_dependencies.len();
    dev_dependencies.retain(|name, version| {
        let Some(version) = version.as_str() else {
            return true;
        };
        workspace_dependency_target(name, version)
            .is_none_or(|target| !excluded_workspaces.contains(target))
    });
    let changed = dev_dependencies.len() != original_len;
    let remove_dev_dependencies = dev_dependencies.is_empty();
    if remove_dev_dependencies {
        if let Some(package_json) = package_json.as_object_mut() {
            package_json.remove("devDependencies");
        }
    }
    changed
}

pub(crate) fn prune_package_json_workspaces(
    package_json: &mut serde_json::Value,
    workspace_paths: &[String],
) {
    let Some(workspaces) = package_json.get_mut("workspaces") else {
        return;
    };

    let pruned_workspaces = || {
        workspace_paths
            .iter()
            .map(|workspace| serde_json::Value::String(workspace.clone()))
            .collect::<Vec<_>>()
    };

    match workspaces {
        serde_json::Value::Array(packages) => *packages = pruned_workspaces(),
        serde_json::Value::Object(config) => {
            if let Some(packages) = config.get_mut("packages") {
                *packages = serde_json::Value::Array(pruned_workspaces());
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_patch_paths(
    lockfile: &dyn turborepo_lockfiles::Lockfile,
    root_package_json: &PackageJson,
    repo_root: &turbopath::AbsoluteSystemPath,
    package_manager: &PackageManager,
) -> Result<Vec<RelativeUnixPathBuf>, Error> {
    let mut patches = lockfile.patches()?;
    let patch_keys = lockfile.patch_keys();

    if !patch_keys.is_empty() {
        patches.extend(package_json_patch_paths(root_package_json, &patch_keys));

        if package_manager.is_pnpm_family() {
            let workspace_yaml_path = repo_root.join_component(
                turborepo_repository::package_manager::pnpm::WORKSPACE_CONFIGURATION_PATH,
            );
            patches.extend(
                turborepo_repository::package_manager::pnpm::patch_paths_for_keys(
                    &workspace_yaml_path,
                    &patch_keys,
                )?,
            );
        }
    }

    patches.sort();
    patches.dedup();
    validate_patch_source_paths(repo_root, &patches)?;
    Ok(patches)
}

pub(crate) fn validate_patch_source_paths(
    repo_root: &AbsoluteSystemPath,
    patches: &[RelativeUnixPathBuf],
) -> Result<(), Error> {
    let repo_root_realpath = repo_root.to_realpath()?;

    for patch in patches {
        let patch_path = repo_root.join_unix_path(patch);
        if !patch_path.starts_with(repo_root.as_std_path()) {
            return Err(Error::InvalidPatchPath(patch.clone()));
        }

        if patch_path.try_exists()? {
            let patch_realpath = patch_path.to_realpath()?;
            if !patch_realpath.starts_with(repo_root_realpath.as_std_path()) {
                return Err(Error::InvalidPatchPath(patch.clone()));
            }
        }
    }

    Ok(())
}

pub(crate) fn package_json_patch_paths(
    package_json: &PackageJson,
    patch_keys: &[String],
) -> Vec<RelativeUnixPathBuf> {
    let patch_keys: BTreeSet<_> = patch_keys.iter().map(String::as_str).collect();
    let mut patches = Vec::new();

    if let Some(patched_dependencies) = package_json.patched_dependencies.as_ref() {
        patches.extend(
            patched_dependencies.iter().filter_map(|(key, path)| {
                patch_keys.contains(key.as_str()).then_some(path.clone())
            }),
        );
    }

    if let Some(patched_dependencies) = package_json
        .pnpm
        .as_ref()
        .and_then(|config| config.patched_dependencies.as_ref())
    {
        patches.extend(
            patched_dependencies.iter().filter_map(|(key, path)| {
                patch_keys.contains(key.as_str()).then_some(path.clone())
            }),
        );
    }

    patches
}

pub(crate) fn bin_paths(package_json: &PackageJson) -> Vec<&str> {
    match package_json.other.get("bin") {
        Some(serde_json::Value::String(path)) => vec![path.as_str()],
        Some(serde_json::Value::Object(entries)) => entries
            .values()
            .filter_map(serde_json::Value::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn merge_preserving_key_order(
    original: &serde_json::Value,
    pruned: &serde_json::Value,
) -> serde_json::Value {
    match (original, pruned) {
        (serde_json::Value::Object(orig_map), serde_json::Value::Object(pruned_map)) => {
            let mut result = serde_json::Map::new();
            for (key, orig_val) in orig_map {
                if let Some(pruned_val) = pruned_map.get(key) {
                    result.insert(
                        key.clone(),
                        merge_preserving_key_order(orig_val, pruned_val),
                    );
                }
            }
            for (key, pruned_val) in pruned_map {
                if !orig_map.contains_key(key) {
                    result.insert(key.clone(), pruned_val.clone());
                }
            }
            serde_json::Value::Object(result)
        }
        (_, pruned) => pruned.clone(),
    }
}
