//! turbo.json reader for configuration values
//!
//! This module reads configuration values from turbo.json files.
//!
//! NOTE: This module contains stub types for RawRemoteCacheOptions,
//! RawRootTurboJson, and RawTurboJson. When the turbo_json module is fully
//! extracted from turborepo-lib, these stubs will be replaced with the real
//! types.

use camino::Utf8PathBuf;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};

use crate::{
    Error,
    config::{ConfigurationOptions, ResolvedConfigurationOptions},
};

// =============================================================================
// Stub Types
// =============================================================================
// These are placeholder types that will be replaced when the full turbo_json
// module is extracted from turborepo-lib. For now, they allow the config
// building logic to compile.

/// Stub for RawRemoteCacheOptions from turbo_json module
/// TODO: Replace with real type when turbo_json is extracted
#[derive(Default)]
pub(crate) struct RawRemoteCacheOptions {
    pub api_url: Option<SpannedValue<String>>,
    pub login_url: Option<SpannedValue<String>>,
    pub team_slug: Option<SpannedValue<String>>,
    pub team_id: Option<SpannedValue<String>>,
    pub signature: Option<SpannedValue<bool>>,
    pub preflight: Option<SpannedValue<bool>>,
    pub timeout: Option<SpannedValue<u64>>,
    pub upload_timeout: Option<SpannedValue<u64>>,
    pub enabled: Option<SpannedValue<bool>>,
}

/// A simple wrapper type to mimic the spanned value pattern
/// TODO: Replace with real SpannedValue when turbo_json is extracted
pub(crate) struct SpannedValue<T>(T);

impl<T> SpannedValue<T> {
    pub fn as_inner(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> Clone for SpannedValue<T> {
    fn clone(&self) -> Self {
        SpannedValue(self.0.clone())
    }
}

/// Stub for RawTurboJson from turbo_json module
/// TODO: Replace with real type when turbo_json is extracted
#[derive(Default)]
pub(crate) struct RawTurboJson {
    pub remote_cache: Option<RawRemoteCacheOptions>,
    pub cache_dir: Option<SpannedCacheDir>,
    pub ui: Option<SpannedValue<crate::config::UIMode>>,
    pub allow_no_package_manager: Option<SpannedValue<bool>>,
    pub daemon: Option<SpannedValue<bool>>,
    pub env_mode: Option<SpannedValue<crate::config::EnvMode>>,
    pub concurrency: Option<SpannedValue<String>>,
    pub future_flags: Option<SpannedValue<crate::config::FutureFlags>>,
}

/// A stub for a spanned cache_dir value that supports conversion to &str
pub(crate) struct SpannedCacheDir(String);

impl std::ops::Deref for SpannedCacheDir {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl SpannedCacheDir {
    #[allow(dead_code)]
    pub fn span_and_text(
        &self,
        _file_name: &str,
    ) -> (Option<miette::SourceSpan>, miette::NamedSource<String>) {
        // Return a dummy span and source for error reporting
        (None, miette::NamedSource::new(_file_name, self.0.clone()))
    }
}

/// Stub for RawRootTurboJson from turbo_json module
/// TODO: Replace with real type when turbo_json is extracted
pub(crate) struct RawRootTurboJson;

impl RawRootTurboJson {
    /// Parse a turbo.json file contents
    /// TODO: Implement proper parsing when turbo_json is extracted
    #[allow(dead_code)]
    pub fn parse(_contents: &str, _path: &str) -> Result<Self, Error> {
        // For now, return a default empty struct
        // Real implementation will parse the JSON and return the proper type
        Ok(Self)
    }
}

impl From<RawRootTurboJson> for RawTurboJson {
    fn from(_root: RawRootTurboJson) -> Self {
        // For now, return default empty struct
        // Real implementation will convert root turbo.json to RawTurboJson
        RawTurboJson::default()
    }
}

// =============================================================================
// TurboJsonReader
// =============================================================================

pub struct TurboJsonReader<'a> {
    repo_root: &'a AbsoluteSystemPath,
}

impl From<&RawRemoteCacheOptions> for ConfigurationOptions {
    fn from(remote_cache_opts: &RawRemoteCacheOptions) -> Self {
        Self {
            api_url: remote_cache_opts
                .api_url
                .as_ref()
                .map(|s| s.as_inner().clone()),
            login_url: remote_cache_opts
                .login_url
                .as_ref()
                .map(|s| s.as_inner().clone()),
            team_slug: remote_cache_opts
                .team_slug
                .as_ref()
                .map(|s| s.as_inner().clone()),
            team_id: remote_cache_opts
                .team_id
                .as_ref()
                .map(|s| s.as_inner().clone()),
            signature: remote_cache_opts.signature.as_ref().map(|s| *s.as_inner()),
            preflight: remote_cache_opts.preflight.as_ref().map(|s| *s.as_inner()),
            timeout: remote_cache_opts.timeout.as_ref().map(|s| *s.as_inner()),
            upload_timeout: remote_cache_opts
                .upload_timeout
                .as_ref()
                .map(|s| *s.as_inner()),
            enabled: remote_cache_opts.enabled.as_ref().map(|s| *s.as_inner()),
            ..Self::default()
        }
    }
}

impl<'a> TurboJsonReader<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPath) -> Self {
        Self { repo_root }
    }

    fn turbo_json_to_config_options(
        turbo_json: RawTurboJson,
    ) -> Result<ConfigurationOptions, Error> {
        let mut opts = if let Some(remote_cache_options) = &turbo_json.remote_cache {
            remote_cache_options.into()
        } else {
            ConfigurationOptions::default()
        };

        let cache_dir = if let Some(cache_dir) = turbo_json.cache_dir {
            let cache_dir_str: &str = &cache_dir;
            let cache_dir_unix = RelativeUnixPath::new(cache_dir_str).map_err(|_| {
                let (span, text) = cache_dir.span_and_text("turbo.json");
                Error::AbsoluteCacheDir { span, text }
            })?;
            // Convert the relative unix path to an anchored system path
            // For unix/macos this is a no-op
            let cache_dir_system = cache_dir_unix.to_anchored_system_path_buf();
            Some(Utf8PathBuf::from(cache_dir_system.to_string()))
        } else {
            None
        };

        // Don't allow token to be set for shared config.
        opts.token = None;
        opts.ui = turbo_json.ui.map(|ui| *ui.as_inner());
        opts.allow_no_package_manager = turbo_json
            .allow_no_package_manager
            .map(|allow| *allow.as_inner());
        opts.daemon = turbo_json.daemon.map(|daemon| *daemon.as_inner());
        opts.env_mode = turbo_json.env_mode.map(|mode| *mode.as_inner());
        opts.cache_dir = cache_dir;
        opts.concurrency = turbo_json.concurrency.map(|c| c.as_inner().clone());
        opts.future_flags = turbo_json.future_flags.map(|f| f.as_inner().clone());
        Ok(opts)
    }
}

impl<'a> ResolvedConfigurationOptions for TurboJsonReader<'a> {
    fn get_configuration_options(
        &self,
        existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        let turbo_json_path = existing_config.root_turbo_json_path(self.repo_root)?;
        let root_relative_turbo_json_path = self.repo_root.anchor(&turbo_json_path).map_or_else(
            |_| turbo_json_path.as_str().to_owned(),
            |relative| relative.to_string(),
        );
        let turbo_json = match turbo_json_path.read_existing_to_string()? {
            Some(contents) => {
                RawRootTurboJson::parse(&contents, &root_relative_turbo_json_path)?.into()
            }
            None => RawTurboJson::default(),
        };
        Self::turbo_json_to_config_options(turbo_json)
    }
}

// NOTE: Tests are commented out for now because they depend on the real
// RawRootTurboJson::parse implementation which requires the full turbo_json
// module to be extracted.
//
// #[cfg(test)]
// mod test {
//     use serde_json::json;
//     use tempfile::tempdir;
//     use test_case::test_case;
//
//     use super::*;
//     use crate::config::CONFIG_FILE;
//
//     #[test]
//     fn test_reads_from_default() {
//         // ... test implementation
//     }
// }
