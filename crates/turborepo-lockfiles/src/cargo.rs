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
    #[error("Cargo.lock dependency '{0}' does not match any package entry.")]
    UnknownDependency(String),
    #[error(
        "Cargo.lock dependency '{0}' is ambiguous: multiple versions exist and no version was \
         specified."
    )]
    AmbiguousDependency(String),
}

#[derive(Deserialize)]
struct CargoLock {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Deserialize)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    #[serde(default)]
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

    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, package) in lock.package.iter().enumerate() {
        by_name.entry(package.name.as_str()).or_default().push(idx);
    }

    let resolve = |dep: &str| -> Result<usize, Error> {
        // "name", "name version", or "name version (source)".
        let mut parts = dep.split_whitespace();
        let name = parts.next().unwrap_or(dep);
        let version = parts.next();
        let candidates = by_name
            .get(name)
            .ok_or_else(|| Error::UnknownDependency(dep.to_string()))?;
        match version {
            Some(version) => candidates
                .iter()
                .copied()
                .find(|&idx| lock.package[idx].version == version)
                .ok_or_else(|| Error::UnknownDependency(dep.to_string())),
            None => match candidates.as_slice() {
                [only] => Ok(*only),
                _ => Err(Error::AmbiguousDependency(dep.to_string())),
            },
        }
    };

    let mut closures = HashMap::new();
    for member in members {
        // A member crate appears in the lock without a source. Duplicate
        // names cannot occur among members (Cargo rejects them), so a
        // sourceless entry with this name is the member.
        let Some(start) = by_name.get(member.as_str()).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|&idx| lock.package[idx].source.is_none())
        }) else {
            continue;
        };

        let mut externals = HashSet::new();
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            let package = &lock.package[idx];
            if package.source.is_some() {
                externals.insert(package.hash_identity());
            }
            for dep in &package.dependencies {
                stack.push(resolve(dep)?);
            }
        }
        closures.insert(member.clone(), externals);
    }
    Ok(closures)
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
