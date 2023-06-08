use std::{borrow::Cow, rc::Rc};

use serde::Deserialize;

use crate::Lockfile;

mod de;
mod ser;

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to parse: {0}")]
    SymlParse(String),
    #[error("unable to convert to structured syml: {0}")]
    SymlStructure(#[from] serde_json::Error),
}

pub struct Yarn1Lockfile {
    inner: Map<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: Option<String>,
    version: Option<String>,
    uid: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
    registry: Option<String>,
    dependencies: Option<Map<String, String>>,
    optional_dependencies: Option<Map<String, String>>,
}

impl Yarn1Lockfile {
    pub fn from_str(input: &str) -> Result<Self, Error> {
        let value = de::parse_syml(input)?;
        let inner = serde_json::from_value(value)?;
        Ok(Self { inner })
    }
}

impl Lockfile for Yarn1Lockfile {
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        todo!()
    }

    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        todo!()
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use test_case::test_case;

    use super::*;

    const MINIMAL: &str = include_str!("../../fixtures/yarn1.lock");
    const FULL: &str = include_str!("../../fixtures/yarn1full.lock");

    #[test_case(MINIMAL ; "minimal lockfile")]
    #[test_case(FULL ; "full lockfile")]
    fn test_roundtrip(input: &str) {
        let lockfile = Yarn1Lockfile::from_str(input).unwrap();
        assert_eq!(input, lockfile.to_string());
    }

    #[test]
    fn test_key_splitting() {
        let lockfile = Yarn1Lockfile::from_str(FULL).unwrap();
        for key in [
            "@babel/types@^7.18.10",
            "@babel/types@^7.18.6",
            "@babel/types@^7.19.0",
        ] {
            assert!(
                lockfile.inner.contains_key(key),
                "missing {} in lockfile",
                key
            );
        }
    }
}
