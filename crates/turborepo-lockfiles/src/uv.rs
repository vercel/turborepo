//! uv.lock parsing for external-dependency hashing.
//!
//! Like the Cargo module (and unlike the JS lockfile implementations), this
//! module does not implement the [`crate::Lockfile`] trait: uv owns
//! resolution, subgraph pruning, and installation, so Turborepo only needs
//! two things from uv.lock — the set of external packages in each workspace
//! member's transitive dependency closure.
//!
//! uv.lock is a flat list of `[[package]]` entries. Workspace members carry
//! a local project source (`{ editable = "…" }`, or `{ virtual = "…" }` for
//! packages that are not built); external packages carry a distribution
//! source (`{ registry = "…" }`, `{ git = "…" }`, or `{ url = "…" }`).
//! Local `path`, `directory`, `editable`, and `virtual` sources must identify
//! workspace members; other local dependencies fail closed until Turborepo
//! can content-hash and copy them. A package's `dependencies` (and its
//! `optional-dependencies` / `dev-dependencies` groups) are tables of the
//! form `{ name = "…" }`, optionally qualified with `version` and `source`
//! when uv forked the resolution (e.g. per Python version or platform
//! marker) and the short form would be ambiguous.
//!
//! Package names in uv.lock are already PEP 503-normalized by uv; callers
//! must pass normalized member names.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

use crate::Package;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to parse uv.lock: {0}")]
    Parse(#[from] Box<toml::de::Error>),
    #[error(
        "uv.lock has unsupported schema version {version}. This version of Turborepo supports \
         uv.lock version 1; a newer Turborepo release may be required."
    )]
    UnsupportedLockVersion { version: i64 },
    #[error("uv.lock dependency '{0}' does not match any package entry.")]
    UnknownDependency(String),
    #[error(
        "Workspace member '{0}' not found in uv.lock; the lockfile is stale. Run `uv lock` and \
         commit the result."
    )]
    MissingMember(String),
    #[error(
        "uv.lock contains reachable local dependency {name:?} at {source_description:?}, but only \
         uv workspace members are supported. Add it to [tool.uv.workspace] or replace it with a \
         non-local source."
    )]
    UnsupportedLocalDependency {
        name: String,
        source_description: String,
    },
}

/// The uv.lock schema version this module understands. The `version` field
/// tracks breaking schema changes (`revision` tracks compatible ones), so an
/// unknown version is a hard error rather than a silent misparse.
const SUPPORTED_LOCK_VERSION: i64 = 1;

#[derive(Deserialize)]
struct UvLock {
    version: Option<i64>,
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Deserialize)]
struct LockPackage {
    name: String,
    version: Option<String>,
    source: Option<PackageSource>,
    #[serde(default)]
    dependencies: Vec<DependencyRef>,
    #[serde(default, rename = "optional-dependencies")]
    optional_dependencies: BTreeMap<String, Vec<DependencyRef>>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, Vec<DependencyRef>>,
    sdist: Option<Artifact>,
    #[serde(default)]
    wheels: Vec<Artifact>,
}

/// A package source table, kept shape-agnostic: the discriminant is the key
/// (`registry`, `git`, `url`, `path`, `editable`, `virtual`, `directory`),
/// and values are compared/serialized generically so schema-compatible
/// additions do not break parsing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
struct PackageSource(BTreeMap<String, toml::Value>);

impl PackageSource {
    /// Whether this source marks a local project (a workspace member or a
    /// local directory dependency) rather than a fetched distribution.
    fn is_local_project(&self) -> bool {
        ["editable", "virtual", "directory", "path"]
            .iter()
            .any(|key| self.0.contains_key(*key))
    }

    fn local_path(&self) -> Option<&str> {
        ["editable", "virtual", "directory", "path"]
            .iter()
            .find_map(|key| self.0.get(*key).and_then(toml::Value::as_str))
    }

    /// Canonical single-line form for hash identities: everything uv.lock
    /// pins about where a package comes from (registry URL, git URL + rev,
    /// direct URL, or local path).
    fn canonical(&self) -> String {
        self.0
            .iter()
            .map(|(key, value)| match value.as_str() {
                Some(value) => format!("{key}+{value}"),
                None => format!("{key}+{value}"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Deserialize)]
struct DependencyRef {
    name: String,
    version: Option<String>,
    source: Option<PackageSource>,
}

#[derive(Deserialize)]
struct Artifact {
    hash: Option<String>,
}

impl LockPackage {
    /// Whether this entry is an external package for hashing purposes: it is
    /// fetched from a registry, git, or URL. Local sources are validated as
    /// workspace members before being excluded from the external closure.
    fn is_external(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| !source.is_local_project())
    }

    /// The hash identity of an external package: everything uv.lock pins
    /// about it. Version alone is insufficient — a git dependency's rev
    /// lives in `source`, and distribution content is pinned by the sdist
    /// and wheel hashes.
    fn hash_identity(&self) -> Package {
        let mut version = self.version.clone().unwrap_or_default();
        if let Some(source) = &self.source {
            version.push(' ');
            version.push_str(&source.canonical());
        }
        let mut hashes: Vec<&str> = self
            .sdist
            .iter()
            .chain(&self.wheels)
            .filter_map(|artifact| artifact.hash.as_deref())
            .collect();
        hashes.sort_unstable();
        for hash in hashes {
            version.push(' ');
            version.push_str(hash);
        }
        Package {
            key: self.name.clone(),
            version,
        }
    }

    fn dependency_refs(&self) -> impl Iterator<Item = &DependencyRef> {
        self.dependencies
            .iter()
            .chain(self.optional_dependencies.values().flatten())
            .chain(self.dev_dependencies.values().flatten())
    }
}

fn parse_lock(contents: &str) -> Result<UvLock, Error> {
    let lock: UvLock = toml::from_str(contents).map_err(Box::new)?;
    let version = lock.version.unwrap_or(0);
    if version != SUPPORTED_LOCK_VERSION {
        return Err(Error::UnsupportedLockVersion { version });
    }
    Ok(lock)
}

/// For each named workspace member, compute the set of external packages in
/// its transitive dependency closure.
///
/// A missing member is a hard error: silently hashing an incomplete closure
/// would allow artifacts built from different dependency resolutions to
/// share a cache key. Optional-dependency extras and dev-dependency groups
/// fold into the same edge set as regular dependencies — over-inclusive,
/// which is the safe direction for hashing. Member names must be PEP
/// 503-normalized (uv.lock entries already are).
pub fn uv_external_closures(
    contents: &str,
    members: &[String],
    workspace_paths: &HashMap<String, String>,
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    let lock = parse_lock(contents)?;
    let index = LockIndex::new(&lock)?;

    let mut starts = Vec::with_capacity(members.len());
    for member in members {
        starts.push(
            index
                .member(member, workspace_paths)
                .ok_or_else(|| Error::MissingMember(member.clone()))?,
        );
    }
    let (external_indices, reachable) = index.external_closures(&starts);
    for idx in reachable {
        reject_non_workspace_local(&lock.package[idx], workspace_paths)?;
    }
    let closures = members
        .iter()
        .zip(external_indices)
        .map(|(member, indices)| {
            let externals = indices
                .into_iter()
                .map(|idx| lock.package[idx].hash_identity())
                .collect();
            (member.clone(), externals)
        })
        .collect();
    Ok(closures)
}

fn reject_non_workspace_local(
    package: &LockPackage,
    workspace_paths: &HashMap<String, String>,
) -> Result<(), Error> {
    let Some(source) = package.source.as_ref() else {
        return Ok(());
    };
    let matches_workspace = package_matches_workspace(package, workspace_paths);
    if source.is_local_project() && !matches_workspace {
        return Err(Error::UnsupportedLocalDependency {
            name: package.name.clone(),
            source_description: source.canonical(),
        });
    }
    Ok(())
}

fn package_matches_workspace(
    package: &LockPackage,
    workspace_paths: &HashMap<String, String>,
) -> bool {
    package
        .source
        .as_ref()
        .and_then(PackageSource::local_path)
        .zip(workspace_paths.get(&package.name))
        .is_some_and(|(actual, expected)| {
            normalize_local_path(actual) == normalize_local_path(expected)
        })
}

fn normalize_local_path(path: &str) -> String {
    #[cfg(windows)]
    let path = path.replace('\\', "/").to_ascii_lowercase();
    #[cfg(not(windows))]
    let path = path.to_string();
    path.trim_start_matches("./")
        .trim_end_matches('/')
        .to_string()
}

/// Index over a parsed lockfile for dependency-reference resolution.
struct LockIndex<'a> {
    lock: &'a UvLock,
    by_name: HashMap<&'a str, Vec<usize>>,
    adjacency: Vec<Vec<usize>>,
}

impl<'a> LockIndex<'a> {
    fn new(lock: &'a UvLock) -> Result<Self, Error> {
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (idx, package) in lock.package.iter().enumerate() {
            by_name.entry(package.name.as_str()).or_default().push(idx);
        }
        let mut index = Self {
            lock,
            by_name,
            adjacency: vec![Vec::new(); lock.package.len()],
        };
        for (idx, package) in lock.package.iter().enumerate() {
            let mut dependencies = Vec::new();
            for dependency in package.dependency_refs() {
                dependencies.extend(index.resolve(dependency)?);
            }
            dependencies.sort_unstable();
            dependencies.dedup();
            index.adjacency[idx] = dependencies;
        }
        Ok(index)
    }

    /// A workspace member appears in the lock with a local project source.
    /// Member names are unique within a uv workspace, so a local-project
    /// entry with this name is the member.
    fn member(&self, name: &str, workspace_paths: &HashMap<String, String>) -> Option<usize> {
        self.by_name.get(name).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|&idx| package_matches_workspace(&self.lock.package[idx], workspace_paths))
        })
    }

    /// External package indices reachable from each requested root, plus the
    /// union of all reachable packages. Root bitsets flow once through the SCC
    /// DAG, avoiding both repeated graph walks and a closure set per node.
    fn external_closures(&self, roots: &[usize]) -> (Vec<HashSet<usize>>, HashSet<usize>) {
        let sccs = crate::closure_dp::tarjan_scc(
            &self
                .adjacency
                .iter()
                .map(|edges| edges.iter().map(|&idx| idx as u32).collect())
                .collect::<Vec<Vec<u32>>>(),
        );
        let mut scc_edges = vec![HashSet::new(); sccs.components.len()];
        for (node, edges) in self.adjacency.iter().enumerate() {
            let from = sccs.scc_of[node] as usize;
            for &target in edges {
                let to = sccs.scc_of[target] as usize;
                if from != to {
                    scc_edges[from].insert(to);
                }
            }
        }

        let words = roots.len().div_ceil(64);
        let mut reachable_by = vec![0u64; sccs.components.len() * words];
        for (root_idx, &root) in roots.iter().enumerate() {
            let scc = sccs.scc_of[root] as usize;
            reachable_by[scc * words + root_idx / 64] |= 1 << (root_idx % 64);
        }
        // Tarjan emits dependencies first. Walk backwards so each root's bits
        // reach its dependency SCCs after all of its dependents are known.
        for scc_idx in (0..sccs.components.len()).rev() {
            let row = reachable_by[scc_idx * words..][..words].to_vec();
            for &successor in &scc_edges[scc_idx] {
                let successor = &mut reachable_by[successor * words..][..words];
                for (target, source) in successor.iter_mut().zip(&row) {
                    *target |= source;
                }
            }
        }

        let mut externals = vec![HashSet::new(); roots.len()];
        let mut reachable = HashSet::new();
        for (scc_idx, members) in sccs.components.iter().enumerate() {
            let row = &reachable_by[scc_idx * words..][..words];
            if row.iter().all(|word| *word == 0) {
                continue;
            }
            for &member in members {
                let member = member as usize;
                reachable.insert(member);
                if !self.lock.package[member].is_external() {
                    continue;
                }
                for (word_idx, &word) in row.iter().enumerate() {
                    let mut word = word;
                    while word != 0 {
                        let bit = word.trailing_zeros() as usize;
                        word &= word - 1;
                        externals[word_idx * 64 + bit].insert(member);
                    }
                }
            }
        }
        (externals, reachable)
    }

    /// Resolve a dependency reference to the package indices it can denote.
    ///
    /// References are qualified with `version`/`source` exactly when uv
    /// forked the resolution (e.g. by Python version or platform marker), so
    /// after filtering, several candidates can legitimately remain — any
    /// environment may select either fork. All of them are returned:
    /// over-inclusion is the safe direction for both hashing and pruning,
    /// so ambiguity is not an error here (unlike Cargo's resolver, which
    /// guarantees a unique match).
    fn resolve(&self, dependency: &DependencyRef) -> Result<Vec<usize>, Error> {
        let display = |dependency: &DependencyRef| {
            let mut display = dependency.name.clone();
            if let Some(version) = &dependency.version {
                display.push(' ');
                display.push_str(version);
            }
            display
        };
        let candidates: Vec<usize> =
            self.by_name
                .get(dependency.name.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .copied()
                .filter(|&idx| {
                    dependency.version.as_ref().is_none_or(|version| {
                        self.lock.package[idx].version.as_ref() == Some(version)
                    })
                })
                .filter(|&idx| {
                    dependency
                        .source
                        .as_ref()
                        .is_none_or(|source| self.lock.package[idx].source.as_ref() == Some(source))
                })
                .collect();
        if candidates.is_empty() {
            return Err(Error::UnknownDependency(display(dependency)));
        }
        Ok(candidates)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Shaped like real `uv lock` output (version 1): editable members, a
    /// virtual root, registry packages with artifact hashes, dev/optional
    /// groups, and a marker-forked package resolved at two versions.
    const LOCK: &str = r#"
version = 1
revision = 3
requires-python = ">=3.11"

[manifest]
members = [
    "py-app",
    "py-lib",
    "root-project",
]

[[package]]
name = "click"
version = "8.4.2"
source = { registry = "https://pypi.org/simple" }
dependencies = [
    { name = "colorama", marker = "sys_platform == 'win32'" },
]
sdist = { url = "https://example.com/click-8.4.2.tar.gz", hash = "sha256:c11c4", size = 338000 }
wheels = [
    { url = "https://example.com/click-8.4.2-py3-none-any.whl", hash = "sha256:c11c4wheel", size = 119243 },
]

[[package]]
name = "colorama"
version = "0.4.6"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/colorama-0.4.6.tar.gz", hash = "sha256:c0104", size = 27697 }

[[package]]
name = "py-app"
version = "0.1.0"
source = { editable = "packages/py-app" }
dependencies = [
    { name = "click" },
    { name = "py-lib" },
]

[package.optional-dependencies]
color = [
    { name = "colorama" },
]

[package.dev-dependencies]
dev = [
    { name = "pytest" },
]

[package.metadata]
requires-dist = [
    { name = "click", specifier = ">=8.1" },
    { name = "colorama", marker = "extra == 'color'", specifier = ">=0.4" },
    { name = "py-lib", editable = "packages/py-lib" },
]

[[package]]
name = "py-lib"
version = "0.1.0"
source = { editable = "packages/py-lib" }
dependencies = [
    { name = "six" },
]

[[package]]
name = "pytest"
version = "9.1.1"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/pytest-9.1.1.tar.gz", hash = "sha256:9e51", size = 1636369 }

[[package]]
name = "root-project"
version = "0.1.0"
source = { virtual = "." }
dependencies = [
    { name = "py-app" },
]

[package.dev-dependencies]
dev = [
    { name = "ruff" },
]

[[package]]
name = "ruff"
version = "0.16.0"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/ruff-0.16.0.tar.gz", hash = "sha256:0uff", size = 4790557 }

[[package]]
name = "six"
version = "1.17.0"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/six-1.17.0.tar.gz", hash = "sha256:51x", size = 34031 }
"#;

    const FORKED_LOCK: &str = r#"
version = 1
revision = 3
requires-python = ">=3.10"
resolution-markers = [
    "python_full_version >= '3.12'",
    "python_full_version < '3.12'",
]

[manifest]
members = [
    "fork-lib",
]

[[package]]
name = "fork-lib"
version = "0.1.0"
source = { editable = "packages/fork-lib" }
dependencies = [
    { name = "typing-extensions", version = "4.8.0", source = { registry = "https://pypi.org/simple" }, marker = "python_full_version < '3.12'" },
    { name = "typing-extensions", version = "4.16.0", source = { registry = "https://pypi.org/simple" }, marker = "python_full_version >= '3.12'" },
]

[[package]]
name = "typing-extensions"
version = "4.8.0"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/typing_extensions-4.8.0.tar.gz", hash = "sha256:01d", size = 71456 }

[[package]]
name = "typing-extensions"
version = "4.16.0"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/typing_extensions-4.16.0.tar.gz", hash = "sha256:new", size = 113555 }
"#;

    fn names(closure: &HashSet<Package>) -> Vec<String> {
        let mut names: Vec<_> = closure
            .iter()
            .map(|package| format!("{}@{}", package.key, package.version))
            .collect();
        names.sort();
        names
    }

    fn workspace_paths(contents: &str) -> HashMap<String, String> {
        parse_lock(contents)
            .unwrap()
            .package
            .into_iter()
            .filter_map(|package| {
                let path = package.source?.local_path()?.to_string();
                Some((package.name, path))
            })
            .collect()
    }

    #[test]
    fn test_closures_are_per_member() {
        let members = vec!["py-app".to_string(), "py-lib".to_string()];
        let closures = uv_external_closures(LOCK, &members, &workspace_paths(LOCK)).unwrap();

        // py-app's closure folds in regular deps, the `color` extra, and the
        // `dev` group; identities capture version, source, and hashes.
        assert_eq!(
            names(&closures["py-app"]),
            vec![
                "click@8.4.2 registry+https://pypi.org/simple sha256:c11c4 sha256:c11c4wheel",
                "colorama@0.4.6 registry+https://pypi.org/simple sha256:c0104",
                "pytest@9.1.1 registry+https://pypi.org/simple sha256:9e51",
                "six@1.17.0 registry+https://pypi.org/simple sha256:51x",
            ]
        );
        // py-lib doesn't see click/pytest. A click bump must not invalidate
        // it.
        assert_eq!(
            names(&closures["py-lib"]),
            vec!["six@1.17.0 registry+https://pypi.org/simple sha256:51x"]
        );
    }

    #[test]
    fn test_virtual_root_is_a_member() {
        let closures =
            uv_external_closures(LOCK, &["root-project".to_string()], &workspace_paths(LOCK))
                .unwrap();
        // The virtual root reaches everything py-app reaches, plus its own
        // dev group (ruff). Members never appear as externals.
        assert_eq!(
            names(&closures["root-project"]),
            vec![
                "click@8.4.2 registry+https://pypi.org/simple sha256:c11c4 sha256:c11c4wheel",
                "colorama@0.4.6 registry+https://pypi.org/simple sha256:c0104",
                "pytest@9.1.1 registry+https://pypi.org/simple sha256:9e51",
                "ruff@0.16.0 registry+https://pypi.org/simple sha256:0uff",
                "six@1.17.0 registry+https://pypi.org/simple sha256:51x",
            ]
        );
    }

    #[test]
    fn test_forked_resolution_includes_both_forks() {
        let closures = uv_external_closures(
            FORKED_LOCK,
            &["fork-lib".to_string()],
            &workspace_paths(FORKED_LOCK),
        )
        .unwrap();
        // Marker-forked references resolve each qualified candidate exactly.
        assert_eq!(
            names(&closures["fork-lib"]),
            vec![
                "typing-extensions@4.16.0 registry+https://pypi.org/simple sha256:new",
                "typing-extensions@4.8.0 registry+https://pypi.org/simple sha256:01d",
            ]
        );
    }

    #[test]
    fn test_unqualified_forked_reference_is_over_inclusive() {
        let lock = r#"
version = 1

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "shared" },
]

[[package]]
name = "shared"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "shared"
version = "2.0.0"
source = { registry = "https://pypi.org/simple" }
"#;
        // Unlike Cargo, an unqualified reference matching several forks is
        // not an error: any environment may use either fork, so both belong
        // in the closure.
        let closures =
            uv_external_closures(lock, &["app".to_string()], &workspace_paths(lock)).unwrap();
        assert_eq!(
            names(&closures["app"]),
            vec![
                "shared@1.0.0 registry+https://pypi.org/simple",
                "shared@2.0.0 registry+https://pypi.org/simple",
            ]
        );
    }

    #[test]
    fn test_missing_member_errors() {
        let error =
            uv_external_closures(LOCK, &["not-in-lock".to_string()], &workspace_paths(LOCK))
                .unwrap_err();
        assert!(matches!(error, Error::MissingMember(_)));
    }

    #[test]
    fn test_unknown_dependency_errors() {
        let lock = r#"
version = 1

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "ghost" },
]
"#;
        let error =
            uv_external_closures(lock, &["app".to_string()], &workspace_paths(lock)).unwrap_err();
        assert!(matches!(error, Error::UnknownDependency(_)));
    }

    #[test]
    fn test_unsupported_lock_version_errors() {
        let error = uv_external_closures("version = 2\n", &[], &HashMap::new()).unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedLockVersion { version: 2 }
        ));
        let error = uv_external_closures("", &[], &HashMap::new()).unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedLockVersion { version: 0 }
        ));
    }

    #[test]
    fn test_git_source_participates_in_identity() {
        let lock = r#"
version = 1

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "git-dep" },
]

[[package]]
name = "git-dep"
version = "0.2.0"
source = { git = "https://example.com/dep?rev=main#deadbeef" }
"#;
        let closures =
            uv_external_closures(lock, &["app".to_string()], &workspace_paths(lock)).unwrap();
        // A git rev bump changes `source` but not `version`; the identity
        // must capture it.
        assert_eq!(
            names(&closures["app"]),
            vec!["git-dep@0.2.0 git+https://example.com/dep?rev=main#deadbeef"]
        );
    }

    #[test]
    fn test_non_workspace_local_dependency_errors() {
        let lock = r#"
version = 1

[manifest]
members = ["app"]

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [{ name = "local-lib" }]

[[package]]
name = "local-lib"
version = "0.1.0"
source = { directory = "vendor/local-lib" }
"#;
        let members = ["app".to_string()];
        let allowed = HashMap::from([("app".to_string(), "packages/app".to_string())]);
        let error = uv_external_closures(lock, &members, &allowed).unwrap_err();
        assert!(matches!(error, Error::UnsupportedLocalDependency { .. }));
    }

    #[test]
    fn test_same_name_local_fork_must_match_workspace_path() {
        let lock = r#"
version = 1

[[package]]
name = "app"
source = { editable = "packages/app" }
dependencies = [
    { name = "shared", source = { directory = "vendor/shared" } },
]

[[package]]
name = "shared"
source = { editable = "packages/shared" }

[[package]]
name = "shared"
source = { directory = "vendor/shared" }
"#;
        let paths = HashMap::from([
            ("app".to_string(), "packages/app".to_string()),
            ("shared".to_string(), "packages/shared".to_string()),
        ]);
        let error = uv_external_closures(lock, &["app".to_string()], &paths).unwrap_err();
        assert!(matches!(error, Error::UnsupportedLocalDependency { .. }));
    }

    #[test]
    fn test_member_selection_uses_workspace_path() {
        let lock = r#"
version = 1

[[package]]
name = "app"
source = { directory = "vendor/app" }

[[package]]
name = "app"
source = { editable = "packages/app" }
dependencies = [{ name = "dep" }]

[[package]]
name = "dep"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
"#;
        let paths = HashMap::from([("app".to_string(), "packages/app".to_string())]);
        let closures = uv_external_closures(lock, &["app".to_string()], &paths).unwrap();
        assert_eq!(
            names(&closures["app"]),
            vec!["dep@1.0.0 registry+https://pypi.org/simple"]
        );
    }
}
