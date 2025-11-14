use std::io;

use serde::Deserialize;
use serde_yaml;
use turbopath::AbsoluteSystemPath;

pub const YARNRC_FILENAME: &str = ".yarnrc.yml";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Encountered error opening yarnrc.yml: {0}")]
    Io(#[from] std::io::Error),
    #[error("Encountered error parsing yarnrc.yml: {0}")]
    SerdeYaml(#[from] serde_yaml::Error),
}

/// A yarnrc.yaml file representing settings affecting the package graph.
#[derive(Debug, PartialEq, Eq, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YarnRc {
    /// Used by Yarn(Berry) as `enableTransparentWorkspaces`.
    /// When true, treats local workspaces that match a package name
    /// and semver range as correct match resulting in turbo including
    /// the package in the dependency graph
    #[serde(default = "default_enable_transparent_workspaces")]
    pub enable_transparent_workspaces: bool,
    /// Yarn 4.11.0+ default catalog (singular) - maps package names to versions
    #[serde(default)]
    pub catalog: Option<std::collections::BTreeMap<String, String>>,
    /// Yarn 4.11.0+ named catalogs (plural) - maps catalog names to package
    /// versions
    #[serde(default)]
    pub catalogs:
        Option<std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>>,
}

fn default_enable_transparent_workspaces() -> bool {
    true
}

impl Default for YarnRc {
    fn default() -> YarnRc {
        YarnRc {
            enable_transparent_workspaces: default_enable_transparent_workspaces(),
            catalog: None,
            catalogs: None,
        }
    }
}

impl YarnRc {
    pub fn from_reader(mut reader: impl io::Read) -> Result<Self, Error> {
        let config: YarnRc = serde_yaml::from_reader(&mut reader)?;
        Ok(config)
    }

    pub fn from_file(repo_root: &AbsoluteSystemPath) -> Result<Self, Error> {
        let yarnrc_path = repo_root.join_component(YARNRC_FILENAME);
        let yarnrc = yarnrc_path
            .read_existing_to_string()?
            .map(|contents| Self::from_reader(contents.as_bytes()))
            .transpose()?
            .unwrap_or_default();
        Ok(yarnrc)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_empty_yarnrc() {
        let empty = YarnRc::from_reader(b"".as_slice()).unwrap();
        assert_eq!(
            empty,
            YarnRc {
                enable_transparent_workspaces: true,
                catalog: None,
                catalogs: None,
            }
        );
    }

    #[test]
    fn test_parses_transparent_workspaces() {
        let empty = YarnRc::from_reader(b"enableTransparentWorkspaces: false".as_slice()).unwrap();
        assert_eq!(
            empty,
            YarnRc {
                enable_transparent_workspaces: false,
                catalog: None,
                catalogs: None,
            }
        );
    }

    #[test]
    fn test_parses_additional_settings() {
        let empty = YarnRc::from_reader(b"httpProxy: \"http://my-proxy.com\"".as_slice()).unwrap();
        assert_eq!(
            empty,
            YarnRc {
                enable_transparent_workspaces: true,
                catalog: None,
                catalogs: None,
            }
        );
    }

    #[test]
    fn test_parses_default_catalog() {
        let yarnrc_content = b"catalog:\n  lodash: ^4.17.21\n  react: ^18.0.0";
        let yarnrc = YarnRc::from_reader(yarnrc_content.as_slice()).unwrap();

        assert!(yarnrc.catalog.is_some());
        let catalog = yarnrc.catalog.unwrap();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.get("lodash"), Some(&"^4.17.21".to_string()));
        assert_eq!(catalog.get("react"), Some(&"^18.0.0".to_string()));
    }

    #[test]
    fn test_parses_catalogs() {
        let yarnrc_content = b"catalogs:\n  react18:\n    react: ^18.0.0\n    react-dom: ^18.0.0";
        let yarnrc = YarnRc::from_reader(yarnrc_content.as_slice()).unwrap();

        assert!(yarnrc.catalogs.is_some());
        let catalogs = yarnrc.catalogs.unwrap();
        assert_eq!(catalogs.len(), 1);
        assert!(catalogs.contains_key("react18"));

        let react18 = &catalogs["react18"];
        assert_eq!(react18.get("react"), Some(&"^18.0.0".to_string()));
        assert_eq!(react18.get("react-dom"), Some(&"^18.0.0".to_string()));
    }
}
