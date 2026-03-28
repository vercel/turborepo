//! TurboJson loading utilities
//!
//! This module provides utilities for reading turbo.json files from disk,
//! including strategies for loading turbo.json in different contexts (single
//! package, workspace, etc.).

use std::collections::HashMap;

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_fixed_map::FixedMap;
use turborepo_repository::{
    package_graph::{PackageInfo, PackageName},
    package_json::PackageJson,
};
use turborepo_task_id::TaskName;
use turborepo_types::EnvMode;

use crate::{Error, FutureFlags, Pipeline, RawTaskDefinition, TurboJson};

/// Configuration file names
pub const CONFIG_FILE: &str = "turbo.json";
pub const CONFIG_FILE_JSONC: &str = "turbo.jsonc";

/// Path to the config file that will be used to store the trace results
/// (relative to repo root)
pub const TASK_ACCESS_CONFIG_PATH: [&str; 2] = [".turbo", "traced-config.json"];

/// Errors specific to TurboJson loading operations.
///
/// These errors are returned by `TurboJsonLoader` and wrap the underlying
/// `turborepo_turbo_json::Error` type plus loader-specific errors.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum LoaderError {
    /// Error from the turbo.json parsing layer
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJson(#[from] Error),

    /// Cannot load turbo.json for this package in single package mode
    #[error("Cannot load turbo.json for {0} in single package mode.")]
    InvalidTurboJsonLoad(PackageName),
}

impl LoaderError {
    /// Returns true if this error indicates that no turbo.json file was found.
    pub fn is_no_turbo_json(&self) -> bool {
        matches!(self, LoaderError::TurboJson(Error::NoTurboJSON))
    }
}

/// Trait for types that can update/enrich a TurboJson after loading.
///
/// This allows external code (like MicrofrontendsConfigs in turborepo-lib) to
/// modify TurboJson structures without requiring this crate to depend on those
/// types directly.
///
/// # Type Parameters
/// * `E` - The error type that the updater can produce. This allows different
///   implementations to use their own error types.
pub trait TurboJsonUpdater {
    /// The error type that this updater can produce
    type Error: std::error::Error + Send + Sync + 'static;

    /// Update a TurboJson for a given package.
    ///
    /// This method is called after the TurboJson has been loaded from disk.
    /// It can modify the TurboJson to add additional configuration (e.g.,
    /// microfrontend-specific tasks) or return an error.
    ///
    /// # Arguments
    /// * `package_name` - The name of the package being loaded
    /// * `turbo_json` - The result of loading the turbo.json (may be Ok or Err)
    ///
    /// # Returns
    /// The potentially modified TurboJson, or an error
    fn update_turbo_json(
        &self,
        package_name: &PackageName,
        turbo_json: Result<TurboJson, LoaderError>,
    ) -> Result<TurboJson, Self::Error>;
}

/// A no-op updater that passes through the TurboJson unchanged.
///
/// This is used when no enrichment is needed (e.g., in simple workspaces
/// without microfrontends).
#[derive(Debug, Clone, Default)]
pub struct NoOpUpdater;

impl TurboJsonUpdater for NoOpUpdater {
    type Error = LoaderError;

    fn update_turbo_json(
        &self,
        _package_name: &PackageName,
        turbo_json: Result<TurboJson, LoaderError>,
    ) -> Result<TurboJson, Self::Error> {
        turbo_json
    }
}

/// A helper structure configured with all settings related to reading a
/// `turbo.json` file from disk.
#[derive(Debug, Clone)]
pub struct TurboJsonReader {
    repo_root: AbsoluteSystemPathBuf,
    future_flags: FutureFlags,
}

impl TurboJsonReader {
    /// Create a new TurboJsonReader with the given repo root
    pub fn new(repo_root: AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root,
            future_flags: Default::default(),
        }
    }

    /// Set the future flags for this reader
    pub fn with_future_flags(mut self, future_flags: FutureFlags) -> Self {
        self.future_flags = future_flags;
        self
    }

    /// Read a turbo.json file from the given path
    ///
    /// # Arguments
    /// * `path` - The path to the turbo.json file
    /// * `is_root` - Whether this is a root turbo.json (affects schema
    ///   validation)
    ///
    /// # Returns
    /// * `Ok(Some(TurboJson))` - Successfully read and parsed the file
    /// * `Ok(None)` - File does not exist
    /// * `Err(Error)` - Error reading or parsing the file
    pub fn read(
        &self,
        path: &AbsoluteSystemPath,
        is_root: bool,
    ) -> Result<Option<TurboJson>, Error> {
        TurboJson::read(&self.repo_root, path, is_root, self.future_flags)
    }

    /// Get the repo root path
    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        &self.repo_root
    }

    /// Get the future flags
    pub fn future_flags(&self) -> FutureFlags {
        self.future_flags
    }
}

/// Represents where to look for a turbo.json file
#[derive(Debug, Clone)]
pub enum TurboJsonPath<'a> {
    /// Look for turbo.json/turbo.jsonc in this directory
    Dir(&'a AbsoluteSystemPath),
    /// Only use this specific file path (does not need to be named turbo.json)
    File(&'a AbsoluteSystemPath),
}

/// Load a turbo.json from a path, handling both turbo.json and turbo.jsonc
///
/// This function handles the logic of:
/// - Looking for both turbo.json and turbo.jsonc in a directory
/// - Erroring if both exist
/// - Returning the appropriate one if only one exists
pub fn load_from_path(
    reader: &TurboJsonReader,
    turbo_json_path: TurboJsonPath,
    is_root: bool,
) -> Result<TurboJson, Error> {
    let result = match turbo_json_path {
        TurboJsonPath::Dir(turbo_json_dir_path) => {
            let turbo_json_path = turbo_json_dir_path.join_component(CONFIG_FILE);
            let turbo_jsonc_path = turbo_json_dir_path.join_component(CONFIG_FILE_JSONC);

            // Load both turbo.json and turbo.jsonc
            let turbo_json = reader.read(&turbo_json_path, is_root);
            let turbo_jsonc = reader.read(&turbo_jsonc_path, is_root);

            select_turbo_json(turbo_json_dir_path, turbo_json, turbo_jsonc)
        }
        TurboJsonPath::File(turbo_json_path) => reader.read(turbo_json_path, is_root),
    };

    // Handle errors or success
    match result {
        // There was an error, and we don't have any chance of recovering
        Err(e) => Err(e),
        Ok(None) => Err(Error::NoTurboJSON),
        // We're not synthesizing anything and there was no error, we're done
        Ok(Some(turbo)) => Ok(turbo),
    }
}

/// Helper for selecting the correct turbo.json read result when both
/// turbo.json and turbo.jsonc might exist
fn select_turbo_json(
    turbo_json_dir_path: &AbsoluteSystemPath,
    turbo_json: Result<Option<TurboJson>, Error>,
    turbo_jsonc: Result<Option<TurboJson>, Error>,
) -> Result<Option<TurboJson>, Error> {
    tracing::debug!(
        "path: {}, turbo_json: {:?}, turbo_jsonc: {:?}",
        turbo_json_dir_path.as_str(),
        turbo_json.as_ref().map(|v| v.as_ref().map(|_| ())),
        turbo_jsonc.as_ref().map(|v| v.as_ref().map(|_| ()))
    );
    match (turbo_json, turbo_jsonc) {
        // If both paths contain valid turbo.json error
        (Ok(Some(_)), Ok(Some(_))) => Err(Error::MultipleTurboConfigs {
            directory: turbo_json_dir_path.to_string(),
        }),
        // If turbo.json is valid and turbo.jsonc is missing or invalid, use turbo.json
        (Ok(Some(turbo_json)), Ok(None)) | (Ok(Some(turbo_json)), Err(_)) => Ok(Some(turbo_json)),
        // If turbo.jsonc is valid and turbo.json is missing or invalid, use turbo.jsonc
        (Ok(None), Ok(Some(turbo_jsonc))) | (Err(_), Ok(Some(turbo_jsonc))) => {
            Ok(Some(turbo_jsonc))
        }
        // If neither are present, then choose nothing
        (Ok(None), Ok(None)) => Ok(None),
        // If only one has an error return the failure
        (Err(e), Ok(None)) | (Ok(None), Err(e)) => Err(e),
        // If both fail then just return error for `turbo.json`
        (Err(e), Err(_)) => Err(e),
    }
}

/// Internal enum representing where to look for a turbo.json file during
/// loading
enum LoadTurboJsonPath<'a> {
    // Look for a turbo.json in this directory
    Dir(&'a AbsoluteSystemPath),
    // Only use this path as a source for turbo.json
    // Does not need to have filename of turbo.json
    File(&'a AbsoluteSystemPath),
}

/// Structure for loading TurboJson structures.
/// Depending on the strategy used, TurboJson might not correspond to a
/// `turbo.json` file.
///
/// # Type Parameters
/// * `U` - The updater type used to enrich TurboJsons after loading
/// * `E` - The error type produced by the updater
#[derive(Debug, Clone)]
pub struct TurboJsonLoader<U: TurboJsonUpdater = NoOpUpdater> {
    reader: TurboJsonReader,
    cache: FixedMap<PackageName, TurboJson>,
    strategy: Strategy<U>,
}

#[derive(Debug, Clone)]
enum Strategy<U: TurboJsonUpdater> {
    SinglePackage {
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    },
    Workspace {
        // Map of package names to their package specific turbo.json
        packages: HashMap<PackageName, AbsoluteSystemPathBuf>,
        updater: Option<U>,
    },
    WorkspaceNoTurboJson {
        // Map of package names to their scripts
        packages: HashMap<PackageName, Vec<String>>,
        updater: Option<U>,
    },
    TaskAccess {
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    },
    Noop,
}

impl TurboJsonLoader<NoOpUpdater> {
    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        let repo_root = reader.repo_root();
        let packages = package_turbo_json_dirs(repo_root, root_turbo_json_path, packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::Workspace {
                packages,
                updater: None,
            },
        }
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s.
    pub fn workspace_no_turbo_json<'a>(
        reader: TurboJsonReader,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        let packages = workspace_package_scripts(packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::WorkspaceNoTurboJson {
                packages,
                updater: None,
            },
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn single_package(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self {
            reader,
            cache: FixedMap::new(Some(PackageName::Root).into_iter()),
            strategy: Strategy::SinglePackage {
                root_turbo_json,
                package_json,
            },
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn task_access(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self {
            reader,
            cache: FixedMap::new(Some(PackageName::Root).into_iter()),
            strategy: Strategy::TaskAccess {
                root_turbo_json,
                package_json,
            },
        }
    }

    /// Create a loader that will only return provided turbo.jsons and will
    /// never hit the file system.
    /// Primarily intended for testing
    pub fn noop(turbo_jsons: HashMap<PackageName, TurboJson>) -> Self {
        let cache = FixedMap::from_iter(
            turbo_jsons
                .into_iter()
                .map(|(key, value)| (key, Some(value))),
        );
        // This never gets read from so we populate it with root
        let repo_root = AbsoluteSystemPath::new(if cfg!(windows) { "C:\\" } else { "/" })
            .expect("wasn't able to create absolute system path")
            .to_owned();
        Self {
            reader: TurboJsonReader::new(repo_root),
            cache,
            strategy: Strategy::Noop,
        }
    }
}

impl<U: TurboJsonUpdater> TurboJsonLoader<U> {
    /// Create a loader that will load turbo.json files throughout the workspace
    /// with a custom updater for enriching loaded TurboJsons.
    pub fn workspace_with_updater<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        updater: U,
    ) -> Self {
        let repo_root = reader.repo_root();
        let packages = package_turbo_json_dirs(repo_root, root_turbo_json_path, packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::Workspace {
                packages,
                updater: Some(updater),
            },
        }
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s, with a custom updater.
    pub fn workspace_no_turbo_json_with_updater<'a>(
        reader: TurboJsonReader,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        updater: Option<U>,
    ) -> Self {
        let packages = workspace_package_scripts(packages);
        Self {
            reader,
            cache: FixedMap::new(packages.keys().cloned()),
            strategy: Strategy::WorkspaceNoTurboJson { packages, updater },
        }
    }

    /// Load a turbo.json for a given package
    pub fn load(&self, package: &PackageName) -> Result<&TurboJson, U::Error>
    where
        U::Error: From<LoaderError>,
    {
        if let Ok(Some(turbo_json)) = self.cache.get(package) {
            return Ok(turbo_json);
        }
        let turbo_json = self.uncached_load(package)?;
        self.cache
            .insert(package, turbo_json)
            .map_err(|_| LoaderError::TurboJson(Error::NoTurboJSON).into())
    }

    /// Pre-warm the cache by loading all package turbo.json files in parallel.
    /// Errors are silently ignored — the next sequential `load()` call will
    /// report them. This is purely an optimization to overlap I/O and parsing.
    pub fn preload_all(&self)
    where
        U::Error: From<LoaderError> + Send,
        U: Sync,
    {
        use rayon::prelude::*;

        let packages: Vec<PackageName> = match &self.strategy {
            Strategy::Workspace { packages, .. } => packages.keys().cloned().collect(),
            Strategy::WorkspaceNoTurboJson { packages, .. } => packages.keys().cloned().collect(),
            _ => return,
        };

        packages.par_iter().for_each(|pkg| {
            let _ = self.load(pkg);
        });
    }

    fn uncached_load(&self, package: &PackageName) -> Result<TurboJson, U::Error>
    where
        U::Error: From<LoaderError>,
    {
        let reader = &self.reader;
        match &self.strategy {
            Strategy::SinglePackage {
                package_json,
                root_turbo_json,
            } => {
                if !matches!(package, PackageName::Root) {
                    Err(LoaderError::InvalidTurboJsonLoad(package.clone()).into())
                } else {
                    load_from_root_package_json(reader, root_turbo_json, package_json)
                        .map_err(|e| e.into())
                }
            }
            Strategy::Workspace { packages, updater } => {
                let turbo_json_path = packages.get(package).ok_or_else(|| {
                    Into::<U::Error>::into(LoaderError::TurboJson(Error::NoTurboJSON))
                })?;
                // Check if this package is at the repo root. This can happen when
                // the workspace definition includes "." as a package. In that case,
                // the package's turbo.json would be the root turbo.json, so we
                // should treat it as root to use the correct schema.
                // We use to_realpath() to resolve symlinks so that a symlinked
                // package pointing to the repo root is also detected correctly.
                let is_package_at_root = !matches!(package, PackageName::Root)
                    && turbo_json_path
                        .to_realpath()
                        .ok()
                        .zip(reader.repo_root().to_realpath().ok())
                        .map(|(pkg_real, root_real)| pkg_real == root_real)
                        .unwrap_or(false);
                let is_root = package == &PackageName::Root || is_package_at_root;
                let turbo_json = load_turbo_json_from_file(
                    reader,
                    if package == &PackageName::Root {
                        LoadTurboJsonPath::File(turbo_json_path)
                    } else {
                        LoadTurboJsonPath::Dir(turbo_json_path)
                    },
                    is_root,
                );
                if let Some(updater) = updater {
                    updater.update_turbo_json(package, turbo_json)
                } else {
                    turbo_json.map_err(|e| e.into())
                }
            }
            Strategy::WorkspaceNoTurboJson { packages, updater } => {
                let script_names =
                    packages
                        .get(package)
                        .ok_or(Into::<U::Error>::into(LoaderError::TurboJson(
                            Error::NoTurboJSON,
                        )))?;
                if matches!(package, PackageName::Root) {
                    root_turbo_json_from_scripts(script_names).map_err(|e| e.into())
                } else {
                    let turbo_json = workspace_turbo_json_from_scripts(script_names);
                    if let Some(updater) = updater {
                        updater.update_turbo_json(package, turbo_json)
                    } else {
                        turbo_json.map_err(|e| e.into())
                    }
                }
            }
            Strategy::TaskAccess {
                package_json,
                root_turbo_json,
            } => {
                if !matches!(package, PackageName::Root) {
                    Err(LoaderError::InvalidTurboJsonLoad(package.clone()).into())
                } else {
                    load_task_access_trace_turbo_json(reader, root_turbo_json, package_json)
                        .map_err(|e| e.into())
                }
            }
            Strategy::Noop => Err(LoaderError::TurboJson(Error::NoTurboJSON).into()),
        }
    }
}

/// Map all packages in the package graph to their dirs that contain a
/// turbo.json
fn package_turbo_json_dirs<'a>(
    repo_root: &AbsoluteSystemPath,
    root_turbo_json_path: AbsoluteSystemPathBuf,
    packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
) -> HashMap<PackageName, AbsoluteSystemPathBuf> {
    let mut package_turbo_jsons = HashMap::new();
    package_turbo_jsons.insert(PackageName::Root, root_turbo_json_path);
    package_turbo_jsons.extend(packages.filter_map(|(pkg, info)| {
        if pkg == &PackageName::Root {
            None
        } else {
            Some((pkg.clone(), repo_root.resolve(info.package_path())))
        }
    }));
    package_turbo_jsons
}

/// Map all packages in the package graph to their scripts
fn workspace_package_scripts<'a>(
    packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
) -> HashMap<PackageName, Vec<String>> {
    packages
        .map(|(pkg, info)| {
            (
                pkg.clone(),
                info.package_json.scripts.keys().cloned().collect(),
            )
        })
        .collect()
}

fn load_turbo_json_from_file(
    reader: &TurboJsonReader,
    turbo_json_path: LoadTurboJsonPath,
    is_root: bool,
) -> Result<TurboJson, LoaderError> {
    let result = match turbo_json_path {
        LoadTurboJsonPath::Dir(turbo_json_dir_path) => {
            let turbo_json_path = turbo_json_dir_path.join_component(CONFIG_FILE);
            let turbo_jsonc_path = turbo_json_dir_path.join_component(CONFIG_FILE_JSONC);

            // Load both turbo.json and turbo.jsonc
            let turbo_json = reader.read(&turbo_json_path, is_root);
            let turbo_jsonc = reader.read(&turbo_jsonc_path, is_root);

            select_turbo_json(turbo_json_dir_path, turbo_json, turbo_jsonc)
        }
        LoadTurboJsonPath::File(turbo_json_path) => reader.read(turbo_json_path, is_root),
    };

    // Handle errors or success
    match result {
        // There was an error, and we don't have any chance of recovering
        Err(e) => Err(LoaderError::TurboJson(e)),
        Ok(None) => Err(LoaderError::TurboJson(Error::NoTurboJSON)),
        // We're not synthesizing anything and there was no error, we're done
        Ok(Some(turbo)) => Ok(turbo),
    }
}

fn load_from_root_package_json(
    reader: &TurboJsonReader,
    turbo_json_path: &AbsoluteSystemPath,
    root_package_json: &PackageJson,
) -> Result<TurboJson, LoaderError> {
    let mut turbo_json = match reader.read(turbo_json_path, true) {
        // we're synthesizing, but we have a starting point
        // Note: this will have to change to support task inference in a monorepo
        // for now, we're going to error on any "root" tasks and turn non-root tasks into root
        // tasks
        Ok(Some(mut turbo_json)) => {
            let mut pipeline = Pipeline::default();
            for (task_name, task_definition) in turbo_json.tasks {
                if task_name.is_package_task() {
                    let (span, text) = task_definition.span_and_text("turbo.json");

                    return Err(LoaderError::TurboJson(
                        Error::PackageTaskInSinglePackageMode {
                            task_id: task_name.to_string(),
                            span,
                            text,
                        },
                    ));
                }

                pipeline.insert(task_name.into_root_task(), task_definition);
            }

            turbo_json.tasks = pipeline;

            turbo_json
        }
        // turbo.json doesn't exist, but we're going try to synthesize something
        Ok(None) => TurboJson::default(),
        // some other happened, we can't recover
        Err(e) => {
            return Err(LoaderError::TurboJson(e));
        }
    };

    // TODO: Add location info from package.json
    for script_name in root_package_json.scripts.keys() {
        let task_name = TaskName::from(script_name.as_str());
        if !turbo_json.has_task(&task_name) {
            let task_name = task_name.into_root_task();
            // Explicitly set cache to Some(false) in this definition
            // so we can pretend it was set on purpose. That way it
            // won't get clobbered by the merge function.
            turbo_json.tasks.insert(
                task_name,
                Spanned::new(RawTaskDefinition {
                    cache: Some(Spanned::new(false)),
                    ..RawTaskDefinition::default()
                }),
            );
        }
    }

    Ok(turbo_json)
}

fn root_turbo_json_from_scripts(scripts: &[String]) -> Result<TurboJson, LoaderError> {
    let mut turbo_json = TurboJson::default();
    for script in scripts {
        let task_name = TaskName::from(script.as_str()).into_root_task();
        turbo_json.tasks.insert(
            task_name,
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                env_mode: Some(Spanned::new(EnvMode::Loose)),
                ..Default::default()
            }),
        );
    }
    Ok(turbo_json)
}

fn workspace_turbo_json_from_scripts(scripts: &[String]) -> Result<TurboJson, LoaderError> {
    let mut turbo_json = TurboJson {
        extends: Spanned::new(vec!["//".to_owned()]),
        ..TurboJson::default()
    };
    for script in scripts {
        let task_name = TaskName::from(script.clone());
        turbo_json.tasks.insert(
            task_name,
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                env_mode: Some(Spanned::new(EnvMode::Loose)),
                ..Default::default()
            }),
        );
    }
    Ok(turbo_json)
}

fn load_task_access_trace_turbo_json(
    reader: &TurboJsonReader,
    turbo_json_path: &AbsoluteSystemPath,
    root_package_json: &PackageJson,
) -> Result<TurboJson, LoaderError> {
    let trace_json_path = reader.repo_root().join_components(&TASK_ACCESS_CONFIG_PATH);
    let turbo_from_trace = reader.read(&trace_json_path, true);

    // check the zero config case (turbo trace file, but no turbo.json file)
    if let Ok(Some(turbo_from_trace)) = turbo_from_trace
        && !turbo_json_path.exists()
    {
        debug!("Using turbo.json synthesized from trace file");
        return Ok(turbo_from_trace);
    }
    load_from_root_package_json(reader, turbo_json_path, root_package_json)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::Path};

    use insta::assert_snapshot;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::AnchoredSystemPathBuf;

    use super::*;

    /// Helper to create a PackageInfo with the given package path relative to
    /// repo root
    fn make_package_info(repo_root: &AbsoluteSystemPath, pkg_path: &Path) -> PackageInfo {
        let relative_path = pkg_path
            .strip_prefix(repo_root.as_std_path())
            .unwrap_or(pkg_path);
        let package_json_relative = relative_path.join("package.json");
        let package_json_path =
            AnchoredSystemPathBuf::try_from(package_json_relative.to_str().unwrap()).unwrap();
        PackageInfo {
            package_json_path,
            ..PackageInfo::default()
        }
    }

    // =========================================================================
    // Basic load_from_path tests
    // =========================================================================

    #[test]
    fn test_load_from_path_with_both_files() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create both turbo.json and turbo.jsonc
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);

        fs::write(&turbo_json_path, "{}").unwrap();
        fs::write(&turbo_jsonc_path, "{}").unwrap();

        // Should error when both files exist
        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::MultipleTurboConfigs { .. }
        ));
    }

    #[test]
    fn test_load_from_path_with_only_turbo_json() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create only turbo.json
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        fs::write(&turbo_json_path, "{}").unwrap();

        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_from_path_with_only_turbo_jsonc() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create only turbo.jsonc
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);
        fs::write(&turbo_jsonc_path, "{}").unwrap();

        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_load_from_path_no_file() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        let result = load_from_path(&reader, TurboJsonPath::Dir(repo_root), true);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::NoTurboJSON));
    }

    #[test]
    fn test_loader_error_is_no_turbo_json() {
        let err = LoaderError::TurboJson(Error::NoTurboJSON);
        assert!(err.is_no_turbo_json());

        let err = LoaderError::InvalidTurboJsonLoad(PackageName::from("test"));
        assert!(!err.is_no_turbo_json());
    }

    // =========================================================================
    // TurboJsonLoader tests
    // =========================================================================

    /// Helper to create TurboJson with global_deps set
    fn turbo_json_with_global_deps(deps: Vec<String>) -> TurboJson {
        TurboJson {
            global_deps: deps,
            ..TurboJson::default()
        }
    }

    /// Helper to create TurboJson with global_pass_through_env set
    fn turbo_json_with_pass_through_env(env: Vec<String>) -> TurboJson {
        TurboJson {
            global_pass_through_env: Some(env),
            ..TurboJson::default()
        }
    }

    #[test_case(r"{}", TurboJson::default() ; "empty")]
    #[test_case(
        r#"{ "globalDependencies": ["tsconfig.json", "jest.config.ts"] }"#,
        turbo_json_with_global_deps(vec!["jest.config.ts".to_string(), "tsconfig.json".to_string()])
    ; "global dependencies (sorted)")]
    #[test_case(
        r#"{ "globalPassThroughEnv": ["GITHUB_TOKEN", "AWS_SECRET_KEY"] }"#,
        turbo_json_with_pass_through_env(vec!["AWS_SECRET_KEY".to_string(), "GITHUB_TOKEN".to_string()])
    )]
    #[test_case(r#"{ "//": "A comment"}"#, TurboJson::default() ; "faux comment")]
    #[test_case(r#"{ "//": "A comment", "//": "Another comment" }"#, TurboJson::default() ; "two faux comments")]
    fn test_get_root_turbo_no_synthesizing(
        turbo_json_content: &str,
        expected_turbo_json: TurboJson,
    ) {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component("turbo.json");
        fs::write(&root_turbo_json, turbo_json_content).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader =
            TurboJsonLoader::<NoOpUpdater>::workspace(reader, root_turbo_json, std::iter::empty());

        let mut turbo_json = loader.load(&PackageName::Root).unwrap().clone();

        turbo_json.clear_metadata();
        assert_eq!(turbo_json, expected_turbo_json);
    }

    /// Helper to create TurboJson with tasks set
    fn turbo_json_with_tasks(tasks: Pipeline) -> TurboJson {
        TurboJson {
            tasks,
            ..TurboJson::default()
        }
    }

    #[test_case(
        None,
        PackageJson {
             scripts: [("build".to_string(), Spanned::new("echo build".to_string()))].into_iter().collect(),
             ..PackageJson::default()
        },
        turbo_json_with_tasks(Pipeline([(
            "//#build".into(),
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                ..RawTaskDefinition::default()
            })
          )].into_iter().collect()
        ))
    )]
    #[test_case(
        Some(r#"{
            "tasks": {
                "build": {
                    "cache": true
                }
            }
        }"#),
        PackageJson {
             scripts: [("test".to_string(), Spanned::new("echo test".to_string()))].into_iter().collect(),
             ..PackageJson::default()
        },
        turbo_json_with_tasks(Pipeline([(
            "//#build".into(),
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(true).with_range(81..85)),
                ..RawTaskDefinition::default()
            }).with_range(50..103)
        ),
        (
            "//#test".into(),
            Spanned::new(RawTaskDefinition {
                 cache: Some(Spanned::new(false)),
                ..RawTaskDefinition::default()
            })
        )].into_iter().collect()))
    )]
    fn test_get_root_turbo_with_synthesizing(
        turbo_json_content: Option<&str>,
        root_package_json: PackageJson,
        expected_turbo_json: TurboJson,
    ) {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component(CONFIG_FILE);

        if let Some(content) = turbo_json_content {
            fs::write(&root_turbo_json, content).unwrap();
        }

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::single_package(
            reader,
            root_turbo_json,
            root_package_json,
        );
        let mut turbo_json = loader.load(&PackageName::Root).unwrap().clone();
        turbo_json.clear_metadata();
        for (_, task_definition) in turbo_json.tasks.iter_mut() {
            task_definition.path = None;
            task_definition.text = None;
        }
        assert_eq!(turbo_json, expected_turbo_json);
    }

    #[test]
    fn test_single_package_loading_non_root() {
        let junk_path = AbsoluteSystemPath::new(if cfg!(windows) {
            "C:\\never\\loaded"
        } else {
            "/never/loaded"
        })
        .unwrap();
        let non_root = PackageName::from("some-pkg");
        let reader = TurboJsonReader::new(junk_path.to_owned());
        let single_loader = TurboJsonLoader::<NoOpUpdater>::single_package(
            reader.clone(),
            junk_path.to_owned(),
            PackageJson::default(),
        );
        let task_access_loader = TurboJsonLoader::<NoOpUpdater>::task_access(
            reader,
            junk_path.to_owned(),
            PackageJson::default(),
        );

        for loader in [single_loader, task_access_loader] {
            let result = loader.load(&non_root);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                matches!(err, LoaderError::InvalidTurboJsonLoad(_)),
                "expected {err} to be InvalidTurboJsonLoad"
            );
        }
    }

    #[test]
    fn test_workspace_turbo_json_loading() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(r#"{"tasks": {}}"#)
            .unwrap();

        let a_turbo_json_dir = repo_root.join_components(&["packages", "a"]);
        a_turbo_json_dir.create_dir_all().unwrap();

        // Create package info for "a"
        let pkg_a_info = make_package_info(repo_root, a_turbo_json_dir.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("a"), &pkg_a_info)].into_iter(),
        );
        let result = loader.load(&PackageName::from("a"));
        assert!(
            result.unwrap_err().is_no_turbo_json(),
            "expected parsing to fail with missing turbo.json"
        );

        let a_turbo_json = a_turbo_json_dir.join_component("turbo.json");
        a_turbo_json
            .create_with_contents(r#"{"extends": ["//"], "tasks": {"build": {}}}"#)
            .unwrap();

        let turbo_json = loader.load(&PackageName::from("a")).unwrap();
        assert_eq!(turbo_json.tasks.len(), 1);
    }

    #[test]
    fn test_turbo_json_caching() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(r#"{"tasks": {}}"#)
            .unwrap();

        let a_turbo_json_dir = repo_root.join_components(&["packages", "a"]);
        a_turbo_json_dir.create_dir_all().unwrap();
        let a_turbo_json = a_turbo_json_dir.join_component("turbo.json");

        // Create package info for "a"
        let pkg_a_info = make_package_info(repo_root, a_turbo_json_dir.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("a"), &pkg_a_info)].into_iter(),
        );
        a_turbo_json
            .create_with_contents(r#"{"extends": ["//"], "tasks": {"build": {}}}"#)
            .unwrap();

        let turbo_json = loader.load(&PackageName::from("a")).unwrap();
        assert_eq!(turbo_json.tasks.len(), 1);
        a_turbo_json.remove().unwrap();
        // Should still succeed due to caching
        assert!(loader.load(&PackageName::from("a")).is_ok());
    }

    #[test]
    fn test_no_turbo_json() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        // Create stub package info with scripts
        let root_pkg_json = PackageJson {
            scripts: BTreeMap::from([
                ("build".to_string(), Spanned::new("echo build".to_string())),
                ("lint".to_string(), Spanned::new("echo lint".to_string())),
                ("test".to_string(), Spanned::new("echo test".to_string())),
            ]),
            ..PackageJson::default()
        };
        let root_info = PackageInfo {
            package_json: root_pkg_json,
            ..PackageInfo::default()
        };

        let pkg_a_json = PackageJson {
            scripts: BTreeMap::from([
                ("build".to_string(), Spanned::new("echo build".to_string())),
                ("lint".to_string(), Spanned::new("echo lint".to_string())),
                (
                    "special".to_string(),
                    Spanned::new("echo special".to_string()),
                ),
            ]),
            ..PackageJson::default()
        };
        let pkg_a_info = PackageInfo {
            package_json: pkg_a_json,
            ..PackageInfo::default()
        };

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace_no_turbo_json(
            reader,
            vec![
                (&PackageName::Root, &root_info),
                (&PackageName::from("pkg-a"), &pkg_a_info),
            ]
            .into_iter(),
        );

        {
            let root_json = loader.load(&PackageName::Root).unwrap();
            for task_name in ["//#build", "//#lint", "//#test"] {
                if let Some(def) = root_json.tasks.get(&TaskName::from(task_name)) {
                    assert_eq!(
                        def.cache.as_ref().map(|cache| *cache.as_inner()),
                        Some(false)
                    );
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }

        {
            let pkg_a_json = loader.load(&PackageName::from("pkg-a")).unwrap();
            for task_name in ["build", "lint", "special"] {
                if let Some(def) = pkg_a_json.tasks.get(&TaskName::from(task_name)) {
                    assert_eq!(
                        def.cache.as_ref().map(|cache| *cache.as_inner()),
                        Some(false)
                    );
                } else {
                    panic!("didn't find {task_name}");
                }
            }
        }
        // Should get no turbo.json error if package wasn't declared
        let goose_err = loader.load(&PackageName::from("goose")).unwrap_err();
        assert!(goose_err.is_no_turbo_json());
    }

    #[test]
    fn test_load_from_file_with_both_files_error_message() {
        let tmp_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();
        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Create both turbo.json and turbo.jsonc
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);

        turbo_json_path.create_with_contents("{}").unwrap();
        turbo_jsonc_path.create_with_contents("{}").unwrap();

        // Test load_turbo_json_from_file with turbo.json path
        let result = load_turbo_json_from_file(&reader, LoadTurboJsonPath::Dir(repo_root), true);

        // The function should return an error when both files exist
        assert!(result.is_err());
        let mut err = result.unwrap_err();
        // Override tmpdir so we can snapshot the error message
        if let LoaderError::TurboJson(Error::MultipleTurboConfigs { directory }) = &mut err {
            *directory = "some-dir".to_owned()
        }
        assert_snapshot!(err, @r"
        Found both turbo.json and turbo.jsonc in the same directory: some-dir
        Remove either turbo.json or turbo.jsonc so there is only one.
        ");
    }

    #[test]
    fn test_context_aware_parsing() {
        // Test that the reader correctly determines root vs package contexts
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        // Create a root turbo.json with root-only fields
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(
                r#"{
            "globalEnv": ["NODE_ENV"],
            "tasks": {"build": {}}
        }"#,
            )
            .unwrap();

        // Create a package turbo.json with extends
        let pkg_dir = repo_root.join_components(&["packages", "foo"]);
        pkg_dir.create_dir_all().unwrap();
        let pkg_turbo_json = pkg_dir.join_component("turbo.json");
        pkg_turbo_json
            .create_with_contents(
                r#"{
            "extends": ["//"],
            "tasks": {"test": {}}
        }"#,
            )
            .unwrap();

        let reader = TurboJsonReader::new(repo_root.to_owned());

        // Reading root turbo.json should work with globalEnv
        let root_result = reader.read(&root_turbo_json, true);
        assert!(root_result.is_ok());
        let root_json = root_result.unwrap().unwrap();
        assert!(!root_json.global_env.is_empty());

        // Reading package turbo.json should work with extends
        let pkg_result = reader.read(&pkg_turbo_json, false);
        assert!(pkg_result.is_ok());
        let pkg_json = pkg_result.unwrap().unwrap();
        assert!(!pkg_json.extends.is_empty());

        // Now test invalid cases
        // Root turbo.json with extends should fail
        root_turbo_json
            .create_with_contents(
                r#"{
            "extends": ["//"],
            "tasks": {"build": {}}
        }"#,
            )
            .unwrap();
        let invalid_root = reader.read(&root_turbo_json, true);
        assert!(invalid_root.is_err());

        // Package turbo.json with globalEnv should fail
        pkg_turbo_json
            .create_with_contents(
                r#"{
            "globalEnv": ["NODE_ENV"],
            "tasks": {"test": {}}
        }"#,
            )
            .unwrap();
        let invalid_pkg = reader.read(&pkg_turbo_json, false);
        assert!(invalid_pkg.is_err());
    }

    #[test]
    fn test_invalid_workspace_turbo_json() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(r#"{"tasks": {}}"#)
            .unwrap();

        let a_turbo_json_dir = repo_root.join_components(&["packages", "a"]);
        a_turbo_json_dir.create_dir_all().unwrap();
        let a_turbo_json = a_turbo_json_dir.join_component("turbo.json");
        a_turbo_json
            .create_with_contents(r#"{"extends": ["//"], "tasks": {"build": {"lol": true}}}"#)
            .unwrap();

        let pkg_a_info = make_package_info(repo_root, a_turbo_json_dir.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("a"), &pkg_a_info)].into_iter(),
        );
        let result = loader.load(&PackageName::from("a"));
        assert!(
            matches!(result.unwrap_err(), LoaderError::TurboJson(_)),
            "expected parsing to fail due to unknown key"
        );
    }

    #[test]
    fn test_package_at_repo_root_uses_root_schema() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        // Create a root turbo.json WITHOUT extends (valid for root, invalid for
        // package)
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(
                r#"{
                "tasks": {
                    "build": {},
                    "test": { "cache": false }
                }
            }"#,
            )
            .unwrap();

        // Create a package that lives at the repo root
        // This simulates `pnpm-workspace.yaml` containing "." as a package
        let pkg_my_app_info = make_package_info(repo_root, repo_root.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("my-app"), &pkg_my_app_info)].into_iter(),
        );

        let root_result = loader.load(&PackageName::Root);
        assert!(
            root_result.is_ok(),
            "Loading root turbo.json for Root package should succeed: {:?}",
            root_result.err()
        );

        let pkg_result = loader.load(&PackageName::from("my-app"));
        assert!(
            pkg_result.is_ok(),
            "Loading root turbo.json for package at repo root should succeed: {:?}",
            pkg_result.err()
        );

        // Verify both loaded the same config
        let root_json = root_result.unwrap();
        let pkg_json = pkg_result.unwrap();
        assert_eq!(
            root_json.tasks.len(),
            pkg_json.tasks.len(),
            "Both should have the same tasks"
        );
    }

    #[test]
    fn test_package_at_repo_root_allows_root_only_fields() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(
                r#"{
                "globalEnv": ["NODE_ENV", "CI"],
                "tasks": {
                    "build": {}
                }
            }"#,
            )
            .unwrap();

        let pkg_my_app_info = make_package_info(repo_root, repo_root.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("my-app"), &pkg_my_app_info)].into_iter(),
        );

        let pkg_result = loader.load(&PackageName::from("my-app"));
        assert!(
            pkg_result.is_ok(),
            "Package at repo root should allow root-only fields like globalEnv: {:?}",
            pkg_result.err()
        );

        // Verify globalEnv was parsed
        let pkg_json = pkg_result.unwrap();
        assert!(
            !pkg_json.global_env.is_empty(),
            "globalEnv should be parsed for package at repo root"
        );
    }

    /// Test that regular packages (not at repo root) use package schema.
    /// A package turbo.json with root-only fields (like globalEnv) should fail
    /// because it uses the package schema which doesn't allow those fields.
    #[test]
    fn test_regular_package_uses_package_schema() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(r#"{ "tasks": { "build": {} } }"#)
            .unwrap();

        let pkg_dir = repo_root.join_components(&["packages", "my-pkg"]);
        pkg_dir.create_dir_all().unwrap();
        let pkg_turbo_json = pkg_dir.join_component("turbo.json");
        pkg_turbo_json
            .create_with_contents(
                r#"{
                "extends": ["//"],
                "globalEnv": ["NODE_ENV"],
                "tasks": {
                    "build": { "cache": false }
                }
            }"#,
            )
            .unwrap();

        let pkg_my_pkg_info = make_package_info(repo_root, pkg_dir.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("my-pkg"), &pkg_my_pkg_info)].into_iter(),
        );

        let pkg_result = loader.load(&PackageName::from("my-pkg"));
        assert!(
            pkg_result.is_err(),
            "Package turbo.json with root-only fields (globalEnv) should fail parsing"
        );
    }

    /// Test that symlinks to repo root are correctly detected as root packages.
    /// This prevents symlink-based bypasses of package-level validation.
    #[cfg(unix)]
    #[test]
    fn test_symlink_to_repo_root_detected_as_root() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();

        // Create a root turbo.json with root-only fields
        let root_turbo_json = repo_root.join_component("turbo.json");
        root_turbo_json
            .create_with_contents(
                r#"{
                "globalEnv": ["NODE_ENV"],
                "tasks": {
                    "build": {}
                }
            }"#,
            )
            .unwrap();

        // Create a packages directory with a symlink to the repo root
        let packages_dir = repo_root.join_component("packages");
        packages_dir.create_dir_all().unwrap();
        let symlink_pkg = packages_dir.join_component("symlink-pkg");
        symlink_pkg.symlink_to_dir(repo_root.as_str()).unwrap();

        // Verify the symlink was created correctly
        assert!(symlink_pkg.exists(), "Symlink should exist");

        let pkg_symlink_info = make_package_info(repo_root, symlink_pkg.as_std_path());

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader = TurboJsonLoader::<NoOpUpdater>::workspace(
            reader,
            root_turbo_json,
            vec![(&PackageName::from("symlink-pkg"), &pkg_symlink_info)].into_iter(),
        );

        // Loading the symlinked package should succeed because the symlink
        // points to the repo root, and we should detect this via canonicalization
        let pkg_result = loader.load(&PackageName::from("symlink-pkg"));
        assert!(
            pkg_result.is_ok(),
            "Symlink to repo root should be treated as root and allow globalEnv: {:?}",
            pkg_result.err()
        );

        // Verify globalEnv was parsed (confirming root schema was used)
        let pkg_json = pkg_result.unwrap();
        assert!(
            !pkg_json.global_env.is_empty(),
            "globalEnv should be parsed when symlink points to repo root"
        );
    }

    #[test]
    fn test_task_access_loading_both_missing() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component(CONFIG_FILE);

        let mut scripts = BTreeMap::new();
        scripts.insert("build".into(), Spanned::new("echo building".into()));
        let root_package_json = PackageJson {
            scripts,
            ..Default::default()
        };

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader =
            TurboJsonLoader::<NoOpUpdater>::task_access(reader, root_turbo_json, root_package_json);
        let turbo_json = loader.load(&PackageName::Root).unwrap();
        let root_build = turbo_json
            .tasks
            .get(&TaskName::from("//#build"))
            .expect("root build should always exist")
            .as_inner();

        // When both are missing, cache should be false (synthesized)
        assert_eq!(
            root_build.cache.as_ref().map(|c| *c.as_inner()),
            Some(false)
        );
    }

    #[test]
    fn test_task_access_loading_trace_only() {
        let root_dir = tempdir().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path()).unwrap();
        let root_turbo_json = repo_root.join_component(CONFIG_FILE);

        // Create trace file but no turbo.json
        let trace_path = repo_root.join_components(&TASK_ACCESS_CONFIG_PATH);
        trace_path.ensure_dir().unwrap();
        trace_path
            .create_with_contents(r#"{ "tasks": {"//#build": {"env": ["SPECIAL_VAR"]}} }"#)
            .unwrap();

        let mut scripts = BTreeMap::new();
        scripts.insert("build".into(), Spanned::new("echo building".into()));
        let root_package_json = PackageJson {
            scripts,
            ..Default::default()
        };

        let reader = TurboJsonReader::new(repo_root.to_owned());
        let loader =
            TurboJsonLoader::<NoOpUpdater>::task_access(reader, root_turbo_json, root_package_json);
        let turbo_json = loader.load(&PackageName::Root).unwrap();
        let root_build = turbo_json
            .tasks
            .get(&TaskName::from("//#build"))
            .expect("root build should always exist")
            .as_inner();

        // Should use trace file config
        assert!(root_build.env.is_some());
    }
}
