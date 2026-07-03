//! Cargo.lock parsing for external-dependency hashing.
//!
//! Unlike the JS lockfile implementations, this module does not implement
//! the [`crate::Lockfile`] trait: Cargo owns resolution, subgraph pruning,
//! and installation, so Turborepo only needs one thing from Cargo.lock —
//! the set of external packages in each workspace member's transitive
//! dependency closure, so a crate task's hash changes exactly when a
//! dependency in *its* closure changes (and not when an unrelated crate
//! bumps a dependency).
//!
//! Cargo.lock is a flat list of `[[package]]` entries. Workspace members
//! have no `source`; external packages (registry or git) do. A package's
//! `dependencies` are strings of the form `"name"`, `"name version"`, or
//! `"name version (source)"` — the longer forms appear only when the short
//! form would be ambiguous.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::Package;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to parse Cargo.lock: {0}")]
    Parse(#[from] Box<toml::de::Error>),
    #[error("Unable to serialize Cargo.lock: {0}")]
    Serialize(#[from] Box<toml::ser::Error>),
    #[error("Cargo.lock dependency '{0}' does not match any package entry.")]
    UnknownDependency(String),
    #[error(
        "Cargo.lock dependency '{0}' is ambiguous: multiple versions exist and no version was \
         specified."
    )]
    AmbiguousDependency(String),
    #[error(
        "Workspace member '{0}' not found in Cargo.lock; the lockfile is stale. Run `cargo \
         metadata` or a build to refresh it."
    )]
    MissingMember(String),
}

#[derive(Deserialize, serde::Serialize)]
struct CargoLock {
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<u32>,
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Deserialize, serde::Serialize, Clone)]
struct LockPackage {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<String>,
}

impl LockPackage {
    /// The hash identity of an external package: everything Cargo.lock pins
    /// about it. Version alone is insufficient — a git dependency's rev
    /// lives in `source`, and a registry package's content is pinned by
    /// `checksum`.
    fn hash_identity(&self) -> Package {
        let mut version = self.version.clone();
        if let Some(source) = &self.source {
            version.push(' ');
            version.push_str(source);
        }
        if let Some(checksum) = &self.checksum {
            version.push(' ');
            version.push_str(checksum);
        }
        Package {
            key: self.name.clone(),
            version,
        }
    }
}

/// For each named workspace member present in the lockfile, compute the set
/// of external packages (those with a `source`) in its transitive dependency
/// closure.
///
/// Members missing from the lockfile are simply absent from the result (a
/// stale lockfile is Cargo's to repair, not ours). Cargo.lock merges normal,
/// build, and dev dependencies into one edge list, so closures include dev
/// dependencies — over-inclusive, which is the safe direction for hashing.
pub fn cargo_external_closures(
    contents: &str,
    members: &[String],
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    let lock: CargoLock = toml::from_str(contents).map_err(Box::new)?;
    let index = LockIndex::new(&lock);

    let mut closures = HashMap::new();
    for member in members {
        let Some(start) = index.member(member) else {
            continue;
        };
        let mut externals = HashSet::new();
        for idx in index.reachable(start)? {
            let package = &lock.package[idx];
            if package.source.is_some() {
                externals.insert(package.hash_identity());
            }
        }
        closures.insert(member.clone(), externals);
    }
    Ok(closures)
}

/// The result of pruning a Cargo.lock to a set of root members.
#[derive(Debug)]
pub struct PrunedCargoLock {
    /// Every workspace member (sourceless lock entry) in the pruned closure,
    /// sorted. A superset of the roots: Cargo.lock merges dev/build
    /// dependency edges, so members reachable only through dev-dependencies
    /// (including cycle-participating ones absent from Turborepo's package
    /// graph closure) are retained — their manifests are referenced by kept
    /// crates and must exist in the pruned workspace.
    pub members: Vec<String>,
    /// The pruned lockfile contents: exactly the packages reachable from
    /// the roots, in the original entry order, so `cargo build --locked`
    /// succeeds in the pruned workspace.
    pub lockfile: String,
}

/// Prune a Cargo.lock to the transitive closure of the given root members.
///
/// Unlike [`cargo_external_closures`], a root missing from the lockfile is a
/// hard error: pruning against a stale lockfile would silently produce a
/// workspace that cannot build.
pub fn cargo_prune_lock(contents: &str, roots: &[String]) -> Result<PrunedCargoLock, Error> {
    let lock: CargoLock = toml::from_str(contents).map_err(Box::new)?;
    let index = LockIndex::new(&lock);

    let mut kept: HashSet<usize> = HashSet::new();
    for root in roots {
        let start = index
            .member(root)
            .ok_or_else(|| Error::MissingMember(root.clone()))?;
        kept.extend(index.reachable(start)?);
    }

    let mut members: Vec<String> = kept
        .iter()
        .filter(|&&idx| lock.package[idx].source.is_none())
        .map(|&idx| lock.package[idx].name.clone())
        .collect();
    members.sort();

    let pruned = CargoLock {
        version: lock.version,
        package: lock
            .package
            .iter()
            .enumerate()
            .filter(|(idx, _)| kept.contains(idx))
            .map(|(_, package)| package.clone())
            .collect(),
    };
    let mut lockfile = String::from(
        "# This file is automatically @generated by Cargo.\n# It is not intended for manual \
         editing.\n",
    );
    lockfile.push_str(&toml::to_string(&pruned).map_err(Box::new)?);

    Ok(PrunedCargoLock { members, lockfile })
}

/// Name-indexed view of a parsed lockfile with dependency-string resolution.
struct LockIndex<'a> {
    lock: &'a CargoLock,
    by_name: HashMap<&'a str, Vec<usize>>,
}

impl<'a> LockIndex<'a> {
    fn new(lock: &'a CargoLock) -> Self {
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (idx, package) in lock.package.iter().enumerate() {
            by_name.entry(package.name.as_str()).or_default().push(idx);
        }
        Self { lock, by_name }
    }

    /// A member crate appears in the lock without a source. Duplicate names
    /// cannot occur among members (Cargo rejects them), so a sourceless
    /// entry with this name is the member.
    fn member(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|&idx| self.lock.package[idx].source.is_none())
        })
    }

    /// Resolve a dependency string — `"name"`, `"name version"`, or
    /// `"name version (source)"` — to a package index.
    fn resolve(&self, dep: &str) -> Result<usize, Error> {
        let mut parts = dep.split_whitespace();
        let name = parts.next().unwrap_or(dep);
        let version = parts.next();
        let candidates = self
            .by_name
            .get(name)
            .ok_or_else(|| Error::UnknownDependency(dep.to_string()))?;
        match version {
            Some(version) => candidates
                .iter()
                .copied()
                .find(|&idx| self.lock.package[idx].version == version)
                .ok_or_else(|| Error::UnknownDependency(dep.to_string())),
            None => match candidates.as_slice() {
                [only] => Ok(*only),
                _ => Err(Error::AmbiguousDependency(dep.to_string())),
            },
        }
    }

    /// All package indices reachable from `start`, including itself.
    fn reachable(&self, start: usize) -> Result<HashSet<usize>, Error> {
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            for dep in &self.lock.package[idx].dependencies {
                stack.push(self.resolve(dep)?);
            }
        }
        Ok(visited)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const LOCK: &str = r#"
version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = ["lib-a", "serde"]

[[package]]
name = "lib-a"
version = "0.1.0"
dependencies = ["itoa 1.0.0"]

[[package]]
name = "other"
version = "0.1.0"
dependencies = ["itoa 0.4.8"]

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123"
dependencies = ["itoa 1.0.0"]

[[package]]
name = "itoa"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "def456"

[[package]]
name = "itoa"
version = "0.4.8"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0ld"

[[package]]
name = "git-dep"
version = "0.2.0"
source = "git+https://example.com/dep#deadbeef"
"#;

    fn names(closure: &HashSet<Package>) -> Vec<String> {
        let mut names: Vec<_> = closure
            .iter()
            .map(|p| format!("{}@{}", p.key, p.version))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_closures_are_per_member() {
        let members = vec!["app".to_string(), "lib-a".to_string(), "other".to_string()];
        let closures = cargo_external_closures(LOCK, &members).unwrap();

        // app's closure flows through lib-a and serde to itoa 1.0.0; the
        // identity captures version, source, and checksum.
        assert_eq!(
            names(&closures["app"]),
            vec![
                "itoa@1.0.0 registry+https://github.com/rust-lang/crates.io-index def456",
                "serde@1.0.200 registry+https://github.com/rust-lang/crates.io-index abc123",
            ]
        );
        // lib-a doesn't see serde; other pins the old itoa. A serde bump
        // must not invalidate either of them.
        assert_eq!(
            names(&closures["lib-a"]),
            vec!["itoa@1.0.0 registry+https://github.com/rust-lang/crates.io-index def456"]
        );
        assert_eq!(
            names(&closures["other"]),
            vec!["itoa@0.4.8 registry+https://github.com/rust-lang/crates.io-index 0ld"]
        );
    }

    #[test]
    fn test_prune_lock_keeps_reachable_closure() {
        let pruned = cargo_prune_lock(LOCK, &["lib-a".to_string()]).unwrap();
        // lib-a reaches only itoa 1.0.0; app/other/serde/git-dep are gone.
        assert_eq!(pruned.members, vec!["lib-a"]);
        assert!(pruned.lockfile.contains("name = \"lib-a\""));
        assert!(pruned.lockfile.contains("name = \"itoa\""));
        assert!(pruned.lockfile.contains("version = \"1.0.0\""));
        assert!(!pruned.lockfile.contains("name = \"app\""));
        assert!(!pruned.lockfile.contains("name = \"serde\""));
        assert!(!pruned.lockfile.contains("0.4.8"));
        // The lock version header survives, and the output re-parses.
        assert!(pruned.lockfile.contains("version = 4"));
        let closures = cargo_external_closures(&pruned.lockfile, &["lib-a".to_string()]).unwrap();
        assert_eq!(
            names(&closures["lib-a"]),
            vec!["itoa@1.0.0 registry+https://github.com/rust-lang/crates.io-index def456"]
        );
    }

    #[test]
    fn test_prune_lock_retains_dev_reachable_members() {
        // `app` depends on member `lib-a`; both stay members of the pruned
        // workspace even though only `app` was requested.
        let pruned = cargo_prune_lock(LOCK, &["app".to_string()]).unwrap();
        assert_eq!(pruned.members, vec!["app", "lib-a"]);
    }

    #[test]
    fn test_prune_lock_stale_root_errors() {
        let err = cargo_prune_lock(LOCK, &["not-in-lock".to_string()]).unwrap_err();
        assert!(matches!(err, Error::MissingMember(_)));
    }

    #[test]
    fn test_missing_member_is_skipped() {
        let members = vec!["not-in-lock".to_string()];
        let closures = cargo_external_closures(LOCK, &members).unwrap();
        assert!(closures.is_empty());
    }

    #[test]
    fn test_ambiguous_short_dependency_errors() {
        let lock = r#"
[[package]]
name = "app"
version = "0.1.0"
dependencies = ["itoa"]

[[package]]
name = "itoa"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "itoa"
version = "0.4.8"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let err = cargo_external_closures(lock, &["app".to_string()]).unwrap_err();
        assert!(matches!(err, Error::AmbiguousDependency(_)));
    }

    #[test]
    fn test_unknown_dependency_errors() {
        let lock = r#"
[[package]]
name = "app"
version = "0.1.0"
dependencies = ["ghost"]
"#;
        let err = cargo_external_closures(lock, &["app".to_string()]).unwrap_err();
        assert!(matches!(err, Error::UnknownDependency(_)));
    }

    #[test]
    fn test_git_source_participates_in_identity() {
        let lock = r#"
[[package]]
name = "app"
version = "0.1.0"
dependencies = ["git-dep"]

[[package]]
name = "git-dep"
version = "0.2.0"
source = "git+https://example.com/dep#deadbeef"
"#;
        let closures = cargo_external_closures(lock, &["app".to_string()]).unwrap();
        // A git rev bump changes `source` but not `version`; the identity
        // must capture it.
        assert_eq!(
            names(&closures["app"]),
            vec!["git-dep@0.2.0 git+https://example.com/dep#deadbeef"]
        );
    }
}
