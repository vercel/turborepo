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
    package_graph::{PackageGraph, PackageName},
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

    /// Gets the lockfile content from SCM if it has changed.
    /// Does *not* error if cannot get content.
    ///
    /// Resolution definition paths come from foundational change knowledge
    /// rather than probing the package manager at classification time.
    pub fn get_lockfile_contents(
        &self,
        from_ref: Option<&str>,
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

        debug!("lockfile changed, have the previous content");
        LockfileContents::Changed {
            path: self
                .turbo_root
                .anchor(&lockfile_path)
                .expect("lockfile should be in repo"),
            previous_contents: content,
        }
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

        let lockfile_contents = self.get_lockfile_contents(from_ref, &changed_files);

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
