//! Structured types for Bun lockfile data.
//!
//! This module provides strongly-typed representations of Bun lockfile concepts
//! to replace error-prone string parsing throughout the codebase.

use std::fmt;

/// Represents a package key in the Bun lockfile's packages section.
///
/// Package keys can take several forms:
/// - Simple: `"lodash"` - top-level package
/// - Scoped: `"@babel/core"` - scoped package
/// - Nested: `"parent/dep"` - nested under parent
/// - ScopedNested: `"@scope/parent/dep"` - nested under scoped parent
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PackageKey {
    /// Top-level package (e.g., "lodash")
    Simple(String),
    /// Scoped package (e.g., "@babel/core")
    Scoped { scope: String, name: String },
    /// Nested under parent (e.g., "parent/dep")
    Nested { parent: String, name: String },
    /// Nested under scoped parent (e.g., "@scope/parent/dep")
    ScopedNested {
        scope: String,
        parent: String,
        name: String,
    },
}

impl PackageKey {
    /// Parse a package key string into its structured form.
    pub fn parse(s: &str) -> Self {
        if s.starts_with('@') {
            // Scoped package or scoped nested
            if let Some(first_slash) = s.find('/') {
                let scope = s[1..first_slash].to_string(); // Skip '@'
                let after_scope = &s[first_slash + 1..];

                if let Some(second_slash) = after_scope.find('/') {
                    // ScopedNested: @scope/parent/name
                    let parent = after_scope[..second_slash].to_string();
                    let name = after_scope[second_slash + 1..].to_string();
                    Self::ScopedNested {
                        scope,
                        parent,
                        name,
                    }
                } else {
                    // Scoped: @scope/name
                    Self::Scoped {
                        scope,
                        name: after_scope.to_string(),
                    }
                }
            } else {
                // Malformed scoped package, treat as simple
                Self::Simple(s.to_string())
            }
        } else if let Some(slash_pos) = s.find('/') {
            // Nested: parent/name
            let parent = s[..slash_pos].to_string();
            let name = s[slash_pos + 1..].to_string();
            Self::Nested { parent, name }
        } else {
            // Simple: name
            Self::Simple(s.to_string())
        }
    }

    /// Returns the package name component.
    pub fn name(&self) -> &str {
        match self {
            Self::Simple(name) => name,
            Self::Scoped { name, .. } => name,
            Self::Nested { name, .. } => name,
            Self::ScopedNested { name, .. } => name,
        }
    }

    /// Returns true if this key is workspace-prefixed (nested under a workspace
    /// name).
    #[cfg(test)]
    pub fn is_workspace_prefixed(
        &self,
        workspace_names: &std::collections::HashSet<String>,
    ) -> bool {
        match self {
            Self::Nested { parent, .. } => workspace_names.contains(parent),
            Self::ScopedNested { scope, parent, .. } => {
                workspace_names.contains(&format!("@{scope}/{parent}"))
            }
            _ => false,
        }
    }

    /// Remove workspace prefix, returning the dealiased key.
    ///
    /// For example, `"workspace/package"` becomes `"package"`.
    /// For `"@babel/workspace/package"` becomes `"package"` (not
    /// `"@babel/package"`). Returns `None` if this key is not
    /// workspace-prefixed.
    pub fn dealias(&self) -> Option<Self> {
        match self {
            Self::Nested { name, .. } => Some(Self::Simple(name.clone())),
            // For ScopedNested, the scope is part of the workspace name,
            // so we just return the bare package name
            Self::ScopedNested { name, .. } => Some(Self::Simple(name.clone())),
            _ => None,
        }
    }

    /// Returns the parent component if this is a nested key.
    pub fn parent(&self) -> Option<String> {
        match self {
            Self::Nested { parent, .. } => Some(parent.clone()),
            Self::ScopedNested { scope, parent, .. } => Some(format!("@{scope}/{parent}")),
            _ => None,
        }
    }
}

impl fmt::Display for PackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Simple(name) => write!(f, "{name}"),
            Self::Scoped { scope, name } => write!(f, "@{scope}/{name}"),
            Self::Nested { parent, name } => write!(f, "{parent}/{name}"),
            Self::ScopedNested {
                scope,
                parent,
                name,
            } => write!(f, "@{scope}/{parent}/{name}"),
        }
    }
}

impl From<&str> for PackageKey {
    fn from(s: &str) -> Self {
        Self::parse(s)
    }
}

impl From<String> for PackageKey {
    fn from(s: String) -> Self {
        Self::parse(&s)
    }
}

/// Represents a package identifier in the format `name@version` or
/// `name@protocol:details`.
///
/// Package idents can be:
/// - Registry: `"react@18.0.0"` - npm registry package
/// - Workspace: `"@repo/ui@workspace:packages/ui"` - workspace package
/// - Git: `"pkg@git+https://github.com/user/repo"` - git package
/// - Other protocols: link, file, tarball, root
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageIdent {
    /// Registry package: name@version
    Registry { name: String, version: String },
    /// Workspace package: name@workspace:path
    Workspace { name: String, path: String },
    /// Git package: name@git+url or name@github:user/repo
    Git {
        name: String,
        url: String,
        rev: Option<String>,
    },
    /// Link package: name@link:path
    Link { name: String, path: String },
    /// File package: name@file:path
    File { name: String, path: String },
    /// Tarball package: name@tarball
    Tarball { name: String },
    /// Root package: name@root:
    Root { name: String },
}

impl PackageIdent {
    /// Parse a package ident string.
    pub fn parse(s: &str) -> Self {
        if let Some((name, rest)) = s.rsplit_once('@') {
            // Handle workspace protocol
            if let Some(path) = rest.strip_prefix("workspace:") {
                return Self::Workspace {
                    name: name.to_string(),
                    path: path.to_string(),
                };
            }

            // Handle git protocols
            if rest.starts_with("git+") || rest.starts_with("github:") {
                return Self::Git {
                    name: name.to_string(),
                    url: rest.to_string(),
                    rev: None,
                };
            }

            // Handle link protocol
            if let Some(path) = rest.strip_prefix("link:") {
                return Self::Link {
                    name: name.to_string(),
                    path: path.to_string(),
                };
            }

            // Handle file protocol
            if let Some(path) = rest.strip_prefix("file:") {
                return Self::File {
                    name: name.to_string(),
                    path: path.to_string(),
                };
            }

            // Handle tarball
            if rest == "tarball" {
                return Self::Tarball {
                    name: name.to_string(),
                };
            }

            // Handle root
            if rest == "root:" || rest.is_empty() && s.ends_with("@root:") {
                return Self::Root {
                    name: name.to_string(),
                };
            }

            // Default to registry package
            Self::Registry {
                name: name.to_string(),
                version: rest.to_string(),
            }
        } else {
            // No @ found, treat as simple name with unknown version
            Self::Registry {
                name: s.to_string(),
                version: String::new(),
            }
        }
    }

    /// Returns the package name.
    pub fn name(&self) -> &str {
        match self {
            Self::Registry { name, .. }
            | Self::Workspace { name, .. }
            | Self::Git { name, .. }
            | Self::Link { name, .. }
            | Self::File { name, .. }
            | Self::Tarball { name }
            | Self::Root { name } => name,
        }
    }

    /// Returns the version for registry packages.
    #[cfg(test)]
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Registry { version, .. } => Some(version),
            _ => None,
        }
    }

    /// Returns the workspace path if this is a workspace ident.
    pub fn workspace_path(&self) -> Option<&str> {
        match self {
            Self::Workspace { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Returns true if this is a workspace ident.
    pub fn is_workspace(&self) -> bool {
        matches!(self, Self::Workspace { .. })
    }
}

impl fmt::Display for PackageIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry { name, version } => write!(f, "{name}@{version}"),
            Self::Workspace { name, path } => write!(f, "{name}@workspace:{path}"),
            Self::Git { name, url, .. } => write!(f, "{name}@{url}"),
            Self::Link { name, path } => write!(f, "{name}@link:{path}"),
            Self::File { name, path } => write!(f, "{name}@file:{path}"),
            Self::Tarball { name } => write!(f, "{name}@tarball"),
            Self::Root { name } => write!(f, "{name}@root:"),
        }
    }
}

impl From<&str> for PackageIdent {
    fn from(s: &str) -> Self {
        Self::parse(s)
    }
}

impl From<String> for PackageIdent {
    fn from(s: String) -> Self {
        Self::parse(&s)
    }
}

/// Represents a version specification from package.json dependencies.
///
/// Version specs can be:
/// - Semver: `"^1.0.0"`, `"~2.3.4"`, `">=1.0.0 <2.0.0"`
/// - Catalog: `"catalog:"` (default) or `"catalog:group"` (named)
/// - Workspace: workspace path reference
/// - Tag: `"latest"`, `"next"`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VersionSpec {
    /// Semver version or range
    Semver(String),
    /// Catalog reference: None = default catalog, Some(name) = named catalog
    Catalog { catalog: Option<String> },
    /// Workspace path
    Workspace(String),
    /// Tag (e.g., "latest", "next")
    Tag(String),
}

impl VersionSpec {
    /// Parse a version spec string.
    pub fn parse(s: &str) -> Self {
        // Handle catalog references
        if let Some(rest) = s.strip_prefix("catalog:") {
            return Self::Catalog {
                catalog: if rest.is_empty() {
                    None
                } else {
                    Some(rest.to_string())
                },
            };
        }

        // Check if it looks like a semver version
        if s.starts_with('^')
            || s.starts_with('~')
            || s.starts_with('=')
            || s.starts_with('>')
            || s.starts_with('<')
            || s.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return Self::Semver(s.to_string());
        }

        // Check for workspace path (contains slash but not a URL)
        if s.contains('/') && !s.contains(':') {
            return Self::Workspace(s.to_string());
        }

        // Check for known tags
        if matches!(s, "latest" | "next" | "canary" | "beta" | "alpha") {
            return Self::Tag(s.to_string());
        }

        // Default to semver for anything else
        Self::Semver(s.to_string())
    }

    /// Returns true if this is a catalog reference.
    pub fn is_catalog(&self) -> bool {
        matches!(self, Self::Catalog { .. })
    }

    /// Returns the catalog name if this is a catalog reference.
    #[cfg(test)]
    pub fn catalog_name(&self) -> Option<&str> {
        match self {
            Self::Catalog { catalog } => catalog.as_deref(),
            _ => None,
        }
    }

    /// Returns the workspace path if this is a workspace reference.
    pub fn workspace_path(&self) -> Option<&str> {
        match self {
            Self::Workspace(path) => Some(path),
            _ => None,
        }
    }

    /// Returns true if this is a workspace reference.
    #[cfg(test)]
    pub fn is_workspace(&self) -> bool {
        matches!(self, Self::Workspace(_))
    }
}

impl fmt::Display for VersionSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Semver(v) => write!(f, "{v}"),
            Self::Catalog { catalog: None } => write!(f, "catalog:"),
            Self::Catalog {
                catalog: Some(name),
            } => write!(f, "catalog:{name}"),
            Self::Workspace(path) => write!(f, "{path}"),
            Self::Tag(tag) => write!(f, "{tag}"),
        }
    }
}

impl From<&str> for VersionSpec {
    fn from(s: &str) -> Self {
        Self::parse(s)
    }
}

impl From<String> for VersionSpec {
    fn from(s: String) -> Self {
        Self::parse(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_key_simple() {
        let key = PackageKey::parse("lodash");
        assert_eq!(key, PackageKey::Simple("lodash".to_string()));
        assert_eq!(key.name(), "lodash");
        assert_eq!(key.to_string(), "lodash");
        assert_eq!(key.parent(), None);
    }

    #[test]
    fn test_package_key_scoped() {
        let key = PackageKey::parse("@babel/core");
        assert_eq!(
            key,
            PackageKey::Scoped {
                scope: "babel".to_string(),
                name: "core".to_string()
            }
        );
        assert_eq!(key.name(), "core");
        assert_eq!(key.to_string(), "@babel/core");
        assert_eq!(key.parent(), None);
    }

    #[test]
    fn test_package_key_nested() {
        let key = PackageKey::parse("parent/dep");
        assert_eq!(
            key,
            PackageKey::Nested {
                parent: "parent".to_string(),
                name: "dep".to_string()
            }
        );
        assert_eq!(key.name(), "dep");
        assert_eq!(key.to_string(), "parent/dep");
        assert_eq!(key.parent(), Some("parent".to_string()));
    }

    #[test]
    fn test_package_key_scoped_nested() {
        let key = PackageKey::parse("@scope/parent/dep");
        assert_eq!(
            key,
            PackageKey::ScopedNested {
                scope: "scope".to_string(),
                parent: "parent".to_string(),
                name: "dep".to_string()
            }
        );
        assert_eq!(key.name(), "dep");
        assert_eq!(key.to_string(), "@scope/parent/dep");
        assert_eq!(key.parent(), Some("@scope/parent".to_string()));
    }

    #[test]
    fn test_package_key_dealias() {
        let key = PackageKey::parse("workspace/package");
        assert_eq!(
            key.dealias(),
            Some(PackageKey::Simple("package".to_string()))
        );

        // For ScopedNested, the scope is part of the workspace name,
        // so dealiasing returns just the package name
        let key = PackageKey::parse("@scope/workspace/package");
        assert_eq!(
            key.dealias(),
            Some(PackageKey::Simple("package".to_string()))
        );

        let key = PackageKey::parse("simple");
        assert_eq!(key.dealias(), None);
    }

    #[test]
    fn test_package_key_workspace_prefixed() {
        let workspaces: std::collections::HashSet<String> =
            ["web".to_string(), "@repo/ui".to_string()]
                .into_iter()
                .collect();

        let key = PackageKey::parse("web/lodash");
        assert!(key.is_workspace_prefixed(&workspaces));

        let key = PackageKey::parse("@repo/ui/react");
        assert!(key.is_workspace_prefixed(&workspaces));

        let key = PackageKey::parse("other/package");
        assert!(!key.is_workspace_prefixed(&workspaces));

        let key = PackageKey::parse("lodash");
        assert!(!key.is_workspace_prefixed(&workspaces));
    }

    #[test]
    fn test_package_ident_registry() {
        let ident = PackageIdent::parse("react@18.0.0");
        assert_eq!(
            ident,
            PackageIdent::Registry {
                name: "react".to_string(),
                version: "18.0.0".to_string()
            }
        );
        assert_eq!(ident.name(), "react");
        assert_eq!(ident.version(), Some("18.0.0"));
        assert_eq!(ident.to_string(), "react@18.0.0");
    }

    #[test]
    fn test_package_ident_workspace() {
        let ident = PackageIdent::parse("@repo/ui@workspace:packages/ui");
        assert_eq!(
            ident,
            PackageIdent::Workspace {
                name: "@repo/ui".to_string(),
                path: "packages/ui".to_string()
            }
        );
        assert_eq!(ident.name(), "@repo/ui");
        assert!(ident.is_workspace());
        assert_eq!(ident.workspace_path(), Some("packages/ui"));
        assert_eq!(ident.to_string(), "@repo/ui@workspace:packages/ui");
    }

    #[test]
    fn test_package_ident_git() {
        let ident = PackageIdent::parse("pkg@git+https://github.com/user/repo");
        match ident {
            PackageIdent::Git { name, url, .. } => {
                assert_eq!(name, "pkg");
                assert_eq!(url, "git+https://github.com/user/repo");
            }
            _ => panic!("Expected Git ident"),
        }
    }

    #[test]
    fn test_package_ident_root() {
        let ident = PackageIdent::parse("some-package@root:");
        assert_eq!(
            ident,
            PackageIdent::Root {
                name: "some-package".to_string()
            }
        );
        assert_eq!(ident.name(), "some-package");
    }

    #[test]
    fn test_version_spec_semver() {
        assert_eq!(
            VersionSpec::parse("^1.0.0"),
            VersionSpec::Semver("^1.0.0".to_string())
        );
        assert_eq!(
            VersionSpec::parse("~2.3.4"),
            VersionSpec::Semver("~2.3.4".to_string())
        );
        assert_eq!(
            VersionSpec::parse("1.0.0"),
            VersionSpec::Semver("1.0.0".to_string())
        );
    }

    #[test]
    fn test_version_spec_catalog() {
        let spec = VersionSpec::parse("catalog:");
        assert_eq!(spec, VersionSpec::Catalog { catalog: None });
        assert!(spec.is_catalog());
        assert_eq!(spec.catalog_name(), None);

        let spec = VersionSpec::parse("catalog:frontend");
        assert_eq!(
            spec,
            VersionSpec::Catalog {
                catalog: Some("frontend".to_string())
            }
        );
        assert_eq!(spec.catalog_name(), Some("frontend"));
    }

    #[test]
    fn test_version_spec_workspace() {
        let spec = VersionSpec::parse("packages/ui");
        assert_eq!(spec, VersionSpec::Workspace("packages/ui".to_string()));
        assert!(spec.is_workspace());
        assert_eq!(spec.workspace_path(), Some("packages/ui"));
    }

    #[test]
    fn test_version_spec_tag() {
        assert_eq!(
            VersionSpec::parse("latest"),
            VersionSpec::Tag("latest".to_string())
        );
        assert_eq!(
            VersionSpec::parse("next"),
            VersionSpec::Tag("next".to_string())
        );
    }
}
