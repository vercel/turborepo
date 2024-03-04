//! Maps changed files to changed packages in a repository.
//! Used for both `--filter` and for isolated builds.

use std::collections::HashSet;

use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};
use wax::Program;

use crate::package_graph::{ChangedPackagesError, PackageGraph, PackageName, WorkspacePackage};

pub enum PackageDetection {
    /// We've hit a global file, so all packages have changed
    All,
    /// This change is meaningless, no packages have changed
    None,
    /// This change has affected one package
    Package(WorkspacePackage),
}

/// Detects which packages are affected by a file change. This can be a single
/// package (`Package`), none of the packages (`None`), or all of the packages
/// (`All`).
pub trait PackageDetector {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageDetection;
}

const DEFAULT_GLOBAL_DEPS: [&str; 2] = ["package.json", "turbo.json"];

/// Detects package by checking if the file is inside the package.
/// Does *not* use the `globalDependencies` in turbo.json.
/// Since we don't have these dependencies, any file that is
/// not in any package will automatically invalidate all
/// packages. This is fine for builds, but less fine
/// for situations like watch mode.
pub struct DefaultPackageDetector<'a> {
    pkg_dep_graph: &'a PackageGraph,
}

impl<'a> DefaultPackageDetector<'a> {
    pub fn new(pkg_dep_graph: &'a PackageGraph) -> Self {
        Self { pkg_dep_graph }
    }
    fn is_file_in_package(file: &AnchoredSystemPath, package_path: &AnchoredSystemPath) -> bool {
        file.components()
            .zip(package_path.components())
            .all(|(a, b)| a == b)
    }
}

impl<'a> PackageDetector for DefaultPackageDetector<'a> {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageDetection {
        for (name, entry) in self.pkg_dep_graph.packages() {
            if name == &PackageName::Root {
                continue;
            }
            if let Some(package_path) = entry.package_json_path.parent() {
                if Self::is_file_in_package(file, package_path) {
                    return PackageDetection::Package(WorkspacePackage {
                        name: name.clone(),
                        path: package_path.to_owned(),
                    });
                }
            }
        }

        PackageDetection::All
    }
}

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

impl<'a, PD: PackageDetector> ChangeMapper<'a, PD> {
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

    fn default_global_file_changed(
        changed_files: &HashSet<AnchoredSystemPathBuf>,
    ) -> Result<bool, ChangeMapError> {
        Ok(changed_files
            .iter()
            .any(|f| DEFAULT_GLOBAL_DEPS.iter().any(|dep| *dep == f.as_str())))
    }

    pub fn changed_packages(
        &self,
        changed_files: HashSet<AnchoredSystemPathBuf>,
        lockfile_change: Option<LockfileChange>,
    ) -> Result<PackageChanges, ChangeMapError> {
        let global_change = Self::default_global_file_changed(&changed_files)?;

        if global_change {
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
                PackageDetection::Package(pkg) => {
                    changed_packages.insert(pkg);
                }
                PackageDetection::All => {
                    return Ok(PackageChanges::All);
                }
                PackageDetection::None => {}
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
    #[error("SCM error: {0}")]
    Scm(#[from] turborepo_scm::Error),
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
    use crate::change_mapper::DefaultPackageDetector;

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
        let changes = ChangeMapper::<DefaultPackageDetector>::lockfile_changed(
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
        let changes = ChangeMapper::<DefaultPackageDetector>::lockfile_changed(
            &turbo_root,
            &changed_files,
            &lockfile_path,
        );

        // we don't want to implement PartialEq on the error type,
        // so simply compare the debug representations
        assert_eq!(changes, expected);
    }
}
