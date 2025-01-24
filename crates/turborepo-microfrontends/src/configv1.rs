use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use serde::Serialize;

use crate::Error;

pub enum ParseResult {
    Actual(ConfigV1),
    Reference(String),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct ConfigV1 {
    version: Option<String>,
    applications: BTreeMap<String, Application>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
struct ChildConfig {
    part_of: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
struct Application {
    development: Option<Development>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
struct Development {
    task: Option<String>,
}

impl ConfigV1 {
    pub fn from_str(input: &str, source: &str) -> Result<ParseResult, Error> {
        // attempt to parse a child, ignoring any errors
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str::<ChildConfig>(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();

        if let Some(ChildConfig { part_of }) = errs.is_empty().then_some(config).flatten() {
            return Ok(ParseResult::Reference(part_of));
        }
        // attempt to parse a real one
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str::<ConfigV1>(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();

        if let Some(config) = config {
            let version = config.version.clone().unwrap_or("1".to_string());
            if version == "1" {
                Ok(ParseResult::Actual(config))
            } else {
                Err(Error::InvalidVersion {
                    expected: "1",
                    actual: version,
                })
            }
        } else {
            Err(Error::biome_error(errs))
        }
    }

    pub fn development_tasks(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.applications
            .iter()
            .map(|(application, config)| (application.as_str(), config.task()))
    }
}

impl Application {
    fn task(&self) -> Option<&str> {
        self.development.as_ref()?.task.as_deref()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_child_config_parse() {
        let input = r#"{"partOf": "web"}"#;
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(_config_v2) => panic!("expected to get reference to default app"),
            ParseResult::Reference(default_app) => {
                assert_eq!(default_app, "web");
            }
        }
    }

    #[test]
    fn test_root_config_parse() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {},
          "docs": {"development": {"task": "serve"}}
        }
    }"#;
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(config_v1.applications.get("web").unwrap().task(), None);
                assert_eq!(
                    config_v1.applications.get("docs").unwrap().task(),
                    Some("serve")
                );
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_no_version_config_parse() {
        let input = r#"{
        "applications": {
          "web": {},
          "docs": {"development": {"task": "serve"}}
        }
    }"#;
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(config_v1.applications.get("web").unwrap().task(), None);
                assert_eq!(
                    config_v1.applications.get("docs").unwrap().task(),
                    Some("serve")
                );
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }
}
