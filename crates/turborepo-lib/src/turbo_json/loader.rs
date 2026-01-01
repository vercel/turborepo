//! TurboJson loading - re-exports from turborepo-turbo-json and MFE integration
//!
//! This module provides the TurboJsonLoader integration with
//! MicrofrontendsConfigs and a unified loader type that can handle both MFE and
//! non-MFE cases.

use std::collections::HashMap;

use turbopath::AbsoluteSystemPathBuf;
use turborepo_engine::BuilderError;
use turborepo_repository::{
    package_graph::{PackageInfo, PackageName},
    package_json::PackageJson,
};
// Re-export TurboJsonLoader and related types from turborepo-turbo-json
pub use turborepo_turbo_json::{
    LoaderError, NoOpUpdater, TurboJsonLoader, TurboJsonReader, TurboJsonUpdater,
};

use super::TurboJson;
use crate::{config::Error, microfrontends::MicrofrontendsConfigs};

/// Implement TurboJsonUpdater for MicrofrontendsConfigs
impl TurboJsonUpdater for MicrofrontendsConfigs {
    type Error = Error;

    fn update_turbo_json(
        &self,
        package_name: &PackageName,
        turbo_json: Result<TurboJson, LoaderError>,
    ) -> Result<TurboJson, Self::Error> {
        // Convert LoaderError to config::Error for compatibility
        let turbo_json = turbo_json.map_err(loader_error_to_config_error);
        MicrofrontendsConfigs::update_turbo_json(self, package_name, turbo_json)
    }
}

/// Convert LoaderError to config::Error
fn loader_error_to_config_error(err: LoaderError) -> Error {
    match err {
        LoaderError::TurboJson(e) => Error::TurboJsonError(e),
        LoaderError::InvalidTurboJsonLoad(pkg) => Error::InvalidTurboJsonLoad(pkg),
    }
}

/// Type alias for TurboJsonLoader with MicrofrontendsConfigs updater
pub type MfeTurboJsonLoader = TurboJsonLoader<MicrofrontendsConfigs>;

/// A unified TurboJson loader that can handle both MFE and non-MFE cases.
///
/// This enum wraps the generic `TurboJsonLoader` to provide a single type
/// that can be used in contexts where the loader type needs to be determined
/// at runtime (e.g., based on whether MFE configs are present).
#[derive(Debug, Clone)]
pub enum UnifiedTurboJsonLoader {
    /// Loader without MFE support (uses NoOpUpdater)
    Standard(TurboJsonLoader<NoOpUpdater>),
    /// Loader with MFE support
    WithMfe(TurboJsonLoader<MicrofrontendsConfigs>),
}

impl UnifiedTurboJsonLoader {
    /// Create a loader that will load turbo.json files throughout the workspace
    pub fn workspace<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
    ) -> Self {
        Self::Standard(TurboJsonLoader::workspace(
            reader,
            root_turbo_json_path,
            packages,
        ))
    }

    /// Create a loader that will load turbo.json files throughout the workspace
    /// with microfrontends support.
    pub fn workspace_with_microfrontends<'a>(
        reader: TurboJsonReader,
        root_turbo_json_path: AbsoluteSystemPathBuf,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        micro_frontends_configs: MicrofrontendsConfigs,
    ) -> Self {
        Self::WithMfe(TurboJsonLoader::workspace_with_updater(
            reader,
            root_turbo_json_path,
            packages,
            micro_frontends_configs,
        ))
    }

    /// Create a loader that will construct turbo.json structures based on
    /// workspace `package.json`s, with optional microfrontends support.
    pub fn workspace_no_turbo_json<'a>(
        reader: TurboJsonReader,
        packages: impl Iterator<Item = (&'a PackageName, &'a PackageInfo)>,
        microfrontends_configs: Option<MicrofrontendsConfigs>,
    ) -> Self {
        if let Some(mfe) = microfrontends_configs {
            Self::WithMfe(TurboJsonLoader::workspace_no_turbo_json_with_updater(
                reader,
                packages,
                Some(mfe),
            ))
        } else {
            Self::Standard(TurboJsonLoader::workspace_no_turbo_json(reader, packages))
        }
    }

    /// Create a loader that will load a root turbo.json or synthesize one if
    /// the file doesn't exist
    pub fn single_package(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self::Standard(TurboJsonLoader::single_package(
            reader,
            root_turbo_json,
            package_json,
        ))
    }

    /// Create a loader for task access tracing
    pub fn task_access(
        reader: TurboJsonReader,
        root_turbo_json: AbsoluteSystemPathBuf,
        package_json: PackageJson,
    ) -> Self {
        Self::Standard(TurboJsonLoader::task_access(
            reader,
            root_turbo_json,
            package_json,
        ))
    }

    /// Create a loader that will only return provided turbo.jsons and will
    /// never hit the file system.
    /// Primarily intended for testing
    pub fn noop(turbo_jsons: HashMap<PackageName, TurboJson>) -> Self {
        Self::Standard(TurboJsonLoader::noop(turbo_jsons))
    }

    /// Load a turbo.json for a given package
    pub fn load(&self, package: &PackageName) -> Result<&TurboJson, Error> {
        match self {
            Self::Standard(loader) => loader.load(package).map_err(loader_error_to_config_error),
            Self::WithMfe(loader) => loader.load(package),
        }
    }
}

// Implement the TurboJsonLoader trait from turborepo-engine for
// UnifiedTurboJsonLoader
impl turborepo_engine::TurboJsonLoader for UnifiedTurboJsonLoader {
    fn load(&self, package: &PackageName) -> Result<&TurboJson, BuilderError> {
        UnifiedTurboJsonLoader::load(self, package).map_err(BuilderError::from)
    }
}
