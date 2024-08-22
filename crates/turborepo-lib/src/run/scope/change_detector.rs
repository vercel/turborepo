use std::collections::HashSet;

use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{ChangeMapper, DefaultPackageChangeMapper, LockfileChange, PackageChanges},
    package_graph::{PackageGraph, PackageName},
};
use turborepo_scm::{git::ChangedFiles, SCM};

use crate::{
    global_deps_package_change_mapper::{Error, GlobalDepsPackageChangeMapper},
    run::scope::ResolutionError,
};

/// Given two git refs, determine which packages have changed between them.
pub trait GitChangeDetector {
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
    ) -> Result<HashSet<PackageName>, ResolutionError>;
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
    /// Does *not* error if cannot get content, instead just
    /// returns an empty lockfile change
    fn get_lockfile_contents(
        &self,
        from_ref: Option<&str>,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> Option<LockfileChange> {
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
            return None;
        }

        let lockfile_path = self
            .pkg_graph
            .package_manager()
            .lockfile_path(self.turbo_root);

        let Ok(content) = self.scm.previous_content(from_ref, &lockfile_path) else {
            return Some(LockfileChange::Empty);
        };

        Some(LockfileChange::WithContent(content))
    }
}

impl<'a> GitChangeDetector for ScopeChangeDetector<'a> {
    fn changed_packages(
        &self,
        from_ref: Option<&str>,
        to_ref: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
    ) -> Result<HashSet<PackageName>, ResolutionError> {
        let changed_files = match self.scm.changed_files(
            self.turbo_root,
            from_ref,
            to_ref,
            include_uncommitted,
            allow_unknown_objects,
        )? {
            ChangedFiles::All => {
                debug!("all packages changed");
                return Ok(self
                    .pkg_graph
                    .packages()
                    .map(|(name, _)| name.to_owned())
                    .collect());
            }
            ChangedFiles::Some(changed_files) => changed_files,
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
                    .map(|(name, _)| name.to_owned())
                    .collect())
            }
            PackageChanges::Some(packages) => {
                debug!(
                    "{} packages changed: {:?}",
                    packages.len(),
                    &packages
                        .iter()
                        .map(|x| x.name.to_string())
                        .collect::<Vec<String>>()
                );

                Ok(packages
                    .iter()
                    .map(|package| package.name.to_owned())
                    .collect())
            }
        }
    }
}
