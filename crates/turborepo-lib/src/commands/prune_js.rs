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
    if version == "*" {
        return Some(name);
    }
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

/// Inputs for the JavaScript-only prune rendering step.
///
/// Closure selection and filesystem layout remain in `commands/prune.rs`;
/// this struct carries only the facts needed to rewrite JS lockfiles,
/// root manifests, patches, and workspace patch tables.
pub(crate) struct JavaScriptPruneRenderInput<'a> {
    pub package_manager: &'a PackageManager,
    pub root_package_json: &'a PackageJson,
    pub original_lockfile: &'a dyn turborepo_lockfiles::Lockfile,
    pub workspace_paths: &'a [String],
    pub lockfile_keys: &'a [String],
    pub excluded_dev_workspaces: &'a HashSet<String>,
    pub repo_root: &'a AbsoluteSystemPath,
    pub original_root_package_json_contents: &'a str,
    pub uses_per_workspace_lockfiles: bool,
}

/// How the pruned lockfile should be materialized by layout code.
#[derive(Debug)]
pub(crate) enum JavaScriptPruneLockfileArtifact {
    /// Copy the original root lockfile as-is (per-workspace lockfile mode).
    CopyOriginalRoot,
    /// Write these encoded bytes as the pruned root lockfile.
    Encoded(Vec<u8>),
}

#[derive(Debug)]
pub(crate) struct JavaScriptPruneFileArtifact {
    pub path: &'static str,
    pub contents: String,
}

/// Outputs of the JavaScript prune rendering step.
#[derive(Debug)]
pub(crate) struct JavaScriptPruneRenderResult {
    pub lockfile_name: &'static str,
    pub lockfile: JavaScriptPruneLockfileArtifact,
    /// `None` means copy the original root package.json unchanged.
    pub root_package_json_contents: Option<String>,
    pub pruned_patches: Vec<RelativeUnixPathBuf>,
    pub workspace_config: Option<JavaScriptPruneFileArtifact>,
}

/// Render JavaScript lockfile/manifest/patch artifacts for a pruned repository.
///
/// Performs no filesystem writes other than reads already implied by patch
/// path collection helpers.
pub(crate) fn render_javascript_prune(
    input: JavaScriptPruneRenderInput<'_>,
) -> Result<JavaScriptPruneRenderResult, Error> {
    let subgraph = input
        .original_lockfile
        .subgraph(input.workspace_paths, input.lockfile_keys)?;
    let lockfile_name = input.package_manager.lockfile_name();
    let lockfile = if input.uses_per_workspace_lockfiles {
        JavaScriptPruneLockfileArtifact::CopyOriginalRoot
    } else {
        JavaScriptPruneLockfileArtifact::Encoded(subgraph.encode()?)
    };

    let original_patches = collect_patch_paths(
        input.original_lockfile,
        input.root_package_json,
        input.repo_root,
        input.package_manager,
    )?;
    let pruned_patches = if original_patches.is_empty() {
        Vec::new()
    } else {
        collect_patch_paths(
            subgraph.as_ref(),
            input.root_package_json,
            input.repo_root,
            input.package_manager,
        )?
    };

    let original_value: serde_json::Value =
        serde_json::from_str(input.original_root_package_json_contents)?;
    let root_package_json_contents = if !original_patches.is_empty()
        || original_value.get("workspaces").is_some()
        || !input.excluded_dev_workspaces.is_empty()
    {
        let pruned_json = if original_patches.is_empty() {
            input.root_package_json.clone()
        } else {
            input.package_manager.prune_patched_packages(
                input.root_package_json,
                &pruned_patches,
                input.repo_root,
            )
        };

        let mut pruned_value = serde_json::to_value(&pruned_json)?;
        prune_package_json_workspaces(&mut pruned_value, input.workspace_paths);
        prune_package_json_dev_dependencies(&mut pruned_value, input.excluded_dev_workspaces);
        let merged = merge_preserving_key_order(&original_value, &pruned_value);
        let mut contents = serde_json::to_string_pretty(&merged)?;
        contents.push('\n');
        Some(contents)
    } else {
        None
    };

    let workspace_config = if input.package_manager.is_pnpm_family() {
        let path = turborepo_repository::package_manager::pnpm::WORKSPACE_CONFIGURATION_PATH;
        let source = input.repo_root.join_component(path);
        source
            .try_exists()?
            .then(|| {
                let contents = source.read_to_string()?;
                let contents =
                    turborepo_repository::package_manager::pnpm::prune_workspace_patches_contents(
                        &contents,
                        &pruned_patches,
                    )?;
                Ok::<_, std::io::Error>(JavaScriptPruneFileArtifact { path, contents })
            })
            .transpose()?
    } else {
        None
    };

    Ok(JavaScriptPruneRenderResult {
        lockfile_name,
        lockfile,
        root_package_json_contents,
        pruned_patches,
        workspace_config,
    })
}
