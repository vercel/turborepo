use std::{any::Any, collections::HashMap};

use semver::Version;
use serde::{
    Deserialize, Serialize,
    de::{Deserializer, IgnoredAny, MapAccess, Visitor},
    ser::SerializeMap,
};
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
    #[serde(default)]
    packages: HashMap<String, NpmPackage>,
    // npm v2 lockfiles carry a top-level legacy `dependencies` tree that
    // duplicates `packages`. Resolution only ever uses `packages`, so instead
    // of materializing a potentially very large redundant tree, we record only
    // whether it has entries: `load` rejects lockfiles that have a legacy tree
    // but no `packages`, since those cannot be resolved.
    // Parsing it as a known field also keeps it out of 'other' so we don't
    // need to worry about accidentally serializing it.
    #[serde(
        default,
        rename = "dependencies",
        deserialize_with = "deserialize_legacy_dependencies"
    )]
    has_legacy_dependencies: bool,
    // We want to reserialize any additional fields, but we don't use them
    // we keep them as raw values to avoid describing the correct schema.
    #[serde(flatten)]
    other: Map<String, Value>,
}

/// Deserializes npm's top-level legacy `dependencies` table, recording only
/// whether it has entries. Keys and values are consumed with [`IgnoredAny`] so
/// the (potentially very large) legacy tree is never materialized; its
/// emptiness is all [`NpmLockfile::load`] needs in order to reject lockfiles
/// that only have a legacy tree.
///
/// This must not become `#[serde(skip_deserializing)]`: skipping the field
/// entirely would silently accept those lockfiles instead of rejecting them.
fn deserialize_legacy_dependencies<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    struct LegacyDependenciesVisitor;

    impl<'de> Visitor<'de> for LegacyDependenciesVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // Matches serde's message for map fields so rejections of
            // malformed lockfiles are unchanged.
            formatter.write_str("a map")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut has_entries = false;
            while map.next_key::<IgnoredAny>()?.is_some() {
                map.next_value::<IgnoredAny>()?;
                has_entries = true;
            }
            Ok(has_entries)
        }
    }

    deserializer.deserialize_map(LegacyDependenciesVisitor)
}

impl Serialize for NpmLockfile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Sort packages keys for deterministic output matching npm's sorted
        // lockfile format.
        let mut sorted_packages: Vec<_> = self.packages.iter().collect();
        sorted_packages.sort_unstable_by_key(|(a, _)| *a);

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

    fn transitive_edge_resolver(&self) -> Option<Box<dyn crate::TransitiveEdgeResolver + '_>> {
        Some(Box::new(NpmEdgeResolver { lockfile: self }))
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
        // Index the entries that link to a retained workspace by their
        // `resolved` path in one pass so each workspace below needs a single
        // lookup instead of another scan of every lockfile entry. Scanning
        // per workspace made link discovery O(retained workspaces × lockfile
        // entries) in the worst case, and a workspace without a link
        // exhausted the entire map. `or_insert` keeps the first entry
        // encountered in iteration order, preserving the first-match-and-break
        // behavior of the previous scan when multiple entries share a
        // `resolved` target.
        let ws_set: std::collections::HashSet<&str> =
            workspace_packages.iter().map(|s| s.as_str()).collect();
        let mut workspace_links: HashMap<&str, (&String, &NpmPackage)> =
            HashMap::with_capacity(ws_set.len());
        for (key, entry) in &self.packages {
            if let Some(resolved) = entry.resolved.as_deref()
                && ws_set.contains(resolved)
            {
                workspace_links.entry(resolved).or_insert((key, entry));
            }
        }

        for workspace in workspace_packages {
            let pkg = self.get_package(workspace)?;
            pruned_packages.insert(workspace.to_string(), pkg.clone());

            if let Some(&(key, entry)) = workspace_links.get(workspace.as_str()) {
                pruned_packages.insert(key.clone(), entry.clone());
            }
        }

        // After pruning, a package nested under a workspace's node_modules
        // (e.g. `apps/web/node_modules/next@15`) may exist without a
        // corresponding hoisted version (`node_modules/next`) if the hoisted
        // version was only needed by a now-pruned workspace and the transitive
        // closure didn't include it. Promote the nested version to the hoisted
        // position so npm ci sees a consistent tree.
        // See https://github.com/vercel/turborepo/issues/10985
        let requested: std::collections::HashSet<&str> =
            packages.iter().map(|s| s.as_str()).collect();
        Self::rehoist_packages(&mut pruned_packages, &ws_set, &requested, &self.packages);

        Ok(Box::new(Self {
            lockfile_version: self.lockfile_version,
            packages: pruned_packages,
            // The pruned lockfile never carries a legacy dependency tree.
            has_legacy_dependencies: false,
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

    fn package_source(&self, package: &Package) -> crate::PackageSource {
        let Some(entry) = self.packages.get(&package.key) else {
            return crate::PackageSource::Registry;
        };
        if entry.link {
            crate::PackageSource::Link
        } else {
            entry
                .resolved
                .as_deref()
                .map(crate::package_source_from_identifier)
                .unwrap_or(crate::PackageSource::Registry)
        }
    }

    fn format_version(&self) -> Option<String> {
        Some(self.lockfile_version.to_string())
    }
}

/// Proves per-edge workspace independence for the shared closure DP.
///
/// npm transitive edges come from `all_dependencies`, which emits fully
/// resolved lockfile keys (`find_dep_in_lockfile` only returns keys present
/// in the packages map). `resolve_package`'s first candidate matches such a
/// key directly without consulting the workspace, so every workspace
/// resolves the edge identically. A name that is not a lockfile key would
/// fall through to the workspace-scoped candidates, so report it sensitive
/// (defensive; unreachable via `all_dependencies` output).
struct NpmEdgeResolver<'a> {
    lockfile: &'a NpmLockfile,
}

impl crate::TransitiveEdgeResolver for NpmEdgeResolver<'_> {
    fn resolve_edge(
        &self,
        name: &str,
        _version: &str,
    ) -> Result<crate::TransitiveEdgeResolution, crate::Error> {
        Ok(match self.lockfile.packages.get(name) {
            // Mirrors the `name` candidate in `resolve_package`.
            Some(pkg) => crate::TransitiveEdgeResolution::Global(Some(Package {
                key: name.to_string(),
                version: pkg.version.clone().unwrap_or_default(),
            })),
            None => crate::TransitiveEdgeResolution::WorkspaceSensitive,
        })
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
            || (lockfile.packages.is_empty() && lockfile.has_legacy_dependencies)
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

        let mut to_rehoist: Vec<(String, String)> = candidates
            .into_iter()
            .filter_map(|(hoisted_key, nested_keys)| {
                if nested_keys.len() == 1 {
                    nested_keys
                        .into_iter()
                        .next()
                        .map(|nested_key| (nested_key, hoisted_key))
                } else {
                    None
                }
            })
            .collect();
        // Candidates come from HashMap iteration; sort so relocation placement
        // decisions below don't depend on hash ordering.
        // See https://github.com/vercel/turborepo/issues/13321
        to_rehoist.sort();

        for (nested_key, hoisted_key) in &to_rehoist {
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
            pruned.remove(hoisted_key);

            // Promote nested entry.
            if let Some(pkg) = pruned.remove(nested_key) {
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

        // Promoting a package changes its position in the tree, so any of its
        // transitive deps that were resolved through workspace-nested siblings
        // (e.g. `apps/app-a/node_modules/mime`) are no longer reachable from
        // the new hoisted position. Walk each promoted package's dependency
        // closure and relocate any stranded versions.
        //
        // This runs as a separate pass after all promotions: a relocation can
        // copy a sibling to the root slot and remove its nested source, and if
        // that sibling were itself a still-pending promotion candidate, the
        // interleaved processing would remove the root entry and then find
        // nothing left to promote, dropping the package entirely.
        // See https://github.com/vercel/turborepo/issues/13321
        for (nested_key, hoisted_key) in &to_rehoist {
            let mut visited = std::collections::HashSet::new();
            Self::relocate_stranded_closure(
                pruned,
                original_packages,
                nested_key,
                hoisted_key,
                &mut visited,
            );
        }
    }

    /// After a workspace-nested package has been promoted to the hoisted
    /// position, its transitive dependencies that previously resolved through
    /// workspace-nested siblings can become unreachable: Node's resolution only
    /// walks upward, so a package now at `node_modules/send` cannot reach
    /// `apps/app-a/node_modules/mime`.
    ///
    /// This walks the promoted package's dependency closure (using the original
    /// lockfile as the source of truth for which version each dependency must
    /// resolve to) and, for every dependency that no longer resolves to the
    /// correct version, copies that version from the original lockfile to a
    /// position the promoted package can reach — hoisted to the root slot when
    /// it's free, otherwise nested directly under the promoted package.
    /// See https://github.com/vercel/turborepo/issues/13109
    fn relocate_stranded_closure(
        pruned: &mut HashMap<String, NpmPackage>,
        original: &HashMap<String, NpmPackage>,
        original_key: &str,
        new_key: &str,
        visited: &mut std::collections::HashSet<String>,
    ) {
        if !visited.insert(new_key.to_string()) {
            return;
        }

        // The authoritative dependency list comes from the original lockfile.
        let Some(pkg) = original.get(original_key) else {
            return;
        };
        let dep_names: Vec<String> = pkg.dep_keys().cloned().collect();

        for dep in dep_names {
            // The version this dependency resolved to in the original tree.
            let Some((orig_dep_key, Some(orig_version))) =
                Self::resolve_in_map(original, original_key, &dep)
            else {
                // No concrete resolution (missing/optional dep or a workspace
                // link without a version) — nothing to relocate.
                continue;
            };

            // What it currently resolves to from the new (pruned) position.
            if let Some((pruned_dep_key, Some(pruned_version))) =
                Self::resolve_in_map(pruned, new_key, &dep)
                && pruned_version == orig_version
            {
                // Already reachable and correct — descend to validate the
                // dependency's own closure.
                Self::relocate_stranded_closure(
                    pruned,
                    original,
                    &orig_dep_key,
                    &pruned_dep_key,
                    visited,
                );
                continue;
            }

            // Missing or wrong version. Place the correct version where the
            // promoted package can reach it: hoist to the root slot if free,
            // otherwise nest directly under the promoted package.
            let hoisted = format!("node_modules/{dep}");
            let placement = if pruned.contains_key(&hoisted) {
                format!("{new_key}/node_modules/{dep}")
            } else {
                hoisted
            };

            // Copy the dependency and its nested subtree from the original
            // lockfile to the new placement.
            if let Some(entry) = original.get(&orig_dep_key) {
                pruned.insert(placement.clone(), entry.clone());
            }
            let orig_sub_prefix = format!("{orig_dep_key}/node_modules/");
            let new_sub_prefix = format!("{placement}/node_modules/");
            for (k, v) in original.iter() {
                if let Some(rest) = k.strip_prefix(&orig_sub_prefix) {
                    pruned.insert(format!("{new_sub_prefix}{rest}"), v.clone());
                }
            }

            // Remove the original nested location only if no remaining package
            // still resolves to it. Sibling consumers in the same workspace can
            // share that nested copy even after this package is promoted.
            if orig_dep_key != placement
                && !Self::is_resolved_by_any_consumer(pruned, &orig_dep_key)
            {
                pruned.remove(&orig_dep_key);
                let strand_prefix = format!("{orig_dep_key}/node_modules/");
                let strays: Vec<String> = pruned
                    .keys()
                    .filter(|k| k.starts_with(&strand_prefix))
                    .cloned()
                    .collect();
                for s in strays {
                    pruned.remove(&s);
                }
            }

            // Descend into the relocated dependency.
            Self::relocate_stranded_closure(pruned, original, &orig_dep_key, &placement, visited);
        }
    }

    fn is_resolved_by_any_consumer(packages: &HashMap<String, NpmPackage>, dep_key: &str) -> bool {
        let Some(dep_name) = dep_key.rsplit_once("node_modules/").map(|(_, name)| name) else {
            return false;
        };

        packages.iter().any(|(consumer_key, pkg)| {
            pkg.dep_keys().any(|dep| {
                dep == dep_name
                    && Self::resolve_in_map(packages, consumer_key, dep)
                        .is_some_and(|(resolved_key, _)| resolved_key == dep_key)
            })
        })
    }

    /// Resolve a dependency name within an arbitrary packages map by walking up
    /// the node_modules hierarchy from `key`, mirroring Node's resolution.
    /// Returns the resolved key and its version (if the entry has one).
    fn resolve_in_map(
        packages: &HashMap<String, NpmPackage>,
        key: &str,
        dep: &str,
    ) -> Option<(String, Option<String>)> {
        // First candidate: nested directly under the current package.
        let nested = format!("{key}/node_modules/{dep}");
        if let Some(entry) = packages.get(&nested) {
            return Some((nested, entry.version.clone()));
        }

        // Walk up the node_modules hierarchy.
        let mut curr = Some(key);
        while let Some(k) = curr {
            let parent = Self::npm_path_parent(k);
            let candidate = match parent {
                Some(p) => format!("{p}node_modules/{dep}"),
                None => format!("node_modules/{dep}"),
            };
            if let Some(entry) = packages.get(&candidate) {
                return Some((candidate, entry.version.clone()));
            }
            curr = parent;
        }
        None
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
            .filter(|&parent| !parent.is_empty())
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

    // Regression test for https://github.com/vercel/turborepo/issues/13109
    //
    // When the full monorepo has two incompatible versions of a shared
    // transitive dep — one hoisted to root, the other workspace-nested — and
    // the workspace-nested package gets promoted to root during pruning, its
    // workspace-nested transitive deps (siblings under the workspace's
    // node_modules, not under the package itself) must follow it so they remain
    // reachable.
    //
    // Here app-1 forces send@1.2.1 + mime@3.0.0 (root devDep) to root, while
    // app-2 needs send@0.17.2 which requires mime@1.6.0. mime@1.6.0 lives at
    // `apps/app-2/node_modules/mime`. Pruning to app-2 drops send@1.2.1 and
    // promotes send@0.17.2 to `node_modules/send`; mime@1.6.0 must move to a
    // position where the promoted send can resolve it (here nested under send,
    // since `node_modules/mime` is still occupied by the root devDep 3.0.0).
    // The original nested copy must remain when another app-2 dependency still
    // resolves to it.
    #[test]
    fn test_subgraph_relocates_stranded_promoted_deps() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["apps/*"],
                    "devDependencies": {
                        "legacy": "^2.0.0",
                        "mime": "^3.0.0"
                    }
                },
                "node_modules/app-1": {
                    "resolved": "apps/app-1",
                    "link": true
                },
                "node_modules/app-2": {
                    "resolved": "apps/app-2",
                    "link": true
                },
                "node_modules/destroy": {
                    "version": "1.0.4"
                },
                "node_modules/mime": {
                    "version": "3.0.0"
                },
                "node_modules/legacy": {
                    "version": "2.0.0"
                },
                "node_modules/send": {
                    "version": "1.2.1",
                    "dependencies": { "encodeurl": "^2.0.0" }
                },
                "node_modules/send/node_modules/encodeurl": {
                    "version": "2.0.0"
                },
                "node_modules/encodeurl": {
                    "version": "1.0.2"
                },
                "apps/app-1": {
                    "version": "1.0.0",
                    "dependencies": { "send": "^1.0.0" }
                },
                "apps/app-2": {
                    "version": "1.0.0",
                    "dependencies": {
                        "legacy": "^1.0.0",
                        "send": "^0.17.0"
                    }
                },
                "apps/app-2/node_modules/legacy": {
                    "version": "1.0.0",
                    "dependencies": { "mime": "1.6.0" }
                },
                "apps/app-2/node_modules/mime": {
                    "version": "1.6.0"
                },
                "apps/app-2/node_modules/send": {
                    "version": "0.17.2",
                    "dependencies": {
                        "destroy": "~1.0.4",
                        "encodeurl": "~1.0.2",
                        "mime": "1.6.0"
                    }
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        // Pruning to app-2: the root workspace keeps its mime@3.0.0 devDep and
        // app-2 pulls in send@0.17.2's closure (mime@1.6.0 nested, destroy and
        // encodeurl@1.0.2 hoisted).
        let workspace_packages = vec!["apps/app-2".to_string()];
        let packages = vec![
            "node_modules/destroy".to_string(),
            "node_modules/legacy".to_string(),
            "node_modules/mime".to_string(),
            "node_modules/encodeurl".to_string(),
            "apps/app-2/node_modules/legacy".to_string(),
            "apps/app-2/node_modules/mime".to_string(),
            "apps/app-2/node_modules/send".to_string(),
        ];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        // send@0.17.2 was promoted to the hoisted slot (the root version 1.2.1
        // was only needed by the now-pruned app-1).
        assert_eq!(
            reparsed
                .packages
                .get("node_modules/send")
                .and_then(|p| p.version.as_deref()),
            Some("0.17.2"),
            "send@0.17.2 should be promoted to node_modules/send"
        );

        // mime@1.6.0 must be reachable from the promoted send. node_modules/mime
        // is still occupied by the root devDep (3.0.0), so it must be nested
        // directly under send rather than stranded under apps/app-2.
        assert_eq!(
            reparsed
                .packages
                .get("node_modules/send/node_modules/mime")
                .and_then(|p| p.version.as_deref()),
            Some("1.6.0"),
            "send@0.17.2's mime@1.6.0 must be relocated where send can resolve it"
        );
        assert_eq!(
            reparsed
                .packages
                .get("apps/app-2/node_modules/mime")
                .and_then(|p| p.version.as_deref()),
            Some("1.6.0"),
            "the shared apps/app-2/node_modules/mime copy must remain for sibling consumers"
        );
        assert_eq!(
            reparsed
                .packages
                .get("apps/app-2/node_modules/legacy")
                .and_then(|p| p.version.as_deref()),
            Some("1.0.0"),
            "legacy@1.0.0 should remain nested and resolve apps/app-2/node_modules/mime"
        );

        // The root devDependency mime@3.0.0 must be preserved untouched.
        assert_eq!(
            reparsed
                .packages
                .get("node_modules/mime")
                .and_then(|p| p.version.as_deref()),
            Some("3.0.0"),
            "root devDep mime@3.0.0 should be preserved"
        );

        // Hoisted deps that already resolve correctly must stay put.
        assert!(
            reparsed.packages.contains_key("node_modules/destroy"),
            "hoisted destroy@1.0.4 should be reachable from send"
        );

        // No part of the old root send@1.2.1 subtree should linger.
        assert!(
            !reparsed
                .packages
                .contains_key("node_modules/send/node_modules/encodeurl"),
            "old send@1.2.1's nested encodeurl@2.0.0 should be gone"
        );
        // send@0.17.2 wants encodeurl@~1.0.2, which is the hoisted 1.0.2.
        assert_eq!(
            reparsed
                .packages
                .get("node_modules/encodeurl")
                .and_then(|p| p.version.as_deref()),
            Some("1.0.2"),
            "hoisted encodeurl@1.0.2 (what send@0.17.2 needs) should remain"
        );
    }

    // Regression test for https://github.com/vercel/turborepo/issues/13321
    //
    // The original lockfile has dev-only hoisted copies of send/http-errors/
    // fresh at the root, and the app workspace has its own nested copies of
    // all three (send@1.2.1 depending on the nested http-errors@2.0.1 and
    // fresh@2.0.0). Pruning to the app makes all three nested entries rehoist
    // candidates. Candidate processing order used to come from HashMap
    // iteration, and relocate_stranded_closure ran interleaved with
    // promotions: if send was promoted first, its relocation moved the nested
    // http-errors/fresh to the root and removed the nested sources, then the
    // still-queued http-errors/fresh candidates removed the root entries and
    // found nothing left to promote — dropping the packages entirely.
    //
    // Run the prune repeatedly since the old failure depended on hash order.
    #[test]
    fn test_subgraph_rehoist_is_deterministic_and_complete() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["apps/*"],
                    "devDependencies": {
                        "fresh": "0.5.2",
                        "http-errors": "2.0.0",
                        "send": "0.19.0"
                    }
                },
                "node_modules/app": {
                    "resolved": "apps/app",
                    "link": true
                },
                "node_modules/fresh": {
                    "version": "0.5.2",
                    "dev": true
                },
                "node_modules/http-errors": {
                    "version": "2.0.0",
                    "dev": true
                },
                "node_modules/send": {
                    "version": "0.19.0",
                    "dev": true,
                    "dependencies": {
                        "fresh": "0.5.2",
                        "http-errors": "2.0.0"
                    }
                },
                "apps/app": {
                    "version": "1.0.0",
                    "dependencies": { "send": "^1.2.1" }
                },
                "apps/app/node_modules/fresh": {
                    "version": "2.0.0"
                },
                "apps/app/node_modules/http-errors": {
                    "version": "2.0.1"
                },
                "apps/app/node_modules/send": {
                    "version": "1.2.1",
                    "dependencies": {
                        "fresh": "^2.0.0",
                        "http-errors": "^2.0.0"
                    }
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["apps/app".to_string()];
        let packages = vec![
            "apps/app/node_modules/send".to_string(),
            "apps/app/node_modules/http-errors".to_string(),
            "apps/app/node_modules/fresh".to_string(),
        ];

        let mut first_encoded: Option<Vec<u8>> = None;
        for run in 0..32 {
            let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
            let encoded = pruned.encode().unwrap();
            let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

            // The promoted send@1.2.1 must always be able to resolve its
            // production deps at their correct versions.
            for (key, version) in [
                ("node_modules/send", "1.2.1"),
                ("node_modules/http-errors", "2.0.1"),
                ("node_modules/fresh", "2.0.0"),
            ] {
                assert_eq!(
                    reparsed
                        .packages
                        .get(key)
                        .and_then(|p| p.version.as_deref()),
                    Some(version),
                    "run {run}: expected {key}@{version} in pruned lockfile"
                );
            }

            // The pruned output must be byte-for-byte identical across runs.
            match &first_encoded {
                None => first_encoded = Some(encoded),
                Some(first) => assert_eq!(
                    first, &encoded,
                    "run {run}: pruned lockfile differs between runs"
                ),
            }
        }
    }

    // Workspace links (`resolved` pointing at a retained workspace path) must
    // be included alongside the workspace entry itself so `npm ci` can
    // reinstall the workspace through its link slot. Links to workspaces that
    // are not retained must be dropped.
    #[test]
    fn test_subgraph_includes_workspace_link_entries() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["apps/*", "packages/*"]
                },
                "node_modules/left-pad": {
                    "version": "1.3.0",
                    "resolved": "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz"
                },
                "node_modules/app": {
                    "resolved": "apps/app",
                    "link": true
                },
                "node_modules/ui": {
                    "resolved": "packages/ui",
                    "link": true
                },
                "node_modules/pruned-lib": {
                    "resolved": "packages/pruned-lib",
                    "link": true
                },
                "apps/app": {
                    "version": "1.0.0",
                    "dependencies": {
                        "ui": "*",
                        "left-pad": "^1.0.0"
                    }
                },
                "packages/ui": {
                    "version": "1.0.0"
                },
                "packages/pruned-lib": {
                    "version": "1.0.0"
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["apps/app".to_string(), "packages/ui".to_string()];
        let packages = vec!["node_modules/left-pad".to_string()];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        for (key, resolved) in [
            ("node_modules/app", "apps/app"),
            ("node_modules/ui", "packages/ui"),
        ] {
            let entry = reparsed
                .packages
                .get(key)
                .unwrap_or_else(|| panic!("workspace link {key:?} was dropped"));
            assert!(entry.link, "link entry {key:?} lost its link flag");
            assert_eq!(
                entry.resolved.as_deref(),
                Some(resolved),
                "link entry {key:?} has the wrong resolved path"
            );
        }

        // The requested registry package and the workspace entries themselves
        // must survive.
        assert!(reparsed.packages.contains_key("node_modules/left-pad"));
        assert!(reparsed.packages.contains_key("apps/app"));
        assert!(reparsed.packages.contains_key("packages/ui"));

        // The link (and entry) of a workspace that was pruned must not.
        assert!(
            !reparsed.packages.contains_key("node_modules/pruned-lib"),
            "link to a pruned workspace was retained"
        );
        assert!(
            !reparsed.packages.contains_key("packages/pruned-lib"),
            "pruned workspace entry was retained"
        );
    }

    // A retained workspace with no matching link entry anywhere in the
    // lockfile must prune without error and without pulling in unrelated
    // entries. (This is the path that previously exhausted the whole packages
    // map once per workspace.)
    #[test]
    fn test_subgraph_workspace_without_link_entry() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["apps/*"]
                },
                "node_modules/left-pad": {
                    "version": "1.3.0",
                    "resolved": "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz"
                },
                "apps/ghost": {
                    "version": "1.0.0",
                    "dependencies": {
                        "left-pad": "^1.0.0"
                    }
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["apps/ghost".to_string()];
        let packages = vec!["node_modules/left-pad".to_string()];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        assert!(
            reparsed.packages.contains_key("apps/ghost"),
            "the workspace without a link was dropped"
        );
        assert!(
            reparsed.packages.contains_key("node_modules/left-pad"),
            "the requested registry package was dropped"
        );
        assert_eq!(
            reparsed.packages.len(),
            3, // "" root, apps/ghost, node_modules/left-pad
            "entries without a matching workspace link must not be retained"
        );
    }

    // When multiple entries link to the same workspace path, exactly one is
    // retained — the first in iteration order, matching the previous
    // per-workspace scan's first-match-and-break behavior. HashMap iteration
    // order varies across processes, so the winner is not asserted; only that
    // a single duplicate is kept.
    #[test]
    fn test_subgraph_duplicate_resolved_targets_keep_single_link() {
        let json = r#"{
            "lockfileVersion": 3,
            "requires": true,
            "packages": {
                "": {
                    "name": "monorepo",
                    "workspaces": ["packages/*"]
                },
                "node_modules/dup": {
                    "resolved": "packages/dup",
                    "link": true
                },
                "node_modules/@scope/dup": {
                    "resolved": "packages/dup",
                    "link": true
                },
                "packages/dup": {
                    "version": "1.0.0"
                }
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();

        let workspace_packages = vec!["packages/dup".to_string()];
        let packages = vec![];

        let pruned = lockfile.subgraph(&workspace_packages, &packages).unwrap();
        let encoded = pruned.encode().unwrap();
        let reparsed: NpmLockfile = NpmLockfile::load(&encoded).unwrap();

        let duplicates = ["node_modules/dup", "node_modules/@scope/dup"];
        let retained: Vec<&str> = duplicates
            .iter()
            .copied()
            .filter(|key| reparsed.packages.contains_key(*key))
            .collect();
        assert_eq!(
            retained.len(),
            1,
            "expected exactly one duplicate-resolved link, retained {retained:?}"
        );
        assert!(
            reparsed.packages.contains_key("packages/dup"),
            "the workspace entry itself was dropped"
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

    // npm v2 lockfiles duplicate the resolved tree in a top-level legacy
    // `dependencies` table. We never materialize that table; these tests pin
    // every behavior that depends on it: rejecting lockfiles that only have a
    // legacy tree, accepting well-formed v2 lockfiles regardless of the
    // table's contents, rejecting malformed tables, and never reserializing
    // the table.
    #[test]
    fn test_load_rejects_lockfile_with_only_legacy_dependencies() {
        let json = r#"{
            "lockfileVersion": 2,
            "dependencies": {
                "foo": {
                    "version": "1.0.0",
                    "requires": { "bar": "^1.0.0" }
                }
            }
        }"#;

        let err = NpmLockfile::load(json.as_bytes()).unwrap_err();
        assert!(matches!(err, Error::UnsupportedNpmVersion));
    }

    #[test]
    fn test_load_rejects_v1_lockfiles() {
        // v1 lockfiles are rejected regardless of their contents.
        for json in [
            r#"{"lockfileVersion": 1, "dependencies": {"foo": {"version": "1.0.0"}}}"#,
            r#"{"lockfileVersion": 0, "packages": {"node_modules/foo": {"version": "1.0.0"}}}"#,
        ] {
            let err = NpmLockfile::load(json.as_bytes()).unwrap_err();
            assert!(matches!(err, Error::UnsupportedNpmVersion));
        }
    }

    #[test]
    fn test_load_accepts_missing_or_empty_legacy_dependencies() {
        // An empty or missing legacy table is not "legacy-only": with no
        // packages to resolve there is nothing unsupported about the lockfile.
        for json in [
            r#"{"lockfileVersion": 2}"#,
            r#"{"lockfileVersion": 2, "packages": {}}"#,
            r#"{"lockfileVersion": 2, "dependencies": {}}"#,
            r#"{"lockfileVersion": 2, "packages": {}, "dependencies": {}}"#,
        ] {
            NpmLockfile::load(json.as_bytes())
                .unwrap_or_else(|err| panic!("should load {json}: {err}"));
        }
    }

    #[test]
    fn test_load_rejects_malformed_legacy_dependencies() {
        // The legacy table must still be validated as a map, exactly as it
        // was when it was deserialized into a map.
        for json in [
            r#"{"lockfileVersion": 2, "dependencies": 5}"#,
            r#"{"lockfileVersion": 2, "dependencies": []}"#,
            r#"{"lockfileVersion": 2, "dependencies": null}"#,
            r#"{"lockfileVersion": 2, "dependencies": "foo"}"#,
        ] {
            let err = NpmLockfile::load(json.as_bytes()).unwrap_err();
            assert!(
                matches!(err, Error::JsonError(_)),
                "expected a JSON error for {json}, got {err:?}"
            );
            assert!(
                err.to_string().contains("expected a map"),
                "unexpected error message for {json}: {err}"
            );
        }
    }

    #[test]
    fn test_load_accepts_arbitrary_legacy_dependency_values() {
        // Legacy tree entries are never interpreted, so any value the old
        // `serde_json::Value` deserialization accepted must still be accepted.
        let json = r#"{
            "lockfileVersion": 2,
            "packages": {
                "": { "name": "monorepo" },
                "node_modules/foo": { "version": "1.0.0" }
            },
            "dependencies": {
                "foo": {
                    "version": "1.0.0",
                    "resolved": "https://registry.npmjs.org/foo/-/foo-1.0.0.tgz",
                    "requires": { "bar": "^2.0.0" },
                    "dependencies": { "bar": { "version": "2.0.0" } }
                },
                "duplicate": { "version": "1.0.0" },
                "duplicate": { "version": "2.0.0" },
                "number": 5,
                "boolean": true,
                "null": null,
                "list": [1, "two", { "three": 3 }]
            }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();
        assert!(lockfile.packages.contains_key("node_modules/foo"));
    }

    #[test]
    fn test_encode_omits_legacy_dependencies_but_keeps_unknown_fields() {
        let json = r#"{
            "lockfileVersion": 2,
            "requires": true,
            "packages": { "node_modules/foo": { "version": "1.0.0" } },
            "dependencies": { "foo": { "version": "1.0.0" } }
        }"#;

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();
        let encoded: serde_json::Value =
            serde_json::from_slice(&lockfile.encode().unwrap()).unwrap();
        assert!(encoded.get("dependencies").is_none());
        assert_eq!(encoded.get("requires"), Some(&serde_json::json!(true)));
        assert_eq!(encoded.get("lockfileVersion"), Some(&serde_json::json!(2)));
    }

    #[test]
    fn test_legacy_dependencies_do_not_change_pruned_output() {
        // The legacy tree is redundant with `packages`, so a v2 lockfile and
        // the same lockfile without the legacy table must produce identical
        // pruned lockfiles.
        let packages = r#""packages": {
            "": { "name": "monorepo", "workspaces": ["packages/*"] },
            "node_modules/pkg": { "resolved": "packages/pkg", "link": true },
            "packages/pkg": { "version": "1.0.0", "dependencies": { "foo": "^1.0.0" } },
            "node_modules/foo": { "version": "1.0.0" }
        }"#;
        let with_legacy = format!(
            r#"{{"lockfileVersion": 2, {packages}, "dependencies": {{
                "foo": {{ "version": "1.0.0", "requires": {{}} }},
                "pkg": {{ "version": "1.0.0" }}
            }}}}"#
        );
        let without_legacy = format!(r#"{{"lockfileVersion": 2, {packages}}}"#);

        let with_legacy = NpmLockfile::load(with_legacy.as_bytes()).unwrap();
        let without_legacy = NpmLockfile::load(without_legacy.as_bytes()).unwrap();

        let pruned = |lockfile: &NpmLockfile| {
            lockfile
                .subgraph(
                    &["packages/pkg".to_string()],
                    &["node_modules/foo".to_string()],
                )
                .unwrap()
                .encode()
                .unwrap()
        };
        assert_eq!(pruned(&with_legacy), pruned(&without_legacy));
    }

    #[test]
    fn test_load_handles_large_legacy_dependency_tree() {
        // npm v2 duplicates the whole resolved tree into the legacy table, so
        // it can be very large. Its contents must not affect loading,
        // resolution, or pruning.
        let mut json = String::from(
            r#"{"lockfileVersion": 2, "packages": {
                "": { "name": "monorepo", "workspaces": ["packages/*"] },
                "node_modules/pkg": { "resolved": "packages/pkg", "link": true },
                "packages/pkg": { "version": "1.0.0", "dependencies": { "foo": "^1.0.0" } },
                "node_modules/foo": { "version": "1.0.0" }
            }, "dependencies": {"#,
        );
        for i in 0..10_000 {
            json.push_str(&format!(
                "\"legacy-{i}\": {{\"version\": \"1.0.{i}\", \"requires\": {{\"bar\": \
                 \"^{i}.0.0\"}}, \"dependencies\": {{\"bar\": {{\"version\": \"2.0.0\"}}}}}},"
            ));
        }
        json.push_str("\"foo\": {\"version\": \"1.0.0\"}}}");

        let lockfile = NpmLockfile::load(json.as_bytes()).unwrap();
        assert!(lockfile.packages.contains_key("node_modules/foo"));

        let pruned = lockfile
            .subgraph(
                &["packages/pkg".to_string()],
                &["node_modules/foo".to_string()],
            )
            .unwrap();
        let encoded: serde_json::Value = serde_json::from_slice(&pruned.encode().unwrap()).unwrap();
        assert!(encoded.get("dependencies").is_none());
        assert!(encoded["packages"].get("node_modules/foo").is_some());
    }
}
