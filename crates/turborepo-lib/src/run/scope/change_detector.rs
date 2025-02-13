use std::collections::{HashMap, HashSet};

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{
        AllPackageChangeReason, ChangeMapper, DefaultPackageChangeMapper, Error,
        GlobalDepsPackageChangeMapper, LockfileContents, PackageChanges, PackageInclusionReason,
    },
    package_graph::{PackageGraph, PackageName},
};
use turborepo_scm::{git::InvalidRange, SCM};

use crate::run::scope::ResolutionError;

/// Given two git refs, determine which packages have changed between them.
pub trait GitChangeDetector {
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError>;
}

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
    pub fn get_lockfile_contents(
        &self,
        from_ref: Option<&str>,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> LockfileContents {
        let lockfile_path = self
            .pkg_graph
            .package_manager()
            .lockfile_path(self.turbo_root);

        if !ChangeMapper::<DefaultPackageChangeMapper>::lockfile_changed(
            self.turbo_root,
            changed_files,
            &lockfile_path,
        ) {
            debug!("lockfile did not change");
            return LockfileContents::Unchanged;
        }

        let Ok(content) = self.scm.previous_content(from_ref, &lockfile_path) else {
            debug!("lockfile did change but could not get previous content");
            return LockfileContents::UnknownChange;
        };

        debug!("lockfile changed, have the previous content");
        LockfileContents::Changed(content)
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
        )? {
            Err(InvalidRange { from_ref, to_ref }) => {
                debug!("invalid ref range, defaulting to all packages changed");
                return Ok(self
                    .pkg_graph
                    .packages()
                    .map(|(name, _)| {
                        (
                            name.to_owned(),
                            PackageInclusionReason::All(AllPackageChangeReason::GitRefNotFound {
                                from_ref: from_ref.clone(),
                                to_ref: to_ref.clone(),
                            }),
                        )
                    })
                    .collect());
            }
            Ok(changed_files) => changed_files,
        };

        let lockfile_contents = self.get_lockfile_contents(from_ref, &changed_files);

        debug!(
            "changed files: {:?}",
            &changed_files
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>()
        );

        match self
            .change_mapper
            .changed_packages(changed_files, lockfile_contents)?
        {
            PackageChanges::All(reason) => {
                debug!("all packages changed: {:?}", reason);
                Ok(self
                    .pkg_graph
                    .packages()
                    .map(|(name, _)| (name.to_owned(), PackageInclusionReason::All(reason.clone())))
                    .collect())
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
