//! Parsing of Cargo manifests (`Cargo.toml`).
//!
//! This is the foundational building block for treating Rust crates as
//! Turborepo packages. It deliberately knows nothing about the existing
//! [`crate::package_manager::PackageManager`] / [`crate::package_json`]
//! machinery — wiring Cargo crates into package discovery and the package
//! graph happens in later iterations. For now this module just turns a
//! `Cargo.toml` on disk into the three things downstream code needs:
//!
//! * the crate's package name (if it is a package),
//! * the workspace member/exclude globs (if it is a workspace root),
//! * its internal (path-based) dependencies on other crates.

use std::{collections::BTreeMap, io, str::FromStr};

use globwalk::ValidatedGlob;
use serde::Deserialize;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

/// The conventional file name for a Cargo manifest.
pub const CARGO_TOML: &str = "Cargo.toml";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error(transparent)]
    Glob(#[from] globwalk::GlobError),
    #[error(transparent)]
    Walk(#[from] globwalk::WalkError),
}

/// A single Rust crate discovered within a Cargo workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoCrate {
    /// The crate's package name (from `[package].name`).
    pub name: String,
    /// Absolute path to the crate's `Cargo.toml`.
    pub manifest_path: AbsoluteSystemPathBuf,
    /// Names of other workspace crates this crate depends on (path
    /// dependencies, including workspace-inherited ones).
    pub internal_dependencies: Vec<String>,
}

/// Discover all Rust crates in the Cargo workspace rooted at `repo_root`.
///
/// Returns an empty vec if `repo_root` is not a Cargo workspace (no readable
/// `Cargo.toml`). Individual member manifests that fail to parse are skipped
/// with a debug log rather than failing the whole discovery — a single broken
/// `Cargo.toml` should not break `turbo run`.
pub fn discover_crates(repo_root: &AbsoluteSystemPath) -> Vec<CargoCrate> {
    discover_crates_inner(repo_root).unwrap_or_else(|err| {
        tracing::warn!("failed to discover Cargo crates: {err}");
        Vec::new()
    })
}

fn discover_crates_inner(repo_root: &AbsoluteSystemPath) -> Result<Vec<CargoCrate>, Error> {
    let root_manifest_path = repo_root.join_component(CARGO_TOML);
    let Ok(root_manifest) = CargoManifest::from_file(&root_manifest_path) else {
        // Not a Cargo workspace (or unreadable root manifest); nothing to do.
        return Ok(Vec::new());
    };

    let mut crates = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // The root manifest may itself declare a `[package]` (a "root crate").
    if root_manifest.package_name().is_some() {
        crates.push(build_crate(&root_manifest_path, &root_manifest, &root_manifest));
        seen.insert(root_manifest_path.clone());
    }

    if let Some(members) = root_manifest.workspace_members() {
        let inclusions = members
            .iter()
            .map(|member| manifest_glob(member))
            .collect::<Result<Vec<_>, _>>()?;
        let exclusions = root_manifest
            .workspace_exclude()
            .unwrap_or(&[])
            .iter()
            .map(|member| manifest_glob(member))
            .collect::<Result<Vec<_>, _>>()?;

        let manifest_paths = globwalk::globwalk_with_settings(
            repo_root,
            &inclusions,
            &exclusions,
            globwalk::WalkType::Files,
            globwalk::Settings::default().follow_links(),
        )?;

        for manifest_path in manifest_paths {
            if !seen.insert(manifest_path.clone()) {
                continue;
            }
            match CargoManifest::from_file(&manifest_path) {
                Ok(manifest) if manifest.package_name().is_some() => {
                    crates.push(build_crate(&manifest_path, &manifest, &root_manifest));
                }
                // Virtual manifest (nested workspace with no package) — skip.
                Ok(_) => {}
                Err(err) => tracing::debug!("skipping Cargo manifest {manifest_path}: {err}"),
            }
        }
    }

    Ok(crates)
}

/// Build the inclusion/exclusion glob for a workspace member directory,
/// matching the member's `Cargo.toml` (mirrors how JS workspace globs target
/// `package.json`).
fn manifest_glob(member: &str) -> Result<ValidatedGlob, globwalk::GlobError> {
    let mut pattern = member.to_string();
    if !pattern.ends_with('/') {
        pattern.push('/');
    }
    pattern.push_str(CARGO_TOML);
    ValidatedGlob::from_str(&pattern)
}

fn build_crate(
    manifest_path: &AbsoluteSystemPath,
    manifest: &CargoManifest,
    workspace_root: &CargoManifest,
) -> CargoCrate {
    let name = manifest
        .package_name()
        .expect("caller guarantees a package name")
        .to_string();
    let mut internal_dependencies: Vec<String> = manifest
        .internal_dependencies(Some(workspace_root))
        .into_iter()
        .map(|dep| dep.name)
        .collect();
    internal_dependencies.sort();
    internal_dependencies.dedup();
    CargoCrate {
        name,
        manifest_path: manifest_path.to_owned(),
        internal_dependencies,
    }
}

/// A parsed `Cargo.toml`.
///
/// A single manifest can be a package (`[package]`), a workspace root
/// (`[workspace]`), or both (a "root crate").
#[derive(Debug, Clone, PartialEq)]
pub struct CargoManifest {
    raw: RawManifest,
}

/// A workspace-internal dependency: a dependency on another crate by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathDependency {
    /// The dependency key as written in the manifest (or its `package`
    /// rename target, when present).
    pub name: String,
    /// The relative path as written in `path = "..."`.
    pub path: String,
    /// What [`PathDependency::path`] is relative to.
    pub base: PathBase,
}

/// What a [`PathDependency`]'s path is anchored to.
///
/// Direct `path = "..."` dependencies are relative to the directory
/// containing the crate's `Cargo.toml`, whereas paths pulled in via
/// `dep = { workspace = true }` are declared in `[workspace.dependencies]`
/// and are therefore relative to the workspace root. Resolving these to
/// absolute paths is the job of a later discovery step, which is why we
/// preserve the anchor here rather than joining eagerly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathBase {
    /// Relative to the directory containing this crate's `Cargo.toml`.
    Crate,
    /// Relative to the workspace root.
    Workspace,
}

impl CargoManifest {
    /// Read and parse a `Cargo.toml` from disk.
    pub fn from_file(path: &AbsoluteSystemPath) -> Result<Self, Error> {
        let contents = path.read_to_string().map_err(|source| Error::Read {
            path: path.to_string(),
            source,
        })?;
        Self::from_str(&contents, path.as_str())
    }

    /// Parse a `Cargo.toml` from its string contents. `name` is used only for
    /// error messages.
    pub fn from_str(contents: &str, name: &str) -> Result<Self, Error> {
        let raw = toml::from_str(contents).map_err(|source| Error::Parse {
            path: name.to_string(),
            source,
        })?;
        Ok(Self { raw })
    }

    /// The crate's package name, if this manifest declares a `[package]`.
    ///
    /// A virtual manifest (workspace root with no `[package]`) returns
    /// `None`.
    pub fn package_name(&self) -> Option<&str> {
        self.raw.package.as_ref().map(|pkg| pkg.name.as_str())
    }

    /// Whether this manifest declares a `[workspace]`.
    pub fn is_workspace_root(&self) -> bool {
        self.raw.workspace.is_some()
    }

    /// The `[workspace].members` globs, if this is a workspace root.
    pub fn workspace_members(&self) -> Option<&[String]> {
        self.raw.workspace.as_ref().map(|ws| ws.members.as_slice())
    }

    /// The `[workspace].exclude` globs, if this is a workspace root.
    pub fn workspace_exclude(&self) -> Option<&[String]> {
        self.raw.workspace.as_ref().map(|ws| ws.exclude.as_slice())
    }

    /// All internal (path-based) dependencies of this crate.
    ///
    /// Direct `path = "..."` dependencies are always returned. Dependencies
    /// declared as `{ workspace = true }` are resolved against
    /// `workspace_root`'s `[workspace.dependencies]` table (when provided)
    /// and included only if that table gives them a `path`.
    pub fn internal_dependencies(
        &self,
        workspace_root: Option<&CargoManifest>,
    ) -> Vec<PathDependency> {
        let mut deps = Vec::new();
        for table in [
            &self.raw.dependencies,
            &self.raw.dev_dependencies,
            &self.raw.build_dependencies,
        ] {
            for (key, value) in table {
                match classify(value) {
                    DepKind::Path(path) => deps.push(PathDependency {
                        name: rename(value).unwrap_or(key).to_string(),
                        path,
                        base: PathBase::Crate,
                    }),
                    DepKind::Workspace => {
                        if let Some(path) = workspace_root.and_then(|root| {
                            root.workspace_dependency_path(rename(value).unwrap_or(key))
                        }) {
                            deps.push(PathDependency {
                                name: rename(value).unwrap_or(key).to_string(),
                                path,
                                base: PathBase::Workspace,
                            });
                        }
                    }
                    DepKind::External => {}
                }
            }
        }
        deps
    }

    /// Look up a path for `name` in this manifest's `[workspace.dependencies]`.
    fn workspace_dependency_path(&self, name: &str) -> Option<String> {
        let deps = &self.raw.workspace.as_ref()?.dependencies;
        match deps.get(name).map(classify) {
            Some(DepKind::Path(path)) => Some(path),
            _ => None,
        }
    }
}

/// How a single dependency entry resolves for the purposes of building the
/// internal package graph.
enum DepKind {
    /// A direct `path = "..."` dependency, value is the path.
    Path(String),
    /// A `{ workspace = true }` dependency to be resolved against the
    /// workspace root.
    Workspace,
    /// A registry/git/version dependency we don't track as an internal edge.
    External,
}

/// Classify a raw dependency value (`toml::Value`) without committing to a
/// rigid schema. Dependencies come in many shapes (`"1.0"`,
/// `{ version = "1" }`, `{ path = ".." }`, `{ workspace = true }`, …) so we
/// inspect the value directly rather than relying on an untagged enum, which
/// `toml` deserializes inconsistently.
fn classify(value: &toml::Value) -> DepKind {
    let Some(table) = value.as_table() else {
        // A bare version string, e.g. `serde = "1.0"`.
        return DepKind::External;
    };
    if table.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
        return DepKind::Workspace;
    }
    match table.get("path").and_then(toml::Value::as_str) {
        Some(path) => DepKind::Path(path.to_string()),
        None => DepKind::External,
    }
}

/// The `package = "..."` rename target of a dependency, if present. This is
/// the crate's real name, which is what `[workspace.dependencies]` is keyed
/// by.
fn rename(value: &toml::Value) -> Option<&str> {
    value.as_table()?.get("package")?.as_str()
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RawManifest {
    package: Option<RawPackage>,
    workspace: Option<RawWorkspace>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, toml::Value>,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RawPackage {
    // `package.name` cannot itself be workspace-inherited, so a plain string.
    name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RawWorkspace {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
}

#[cfg(test)]
mod test {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    fn parse(contents: &str) -> CargoManifest {
        CargoManifest::from_str(contents, "Cargo.toml").unwrap()
    }

    #[test]
    fn test_virtual_workspace_root() {
        let manifest = parse(
            r#"
            [workspace]
            members = ["crates/*", "apps/server"]
            exclude = ["crates/legacy"]
            "#,
        );
        assert!(manifest.is_workspace_root());
        assert_eq!(manifest.package_name(), None);
        assert_eq!(
            manifest.workspace_members(),
            Some(["crates/*".to_string(), "apps/server".to_string()].as_slice())
        );
        assert_eq!(
            manifest.workspace_exclude(),
            Some(["crates/legacy".to_string()].as_slice())
        );
    }

    #[test]
    fn test_plain_package() {
        let manifest = parse(
            r#"
            [package]
            name = "my-crate"
            version = "0.1.0"
            "#,
        );
        assert!(!manifest.is_workspace_root());
        assert_eq!(manifest.package_name(), Some("my-crate"));
        assert!(manifest.workspace_members().is_none());
    }

    #[test]
    fn test_root_crate_is_both() {
        let manifest = parse(
            r#"
            [package]
            name = "root-crate"

            [workspace]
            members = ["crates/*"]
            "#,
        );
        assert!(manifest.is_workspace_root());
        assert_eq!(manifest.package_name(), Some("root-crate"));
    }

    #[test]
    fn test_direct_path_dependencies() {
        let manifest = parse(
            r#"
            [package]
            name = "app"

            [dependencies]
            serde = "1.0"
            lib-a = { path = "../lib-a" }
            lib-b = { version = "0.2", path = "../lib-b" }

            [dev-dependencies]
            test-util = { path = "../test-util" }

            [build-dependencies]
            codegen = { path = "../codegen" }
            "#,
        );
        let mut deps = manifest.internal_dependencies(None);
        deps.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            deps,
            vec![
                PathDependency {
                    name: "codegen".into(),
                    path: "../codegen".into(),
                    base: PathBase::Crate,
                },
                PathDependency {
                    name: "lib-a".into(),
                    path: "../lib-a".into(),
                    base: PathBase::Crate,
                },
                PathDependency {
                    name: "lib-b".into(),
                    path: "../lib-b".into(),
                    base: PathBase::Crate,
                },
                PathDependency {
                    name: "test-util".into(),
                    path: "../test-util".into(),
                    base: PathBase::Crate,
                },
            ]
        );
    }

    #[test]
    fn test_external_dependencies_ignored() {
        let manifest = parse(
            r#"
            [package]
            name = "app"

            [dependencies]
            serde = "1.0"
            tokio = { version = "1", features = ["full"] }
            rand = { git = "https://github.com/rust-random/rand" }
            "#,
        );
        assert!(manifest.internal_dependencies(None).is_empty());
    }

    #[test]
    fn test_workspace_inherited_path_dependency() {
        let root = parse(
            r#"
            [workspace]
            members = ["crates/*"]

            [workspace.dependencies]
            lib-a = { path = "crates/lib-a" }
            serde = "1.0"
            "#,
        );
        let crate_manifest = parse(
            r#"
            [package]
            name = "app"

            [dependencies]
            lib-a = { workspace = true }
            serde = { workspace = true }
            "#,
        );
        let deps = crate_manifest.internal_dependencies(Some(&root));
        // `lib-a` resolves to a workspace-rooted path; `serde` is external.
        assert_eq!(
            deps,
            vec![PathDependency {
                name: "lib-a".into(),
                path: "crates/lib-a".into(),
                base: PathBase::Workspace,
            }]
        );
    }

    #[test]
    fn test_workspace_inherited_without_root_is_skipped() {
        let crate_manifest = parse(
            r#"
            [package]
            name = "app"

            [dependencies]
            lib-a = { workspace = true }
            "#,
        );
        // Without the workspace root we cannot resolve the path, so it's
        // omitted rather than guessed.
        assert!(crate_manifest.internal_dependencies(None).is_empty());
    }

    #[test]
    fn test_discover_crates() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::new(tmp.path().to_string_lossy().to_string()).unwrap();

        let write = |rel: &[&str], contents: &str| {
            let path = root.join_components(rel);
            std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
            std::fs::write(path.as_std_path(), contents).unwrap();
        };

        write(
            &["Cargo.toml"],
            r#"
            [workspace]
            members = ["crates/*"]
            exclude = ["crates/ignored"]

            [workspace.dependencies]
            lib-a = { path = "crates/lib-a" }
            "#,
        );
        write(&["crates", "lib-a", "Cargo.toml"], "[package]\nname = \"lib-a\"\n");
        write(
            &["crates", "app", "Cargo.toml"],
            r#"
            [package]
            name = "app"

            [dependencies]
            lib-a = { workspace = true }
            serde = "1.0"
            "#,
        );
        write(&["crates", "ignored", "Cargo.toml"], "[package]\nname = \"ignored\"\n");

        let mut crates = discover_crates(&root);
        crates.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(crates.len(), 2, "expected app and lib-a, got {crates:?}");
        assert_eq!(crates[0].name, "app");
        assert_eq!(crates[0].internal_dependencies, vec!["lib-a".to_string()]);
        assert_eq!(crates[1].name, "lib-a");
        assert!(crates[1].internal_dependencies.is_empty());
    }

    #[test]
    fn test_discover_crates_not_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::new(tmp.path().to_string_lossy().to_string()).unwrap();
        assert!(discover_crates(&root).is_empty());
    }

    #[test]
    fn test_renamed_dependency_uses_real_crate_name() {
        let manifest = parse(
            r#"
            [package]
            name = "app"

            [dependencies]
            my-alias = { path = "../real-crate", package = "real-crate" }
            "#,
        );
        let deps = manifest.internal_dependencies(None);
        assert_eq!(deps[0].name, "real-crate");
        assert_eq!(deps[0].path, "../real-crate");
    }
}
