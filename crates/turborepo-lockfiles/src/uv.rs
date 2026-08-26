//! Mechanical `uv.lock` filtering for `turbo prune`.
//!
//! uv owns workspace membership and dependency-graph semantics through
//! `uv workspace metadata`. This module does not resolve uv dependencies; it
//! only removes `[[package]]` tables that metadata determined are unreachable.

use std::collections::HashSet;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to parse uv.lock: {0}")]
    ParseDocument(#[from] Box<toml_edit::TomlError>),
    #[error("uv.lock is malformed: the [[package]] entries are not a valid array of tables.")]
    MalformedPackageArray,
    #[error("uv.lock package entry is missing a string name")]
    MissingPackageName,
}

/// The lockfile-visible identity of a package selected by uv metadata.
///
/// Multiple uv resolution nodes can share this identity when they are forked
/// only by source or markers. Pruning intentionally retains every matching
/// lockfile entry, which is conservative and leaves uv to interpret the forks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UvPackageKey {
    pub name: String,
    pub version: Option<String>,
}

/// The result of mechanically filtering a uv.lock.
#[derive(Debug)]
pub struct PrunedUvLock {
    /// Every retained workspace member, sorted.
    pub members: Vec<String>,
    /// The filtered lockfile, preserving retained package metadata verbatim.
    pub lockfile: String,
}

/// Filter a uv.lock to package identities and workspace members selected by
/// `uv workspace metadata`.
pub fn uv_prune_lock(
    contents: &str,
    kept_packages: &HashSet<UvPackageKey>,
    members: &HashSet<String>,
) -> Result<PrunedUvLock, Error> {
    let mut document: toml_edit::DocumentMut = contents.parse().map_err(Box::new)?;
    let packages = document
        .get_mut("package")
        .and_then(|item| item.as_array_of_tables_mut())
        .ok_or(Error::MalformedPackageArray)?;

    let mut malformed = false;
    packages.retain(|package| {
        let Some(name) = package.get("name").and_then(|item| item.as_str()) else {
            malformed = true;
            return false;
        };
        let version = package
            .get("version")
            .and_then(|item| item.as_str())
            .map(str::to_string);
        kept_packages.contains(&UvPackageKey {
            name: name.to_string(),
            version,
        })
    });
    if malformed {
        return Err(Error::MissingPackageName);
    }

    if let Some(manifest_members) = document
        .get_mut("manifest")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|manifest| manifest.get_mut("members"))
        .and_then(|item| item.as_array_mut())
    {
        manifest_members.retain(|entry| entry.as_str().is_some_and(|name| members.contains(name)));
    }

    let mut members = members.iter().cloned().collect::<Vec<_>>();
    members.sort();
    Ok(PrunedUvLock {
        members,
        lockfile: document.to_string(),
    })
}

#[cfg(test)]
mod test {
    use super::*;

    const LOCK: &str = r#"version = 1
revision = 3

[manifest]
members = ["app", "lib"]

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [{ name = "dep" }]

[package.metadata]
requires-dist = [{ name = "dep", specifier = ">=1" }]

[[package]]
name = "lib"
version = "0.1.0"
source = { virtual = "packages/lib" }

[[package]]
name = "dep"
version = "1.0.0"
source = { registry = "https://pypi.org/simple" }
wheels = [{ hash = "sha256:abc" }]

[[package]]
name = "unused"
version = "2.0.0"
source = { registry = "https://pypi.org/simple" }
"#;

    #[test]
    fn filters_packages_and_manifest_members() {
        let kept = HashSet::from([
            UvPackageKey {
                name: "app".to_string(),
                version: Some("0.1.0".to_string()),
            },
            UvPackageKey {
                name: "dep".to_string(),
                version: Some("1.0.0".to_string()),
            },
        ]);
        let members = HashSet::from(["app".to_string()]);
        let pruned = uv_prune_lock(LOCK, &kept, &members).unwrap();
        assert_eq!(pruned.members, vec!["app"]);
        assert!(pruned.lockfile.contains("name = \"app\""));
        assert!(pruned.lockfile.contains("[package.metadata]"));
        assert!(pruned.lockfile.contains("sha256:abc"));
        assert!(!pruned.lockfile.contains("name = \"lib\""));
        assert!(!pruned.lockfile.contains("name = \"unused\""));
        assert!(!pruned.lockfile.contains("\"lib\","));
    }

    #[test]
    fn keeps_all_matching_forks() {
        let lock = r#"version = 1
[[package]]
name = "fork"
version = "1.0.0"
source = { registry = "https://one.example" }
[[package]]
name = "fork"
version = "1.0.0"
source = { registry = "https://two.example" }
"#;
        let kept = HashSet::from([UvPackageKey {
            name: "fork".to_string(),
            version: Some("1.0.0".to_string()),
        }]);
        let pruned = uv_prune_lock(lock, &kept, &HashSet::new()).unwrap();
        assert!(pruned.lockfile.contains("https://one.example"));
        assert!(pruned.lockfile.contains("https://two.example"));
    }
}
