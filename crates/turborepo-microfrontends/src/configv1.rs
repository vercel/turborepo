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
    #[serde(rename = "$schema", skip)]
    schema: Option<String>,
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
struct ProductionConfig {
    protocol: Option<String>,
    host: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct VercelConfig {
    #[serde(rename = "projectId")]
    project_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserializable, Default, Clone)]
struct Development {
    task: Option<String>,
    local: Option<LocalHost>,
    fallback: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Default, Clone, Copy)]
struct LocalHost {
    port: Option<u16>,
}

impl biome_deserialize::Deserializable for LocalHost {
    fn deserialize(
        value: &impl biome_deserialize::DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<biome_deserialize::DeserializationDiagnostic>,
    ) -> Option<Self> {
        use biome_deserialize::VisitableType;

        // Check what type we have
        match value.visitable_type()? {
            // Deserialize as a plain number (just the port)
            VisitableType::NUMBER => {
                let port_num = u16::deserialize(value, name, diagnostics)?;
                Some(LocalHost {
                    port: Some(port_num),
                })
            }
            // Deserialize as a string (host with optional port)
            VisitableType::STR => {
                let host_str = String::deserialize(value, name, diagnostics)?;
                let port = parse_port_from_host(&host_str);
                Some(LocalHost { port })
            }
            // Deserialize as an object (with explicit port field)
            VisitableType::MAP => {
                #[derive(Deserializable, Default)]
                struct LocalHostObject {
                    port: Option<u16>,
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
    // Try to extract port from host string
    // Formats: "hostname:port", "protocol://hostname:port"

    // Remove protocol if present
    let without_protocol = if let Some(idx) = host.find("://") {
        &host[idx + 3..]
    } else {
        host
    };

    // Extract port after the last colon
    if let Some(colon_idx) = without_protocol.rfind(':')
        && let Ok(port) = without_protocol[colon_idx + 1..].parse::<u16>()
    {
        return Some(port);
    }

    None
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
            // Only accept the config if there were no errors during parsing
            if !errs.is_empty() {
                return Err(Error::biome_error(errs));
            }
            // Accept any version. This allows the Turborepo proxy to work with
            // configurations that have different version numbers than expected,
            // as long as the structure is compatible with what Turborepo needs
            // to route traffic.
            Ok(ParseResult::Actual(config))
        } else {
            Err(Error::biome_error(errs))
        }
    }

    /// Converts a TurborepoConfig to ConfigV1 for compatibility with
    /// the proxy. This preserves only the fields that TurborepoConfig knows
    /// about, discarding any Vercel-specific metadata.
    pub fn from_turborepo_config(config: &crate::schema::TurborepoConfig) -> Self {
        let mut applications = BTreeMap::new();

        for (app_name, turbo_app) in config.applications() {
            let app = Application {
                package_name: turbo_app.package_name.clone(),
                development: turbo_app.development.as_ref().map(|dev| Development {
                    task: None,
                    local: dev.local.map(|lh| LocalHost { port: lh.port }),
                    fallback: dev.fallback.clone(),
                }),
                routing: turbo_app.routing.as_ref().map(|routes| {
                    routes
                        .iter()
                        .map(|r| PathGroup {
                            paths: r.paths.clone(),
                            group: r.group.clone(),
                            flag: None,
                        })
                        .collect()
                }),
                asset_prefix: None,
                production: None,
                vercel: None,
            };
            applications.insert(app_name, app);
        }

        ConfigV1 {
            version: None,
            applications,
            options: config.local_proxy_port().map(|port| Options {
                local_proxy_port: Some(port),
                disable_overrides: None,
            }),
            schema: None,
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

    pub fn applications(&self) -> impl Iterator<Item = crate::Application<'_>> {
        self.applications
            .iter()
            .map(|(application, config)| crate::Application {
                application_name: application,
                package: config.package_name(application),
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

    pub fn fallback(&self, name: &str) -> Option<&str> {
        let application = self.applications.get(name)?;
        application.fallback()
    }

    /// Returns the name and package of the application that serves the root
    /// route. The root route app is the first one without explicit routing
    /// configuration.
    pub fn root_route_app(&self) -> Option<(&str, &str)> {
        self.applications
            .iter()
            .find(|(_, app)| app.routing.is_none())
            .map(|(app_name, app)| (app_name.as_str(), app.package_name(app_name)))
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
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(config_v1.port("web"), Some(3000));
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_local_port_string_with_port() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": "localhost:3002"
            }
          }
        }
    }"#;
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(config_v1.port("web"), Some(3002));
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_local_port_string_with_protocol() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": "http://localhost:3003"
            }
          }
        }
    }"#;
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(config_v1.port("web"), Some(3003));
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_local_port_string_without_port() {
        let input = r#"{
        "version": "1",
        "applications": {
          "web": {
            "development": {
              "local": "localhost"
            }
          }
        }
    }"#;
        let config = ConfigV1::from_str(input, "somewhere").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                // Should fall back to generated port
                assert!(config_v1.port("web").is_some());
                let port = config_v1.port("web").unwrap();
                assert!((MIN_PORT..MAX_PORT).contains(&port));
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_user_config_format() {
        // Test the exact format from the user's issue
        let input = r#"{
        "$schema": "https://openapi.vercel.sh/microfrontends.json",
        "applications": {
          "microfrontends-marketing": {
            "development": {
              "local": 3000,
              "fallback": "microfrontends-marketing.labs.vercel.dev"
            }
          },
          "microfrontends-docs": {
            "development": {
              "local": 3001
            },
            "routing": [
              {
                "group": "docs",
                "paths": ["/docs", "/docs/:path*"]
              }
            ]
          }
        }
      }"#;
        let config = ConfigV1::from_str(input, "microfrontends.json").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                // Verify the ports are correctly parsed
                assert_eq!(config_v1.port("microfrontends-marketing"), Some(3000));
                assert_eq!(config_v1.port("microfrontends-docs"), Some(3001));
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
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
          },
          "docs": {
            "development": {
              "local": 3001,
              "fallback": "https://docs.example.com"
            }
          },
          "api": {
            "development": {
              "local": 3002
            }
          }
        }
      }"#;
        let config = ConfigV1::from_str(input, "microfrontends.json").unwrap();
        match config {
            ParseResult::Actual(config_v1) => {
                assert_eq!(config_v1.fallback("web"), Some("example.com"));
                assert_eq!(config_v1.fallback("docs"), Some("https://docs.example.com"));
                assert_eq!(config_v1.fallback("api"), None);
                assert_eq!(config_v1.fallback("nonexistent"), None);
            }
            ParseResult::Reference(_) => panic!("expected to get main config"),
        }
    }

    #[test]
    fn test_malformed_json_unclosed_bracket() {
        let input = r#"{"applications": {"web": {"development": {"local": 3000}}"#;
        let config = ConfigV1::from_str(input, "microfrontends.json");
        assert!(
            config.is_err(),
            "Parser should reject JSON with unclosed bracket"
        );
    }

    #[test]
    fn test_malformed_json_trailing_comma() {
        let input = r#"{"applications": {"web": {"development": {"local": 3000,}}}}"#;
        let config = ConfigV1::from_str(input, "microfrontends.json");
        assert!(
            config.is_err(),
            "Parser should reject JSON with trailing comma"
        );
    }

    #[test]
    fn test_missing_required_applications() {
        // Even though applications has defaults, if JSON structure is invalid it should
        // fail
        let input = r#"{"applications": {, "web": {}}}"#;
        let config = ConfigV1::from_str(input, "microfrontends.json");
        assert!(
            config.is_err(),
            "Parser should reject JSON with syntax errors"
        );
    }

    #[test]
    fn test_invalid_routing_structure() {
        let input = r#"{
        "applications": {
          "docs": {
            "routing": "invalid"
          }
        }
      }"#;
        let config = ConfigV1::from_str(input, "microfrontends.json");
        assert!(
            config.is_err(),
            "Parser should reject routing that is not an array"
        );
    }

    #[test]
    fn test_invalid_path_group_structure() {
        let input = r#"{
        "applications": {
          "docs": {
            "routing": [
              {
                "group": "docs",
                "paths": "should_be_array"
              }
            ]
          }
        }
      }"#;
        let config = ConfigV1::from_str(input, "microfrontends.json");
        assert!(
            config.is_err(),
            "Parser should reject paths that is not an array"
        );
    }
}
