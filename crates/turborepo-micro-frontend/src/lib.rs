#![deny(clippy::all)]
mod error;

use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
pub use error::Error;
use serde::Serialize;
use turbopath::AbsoluteSystemPath;

/// Currently the default path for a package that provides a configuration.
///
/// This is subject to change at any time.
pub const DEFAULT_MICRO_FRONTENDS_CONFIG: &str = "micro-frontends.jsonc";
pub const MICRO_FRONTENDS_PACKAGES: &[&str] = ["@vercel/micro-frontends-internal"].as_slice();

/// The minimal amount of information Turborepo needs to correctly start a local
/// proxy server for microfrontends
#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct Config {
    pub version: String,
    pub applications: BTreeMap<String, Application>,
}

impl Config {
    /// Reads config from given path.
    /// Returns `Ok(None)` if the file does not exist
    pub fn load(config_path: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let Some(contents) = config_path.read_existing_to_string()? else {
            return Ok(None);
        };
        let config = Self::from_str(&contents, config_path.as_str()).map_err(Error::biome_error)?;
        Ok(Some(config))
    }

    pub fn from_str(input: &str, source: &str) -> Result<Self, Vec<biome_diagnostics::Error>> {
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();
        if let Some(config) = config {
            Ok(config)
        } else {
            Err(errs)
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
    use super::*;

    #[test]
    fn test_example_parses() {
        let input = include_str!("../fixtures/sample.jsonc");
        let example_config = Config::from_str(input, "something.json");
        assert!(example_config.is_ok());
    }
}
