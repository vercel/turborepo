use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
};

use semver::Version;
use turbopath::RelativeUnixPathBuf;

use super::{BunLockfile, VersionSpec};
use crate::Lockfile;

impl Lockfile for BunLockfile {
    #[tracing::instrument(skip(self, workspace_path))]
    fn resolve_package(
        &self,
        workspace_path: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<crate::Package>, crate::Error> {
        let workspace_entry = self
            .data
            .workspaces
            .get(workspace_path)
            .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.into()))?;
        let workspace_name = &workspace_entry.name;

        // Parse version spec using structured type
        let version_spec = VersionSpec::parse(version);

        // Handle catalog references
        let resolved_version = if version_spec.is_catalog() {
            // Try to resolve catalog reference
            if let Some(catalog_version) = self.resolve_catalog_version(name, version) {
                catalog_version
            } else {
                // Catalog reference couldn't be resolved, return None
                return Ok(None);
            }
        } else {
            version
        };

        // Apply overrides to the resolved version if any exist for this package
        let override_version = self.apply_overrides(name, resolved_version);

        // V1 optimization: Check if this is a workspace dependency that can be resolved
        // directly from the workspaces section without requiring a packages entry
        if self.data.lockfile_version >= 1 {
            let override_spec = VersionSpec::parse(override_version);
            if let Some(workspace_target_path) = override_spec.workspace_path()
                && let Some(target_workspace) = self.data.workspaces.get(workspace_target_path)
            {
                // This is a workspace dependency, create a synthetic package entry
                let workspace_version = target_workspace.version.as_deref().unwrap_or("0.0.0");
                return Ok(Some(crate::Package {
                    key: format!("{name}@{workspace_version}"),
                    version: workspace_version.to_string(),
                }));
            }
        }

        // Find a package version that satisfies the version spec
        // This searches workspace-scoped, hoisted, and nested entries
        if let Some(pkg) = self.find_matching_version(
            workspace_name,
            name,
            version,
            override_version,
            resolved_version,
        )? {
            return Ok(Some(pkg));
        }

        Ok(None)
    }

    #[tracing::instrument(skip(self))]
    fn all_dependencies(
        &self,
        key: &str,
    ) -> Result<
        Option<std::borrow::Cow<'_, std::collections::BTreeMap<String, String>>>,
        crate::Error,
    > {
        let entry_key = self
            .key_to_entry
            .get(key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;
        let entry = self
            .data
            .packages
            .get(entry_key)
            .ok_or_else(|| crate::Error::MissingPackage(key.into()))?;

        let mut deps = std::collections::BTreeMap::new();

        let Some(info) = &entry.info else {
            return Ok(Some(std::borrow::Cow::Owned(deps)));
        };

        for (dependency, version) in info.all_dependencies() {
            // Bun resolves nested dependencies by walking up the parent chain
            // from the current package.
            let nested_entry = self
                .nested_dependency_entry(entry_key, dependency, version)
                .map(|(_, entry)| entry);

            let is_optional = info.optional_dependencies.contains_key(dependency)
                || info.optional_peers.contains(dependency);

            if is_optional {
                let has_nested = nested_entry.is_some();

                if !has_nested {
                    let is_optional_peer_only =
                        !info.optional_dependencies.contains_key(dependency);
                    let has_hoisted = self.data.packages.contains_key(dependency);

                    if is_optional_peer_only || !has_hoisted {
                        continue;
                    }
                }
            }

            // When a nested entry exists for this dependency (e.g.,
            // "chalk/ansi-styles/color-convert/color-name"), return an exact
            // version constraint so resolve_package matches the correct
            // nested version rather than a different nested entry that
            // happens to also satisfy the semver range.
            let resolved_version = if let Some(nested) = nested_entry {
                format!("={}", nested.version())
            } else {
                version.to_string()
            };

            deps.insert(dependency.to_string(), resolved_version);
        }

        Ok(Some(std::borrow::Cow::Owned(deps)))
    }

    fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<Box<dyn Lockfile>, crate::Error> {
        // Workspace mappings must be included in the packages list to ensure they're
        // found during pruning
        let mut packages_with_workspaces: std::collections::HashSet<String> =
            packages.iter().cloned().collect();

        if let Some(root_workspace) = self.data.workspaces.get("") {
            let mut root_deps = BTreeMap::new();
            if let Some(deps) = &root_workspace.dependencies {
                root_deps.extend(deps.clone());
            }
            if let Some(dev_deps) = &root_workspace.dev_dependencies {
                root_deps.extend(dev_deps.clone());
            }
            if let Some(optional_deps) = &root_workspace.optional_dependencies {
                root_deps.extend(optional_deps.clone());
            }
            if let Some(peer_deps) = &root_workspace.peer_dependencies {
                root_deps.extend(peer_deps.clone());
            }

            if !root_deps.is_empty() {
                let root_closures = crate::all_transitive_closures(
                    self,
                    Some(("".to_string(), root_deps)).into_iter().collect(),
                    true,
                )?;
                if let Some(root_closure) = root_closures.get("") {
                    packages_with_workspaces
                        .extend(root_closure.iter().map(|package| package.key.clone()));
                }
            }
        }

        let workspace_deps: HashMap<String, BTreeMap<String, String>> = workspace_packages
            .iter()
            .filter(|ws_path| !ws_path.is_empty())
            .filter_map(|ws_path| {
                let workspace_entry = self.data.workspaces.get(ws_path.as_str())?;
                let mut deps = BTreeMap::new();
                if let Some(d) = &workspace_entry.dependencies {
                    deps.extend(d.clone());
                }
                if let Some(dd) = &workspace_entry.dev_dependencies {
                    deps.extend(dd.clone());
                }
                if let Some(od) = &workspace_entry.optional_dependencies {
                    deps.extend(od.clone());
                }
                if let Some(pd) = &workspace_entry.peer_dependencies {
                    deps.extend(pd.clone());
                }
                (!deps.is_empty()).then(|| (ws_path.clone(), deps))
            })
            .collect();

        if !workspace_deps.is_empty() {
            let workspace_closures = crate::all_transitive_closures(self, workspace_deps, true)?;
            packages_with_workspaces.extend(
                workspace_closures
                    .values()
                    .flat_map(|closure| closure.iter().map(|package| package.key.clone())),
            );
        }

        for ws_path in workspace_packages {
            if ws_path.is_empty() {
                continue;
            }
            if let Some(workspace_entry) = self.data.workspaces.get(ws_path.as_str()) {
                packages_with_workspaces.insert(workspace_entry.name.clone());
            }
        }

        // Add workspace peer dependencies that are actually installed
        // Peer dependencies declared at workspace level are requirements, not automatic
        // dependencies, but if they're installed (exist in packages section), they
        // should be included in the pruned lockfile
        for ws_path in workspace_packages {
            if let Some(workspace_entry) = self.data.workspaces.get(ws_path.as_str())
                && let Some(peer_deps) = &workspace_entry.peer_dependencies
            {
                for peer_name in peer_deps.keys() {
                    // Check if this peer dependency exists as an installed package
                    if self.data.packages.contains_key(peer_name) {
                        packages_with_workspaces.insert(peer_name.clone());
                    }
                }
            }
        }

        // Also check root workspace peer dependencies
        if let Some(root_workspace) = self.data.workspaces.get("")
            && let Some(peer_deps) = &root_workspace.peer_dependencies
        {
            for peer_name in peer_deps.keys() {
                if self.data.packages.contains_key(peer_name) {
                    packages_with_workspaces.insert(peer_name.clone());
                }
            }
        }

        let packages_vec: Vec<String> = packages_with_workspaces.into_iter().collect();

        let subgraph = self.subgraph(workspace_packages, &packages_vec)?;
        Ok(Box::new(subgraph))
    }

    fn encode(&self) -> Result<Vec<u8>, crate::Error> {
        let mut output = String::new();
        self.write_header(&mut output);
        self.write_workspaces(&mut output)?;
        self.write_trusted_dependencies(&mut output)?;
        self.write_overrides(&mut output)?;
        self.write_catalogs(&mut output)?;
        self.write_packages(&mut output)?;
        self.write_patched_dependencies(&mut output)?;
        output.push_str("}\n");
        Ok(output.into_bytes())
    }

    fn patches(&self) -> Result<Vec<RelativeUnixPathBuf>, crate::Error> {
        let mut patches = self
            .data
            .patched_dependencies
            .values()
            .map(RelativeUnixPathBuf::new)
            .collect::<Result<Vec<_>, turbopath::PathError>>()?;
        patches.sort();
        Ok(patches)
    }

    fn global_change(&self, other: &dyn Lockfile) -> bool {
        let any_other = other as &dyn Any;
        let Some(other_bun) = any_other.downcast_ref::<Self>() else {
            return true;
        };

        self.data.lockfile_version != other_bun.data.lockfile_version
    }

    fn turbo_version(&self) -> Option<String> {
        let (_, entry) = self.package_entry("turbo")?;
        let version = entry.version();
        Version::parse(version).ok()?;
        Some(version.to_owned())
    }

    fn human_name(&self, package: &crate::Package) -> Option<String> {
        let entry = self.data.packages.get(&package.key)?;
        Some(entry.ident.clone())
    }
}
