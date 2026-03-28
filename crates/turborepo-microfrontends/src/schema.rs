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

use crate::{
    Error,
    port::{generate_port_from_name, parse_port_from_host},
};

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
pub struct TurborepoConfig {
    #[serde(rename = "$schema", skip)]
    schema: Option<String>,
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

impl TurborepoConfig {
    pub fn from_str(input: &str, source: &str) -> Result<Self, Error> {
        let jsonc_options = JsonParserOptions::default()
            .with_allow_comments()
            .with_allow_trailing_commas();

        let (config, errs) = biome_deserialize::json::deserialize_from_json_str::<TurborepoConfig>(
            input,
            jsonc_options,
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

    /// Resolves an application by name. Tries a direct key match first,
    /// then falls back to scanning by `packageName`. This handles the common
    /// case where the config key (e.g. a Vercel project name) differs from
    /// the local package name.
    fn find_application(&self, name: &str) -> Option<(&String, &TurborepoApplication)> {
        self.applications.get_key_value(name).or_else(|| {
            self.applications
                .iter()
                .find(|(key, app)| app.package_name(key) == name)
        })
    }

    /// Returns the dev server port for the given application.
    ///
    /// Looks up `name` first as a config map key, then falls back to
    /// scanning by `packageName`. When no explicit port is configured,
    /// a deterministic port is generated from the config map key.
    pub fn port(&self, name: &str) -> Option<u16> {
        let (key, app) = self.find_application(name)?;
        Some(app.port(key))
    }

    pub fn local_proxy_port(&self) -> Option<u16> {
        self.options.as_ref()?.local_proxy_port
    }

    /// Returns the routing configuration for the given application.
    ///
    /// Looks up `name` first as a config map key, then falls back to
    /// scanning by `packageName`.
    pub fn routing(&self, app_name: &str) -> Option<&[PathGroup]> {
        let (_, app) = self.find_application(app_name)?;
        app.routing.as_deref()
    }

    /// Returns the fallback URL for the given application.
    ///
    /// Looks up `name` first as a config map key, then falls back to
    /// scanning by `packageName`.
    pub fn fallback(&self, name: &str) -> Option<&str> {
        let (_, app) = self.find_application(name)?;
        app.fallback()
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
    fn test_port_lookup_by_package_name() {
        let input = r#"{
        "applications": {
            "my-vercel-project": {
                "packageName": "my-app",
                "development": {"local": 3001}
            }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "microfrontends.json").unwrap();
        assert_eq!(config.port("my-app"), Some(3001));
        assert_eq!(config.port("my-vercel-project"), Some(3001));
    }

    #[test]
    fn test_port_lookup_by_package_name_auto_generated() {
        let input = r#"{
        "applications": {
            "my-vercel-project": {
                "packageName": "my-app"
            }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "microfrontends.json").unwrap();
        let port_by_pkg = config.port("my-app");
        let port_by_key = config.port("my-vercel-project");
        assert!(port_by_pkg.is_some());
        assert!(port_by_key.is_some());
        assert_eq!(port_by_pkg, port_by_key);
        assert_eq!(
            port_by_key,
            Some(generate_port_from_name("my-vercel-project"))
        );
    }

    #[test]
    fn test_port_returns_none_for_unknown_name() {
        let input = r#"{"applications": {"web": {"development": {"local": 3000}}}}"#;
        let config = TurborepoConfig::from_str(input, "microfrontends.json").unwrap();
        assert_eq!(config.port("nonexistent"), None);
    }

    #[test]
    fn test_direct_key_takes_priority_over_package_name() {
        let input = r#"{
        "applications": {
            "web": {
                "development": {"local": 3000}
            },
            "vercel-web": {
                "packageName": "web",
                "development": {"local": 4000}
            }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "microfrontends.json").unwrap();
        assert_eq!(config.port("web"), Some(3000));
        assert_eq!(config.port("vercel-web"), Some(4000));
    }

    #[test]
    fn test_fallback_lookup_by_package_name() {
        let input = r#"{
        "applications": {
            "my-vercel-project": {
                "packageName": "my-app",
                "development": {
                    "local": 3001,
                    "fallback": "example.com"
                }
            }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "microfrontends.json").unwrap();
        assert_eq!(config.fallback("my-app"), Some("example.com"));
        assert_eq!(config.fallback("my-vercel-project"), Some("example.com"));
    }

    #[test]
    fn test_routing_lookup_by_package_name() {
        let input = r#"{
        "applications": {
            "my-vercel-project": {
                "packageName": "my-app",
                "routing": [{"paths": ["/docs"], "group": "docs"}],
                "development": {"local": 3001}
            }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "microfrontends.json").unwrap();
        assert!(config.routing("my-app").is_some());
        assert!(config.routing("my-vercel-project").is_some());
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
    fn test_jsonc_trailing_commas_accepted() {
        let input = r#"{"applications": {"web": {"development": {"local": 3000,}}}}"#;
        let config = TurborepoConfig::from_str(input, "somewhere.jsonc");
        assert!(
            config.is_ok(),
            "Parser should accept JSONC with trailing commas"
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
    fn test_schema_field_accepted() {
        let input = r#"{
        "$schema": "https://turborepo.dev/microfrontends/schema.json",
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": 3000
            }
          },
          "docs": {
            "routing": [
              {
                "paths": ["/docs"],
                "group": "docs"
              }
            ],
            "development": {
              "local": 3001
            }
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(config.is_ok(), "Parser should accept $schema field");
    }

    #[test]
    fn test_vercel_specific_fields_rejected() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": 3000,
              "task": "dev"
            }
          }
        }
    }"#;
        let config = TurborepoConfig::from_str(input, "somewhere");
        assert!(
            config.is_err(),
            "Strict parser should reject Vercel-specific fields like \"task\" and \"flag\""
        );
    }

    #[test]
    fn test_flag_field_rejected() {
        let input = r#"{
        "applications": {
          "docs": {
            "routing": [
              {
                "paths": ["/docs"],
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
            "Strict parser should reject Vercel-specific fields like \"task\" and \"flag\""
        );
    }
}
