use std::str::FromStr;

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
    #[error("unexpected non-utf8 yarn.lock")]
    NonUTF8(#[from] std::str::Utf8Error),
}

pub struct Yarn1Lockfile {
    inner: Map<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: Option<String>,
    version: String,
    uid: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
    registry: Option<String>,
    dependencies: Option<Map<String, String>>,
    optional_dependencies: Option<Map<String, String>>,
}

impl Yarn1Lockfile {
    pub fn from_bytes(input: &[u8]) -> Result<Self, super::Error> {
        let input = std::str::from_utf8(input).map_err(Error::from)?;
        Self::from_str(input)
    }
}

impl FromStr for Yarn1Lockfile {
    type Err = super::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = de::parse_syml(s)?;
        let inner = serde_json::from_value(value)?;
        Ok(Self { inner })
    }
}

impl Lockfile for Yarn1Lockfile {
    fn resolve_package(
        &self,
        _workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        for key in possible_keys(name, version) {
            if let Some(entry) = self.inner.get(&key) {
                return Ok(Some(crate::Package {
                    key,
                    version: entry.version.clone(),
                }));
            }
        }

        Ok(None)
    }

    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        let Some(entry) = self.inner.get(key) else {
            return Ok(None);
        };

        let all_deps: std::collections::HashMap<_, _> = entry.dependency_entries().collect();
        Ok(match all_deps.is_empty() {
            false => Some(all_deps),
            true => None,
        })
    }

    fn subgraph(
        &self,
        _workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, super::Error> {
        let mut inner = Map::new();

        for (key, entry) in packages.iter().filter_map(|key| {
            let entry = self.inner.get(key)?;
            Some((key, entry))
        }) {
            inner.insert(key.clone(), entry.clone());
        }

        Ok(Box::new(Self { inner }))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(self.to_string().into_bytes())
    }

    fn global_change_key(&self) -> Vec<u8> {
        // todo: need advice on impl for this
        vec![]
    }
}

pub fn yarn_subgraph(contents: &[u8], packages: &[String]) -> Result<Vec<u8>, crate::Error> {
    let lockfile = Yarn1Lockfile::from_bytes(contents)?;
    let pruned_lockfile = lockfile.subgraph(&[], packages)?;
    pruned_lockfile.encode()
}

impl Entry {
    fn dependency_entries(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.dependencies
            .iter()
            .flatten()
            .chain(self.optional_dependencies.iter().flatten())
            .map(|(k, v)| (k.clone(), v.clone()))
    }
}

const PROTOCOLS: &[&str] = ["", "npm:", "file:", "workspace:", "yarn:"].as_slice();

fn possible_keys<'a>(name: &'a str, version: &'a str) -> impl Iterator<Item = String> + 'a {
    PROTOCOLS
        .iter()
        .copied()
        .map(move |protocol| format!("{name}@{protocol}{version}"))
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
