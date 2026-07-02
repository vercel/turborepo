use semver::{Version, VersionReq};

use super::{BunLockfile, PackageEntry, PackageIdent, PossibleKeyIter, VersionSpec};

impl BunLockfile {
    fn process_package_entry(
        &self,
        entry: &PackageEntry,
        name: &str,
        override_version: &str,
        resolved_version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        let ident = PackageIdent::parse(&entry.ident);

        // Filter out workspace mapping entries
        if ident.is_workspace() {
            return Ok(None);
        }

        // Check for overrides
        if override_version != resolved_version {
            let override_ident = format!("{name}@{override_version}");
            if let Some((_override_key, override_entry)) = self.index.get_by_ident(&override_ident)
            {
                let mut pkg_version = override_entry.version().to_string();
                if let Some(patch) = self.data.patched_dependencies.get(&override_entry.ident) {
                    pkg_version.push('+');
                    pkg_version.push_str(patch);
                }
                return Ok(Some(crate::Package {
                    key: override_entry.ident.to_string(),
                    version: pkg_version,
                }));
            }
        }

        // Return the package with its version (and patch if applicable)
        let mut version = entry.version().to_string();
        if let Some(patch) = self.data.patched_dependencies.get(&entry.ident) {
            version.push('+');
            version.push_str(patch);
        }
        Ok(Some(crate::Package {
            key: entry.ident.to_string(),
            version,
        }))
    }

    /// Check if a package version satisfies a version specification.
    ///
    /// Returns true if the version satisfies the spec, false otherwise.
    /// For non-semver specs (tags, catalogs, workspaces), returns true.
    pub(super) fn version_satisfies_spec(&self, version: &str, version_spec: &str) -> bool {
        let spec = VersionSpec::parse(version_spec);

        match spec {
            VersionSpec::Semver(spec_str) => {
                // Parse both the requirement and the version
                let Ok(req) = VersionReq::parse(&spec_str) else {
                    // If we can't parse the requirement, be lenient and accept it
                    return true;
                };

                let Ok(ver) = Version::parse(version) else {
                    // If we can't parse the version, be lenient and accept it
                    return true;
                };

                req.matches(&ver)
            }
            // For non-semver specs (tags, catalogs, workspace), accept any version
            // since validation happens elsewhere
            _ => true,
        }
    }

    pub(super) fn nested_dependency_entry(
        &self,
        entry_key: &str,
        dependency: &str,
        version: &str,
    ) -> Option<(String, &PackageEntry)> {
        let direct_key = format!("{entry_key}/{dependency}");
        if let Some(entry) = self.data.packages.get(&direct_key) {
            return Some((direct_key, entry));
        }

        let mut search_key = entry_key;
        while let Some(slash_pos) = search_key.rfind('/') {
            search_key = &search_key[..slash_pos];
            let ancestor_key = format!("{search_key}/{dependency}");
            if let Some(entry) = self.data.packages.get(&ancestor_key) {
                if self.version_satisfies_spec(entry.version(), version) {
                    return Some((ancestor_key, entry));
                }
                break;
            }
        }

        None
    }

    /// Find a package version that satisfies the given version spec.
    ///
    /// Searches in order:
    /// 1. Workspace-scoped entries
    /// 2. Top-level entries
    /// 3. Nested/aliased entries (by searching all idents)
    pub(super) fn find_matching_version(
        &self,
        workspace_name: &str,
        name: &str,
        version_spec: &str,
        override_version: &str,
        resolved_version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        // When an override is active, the overridden version is authoritative
        // and should always be accepted regardless of the original version spec.
        // For example, if a package declares `"lightningcss": "1.30.2"` but the
        // root package.json overrides it to `"1.30.1"`, the resolved version
        // 1.30.1 must be accepted even though it doesn't satisfy `^1.30.2`.
        let has_override = override_version != resolved_version;

        // Try workspace-scoped first
        if let Some(entry) = self.index.get_workspace_scoped(workspace_name, name)
            && let Some(pkg) =
                self.process_package_entry(entry, name, override_version, resolved_version)?
            && (has_override || self.version_satisfies_spec(&pkg.version, version_spec))
        {
            return Ok(Some(pkg));
        }

        // Try hoisted/top-level
        if let Some((_key, entry)) = self.index.find_package(Some(workspace_name), name)
            && let Some(pkg) =
                self.process_package_entry(entry, name, override_version, resolved_version)?
            && (has_override || self.version_satisfies_spec(&pkg.version, version_spec))
        {
            return Ok(Some(pkg));
        }

        // Search for nested/aliased versions that match
        // Only search explicitly nested entries (with '/' in key), not bundled deps
        for (lockfile_key, entry) in &self.data.packages {
            // Only consider explicitly nested entries (not bundled)
            if !lockfile_key.contains('/') {
                continue;
            }

            // Skip bundled dependencies
            if let Some(info) = &entry.info
                && info
                    .other
                    .get("bundled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                continue;
            }

            let ident = PackageIdent::parse(&entry.ident);

            // Skip if the name doesn't match
            if ident.name() != name {
                continue;
            }

            // Skip workspace mappings
            if ident.is_workspace() {
                continue;
            }

            if let Some(pkg) =
                self.process_package_entry(entry, name, override_version, resolved_version)?
                && (has_override || self.version_satisfies_spec(&pkg.version, version_spec))
            {
                tracing::debug!(
                    "Found matching version {} for {} (spec: {}) in nested entry {}",
                    pkg.version,
                    name,
                    version_spec,
                    lockfile_key
                );
                return Ok(Some(pkg));
            }
        }

        Ok(None)
    }

    pub(super) fn apply_overrides<'a>(&'a self, name: &str, version: &'a str) -> &'a str {
        self.data
            .overrides
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or(version)
    }

    /// Resolves a catalog reference to the actual version
    /// Supports both default catalog ("catalog:") and named catalogs
    /// ("catalog:group:")
    pub(super) fn resolve_catalog_version(&self, name: &str, catalog_ref: &str) -> Option<&str> {
        if let Some(stripped) = catalog_ref.strip_prefix("catalog:") {
            if stripped.is_empty() {
                // Default catalog reference: "catalog:"
                self.data.catalog.get(name).map(|s| s.as_str())
            } else {
                // Named catalog reference: "catalog:group:"
                self.data
                    .catalogs
                    .get(stripped)
                    .and_then(|catalog| catalog.get(name).map(|s| s.as_str()))
            }
        } else {
            None
        }
    }

    // Given a specific key for a package, return the most specific key that is
    // present in the lockfile
    pub(super) fn package_entry(&self, key: &str) -> Option<(&str, &PackageEntry)> {
        let (key, entry) =
            PossibleKeyIter::new(key).find_map(|k| self.data.packages.get_key_value(k))?;
        Some((key, entry))
    }
}
