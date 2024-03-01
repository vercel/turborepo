//! Maps changed files to changed packages in a repository.
//! Used for both `--filter` and for isolated builds.

use std::collections::HashSet;

pub use package::{
    DefaultPackageChangeMapper, GlobalDepsPackageChangeMapper, PackageChangeMapper, PackageMapping,
};
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use wax::Program;

use crate::package_graph::{ChangedPackagesError, PackageGraph, WorkspacePackage};

mod package;

const DEFAULT_GLOBAL_DEPS: [&str; 2] = ["package.json", "turbo.json"];

// We may not be able to load the lockfile contents, but we
// still want to be able to express a generic change.
pub enum LockfileChange {
    Empty,
    WithContent(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum PackageChanges {
    All,
    Some(HashSet<WorkspacePackage>),
}

pub struct ChangeMapper<'a, PD> {
    pkg_graph: &'a PackageGraph,

    ignore_patterns: Vec<String>,
    package_detector: PD,
}

impl<'a, PD: PackageChangeMapper> ChangeMapper<'a, PD> {
    pub fn new(
        pkg_graph: &'a PackageGraph,
        ignore_patterns: Vec<String>,
        package_detector: PD,
    ) -> Self {
        Self {
            pkg_graph,
            ignore_patterns,
            package_detector,
        }
    }

    fn default_global_file_changed(changed_files: &HashSet<AnchoredSystemPathBuf>) -> bool {
        changed_files
            .iter()
            .any(|f| DEFAULT_GLOBAL_DEPS.iter().any(|dep| *dep == f.as_str()))
    }

    pub fn changed_packages(
        &self,
        changed_files: HashSet<AnchoredSystemPathBuf>,
        lockfile_change: Option<LockfileChange>,
    ) -> Result<PackageChanges, ChangeMapError> {
        if Self::default_global_file_changed(&changed_files) {
            return Ok(PackageChanges::All);
        }

        // get filtered files and add the packages that contain them
        let filtered_changed_files = self.filter_ignored_files(changed_files.iter())?;
        let PackageChanges::Some(mut changed_pkgs) =
            self.get_changed_packages(filtered_changed_files.into_iter())?
        else {
            return Ok(PackageChanges::All);
        };

        match lockfile_change {
            Some(LockfileChange::WithContent(content)) => {
                // if we run into issues, don't error, just assume all packages have changed
                let Ok(lockfile_changes) = self.get_changed_packages_from_lockfile(content) else {
                    return Ok(PackageChanges::All);
                };

                changed_pkgs.extend(lockfile_changes);

                Ok(PackageChanges::Some(changed_pkgs))
            }
            // We don't have the actual contents, so just invalidate everything
            Some(LockfileChange::Empty) => Ok(PackageChanges::All),
            None => Ok(PackageChanges::Some(changed_pkgs)),
        }
    }

    fn filter_ignored_files<'b>(
        &self,
        changed_files: impl Iterator<Item = &'b AnchoredSystemPathBuf> + 'b,
    ) -> Result<HashSet<&'b AnchoredSystemPathBuf>, ChangeMapError> {
        let matcher = wax::any(self.ignore_patterns.iter().map(|s| s.as_str()))?;
        Ok(changed_files
            .filter(move |f| !matcher.is_match(f.as_path()))
            .collect())
    }

    // note: this could probably be optimized by using a hashmap of package paths
    fn get_changed_packages<'b>(
        &self,
        files: impl Iterator<Item = &'b AnchoredSystemPathBuf>,
    ) -> Result<PackageChanges, turborepo_scm::Error> {
        let mut changed_packages = HashSet::new();
        for file in files {
            match self.package_detector.detect_package(file) {
                PackageMapping::Package(pkg) => {
                    changed_packages.insert(pkg);
                }
                PackageMapping::All => {
                    return Ok(PackageChanges::All);
                }
                PackageMapping::None => {}
            }
        }

        Ok(PackageChanges::Some(changed_packages))
    }

    fn get_changed_packages_from_lockfile(
        &self,
        lockfile_content: Vec<u8>,
    ) -> Result<Vec<WorkspacePackage>, ChangeMapError> {
        let previous_lockfile = self
            .pkg_graph
            .package_manager()
            .parse_lockfile(self.pkg_graph.root_package_json(), &lockfile_content)?;

        let additional_packages = self
            .pkg_graph
            .changed_packages_from_lockfile(previous_lockfile.as_ref())?;

        Ok(additional_packages)
    }

    pub fn lockfile_changed(
        turbo_root: &AbsoluteSystemPath,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
        lockfile_path: &AbsoluteSystemPath,
    ) -> bool {
        let lockfile_path_relative = turbo_root
            .anchor(lockfile_path)
            .expect("lockfile should be in repo");

        changed_files.iter().any(|f| f == &lockfile_path_relative)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ChangeMapError {
    #[error(transparent)]
    Wax(#[from] wax::BuildError),
    #[error("Package manager error: {0}")]
    PackageManager(#[from] crate::package_manager::Error),
    #[error("No lockfile")]
    NoLockfile,
    #[error("Lockfile error: {0}")]
    Lockfile(turborepo_lockfiles::Error),
}

impl From<ChangedPackagesError> for ChangeMapError {
    fn from(value: ChangedPackagesError) -> Self {
        match value {
            ChangedPackagesError::NoLockfile => Self::NoLockfile,
            ChangedPackagesError::Lockfile(e) => Self::Lockfile(e),
        }
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::ChangeMapper;
    use crate::change_mapper::package::DefaultPackageChangeMapper;

    #[cfg(unix)]
    #[test_case("/a/b/c", &["package.lock"], "/a/b/c/package.lock", true ; "simple")]
    #[test_case("/a/b/c", &["a", "b", "c"], "/a/b/c/package.lock", false ; "lockfile unchanged")]
    fn test_lockfile_changed(
        turbo_root: &str,
        changed_files: &[&str],
        lockfile_path: &str,
        expected: bool,
    ) {
        let turbo_root = turbopath::AbsoluteSystemPathBuf::new(turbo_root).unwrap();
        let lockfile_path = turbopath::AbsoluteSystemPathBuf::new(lockfile_path).unwrap();
        let changed_files = changed_files
            .iter()
            .map(|s| turbopath::AnchoredSystemPathBuf::from_raw(s).unwrap())
            .collect();
        let changes = ChangeMapper::<DefaultPackageChangeMapper>::lockfile_changed(
            &turbo_root,
            &changed_files,
            &lockfile_path,
        );

        assert_eq!(changes, expected);
    }

    #[cfg(windows)]
    #[test_case("C:\\\\a\\b\\c", &["package.lock"], "C:\\\\a\\b\\c\\package.lock", true ; "simple")]
    #[test_case("C:\\\\a\\b\\c", &["a", "b", "c"],  "C:\\\\a\\b\\c\\package.lock", false ; "lockfile unchanged")]
    fn test_lockfile_changed(
        turbo_root: &str,
        changed_files: &[&str],
        lockfile_path: &str,
        expected: bool,
    ) {
        let turbo_root = turbopath::AbsoluteSystemPathBuf::new(turbo_root).unwrap();
        let lockfile_path = turbopath::AbsoluteSystemPathBuf::new(lockfile_path).unwrap();
        let changed_files = changed_files
            .iter()
            .map(|s| turbopath::AnchoredSystemPathBuf::from_raw(s).unwrap())
            .collect();
        let changes = ChangeMapper::<DefaultPackageChangeMapper>::lockfile_changed(
            &turbo_root,
            &changed_files,
            &lockfile_path,
        );

        // we don't want to implement PartialEq on the error type,
        // so simply compare the debug representations
        assert_eq!(changes, expected);
    }
}
