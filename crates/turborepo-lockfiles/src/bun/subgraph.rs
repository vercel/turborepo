use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use semver::{Version, VersionReq};

use super::{
    BunLockfile, BunLockfileData, Error, Map, PackageEntry, PackageIdent, PackageIndex,
    PackageInfo, PackageKey, data::WorkspaceEntry,
};

impl BunLockfile {
    fn include_duplicate_alias_children(&self, pruned_data: &mut BunLockfileData) {
        loop {
            let mut keys_by_ident: HashMap<String, Vec<String>> = HashMap::new();
            for (key, entry) in &pruned_data.packages {
                if PackageIdent::parse(&entry.ident).is_workspace() {
                    continue;
                }
                keys_by_ident
                    .entry(entry.ident.clone())
                    .or_default()
                    .push(key.clone());
            }

            let current_keys: std::collections::HashSet<String> =
                pruned_data.packages.keys().cloned().collect();
            let mut additions = BTreeMap::new();

            for keys in keys_by_ident.values().filter(|keys| keys.len() > 1) {
                let mut child_suffixes = BTreeSet::new();
                for key in keys {
                    let prefix = format!("{key}/");
                    child_suffixes.extend(
                        current_keys
                            .iter()
                            .filter_map(|current_key| current_key.strip_prefix(&prefix))
                            .map(ToString::to_string),
                    );
                }

                for key in keys {
                    for suffix in &child_suffixes {
                        let child_key = format!("{key}/{suffix}");
                        if current_keys.contains(&child_key) || additions.contains_key(&child_key) {
                            continue;
                        }
                        if let Some(entry) = pruned_data.packages.get(&child_key) {
                            additions.insert(child_key, entry.clone());
                        }
                    }
                }
            }

            if additions.is_empty() {
                break;
            }

            pruned_data.packages.extend(additions);
        }
    }

    pub fn lockfile(self) -> Result<BunLockfileData, Error> {
        Ok(self.data)
    }

    pub(super) fn subgraph(
        &self,
        workspace_packages: &[String],
        packages: &[String],
    ) -> Result<BunLockfile, Error> {
        // Create pruned lockfile structure
        let mut pruned_data = BunLockfileData {
            lockfile_version: self.data.lockfile_version,
            config_version: self.data.config_version,
            workspaces: Map::new(),
            // trustedDependencies are intentionally left empty. turbo prune
            // copies the root package.json which is the source of truth for
            // trusted scripts; bun re-derives the set at install time.
            trusted_dependencies: Vec::new(),
            overrides: Map::new(),
            catalog: self.data.catalog.clone(),
            catalogs: self.data.catalogs.clone(),
            packages: Map::new(),
            patched_dependencies: Map::new(),
        };

        if let Some(root) = self.data.workspaces.get("") {
            pruned_data.workspaces.insert("".to_string(), root.clone());
        }

        for ws_path in workspace_packages {
            if let Some(entry) = self.data.workspaces.get(ws_path) {
                pruned_data
                    .workspaces
                    .insert(ws_path.clone(), entry.clone());
            }
        }

        let mut keys_to_include = HashSet::new();

        let target_workspace_names: HashSet<String> = workspace_packages
            .iter()
            .filter_map(|ws_path| self.data.workspaces.get(ws_path).map(|ws| ws.name.clone()))
            .collect();

        // When idents map to multiple lockfile keys, only include workspace-specific
        // entries for target workspaces to avoid pulling in unrelated workspace
        // versions
        for pkg in packages {
            if self.data.packages.contains_key(pkg)
                || pruned_data.workspaces.values().any(|ws| &ws.name == pkg)
            {
                keys_to_include.insert(pkg.clone());
            } else if let Some(at_pos) = pkg.rfind('@') {
                let name = &pkg[..at_pos];

                if let Some(entry) = self.data.packages.get(name)
                    && entry.ident.contains("@workspace:")
                {
                    keys_to_include.insert(name.to_string());
                    // Continue to also find package entries with this ident
                    // (e.g., both "storybook" workspace mapping and
                    // "storybook/storybook")
                }

                for (lockfile_key, entry) in &self.data.packages {
                    if &entry.ident != pkg {
                        continue;
                    }

                    if let Some(slash_pos) = lockfile_key.find('/') {
                        let prefix = &lockfile_key[..slash_pos];

                        let is_workspace_prefix =
                            self.data.workspaces.values().any(|ws| ws.name == prefix);

                        if is_workspace_prefix {
                            if target_workspace_names.contains(prefix) {
                                keys_to_include.insert(lockfile_key.clone());
                            }
                        } else {
                            keys_to_include.insert(lockfile_key.clone());
                        }
                    } else {
                        keys_to_include.insert(lockfile_key.clone());
                    }
                }
            } else {
                for (lockfile_key, entry) in &self.data.packages {
                    if &entry.ident != pkg {
                        continue;
                    }

                    if let Some(slash_pos) = lockfile_key.find('/') {
                        let prefix = &lockfile_key[..slash_pos];

                        let is_workspace_prefix =
                            self.data.workspaces.values().any(|ws| ws.name == prefix);

                        if is_workspace_prefix {
                            if target_workspace_names.contains(prefix) {
                                keys_to_include.insert(lockfile_key.clone());
                            }
                        } else {
                            keys_to_include.insert(lockfile_key.clone());
                        }
                    } else {
                        keys_to_include.insert(lockfile_key.clone());
                    }
                }
            }
        }

        // De-alias workspace-specific keys (e.g., "blog/@types/react" ->
        // "@types/react") so peer dependencies resolve correctly in pruned
        // lockfiles
        let should_dealias = !workspace_packages.is_empty();

        let mut dealias_set: std::collections::HashSet<String> = std::collections::HashSet::new();
        if should_dealias {
            for key in &keys_to_include {
                let parsed_key = PackageKey::parse(key);

                // Only nested keys can be dealiased
                if let Some(parent) = parsed_key.parent() {
                    // Check if this is nested under a target workspace
                    if target_workspace_names.contains(&parent) {
                        // Get the dealiased version
                        if let Some(dealiased_key) = parsed_key.dealias() {
                            let dealiased_str = dealiased_key.to_string();

                            // Check if dealiasing would conflict with an existing package.
                            // If the top-level key points at a different ident, keep the
                            // nested key so both versions remain addressable.
                            let would_conflict = if let Some(existing_entry) =
                                self.data.packages.get(&dealiased_str)
                            {
                                let ident = PackageIdent::parse(&existing_entry.ident);
                                let current_entry = self.data.packages.get(key);
                                ident.is_workspace()
                                    || current_entry
                                        .map(|entry| entry.ident != existing_entry.ident)
                                        .unwrap_or(false)
                            } else {
                                false
                            };

                            if !would_conflict {
                                dealias_set.insert(dealiased_str);
                            }
                        }
                    }
                }
            }
        }

        let mut sorted_keys: Vec<_> = keys_to_include.iter().collect();
        sorted_keys.sort();

        let mut renamed_prefixes: Vec<(String, String)> = Vec::new();

        for key in sorted_keys {
            if let Some(entry) = self.data.packages.get(key) {
                let mut pruned_key = if should_dealias {
                    let parsed_key = PackageKey::parse(key);

                    // Check if this is a nested key that could be dealiased
                    if let Some(parent) = parsed_key.parent() {
                        let is_target_workspace_prefix = target_workspace_names.contains(&parent);

                        if is_target_workspace_prefix {
                            // Try to dealias
                            if let Some(dealiased_key) = parsed_key.dealias() {
                                let dealiased_str = dealiased_key.to_string();

                                // Check if dealiasing would conflict with an existing package.
                                // Different idents must keep distinct keys.
                                if let Some(existing_entry) = self.data.packages.get(&dealiased_str)
                                {
                                    let ident = PackageIdent::parse(&existing_entry.ident);
                                    if ident.is_workspace() || entry.ident != existing_entry.ident {
                                        // This would conflict with another package - keep full key.
                                        key.clone()
                                    } else {
                                        // No conflict - safe to dealias
                                        dealiased_str
                                    }
                                } else {
                                    // No existing entry - safe to dealias
                                    dealiased_str
                                }
                            } else {
                                // Cannot dealias
                                key.clone()
                            }
                        } else {
                            // Keep the key as-is (it's nested under a package, not a workspace)
                            key.clone()
                        }
                    } else {
                        // No slash - this is a top-level entry
                        // Check if a workspace-scoped version will be de-aliased to this same key
                        if dealias_set.contains(key) {
                            // Skip this top-level entry - it conflicts with a workspace-scoped
                            // version
                            continue;
                        }
                        key.clone()
                    }
                } else {
                    // Not dealiasing - keep key as-is
                    key.clone()
                };

                if let Some((old_prefix, new_prefix)) = renamed_prefixes
                    .iter()
                    .filter(|(old_prefix, _)| key.starts_with(old_prefix))
                    .max_by_key(|(old_prefix, _)| old_prefix.len())
                {
                    pruned_key = format!("{}{}", new_prefix, &key[old_prefix.len()..]);
                }

                if pruned_key != *key {
                    renamed_prefixes.push((format!("{key}/"), format!("{pruned_key}/")));
                }

                // Check if this is a workspace mapping entry (e.g., "storybook":
                // ["storybook@workspace:apps/storybook"])
                let ident = PackageIdent::parse(&entry.ident);
                let is_workspace_mapping = ident.is_workspace() && ident.name() == key;

                // Handle workspace mapping entries
                if is_workspace_mapping {
                    // Extract the workspace path from the mapping
                    // Format: "storybook@workspace:apps/storybook" -> workspace path is
                    // "apps/storybook"
                    if let Some(workspace_path) = ident.workspace_path() {
                        // Ensure transitive workspace dependencies are in the
                        // pruned set. The initial pruned_data.workspaces only
                        // contains the root and target workspaces, but
                        // workspaces depended on transitively must also be
                        // included.
                        if !pruned_data.workspaces.contains_key(workspace_path)
                            && let Some(ws_entry) = self.data.workspaces.get(workspace_path)
                        {
                            pruned_data
                                .workspaces
                                .insert(workspace_path.to_string(), ws_entry.clone());
                        }

                        // Check if this workspace is in the pruned set
                        if pruned_data.workspaces.contains_key(workspace_path) {
                            // This workspace IS in the pruned set - keep the mapping as-is
                            pruned_data
                                .packages
                                .insert(pruned_key.clone(), entry.clone());
                            continue;
                        }

                        // This workspace is NOT in the pruned set (doesn't
                        // exist in the original data either). Try to find the
                        // actual npm package entry instead.
                        // Get the workspace name (last component of path)
                        let workspace_name = workspace_path
                            .split('/')
                            .next_back()
                            .unwrap_or(workspace_path);

                        // Look for the actual package entry stored with workspace-scoped key
                        // e.g., "storybook/storybook" for workspace "storybook"
                        let scoped_key = format!("{workspace_name}/{key}");

                        if let Some(actual_package) = self.data.packages.get(&scoped_key) {
                            // Include the actual package entry with the unscoped key
                            pruned_data
                                .packages
                                .insert(pruned_key.clone(), actual_package.clone());
                        }
                    }

                    // Skip the workspace mapping entry itself
                    continue;
                }

                pruned_data
                    .packages
                    .insert(pruned_key.clone(), entry.clone());

                if pruned_key != *key {
                    let old_prefix = format!("{key}/");
                    let new_prefix = format!("{pruned_key}/");
                    for (descendant_key, descendant_entry) in &self.data.packages {
                        if descendant_key.starts_with(&old_prefix) {
                            let renamed_key =
                                format!("{}{}", new_prefix, &descendant_key[old_prefix.len()..]);
                            pruned_data
                                .packages
                                .insert(renamed_key, descendant_entry.clone());
                        }
                    }
                }

                // Check if this package references a workspace (e.g., via @workspace: in ident)
                // and ensure that workspace is included
                let package_ident = PackageIdent::parse(&entry.ident);
                if let Some(workspace_path) = package_ident.workspace_path() {
                    // Add this workspace if not already included
                    if !pruned_data.workspaces.contains_key(workspace_path)
                        && let Some(ws_entry) = self.data.workspaces.get(workspace_path)
                    {
                        pruned_data
                            .workspaces
                            .insert(workspace_path.to_string(), ws_entry.clone());
                    }
                }

                // Include bundled dependencies
                // Bundled dependencies are stored with nested keys like "parent/dep"
                // and have "bundled": true in their info
                // Note: We search using the original key from the source lockfile
                let bundled_prefix = format!("{key}/");
                for (lockfile_key, bundled_entry) in &self.data.packages {
                    if lockfile_key.starts_with(&bundled_prefix)
                        && let Some(bundled_info) = &bundled_entry.info
                        && bundled_info
                            .other
                            .get("bundled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    {
                        // Check if this is a bundled dependency
                        // In Bun's format, bundled is indicated by the "bundled" field
                        // Bundled deps are always nested under their parent,
                        // so we need to adjust the key if we dealiased the parent
                        let bundled_pruned_key =
                            if should_dealias && lockfile_key.starts_with(&bundled_prefix) {
                                // Replace the parent prefix with the dealiased version
                                format!("{}{}", pruned_key, &lockfile_key[key.len()..])
                            } else {
                                lockfile_key.clone()
                            };
                        pruned_data
                            .packages
                            .insert(bundled_pruned_key, bundled_entry.clone());
                    }
                }
            } else {
                // Key doesn't exist in original lockfile - check if it's a workspace name
                // and create a workspace mapping entry for it
                if let Some((ws_path, ws_entry)) = pruned_data
                    .workspaces
                    .iter()
                    .find(|(_, ws)| &ws.name == key)
                {
                    // Skip root workspace
                    if !ws_path.is_empty() {
                        let ident = format!("{key}@workspace:{ws_path}");
                        let info = PackageInfo {
                            dependencies: ws_entry.dependencies.clone().unwrap_or_default(),
                            dev_dependencies: ws_entry.dev_dependencies.clone().unwrap_or_default(),
                            optional_dependencies: ws_entry
                                .optional_dependencies
                                .clone()
                                .unwrap_or_default(),
                            peer_dependencies: ws_entry
                                .peer_dependencies
                                .clone()
                                .unwrap_or_default(),
                            optional_peers: ws_entry
                                .optional_peers
                                .as_ref()
                                .map(|v| v.iter().cloned().collect())
                                .unwrap_or_default(),
                            ..Default::default()
                        };
                        let entry = PackageEntry {
                            ident,
                            registry: None,
                            info: Some(info),
                            checksum: None,
                            root: None,
                        };
                        pruned_data.packages.insert(key.clone(), entry);
                    }
                }
            }
        }

        // Preserve ALL overrides from the original lockfile. turbo prune copies
        // the root package.json with all overrides intact, so the lockfile must
        // keep them in sync. If overrides are selectively stripped here, bun
        // detects the mismatch with package.json and re-resolves, which breaks
        // `bun install --frozen-lockfile` when a range has drifted (e.g. "latest").
        pruned_data.overrides = self.data.overrides.clone();

        // ORPHAN REMOVAL: After de-aliasing, some packages may be orphaned
        // (only depended on by packages that were skipped due to de-aliasing
        // conflicts). We need to remove these orphaned packages.
        //
        // Strategy: Recompute which packages are reachable from workspace dependencies
        // using the pruned packages. Packages not reachable are orphans.

        // Build key_to_entry for closure computation
        let mut temp_key_to_entry: HashMap<String, String> = HashMap::new();
        for (path, entry) in &pruned_data.packages {
            // Take first occurrence for duplicate idents (shouldn't happen after
            // de-aliasing)
            temp_key_to_entry
                .entry(entry.ident.clone())
                .or_insert(path.clone());
        }

        // Create temporary lockfile for recomputation
        let temp_data = BunLockfileData {
            lockfile_version: pruned_data.lockfile_version,
            config_version: pruned_data.config_version,
            workspaces: pruned_data.workspaces.clone(),
            trusted_dependencies: pruned_data.trusted_dependencies.clone(),
            overrides: pruned_data.overrides.clone(),
            catalog: self.data.catalog.clone(),
            catalogs: self.data.catalogs.clone(),
            packages: pruned_data.packages.clone(),
            patched_dependencies: pruned_data.patched_dependencies.clone(),
        };
        let temp_index = PackageIndex::new(&temp_data.packages);
        let temp_lockfile = BunLockfile {
            data: temp_data,
            key_to_entry: temp_key_to_entry,
            index: temp_index,
        };

        // Collect workspace dependencies
        let workspace_deps: HashMap<String, BTreeMap<String, String>> = pruned_data
            .workspaces
            .iter()
            .map(|(ws_path, ws_entry)| {
                let mut deps = BTreeMap::new();
                if let Some(d) = &ws_entry.dependencies {
                    deps.extend(d.clone());
                }
                if let Some(dd) = &ws_entry.dev_dependencies {
                    deps.extend(dd.clone());
                }
                if let Some(od) = &ws_entry.optional_dependencies {
                    deps.extend(od.clone());
                }
                // Include peer dependencies for orphan removal computation
                // Peer dependencies that are actually installed should not be considered
                // orphans
                if let Some(pd) = &ws_entry.peer_dependencies {
                    deps.extend(pd.clone());
                }
                (ws_path.clone(), deps)
            })
            .collect();

        // Recompute transitive closure
        match crate::all_transitive_closures(&temp_lockfile, workspace_deps, true) {
            Ok(recomputed_closures) => {
                let reachable_idents: HashSet<String> = recomputed_closures
                    .values()
                    .flat_map(|closure| closure.iter().map(|p| p.key.clone()))
                    .collect();

                // Also keep track of reachable lockfile keys for nested package detection
                let reachable_lockfile_keys: HashSet<String> = recomputed_closures
                    .values()
                    .flat_map(|closure| {
                        closure
                            .iter()
                            .filter_map(|p| temp_lockfile.key_to_entry.get(&p.key).cloned())
                    })
                    .collect();

                let pruned_workspace_names: HashSet<String> = pruned_data
                    .workspaces
                    .values()
                    .map(|workspace| workspace.name.clone())
                    .collect();

                pruned_data.packages.retain(|key, entry| {
                    if entry.ident.contains("@workspace:") {
                        return true;
                    }

                    let parsed = PackageKey::parse(key);
                    if let Some(parent) = parsed.parent() {
                        reachable_idents.contains(&entry.ident)
                            && (reachable_lockfile_keys.contains(&parent)
                                || pruned_workspace_names.contains(&parent))
                    } else {
                        reachable_idents.contains(&entry.ident)
                    }
                });

                loop {
                    let current_keys: HashSet<String> =
                        pruned_data.packages.keys().cloned().collect();
                    let before = current_keys.len();
                    pruned_data.packages.retain(|key, _entry| {
                        let parsed = PackageKey::parse(key);
                        match parsed.parent() {
                            Some(parent) => current_keys.contains(&parent),
                            None => true,
                        }
                    });
                    if pruned_data.packages.len() == before {
                        break;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to recompute transitive closures for orphan removal: {e}");
            }
        }

        // NESTED KEY PROMOTION: In bun lockfiles, nested entries like
        // "accepts/negotiator" only exist as overrides of a hoisted top-level
        // entry ("negotiator"). When pruning removes the hoisted version but
        // keeps a nested version, we must promote the nested entry to top-level
        // to maintain a valid lockfile structure that bun's --frozen-lockfile
        // accepts.
        //
        // Restore patched hoisted entries first so promotion does not replace a
        // still-required patched version with an unrelated nested alternative.
        // See https://github.com/vercel/turborepo/issues/13101
        let required_patched_idents =
            self.workspace_required_patched_idents(&pruned_data.workspaces);
        self.restore_patched_hoisted_entries(&mut pruned_data, &required_patched_idents);

        loop {
            let top_level_pkg_names: HashSet<String> = pruned_data
                .packages
                .iter()
                .filter(|(key, _)| PackageKey::parse(key).parent().is_none())
                .filter_map(|(_, entry)| {
                    let ident = PackageIdent::parse(&entry.ident);
                    if !ident.is_workspace() {
                        Some(ident.name().to_string())
                    } else {
                        None
                    }
                })
                .collect();

            // Find the first nested entry (by sorted key order) whose package
            // name has no top-level entry. We promote one package per iteration
            // because promotion renames children, which may expose further
            // entries that need promotion.
            let mut sorted_pkg_keys: Vec<_> = pruned_data.packages.keys().cloned().collect();
            sorted_pkg_keys.sort();

            let mut promote_target: Option<(String, String)> = None; // (pkg_name, old_key)
            for key in &sorted_pkg_keys {
                if let Some(parent) = PackageKey::parse(key).parent() {
                    if (parent == "npm" || parent.starts_with("npm/"))
                        && pruned_data.packages.contains_key(&parent)
                    {
                        continue;
                    }
                } else {
                    continue;
                }

                let entry = &pruned_data.packages[key];
                let ident = PackageIdent::parse(&entry.ident);
                if ident.is_workspace() {
                    continue;
                }

                let pkg_name = ident.name().to_string();
                if required_patched_idents
                    .iter()
                    .any(|patched_ident| PackageIdent::parse(patched_ident).name() == pkg_name)
                {
                    continue;
                }
                if !top_level_pkg_names.contains(&pkg_name) {
                    promote_target = Some((pkg_name, key.clone()));
                    break;
                }
            }

            let Some((pkg_name, old_key)) = promote_target else {
                break;
            };

            // Rename the entry and all its children
            let old_prefix = format!("{old_key}/");
            let new_prefix = format!("{pkg_name}/");
            let mut renames: Vec<(String, String)> = vec![(old_key, pkg_name)];
            for key in &sorted_pkg_keys {
                if key.starts_with(&old_prefix) {
                    let new_key = format!("{new_prefix}{}", &key[old_prefix.len()..]);
                    renames.push((key.clone(), new_key));
                }
            }

            for (old, new) in renames {
                if let Some(entry) = pruned_data.packages.remove(&old) {
                    pruned_data.packages.insert(new, entry);
                }
            }
        }

        // After pruning, some packages may have required peer dependencies that
        // are no longer present in the pruned set (e.g. expo-network is provided
        // by mobile workspaces that were pruned away). Bun refuses to parse a
        // lockfile with unresolvable required peers, so we move them to
        // optionalPeers.
        let pruned_package_names: HashSet<String> = pruned_data.packages.keys().cloned().collect();
        for entry in pruned_data.packages.values_mut() {
            if let Some(info) = &mut entry.info {
                if info.peer_dependencies.is_empty() {
                    continue;
                }
                let missing_peers: Vec<String> = info
                    .peer_dependencies
                    .keys()
                    .filter(|peer_name| {
                        !info.optional_peers.contains(*peer_name)
                            && !pruned_package_names.contains(*peer_name)
                    })
                    .cloned()
                    .collect();
                for peer_name in missing_peers {
                    info.optional_peers.insert(peer_name);
                }
            }
        }

        // SUFFIXED KEY RENAME: Bun appends suffixes like `--for-generate-function-map`
        // to distinguish multiple versions of the same package. When the primary
        // (unsuffixed) entry is pruned and only the suffixed one remains, rename
        // the key to the base package name so bun can resolve it.
        {
            let top_level_pkg_names: HashSet<String> = pruned_data
                .packages
                .iter()
                .filter(|(key, _)| !key.contains("--") && PackageKey::parse(key).parent().is_none())
                .map(|(_, entry)| PackageIdent::parse(&entry.ident).name().to_string())
                .collect();

            let mut renames: Vec<(String, String)> = Vec::new();
            for (key, entry) in &pruned_data.packages {
                if !key.contains("--") {
                    continue;
                }
                let parsed = PackageKey::parse(key);
                if parsed.parent().is_some() {
                    continue;
                }
                let ident = PackageIdent::parse(&entry.ident);
                let pkg_name = ident.name();
                if key != pkg_name && !top_level_pkg_names.contains(pkg_name) {
                    renames.push((key.clone(), pkg_name.to_string()));
                }
            }
            for (old_key, new_key) in renames {
                if let Some(entry) = pruned_data.packages.remove(&old_key) {
                    pruned_data.packages.insert(new_key, entry);
                }
            }
        }

        // NESTED ENTRY DEDUPLICATION: In bun lockfiles, nested entries like
        // `@babel/core/@babel/traverse` only need to exist when they resolve to
        // a DIFFERENT version than the hoisted top-level `@babel/traverse`.
        // After pruning removes some versions, nested entries that now match
        // the hoisted entry are redundant and must be removed — bun's
        // --frozen-lockfile rejects them.
        {
            let hoisted_idents: HashMap<String, String> = pruned_data
                .packages
                .iter()
                .filter(|(key, entry)| {
                    let parsed = PackageKey::parse(key);
                    if parsed.parent().is_some() {
                        return false;
                    }
                    let ident = PackageIdent::parse(&entry.ident);
                    let pkg_name = ident.name();
                    *key == pkg_name
                })
                .map(|(_, entry)| {
                    let ident = PackageIdent::parse(&entry.ident);
                    (ident.name().to_string(), entry.ident.clone())
                })
                .collect();

            // A nested entry that matches the hoisted top-level is redundant ONLY
            // if removing it would not change resolution. Bun resolves a nested
            // package by walking up parent scopes (nearest ancestor wins). If an
            // INTERMEDIATE ancestor scope pins a different version of the same
            // package, the nested entry shadows it and must be kept — otherwise
            // bun resolves to the intermediate (wrong) version instead of the
            // hoisted one. See vercel/turborepo#12962 follow-up: a 3-level split
            // such as `@vite-pwa/nuxt/@nuxt/kit/pathe@2` sitting above
            // `@vite-pwa/nuxt/pathe@1` was incorrectly dropped.
            let redundant_keys: Vec<String> = pruned_data
                .packages
                .iter()
                .filter_map(|(key, entry)| {
                    PackageKey::parse(key).parent()?;
                    let ident = PackageIdent::parse(&entry.ident);
                    if ident.is_workspace() {
                        return None;
                    }
                    let name = ident.name();
                    // Only a candidate if it matches the hoisted top-level ident.
                    match hoisted_idents.get(name) {
                        Some(hoisted_ident) if hoisted_ident == &entry.ident => {}
                        _ => return None,
                    }
                    let suffix = format!("/{name}");
                    let parent_scope = key.strip_suffix(&suffix)?;
                    // Find the nearest ancestor scope (longest strict prefix-scope
                    // of parent_scope) that also provides `name`.
                    let nearest = pruned_data
                        .packages
                        .iter()
                        .filter_map(|(other_key, other_entry)| {
                            let anc = other_key.strip_suffix(&suffix)?;
                            let descends =
                                parent_scope == anc || parent_scope.starts_with(&format!("{anc}/"));
                            (anc.len() < parent_scope.len() && descends)
                                .then_some((anc.len(), &other_entry.ident))
                        })
                        .max_by_key(|(len, _)| *len)
                        .map(|(_, ident)| ident);
                    // Without this entry, resolution falls to the nearest ancestor
                    // (or the hoisted top-level when none). Redundant only if that
                    // resolves to the same ident.
                    let resolves_to = nearest.unwrap_or(&entry.ident);
                    (resolves_to == &entry.ident).then(|| key.clone())
                })
                .collect();
            for key in redundant_keys {
                pruned_data.packages.remove(&key);
            }
        }

        // De-aliasing can turn a workspace-scoped package key like
        // `@repo/app/pkg` into `pkg`. Its nested resolutions need to remain
        // addressable from the new key, otherwise bun treats required deps as
        // missing during frozen installs.
        loop {
            let package_snapshot: Vec<(String, PackageEntry)> = pruned_data
                .packages
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone()))
                .collect();
            let mut additions = Vec::new();

            for (key, entry) in package_snapshot {
                let Some(info) = &entry.info else {
                    continue;
                };

                for (dep_name, dep_version) in info
                    .dependencies
                    .iter()
                    .chain(info.optional_dependencies.iter())
                {
                    let nested_key = format!("{key}/{dep_name}");
                    let top_level_satisfies =
                        pruned_data.packages.get(dep_name).is_some_and(|entry| {
                            self.version_satisfies_spec(entry.version(), dep_version)
                        });
                    let nested_satisfies =
                        pruned_data.packages.get(&nested_key).is_some_and(|entry| {
                            self.version_satisfies_spec(entry.version(), dep_version)
                        });
                    // Bun resolution walks up from the dependent's key, so an
                    // entry in an ancestor scope also satisfies the dependency.
                    let ancestor_satisfies = || {
                        let mut scope = PackageKey::parse(&key).parent();
                        while let Some(parent) = scope {
                            if let Some(entry) =
                                pruned_data.packages.get(&format!("{parent}/{dep_name}"))
                            {
                                return self.version_satisfies_spec(entry.version(), dep_version);
                            }
                            scope = PackageKey::parse(&parent).parent();
                        }
                        false
                    };
                    if top_level_satisfies || nested_satisfies || ancestor_satisfies() {
                        continue;
                    }

                    let exact_source = self
                        .data
                        .packages
                        .get_key_value(&key)
                        .filter(|(_, source_entry)| source_entry.ident == entry.ident)
                        .and_then(|(source_key, _)| {
                            self.nested_dependency_entry(source_key, dep_name, dep_version)
                                .map(|(dep_key, dep_entry)| (dep_key, dep_entry, source_key))
                        });

                    let ident_source = || {
                        self.data
                            .packages
                            .iter()
                            .filter(|(source_key, source_entry)| {
                                source_key.as_str() != key && source_entry.ident == entry.ident
                            })
                            .find_map(|(source_key, _)| {
                                self.nested_dependency_entry(source_key, dep_name, dep_version)
                                    .map(|(dep_key, dep_entry)| (dep_key, dep_entry, source_key))
                            })
                    };

                    let Some((source_dep_key, source_dep_entry, source_parent_key)) =
                        exact_source.or_else(ident_source)
                    else {
                        continue;
                    };

                    let source_prefix = format!("{source_parent_key}/");
                    let pruned_dep_key =
                        if let Some(suffix) = source_dep_key.strip_prefix(&source_prefix) {
                            format!("{key}/{suffix}")
                        } else {
                            // The entry was found in an ancestor scope of the source
                            // key, so its key can't be re-parented under the
                            // dependent's pruned key. Bun resolves dependencies by
                            // walking up the dependent's scope chain by name, so the
                            // verbatim key is fine as long as SOME entry named
                            // dep_name is reachable from the dependent. When none is
                            // (the dependent was renamed by de-aliasing/promotion and
                            // the ancestor chain was pruned), bun fails to parse the
                            // lockfile; nest the entry directly under the dependent
                            // instead. See
                            // https://github.com/vercel/turborepo/issues/13233
                            let name_resolvable = pruned_data.packages.contains_key(dep_name) || {
                                let mut resolvable = false;
                                let mut scope = Some(key.clone());
                                while let Some(scope_key) = scope {
                                    if pruned_data
                                        .packages
                                        .contains_key(&format!("{scope_key}/{dep_name}"))
                                    {
                                        resolvable = true;
                                        break;
                                    }
                                    scope = PackageKey::parse(&scope_key).parent();
                                }
                                resolvable
                            };
                            if name_resolvable {
                                source_dep_key
                            } else {
                                format!("{key}/{dep_name}")
                            }
                        };

                    if pruned_data.packages.contains_key(&pruned_dep_key) {
                        continue;
                    }

                    additions.push((pruned_dep_key, source_dep_entry.clone()));
                }
            }

            if additions.is_empty() {
                break;
            }

            for (key, entry) in additions {
                pruned_data.packages.insert(key, entry);
            }
        }

        // Alias keys can share the same package ident, e.g. `string-width` and
        // `string-width-cjs`. Bun expects their nested dependency sets to stay in
        // sync; keeping only one sibling makes `bun install --frozen-lockfile`
        // rewrite the pruned lockfile.
        self.include_duplicate_alias_children(&mut pruned_data);

        self.preserve_patched_dependencies(&mut pruned_data);

        // Rebuild key_to_entry HashMap for the pruned lockfile
        let mut key_to_entry: HashMap<String, String> =
            HashMap::with_capacity(pruned_data.packages.len());
        for (path, entry) in pruned_data.packages.iter() {
            if let Some(prev_path) = key_to_entry.insert(entry.ident.clone(), path.clone()) {
                let Some(prev_entry) = pruned_data.packages.get(&prev_path) else {
                    continue;
                };

                // Verify checksums match for duplicate idents
                if prev_entry.checksum != entry.checksum {
                    return Err(Error::MismatchedShas {
                        ident: entry.ident.clone(),
                        sha1: prev_entry.checksum.clone().unwrap_or_default(),
                        sha2: entry.checksum.clone().unwrap_or_default(),
                    });
                }
            }
        }

        // Build package index for pruned data
        let index = PackageIndex::new(&pruned_data.packages);

        Ok(BunLockfile {
            data: pruned_data,
            key_to_entry,
            index,
        })
    }

    fn workspace_required_patched_idents(
        &self,
        workspaces: &Map<String, WorkspaceEntry>,
    ) -> HashSet<String> {
        let mut required = HashSet::new();
        for workspace in workspaces.values() {
            for deps in [
                workspace.dependencies.as_ref(),
                workspace.dev_dependencies.as_ref(),
                workspace.optional_dependencies.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                for (name, version) in deps {
                    if version.contains("workspace:") {
                        continue;
                    }
                    for patched_ident in self.data.patched_dependencies.keys() {
                        if self.patched_ident_satisfies_dep(patched_ident, name, version) {
                            required.insert(patched_ident.clone());
                        }
                    }
                }
            }
        }
        required
    }

    fn patched_ident_satisfies_dep(&self, patched_ident: &str, name: &str, version: &str) -> bool {
        let parsed_ident = PackageIdent::parse(patched_ident);
        if parsed_ident.name() != name {
            return false;
        }

        let Some((_, patched_version)) = patched_ident.rsplit_once('@') else {
            return false;
        };

        let version = self
            .resolve_catalog_version(name, version)
            .unwrap_or(version);
        let version = self.apply_overrides(name, version);

        if let Some(exact_version) = version.strip_prefix('=') {
            return patched_version == exact_version;
        }
        if Version::parse(version).is_ok() {
            return patched_version == version;
        }

        let Ok(req) = VersionReq::parse(version) else {
            return false;
        };
        let Ok(patched_version) = Version::parse(patched_version) else {
            return false;
        };

        req.matches(&patched_version)
    }

    fn restore_patched_hoisted_entries(
        &self,
        pruned_data: &mut BunLockfileData,
        required_patched_idents: &HashSet<String>,
    ) {
        for ident in required_patched_idents {
            let pkg_name = PackageIdent::parse(ident).name().to_string();
            let hoisted_correct = pruned_data
                .packages
                .get(&pkg_name)
                .is_some_and(|entry| entry.ident == *ident);
            if hoisted_correct {
                continue;
            }

            if let Some(entry) = self.data.packages.get(&pkg_name)
                && entry.ident == *ident
            {
                pruned_data.packages.insert(pkg_name.clone(), entry.clone());
                let hoisted_prefix = format!("{pkg_name}/");
                for (key, child) in &self.data.packages {
                    if key.starts_with(&hoisted_prefix) {
                        pruned_data.packages.insert(key.clone(), child.clone());
                    }
                }
            }
        }
    }

    fn preserve_patched_dependencies(&self, pruned_data: &mut BunLockfileData) {
        let required_patched_idents =
            self.workspace_required_patched_idents(&pruned_data.workspaces);
        self.restore_patched_hoisted_entries(pruned_data, &required_patched_idents);

        let included_idents: HashSet<&str> = pruned_data
            .packages
            .values()
            .map(|entry| entry.ident.as_str())
            .collect();

        pruned_data.patched_dependencies.clear();
        for (pkg_ident, patch_path) in &self.data.patched_dependencies {
            if required_patched_idents.contains(pkg_ident)
                || included_idents.contains(pkg_ident.as_str())
            {
                pruned_data
                    .patched_dependencies
                    .insert(pkg_ident.clone(), patch_path.clone());
            }
        }
    }
}
