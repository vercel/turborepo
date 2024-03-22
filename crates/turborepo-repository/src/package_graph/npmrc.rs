use std::io;

use ini::Ini;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("encountered error parsing .npmrc: {0}")]
    Ini(#[from] ini::Error),
}

/// Representation of .npmrc used by both npm and pnpm to configure behavior
/// The representation is intentionally incomplete and is only intended to
/// contain settings that affect the package graph.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct NpmRc {
    /// Used by pnpm to determine whether dependencies
    /// declared without an explicit workspace protocol
    /// can target a workspace package.
    pub link_workspace_packages: Option<bool>,
}

impl NpmRc {
    pub fn from_reader(mut reader: impl io::Read) -> Result<Self, Error> {
        let ini = Ini::read_from(&mut reader)?;
        Ok(Self::from_ini(ini))
    }

    // Private to avoid leaking the underlying ini parsing library we use
    fn from_ini(ini: Ini) -> Self {
        let link_workspace_packages = ini
            .get_from::<&str>(None, "link-workspace-packages")
            .and_then(parse_link_workspace_packages);

        Self {
            link_workspace_packages,
        }
    }
}

fn parse_link_workspace_packages(value: &str) -> Option<bool> {
    match value {
        // "deep" changes the underlying linking strategy used by pnpm, but it still results
        // in workspace packages being used over npm packages
        "true" | "deep" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_empty_npmrc() {
        let empty = NpmRc::from_reader(b"".as_slice()).unwrap();
        assert_eq!(
            empty,
            NpmRc {
                link_workspace_packages: None
            }
        );
    }

    #[test]
    fn test_example_pnpm_npmrc() {
        let contents = b"auto-install-peers=true
        enable-pre-post-scripts=true
        link-workspace-packages=false
        "
        .as_slice();
        let example = NpmRc::from_reader(contents).unwrap();
        assert_eq!(
            example,
            NpmRc {
                link_workspace_packages: Some(false)
            }
        );
    }
}
