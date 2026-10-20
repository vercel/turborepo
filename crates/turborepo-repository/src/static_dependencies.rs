//! Manifest-derived package inputs, separate from authoritative task knowledge.

use std::collections::HashMap;

use turbopath::AbsoluteSystemPathBuf;
use wax::Program;

/// Match changed paths, including deletions, rather than testing existence on
/// disk. Ignoring exclusions over-approximates global inputs without false
/// skips.
pub fn global_inputs_changed(patterns: &[String], files: &[String]) -> bool {
    patterns
        .iter()
        .filter(|pattern| !pattern.starts_with('!'))
        .any(|pattern| {
            let pattern = pattern.strip_prefix("$TURBO_ROOT$/").unwrap_or(pattern);
            if pattern.contains('$') {
                return true;
            }
            match wax::Glob::new(pattern) {
                Ok(glob) => files
                    .iter()
                    .any(|file| glob.is_match(std::path::Path::new(file))),
                Err(_) => true,
            }
        })
}

use crate::package_graph::{PackageGraph, PackageName};

pub(crate) fn ancestor_configuration_paths(
    root: &turbopath::AbsoluteSystemPath,
    manifest: &turbopath::AbsoluteSystemPath,
    files: &[&str],
) -> Vec<AbsoluteSystemPathBuf> {
    let mut result = Vec::new();
    let mut directory = manifest.parent();
    while let Some(path) = directory {
        if !root.contains(path) {
            break;
        }
        result.extend(
            files
                .iter()
                .map(|file| AbsoluteSystemPathBuf::from_unknown(path, file)),
        );
        directory = path.parent();
    }
    result
}

/// A conservative superset of one package's local dependency inputs. This does
/// not describe task ordering, compiler targets, or resolved external packages.
#[derive(Debug, Clone)]
pub struct StaticPackageDependencies {
    pub package: String,
    pub dependencies: Vec<String>,
    pub invalidation_paths: Vec<AbsoluteSystemPathBuf>,
    /// Workspace-wide manifest names, including manifests removed from
    /// inventory.
    pub invalidation_file_names: Vec<String>,
    pub unresolved: bool,
}

#[derive(Debug, Default)]
pub struct StaticAffectedness {
    pub(crate) packages: Vec<StaticPackageDependencies>,
    pub(crate) unsupported: bool,
    paths: HashMap<std::path::PathBuf, Vec<PackageName>>,
    file_names: HashMap<String, Vec<PackageName>>,
}

impl StaticAffectedness {
    pub(crate) fn index_inputs(&mut self) {
        for package in &self.packages {
            let name = PackageName::Other(package.package.clone());
            for path in &package.invalidation_paths {
                self.paths
                    .entry(path.as_std_path().to_path_buf())
                    .or_default()
                    .push(name.clone());
            }
            for file in &package.invalidation_file_names {
                self.file_names
                    .entry(file.clone())
                    .or_default()
                    .push(name.clone());
            }
        }
        for names in self.paths.values_mut().chain(self.file_names.values_mut()) {
            names.sort();
            names.dedup();
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.unsupported && self.packages.iter().all(|package| !package.unresolved)
    }

    /// Native manifests/configuration/lockfiles invalidate their workspace's
    /// packages, not unrelated ecosystems. Includes deleted member manifests.
    pub fn seeds_for_path(&self, path: &turbopath::AbsoluteSystemPath) -> Vec<PackageName> {
        let mut seeds = self
            .paths
            .get(path.as_std_path())
            .cloned()
            .unwrap_or_default();
        if let Some(names) = path
            .as_std_path()
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| self.file_names.get(name))
        {
            seeds.extend(names.iter().cloned());
        }
        seeds.sort();
        seeds.dedup();
        seeds
    }

    pub fn affected_by(
        &self,
        graph: &PackageGraph,
        seeds: &[PackageName],
    ) -> Result<Vec<PackageName>, crate::package_graph::RelationshipProjectionError> {
        let mut inputs = self
            .packages
            .iter()
            .flat_map(|package| {
                package.dependencies.iter().map(|dependency| {
                    (
                        PackageName::Other(package.package.clone()),
                        PackageName::Other(dependency.clone()),
                    )
                })
            })
            .collect::<Vec<_>>();
        let mut colocated = std::collections::HashMap::new();
        for context in graph
            .package_task_contexts()
            .filter(|context| graph.is_real_package(context.package()))
        {
            colocated
                .entry(context.directory().to_owned())
                .or_insert_with(Vec::new)
                .push(context.package().clone());
        }
        for scopes in colocated.values() {
            if let Some((first, rest)) = scopes.split_first() {
                for scope in rest {
                    inputs.push((first.clone(), scope.clone()));
                    inputs.push((scope.clone(), first.clone()));
                }
            }
        }
        graph
            .affected_relationships()
            .affected_by_with_inputs(seeds, &inputs)
    }
}
