//! Change detection for affected packages.
//!
//! This module contains logic for detecting which packages have changed
//! based on git diffs.

use std::collections::{HashMap, HashSet};

use tracing::{debug, warn};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{
        AllPackageChangeReason, ChangeMapper, DefaultPackageChangeMapper, Error,
        GlobalDepsPackageChangeMapper, LockfileContents, PackageChanges, PackageInclusionReason,
    },
    package_graph::{ExternalDependencyChange, PackageGraph, PackageName},
};
use turborepo_scm::{Error as ScmError, SCM, git::InvalidRange};

use crate::ResolutionError;

/// Expands an all-packages change to the root Turbo task namespace and every
/// authoritative non-root execution scope.
///
/// The root namespace exists even when a pure native repository has no root
/// JavaScript package. Package and aggregate identities still come exclusively
/// from repository knowledge.
pub(crate) fn all_package_changes(
    pkg_graph: &PackageGraph,
    reason: AllPackageChangeReason,
) -> HashMap<PackageName, PackageInclusionReason> {
    std::iter::once(PackageName::Root)
        .chain(
            pkg_graph
                .package_scope_directories()
                .map(|(name, _)| name)
                .filter(|name| name != &PackageName::Root),
        )
        .map(|name| (name, PackageInclusionReason::All(reason.clone())))
        .collect()
}

/// Resolves raw changed files for an affected comparison. The merge-base and
/// working-tree policies are explicit so run and query callers can use the
/// same SCM behavior while tests provide fixed file sets.
pub trait ChangedFilesDetector {
    fn changed_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<Result<HashSet<AnchoredSystemPathBuf>, InvalidRange>, ScmError>;

    /// Resolves task input changes without losing tracked-file provenance.
    fn task_input_changes(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<Result<turborepo_types::TaskInputChanges, InvalidRange>, ScmError>;

    /// Describes how the lockfile changed between `from_ref` and `to_ref`,
    /// using the same snapshot comparison as package-level affected
    /// detection. `changed_files` must be the change set for that range.
    ///
    /// Detectors without lockfile history report an unknown change, which
    /// callers treat as affecting every package.
    fn lockfile_contents(
        &self,
        _turbo_root: &AbsoluteSystemPath,
        _pkg_graph: &PackageGraph,
        _from_ref: Option<&str>,
        _to_ref: Option<&str>,
        _include_uncommitted: bool,
        _changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> LockfileContents {
        LockfileContents::UnknownChange
    }
}

impl ChangedFilesDetector for SCM {
    fn changed_files(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<Result<HashSet<AnchoredSystemPathBuf>, InvalidRange>, ScmError> {
        SCM::changed_files(
            self,
            turbo_root,
            from_ref,
            to_ref,
            include_uncommitted,
            allow_unknown_objects,
            merge_base,
        )
    }

    fn task_input_changes(
        &self,
        turbo_root: &AbsoluteSystemPath,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<Result<turborepo_types::TaskInputChanges, InvalidRange>, ScmError> {
        let files = match self.changed_files(
            turbo_root,
            from_ref,
            to_ref,
            include_uncommitted,
            allow_unknown_objects,
            merge_base,
        )? {
            Ok(files) => files,
            Err(range) => return Ok(Err(range)),
        };
        if !include_uncommitted || files.is_empty() {
            return Ok(Ok(turborepo_types::TaskInputChanges::from_tracked(files)));
        }
        let mut tracked = match self.changed_files(
            turbo_root,
            from_ref,
            to_ref,
            false,
            allow_unknown_objects,
            merge_base,
        )? {
            Ok(files) => files,
            Err(range) => return Ok(Err(range)),
        };
        tracked.extend(self.tracked_changed_files(turbo_root, &files)?);
        Ok(Ok(turborepo_types::TaskInputChanges { files, tracked }))
    }

    fn lockfile_contents(
        &self,
        turbo_root: &AbsoluteSystemPath,
        pkg_graph: &PackageGraph,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> LockfileContents {
        LockfileChangeReader {
            turbo_root,
            scm: self,
            pkg_graph,
        }
        .lockfile_contents(from_ref, to_ref, include_uncommitted, changed_files)
    }
}

/// Given two git refs, determine which packages have changed between them.
pub trait GitChangeDetector {
    /// Determine which packages have changed between two git refs.
    ///
    /// # Arguments
    /// * `from_ref` - Starting git ref (e.g., "HEAD~1", "main")
    /// * `to_ref` - Ending git ref; if None with include_uncommitted, uses
    ///   working tree
    /// * `include_uncommitted` - Include uncommitted changes in the diff
    /// * `allow_unknown_objects` - Treat unknown git objects as "all changed"
    ///   instead of error
    /// * `merge_base` - Calculate diff from merge-base of the two refs
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError>;
}

/// Detects changed packages based on SCM state.
pub struct ScopeChangeDetector<'a> {
    turbo_root: &'a AbsoluteSystemPath,
    change_mapper: ChangeMapper<'a, GlobalDepsPackageChangeMapper<'a>>,
    scm: &'a SCM,
    pkg_graph: &'a PackageGraph,
}

impl<'a> ScopeChangeDetector<'a> {
    pub fn new(
        turbo_root: &'a AbsoluteSystemPath,
        scm: &'a SCM,
        pkg_graph: &'a PackageGraph,
        global_deps: impl Iterator<Item = &'a str>,
        ignore_patterns: Vec<String>,
    ) -> Result<Self, Error> {
        let pkg_detector = GlobalDepsPackageChangeMapper::new(pkg_graph, global_deps)?;
        let change_mapper = ChangeMapper::new(pkg_graph, ignore_patterns, pkg_detector);

        Ok(Self {
            turbo_root,
            change_mapper,
            scm,
            pkg_graph,
        })
    }

    pub fn with_managed_setup(mut self, managed_setup: bool) -> Self {
        self.change_mapper = self.change_mapper.with_managed_setup(managed_setup);
        self
    }

    /// Gets the lockfile content from SCM if it has changed.
    /// Does *not* error if cannot get content.
    pub fn get_lockfile_contents(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> LockfileContents {
        LockfileChangeReader {
            turbo_root: self.turbo_root,
            scm: self.scm,
            pkg_graph: self.pkg_graph,
        }
        .lockfile_contents(from_ref, to_ref, include_uncommitted, changed_files)
    }
}

/// Reads lockfile snapshots from SCM and describes how the lockfile changed
/// between two refs. Shared by package-level (`ScopeChangeDetector`) and
/// task-level (`ChangedFilesDetector::lockfile_contents`) affected detection
/// so both scope lockfile changes identically.
struct LockfileChangeReader<'a> {
    turbo_root: &'a AbsoluteSystemPath,
    scm: &'a SCM,
    pkg_graph: &'a PackageGraph,
}

impl LockfileChangeReader<'_> {
    /// Gets the lockfile content from SCM if it has changed.
    /// Does *not* error if cannot get content.
    ///
    /// Resolution definition paths come from foundational change knowledge
    /// rather than probing the package manager at classification time.
    fn lockfile_contents(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> LockfileContents {
        let resolution_paths = self.pkg_graph.change_knowledge().resolution_paths();
        if resolution_paths.is_empty() {
            return LockfileContents::Unchanged;
        }

        let Some(lockfile_path) = resolution_paths.iter().find_map(|path| {
            let relative = turbopath::RelativeUnixPath::new(path).ok()?;
            let absolute = self.turbo_root.join_unix_path(relative);
            ChangeMapper::<DefaultPackageChangeMapper>::lockfile_changed(
                self.turbo_root,
                changed_files,
                &absolute,
            )
            .then_some(absolute)
        }) else {
            debug!("lockfile did not change");
            return LockfileContents::Unchanged;
        };

        let Ok(content) = self.scm.previous_content(from_ref, &lockfile_path) else {
            debug!("lockfile did change but could not get previous content");
            return LockfileContents::UnknownChange;
        };

        let Ok(path) = self.turbo_root.anchor(&lockfile_path) else {
            debug!("lockfile changed but was outside the repository");
            return LockfileContents::UnknownChange;
        };
        // The package graph is built from the working tree. When that tree is
        // the head snapshot, comparing against it is exact and avoids reading
        // every manifest from git.
        if let Some(to_ref) = to_ref.filter(|to_ref| !self.checkout_matches(to_ref)) {
            return match self.lockfile_changes_at(
                to_ref,
                &path,
                &lockfile_path,
                &content,
                include_uncommitted,
            ) {
                Some(changes) => LockfileContents::Resolved(changes),
                None => LockfileContents::UnknownChange,
            };
        }
        debug!("lockfile changed, have the previous content");
        LockfileContents::Changed {
            path,
            previous_contents: content,
        }
    }

    fn checkout_matches(&self, to_ref: &str) -> bool {
        self.scm
            .changed_files(self.turbo_root, Some(to_ref), None, true, false, false)
            .is_ok_and(|result| result.is_ok_and(|files| files.is_empty()))
            && self.pkg_graph.dependency_configuration_matches(|file| {
                self.scm.previous_content(Some(to_ref), file).ok()
            })
    }

    fn lockfile_changes_at(
        &self,
        to_ref: &str,
        path: &AnchoredSystemPathBuf,
        lockfile_path: &AbsoluteSystemPath,
        previous: &[u8],
        include_uncommitted: bool,
    ) -> Option<Vec<ExternalDependencyChange>> {
        let current = self
            .scm
            .previous_content(Some(to_ref), lockfile_path)
            .ok()?;
        let mut changes = self
            .pkg_graph
            .changed_packages_from_lockfile_snapshots(path, previous, &current, |file| {
                self.scm.previous_content(Some(to_ref), file).ok()
            })
            .inspect_err(|error| debug!(%error, "unable to resolve the head lockfile snapshot"))
            .ok()?;
        if include_uncommitted {
            // File selection unions base..head with local changes relative to
            // the checkout. Compare those lockfile changes separately, so later
            // committed dependencies do not leak into base..head.
            let checkout = self
                .scm
                .previous_content(Some("HEAD"), lockfile_path)
                .ok()?;
            let working = std::fs::read(lockfile_path).ok()?;
            if checkout != working {
                changes.extend(
                    self.pkg_graph
                        .changed_packages_from_lockfile_contents(path, &checkout)
                        .ok()?,
                );
            }
        }
        Some(changes)
    }
}

impl<'a> GitChangeDetector for ScopeChangeDetector<'a> {
    /// get the actual changed packages between two git refs
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        let changed_files = match self.scm.changed_files(
            self.turbo_root,
            from_ref,
            to_ref,
            include_uncommitted,
            allow_unknown_objects,
            merge_base,
        ) {
            Ok(Ok(changed_files)) => changed_files,
            Ok(Err(InvalidRange { from_ref, to_ref })) => {
                debug!("invalid ref range, defaulting to all packages changed");
                return Ok(all_package_changes(
                    self.pkg_graph,
                    AllPackageChangeReason::GitRefNotFound { from_ref, to_ref },
                ));
            }
            Err(ScmError::Path(err, _)) => {
                warn!(
                    "SCM path error while detecting changed files: {err}. Defaulting to all \
                     packages changed."
                );
                return Ok(all_package_changes(
                    self.pkg_graph,
                    AllPackageChangeReason::ScmError {
                        error: err.to_string(),
                    },
                ));
            }
            Err(err) => return Err(err.into()),
        };

        let lockfile_contents =
            self.get_lockfile_contents(from_ref, to_ref, include_uncommitted, &changed_files);

        debug!(
            "changed files: {:?}",
            changed_files.iter().map(|x| x.as_str()).collect::<Vec<_>>()
        );

        match self
            .change_mapper
            .changed_packages(changed_files, lockfile_contents)?
        {
            PackageChanges::All(reason) => {
                debug!("all packages changed: {:?}", reason);
                Ok(all_package_changes(self.pkg_graph, reason))
            }
            PackageChanges::Some(packages) => {
                debug!(
                    "{} packages changed: {:?}",
                    packages.len(),
                    packages
                        .keys()
                        .map(|x| x.name.to_string())
                        .collect::<Vec<String>>()
                );

                Ok(packages
                    .iter()
                    .map(|(package, reason)| (package.name.clone(), reason.clone()))
                    .collect())
            }
        }
    }
}
