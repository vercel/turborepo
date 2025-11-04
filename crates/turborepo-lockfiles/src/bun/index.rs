//! Package indexing for efficient lockfile lookups.

use std::collections::HashMap;

use super::{PackageEntry, types::PackageKey};

#[derive(Debug, Clone)]
pub struct PackageIndex {
    /// Direct lookup by lockfile key (e.g., "lodash", "parent/dep")
    by_key: HashMap<String, PackageEntry>,

    /// Lookup by ident (e.g., "lodash@4.17.21")
    /// Maps ident -> lockfile key
    /// Multiple keys may map to the same ident (nested versions)
    by_ident: HashMap<String, Vec<String>>,

    /// Workspace-scoped lookup for quick resolution
    /// Maps (workspace_name, package_name) -> lockfile key
    workspace_scoped: HashMap<(String, String), String>,

    /// Bundled dependency lookup
    /// Maps (parent_key, dep_name) -> lockfile key
    bundled_deps: HashMap<(String, String), String>,
}

impl PackageIndex {
    /// Create a new package index from a packages map.
    pub fn new(packages: &super::Map<String, PackageEntry>) -> Self {
        let mut by_key = HashMap::with_capacity(packages.len());
        let mut by_ident: HashMap<String, Vec<String>> = HashMap::new();
        let mut workspace_scoped = HashMap::new();
        let mut bundled_deps = HashMap::new();

        // First pass: populate by_key and by_ident
        for (key, entry) in packages {
            by_key.insert(key.clone(), entry.clone());

            // Index by ident
            by_ident
                .entry(entry.ident.clone())
                .or_default()
                .push(key.clone());

            // Index workspace-scoped packages
            // Example: "workspace/package" -> ("workspace", "package")
            let parsed_key = PackageKey::parse(key);
            if let Some(parent) = parsed_key.parent() {
                workspace_scoped.insert((parent, parsed_key.name().to_string()), key.clone());
            }

            // Index bundled dependencies
            if key.contains('/') {
                let parsed_key = PackageKey::parse(key);
                if let Some(parent) = parsed_key.parent()
                    && let Some(info) = &entry.info
                    && info
                        .other
                        .get("bundled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                {
                    bundled_deps
                        .insert((parent.clone(), parsed_key.name().to_string()), key.clone());
                }
            }
        }

        // Sort by_ident vectors for deterministic selection (prefer workspace-scoped)
        for keys in by_ident.values_mut() {
            keys.sort();
        }

        Self {
            by_key,
            by_ident,
            workspace_scoped,
            bundled_deps,
        }
    }

    /// Get a package entry by lockfile key.
    #[cfg(test)]
    pub fn get_by_key(&self, key: &str) -> Option<&PackageEntry> {
        self.by_key.get(key)
    }

    /// Returns the number of packages in the index.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Get a package entry by ident (e.g., "lodash@4.17.21").
    ///
    /// If multiple keys map to the same ident, returns the first one
    /// (which is typically the workspace-scoped one due to sorting).
    pub fn get_by_ident(&self, ident: &str) -> Option<(&str, &PackageEntry)> {
        let keys = self.by_ident.get(ident)?;
        let key = keys.first()?;
        let entry = self.by_key.get(key)?;
        Some((key, entry))
    }

    /// Get all lockfile keys that map to a given ident.
    ///
    /// This is useful when you need to find all aliases for a package.
    #[cfg(test)]
    pub fn get_all_keys_for_ident(&self, ident: &str) -> Option<&[String]> {
        self.by_ident.get(ident).map(|v| v.as_slice())
    }

    /// Get a workspace-scoped package entry.
    ///
    /// For example, get_workspace_scoped("web", "lodash") looks up
    /// "web/lodash".
    pub fn get_workspace_scoped(&self, workspace: &str, package: &str) -> Option<&PackageEntry> {
        let key = self
            .workspace_scoped
            .get(&(workspace.to_string(), package.to_string()))?;
        self.by_key.get(key)
    }

    /// Get a bundled dependency entry.
    ///
    /// For example, get_bundled("parent", "dep") looks up "parent/dep" if it's
    /// bundled.
    #[cfg(test)]
    pub fn get_bundled(&self, parent: &str, dep: &str) -> Option<&PackageEntry> {
        let key = self
            .bundled_deps
            .get(&(parent.to_string(), dep.to_string()))?;
        self.by_key.get(key)
    }

    /// Find a package entry by name, searching in order:
    /// 1. Workspace-scoped (if workspace provided)
    /// 2. Top-level / hoisted
    /// 3. Bundled dependencies
    pub fn find_package<'a>(
        &'a self,
        workspace: Option<&str>,
        name: &'a str,
    ) -> Option<(&'a str, &'a PackageEntry)> {
        // Try workspace-scoped first
        if let Some(ws) = workspace
            && let Some(key) = self
                .workspace_scoped
                .get(&(ws.to_string(), name.to_string()))
            && let Some(entry) = self.by_key.get(key)
        {
            return Some((key.as_str(), entry));
        }

        // Try top-level
        if let Some(entry) = self.by_key.get(name) {
            return Some((name, entry));
        }

        // Try bundled (search all parents)
        for ((_parent, dep_name), key) in &self.bundled_deps {
            if dep_name == name
                && let Some(entry) = self.by_key.get(key)
            {
                return Some((key.as_str(), entry));
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::bun::{Map, PackageInfo};

    fn create_test_entry(ident: &str) -> PackageEntry {
        PackageEntry {
            ident: ident.to_string(),
            registry: Some("".to_string()),
            info: Some(PackageInfo::default()),
            checksum: Some("sha512".to_string()),
            root: None,
        }
    }

    fn create_bundled_entry(ident: &str) -> PackageEntry {
        let mut info = PackageInfo::default();
        info.other.insert("bundled".to_string(), json!(true));
        PackageEntry {
            ident: ident.to_string(),
            registry: Some("".to_string()),
            info: Some(info),
            checksum: Some("sha512".to_string()),
            root: None,
        }
    }

    #[test]
    fn test_package_index_basic_lookup() {
        let mut packages = Map::new();
        packages.insert("lodash".to_string(), create_test_entry("lodash@4.17.21"));
        packages.insert("react".to_string(), create_test_entry("react@18.0.0"));

        let index = PackageIndex::new(&packages);

        assert_eq!(index.len(), 2);
        assert!(index.get_by_key("lodash").is_some());
        assert!(index.get_by_key("react").is_some());
        assert!(index.get_by_key("nonexistent").is_none());
    }

    #[test]
    fn test_package_index_by_ident() {
        let mut packages = Map::new();
        packages.insert("lodash".to_string(), create_test_entry("lodash@4.17.21"));
        packages.insert(
            "web/lodash".to_string(),
            create_test_entry("lodash@4.17.21"),
        );

        let index = PackageIndex::new(&packages);

        // Should find the entry
        let (_key, entry) = index.get_by_ident("lodash@4.17.21").unwrap();
        assert_eq!(entry.ident, "lodash@4.17.21");

        // Should have both keys indexed
        let all_keys = index.get_all_keys_for_ident("lodash@4.17.21").unwrap();
        assert_eq!(all_keys.len(), 2);
        assert!(all_keys.contains(&"lodash".to_string()));
        assert!(all_keys.contains(&"web/lodash".to_string()));
    }

    #[test]
    fn test_package_index_workspace_scoped() {
        let mut packages = Map::new();
        packages.insert(
            "web/lodash".to_string(),
            create_test_entry("lodash@4.17.21"),
        );
        packages.insert(
            "@repo/ui/react".to_string(),
            create_test_entry("react@18.0.0"),
        );

        let index = PackageIndex::new(&packages);

        // Workspace-scoped lookup
        let entry = index.get_workspace_scoped("web", "lodash").unwrap();
        assert_eq!(entry.ident, "lodash@4.17.21");

        let entry = index.get_workspace_scoped("@repo/ui", "react").unwrap();
        assert_eq!(entry.ident, "react@18.0.0");

        // Non-existent workspace
        assert!(
            index
                .get_workspace_scoped("nonexistent", "lodash")
                .is_none()
        );
    }

    #[test]
    fn test_package_index_bundled() {
        let mut packages = Map::new();
        packages.insert("parent".to_string(), create_test_entry("parent@1.0.0"));
        packages.insert(
            "parent/bundled-dep".to_string(),
            create_bundled_entry("bundled-dep@2.0.0"),
        );

        let index = PackageIndex::new(&packages);

        // Bundled lookup
        let entry = index.get_bundled("parent", "bundled-dep").unwrap();
        assert_eq!(entry.ident, "bundled-dep@2.0.0");

        // Non-existent bundled
        assert!(index.get_bundled("parent", "nonexistent").is_none());
    }

    #[test]
    fn test_package_index_find_package() {
        let mut packages = Map::new();
        packages.insert("lodash".to_string(), create_test_entry("lodash@4.17.21"));
        packages.insert(
            "web/lodash".to_string(),
            create_test_entry("lodash@4.17.20"),
        );
        packages.insert(
            "parent/bundled".to_string(),
            create_bundled_entry("bundled@1.0.0"),
        );

        let index = PackageIndex::new(&packages);

        // Workspace-scoped takes priority
        let (key, entry) = index.find_package(Some("web"), "lodash").unwrap();
        assert_eq!(key, "web/lodash");
        assert_eq!(entry.ident, "lodash@4.17.20");

        // Falls back to top-level if workspace not found
        let (key, entry) = index.find_package(Some("other"), "lodash").unwrap();
        assert_eq!(key, "lodash");
        assert_eq!(entry.ident, "lodash@4.17.21");

        // Finds bundled dependencies
        let (key, entry) = index.find_package(None, "bundled").unwrap();
        assert_eq!(key, "parent/bundled");
        assert_eq!(entry.ident, "bundled@1.0.0");
    }
}
