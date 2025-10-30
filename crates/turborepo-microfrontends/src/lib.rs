//! `@vercel/microfrontends` configuration parsing
//! This crate is only concerned with parsing the minimal amount of information
//! that Turborepo needs to correctly invoke a local proxy. This allows this
//! crate to avoid being kept in lock step with `@vercel/microfrontends`.
//!
//! The information required for the local proxy is the default package and the
//! package names that are a part of microfrontend and their development task
//! names.
//!
//! ## Architecture
//!
//! **Data Flow:**
//! 1. turborepo-lib loads configuration using
//!    `TurborepoMfeConfig::load_from_dir()`
//! 2. `TurborepoMfeConfig` only extracts Turborepo-relevant fields
//! 3. When starting the proxy, `TurborepoMfeConfig` is converted to `Config`
//!    via `into_config()`
//! 4. The proxy (`turborepo-microfrontends-proxy`) receives the full `Config`
//!    and can route requests
//! 5. Vercel-specific fields (asset_prefix, production, vercel config) are
//!    passed through but ignored by Turborepo

#![feature(assert_matches)]
#![deny(clippy::all)]
mod configv1;
mod error;
mod schema;

use std::io;

use configv1::ConfigV1;
pub use configv1::PathGroup;
pub use error::Error;
pub use schema::{TurborepoConfig, TurborepoDevelopment};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

/// Currently the default path for a package that provides a configuration.
///
/// This is subject to change at any time.
pub const DEFAULT_MICROFRONTENDS_CONFIG_V1: &str = "microfrontends.json";
pub const DEFAULT_MICROFRONTENDS_CONFIG_V1_ALT: &str = "microfrontends.jsonc";
pub const MICROFRONTENDS_PACKAGE: &str = "@vercel/microfrontends";
pub const SUPPORTED_VERSIONS: &[&str] = ["1"].as_slice();

/// Strict Turborepo-only configuration for the microfrontends proxy.
/// This configuration parser only accepts fields that Turborepo's native proxy
/// actually uses. Provider packages can extend this with additional fields as
/// needed.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TurborepoMfeConfig {
    inner: TurborepoConfig,
    config_v1: ConfigV1,
    filename: String,
    path: Option<AnchoredSystemPathBuf>,
}

impl TurborepoMfeConfig {
    /// Reads config from given path using strict Turborepo schema.
    /// Returns `Ok(None)` if the file does not exist
    pub fn load(config_path: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let Some(contents) = config_path.read_existing_to_string()? else {
            return Ok(None);
        };
        let config = Self::from_str(&contents, config_path.as_str())?;
        Ok(Some(config))
    }

    /// Attempts to load a configuration file from the given directory using
    /// strict schema Returns `Ok(None)` if no configuration is found in the
    /// directory
    pub fn load_from_dir(
        repo_root: &AbsoluteSystemPath,
        package_dir: &AnchoredSystemPath,
    ) -> Result<Option<Self>, Error> {
        Self::load_from_dir_with_mfe_dep(repo_root, package_dir, false)
    }

    /// Attempts to load a configuration file from the given directory
    /// If `has_mfe_dependency` is true, uses the lenient ConfigV1 parser
    /// Otherwise uses the strict Turborepo parser
    pub fn load_from_dir_with_mfe_dep(
        repo_root: &AbsoluteSystemPath,
        package_dir: &AnchoredSystemPath,
        has_mfe_dependency: bool,
    ) -> Result<Option<Self>, Error> {
        let absolute_dir = repo_root.resolve(package_dir);

        Config::validate_package_path(repo_root, &absolute_dir)?;

        let Some((contents, path)) = Self::load_v1_dir(&absolute_dir) else {
            return Ok(None);
        };
        let contents = contents?;
        let mut config = Self::from_str_with_mfe_dep(&contents, path.as_str(), has_mfe_dependency)?;
        config.filename = path
            .file_name()
            .expect("microfrontends config should not be root")
            .to_owned();
        config.set_path(package_dir);
        Ok(Some(config))
    }

    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        Self::from_str_with_mfe_dep(input, source, false)
    }

    /// Parses configuration from a string
    /// If `has_mfe_dependency` is true, uses the lenient ConfigV1 parser
    /// directly Otherwise tries the strict Turborepo parser only
    pub fn from_str_with_mfe_dep(
        input: &str,
        source: &str,
        has_mfe_dependency: bool,
    ) -> Result<Self, Error> {
        // If package has @vercel/microfrontends dependency, use lenient ConfigV1 parser
        if has_mfe_dependency {
            let config_v1_result = ConfigV1::from_str(input, source)?;
            match config_v1_result {
                configv1::ParseResult::Actual(config_v1) => {
                    return Ok(Self {
                        inner: TurborepoConfig::default(),
                        config_v1,
                        filename: source.to_owned(),
                        path: None,
                    });
                }
                configv1::ParseResult::Reference(default_app) => {
                    return Err(Error::ChildConfig {
                        reference: default_app,
                    });
                }
            }
        }

        // Without @vercel/microfrontends dependency, use strict Turborepo schema only
        let config = TurborepoConfig::from_str(input, source)?;
        Ok(Self {
            inner: config.clone(),
            config_v1: ConfigV1::from_turborepo_config(&config),
            filename: source.to_owned(),
            path: None,
        })
    }

    pub fn port(&self, name: &str) -> Option<u16> {
        // Prefer config_v1 for compatibility with lenient parsing
        self.config_v1.port(name)
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn path(&self) -> Option<&AnchoredSystemPath> {
        self.path.as_deref()
    }

    pub fn local_proxy_port(&self) -> Option<u16> {
        // Prefer config_v1 for compatibility with lenient parsing
        self.config_v1.local_proxy_port()
    }

    pub fn routing(&self, app_name: &str) -> Option<&[schema::PathGroup]> {
        // Return empty slice since config_v1::PathGroup is different from
        // schema::PathGroup This is only used for validation; actual routing
        // uses config_v1
        self.inner.routing(app_name)
    }

    pub fn fallback(&self, app_name: &str) -> Option<&str> {
        // Prefer config_v1 for compatibility with lenient parsing
        self.config_v1.fallback(app_name)
    }

    pub fn root_route_app(&self) -> Option<(&str, &str)> {
        // Prefer config_v1 for compatibility with lenient parsing
        self.config_v1.root_route_app()
    }

    pub fn development_tasks<'a>(&'a self) -> Box<dyn Iterator<Item = DevelopmentTask<'a>> + 'a> {
        Box::new(self.config_v1.development_tasks())
    }

    pub fn version(&self) -> &'static str {
        "1"
    }

    /// Converts this strict Turborepo config to a full Config for use by the
    /// proxy. This is needed because the proxy requires routing information
    /// to function.
    pub fn into_config(self) -> Config {
        Config {
            inner: ConfigInner::V1(self.config_v1),
            filename: self.filename,
            path: self.path,
        }
    }

    fn load_v1_dir(
        dir: &AbsoluteSystemPath,
    ) -> Option<(Result<String, io::Error>, AbsoluteSystemPathBuf)> {
        // Collect all matching files
        let mut matching_files = Vec::new();

        // Check for microfrontends*.json(c) files
        if let Ok(entries) = std::fs::read_dir(dir.as_path()) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && name.starts_with("microfrontends")
                    && (name.ends_with(".json") || name.ends_with(".jsonc"))
                {
                    matching_files.push(name.to_string());
                }
            }
        }

        // Error if multiple files found
        if matching_files.len() > 1 {
            matching_files.sort();
            return Some((
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Multiple microfrontends configuration files found: {matching_files:?}. \
                         Only one configuration file is allowed."
                    ),
                )),
                dir.to_owned(),
            ));
        }

        // Load the single matching file if found
        if let Some(filename) = matching_files.first() {
            let path = dir.join_component(filename);
            let contents = path.read_existing_to_string().transpose()?;
            return Some((contents, path));
        }

        None
    }

    pub fn set_path(&mut self, dir: &AnchoredSystemPath) {
        self.path = Some(dir.join_component(&self.filename));
    }
}

/// The minimal amount of information Turborepo needs to correctly start a local
/// proxy server for microfrontends
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Config {
    inner: ConfigInner,
    filename: String,
    path: Option<AnchoredSystemPathBuf>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct DevelopmentTask<'a> {
    // The key in the applications object in microfrontends.json
    // This will match package unless packageName is provided
    pub application_name: &'a str,
    pub package: &'a str,
    pub task: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct Application<'a> {
    // The key in the applications object in microfrontends.json
    pub application_name: &'a str,
    // The package name (either from packageName field or defaults to application_name)
    pub package: &'a str,
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum ConfigInner {
    V1(ConfigV1),
}

impl Config {
    /// Reads config from given path.
    /// Returns `Ok(None)` if the file does not exist
    pub fn load(config_path: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let Some(contents) = config_path.read_existing_to_string()? else {
            return Ok(None);
        };
        let config = Self::from_str(&contents, config_path.as_str())?;
        Ok(Some(config))
    }

    /// Validates that the resolved path is within the repository root
    pub fn validate_package_path(
        repo_root: &AbsoluteSystemPath,
        resolved_path: &AbsoluteSystemPath,
    ) -> Result<(), Error> {
        match resolved_path.to_realpath() {
            Ok(path) => {
                let root_real = repo_root
                    .to_realpath()
                    .map_err(|_| Error::PathTraversal(repo_root.to_string()))?;
                if !path.starts_with(&root_real) {
                    return Err(Error::PathTraversal(resolved_path.to_string()));
                }
                Ok(())
            }
            Err(_) => {
                let root_clean = repo_root
                    .clean()
                    .map_err(|_| Error::PathTraversal(repo_root.to_string()))?;
                let path_clean = resolved_path
                    .clean()
                    .map_err(|_| Error::PathTraversal(resolved_path.to_string()))?;

                if !path_clean.starts_with(&root_clean) {
                    return Err(Error::PathTraversal(resolved_path.to_string()));
                }
                Ok(())
            }
        }
    }

    /// Attempts to load a configuration file from the given directory
    /// Returns `Ok(None)` if no configuration is found in the directory
    pub fn load_from_dir(
        repo_root: &AbsoluteSystemPath,
        package_dir: &AnchoredSystemPath,
    ) -> Result<Option<Self>, Error> {
        let absolute_dir = repo_root.resolve(package_dir);

        Self::validate_package_path(repo_root, &absolute_dir)?;

        // we want to try different paths and then do `from_str`
        let Some((contents, path)) = Self::load_v1_dir(&absolute_dir) else {
            return Ok(None);
        };
        let contents = contents?;
        let mut config = Config::from_str(&contents, path.as_str())?;
        config.filename = path
            .file_name()
            .expect("microfrontends config should not be root")
            .to_owned();
        config.set_path(package_dir);
        Ok(Some(config))
    }

    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        let inner = ConfigV1::from_str(input, source).and_then(|result| match result {
            configv1::ParseResult::Actual(config_v1) => Ok(ConfigInner::V1(config_v1)),
            configv1::ParseResult::Reference(default_app) => Err(Error::ChildConfig {
                reference: default_app,
            }),
        })?;
        Ok(Self {
            inner,
            filename: source.to_owned(),
            path: None,
        })
    }

    pub fn development_tasks<'a>(&'a self) -> Box<dyn Iterator<Item = DevelopmentTask<'a>> + 'a> {
        match &self.inner {
            ConfigInner::V1(config_v1) => Box::new(config_v1.development_tasks()),
        }
    }

    pub fn applications<'a>(&'a self) -> Box<dyn Iterator<Item = Application<'a>> + 'a> {
        match &self.inner {
            ConfigInner::V1(config_v1) => Box::new(config_v1.applications()),
        }
    }

    pub fn port(&self, name: &str) -> Option<u16> {
        match &self.inner {
            ConfigInner::V1(config_v1) => config_v1.port(name),
        }
    }

    /// Filename of the loaded configuration
    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn path(&self) -> Option<&AnchoredSystemPath> {
        let path = self.path.as_deref()?;
        Some(path)
    }

    pub fn version(&self) -> &'static str {
        match &self.inner {
            ConfigInner::V1(_) => "1",
        }
    }

    pub fn local_proxy_port(&self) -> Option<u16> {
        match &self.inner {
            ConfigInner::V1(config_v1) => config_v1.local_proxy_port(),
        }
    }

    pub fn routing(&self, app_name: &str) -> Option<&[PathGroup]> {
        match &self.inner {
            ConfigInner::V1(config_v1) => config_v1.routing(app_name),
        }
    }

    pub fn fallback(&self, app_name: &str) -> Option<&str> {
        match &self.inner {
            ConfigInner::V1(config_v1) => config_v1.fallback(app_name),
        }
    }

    /// Returns the name and package of the application that serves the root
    /// route. The root route app is the one without explicit routing
    /// configuration.
    pub fn root_route_app(&self) -> Option<(&str, &str)> {
        match &self.inner {
            ConfigInner::V1(config_v1) => config_v1.root_route_app(),
        }
    }

    fn load_v1_dir(
        dir: &AbsoluteSystemPath,
    ) -> Option<(Result<String, io::Error>, AbsoluteSystemPathBuf)> {
        // Collect all matching files
        let mut matching_files = Vec::new();

        // Check for microfrontends*.json(c) files
        if let Ok(entries) = std::fs::read_dir(dir.as_path()) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && name.starts_with("microfrontends")
                    && (name.ends_with(".json") || name.ends_with(".jsonc"))
                {
                    matching_files.push(name.to_string());
                }
            }
        }

        // Error if multiple files found
        if matching_files.len() > 1 {
            matching_files.sort();
            return Some((
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Multiple microfrontends configuration files found: {matching_files:?}. \
                         Only one configuration file is allowed."
                    ),
                )),
                dir.to_owned(),
            ));
        }

        // Load the single matching file if found
        if let Some(filename) = matching_files.first() {
            let path = dir.join_component(filename);
            let contents = path.read_existing_to_string().transpose()?;
            return Some((contents, path));
        }

        None
    }

    /// Sets the path the configuration was loaded from
    pub fn set_path(&mut self, dir: &AnchoredSystemPath) {
        self.path = Some(dir.join_component(&self.filename));
    }
}

#[cfg(test)]
mod test {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_example_parses() {
        let input = include_str!("../fixtures/vercel-package.jsonc");
        let example_config = Config::from_str(input, "something.json");
        assert!(example_config.is_ok());
    }

    #[test]
    fn test_turborepo_strict_config_parses() {
        let input = include_str!("../fixtures/turborepo-only.jsonc");
        let strict_config = TurborepoMfeConfig::from_str(input, "something.jsonc");
        assert!(strict_config.is_ok());
    }

    #[test]
    fn test_unsupported_version() {
        let input = r#"{"version": "yolo"}"#;
        // Unsupported versions are now accepted if the structure is compatible.
        // This allows the Turborepo proxy to work with configs of any version.
        let config = Config::from_str(input, "something.json").expect("Config should parse");
        assert_eq!(config.filename(), "something.json");
    }

    fn add_v1_config(dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V1);
        path.ensure_dir()?;
        path.create_with_contents(r#"{"version": "1", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    fn add_v2_config(dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V1);
        path.ensure_dir()?;
        path.create_with_contents(r#"{"version": "2", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    fn add_v1_alt_config(dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V1_ALT);
        path.ensure_dir()?;
        path.create_with_contents(r#"{"version": "1", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    fn add_config_with_name(
        dir: &AbsoluteSystemPath,
        filename: &str,
    ) -> Result<(), std::io::Error> {
        let path = dir.join_component(filename);
        path.ensure_dir()?;
        path.create_with_contents(r#"{"version": "1", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    #[test]
    fn test_load_v1() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);
        add_v1_config(&pkg_path).unwrap();

        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.filename(), DEFAULT_MICROFRONTENDS_CONFIG_V1);
        assert_eq!(cfg.version(), "1");
    }

    #[test]
    fn test_load_v1_alt() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);
        add_v1_alt_config(&pkg_path).unwrap();

        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.filename(), DEFAULT_MICROFRONTENDS_CONFIG_V1_ALT);
        assert_eq!(cfg.version(), "1");
    }

    #[test]
    fn test_load_v1_custom_path() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);
        add_config_with_name(&pkg_path, "microfrontends-custom.json").unwrap();

        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.filename(), "microfrontends-custom.json");
        assert_eq!(cfg.version(), "1");
    }

    #[test]
    fn test_load_none() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);
        pkg_path.ensure_dir().unwrap();

        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn test_unsupported_version_from_dir() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);
        add_v2_config(&pkg_path).unwrap();
        let config = Config::load_from_dir(repo_root, pkg_dir);

        // Version 2 configs are now accepted if the structure is compatible.
        // This allows the Turborepo proxy to work with configs of any version.
        assert!(config.is_ok(), "Version 2 config should be accepted");
        let cfg = config.unwrap().expect("Config should be loaded");
        assert_eq!(cfg.version(), "1");
    }

    #[test]
    fn test_fallback_accessor() {
        let input = r#"{
        "applications": {
          "web": {
            "development": {
              "local": 3000,
              "fallback": "web.example.com"
            }
          },
          "docs": {
            "development": {
              "local": 3001
            }
          }
        }
      }"#;
        let config = Config::from_str(input, "microfrontends.json").unwrap();

        assert_eq!(config.fallback("web"), Some("web.example.com"));
        assert_eq!(config.fallback("docs"), None);
        assert_eq!(config.fallback("nonexistent"), None);
    }

    #[test]
    fn test_path_traversal_protection() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();

        let outside_dir = TempDir::new().unwrap();
        let outside_path = AbsoluteSystemPath::new(outside_dir.path().to_str().unwrap()).unwrap();
        add_v1_config(outside_path).unwrap();

        let traversal_path = format!("../{}", outside_path.file_name().unwrap());
        let pkg_dir = AnchoredSystemPath::new(&traversal_path).unwrap();

        let result = Config::load_from_dir(repo_root, pkg_dir);

        assert!(result.is_err(), "Path traversal should be rejected");
        if let Err(Error::PathTraversal(_)) = result {
        } else {
            panic!("Expected PathTraversal error, got: {result:?}");
        }
    }

    #[test]
    fn test_valid_package_path() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();

        let pkg_dir = AnchoredSystemPath::new("packages/web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);
        add_v1_config(&pkg_path).unwrap();

        let result = Config::load_from_dir(repo_root, pkg_dir);

        assert!(result.is_ok(), "Valid path within repo should be accepted");
        assert!(result.unwrap().is_some(), "Config should be loaded");
    }

    #[test]
    fn test_multiple_config_files_error() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);

        // Add multiple config files
        add_v1_config(&pkg_path).unwrap();
        add_config_with_name(&pkg_path, "microfrontends-custom.json").unwrap();

        let result = Config::load_from_dir(repo_root, pkg_dir);

        assert!(
            result.is_err(),
            "Multiple config files should result in error"
        );
        if let Err(Error::Io(e)) = result {
            let msg = e.to_string();
            assert!(
                msg.contains("Multiple microfrontends configuration files found"),
                "Error message should mention multiple files, got: {}",
                msg
            );
        } else {
            panic!(
                "Expected Io error with multiple files message, got: {:?}",
                result
            );
        }
    }

    #[test]
    fn test_custom_named_config() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);

        // Add a custom named config file
        add_config_with_name(&pkg_path, "microfrontends-staging.jsonc").unwrap();

        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();

        assert!(config.is_some(), "Custom named config should be loaded");
        let cfg = config.unwrap();
        assert_eq!(cfg.filename(), "microfrontends-staging.jsonc");
    }

    #[test]
    fn test_file_without_hyphen_matched() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);

        // Add a file that starts with "microfrontends" but has no hyphen
        add_config_with_name(&pkg_path, "microfrontendsconfig.json").unwrap();

        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();

        assert!(config.is_some(), "Files without hyphen should be matched");
        assert_eq!(config.unwrap().filename(), "microfrontendsconfig.json");
    }

    #[test]
    fn test_exact_names_still_work() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);

        // Verify microfrontends.json still works
        add_v1_config(&pkg_path).unwrap();
        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();
        assert!(config.is_some());
        assert_eq!(config.unwrap().filename(), "microfrontends.json");
    }

    #[test]
    fn test_nested_config_not_found() {
        let dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_dir = AnchoredSystemPath::new("web").unwrap();
        let pkg_path = repo_root.resolve(pkg_dir);

        // Create a nested directory with a config file
        let nested_path = pkg_path.join_component("config");
        add_v1_config(&nested_path).unwrap();

        // Should not find the nested config
        let config = Config::load_from_dir(repo_root, pkg_dir).unwrap();
        assert!(config.is_none(), "Nested config files should not be found");
    }
}
