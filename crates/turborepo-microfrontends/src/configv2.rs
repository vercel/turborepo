use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use serde::Serialize;

use crate::Error;

pub enum ParseResult {
    Actual(ConfigV2),
    Reference(String),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
pub struct ConfigV2 {
    version: String,
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
    local: Option<LocalHost>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default)]
struct LocalHost {
    port: Option<u16>,
}

impl ConfigV2 {
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
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str::<ConfigV2>(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();

        if let Some(config) = config {
            if config.version == "2" || config.version.is_empty() {
                Ok(ParseResult::Actual(config))
            } else {
                Err(Error::InvalidVersion {
                    expected: "2",
                    actual: config.version,
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

    pub fn port(&self, name: &str) -> Option<u16> {
        let application = self.applications.get(name)?;
        Some(application.port(name))
    }
}

impl Application {
    fn task(&self) -> Option<&str> {
        self.development.as_ref()?.task.as_deref()
    }

    fn user_port(&self) -> Option<u16> {
        self.development.as_ref()?.local.as_ref()?.port
    }

    fn port(&self, name: &str) -> u16 {
        self.user_port()
            .unwrap_or_else(|| generate_port_from_name(name))
    }
}

const MIN_PORT: u16 = 3000;
const MAX_PORT: u16 = 8000;
const PORT_RANGE: u16 = MAX_PORT - MIN_PORT;

fn generate_port_from_name(name: &str) -> u16 {
    let mut hash: i32 = 0;
    for c in name.chars() {
        let code = i32::try_from(u32::from(c)).expect("char::MAX is less than 2^31");
        hash = (hash << 5).overflowing_sub(hash).0.overflowing_add(code).0;
    }
    let hash = hash.abs_diff(0);
    let port = hash % u32::from(PORT_RANGE);
    MIN_PORT + u16::try_from(port).expect("u32 modulo a u16 number will be a valid u16")
}

#[cfg(test)]
mod test {
    use std::char;

    use super::*;

    #[test]
    fn test_char_as_i32() {
        let max_char = u32::from(char::MAX);
        assert!(
            i32::try_from(max_char).is_ok(),
            "max char should fit in i32"
        );
    }

    #[test]
    fn test_child_config_parse() {
        let input = r#"{"partOf": "web"}"#;
        let config = ConfigV2::from_str(input, "somewhere").unwrap();
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
        "version": "2",
        "applications": {
          "web": {},
          "docs": {"development": {"task": "serve"}}
        }
    }"#;
        let config = ConfigV2::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v2) => {
                assert_eq!(config_v2.applications.get("web").unwrap().task(), None);
                assert_eq!(
                    config_v2.applications.get("docs").unwrap().task(),
                    Some("serve")
                );
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_generate_port() {
        assert_eq!(generate_port_from_name("test-450"), 7724);
    }
}
