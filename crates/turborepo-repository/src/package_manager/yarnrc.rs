use std::io;

use serde::Deserialize;
use serde_yml;
use turbopath::AbsoluteSystemPath;

pub const YARNRC_FILENAME: &str = ".yarnrc.yml";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Encountered error opening yarnrc.yml: {0}")]
    Io(#[from] std::io::Error),
    #[error("Encountered error parsing yarnrc.yml: {0}")]
    SerdeYaml(#[from] serde_yml::Error),
}

type Map<K, V> = std::collections::BTreeMap<K, V>;

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
    /// Yarn 4+ catalog support - default catalog
    #[serde(default)]
    pub catalog: Option<Map<String, String>>,
    /// Yarn 4+ catalog support - named catalogs
    #[serde(default)]
    pub catalogs: Option<Map<String, Map<String, String>>>,
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
        let config: YarnRc = serde_yml::from_reader(&mut reader)?;
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
    fn test_parses_catalog() {
        let yarnrc =
            YarnRc::from_reader(b"catalog:\n  lodash: ^4.17.21\n  react: ^18.2.0".as_slice())
                .unwrap();
        let mut expected_catalog = Map::new();
        expected_catalog.insert("lodash".to_string(), "^4.17.21".to_string());
        expected_catalog.insert("react".to_string(), "^18.2.0".to_string());
        assert_eq!(
            yarnrc,
            YarnRc {
                enable_transparent_workspaces: true,
                catalog: Some(expected_catalog),
                catalogs: None,
            }
        );
    }

    #[test]
    fn test_parses_named_catalogs() {
        let yarnrc = YarnRc::from_reader(
            b"catalogs:\n  react18:\n    react: ^18.2.0\n  react17:\n    react: ^17.0.2".as_slice(),
        )
        .unwrap();
        let mut react18 = Map::new();
        react18.insert("react".to_string(), "^18.2.0".to_string());
        let mut react17 = Map::new();
        react17.insert("react".to_string(), "^17.0.2".to_string());
        let mut expected_catalogs = Map::new();
        expected_catalogs.insert("react18".to_string(), react18);
        expected_catalogs.insert("react17".to_string(), react17);
        assert_eq!(
            yarnrc,
            YarnRc {
                enable_transparent_workspaces: true,
                catalog: None,
                catalogs: Some(expected_catalogs),
            }
        );
    }
}
