#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};

use either::Either;
use napi::Error;
use napi_derive::napi;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_lockfiles::{Package as ResolvedPackage, PackageSource as CorePackageSource};
use turborepo_repository::{
    change_mapper::{
        ChangeMapper, DefaultPackageChangeMapper, DefaultPackageChangeMapperWithLockfile,
        LockfileContents, PackageChangeMapper, PackageChanges,
    },
    package_graph::{
        JavascriptExternalResolution, PackageGraph, PackageName, PackageNode, WorkspacePackage,
    },
};
use turborepo_scm::SCM;
mod internal;

#[napi]
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct Package {
    pub name: String,
    /// The absolute path to the package root.
    #[napi(readonly)]
    pub absolute_path: String,
    /// The relative path from the workspace root to the package root.
    #[napi(readonly)]
    pub relative_path: String,
}

/// Wrapper for dependents and dependencies.
/// Each are a list of package paths, relative to the workspace root.
#[napi]
#[derive(Debug)]
pub struct PackageDetails {
    /// the package's dependencies
    #[napi(readonly)]
    pub dependencies: Vec<String>,
    /// the packages that depend on this package
    #[napi(readonly)]
    pub dependents: Vec<String>,
}

#[derive(Clone)]
#[napi]
pub struct PackageManager {
    /// The package manager name in lower case.
    #[napi(readonly)]
    pub name: String,
    /// The declared package manager version (from the root `package.json`
    /// `packageManager` or `devEngines.packageManager` field), if available.
    #[napi(readonly)]
    pub version: Option<String>,
}

/// A single external package resolved from the workspace lockfile, as a fully
/// qualified `name` and `version` (e.g. `{ name: "lodash", version: "4.17.21"
/// }`). Peer-dependency closures are stripped so `${name}@${version}` is
/// always a plain `pkg@1.2.3` identifier.
fn package_source_name(source: CorePackageSource) -> String {
    match source {
        CorePackageSource::Registry => "registry",
        CorePackageSource::Git => "git",
        CorePackageSource::File => "file",
        CorePackageSource::Link => "link",
        CorePackageSource::Workspace => "workspace",
        CorePackageSource::Patch => "patch",
    }
    .to_string()
}

#[napi(object)]
pub struct LockfilePackage {
    pub name: String,
    pub version: String,
    #[napi(ts_type = "'registry' | 'git' | 'file' | 'link' | 'workspace' | 'patch'")]
    pub source: String,
}

/// A typed classifier for why lockfile package extraction was incomplete.
/// Stable across releases so consumers can group failures in metrics without
/// parsing human-readable messages.
#[napi(string_enum)]
pub enum LockfileErrorKind {
    /// No JavaScript lockfile resolution is available for this workspace (for
    /// example a single-package workspace, or one with no lockfile at all).
    NoLockfile,
    /// A lockfile is present but could not be read or parsed.
    LockfileUnreadable,
    /// The lockfile was read, but its dependency graph could not be resolved
    /// (for example a transitive closure or declaration could not be
    /// computed).
    ResolutionFailed,
    /// A specific lockfile entry could not be split into a `name` and
    /// `version` (for example an unrecognized key format).
    UnparseableEntry,
    /// npm lockfile v1 lacks the `packages` data required for dependency
    /// resolution.
    UnsupportedNpmLockfileVersion,
    /// Bun's binary `bun.lockb` format cannot be read; a text `bun.lock` is
    /// required.
    UnsupportedBunLockfile,
}

/// A single, typed reason lockfile package extraction was incomplete.
#[napi(object)]
pub struct LockfileError {
    /// The typed failure category, for grouping in metrics.
    pub kind: LockfileErrorKind,
    /// A human-readable explanation, including the underlying resolver reason
    /// code where one applies.
    pub message: String,
}

/// The external packages referenced by a workspace's lockfile, plus any typed
/// reasons the lockfile could not be fully parsed. Intended to be consumed for
/// metrics: a non-empty `errors` list (or an empty `packages` list alongside
/// one) signals that extraction was incomplete.
#[napi(object)]
pub struct LockfilePackages {
    /// Fully qualified external packages found in the lockfile, sorted and
    /// deduplicated.
    pub packages: Vec<LockfilePackage>,
    /// Typed reasons the lockfile could not be read or fully parsed. Empty
    /// when extraction succeeded.
    pub errors: Vec<LockfileError>,
    /// Absolute path to the lockfile used for resolution.
    pub lockfile_path: String,
    /// Lockfile family, such as `npm`, `pnpm`, `yarn`, or `bun`.
    pub lockfile_format: String,
    /// Package-manager-specific lockfile format version, when parsing
    /// succeeded.
    pub lockfile_version: Option<String>,
    /// Declared package-manager name.
    pub package_manager: String,
    /// Declared package-manager version, when available.
    pub package_manager_version: Option<String>,
}

pub(crate) struct LockfilePackagesMetadata {
    pub lockfile_path: String,
    pub lockfile_format: String,
    pub lockfile_version: Option<String>,
    pub package_manager: String,
    pub package_manager_version: Option<String>,
}

impl LockfilePackages {
    pub(crate) fn new(
        packages: Vec<LockfilePackage>,
        errors: Vec<LockfileError>,
        metadata: LockfilePackagesMetadata,
    ) -> Self {
        Self {
            packages,
            errors,
            lockfile_path: metadata.lockfile_path,
            lockfile_format: metadata.lockfile_format,
            lockfile_version: metadata.lockfile_version,
            package_manager: metadata.package_manager,
            package_manager_version: metadata.package_manager_version,
        }
    }
}

#[napi]
pub struct Workspace {
    /// The absolute path to the workspace root.
    #[napi(readonly)]
    pub absolute_path: String,
    /// `true` when the workspace is a multi-package workspace.
    #[napi(readonly)]
    pub is_multi_package: bool,
    /// The package manager used by the workspace.
    #[napi(readonly)]
    pub package_manager: PackageManager,
    /// The package graph for the workspace.
    graph: PackageGraph,
    /// Inputs for resolving a single-package repository's root lockfile on
    /// demand. Core single-package graphs intentionally skip lockfile
    /// resolution, so the JS API retains this repository-local fallback.
    lockfile: internal::SinglePackageLockfile,
    lockfile_path: String,
    lockfile_format: String,
}

#[napi]
impl Package {
    fn new(
        name: String,
        workspace_path: &AbsoluteSystemPath,
        package_path: &AbsoluteSystemPath,
    ) -> Result<Self, turbopath::PathError> {
        let relative_path = workspace_path.anchor(package_path)?;
        Ok(Self {
            name,
            absolute_path: package_path.to_string(),
            relative_path: relative_path.to_string(),
        })
    }

    fn dependents(
        &self,
        graph: &PackageGraph,
        workspace_path: &AbsoluteSystemPath,
    ) -> Vec<Package> {
        let node = PackageNode::Workspace(PackageName::Other(self.name.clone()));
        let pkgs = match graph.immediate_ancestors(&node) {
            Some(pkgs) => pkgs,
            None => return vec![],
        };

        pkgs.iter()
            .filter(|node| graph.is_real_package(node.as_package_name()))
            .filter_map(|node| {
                let context = graph.package_task_context(node.as_package_name())?;
                let package_path = workspace_path.resolve(context.directory());
                Package::new(context.package().to_string(), workspace_path, &package_path).ok()
            })
            .collect()
    }

    fn dependencies(
        &self,
        graph: &PackageGraph,
        workspace_path: &AbsoluteSystemPath,
    ) -> Vec<Package> {
        let node = PackageNode::Workspace(PackageName::Other(self.name.clone()));
        let pkgs = match graph.immediate_dependencies(&node) {
            Some(pkgs) => pkgs,
            None => return vec![],
        };

        pkgs.iter()
            .filter(|node| !matches!(node, PackageNode::Root))
            .filter(|node| graph.is_real_package(node.as_package_name()))
            .filter_map(|node| {
                let context = graph.package_task_context(node.as_package_name())?;
                let package_path = workspace_path.resolve(context.directory());
                Package::new(context.package().to_string(), workspace_path, &package_path).ok()
            })
            .collect()
    }
}

#[napi]
impl Workspace {
    /// Finds the workspace root from the given path, and returns a new
    /// Workspace.
    #[napi(factory)]
    pub async fn find(path: Option<String>) -> Result<Workspace, napi::Error> {
        Self::find_internal(path).await.map_err(|e| e.into())
    }

    /// Finds and returns packages within the workspace.
    #[napi]
    pub async fn find_packages(&self) -> std::result::Result<Vec<Package>, napi::Error> {
        self.packages_internal().await.map_err(|e| e.into())
    }

    /// Returns a map of packages within the workspace, its dependencies and
    /// dependents. The response looks like this:
    ///  {
    ///    "package-path": {
    ///      "dependents": ["dependent1_path", "dependent2_path"],
    ///      "dependencies": ["dependency1_path", "dependency2_path"]
    ///      }
    ///  }
    #[napi]
    pub async fn find_packages_with_graph(&self) -> Result<HashMap<String, PackageDetails>, Error> {
        let packages = self.find_packages().await?;

        let workspace_path = match AbsoluteSystemPath::new(self.absolute_path.as_str()) {
            Ok(path) => path,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let map: HashMap<String, PackageDetails> = packages
            .into_iter()
            .map(|package| {
                let details = PackageDetails {
                    dependencies: package
                        .dependencies(&self.graph, workspace_path)
                        .into_iter()
                        .map(|p| p.relative_path)
                        .collect(),
                    dependents: package
                        .dependents(&self.graph, workspace_path)
                        .into_iter()
                        .map(|p| p.relative_path)
                        .collect(),
                };

                (package.relative_path, details)
            })
            .collect();

        Ok(map)
    }

    /// Returns all external packages from the JavaScript resolution domain as
    /// `npm/<name>@<version>` strings. Cargo identities are excluded to
    /// preserve the historical lockfile-only listing.
    #[napi]
    pub async fn packages_from_lockfile(&self) -> Result<Vec<String>, napi::Error> {
        let identities = self.graph.javascript_external_package_identities();
        if identities.is_empty() && self.graph.lockfile().is_none() {
            return Err(napi::Error::from_reason("No lockfile found"));
        }

        let mut seen_keys = HashSet::new();
        let mut result: Vec<String> = identities
            .into_iter()
            .filter(|identity| seen_keys.insert(identity.key().to_string()))
            .map(|identity| format!("npm/{}", identity.display_name()))
            .collect();

        result.sort();
        Ok(result)
    }

    /// Returns the external packages referenced by the workspace lockfile as a
    /// flat, sorted, deduplicated list of `{ name, version }` structs, together
    /// with any reasons the lockfile could not be fully parsed.
    ///
    /// Unlike [`Self::packages_from_lockfile`], this never rejects: a missing
    /// or unparseable lockfile yields an empty `packages` list and a populated
    /// `errors` list rather than an exception, so callers can emit metrics on
    /// both success and failure. Package manager identity is exposed separately
    /// on [`Workspace::package_manager`].
    #[napi]
    pub async fn lockfile_packages(&self) -> LockfilePackages {
        if !self.is_multi_package {
            return self.lockfile.packages(
                &self.lockfile_path,
                &self.lockfile_format,
                &self.package_manager,
            );
        }

        let lockfile_version = self
            .graph
            .lockfile()
            .and_then(|lockfile| lockfile.format_version());
        let metadata = || LockfilePackagesMetadata {
            lockfile_path: self.lockfile_path.clone(),
            lockfile_format: self.lockfile_format.clone(),
            lockfile_version: lockfile_version.clone(),
            package_manager: self.package_manager.name.clone(),
            package_manager_version: self.package_manager.version.clone(),
        };

        match self.graph.javascript_external_resolution() {
            JavascriptExternalResolution::Resolved(identities) => {
                let mut packages = Vec::with_capacity(identities.len());
                let mut errors = Vec::new();
                for identity in identities {
                    match split_identity(identity.display_name()) {
                        Some((name, version)) => {
                            let source = self
                                .graph
                                .lockfile()
                                .map(|lockfile| {
                                    lockfile.package_source(&ResolvedPackage {
                                        key: identity.key().to_string(),
                                        version: identity.version().to_string(),
                                    })
                                })
                                .unwrap_or(CorePackageSource::Registry);
                            let source = package_source_name(source);
                            packages.push(LockfilePackage {
                                name,
                                version,
                                source,
                            });
                        }
                        None => errors.push(LockfileError {
                            kind: LockfileErrorKind::UnparseableEntry,
                            message: format!(
                                "could not parse name and version from lockfile entry '{}'",
                                identity.display_name()
                            ),
                        }),
                    }
                }
                packages.sort_by(|left, right| {
                    (&left.name, &left.version).cmp(&(&right.name, &right.version))
                });
                // Distinct lockfile identities can collapse to the same
                // `(name, version)` after `split_identity` strips pnpm peer
                // closures (e.g. `react-dom@18.2.0(react@18.2.0)` vs
                // `react-dom@18.2.0(react@17.0.0)`). The upstream dedup keys on
                // the full identity, so those variants both survive; dedup here
                // to honor the documented "deduplicated" contract.
                packages.dedup_by(|a, b| a.name == b.name && a.version == b.version);
                LockfilePackages::new(packages, errors, metadata())
            }
            JavascriptExternalResolution::Unavailable { code, message } => LockfilePackages::new(
                Vec::new(),
                vec![LockfileError {
                    kind: self
                        .lockfile
                        .error_kind()
                        .unwrap_or_else(|| classify_resolution_code(&code)),
                    message: self
                        .lockfile
                        .error_message()
                        .unwrap_or_else(|| format!("{code}: {message}")),
                }],
                metadata(),
            ),
            JavascriptExternalResolution::NotAvailable => LockfilePackages::new(
                Vec::new(),
                vec![LockfileError {
                    kind: LockfileErrorKind::NoLockfile,
                    message: "no JavaScript lockfile resolution is available for this workspace"
                        .to_string(),
                }],
                metadata(),
            ),
        }
    }

    pub fn get_lockfile_contents(
        &self,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
        workspace_root: &AbsoluteSystemPath,
        from_commit: &str,
    ) -> LockfileContents {
        let Some(lockfile_path) = self
            .graph
            .change_knowledge()
            .resolution_paths()
            .iter()
            .find_map(|path| {
                let path = AnchoredSystemPath::new(path).ok()?;
                changed_files.contains(path).then_some(path)
            })
        else {
            return LockfileContents::Unchanged;
        };

        let git = SCM::new(workspace_root);
        let anchored_path = workspace_root.resolve(lockfile_path);
        match git.previous_content(Some(from_commit), &anchored_path) {
            Ok(previous_contents) => LockfileContents::Changed {
                path: lockfile_path.to_owned(),
                previous_contents,
            },
            Err(e) => {
                debug!("{e}");
                LockfileContents::UnknownChange
            }
        }
    }

    /// Given a set of "changed" files, returns a set of packages that are
    /// "affected" by the changes. The `files` argument is expected to be a list
    /// of strings relative to the monorepo root and use the current system's
    /// path separator.
    #[napi]
    pub async fn affected_packages(
        &self,
        files: Vec<String>,
        base: Option<&str>, // this is required when optimize_global_invalidations is true
        optimize_global_invalidations: Option<bool>,
    ) -> Result<Vec<Package>, Error> {
        let base = matches!(optimize_global_invalidations, Some(true))
            .then(|| {
                base.ok_or_else(|| {
                    Error::from_reason("optimizeGlobalInvalidations true, but no base commit given")
                })
            })
            .transpose()?;
        let workspace_root = match AbsoluteSystemPath::new(&self.absolute_path) {
            Ok(path) => path,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };
        let changed_files: HashSet<AnchoredSystemPathBuf> = files
            .into_iter()
            .filter_map(|path| {
                let path_components = path.split(std::path::MAIN_SEPARATOR).collect::<Vec<&str>>();
                let absolute_path = workspace_root.join_components(&path_components);
                workspace_root.anchor(&absolute_path).ok()
            })
            .collect();

        // Create a ChangeMapper with no ignore patterns
        let change_detector = if base.is_some() {
            Either::Left(DefaultPackageChangeMapperWithLockfile::new(&self.graph))
        } else {
            Either::Right(DefaultPackageChangeMapper::new(&self.graph))
        };
        let mapper = ChangeMapper::new(&self.graph, vec![], change_detector);

        let lockfile_contents = if let Some(base) = base {
            self.get_lockfile_contents(&changed_files, workspace_root, base)
        } else if self
            .graph
            .change_knowledge()
            .resolution_paths()
            .iter()
            .filter_map(|path| AnchoredSystemPath::new(path).ok())
            .any(|path| changed_files.contains(path))
        {
            LockfileContents::UnknownChange
        } else {
            LockfileContents::Unchanged
        };

        let package_changes = match mapper.changed_packages(changed_files, lockfile_contents) {
            Ok(changes) => changes,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let packages = match package_changes {
            PackageChanges::All(_) => self
                .graph
                .package_task_contexts()
                .map(|context| WorkspacePackage {
                    name: context.package().clone(),
                    path: context.directory().to_owned(),
                })
                .collect::<Vec<WorkspacePackage>>(),
            PackageChanges::Some(packages) => packages.into_keys().collect(),
        };

        self.serialize_packages(packages)
    }

    /// Given a path (relative to the workspace root), returns the
    /// package that contains it.
    ///
    /// This is a naive implementation that simply "iterates-up". If this
    /// function is expected to be called many times for files that are deep
    /// within the same package, we could optimize this by caching the
    /// containing-package of every ancestor.
    #[napi]
    pub async fn find_package_by_path(&self, path: String) -> Result<Package, Error> {
        let package_mapper = DefaultPackageChangeMapper::new(&self.graph);
        let anchored_path = AnchoredSystemPath::new(&path)
            .map_err(|e| Error::from_reason(e.to_string()))?
            .clean();
        match package_mapper.detect_package(&anchored_path) {
            turborepo_repository::change_mapper::PackageMapping::All(
                _all_package_change_reason,
            ) => Err(Error::from_reason("file belongs to many packages")),
            turborepo_repository::change_mapper::PackageMapping::None => Err(Error::from_reason(
                "iterated to the root of the workspace and found no package",
            )),
            turborepo_repository::change_mapper::PackageMapping::Package((package, _reason)) => {
                let workspace_root = match AbsoluteSystemPath::new(&self.absolute_path) {
                    Ok(path) => path,
                    Err(e) => return Err(Error::from_reason(e.to_string())),
                };
                let package_path = workspace_root.resolve(&package.path);
                Package::new(package.name.to_string(), workspace_root, &package_path)
                    .map_err(|e| Error::from_reason(e.to_string()))
            }
        }
    }

    fn serialize_packages(
        &self,
        packages: impl IntoIterator<Item = WorkspacePackage>,
    ) -> Result<Vec<Package>, Error> {
        let workspace_root = AbsoluteSystemPath::new(&self.absolute_path)
            .map_err(|error| Error::from_reason(error.to_string()))?;
        let mut packages = packages
            .into_iter()
            .filter(|package| self.graph.is_real_package(&package.name))
            .map(|package| {
                let package_path = workspace_root.resolve(&package.path);
                Package::new(package.name.to_string(), workspace_root, &package_path)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| Error::from_reason(error.to_string()))?;
        packages.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(packages)
    }
}

/// Maps an underlying JavaScript external-resolution reason code to a typed
/// [`LockfileErrorKind`]. Unrecognized codes fall back to
/// [`LockfileErrorKind::ResolutionFailed`], since an `Unavailable` domain
/// always means resolution did not complete; the raw code is preserved in the
/// error message for debugging.
fn classify_resolution_code(code: &str) -> LockfileErrorKind {
    match code {
        "lockfile-unavailable" => LockfileErrorKind::LockfileUnreadable,
        _ => LockfileErrorKind::ResolutionFailed,
    }
}

/// Splits a fully qualified lockfile identity display name into a clean
/// `(name, version)` pair.
///
/// Display names are `name@version` across every supported JavaScript package
/// manager, but pnpm keys additionally carry a peer-dependency closure — e.g.
/// `pkg@1.2.3(other@4.5.6)(another@7.8.9)` — and pnpm v5/v6 keys are prefixed
/// with `/`. Both are stripped so the result is always a plain `pkg@1.2.3`
/// style identity. Scoped names (`@scope/pkg@1.2.3`) are handled by locating
/// the version delimiter after the leading scope `@`.
///
/// Returns `None` when no `name@version` delimiter can be found (for example a
/// pnpm v5 `name/version` key), letting the caller record the entry as a
/// parse failure instead of emitting a malformed package.
fn split_identity(display_name: &str) -> Option<(String, String)> {
    // pnpm v5/v6 dependency-path keys are prefixed with a leading slash.
    let trimmed = display_name.strip_prefix('/').unwrap_or(display_name);
    // Drop any peer-dependency closure: `pkg@1.2.3(react@18.0.0)` -> `pkg@1.2.3`.
    let base = trimmed.split('(').next().unwrap_or(trimmed);
    // Scoped names start with '@', so search for the version delimiter after
    // the leading scope '@'. The version is the final `@`-delimited segment
    // (the closure that could reintroduce an '@' has already been removed).
    let search_start = usize::from(base.starts_with('@'));
    let at = base[search_start..].rfind('@').map(|i| i + search_start)?;
    let name = &base[..at];
    let version = &base[at + 1..];
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

#[cfg(test)]
mod tests {
    use super::split_identity;

    #[test]
    fn splits_plain_identity() {
        assert_eq!(
            split_identity("lodash@4.17.21"),
            Some(("lodash".to_string(), "4.17.21".to_string()))
        );
    }

    #[test]
    fn splits_scoped_identity() {
        assert_eq!(
            split_identity("@scope/pkg@1.2.3"),
            Some(("@scope/pkg".to_string(), "1.2.3".to_string()))
        );
    }

    #[test]
    fn strips_pnpm_peer_closure() {
        assert_eq!(
            split_identity("pkg@1.2.3(other@4.5.6)(another@7.8.9)"),
            Some(("pkg".to_string(), "1.2.3".to_string()))
        );
    }

    #[test]
    fn strips_scoped_pnpm_peer_closure() {
        assert_eq!(
            split_identity("@scope/pkg@1.2.3(react@18.0.0)"),
            Some(("@scope/pkg".to_string(), "1.2.3".to_string()))
        );
    }

    #[test]
    fn strips_leading_slash_from_pnpm_key() {
        assert_eq!(
            split_identity("/react-dom@18.2.0(react@18.2.0)"),
            Some(("react-dom".to_string(), "18.2.0".to_string()))
        );
    }

    #[test]
    fn preserves_underscores_in_package_name() {
        assert_eq!(
            split_identity("some_package@1.0.0"),
            Some(("some_package".to_string(), "1.0.0".to_string()))
        );
    }

    #[test]
    fn returns_none_without_version_delimiter() {
        // e.g. a pnpm v5 `name/version` key that uses `/` as its delimiter.
        assert_eq!(split_identity("react-dom/18.2.0"), None);
        assert_eq!(split_identity("no-version"), None);
        assert_eq!(split_identity("@scope/only"), None);
    }
}
