use std::{any::Any, collections::HashMap, str::FromStr};

use biome_json_formatter::context::JsonFormatOptions;
use biome_json_parser::JsonParserOptions;
use id::PossibleKeyIter;
use itertools::Itertools as _;
use serde::Deserialize;
use serde_json::Value;
use turborepo_errors::ParseDiagnostic;

use crate::Lockfile;

mod de;
mod id;

type Map<K, V> = std::collections::HashMap<K, V>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to strip commas: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Failed to strip commas: {0}")]
    Format(#[from] biome_formatter::FormatError),
    #[error("Failed to strip commas: {0}")]
    Print(#[from] biome_formatter::PrintError),
    #[error("Turborepo cannot serialize Bun lockfiles.")]
    NotImplemented,
}

#[derive(Debug)]
pub struct BunLockfile {
    data: BunLockfileData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BunLockfileData {
    #[allow(unused)]
    lockfile_version: i32,
    workspaces: Map<String, WorkspaceEntry>,
    packages: Map<String, PackageEntry>,
    #[serde(default)]
    patched_dependencies: Map<String, String>,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
struct WorkspaceEntry {
    name: String,
    version: Option<String>,
    dependencies: Option<Map<String, String>>,
    dev_dependencies: Option<Map<String, String>>,
    optional_dependencies: Option<Map<String, String>>,
    peer_dependencies: Option<Map<String, String>>,
    optional_peers: Option<Vec<String>>,
}

#[derive(Debug, PartialEq)]
struct PackageEntry {
    ident: String,
    registry: Option<String>,
    // Present except for workspace & root deps
    info: Option<PackageInfo>,
    // Present on registry
    checksum: Option<String>,
    root: Option<RootInfo>,
}

#[derive(Debug, Deserialize, Default, PartialEq)]
struct PackageInfo {
    #[serde(default)]
    dependencies: Map<String, String>,
    #[serde(default)]
    dev_dependencies: Map<String, String>,
    #[serde(default)]
    optional_dependencies: Map<String, String>,
    #[serde(default)]
    peer_dependencies: Map<String, String>,
    // We do not care about the rest here
    // the values here should be generic
    #[serde(flatten)]
    other: Map<String, Value>,
}
#[derive(Debug, Deserialize, PartialEq)]
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
        let entry = self
            .data
            .packages
            .get(key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;

        let mut deps = HashMap::new();
        for (dependency, version) in entry.info.iter().flat_map(|info| info.all_dependencies()) {
            let parent_key = format!("{key}/{dependency}");
            let Some((dep_key, _)) = self.package_entry(&parent_key) else {
                return Err(crate::Error::MissingPackage(dependency.to_string()));
            };
            deps.insert(dep_key.to_string(), version.to_string());
        }

        Ok(Some(deps))
    }

    #[allow(unused)]
    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, super::Error> {
        Err(crate::Error::Bun(Error::NotImplemented))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Err(crate::Error::Bun(Error::NotImplemented))
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
        let data = serde_json::from_str(strict_json.as_code())?;
        Ok(Self { data })
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
    const TURBO_GEN_CHALK_DEPS: &[&str] = [
        "@turbo/gen/chalk/ansi-styles",
        "@turbo/gen/chalk/escape-string-regexp",
        "@turbo/gen/chalk/supports-color",
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
}
