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
    use std::fs;

    use tempfile::tempdir;

    use super::*;

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
}
