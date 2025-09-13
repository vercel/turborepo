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

        let workspace_versions = workspace_packages
            .iter()
            .map(|workspace| format!("workspace:{workspace}"))
            .collect::<HashSet<_>>();

        // Filter out packages that are not in the subgraph. Note that _multiple_
        // entries can correspond to the same ident.
        let idents: HashSet<_> = packages.iter().map(|s| s.as_str()).collect();
        #[allow(clippy::if_same_then_else)]
        let new_packages: Map<_, _> = self
            .data
            .packages
            .iter()
            .filter_map(|(key, entry)| {
                let should_include_entry = (idents.contains(entry.ident.as_str())
                // If the entry is scoped to a specific package, only include the entry
                // if the closure includes the introducing package
                && PossibleKeyIter::new(key).skip(1)
                        .last()
                        .is_none_or(|pkg| idents.contains(pkg)))
                    // If the entry is for a workspace in the pruned lockfile also include it
                    || workspace_versions
                        .iter()
                        .any(|workspace| entry.ident.ends_with(workspace));
                if should_include_entry {
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
                if idents.contains(ident.as_str()) {
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
    const GH_10410: &str = include_str!("../../fixtures/bun-gh10410.lock");

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
    fn gh_10410() {
        let lockfile = BunLockfile::from_str(GH_10410).unwrap();
        let closure = crate::transitive_closure(
            &lockfile,
            "packages/a",
            vec![
                ("@types/compression", "^1.7.5"),
                ("@types/cookie-parser", "^1.4.8"),
                ("@types/express", "^4.17.21"),
                ("@types/bun", "^1.2.11"),
                ("@babel/register", "7.25.9"),
                ("eslint", "9.25.1"),
                ("compression", "^1.8.0"),
                ("cookie-parser", "^1.4.7"),
                ("express", "^4.21.2"),
                ("express-openapi-validator", "^5.4.9"),
                ("http-errors", "^2.0.0"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
            false,
        )
        .unwrap();
        let packages = closure
            .iter()
            .map(|pkg| pkg.key.clone())
            .collect::<Vec<_>>();
        let pruned_lockfile = lockfile
            .subgraph(&["packages/a".into()], &packages)
            .unwrap();
        assert!(!pruned_lockfile
            .data
            .packages
            .contains_key("@react-native/community-cli-plugin/debug"));
        assert!(pruned_lockfile
            .data
            .packages
            .contains_key("bun-turbo-prune-repro-a"));
        // This is a renamed import from metro
        assert!(!pruned_lockfile
            .data
            .packages
            .contains_key("@babel/traverse--for-generate-function-map"));
        assert!(!pruned_lockfile.data.packages.contains_key("p-try"));
        assert!(!pruned_lockfile.data.packages.contains_key("fast-uri"));
        assert!(!pruned_lockfile
            .data
            .packages
            .contains_key("require-from-string"));
    }
}
