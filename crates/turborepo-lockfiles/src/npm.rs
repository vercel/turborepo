use std::{any::Any, collections::HashMap};

use semver::Version;
use serde::{Deserialize, Serialize, ser::SerializeMap};
use serde_json::Value;

use super::{Error, Lockfile, Package};

type Map<K, V> = std::collections::BTreeMap<K, V>;

// we change graph traversal now
// resolve_package should only be used now for converting initial contents
// of workspace package.json into a set of node ids
#[derive(Debug, Default, Deserialize)]
pub struct NpmLockfile {
    #[serde(rename = "lockfileVersion")]
    lockfile_version: i32,
    packages: HashMap<String, NpmPackage>,
    // We parse this so it doesn't end up in 'other' and we don't need to worry
    // about accidentally serializing it.
    #[serde(skip_serializing, default)]
    dependencies: Map<String, Value>,
    // We want to reserialize any additional fields, but we don't use them
    // we keep them as raw values to avoid describing the correct schema.
    #[serde(flatten)]
    other: Map<String, Value>,
}

impl Serialize for NpmLockfile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Sort packages keys for deterministic output matching npm's sorted
        // lockfile format.
        let mut sorted_packages: Vec<_> = self.packages.iter().collect();
        sorted_packages.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        let field_count = 2 + self.other.len(); // lockfileVersion + packages + flattened other fields
        let mut map = serializer.serialize_map(Some(field_count))?;
        map.serialize_entry("lockfileVersion", &self.lockfile_version)?;

        // Serialize sorted packages as a JSON object
        map.serialize_entry("packages", &SortedPackages(&sorted_packages))?;

        for (k, v) in &self.other {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

struct SortedPackages<'a>(&'a [(&'a String, &'a NpmPackage)]);

impl Serialize for SortedPackages<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NpmPackage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    integrity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    license: Option<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    dev: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    optional: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    peer: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    link: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    has_install_script: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deprecated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bin: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    engines: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    os: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cpu: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    funding: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspaces: Option<Value>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    dev_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    peer_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    optional_dependencies: Map<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peer_dependencies_meta: Option<Value>,
    // Fallback for any fields not explicitly enumerated above. Using flatten
    // here is still correct — but the vast majority of packages will have an
    // empty `other` because all common fields are now enumerated, so the
    // flatten overhead is minimal.
    #[serde(flatten)]
    other: Map<String, Value>,
}

impl Lockfile for NpmLockfile {
    #[tracing::instrument(skip(self, _version))]
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        _version: &str,
    ) -> Result<Option<Package>, Error> {
        if !self.packages.contains_key(workspace_path) {
            return Err(Error::MissingWorkspace(workspace_path.to_string()));
        }

        let possible_keys = [
            // AllDependencies will return a key to avoid choosing the incorrect transitive dep
            name.to_string(),
            // If we didn't find the entry just using name, then this is an initial call to
            // ResolvePackage based on information coming from internal packages'
            // package.json First we check if the workspace uses a nested version of
            // the package
            format!("{workspace_path}/node_modules/{name}"),
            // Next we check for a top level version of the package
            format!("node_modules/{name}"),
        ];
        possible_keys
            .into_iter()
            .filter_map(|key| {
                self.packages.get(&key).map(|pkg| {
                    let version = pkg.version.clone().unwrap_or_default();
                    Ok(Package { key, version })
                })
            })
            .next()
            .transpose()
    }

    #[tracing::instrument(skip(self))]
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<Option<std::borrow::Cow<'_, std::collections::BTreeMap<String, String>>>, Error>
    {
        let Some(pkg) = self.packages.get(key) else {
            return Ok(None);
        };

        let mut deps = std::collections::BTreeMap::new();
        let mut buf = String::new();
        for name in pkg.dep_keys() {
            if let Some((resolved_key, version)) = self.find_dep_in_lockfile(key, name, &mut buf)? {
                deps.insert(resolved_key, version);
            }
        }
        Ok(Some(std::borrow::Cow::Owned(deps)))
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, Error> {
        let mut pruned_packages = HashMap::new();
        for pkg_key in packages {
            let pkg = self.get_package(pkg_key)?;
            pruned_packages.insert(pkg_key.to_string(), pkg.clone());
        }
        if let Some(root) = self.packages.get("") {
            pruned_packages.insert("".into(), root.clone());
        }
        for workspace in workspace_packages {
            let pkg = self.get_package(workspace)?;
            pruned_packages.insert(workspace.to_string(), pkg.clone());

            for (key, entry) in &self.packages {
                if entry.resolved.as_deref() == Some(workspace.as_str()) {
                    pruned_packages.insert(key.clone(), entry.clone());
                    break;
                }
            }
        }

        // After pruning, a package nested under a workspace's node_modules
        // (e.g. `apps/web/node_modules/next@15`) may exist without a
        // corresponding hoisted version (`node_modules/next`) if the hoisted
        // version was only needed by a now-pruned workspace and the transitive
        // closure didn't include it. Promote the nested version to the hoisted
        // position so npm ci sees a consistent tree.
        // See https://github.com/vercel/turborepo/issues/10985
        let ws_set: std::collections::HashSet<&str> =
            workspace_packages.iter().map(|s| s.as_str()).collect();
        let requested: std::collections::HashSet<&str> =
            packages.iter().map(|s| s.as_str()).collect();
        Self::rehoist_packages(&mut pruned_packages, &ws_set, &requested, &self.packages);

        Ok(Box::new(Self {
            lockfile_version: self.lockfile_version,
            packages: pruned_packages,
            dependencies: Map::default(),
            other: self.other.clone(),
        }))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        Ok(serde_json::to_vec_pretty(&self)?)
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        if let Some(other) = any_other.downcast_ref::<Self>() {
            self.lockfile_version != other.lockfile_version
                || self.other.get("requires") != other.other.get("requires")
        } else {
            true
        }
    }

    fn turbo_version(&self) -> Option<String> {
        let turbo_entry = self.packages.get("node_modules/turbo")?;
        let version = turbo_entry.version.as_ref()?;
        Version::parse(version).ok()?;
        Some(version.clone())
    }

    fn human_name(&self, package: &Package) -> Option<String> {
        let npm_package = self.packages.get(&package.key)?;
        let version = npm_package.version.as_deref()?;
        let name = package.key.split("node_modules/").last()?;
        Some(format!("{name}@{version}"))
    }
}

impl NpmLockfile {
    pub fn load(content: &[u8]) -> Result<Self, Error> {
        let lockfile: NpmLockfile = serde_json::from_slice(content)?;

        // We don't support lockfiles without 'packages' as older versions
        // required reading through the contents of node_modules in order
        // to resolve dependencies.
        // See https://github.com/npm/cli/blob/9609e9eed87c735f0319ac0af265f4d406cbf800/workspaces/arborist/lib/shrinkwrap.js#L674
        if lockfile.lockfile_version <= 1
            || (lockfile.packages.is_empty() && !lockfile.dependencies.is_empty())
        {
            Err(Error::UnsupportedNpmVersion)
        } else {
            Ok(lockfile)
        }
    }

    fn get_package(&self, package: impl AsRef<str>) -> Result<&NpmPackage, Error> {
        let pkg_str = package.as_ref();
        self.packages
            .get(pkg_str)
            .ok_or_else(|| Error::MissingPackage(pkg_str.to_string()))
    }

    /// Promotes workspace-nested packages to the hoisted position when the
    /// hoisted slot is either empty or occupied by a version that no
    /// workspace's transitive closure actually requested.
    ///
    /// Only rehoists when the original (unpruned) lockfile had an entry at
    /// the hoisted position. This preserves the install strategy: lockfiles
    /// produced with `install-strategy=shallow` never have hoisted entries
    /// for workspace dependencies, so we won't create them during pruning.
    /// See https://github.com/vercel/turborepo/issues/12493
    fn rehoist_packages(
        pruned: &mut HashMap<String, NpmPackage>,
        workspace_packages: &std::collections::HashSet<&str>,
        requested: &std::collections::HashSet<&str>,
        original_packages: &HashMap<String, NpmPackage>,
    ) {
        // Group workspace-nested entries by their target hoisted key. When
        // multiple workspaces each have their own nested copy of the same
        // package (common with install-strategy=shallow), promoting any one
        // of them would silently discard the others. Only rehoist when
        // exactly one workspace claims a given hoisted position.
        let mut candidates: HashMap<String, Vec<String>> = HashMap::new();

        for key in pruned.keys() {
            let Some(idx) = key.find("/node_modules/") else {
                continue;
            };
            let prefix = &key[..idx];
            if prefix.contains("node_modules/") || !workspace_packages.contains(prefix) {
                continue;
            }
            let pkg_name = &key[idx + "/node_modules/".len()..];
            if pkg_name.is_empty() || pkg_name.contains("/node_modules/") {
                continue;
            }
            let hoisted_key = format!("node_modules/{pkg_name}");

            // If the hoisted key was explicitly requested by a workspace's
            // transitive closure, another workspace genuinely needs that
            // version — don't replace it.
            if requested.contains(hoisted_key.as_str()) {
                continue;
            }

            // Only rehoist if the original lockfile had an entry at this
            // hoisted position. If the original never hoisted this package
            // (e.g. install-strategy=shallow), creating a hoisted entry
            // would break the lockfile structure.
            if !original_packages.contains_key(&hoisted_key) {
                continue;
            }

            candidates.entry(hoisted_key).or_default().push(key.clone());
        }

        let to_rehoist: Vec<(String, String)> = candidates
            .into_iter()
            .filter_map(|(hoisted_key, nested_keys)| {
                if nested_keys.len() == 1 {
                    Some((nested_keys.into_iter().next().unwrap(), hoisted_key))
                } else {
                    None
                }
            })
            .collect();

        for (nested_key, hoisted_key) in to_rehoist {
            // Remove old hoisted entry and its sub-deps.
            let old_prefix = format!("{hoisted_key}/");
            let old_sub: Vec<String> = pruned
                .keys()
                .filter(|k| k.starts_with(&old_prefix))
                .cloned()
                .collect();
            for k in old_sub {
                pruned.remove(&k);
            }
            pruned.remove(&hoisted_key);

            // Promote nested entry.
            if let Some(pkg) = pruned.remove(&nested_key) {
                pruned.insert(hoisted_key.clone(), pkg);
            }

            // Relocate sub-deps from nested path to hoisted path.
            let nested_prefix = format!("{nested_key}/");
            let new_prefix = format!("{hoisted_key}/");
            let sub_keys: Vec<String> = pruned
                .keys()
                .filter(|k| k.starts_with(&nested_prefix))
                .cloned()
                .collect();
            for sub_key in sub_keys {
                if let Some(pkg) = pruned.remove(&sub_key) {
                    let new_key = format!("{new_prefix}{}", &sub_key[nested_prefix.len()..]);
                    pruned.insert(new_key, pkg);
                }
            }
        }
    }

    /// Resolve a dependency name by walking up the node_modules hierarchy,
    /// checking each candidate key in the packages map. Uses `buf` to avoid
    /// allocating a new String for each candidate.
    fn find_dep_in_lockfile(
        &self,
        key: &str,
        dep: &str,
        buf: &mut String,
    ) -> Result<Option<(String, String)>, Error> {
        // First candidate: nested directly under the current package
        buf.clear();
        buf.reserve(key.len() + "/node_modules/".len() + dep.len());
        buf.push_str(key);
        buf.push_str("/node_modules/");
        buf.push_str(dep);
        if let Some(result) = self.check_package_entry(buf)? {
            return Ok(Some(result));
        }

        // Walk up the node_modules hierarchy
        let mut curr = Some(key);
        while let Some(k) = curr {
            let parent = Self::npm_path_parent(k);
            buf.clear();
            if let Some(p) = parent {
                buf.reserve(p.len() + "node_modules/".len() + dep.len());
                buf.push_str(p);
            } else {
                buf.reserve("node_modules/".len() + dep.len());
            }
            buf.push_str("node_modules/");
            buf.push_str(dep);

            if let Some(result) = self.check_package_entry(buf)? {
                return Ok(Some(result));
            }
            curr = parent;
        }

        Ok(None)
    }

    fn check_package_entry(&self, candidate_key: &str) -> Result<Option<(String, String)>, Error> {
        let Some(entry) = self.packages.get(candidate_key) else {
            return Ok(None);
        };
        match entry.version.as_deref() {
            Some(version) => Ok(Some((candidate_key.to_owned(), version.to_owned()))),
            None if entry.resolved.is_some() => Ok(None),
            None => Err(Error::MissingVersion(candidate_key.to_owned())),
        }
    }

    #[cfg(test)]
    fn possible_npm_deps(key: &str, dep: &str) -> Vec<String> {
        let mut possible_deps = vec![format!("{key}/node_modules/{dep}")];

        let mut curr = Some(key);
        while let Some(key) = curr {
            let next = Self::npm_path_parent(key);
            possible_deps.push(format!("{}node_modules/{}", next.unwrap_or(""), dep));
            curr = next;
        }

        possible_deps
    }

    fn npm_path_parent(key: &str) -> Option<&str> {
        key.rsplit_once("node_modules/")
            .map(|(first, _)| first)
            .and_then(|parent| {
                if parent.is_empty() {
                    None
                } else {
                    Some(parent)
                }
            })
    }
}

impl NpmPackage {
    pub fn dep_keys(&self) -> impl Iterator<Item = &String> {
        self.dependencies
            .keys()
            .chain(self.dev_dependencies.keys())
            .chain(self.optional_dependencies.keys())
            .chain(self.peer_dependencies.keys())
    }
}

pub fn npm_subgraph(
    contents: &[u8],
    workspace_packages: &[String],
    packages: &[String],
) -> Result<Vec<u8>, Error> {
    let lockfile = NpmLockfile::load(contents)?;
    let pruned_lockfile = lockfile.subgraph(workspace_packages, packages)?;
    let new_contents = pruned_lockfile.encode()?;

    Ok(new_contents)
}

pub fn npm_global_change(prev_contents: &[u8], curr_contents: &[u8]) -> Result<bool, Error> {
    let prev_lockfile = NpmLockfile::load(prev_contents)?;
    let curr_lockfile = NpmLockfile::load(curr_contents)?;

    Ok(
        prev_lockfile.lockfile_version != curr_lockfile.lockfile_version
            || prev_lockfile.other.get("requires") != curr_lockfile.other.get("requires"),
    )
}
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_npm_parent() {
        let tests = [
            ("apps/docs", None),
            ("apps/docs/node_modules/foo", Some("apps/docs/")),
            ("node_modules/foo", None),
            (
                "node_modules/foo/node_modules/bar",
                Some("node_modules/foo/"),
            ),
        ];

        for (key, parent) in &tests {
            assert_eq!(NpmLockfile::npm_path_parent(key), *parent);
        }
    }

    #[test]
    fn test_possible_npm_deps() {
        let tests = [
            (
                "node_modules/foo",
                "baz",
                vec!["node_modules/foo/node_modules/baz", "node_modules/baz"],
            ),
            (
                "node_modules/foo/node_modules/bar",
                "baz",
                vec![
                    "node_modules/foo/node_modules/bar/node_modules/baz",
                    "node_modules/foo/node_modules/baz",
                    "node_modules/baz",
                ],
            ),
            (
                "node_modules/foo1/node_modules/foo2/node_modules/foo3/node_modules/foo4",
                "bar",
                vec![
                    "node_modules/foo1/node_modules/foo2/node_modules/foo3/node_modules/foo4/\
                     node_modules/bar",
                    "node_modules/foo1/node_modules/foo2/node_modules/foo3/node_modules/bar",
                    "node_modules/foo1/node_modules/foo2/node_modules/bar",
                    "node_modules/foo1/node_modules/bar",
                    "node_modules/bar",
                ],
            ),
            (
                "apps/docs/node_modules/foo",
                "baz",
                vec![
                    "apps/docs/node_modules/foo/node_modules/baz",
                    "apps/docs/node_modules/baz",
                    "node_modules/baz",
                ],
            ),
        ];

        for (key, dep, expected) in &tests {
            assert_eq!(&NpmLockfile::possible_npm_deps(key, dep), expected);
        }
    }

    // Regression test for https://github.com/vercel/turborepo/issues/12139
    // When a workspace has deeply nested deps (e.g.
    // packages/pkg1/node_modules/parent/node_modules/child), rehoist_packages
    // must not double-process them. The parent entry's sub-dep relocation
    // already handles moving children; individually rehoisting a child would
    // delete the entry that was just relocated.
    //
    // The original lockfile includes a hoisted `node_modules/parent@1.0.0`
    // (used by a now-pruned workspace) alongside the nested v2 under pkg1.
    // After pruning, the hoisted v1 is no longer requested, so the nested v2
    // should be promoted to `node_modules/parent`.
    #[test]
    fn test_subgraph_preserves_deeply_nested_workspace_deps() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["packages/*"]
                },
                "node_modules/pkg1": {
                    "resolved": "packages/pkg1",
                    "link": true
                },
                "node_modules/parent": {
                    "version": "1.0.0"
                },
                "packages/pkg1": {
                    "version": "1.0.0",
                    "dependencies": {
                        "parent": "2.0.0"
                    }
                },
                "packages/pkg1/node_modules/parent": {
                    "version": "2.0.0",
                    "dependencies": {
                        "child-a": "^1.0.0",
                        "child-b": "^1.0.0"
                    }
                },
                "packages/pkg1/node_modules/parent/node_modules/child-a": {
                    "version": "1.0.0"
                },
                "packages/pkg1/node_modules/parent/node_modules/child-b": {
                    "version": "1.0.0",
                    "dependencies": {
                        "grandchild": "^1.0.0"
                    }
                },
                "packages/pkg1/node_modules/parent/node_modules/child-b/node_modules/grandchild": {
                    "version": "1.0.0"
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["packages/pkg1".to_string()];
        let packages = vec![
            "packages/pkg1/node_modules/parent".to_string(),
            "packages/pkg1/node_modules/parent/node_modules/child-a".to_string(),
            "packages/pkg1/node_modules/parent/node_modules/child-b".to_string(),
            "packages/pkg1/node_modules/parent/node_modules/child-b/node_modules/grandchild"
                .to_string(),
        ];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        // parent and all its nested children must survive rehoisting
        let expected_keys = [
            "node_modules/parent",
            "node_modules/parent/node_modules/child-a",
            "node_modules/parent/node_modules/child-b",
            "node_modules/parent/node_modules/child-b/node_modules/grandchild",
        ];
        for key in expected_keys {
            assert!(
                reparsed.packages.contains_key(key),
                "pruned lockfile is missing {key:?} — deeply nested deps were dropped"
            );
        }
    }

    // Regression test for https://github.com/vercel/turborepo/issues/12139
    // With install-strategy=shallow, each workspace has its own node_modules
    // with potentially different versions of the same package. rehoist_packages
    // must not collapse them into a single hoisted entry.
    #[test]
    fn test_subgraph_preserves_multiple_workspace_versions() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["apps/*", "packages/*"]
                },
                "node_modules/app-a": {
                    "resolved": "apps/app-a",
                    "link": true
                },
                "node_modules/pkg-b": {
                    "resolved": "packages/pkg-b",
                    "link": true
                },
                "apps/app-a": {
                    "version": "1.0.0",
                    "dependencies": {
                        "pkg-b": "*",
                        "chai": "^5.0.0"
                    }
                },
                "apps/app-a/node_modules/chai": {
                    "version": "5.3.3",
                    "dependencies": {
                        "deep-eql": "^5.0.0"
                    }
                },
                "apps/app-a/node_modules/chai/node_modules/deep-eql": {
                    "version": "5.0.2"
                },
                "packages/pkg-b": {
                    "version": "0.0.0",
                    "devDependencies": {
                        "chai": "^4.0.0"
                    }
                },
                "packages/pkg-b/node_modules/chai": {
                    "version": "4.5.0"
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["apps/app-a".to_string(), "packages/pkg-b".to_string()];
        let packages = vec![
            "apps/app-a/node_modules/chai".to_string(),
            "apps/app-a/node_modules/chai/node_modules/deep-eql".to_string(),
            "packages/pkg-b/node_modules/chai".to_string(),
        ];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        // Both workspace-nested versions must survive — neither should be
        // collapsed into node_modules/chai.
        assert!(
            reparsed
                .packages
                .contains_key("apps/app-a/node_modules/chai"),
            "app-a's chai was incorrectly rehoisted"
        );
        assert!(
            reparsed
                .packages
                .contains_key("packages/pkg-b/node_modules/chai"),
            "pkg-b's chai was incorrectly rehoisted"
        );
        assert!(
            reparsed
                .packages
                .contains_key("apps/app-a/node_modules/chai/node_modules/deep-eql"),
            "chai's sub-dep deep-eql was dropped"
        );
        // There should be no hoisted chai since both workspaces have their own
        assert!(
            !reparsed.packages.contains_key("node_modules/chai"),
            "a spurious hoisted node_modules/chai was created"
        );
    }

    // Regression test for https://github.com/vercel/turborepo/issues/12493
    //
    // With install-strategy=shallow, all of a workspace's dependencies live
    // under its own node_modules/ — there are no hoisted copies at the root.
    // When pruning down to a single workspace, rehoist_packages() must NOT
    // promote those nested entries to node_modules/ because that would create
    // entries that never existed in the original lockfile, breaking npm ci.
    #[test]
    fn test_subgraph_shallow_single_workspace_no_rehoist() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["apps/*", "packages/*"],
                    "devDependencies": {
                        "eslint": "9.0.0"
                    }
                },
                "node_modules/app-a": {
                    "resolved": "apps/app-a",
                    "link": true
                },
                "node_modules/eslint": {
                    "version": "9.0.0"
                },
                "apps/app-a": {
                    "version": "1.0.0",
                    "dependencies": {
                        "serverless": "^3.0.0"
                    }
                },
                "apps/app-a/node_modules/serverless": {
                    "version": "3.40.0",
                    "hasInstallScript": true,
                    "dependencies": {
                        "chalk": "^4.0.0"
                    }
                },
                "apps/app-a/node_modules/chalk": {
                    "version": "4.1.2"
                },
                "apps/app-a/node_modules/serverless/node_modules/json-colorizer": {
                    "version": "2.6.0"
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["apps/app-a".to_string()];
        let packages = vec![
            "apps/app-a/node_modules/serverless".to_string(),
            "apps/app-a/node_modules/chalk".to_string(),
            "apps/app-a/node_modules/serverless/node_modules/json-colorizer".to_string(),
            // Root devDep (from the root workspace's transitive closure)
            "node_modules/eslint".to_string(),
        ];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        // Workspace-nested entries must stay nested
        assert!(
            reparsed
                .packages
                .contains_key("apps/app-a/node_modules/serverless"),
            "serverless should remain under apps/app-a/node_modules/"
        );
        assert!(
            reparsed
                .packages
                .contains_key("apps/app-a/node_modules/chalk"),
            "chalk should remain under apps/app-a/node_modules/"
        );
        assert!(
            reparsed
                .packages
                .contains_key("apps/app-a/node_modules/serverless/node_modules/json-colorizer"),
            "nested sub-dep should remain in place"
        );

        // No hoisted copies should be created — these never existed in the
        // original lockfile (install-strategy=shallow).
        assert!(
            !reparsed.packages.contains_key("node_modules/serverless"),
            "serverless was wrongly hoisted to root"
        );
        assert!(
            !reparsed.packages.contains_key("node_modules/chalk"),
            "chalk was wrongly hoisted to root"
        );

        // Root devDependencies should still be present
        assert!(
            reparsed.packages.contains_key("node_modules/eslint"),
            "root devDep eslint should be preserved"
        );
    }

    #[test]
    fn test_turbo_version_rejects_non_semver() {
        // Malicious version strings that could be used for RCE via npx should be
        // rejected
        let malicious_versions = [
            "file:./malicious.tgz",
            "https://evil.com/malicious.tgz",
            "http://evil.com/malicious.tgz",
            "git+https://github.com/evil/repo.git",
            "git://github.com/evil/repo.git",
            "../../../etc/passwd",
            "1.0.0 && curl evil.com",
        ];

        for malicious_version in malicious_versions {
            let json = format!(
                r#"{{
                    "lockfileVersion": 3,
                    "packages": {{
                        "": {{}},
                        "node_modules/turbo": {{
                            "version": "{}"
                        }}
                    }}
                }}"#,
                malicious_version
            );
            let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();
            assert_eq!(
                lockfile.turbo_version(),
                None,
                "should reject malicious version: {}",
                malicious_version
            );
        }
    }
}
