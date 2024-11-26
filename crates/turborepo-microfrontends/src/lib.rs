#![feature(assert_matches)]
#![deny(clippy::all)]
mod configv1;
mod configv2;
mod error;

use std::io;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use configv1::ConfigV1;
use configv2::ConfigV2;
pub use error::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

/// Currently the default path for a package that provides a configuration.
///
/// This is subject to change at any time.
pub const DEFAULT_MICROFRONTENDS_CONFIG_V1: &str = "micro-frontends.jsonc";
pub const DEFAULT_MICROFRONTENDS_CONFIG_V2: &str = "microfrontends.json";
pub const DEFAULT_MICROFRONTENDS_CONFIG_V2_ALT: &str = "microfrontends.jsonc";
pub const MICROFRONTENDS_PACKAGES: &[&str] = [
    MICROFRONTENDS_PACKAGE_EXTERNAL,
    MICROFRONTENDS_PACKAGE_INTERNAL,
]
.as_slice();
pub const MICROFRONTENDS_PACKAGE_INTERNAL: &str = "@vercel/micro-frontends-internal";
pub const MICROFRONTENDS_PACKAGE_EXTERNAL: &str = "@vercel/microfrontends";
pub const SUPPORTED_VERSIONS: &[&str] = ["1", "2"].as_slice();

/// The minimal amount of information Turborepo needs to correctly start a local
/// proxy server for microfrontends
#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    inner: ConfigInner,
    filename: String,
}

#[derive(Debug, PartialEq, Eq)]
enum ConfigInner {
    V1(ConfigV1),
    V2(ConfigV2),
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

    pub fn load_from_dir(dir: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        if let Some(config) = Self::load_v2_dir(dir)? {
            Ok(Some(config))
        } else {
            Self::load_v1_dir(dir)
        }
    }

    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        #[derive(Deserializable, Default)]
        struct VersionOnly {
            version: String,
        }
        let (version_only, _errs) = biome_deserialize::json::deserialize_from_json_str(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();

        let version = match version_only {
            Some(VersionOnly { version }) => version,
            // Default to version 1 if no version found
            None => "2".to_string(),
        };

        let inner = match version.as_str() {
            "1" => ConfigV1::from_str(input, source).map(ConfigInner::V1),
            "2" => ConfigV2::from_str(input, source).and_then(|result| match result {
                configv2::ParseResult::Actual(config_v2) => Ok(ConfigInner::V2(config_v2)),
                configv2::ParseResult::Reference(default_app) => Err(Error::ChildConfig {
                    reference: default_app,
                }),
            }),
            version => Err(Error::UnsupportedVersion(version.to_string())),
        }?;
        Ok(Self {
            inner,
            filename: source.to_owned(),
        })
    }

    pub fn development_tasks<'a>(&'a self) -> Box<dyn Iterator<Item = (&str, Option<&str>)> + 'a> {
        match &self.inner {
            ConfigInner::V1(config_v1) => Box::new(config_v1.development_tasks()),
            ConfigInner::V2(config_v2) => Box::new(config_v2.development_tasks()),
        }
    }

    /// Filename of the loaded configuration
    pub fn filename(&self) -> &str {
        &self.filename
    }

    fn load_v2_dir(dir: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let load_config =
            |filename: &str| -> Option<(Result<String, io::Error>, AbsoluteSystemPathBuf)> {
                let path = dir.join_component(filename);
                let contents = path.read_existing_to_string().transpose()?;
                Some((contents, path))
            };
        let Some((contents, path)) = load_config(DEFAULT_MICROFRONTENDS_CONFIG_V2)
            .or_else(|| load_config(DEFAULT_MICROFRONTENDS_CONFIG_V2_ALT))
        else {
            return Ok(None);
        };
        let contents = contents?;

        ConfigV2::from_str(&contents, path.as_str())
            .and_then(|result| match result {
                configv2::ParseResult::Actual(config_v2) => Ok(Config {
                    inner: ConfigInner::V2(config_v2),
                    filename: path
                        .file_name()
                        .expect("microfrontends config should not be root")
                        .to_owned(),
                }),
                configv2::ParseResult::Reference(default_app) => Err(Error::ChildConfig {
                    reference: default_app,
                }),
            })
            .map(Some)
    }

    fn load_v1_dir(dir: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V1);
        let Some(contents) = path.read_existing_to_string()? else {
            return Ok(None);
        };

        ConfigV1::from_str(&contents, path.as_str())
            .map(|config_v1| Self {
                inner: ConfigInner::V1(config_v1),
                filename: DEFAULT_MICROFRONTENDS_CONFIG_V1.to_owned(),
            })
            .map(Some)
    }
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use insta::assert_snapshot;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_example_parses() {
        let input = include_str!("../fixtures/sample.jsonc");
        let example_config = Config::from_str(input, "something.json");
        assert!(example_config.is_ok());
    }

    #[test]
    fn test_unsupported_version() {
        let input = r#"{"version": "yolo"}"#;
        let err = Config::from_str(input, "something.json").unwrap_err();
        assert_snapshot!(err, @r###"Unsupported micro-frontends configuration version: yolo. Supported versions: ["1", "2"]"###);
    }

    fn add_v1_config(dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V1);
        path.create_with_contents(r#"{"version": "1", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    fn add_v2_config(dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V2);
        path.create_with_contents(r#"{"version": "2", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    fn add_v2_alt_config(dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let path = dir.join_component(DEFAULT_MICROFRONTENDS_CONFIG_V2_ALT);
        path.create_with_contents(r#"{"version": "2", "applications": {"web": {"development": {"task": "serve"}}, "docs": {}}}"#)
    }

    #[test]
    fn test_load_dir_v1() {
        let dir = TempDir::new().unwrap();
        let path = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        add_v1_config(path).unwrap();
        let config = Config::load_from_dir(path)
            .unwrap()
            .map(|config| config.inner);
        assert_matches!(config, Some(ConfigInner::V1(_)));
    }

    #[test]
    fn test_load_dir_v2() {
        let dir = TempDir::new().unwrap();
        let path = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        add_v2_config(path).unwrap();
        let config = Config::load_from_dir(path)
            .unwrap()
            .map(|config| config.inner);
        assert_matches!(config, Some(ConfigInner::V2(_)));
    }

    #[test]
    fn test_load_dir_both() {
        let dir = TempDir::new().unwrap();
        let path = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        add_v1_config(path).unwrap();
        add_v2_config(path).unwrap();
        let config = Config::load_from_dir(path)
            .unwrap()
            .map(|config| config.inner);
        assert_matches!(config, Some(ConfigInner::V2(_)));
    }

    #[test]
    fn test_load_dir_v2_alt() {
        let dir = TempDir::new().unwrap();
        let path = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        add_v2_alt_config(path).unwrap();
        let config = Config::load_from_dir(path)
            .unwrap()
            .map(|config| config.inner);
        assert_matches!(config, Some(ConfigInner::V2(_)));
    }

    #[test]
    fn test_load_dir_none() {
        let dir = TempDir::new().unwrap();
        let path = AbsoluteSystemPath::new(dir.path().to_str().unwrap()).unwrap();
        assert!(Config::load_from_dir(path).unwrap().is_none());
    }
}
