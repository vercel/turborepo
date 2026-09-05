//! Lockfile closure resolution without a [`super::PackageGraph`].
//!
//! Consumers that only need the set of external packages a workspace's
//! lockfile pins (for example dependency auditing) don't need the internal
//! package graph, task knowledge, or change knowledge that
//! [`super::PackageGraphBuilder`] constructs. This walks the workspace
//! `package.json` files just far enough to split internal from external
//! declarations, then computes the transitive closure over the lockfile.

use std::collections::{BTreeMap, HashMap, HashSet};

use rayon::prelude::*;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_lockfiles::{Lockfile, Package};

use super::{
    PackageName,
    dep_splitter::{DependencySplitter, WorkspacePathIndex},
};
use crate::{
    package_json::{DependencyKind, PackageJson},
    package_manager::{self, PackageManager},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    PackageManager(#[from] package_manager::Error),
    #[error(transparent)]
    PackageJson(#[from] crate::package_json::Error),
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
}

/// Every external package reachable from any workspace package's
/// declarations, deduplicated by lockfile key and sorted.
///
/// Workspace packages are discovered through the package manager's workspace
/// globs; declarations that resolve to another workspace package are excluded
/// so the result mirrors the lockfile domain of
/// [`super::PackageGraph::javascript_external_resolution`].
pub fn external_packages(
    repo_root: &AbsoluteSystemPath,
    package_manager: &PackageManager,
    root_package_json: &PackageJson,
    lockfile: &dyn Lockfile,
) -> Result<Vec<Package>, Error> {
    let workspace_paths = match package_manager.get_package_jsons(repo_root) {
        Ok(paths) => paths.collect::<Vec<_>>(),
        // No configured workspaces is not an error: only the root manifest
        // contributes declarations.
        Err(package_manager::Error::Workspace(_)) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let mut manifests = workspace_paths
        .into_par_iter()
        .map(|path| {
            let package_json = PackageJson::load(&path)?;
            let directory = repo_root.anchor(path.parent().unwrap_or(repo_root))?;
            Ok((directory, package_json))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    manifests.push((AnchoredSystemPathBuf::default(), root_package_json.clone()));

    // Name-keyed manifests for `workspace:` alias resolution. Nameless
    // manifests can't be depended on by name but still declare externals.
    let workspaces: HashMap<PackageName, PackageJson> = manifests
        .iter()
        .filter_map(|(directory, package_json)| {
            let name = if directory.as_str().is_empty() {
                PackageName::Root
            } else {
                PackageName::Other(package_json.name.as_ref()?.as_inner().clone())
            };
            Some((name, package_json.clone()))
        })
        .collect();
    let path_index = WorkspacePathIndex::from_directories(manifests.iter().filter_map(
        |(directory, package_json)| {
            let name = if directory.as_str().is_empty() {
                PackageName::Root
            } else {
                PackageName::Other(package_json.name.as_ref()?.as_inner().clone())
            };
            Some((directory.as_ref(), name))
        },
    ));
    let link_workspace_packages = package_manager.link_workspace_packages(repo_root);
    let catalogs = package_manager.read_catalogs(repo_root);

    let external_dependencies: HashMap<String, BTreeMap<String, String>> = manifests
        .iter()
        .map(|(directory, package_json)| {
            let workspace_dir = repo_root.resolve(directory);
            let splitter = DependencySplitter::new(
                repo_root,
                &workspace_dir,
                &workspaces,
                link_workspace_packages,
                &path_index,
                catalogs.as_ref(),
            );
            // First declaration of a name wins, before the internal/external
            // split, matching `javascript::external_dependencies`.
            let mut seen = HashSet::new();
            let mut dependencies = BTreeMap::new();
            for (name, specifier, kind) in package_json.dependencies_with_kind() {
                if !seen.insert(name.as_str())
                    || matches!(kind, DependencyKind::Peer { .. })
                    || splitter.is_internal(name, specifier).is_some()
                {
                    continue;
                }
                dependencies.insert(name.clone(), specifier.clone());
            }
            (directory.to_unix().to_string(), dependencies)
        })
        .collect();

    let closures = turborepo_lockfiles::all_transitive_closures_sorted(
        lockfile,
        external_dependencies,
        false,
    )?;
    let mut seen = HashSet::new();
    let mut packages: Vec<Package> = closures
        .into_values()
        .flatten()
        .filter(|package| seen.insert(package.key.clone()))
        .map(|package| (*package).clone())
        .collect();
    packages.sort_unstable();
    Ok(packages)
}
