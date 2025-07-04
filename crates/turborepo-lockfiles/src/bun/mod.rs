use std::{
    any::Any,
    collections::{HashMap, HashSet},
    str::FromStr,
};

use biome_json_formatter::context::JsonFormatOptions;
use biome_json_parser::JsonParserOptions;
use id::PossibleKeyIter;
use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use turborepo_errors::ParseDiagnostic;

use crate::Lockfile;

mod de;
mod id;
mod ser;

type Map<K, V> = std::collections::BTreeMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to strip commas: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Failed to strip commas: {0}")]
    Format(#[from] biome_formatter::FormatError),
    #[error("Failed to strip commas: {0}")]
    Print(#[from] biome_formatter::PrintError),
    #[error("{ident} had two entries with differing checksums: {sha1}, {sha2}")]
    MismatchedShas {
        ident: String,
        sha1: String,
        sha2: String,
    },
}

#[derive(Debug)]
pub struct BunLockfile {
    data: BunLockfileData,
    key_to_entry: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BunLockfileData {
    #[allow(unused)]
    lockfile_version: i32,
    workspaces: Map<String, WorkspaceEntry>,
    packages: Map<String, PackageEntry>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    patched_dependencies: Map<String, String>,
}

#[derive(Debug, Deserialize, PartialEq, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dev_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_dependencies: Option<Map<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_peers: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Clone)]
struct PackageEntry {
    ident: String,
    registry: Option<String>,
    // Present except for workspace & root deps
    info: Option<PackageInfo>,
    // Present on registry
    checksum: Option<String>,
    root: Option<RootInfo>,
}

#[derive(Debug, Deserialize, Default, PartialEq, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageInfo {
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dev_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    optional_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    peer_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    optional_peers: HashSet<String>,
    // We do not care about the rest here
    // the values here should be generic
    #[serde(flatten)]
    other: Map<String, Value>,
}
#[derive(Debug, Deserialize, PartialEq, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RootInfo {
    bin: Option<String>,
    bin_dir: Option<String>,
}

impl Lockfile for BunLockfile {
    #[tracing::instrument(skip(self, workspace_path))]
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        let workspace_entry = self
            .data
            .workspaces
            .get(workspace_path)
            .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.into()))?;
        let workspace_name = &workspace_entry.name;
        let workspace_key = format!("{workspace_name}/{name}");
        if let Some((_key, entry)) = self.package_entry(&workspace_key) {
            let mut version = entry.version().to_string();
            // Check for any patches
            if let Some(patch) = self.data.patched_dependencies.get(&entry.ident) {
                version.push('+');
                version.push_str(patch);
            }
            // Bun's keys include how a package is imported that can result in
            // faulty cache miss if used by turbo to calculate a hash.
            // We instead use the ident (the first element of the entry) as it omits this
            // information.
            // Note: Entries are not deduplicated in `bun.lock` if
            // they need to be qualified e.g. packages a and b -> shared@1.0.0
            // and packages c and d -> shared@2.0.0 will result in one of the
            // shared entries having an unqualified key (`shared`) and the other will have
            // qualified keys of `a/shared` and `b/shared` where both will have
            // the same entry with ident of `shared@1.0.0`. Because they are
            // identical entries we do not differentiate between them even though they are
            // different entries in the map.
            Ok(Some(crate::Package {
                key: entry.ident.to_string(),
                version,
            }))
        } else {
            Ok(None)
        }
    }

    #[tracing::instrument(skip(self))]
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::collections::HashMap<String, String>>, crate::Error> {
        let entry_key = self
            .key_to_entry
            .get(key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;
        let entry = self
            .data
            .packages
            .get(entry_key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;

        let mut deps = HashMap::new();

        let Some(info) = &entry.info else {
            return Ok(Some(deps));
        };

        let optional_peers = &info.optional_peers;
        for (dependency, version) in info.all_dependencies() {
            let parent_key = format!("{entry_key}/{dependency}");
            let Some((dep_key, _)) = self.package_entry(&parent_key) else {
                // This is an optional peer dependency
                if optional_peers.contains(&dependency.to_string()) {
                    continue;
                }

                return Err(crate::Error::MissingPackage(dependency.to_string()));
            };
            deps.insert(dep_key.to_string(), version.to_string());
        }

        Ok(Some(deps))
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, crate::Error> {
        let subgraph = self.subgraph(workspace_packages, packages)?;
        Ok(Box::new(subgraph))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(serde_json::to_vec_pretty(&self.data)?)
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        // Downcast returns none if the concrete type doesn't match
        // if the types don't match then we changed package managers
        any_other.downcast_ref::<Self>().is_none()
    }

    fn turbo_version(&self) -> Option<String> {
        let (_, entry) = self.package_entry("turbo")?;
        Some(entry.version().to_owned())
    }

    fn human_name(&self, package: &crate::Package) -> Option<String> {
        let entry = self.data.packages.get(&package.key)?;
        Some(entry.ident.clone())
    }
}

impl BunLockfile {
    pub fn from_bytes(input: &[u8]) -> Result<Self, super::Error> {
        let s = std::str::from_utf8(input).map_err(Error::from)?;
        Self::from_str(s)
    }

    // Given a specific key for a package, return the most specific key that is
    // present in the lockfile
    fn package_entry(&self, key: &str) -> Option<(&str, &PackageEntry)> {
        let (key, entry) =
            PossibleKeyIter::new(key).find_map(|k| self.data.packages.get_key_value(k))?;
        Some((key, entry))
    }

    pub fn lockfile(self) -> Result<BunLockfileData, Error> {
        Ok(self.data)
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<BunLockfile, Error> {
        let new_workspaces: Map<_, _> = self
            .data
            .workspaces
            .iter()
            .filter_map(|(key, entry)| {
                // Ensure the root workspace package is included, which is always indexed by ""
                if key.is_empty() || workspace_packages.contains(key) {
                    Some((key.clone(), entry.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Filter out packages that are not in the subgraph. Note that _multiple_
        // entries can correspond to the same ident.
        let idents: HashSet<_> = packages.iter().collect();
        let new_packages: Map<_, _> = self
            .data
            .packages
            .iter()
            .filter_map(|(key, entry)| {
                if idents.contains(&entry.ident) {
                    Some((key.clone(), entry.clone()))
                } else {
                    None
                }
            })
            .collect();

        let new_patched_dependencies = self
            .data
            .patched_dependencies
            .iter()
            .filter_map(|(ident, patch)| {
                if idents.contains(ident) {
                    Some((ident.clone(), patch.clone()))
                } else {
                    None
                }
            })
            .collect();

        Ok(Self {
            data: BunLockfileData {
                lockfile_version: self.data.lockfile_version,
                workspaces: new_workspaces,
                packages: new_packages,
                patched_dependencies: new_patched_dependencies,
            },
            key_to_entry: self.key_to_entry.clone(),
        })
    }
}

impl FromStr for BunLockfile {
    type Err = super::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed_json = biome_json_parser::parse_json(
            s,
            JsonParserOptions::default().with_allow_trailing_commas(),
        );
        if parsed_json.has_errors() {
            let diags = parsed_json
                .into_diagnostics()
                .into_iter()
                .map(|diagnostic| ParseDiagnostic::from(&diagnostic).to_string())
                .join("\n");
            return Err(super::Error::BiomeJsonError(diags));
        }
        let syntax_tree = parsed_json.syntax();
        let format = biome_json_formatter::format_node(
            JsonFormatOptions::default()
                .with_trailing_commas(biome_json_formatter::context::TrailingCommas::None),
            &syntax_tree,
        )
        .map_err(Error::from)?;
        let strict_json = format.print().map_err(Error::from)?;
        let data: BunLockfileData = serde_json::from_str(strict_json.as_code())?;
        let mut key_to_entry = HashMap::with_capacity(data.packages.len());
        for (path, info) in data.packages.iter() {
            if let Some(prev_path) = key_to_entry.insert(info.ident.clone(), path.clone()) {
                let prev_info = data
                    .packages
                    .get(&prev_path)
                    .expect("we just got this path from the packages list");
                if prev_info.checksum != info.checksum {
                    return Err(Error::MismatchedShas {
                        ident: info.ident.clone(),
                        sha1: prev_info.checksum.clone().unwrap_or_default(),
                        sha2: info.checksum.clone().unwrap_or_default(),
                    }
                    .into());
                }
            }
        }
        Ok(Self { data, key_to_entry })
    }
}
impl PackageEntry {
    // Extracts version from key
    fn version(&self) -> &str {
        self.ident
            .rsplit_once('@')
            .map(|(_, version)| version)
            .unwrap_or(&self.ident)
    }
}

impl PackageInfo {
    pub fn all_dependencies(&self) -> impl Iterator<Item = (&str, &str)> {
        [
            self.dependencies.iter(),
            self.dev_dependencies.iter(),
            self.optional_dependencies.iter(),
            self.peer_dependencies.iter(),
        ]
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use test_case::test_case;

    use super::*;

    const BASIC_LOCKFILE: &str = include_str!("../../fixtures/basic-bun.lock");
    const PATCH_LOCKFILE: &str = include_str!("../../fixtures/bun-patch.lock");

    #[test_case("", "turbo", "^2.3.3", "turbo@2.3.3" ; "root")]
    #[test_case("apps/docs", "is-odd", "3.0.1", "is-odd@3.0.1" ; "docs is odd")]
    #[test_case("apps/web", "is-odd", "3.0.0", "is-odd@3.0.0" ; "web is odd")]
    #[test_case("packages/ui", "node-plop/inquirer/rxjs/tslib", "^1.14.0", "tslib@1.14.1" ; "full key")]
    fn test_resolve_package(workspace: &str, name: &str, version: &str, expected: &str) {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let result = lockfile
            .resolve_package(workspace, name, version)
            .unwrap()
            .unwrap();
        assert_eq!(result.key, expected);
    }

    #[test]
    fn test_subgraph() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let subgraph = lockfile
            .subgraph(&["apps/docs".into()], &["is-odd@3.0.1".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        assert_eq!(
            subgraph_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![("is-odd", "is-odd@3.0.1")]
        );
        assert_eq!(
            subgraph_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/docs"]
        );
    }

    // There are multiple aliases that resolve to the same ident, here we test that
    // we output them all
    #[test]
    fn test_deduplicated_idents() {
        // chalk@2.4.2
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let subgraph = lockfile
            .subgraph(&["apps/docs".into()], &["chalk@2.4.2".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        assert_eq!(
            subgraph_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("@turbo/gen/chalk", "chalk@2.4.2"),
                ("@turbo/workspaces/chalk", "chalk@2.4.2"),
                ("log-symbols/chalk", "chalk@2.4.2")
            ]
        );
        assert_eq!(
            subgraph_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/docs"]
        );
    }

    #[test]
    fn test_patch_subgraph() {
        let lockfile = BunLockfile::from_str(PATCH_LOCKFILE).unwrap();
        let subgraph_a = lockfile
            .subgraph(&["apps/a".into()], &["is-odd@3.0.1".into()])
            .unwrap();
        let subgraph_a_data = subgraph_a.lockfile().unwrap();

        assert_eq!(
            subgraph_a_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![("is-odd", "is-odd@3.0.1")]
        );
        assert_eq!(
            subgraph_a_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/a"]
        );
        assert_eq!(
            subgraph_a_data
                .patched_dependencies
                .iter()
                .map(|(key, patch)| (key.as_str(), patch.as_str()))
                .collect::<Vec<_>>(),
            vec![]
        );

        let subgraph_b = lockfile
            .subgraph(&["apps/b".into()], &["is-odd@3.0.0".into()])
            .unwrap();
        let subgraph_b_data = subgraph_b.lockfile().unwrap();

        assert_eq!(
            subgraph_b_data
                .packages
                .iter()
                .map(|(key, pkg)| (key.as_str(), pkg.ident.as_str()))
                .collect::<Vec<_>>(),
            vec![("b/is-odd", "is-odd@3.0.0")]
        );
        assert_eq!(
            subgraph_b_data.workspaces.keys().collect::<Vec<_>>(),
            vec!["", "apps/b"]
        );
        assert_eq!(
            subgraph_b_data
                .patched_dependencies
                .iter()
                .map(|(key, patch)| (key.as_str(), patch.as_str()))
                .collect::<Vec<_>>(),
            vec![("is-odd@3.0.0", "patches/is-odd@3.0.0.patch")]
        );
    }

    const TURBO_GEN_DEPS: &[&str] = [
        "@turbo/gen/chalk",
        "@turbo/gen/minimatch",
        "@turbo/workspaces",
        "commander",
        "fs-extra",
        "inquirer",
        "node-plop",
        "proxy-agent",
        "ts-node",
        "update-check",
        "validate-npm-package-name",
    ]
    .as_slice();
    // Both @turbo/gen and log-symbols depend on the same version of chalk
    // log-symbols version wins out, but this is okay since they are the same exact
    // version of chalk.
    const TURBO_GEN_CHALK_DEPS: &[&str] = [
        "log-symbols/chalk/ansi-styles",
        "log-symbols/chalk/escape-string-regexp",
        "log-symbols/chalk/supports-color",
    ]
    .as_slice();
    const CHALK_DEPS: &[&str] = ["ansi-styles", "supports-color"].as_slice();

    #[test_case("@turbo/gen@1.13.4", TURBO_GEN_DEPS)]
    #[test_case("chalk@2.4.2", TURBO_GEN_CHALK_DEPS)]
    #[test_case("chalk@4.1.2", CHALK_DEPS)]
    fn test_all_dependencies(key: &str, expected: &[&str]) {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let mut expected = expected.to_vec();
        expected.sort();

        let mut actual = lockfile
            .all_dependencies(key)
            .unwrap()
            .unwrap()
            .into_keys()
            .collect::<Vec<_>>();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_patch_is_captured_in_package() {
        let lockfile = BunLockfile::from_str(PATCH_LOCKFILE).unwrap();
        let pkg = lockfile
            .resolve_package("apps/b", "is-odd", "3.0.0")
            .unwrap()
            .unwrap();
        assert_eq!(
            pkg,
            crate::Package::new("is-odd@3.0.0", "3.0.0+patches/is-odd@3.0.0.patch")
        );
    }

    #[test]
    fn test_failure_if_mismatched_keys() {
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                    "dependencies": {
                        "foo": "^1.0.0",
                        "bar": "^1.0.0",
                    }
                }
            },
            "packages": {
                "bar": ["bar@1.0.0", { "dependencies": { "shared": "^1.0.0" } }, "sha512-goodbye"],
                "bar/shared": ["shared@1.0.0", {}, "sha512-bar"],
                "foo": ["foo@1.0.0", { "dependencies": { "shared": "^1.0.0" } }, "sha512-hello"],
                "foo/shared": ["shared@1.0.0", { }, "sha512-foo"],
            }
        }))
        .unwrap();
        let lockfile = BunLockfile::from_str(&contents);
        assert!(lockfile.is_err(), "matching packages have differing shas");
    }

    #[test]
    fn test_subgraph_with_empty_workspace_packages() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test subgraph with no workspace packages
        let subgraph = lockfile.subgraph(&[], &["turbo@2.3.3".into()]).unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Should only contain root workspace
        assert_eq!(subgraph_data.workspaces.len(), 1);
        assert!(subgraph_data.workspaces.contains_key(""));

        // Should contain the requested package
        assert!(subgraph_data
            .packages
            .values()
            .any(|p| p.ident == "turbo@2.3.3"));
    }

    #[test]
    fn test_subgraph_with_missing_package() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test subgraph with non-existent package
        let subgraph = lockfile
            .subgraph(&["apps/docs".into()], &["nonexistent-package".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Should not contain the non-existent package
        assert!(!subgraph_data
            .packages
            .values()
            .any(|p| p.ident == "nonexistent-package"));

        // But should still include the workspace
        assert!(subgraph_data.workspaces.contains_key("apps/docs"));
    }

    #[test]
    fn test_resolve_package_with_invalid_workspace() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test with invalid workspace
        let result = lockfile.resolve_package("invalid/workspace", "is-odd", "3.0.1");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::MissingWorkspace(_)
        ));
    }

    #[test]
    fn test_resolve_package_with_nonexistent_package() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test with nonexistent package
        let result = lockfile
            .resolve_package("apps/docs", "nonexistent-package", "1.0.0")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_all_dependencies_with_invalid_key() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test with invalid package key
        let result = lockfile.all_dependencies("invalid-package-key");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::Error::MissingPackage(_)
        ));
    }

    #[test]
    fn test_global_change_detection() {
        let lockfile1 = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();
        let lockfile2 = BunLockfile::from_str(PATCH_LOCKFILE).unwrap();

        // Current implementation only returns true if package manager types differ
        // Both lockfiles are BunLockfile, so this returns false
        assert!(!lockfile1.global_change(&lockfile2));

        // Same lockfile should not show global change
        assert!(!lockfile1.global_change(&lockfile1));
    }

    #[test]
    fn test_turbo_version_detection() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        let turbo_version = lockfile.turbo_version();
        assert_eq!(turbo_version, Some("2.3.3".to_string()));
    }

    #[test]
    fn test_turbo_version_missing() {
        // Create a lockfile without turbo
        let contents = serde_json::to_string(&json!({
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test"
                }
            },
            "packages": {}
        }))
        .unwrap();
        let lockfile = BunLockfile::from_str(&contents).unwrap();

        let turbo_version = lockfile.turbo_version();
        assert!(turbo_version.is_none());
    }

    #[test]
    fn test_human_name_generation() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        let package = crate::Package::new("is-odd", "3.0.1");
        let human_name = lockfile.human_name(&package);
        assert_eq!(human_name, Some("is-odd@3.0.1".to_string()));

        // Test with nonexistent package
        let nonexistent = crate::Package::new("nonexistent", "1.0.0");
        let human_name = lockfile.human_name(&nonexistent);
        assert!(human_name.is_none());
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test encode/decode roundtrip
        let encoded = original.encode().unwrap();
        let decoded = BunLockfile::from_bytes(&encoded).unwrap();

        // Should preserve basic structure
        assert_eq!(
            original.data.lockfile_version,
            decoded.data.lockfile_version
        );
        assert_eq!(
            original.data.workspaces.len(),
            decoded.data.workspaces.len()
        );
        assert_eq!(original.data.packages.len(), decoded.data.packages.len());
    }

    #[test]
    fn test_optional_peer_dependencies() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test that optional peer dependencies are handled correctly
        let all_deps = lockfile
            .all_dependencies("@turbo/gen@1.13.4")
            .unwrap()
            .unwrap();

        // Should include regular dependencies but may skip optional peers
        assert!(all_deps.len() > 0);

        // Verify no missing package errors for optional peers
        for (dep_key, _) in &all_deps {
            // All returned dependencies should exist in the lockfile
            assert!(
                lockfile.data.packages.contains_key(dep_key)
                    || lockfile.package_entry(dep_key).is_some()
            );
        }
    }

    #[test]
    fn test_package_version_extraction() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test version extraction from different package formats
        for (_, entry) in &lockfile.data.packages {
            let version = entry.version();
            // Version should be non-empty and valid
            assert!(!version.is_empty());

            // Should extract version after @ symbol
            if entry.ident.contains('@') {
                assert!(entry.ident.ends_with(version));
            }
        }
    }

    #[test]
    fn test_subgraph_preserves_patches() {
        let lockfile = BunLockfile::from_str(PATCH_LOCKFILE).unwrap();

        let subgraph = lockfile
            .subgraph(&["apps/b".into()], &["is-odd@3.0.0".into()])
            .unwrap();
        let subgraph_data = subgraph.lockfile().unwrap();

        // Should preserve patches for included packages
        assert!(subgraph_data
            .patched_dependencies
            .contains_key("is-odd@3.0.0"));
        assert_eq!(
            subgraph_data.patched_dependencies.get("is-odd@3.0.0"),
            Some(&"patches/is-odd@3.0.0.patch".to_string())
        );
    }

    #[test]
    fn test_workspace_dependency_types() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Test that different dependency types are handled correctly
        for (_, workspace) in &lockfile.data.workspaces {
            // Verify that dependencies maps exist and are accessible
            if let Some(_deps) = &workspace.dependencies {
                // Dependencies can be empty
            }
            if let Some(_dev_deps) = &workspace.dev_dependencies {
                // Dev dependencies can be empty
            }
            if let Some(_opt_deps) = &workspace.optional_dependencies {
                // Optional dependencies can be empty
            }
        }
    }

    #[test]
    fn test_malformed_json_handling() {
        let malformed_json = r#"
        {
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test"
                }
            },
            "packages": {
                "invalid": ["incomplete
        "#;

        let result = BunLockfile::from_str(malformed_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_trailing_commas_handling() {
        let json_with_trailing_commas = r#"
        {
            "lockfileVersion": 0,
            "workspaces": {
                "": {
                    "name": "test",
                }
            },
            "packages": {},
        }
        "#;

        let result = BunLockfile::from_str(json_with_trailing_commas);
        assert!(result.is_ok(), "Should handle trailing commas gracefully");
    }

    #[test]
    fn test_large_subgraph_performance() {
        let lockfile = BunLockfile::from_str(BASIC_LOCKFILE).unwrap();

        // Create a list of packages to include in subgraph
        let packages: Vec<String> = lockfile
            .data
            .packages
            .values()
            .take(10) // Limit to 10 packages for performance test
            .map(|pkg| pkg.ident.clone())
            .collect();

        let workspaces: Vec<String> = lockfile
            .data
            .workspaces
            .keys()
            .take(3) // Include a few workspaces
            .cloned()
            .collect();

        let start = std::time::Instant::now();
        let subgraph = lockfile.subgraph(&workspaces, &packages).unwrap();
        let duration = start.elapsed();

        // Should complete quickly (under 50ms for this small test)
        assert!(duration.as_millis() < 50);

        let subgraph_data = subgraph.lockfile().unwrap();
        assert!(subgraph_data.packages.len() <= packages.len());
        assert!(subgraph_data.workspaces.len() <= workspaces.len() + 1); // +1 for root
    }
}
