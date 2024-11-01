use std::io;

use serde::Deserialize;
use serde_yaml;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("encountered error parsing yarnrc.yml: {0}")]
    SerdeYaml(#[from] serde_yaml::Error),
}

/// A yarnrc.yaml file representing settings affecting the package graph.
#[allow(non_snake_case)]
#[derive(Debug, PartialEq, Eq, Clone, Deserialize)]
pub struct YarnRc {
    /// Used by Yarn(Berry) as `enableTransparentWorkspaces`.
    /// When true, treats local workspaces that match a package name
    /// and semver range as correct match resulting in turbo including
    /// the package in the dependency graph
    pub enableTransparentWorkspaces: bool,
}

impl Default for YarnRc {
    fn default() -> YarnRc {
        YarnRc {
            enableTransparentWorkspaces: true,
        }
    }
}

impl YarnRc {
    pub fn from_reader(mut reader: impl io::Read) -> Result<Self, Error> {
        let config: YarnRc = serde_yaml::from_reader(&mut reader)?;
        Ok(config)
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
                enableTransparentWorkspaces: true
            }
        );
    }

    #[test]
    fn test_parses_transparent_workspaces() {
        let empty = YarnRc::from_reader(b"enableTransparentWorkspaces: false".as_slice()).unwrap();
        assert_eq!(
            empty,
            YarnRc {
                enableTransparentWorkspaces: false
            }
        );
    }
}
