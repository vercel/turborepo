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
}

fn default_enable_transparent_workspaces() -> bool {
    true
}

impl Default for YarnRc {
    fn default() -> YarnRc {
        YarnRc {
            enable_transparent_workspaces: default_enable_transparent_workspaces(),
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
                enable_transparent_workspaces: true
            }
        );
    }

    #[test]
    fn test_parses_transparent_workspaces() {
        let empty = YarnRc::from_reader(b"enableTransparentWorkspaces: false".as_slice()).unwrap();
        assert_eq!(
            empty,
            YarnRc {
                enable_transparent_workspaces: false
            }
        );
    }

    #[test]
    fn test_parses_additional_settings() {
        let empty = YarnRc::from_reader(b"httpProxy: \"http://my-proxy.com\"".as_slice()).unwrap();
        assert_eq!(
            empty,
            YarnRc {
                enable_transparent_workspaces: true
            }
        );
    }
}
