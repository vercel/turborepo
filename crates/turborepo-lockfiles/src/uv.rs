//! uv.lock parsing for external-dependency hashing and pruning.
//!
//! Like the Cargo module (and unlike the JS lockfile implementations), this
//! module does not implement the [`crate::Lockfile`] trait: uv owns
//! resolution, environment syncing, and installation, so Turborepo only
//! needs two things from uv.lock — the set of external packages in each
//! workspace member's transitive dependency closure (so a member task's hash
//! changes exactly when a dependency in *its* closure changes), and a
//! reachability-based subset for `turbo prune`.
//!
//! uv.lock is a flat list of `[[package]]` entries. Workspace members have
//! an `editable` or `virtual` source (a directory inside the workspace);
//! external packages have a `registry`, `git`, `url`, `path`, or `directory`
//! source. A package's `dependencies` (and `optional-dependencies` /
//! `dev-dependencies` groups) reference other entries by name, with
//! `version`/`source` added only when the name alone would be ambiguous
//! (uv can resolve multiple versions of one package across disjoint
//! environment markers).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

use crate::Package;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unable to parse uv.lock: {0}")]
    Parse(#[from] Box<toml::de::Error>),
    #[error("Unable to rewrite uv.lock: {0}")]
    Edit(#[from] Box<toml_edit::TomlError>),
    #[error("uv.lock dependency '{0}' does not match any package entry.")]
    UnknownDependency(String),
    #[error(
        "Workspace member '{0}' not found in uv.lock; the lockfile is stale. Run `uv lock` to \
         refresh it."
    )]
    MissingMember(String),
}

#[derive(Deserialize)]
struct UvLock {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Deserialize)]
struct LockPackage {
    name: String,
    #[serde(default)]
    version: Option<String>,
    /// The package's provenance: `{ registry = ... }`, `{ git = ... }`,
    /// `{ editable = ... }`, etc. Kept as a raw map so identity hashing and
    /// member detection survive source shapes we don't know about yet.
    #[serde(default)]
    source: BTreeMap<String, toml::Value>,
    #[serde(default)]
    dependencies: Vec<LockDependency>,
    #[serde(default, rename = "optional-dependencies")]
    optional_dependencies: BTreeMap<String, Vec<LockDependency>>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, Vec<LockDependency>>,
    #[serde(default)]
    sdist: Option<LockArtifact>,
    #[serde(default)]
    wheels: Vec<LockArtifact>,
}

#[derive(Deserialize)]
struct LockDependency {
    name: String,
    /// Present only when multiple resolutions of `name` exist and the name
    /// alone is ambiguous.
    #[serde(default)]
    version: Option<String>,
}

#[derive(Deserialize)]
struct LockArtifact {
    #[serde(default)]
    hash: Option<String>,
}

impl LockPackage {
    /// Whether this entry is a workspace member: uv records members with an
    /// `editable` or `virtual` source pointing inside the workspace.
    fn is_member(&self) -> bool {
        self.source.contains_key("editable") || self.source.contains_key("virtual")
    }

    fn dependency_lists(&self) -> impl Iterator<Item = &LockDependency> {
        self.dependencies
            .iter()
            .chain(self.optional_dependencies.values().flatten())
            .chain(self.dev_dependencies.values().flatten())
    }

    /// The hash identity of an external package: everything uv.lock pins
    /// about it. Version alone is insufficient — a git dependency's rev
    /// lives in `source`, and a registry package's content is pinned by its
    /// sdist/wheel hashes.
    fn hash_identity(&self) -> Package {
        let mut version = self.version.clone().unwrap_or_default();
        for (key, value) in &self.source {
            version.push(' ');
            version.push_str(key);
            version.push('+');
            // TOML string values render quoted via Display; strip the quotes
            // for strings, and fall back to the TOML rendering otherwise.
            match value.as_str() {
                Some(s) => version.push_str(s),
                None => version.push_str(&value.to_string()),
            }
        }
        if let Some(hash) = self.sdist.as_ref().and_then(|s| s.hash.as_deref()) {
            version.push(' ');
            version.push_str(hash);
        } else {
            let mut hashes: Vec<&str> = self
                .wheels
                .iter()
                .filter_map(|w| w.hash.as_deref())
                .collect();
            hashes.sort_unstable();
            for hash in hashes {
                version.push(' ');
                version.push_str(hash);
            }
        }
        Package {
            key: self.name.clone(),
            version,
        }
    }
}

/// For each named workspace member present in the lockfile, compute the set
/// of external packages (non-member sources) in its transitive dependency
/// closure.
///
/// Members missing from the lockfile are simply absent from the result (a
/// stale lockfile is uv's to repair, not ours). Optional-dependency extras
/// and dev-dependency groups are merged into the closure — over-inclusive,
/// which is the safe direction for hashing.
pub fn uv_external_closures(
    contents: &str,
    members: &[String],
) -> Result<HashMap<String, HashSet<Package>>, Error> {
    let lock: UvLock = toml::from_str(contents).map_err(Box::new)?;
    let index = LockIndex::new(&lock);

    let mut closures = HashMap::new();
    for member in members {
        let Some(start) = index.member(member) else {
            continue;
        };
        let mut externals = HashSet::new();
        for idx in index.reachable(start)? {
            let package = &lock.package[idx];
            if !package.is_member() {
                externals.insert(package.hash_identity());
            }
        }
        closures.insert(member.clone(), externals);
    }
    Ok(closures)
}

/// The result of pruning a uv.lock to a set of root members.
#[derive(Debug)]
pub struct PrunedUvLock {
    /// Every workspace member in the pruned closure, sorted. A superset of
    /// the roots: members reachable through optional or dev-dependency
    /// groups are retained — the lockfile references their directories, so
    /// they must exist in the pruned workspace.
    pub members: Vec<String>,
    /// The pruned lockfile contents: exactly the packages reachable from
    /// the roots, in the original entry order and formatting, with the
    /// `[manifest].members` list rewritten to the kept members.
    pub lockfile: String,
}

/// Prune a uv.lock to the transitive closure of the given root members.
///
/// Unlike [`uv_external_closures`], a root missing from the lockfile is a
/// hard error: pruning against a stale lockfile would silently produce a
/// workspace that cannot sync.
pub fn uv_prune_lock(contents: &str, roots: &[String]) -> Result<PrunedUvLock, Error> {
    let lock: UvLock = toml::from_str(contents).map_err(Box::new)?;
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
        .filter(|&&idx| lock.package[idx].is_member())
        .map(|&idx| lock.package[idx].name.clone())
        .collect();
    members.sort();

    // Rewrite via toml_edit so untouched entries keep uv's formatting:
    // uv validates its own lockfile aggressively, and gratuitous format
    // churn would make every pruned lock diff noisy.
    let mut doc: toml_edit::DocumentMut = contents.parse().map_err(Box::new)?;
    if let Some(packages) = doc
        .get_mut("package")
        .and_then(|item| item.as_array_of_tables_mut())
    {
        let mut idx = 0;
        packages.retain(|_| {
            let keep = kept.contains(&idx);
            idx += 1;
            keep
        });
    }
    if let Some(manifest_members) = doc
        .get_mut("manifest")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|manifest| manifest.get_mut("members"))
    {
        let mut array = toml_edit::Array::new();
        for member in &members {
            array.push(member.as_str());
        }
        *manifest_members = toml_edit::value(array);
    }

    Ok(PrunedUvLock {
        members,
        lockfile: doc.to_string(),
    })
}

/// Name-indexed view of a parsed lockfile with dependency resolution.
struct LockIndex<'a> {
    lock: &'a UvLock,
    by_name: HashMap<&'a str, Vec<usize>>,
}

impl<'a> LockIndex<'a> {
    fn new(lock: &'a UvLock) -> Self {
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (idx, package) in lock.package.iter().enumerate() {
            by_name.entry(package.name.as_str()).or_default().push(idx);
        }
        Self { lock, by_name }
    }

    /// A member appears in the lock with an editable/virtual source.
    /// Duplicate names cannot occur among members (uv rejects them), so an
    /// entry with a member source and this name is the member.
    fn member(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|&idx| self.lock.package[idx].is_member())
        })
    }

    /// Resolve a dependency reference to package indices. When uv resolves
    /// multiple versions of a package across environment markers, a
    /// reference without a version legitimately matches several entries;
    /// all of them are returned (over-inclusive, the safe direction for
    /// both hashing and pruning).
    fn resolve(&self, dep: &LockDependency) -> Result<Vec<usize>, Error> {
        let candidates = self
            .by_name
            .get(dep.name.as_str())
            .ok_or_else(|| Error::UnknownDependency(dep.name.clone()))?;
        let matches: Vec<usize> = match &dep.version {
            Some(version) => candidates
                .iter()
                .copied()
                .filter(|&idx| self.lock.package[idx].version.as_deref() == Some(version.as_str()))
                .collect(),
            None => candidates.clone(),
        };
        if matches.is_empty() {
            return Err(Error::UnknownDependency(dep.name.clone()));
        }
        Ok(matches)
    }

    /// All package indices reachable from `start`, including itself.
    fn reachable(&self, start: usize) -> Result<HashSet<usize>, Error> {
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if !visited.insert(idx) {
                continue;
            }
            for dep in self.lock.package[idx].dependency_lists() {
                stack.extend(self.resolve(dep)?);
            }
        }
        Ok(visited)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const LOCK: &str = r#"version = 1
revision = 3
requires-python = ">=3.12"

[manifest]
members = [
    "app",
    "lib-a",
    "virt",
]

[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "lib-a" },
    { name = "requests" },
]

[package.dev-dependencies]
dev = [
    { name = "virt" },
]

[[package]]
name = "certifi"
version = "2026.6.17"
source = { registry = "https://pypi.org/simple" }
sdist = { url = "https://example.com/certifi.tar.gz", hash = "sha256:certifi-sdist" }

[[package]]
name = "idna"
version = "3.18"
source = { registry = "https://pypi.org/simple" }
wheels = [
    { url = "https://example.com/idna.whl", hash = "sha256:idna-wheel" },
]

[[package]]
name = "lib-a"
version = "0.1.0"
source = { editable = "packages/lib-a" }
dependencies = [
    { name = "idna" },
]

[[package]]
name = "requests"
version = "2.34.2"
source = { registry = "https://pypi.org/simple" }
dependencies = [
    { name = "certifi" },
]
sdist = { url = "https://example.com/requests.tar.gz", hash = "sha256:requests-sdist" }

[[package]]
name = "virt"
version = "0.1.0"
source = { virtual = "packages/virt" }
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
        let members = vec!["app".to_string(), "lib-a".to_string(), "virt".to_string()];
        let closures = uv_external_closures(LOCK, &members).unwrap();

        // app's closure flows through lib-a, requests, and its dev group;
        // the identity captures version, source, and artifact hashes.
        assert_eq!(
            names(&closures["app"]),
            vec![
                "certifi@2026.6.17 registry+https://pypi.org/simple sha256:certifi-sdist",
                "idna@3.18 registry+https://pypi.org/simple sha256:idna-wheel",
                "requests@2.34.2 registry+https://pypi.org/simple sha256:requests-sdist",
            ]
        );
        // lib-a doesn't see requests; a requests bump must not invalidate it.
        assert_eq!(
            names(&closures["lib-a"]),
            vec!["idna@3.18 registry+https://pypi.org/simple sha256:idna-wheel"]
        );
        assert!(closures["virt"].is_empty());
    }

    #[test]
    fn test_prune_lock_keeps_reachable_closure() {
        let pruned = uv_prune_lock(LOCK, &["lib-a".to_string()]).unwrap();
        assert_eq!(pruned.members, vec!["lib-a"]);
        assert!(pruned.lockfile.contains("name = \"lib-a\""));
        assert!(pruned.lockfile.contains("name = \"idna\""));
        assert!(!pruned.lockfile.contains("name = \"app\""));
        assert!(!pruned.lockfile.contains("name = \"requests\""));
        assert!(!pruned.lockfile.contains("name = \"virt\""));
        // The manifest members list is rewritten, headers survive, and the
        // output re-parses.
        assert!(pruned.lockfile.contains("version = 1"));
        assert!(pruned.lockfile.contains("requires-python = \">=3.12\""));
        assert!(!pruned.lockfile.contains("\"app\""));
        let closures = uv_external_closures(&pruned.lockfile, &["lib-a".to_string()]).unwrap();
        assert_eq!(
            names(&closures["lib-a"]),
            vec!["idna@3.18 registry+https://pypi.org/simple sha256:idna-wheel"]
        );
    }

    #[test]
    fn test_prune_lock_retains_dev_reachable_members() {
        // app's dev group pulls in virt; both stay members of the pruned
        // workspace even though only app was requested.
        let pruned = uv_prune_lock(LOCK, &["app".to_string()]).unwrap();
        assert_eq!(pruned.members, vec!["app", "lib-a", "virt"]);
    }

    #[test]
    fn test_prune_lock_stale_root_errors() {
        let err = uv_prune_lock(LOCK, &["not-in-lock".to_string()]).unwrap_err();
        assert!(matches!(err, Error::MissingMember(_)));
    }

    #[test]
    fn test_missing_member_is_skipped() {
        let members = vec!["not-in-lock".to_string()];
        let closures = uv_external_closures(LOCK, &members).unwrap();
        assert!(closures.is_empty());
    }

    #[test]
    fn test_multi_version_reference_includes_all_matches() {
        let lock = r#"
[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "numpy" },
]

[[package]]
name = "numpy"
version = "1.26.4"
source = { registry = "https://pypi.org/simple" }
sdist = { hash = "sha256:old" }

[[package]]
name = "numpy"
version = "2.1.0"
source = { registry = "https://pypi.org/simple" }
sdist = { hash = "sha256:new" }
"#;
        let closures = uv_external_closures(lock, &["app".to_string()]).unwrap();
        assert_eq!(
            names(&closures["app"]),
            vec![
                "numpy@1.26.4 registry+https://pypi.org/simple sha256:old",
                "numpy@2.1.0 registry+https://pypi.org/simple sha256:new",
            ]
        );
    }

    #[test]
    fn test_versioned_reference_selects_one_match() {
        let lock = r#"
[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "numpy", version = "1.26.4" },
]

[[package]]
name = "numpy"
version = "1.26.4"
source = { registry = "https://pypi.org/simple" }
sdist = { hash = "sha256:old" }

[[package]]
name = "numpy"
version = "2.1.0"
source = { registry = "https://pypi.org/simple" }
sdist = { hash = "sha256:new" }
"#;
        let closures = uv_external_closures(lock, &["app".to_string()]).unwrap();
        assert_eq!(
            names(&closures["app"]),
            vec!["numpy@1.26.4 registry+https://pypi.org/simple sha256:old"]
        );
    }

    #[test]
    fn test_unknown_dependency_errors() {
        let lock = r#"
[[package]]
name = "app"
version = "0.1.0"
source = { editable = "packages/app" }
dependencies = [
    { name = "ghost" },
]
"#;
        let err = uv_external_closures(lock, &["app".to_string()]).unwrap_err();
        assert!(matches!(err, Error::UnknownDependency(_)));
    }

    #[test]
    fn test_git_source_participates_in_identity() {
        let lock = r#"
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
        let closures = uv_external_closures(lock, &["app".to_string()]).unwrap();
        // A git rev bump changes `source` but not `version`; the identity
        // must capture it.
        assert_eq!(
            names(&closures["app"]),
            vec!["git-dep@0.2.0 git+https://example.com/dep?rev=main#deadbeef"]
        );
    }
}
