use std::{any::Any, str::FromStr};

use serde::Deserialize;

use crate::Lockfile;

mod de;

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to parse: {0}")]
    SymlParse(String),
    #[error("unable to convert to structured syml: {0}")]
    SymlStructure(#[from] serde_json::Error),
    #[error("unexpected non-utf8 yarn.lock")]
    NonUTF8(#[from] std::str::Utf8Error),
    #[error("Turborepo cannot serialize Bun lockfiles.")]
    NotImplemented(),
}

#[derive(Debug)]
pub struct BunLockfile {
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

impl BunLockfile {
    pub fn from_bytes(input: &[u8]) -> Result<Self, super::Error> {
        let input = std::str::from_utf8(input).map_err(Error::from)?;
        Self::from_str(input)
    }
}

impl FromStr for BunLockfile {
    type Err = super::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = de::parse_syml(s)?;
        let inner = serde_json::from_value(value)?;
        Ok(Self { inner })
    }
}

impl Lockfile for BunLockfile {
    #[tracing::instrument(skip(self, _workspace_path))]
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

    #[tracing::instrument(skip(self))]
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
        Err(crate::Error::Bun(Error::NotImplemented()))
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        // Downcast returns none if the concrete type doesn't match
        // if the types don't match then we changed package managers
        any_other.downcast_ref::<Self>().is_none()
    }
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
    use super::*;
    const FULL: &str = include_str!("../../fixtures/yarn1full.lock");

    #[test]
    fn test_key_splitting() {
        let lockfile = BunLockfile::from_str(FULL).unwrap();
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
