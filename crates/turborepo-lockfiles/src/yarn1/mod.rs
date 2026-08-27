use std::{any::Any, str::FromStr};

use semver::Version;
use serde::Deserialize;

use crate::Lockfile;

mod de;
mod fast_parse;
mod ser;

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to parse: {0}")]
    SymlParse(String),
    #[error("Unable to convert to structured syml: {0}")]
    SymlStructure(#[from] serde_json::Error),
    #[error("Unexpected non-utf8 yarn.lock")]
    NonUTF8(#[from] std::str::Utf8Error),
}

#[derive(Debug)]
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
        // Single-pass fast path for the machine-generated subset; the nom
        // parser below is the parser of record and handles everything the
        // fast path declines.
        if let Some(inner) = fast_parse::parse(s) {
            return Ok(Self { inner });
        }
        tracing::debug!("yarn1 fast parse declined, using general parser");
        let value = de::parse_syml(s)?;
        let inner = serde_json::from_value(value)?;
        Ok(Self { inner })
    }
}

impl Lockfile for Yarn1Lockfile {
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
    ) -> Result<
        Option<std::borrow::Cow<'_, std::collections::BTreeMap<String, String>>>,
        crate::Error,
    > {
        let Some(entry) = self.inner.get(key) else {
            return Ok(None);
        };

        let all_deps: std::collections::BTreeMap<_, _> = entry.dependency_entries().collect();
        Ok(match all_deps.is_empty() {
            false => Some(std::borrow::Cow::Owned(all_deps)),
            true => None,
        })
    }

    fn transitive_edge_resolver(&self) -> Option<Box<dyn crate::TransitiveEdgeResolver + '_>> {
        Some(Box::new(Yarn1EdgeResolver { lockfile: self }))
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, super::Error> {
        let mut inner = Map::new();

        for (key, entry) in packages.iter().filter_map(|key| {
            let entry = self.inner.get(key)?;
            Some((key, entry))
        }) {
            inner.insert(key.clone(), entry.clone());
        }

        // Yarn v1 creates lockfile entries for `file:` protocol dependencies
        // that point to workspace packages. These are classified as internal
        // by the dependency splitter and therefore not included in `packages`,
        // but they must be present in the pruned lockfile for
        // `yarn install --frozen-lockfile` to succeed.
        for (key, entry) in &self.inner {
            if let Some(file_path) = extract_file_path(key) {
                let normalized = file_path.strip_prefix("./").unwrap_or(file_path);
                if workspace_packages
                    .iter()
                    .any(|wp| normalized == wp || normalized.ends_with(&format!("/{wp}")))
                {
                    inner.insert(key.clone(), entry.clone());
                }
            }
        }

        Ok(Box::new(Self { inner }))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(self.to_string().into_bytes())
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        // Downcast returns none if the concrete type doesn't match
        // if the types don't match then we changed package managers
        any_other.downcast_ref::<Self>().is_none()
    }

    fn turbo_version(&self) -> Option<String> {
        // Yarn lockfiles can have multiple descriptors as a key e.g. turbo@latest,
        // turbo@1.2.3 We just check if the first descriptor is for turbo and
        // return that. Using multiple versions of turbo in a single project is
        // not supported.
        let key = self.inner.keys().find(|key| key.starts_with("turbo@"))?;
        let entry = self.inner.get(key)?;
        let version = &entry.version;
        Version::parse(version).ok()?;
        Some(version.clone())
    }

    fn human_name(&self, package: &crate::Package) -> Option<String> {
        let entry = self.inner.get(&package.key)?;
        let name = entry
            .name
            .as_deref()
            .or_else(|| package_name_from_key(&package.key))?;
        let version = &entry.version;
        Some(format!("{name}@{version}"))
    }

    fn package_source(&self, package: &crate::Package) -> crate::PackageSource {
        let Some(entry) = self.inner.get(&package.key) else {
            return crate::PackageSource::Registry;
        };
        entry
            .resolved
            .as_deref()
            .map(crate::package_source_from_identifier)
            .unwrap_or_else(|| crate::package_source_from_identifier(&package.key))
    }

    fn format_version(&self) -> String {
        "1".to_string()
    }
}

/// Proves per-edge workspace independence for the shared closure DP.
///
/// yarn1 resolution never consults the workspace: `resolve_package` ignores
/// its workspace argument entirely and resolves purely from the lockfile's
/// `name@specifier` keys, so every edge is globally uniform by construction.
struct Yarn1EdgeResolver<'a> {
    lockfile: &'a Yarn1Lockfile,
}

impl crate::TransitiveEdgeResolver for Yarn1EdgeResolver<'_> {
    fn resolve_edge(
        &self,
        name: &str,
        version: &str,
    ) -> Result<crate::TransitiveEdgeResolution, crate::Error> {
        Ok(crate::TransitiveEdgeResolution::Global(
            self.lockfile.resolve_package("", name, version)?,
        ))
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

/// Extracts the file path from a yarn1 lockfile key like
/// `@scope/pkg@file:./packages/foo`. Returns `None` if the key doesn't use
/// the `file:` protocol.
fn extract_file_path(key: &str) -> Option<&str> {
    // Keys look like `name@file:path` or `name@npm:version`.
    // The name may be scoped (`@scope/pkg@file:path`) so we find `@file:`
    // anywhere after the first character.
    let idx = key[1..].find("@file:")? + 1;
    Some(&key[idx + "@file:".len()..])
}

fn possible_keys<'a>(name: &'a str, version: &'a str) -> impl Iterator<Item = String> + 'a {
    PROTOCOLS
        .iter()
        .copied()
        .map(move |protocol| format!("{name}@{protocol}{version}"))
}

fn package_name_from_key(key: &str) -> Option<&str> {
    let search_start = usize::from(key.starts_with('@'));
    let delimiter = key[search_start..].find('@')? + search_start;
    Some(&key[..delimiter])
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_extract_file_path() {
        assert_eq!(
            extract_file_path("@hardfin/eslint-config@file:./packages/eslint-config"),
            Some("./packages/eslint-config")
        );
        assert_eq!(
            extract_file_path("my-pkg@file:packages/foo"),
            Some("packages/foo")
        );
        assert_eq!(extract_file_path("lodash@^4.17.21"), None);
        assert_eq!(extract_file_path("@scope/pkg@npm:1.0.0"), None);
    }

    #[test]
    fn test_human_name_uses_resolved_version() {
        let lockfile = Yarn1Lockfile::from_str(
            r#"# yarn lockfile v1

lodash@^4.17.21:
  version "4.17.21"

"@scope/pkg@npm:^1.0.0":
  version "1.2.3"
"#,
        )
        .unwrap();

        assert_eq!(
            lockfile.human_name(&crate::Package {
                key: "lodash@^4.17.21".to_string(),
                version: "4.17.21".to_string(),
            }),
            Some("lodash@4.17.21".to_string())
        );
        assert_eq!(
            lockfile.human_name(&crate::Package {
                key: "@scope/pkg@npm:^1.0.0".to_string(),
                version: "1.2.3".to_string(),
            }),
            Some("@scope/pkg@1.2.3".to_string())
        );
    }

    #[test]
    fn test_subgraph_includes_file_deps_for_workspaces() {
        // Reproduces https://github.com/vercel/turborepo/issues/4105
        // file: dependencies pointing to workspace packages must appear in
        // the pruned lockfile even though they are classified as internal.
        let lockfile_content = r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1


"@repo/eslint-config@file:./packages/eslint-config":
  version "0.0.0"
  dependencies:
    eslint-config-prettier "8.6.0"

eslint-config-prettier@8.6.0:
  version "8.6.0"
  resolved "https://registry.yarnpkg.com/eslint-config-prettier/-/eslint-config-prettier-8.6.0.tgz"
  integrity sha512-abc

is-odd@^3.0.1:
  version "3.0.1"
  resolved "https://registry.yarnpkg.com/is-odd/-/is-odd-3.0.1.tgz"
  integrity sha512-def
"#;
        let lockfile = Yarn1Lockfile::from_str(lockfile_content).unwrap();

        // The transitive closure only contains the normal package — the file:
        // dep was classified as internal so it won't be in `packages`.
        let packages = vec!["is-odd@^3.0.1".to_string()];
        let workspace_packages = vec!["packages/eslint-config".to_string()];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = String::from_utf8(pruned.encode().unwrap()).unwrap();

        assert!(
            encoded.contains("@repo/eslint-config@file:./packages/eslint-config"),
            "pruned lockfile must include file: entry for workspace package"
        );
        assert!(
            encoded.contains("is-odd@^3.0.1"),
            "pruned lockfile must include normal packages"
        );
    }

    #[test]
    fn test_turbo_version_rejects_non_semver() {
        // Malicious version strings that could be used for RCE via npx should be
        // rejected
        let malicious_versions = [
            "file:./malicious.tgz",
            "https://evil.com/malicious.tgz",
            "git+https://github.com/evil/repo.git",
            "../../../etc/passwd",
            "1.0.0 && curl evil.com",
        ];

        for malicious_version in malicious_versions {
            let lockfile_content = format!(
                r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1


turbo@^1.0.0:
  version "{malicious_version}"
  resolved "https://registry.yarnpkg.com/turbo/-/turbo-1.0.0.tgz#abc123"
"#
            );
            let lockfile = Yarn1Lockfile::from_str(&lockfile_content).unwrap();
            assert_eq!(
                lockfile.turbo_version(),
                None,
                "should reject malicious version: {}",
                malicious_version
            );
        }
    }
}
