//! Strict Turborepo-only schema for microfrontends configuration.
//!
//! ## Extendable by providers
//!
//! A provider like `@vercel/microfrontends` would parse this SAME config file
//! but also extract:
//! - The `task` field for orchestration
//! - The `partOf` field for child configs
//! - Production configuration
//! - And other provider-specific features

use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use biome_json_parser::JsonParserOptions;
use serde::Serialize;

use crate::Error;

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct TurborepoConfig {
    version: Option<String>,
    applications: BTreeMap<String, TurborepoApplication>,
    options: Option<TurborepoOptions>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct TurborepoOptions {
    #[serde(rename = "localProxyPort")]
    local_proxy_port: Option<u16>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct TurborepoApplication {
    #[serde(rename = "packageName")]
    pub package_name: Option<String>,
    pub development: Option<TurborepoDevelopment>,
    pub routing: Option<Vec<PathGroup>>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct PathGroup {
    pub paths: Vec<String>,
    pub group: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct TurborepoDevelopment {
    pub local: Option<LocalHost>,
    pub fallback: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Default, Clone, Copy)]
pub struct LocalHost {
    pub port: Option<u16>,
}

impl biome_deserialize::Deserializable for LocalHost {
    fn deserialize(
        value: &impl biome_deserialize::DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<biome_deserialize::DeserializationDiagnostic>,
    ) -> Option<Self> {
        use biome_deserialize::VisitableType;

        match value.visitable_type()? {
            VisitableType::NUMBER => {
                let port_num = u16::deserialize(value, name, diagnostics)?;
                Some(LocalHost {
                    port: Some(port_num),
                })
            }
            VisitableType::STR => {
                let host_str = String::deserialize(value, name, diagnostics)?;
                let port = parse_port_from_host(&host_str);
                Some(LocalHost { port })
            }
            VisitableType::MAP => {
                #[derive(Deserializable, Default)]
                struct LocalHostObject {
                    pub port: Option<u16>,
                }
                let obj = LocalHostObject::deserialize(value, name, diagnostics)?;
                Some(LocalHost { port: obj.port })
            }
            _ => {
                diagnostics.push(
                    biome_deserialize::DeserializationDiagnostic::new(format!(
                        "Expected a number, string, or object for '{name}'"
                    ))
                    .with_range(value.range()),
                );
                None
            }
        }
    }
}

fn parse_port_from_host(host: &str) -> Option<u16> {
    let without_protocol = if let Some(idx) = host.find("://") {
        &host[idx + 3..]
    } else {
        host
    };

    if let Some(colon_idx) = without_protocol.rfind(':')
        && let Ok(port) = without_protocol[colon_idx + 1..].parse::<u16>()
    {
        return Some(port);
    }

    None
}

impl TurborepoConfig {
    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        let (config, errs) = biome_deserialize::json::deserialize_from_json_str::<TurborepoConfig>(
            input,
            JsonParserOptions::default().with_allow_comments(),
            source,
        )
        .consume();

        if let Some(config) = config {
            // Only accept the config if there were no errors during parsing
            if !errs.is_empty() {
                return Err(Error::biome_error(errs));
            }
            Ok(config)
        } else {
            Err(Error::biome_error(errs))
        }
    }

    pub fn applications(&self) -> impl Iterator<Item = (String, &TurborepoApplication)> {
        self.applications.iter().map(|(k, v)| (k.clone(), v))
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

    pub fn fallback(&self, name: &str) -> Option<&str> {
        let application = self.applications.get(name)?;
        application.fallback()
    }

    pub fn root_route_app(&self) -> Option<(&str, &str)> {
        self.applications
            .iter()
            .find(|(_, app)| app.routing.is_none())
            .map(|(app_name, app)| (app_name.as_str(), app.package_name(app_name)))
    }
}

impl TurborepoApplication {
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

    fn fallback(&self) -> Option<&str> {
        self.development.as_ref()?.fallback.as_deref()
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
    use super::*;

    #[test]
    fn test_turborepo_config_parse() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {},
          "docs": {"routing": [{"paths": ["/docs"]}]}
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere").unwrap();
        assert!(config.applications.contains_key("web"));
        assert!(config.applications.contains_key("docs"));
    }

    #[test]
    fn test_port_generation() {
        assert_eq!(generate_port_from_name("test-450"), 7724);
    }

    #[test]
    fn test_root_route_app() {
        let input = r#"{
        "applications": {
          "web": {},
          "docs": {"routing": [{"paths": ["/docs"]}]}
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere").unwrap();
        let (app, pkg) = config.root_route_app().expect("should find root app");
        assert_eq!(app, "web");
        assert_eq!(pkg, "web");
    }

    #[test]
    fn test_fallback_parsing() {
        let input = r#"{
        "applications": {
          "web": {
            "development": {
              "local": 3000,
              "fallback": "example.com"
            }
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere").unwrap();
        assert_eq!(config.fallback("web"), Some("example.com"));
    }

    #[test]
    fn test_local_port_plain_number() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": 3000
            }
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere").unwrap();
        assert_eq!(config.port("web"), Some(3000));
    }

    #[test]
    fn test_malformed_json_unclosed_bracket() {
        let input = r#"{"applications": {"web": {"development": {"local": 3000}}"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(
            config.is_err(),
            "Parser should reject JSON with unclosed bracket"
        );
    }

    #[test]
    fn test_malformed_json_trailing_comma() {
        let input = r#"{"applications": {"web": {"development": {"local": 3000,}}}}"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(
            config.is_err(),
            "Parser should reject JSON with trailing comma"
        );
    }

    #[test]
    fn test_invalid_routing_type() {
        let input = r#"{
        "applications": {
          "docs": {
            "routing": "should_be_array"
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(
            config.is_err(),
            "Parser should reject routing that is not an array"
        );
    }

    #[test]
    fn test_invalid_paths_structure() {
        let input = r#"{
        "applications": {
          "docs": {
            "routing": [
              {
                "paths": "should_be_array"
              }
            ]
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(
            config.is_err(),
            "Parser should reject paths that is not an array"
        );
    }

    #[test]
    fn test_vercel_specific_fields_accepted() {
        let input = r#"{
        "$schema": "https://example.com/schema.json",
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": 3000,
              "task": "dev"
            }
          },
          "docs": {
            "routing": [
              {
                "paths": ["/docs"],
                "group": "docs",
                "flag": "enable_docs"
              }
            ],
            "development": {
              "local": 3001
            }
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(
            config.is_err(),
            "Strict parser should reject Vercel-specific fields like $schema, task, and flag"
        );
    }
}
