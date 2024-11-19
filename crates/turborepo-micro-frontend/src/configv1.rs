use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use serde::Serialize;

use crate::Error;

/// The minimal amount of information Turborepo needs to correctly start a local
/// proxy server for microfrontends
#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct ConfigV1 {
    pub version: String,
    pub applications: BTreeMap<String, Application>,
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

impl ConfigV1 {
    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str::<Self>(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();
        if let Some(config) = config {
            if config.version == "1" {
                Ok(config)
            } else {
                Err(Error::InvalidVersion {
                    expected: "1",
                    actual: config.version,
                })
            }
        } else {
            Err(Error::biome_error(errs))
        }
    }

    pub fn applications(&self) -> impl Iterator<Item = (&String, &Application)> {
        self.applications.iter()
    }

    pub fn development_tasks(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.applications
            .iter()
            .map(|(application, config)| (application.as_str(), config.development.task.as_deref()))
    }
}
