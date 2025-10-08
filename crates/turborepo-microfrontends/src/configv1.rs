use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use serde::Serialize;

use crate::{DevelopmentTask, Error};

pub enum ParseResult {
    Actual(ConfigV1),
    Reference(String),
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct ConfigV1 {
    version: Option<String>,
    applications: BTreeMap<String, Application>,
    options: Option<Options>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct Options {
    local_proxy_port: Option<u16>,
    disable_overrides: Option<bool>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct ChildConfig {
    part_of: String,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct Application {
    package_name: Option<String>,
    development: Option<Development>,
    routing: Option<Vec<PathGroup>>,
    asset_prefix: Option<String>,
    production: Option<ProductionConfig>,
    vercel: Option<VercelConfig>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct PathGroup {
    pub paths: Vec<String>,
    pub group: Option<String>,
    pub flag: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct ProductionConfig {}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct VercelConfig {}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct Development {
    task: Option<String>,
    local: Option<LocalHost>,
    fallback: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone, Copy)]
struct LocalHost {
    port: Option<u16>,
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
                    path: source.to_string(),
                })
            }
        } else {
            Err(Error::biome_error(errs))
        }
    }

    pub fn development_tasks(&self) -> impl Iterator<Item = DevelopmentTask<'_>> {
        self.applications
            .iter()
            .map(|(application, config)| DevelopmentTask {
                application_name: application,
                package: config.package_name(application),
                task: config.task(),
            })
    }

    pub fn port(&self, name: &str) -> Option<u16> {
        let application = self.applications.get(name)?;
        Some(application.port(name))
    }

    pub fn local_proxy_port(&self) -> Option<u16> {
        self.options.as_ref()?.local_proxy_port
    }

    pub fn routing(&self, app_name: &str) -> Option<&[PathGroup]> {
        let application = self.applications.get(app_name)?;
        application.routing.as_deref()
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

    fn package_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.package_name.as_deref().unwrap_or(key)
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

    #[test]
    fn test_package_name_parse() {
        let input = r#"{
        "applications": {
          "web": {
            "packageName": "@acme/web"
          },
          "docs": {"development": {"task": "serve"}}
        }
    }"#;

        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(
                    config_v1.applications.get("web").unwrap().package_name,
                    Some("@acme/web".into())
                );
                assert_eq!(
                    config_v1.applications.get("docs").unwrap().package_name,
                    None
                );
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_package_name_development_tasks() {
        let input = r#"{
        "applications": {
          "web": {
            "packageName": "@acme/web"
          },
          "docs": {"development": {"task": "serve"}}
        }
    }"#;

        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                let mut dev_tasks = config_v1.development_tasks().collect::<Vec<_>>();
                dev_tasks.sort();
                assert_eq!(
                    dev_tasks,
                    vec![
                        DevelopmentTask {
                            application_name: "docs",
                            package: "docs",
                            task: Some("serve")
                        },
                        DevelopmentTask {
                            application_name: "web",
                            package: "@acme/web",
                            task: None
                        },
                    ]
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
