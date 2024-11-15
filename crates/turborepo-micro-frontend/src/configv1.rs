use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use serde::Serialize;

use crate::{Application, Error};

/// The minimal amount of information Turborepo needs to correctly start a local
/// proxy server for microfrontends
#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct ConfigV1 {
    pub version: String,
    pub applications: BTreeMap<String, Application>,
}

impl ConfigV1 {
    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();
        if let Some(config) = config {
            Ok(config)
        } else {
            Err(Error::biome_error(errs))
        }
    }

    pub fn applications(&self) -> impl Iterator<Item = (&String, &Application)> {
        self.applications.iter()
    }
}
