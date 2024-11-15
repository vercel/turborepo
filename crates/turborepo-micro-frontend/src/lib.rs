#![deny(clippy::all)]
mod configv1;
mod error;

use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use configv1::ConfigV1;
pub use error::Error;
use serde::Serialize;
use turbopath::AbsoluteSystemPath;

/// Currently the default path for a package that provides a configuration.
///
/// This is subject to change at any time.
pub const DEFAULT_MICRO_FRONTENDS_CONFIG: &str = "micro-frontends.jsonc";
pub const MICRO_FRONTENDS_PACKAGES: &[&str] = [
    MICRO_FRONTENDS_PACKAGE_EXTERNAL,
    MICRO_FRONTENDS_PACKAGE_INTERNAL,
]
.as_slice();
pub const MICRO_FRONTENDS_PACKAGE_INTERNAL: &str = "@vercel/micro-frontends-internal";
pub const MICRO_FRONTENDS_PACKAGE_EXTERNAL: &str = "@vercel/microfrontends";
pub const SUPPORTED_VERSIONS: &[&str] = ["1"].as_slice();

/// The minimal amount of information Turborepo needs to correctly start a local
/// proxy server for microfrontends
#[derive(Debug, PartialEq, Eq)]
pub enum Config {
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
            None => "1".to_string(),
        };

        match version.as_str() {
            "1" => ConfigV1::from_str(input, source).map(Config::V1),
            version => Err(Error::UnsupportedVersion(version.to_string())),
        }
    }

    pub fn applications(&self) -> impl Iterator<Item = (&String, &Application)> {
        match self {
            Config::V1(config_v1) => config_v1.applications(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct Application {
    pub development: Development,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct Development {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

#[cfg(test)]
mod test {
    use insta::assert_snapshot;

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
        assert_snapshot!(err, @r###"Unsupported micro-frontends configuration version: yolo. Supported versions: ["1"]"###);
    }
}
