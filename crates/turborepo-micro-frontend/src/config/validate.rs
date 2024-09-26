use std::collections::BTreeMap;

use biome_deserialize::json::deserialize_from_json_str;
use turbopath::AbsoluteSystemPath;
use url::Url;

use super::parse::{
    Application as RawApplication, Config as RawConfig, Development as RawDevelopment, Federation,
    Host as RawHost, Options, Protocol, Vercel, ZoneRouting,
};
use crate::Error;

const SUPPORTED_VERSIONS: &[&str] = ["1"].as_slice();

#[derive(Debug)]
pub struct Config {
    version: String,
    default_application: Application,
    zones: BTreeMap<String, ApplicationWithRouting>,
    options: Option<Options>,
}

#[derive(Debug)]
pub struct Application {
    name: String,
    development: Development,
    production: Host,
    metadata: Option<BTreeMap<String, String>>,
    federation: Option<Federation>,
    vercel: Option<Vercel>,
}

#[derive(Debug)]
pub struct Development {
    local: Host,
    fallback: Option<Host>,
    task: Option<String>,
}

#[derive(Debug)]
pub struct Host {
    url: Url,
    protocol: Protocol,
    host: String,
    port: Option<u16>,
}

#[derive(Debug)]
pub struct ApplicationWithRouting {
    app: Application,
    routing: ZoneRouting,
}

impl Config {
    /// Reads config from given path.
    /// Retruns `Ok(None)` if the file does not exist
    pub fn load(config_path: &AbsoluteSystemPath) -> Result<Option<Self>, Error> {
        let Some(contents) = config_path.read_existing_to_string()? else {
            return Ok(None);
        };
        let raw_config =
            RawConfig::from_str(&contents, config_path.as_str()).map_err(Error::biome_error)?;
        Ok(Some(Config::try_from(raw_config)?))
    }

    pub fn default_application(&self) -> &Application {
        &self.default_application
    }

    pub fn zone(&self, name: &str) -> Result<&ApplicationWithRouting, Error> {
        self.zones
            .get(name)
            .ok_or_else(|| crate::Error::NoApplicationConfiguration(name.to_owned()))
    }

    pub fn zones(&self) -> impl Iterator<Item = &ApplicationWithRouting> {
        self.zones.values()
    }
}

impl Application {
    pub fn new_default(name: String, raw_application: RawApplication) -> Result<Self, Error> {
        let RawApplication {
            default,
            routing,
            development,
            production,
            metadata,
            federation,
            vercel,
        } = raw_application;
        debug_assert!(default, "tried to make {name} the default application");
        if routing.is_some() {
            return Err(crate::Error::RoutingOnDefaultApplication(name));
        }
        let development = Development::try_from(development)?;
        let production = Host::try_from(production)?;
        Ok(Application {
            name,
            development,
            production,
            metadata,
            federation,
            vercel,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl ApplicationWithRouting {
    pub fn new(name: String, raw_application: RawApplication) -> Result<Self, Error> {
        let RawApplication {
            default,
            routing,
            development,
            production,
            metadata,
            federation,
            vercel,
        } = raw_application;
        debug_assert!(!default, "tried to make {name} a non-default application");
        let Some(routing) = routing else {
            return Err(Error::MissingRouting(name));
        };
        let development = Development::try_from(development)?;
        let production = Host::try_from(production)?;
        Self::validate_route(&name, &routing)?;
        let app = Application {
            name,
            development,
            production,
            metadata,
            federation,
            vercel,
        };
        Ok(ApplicationWithRouting { app, routing })
    }

    fn validate_route(name: &str, route: &ZoneRouting) -> Result<(), Error> {
        if route.asset_prefix.as_deref().map_or(false, |asset_prefix| {
            asset_prefix.starts_with('/') || asset_prefix.ends_with('/')
        }) {
            return Err(Error::InvalidAssetPrefix(name.to_owned()));
        }

        for path in route
            .matches
            .iter()
            .flat_map(|group| group.paths.iter())
            .filter(|path| path.as_str() != "/")
        {
            if path.ends_with('/') {
                return Err(Error::PathTrailingSlash {
                    name: name.to_owned(),
                    path: path.clone(),
                });
            }
            if !path.starts_with('/') {
                return Err(Error::PathNoLeadingSlash {
                    name: name.to_owned(),
                    path: path.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn application(&self) -> &Application {
        &self.app
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.routing
            .matches
            .iter()
            .flat_map(|path_group| path_group.paths.iter())
            .map(|path| path.as_str())
    }
}

impl Host {
    pub fn port(&self) -> u16 {
        self.port.unwrap_or({
            match self.protocol {
                Protocol::Http => 80,
                Protocol::Https => 443,
            }
        })
    }

    pub fn is_default_port(&self) -> bool {
        self.port() == self.protocol.default_port()
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn as_str(&self) -> &str {
        let url = self.url();
        let url_str = url.as_str();
        url_str.strip_suffix('/').unwrap_or(url_str)
    }
}

impl TryFrom<RawConfig> for Config {
    type Error = Error;

    fn try_from(value: RawConfig) -> Result<Self, Self::Error> {
        let RawConfig {
            version,
            applications,
            options,
            ..
        } = value;
        if !SUPPORTED_VERSIONS.contains(&version.as_str()) {
            return Err(Error::UnsupportedVersion {
                version,
                supported_versions: SUPPORTED_VERSIONS.join(", "),
            });
        }
        let (default_app, applications): (Vec<_>, _) = applications
            .into_iter()
            .partition(|(_, config)| config.default);
        let default_application = match default_app.len().cmp(&1) {
            std::cmp::Ordering::Less => Err(Error::NoDefaultApplication),
            std::cmp::Ordering::Equal => {
                let (name, app) = default_app
                    .into_iter()
                    .next()
                    .expect("just verified that there's one app in the list");
                Application::new_default(name, app)
            }
            std::cmp::Ordering::Greater => Err(Error::MultipleDefaultApplications(
                default_app.into_iter().map(|(k, _)| k).collect(),
            )),
        }?;
        let mut zones = BTreeMap::new();
        for (name, raw_app) in applications {
            let app = ApplicationWithRouting::new(name.clone(), raw_app)?;
            zones.insert(name, app);
        }
        Ok(Self {
            version,
            default_application,
            zones,
            options,
        })
    }
}

impl TryFrom<RawDevelopment> for Development {
    type Error = url::ParseError;

    fn try_from(value: RawDevelopment) -> Result<Self, Self::Error> {
        let RawDevelopment {
            local,
            fallback,
            task,
        } = value;
        let local = Host::try_from(local)?;
        let fallback = fallback.map(Host::try_from).transpose()?;
        Ok(Development {
            local,
            fallback,
            task,
        })
    }
}

impl TryFrom<RawHost> for Host {
    type Error = url::ParseError;

    fn try_from(value: RawHost) -> Result<Self, Self::Error> {
        let RawHost {
            protocol,
            host,
            port,
        } = value;
        let mut url = Url::parse(&format!("{protocol}://{host}"))?;
        if let Some(port) = port {
            url.set_port(Some(port))
                .expect("http and https urls can always have a port");
        }
        Ok(Self {
            url,
            protocol,
            host,
            port,
        })
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;
    const EXAMPLE_CONFIG: &str = include_str!("../../fixtures/micro-frontend.jsonc");

    #[test]
    fn test_example_parses() {
        let input = EXAMPLE_CONFIG;
        let example_config =
            Config::try_from(RawConfig::from_str(input, "something.json").unwrap());
        assert!(example_config.is_ok(), "{}", example_config.unwrap_err());
    }

    #[test]
    fn test_load_config() {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPath::new(tempdir.path().to_str().unwrap()).unwrap();
        let config_path = dir.join_component("micro-frontend.jsonc");
        config_path
            .create_with_contents(EXAMPLE_CONFIG.as_bytes())
            .unwrap();
        let config = Config::load(&config_path).unwrap();
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.default_application().name(), "main-site");
        assert!(config.zone("docs").is_ok());
    }

    #[test]
    fn test_host_url() {
        #[derive(Debug, Default)]
        struct TestCase {
            protocol: Protocol,
            host: &'static str,
            port: Option<u16>,
            expected: &'static str,
        }
        for test_case in &[
            TestCase {
                protocol: Protocol::Http,
                host: "example.com",
                expected: "http://example.com",
                ..Default::default()
            },
            TestCase {
                protocol: Protocol::Http,
                host: "example.com",
                port: Some(42),
                expected: "http://example.com:42",
            },
            TestCase {
                protocol: Protocol::Http,
                host: "example.com",
                port: Some(80),
                expected: "http://example.com",
            },
            TestCase {
                protocol: Protocol::Https,
                host: "example.com",
                port: Some(42),
                expected: "https://example.com:42",
            },
            TestCase {
                protocol: Protocol::Https,
                host: "example.com",
                expected: "https://example.com",
                ..Default::default()
            },
            TestCase {
                protocol: Protocol::Https,
                host: "example.com",
                port: Some(443),
                expected: "https://example.com",
            },
        ] {
            let TestCase {
                protocol,
                host,
                port,
                expected,
            } = &test_case;
            let host = Host::try_from(RawHost {
                protocol: *protocol,
                host: host.to_string(),
                port: *port,
            })
            .unwrap_or_else(|e| panic!("invalid host: {e} in test case {test_case:?}"));
            assert_eq!(host.as_str(), *expected, "{test_case:?}");
        }
    }

    #[test]
    fn test_unsupported_version() {
        let config = Config::try_from(RawConfig {
            version: "-1".into(),
            ..Default::default()
        });
        assert!(config.is_err());
        let err_msg = config.unwrap_err().to_string();
        insta::assert_snapshot!(err_msg);
    }

    #[test]
    fn test_no_default_app() {
        let config = Config::try_from(RawConfig {
            version: "1".into(),
            ..Default::default()
        });
        assert!(config.is_err());
        let err_msg = config.unwrap_err().to_string();
        insta::assert_snapshot!(err_msg);
    }

    #[test]
    fn test_multiple_default_app() {
        let mut applications = BTreeMap::new();
        applications.insert(
            "web".into(),
            RawApplication {
                default: true,
                ..Default::default()
            },
        );
        applications.insert(
            "docs".into(),
            RawApplication {
                default: true,
                ..Default::default()
            },
        );
        let config = Config::try_from(RawConfig {
            version: "1".into(),
            applications,
            ..Default::default()
        });
        assert!(config.is_err());
        let err_msg = config.unwrap_err().to_string();
        insta::assert_snapshot!(err_msg);
    }

    #[test]
    fn test_default_app_with_routing() {
        let mut applications = BTreeMap::new();
        applications.insert(
            "web".into(),
            RawApplication {
                default: true,
                routing: Some(ZoneRouting::default()),
                ..Default::default()
            },
        );
        let config = Config::try_from(RawConfig {
            version: "1".into(),
            applications,
            ..Default::default()
        });
        assert!(config.is_err());
        let err_msg = config.unwrap_err().to_string();
        insta::assert_snapshot!(err_msg);
    }

    #[test]
    fn test_app_without_routing() {
        let mut applications = BTreeMap::new();
        applications.insert(
            "web".into(),
            RawApplication {
                default: true,
                production: RawHost {
                    protocol: Protocol::Http,
                    host: "example.com".into(),
                    port: None,
                },
                development: RawDevelopment {
                    local: RawHost {
                        protocol: Protocol::Http,
                        host: "localhost".into(),
                        port: Some(3000),
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        applications.insert(
            "docs".into(),
            RawApplication {
                ..Default::default()
            },
        );
        let config = Config::try_from(RawConfig {
            version: "1".into(),
            applications,
            ..Default::default()
        });
        assert!(config.is_err());
        let err_msg = config.unwrap_err().to_string();
        insta::assert_snapshot!(err_msg);
    }

    #[test]
    fn test_asset_prefix_err() {
        let leading_slash = ZoneRouting {
            asset_prefix: Some("/docs".into()),
            ..Default::default()
        };
        let trailing_slash = ZoneRouting {
            asset_prefix: Some("docs/".into()),
            ..Default::default()
        };
        let err1 = ApplicationWithRouting::validate_route("docs", &leading_slash);
        let err2 = ApplicationWithRouting::validate_route("docs", &trailing_slash);
        assert!(err1.is_err());
        assert!(err2.is_err());
        let err_msg = err1.unwrap_err().to_string();
        insta::assert_snapshot!(err_msg);
    }
}
