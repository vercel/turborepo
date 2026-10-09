//! Acting on a plan.
//!
//! Only on unix. Every removal starts again from the real repository root,
//! walking with directory handles opened with `O_NOFOLLOW`, and the final
//! `unlinkat` never follows a link, so a directory replaced by a symlink
//! after planning makes that removal fail instead of reaching outside the
//! repository. Each opened directory and each entry is checked against the
//! (device, inode) recorded when it was planned, so a directory renamed into
//! a planned path is not emptied in its place.
//!
//! Elsewhere (Windows) nothing is deleted: path-based removal cannot rule
//! out a junction swapped in between the check and the removal.

use crate::{
    Error,
    plan::{CleanPlan, counts},
};

/// A removal that failed. Other removals still ran.
#[derive(Debug)]
pub struct Failure {
    pub path: String,
    pub error: std::io::Error,
}

/// What a clean actually did.
#[derive(Debug, Default)]
pub struct Report {
    /// Files and symlinks removed.
    pub files: usize,
    pub directories: usize,
    pub failures: Vec<Failure>,
    /// Directories kept because something appeared in them after planning.
    pub not_empty: Vec<String>,
}

impl Report {
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }

    /// The lines `turbo clean` prints after removing.
    pub fn describe(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .failures
            .iter()
            .map(|failure| format!("x Failed to remove {}: {}", failure.path, failure.error))
            .collect();
        lines.extend(
            self.not_empty
                .iter()
                .map(|path| format!("• Not removing {path} (no longer empty)")),
        );
        lines.push(format!("Removed {}.", counts(self.files, self.directories)));
        lines
    }
}

#[cfg(unix)]
fn changed_since_planning() -> std::io::Error {
    std::io::Error::other("it changed since it was planned for removal")
}

/// Whether this platform can delete: `Ok` on unix. Elsewhere,
/// [`Error::DeletionUnsupported`]; dry runs still work.
pub fn ensure_deletion_supported() -> Result<(), Error> {
    if cfg!(unix) {
        Ok(())
    } else {
        Err(Error::DeletionUnsupported)
    }
}

impl CleanPlan {
    /// Removes everything in the plan. A failed removal is reported and the
    /// rest still run; directories above a failure are left in place.
    ///
    /// Errors without removing anything where deletion is unsupported
    /// ([`ensure_deletion_supported`]).
    pub fn execute(&self) -> Result<Report, Error> {
        ensure_deletion_supported()?;
        #[cfg(unix)]
        {
            Ok(self.remove_all())
        }
        #[cfg(not(unix))]
        {
            unreachable!("deletion is refused above")
        }
    }

    #[cfg(unix)]
    fn remove_all(&self) -> Report {
        use crate::plan::RemovalKind;

        let mut report = Report::default();
        let mut remover = sys::Remover::new(self.real_root.as_std_path());
        let mut failed: Vec<&[String]> = Vec::new();
        for removal in &self.removals {
            if removal.kind == RemovalKind::Directory
                && failed
                    .iter()
                    .any(|path| path.starts_with(&removal.components))
            {
                continue;
            }
            match remover.remove(removal) {
                Ok(()) => match removal.kind {
                    RemovalKind::Directory => report.directories += 1,
                    RemovalKind::File | RemovalKind::Symlink => report.files += 1,
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error)
                    if removal.kind == RemovalKind::Directory
                        && error.kind() == std::io::ErrorKind::DirectoryNotEmpty =>
                {
                    report.not_empty.push(removal.display());
                }
                Err(error) => {
                    failed.push(&removal.components);
                    report.failures.push(Failure {
                        path: removal.display(),
                        error,
                    });
                }
            }
        }
        report
    }
}

#[cfg(unix)]
mod sys {
    use std::{os::fd::OwnedFd, path::PathBuf};

    use rustix::fs::{
        AtFlags, FileType, Mode, OFlags, Stat, fstat, open, openat, statat, unlinkat,
    };

    use super::changed_since_planning;
    use crate::{
        ids::FileId,
        plan::{Removal, RemovalKind},
    };

    const DIRECTORY: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    fn id(stat: &Stat) -> FileId {
        FileId {
            dev: stat.st_dev as u64,
            ino: stat.st_ino as u64,
        }
    }

    /// Errors unless what was opened or found is what was planned.
    fn verify(stat: &Stat, planned: Option<FileId>) -> std::io::Result<()> {
        match planned {
            Some(planned) if planned == id(stat) => Ok(()),
            _ => Err(changed_since_planning()),
        }
    }

    pub(super) struct Remover {
        root: PathBuf,
        /// The most recently opened parent, reused by its siblings.
        parent: Option<(Vec<String>, OwnedFd)>,
    }

    impl Remover {
        pub(super) fn new(root: &std::path::Path) -> Self {
            Self {
                root: root.to_owned(),
                parent: None,
            }
        }

        fn open_parent(
            &mut self,
            components: &[String],
            planned: Option<FileId>,
        ) -> std::io::Result<&OwnedFd> {
            let cached = self
                .parent
                .as_ref()
                .is_some_and(|(path, _)| path.as_slice() == components);
            if !cached {
                self.parent = None;
                let mut directory = open(&self.root, DIRECTORY, Mode::empty())?;
                for component in components {
                    directory = openat(&directory, component.as_str(), DIRECTORY, Mode::empty())?;
                }
                verify(&fstat(&directory)?, planned)?;
                self.parent = Some((components.to_vec(), directory));
            }
            match &self.parent {
                Some((_, directory)) => Ok(directory),
                None => Err(changed_since_planning()),
            }
        }

        pub(super) fn remove(&mut self, removal: &Removal) -> std::io::Result<()> {
            let Some((name, parents)) = removal.components.split_last() else {
                return Err(changed_since_planning());
            };
            let directory = self.open_parent(parents, removal.parent_id)?;
            let stat = statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)?;
            #[allow(clippy::useless_conversion)]
            let file_type = FileType::from_raw_mode(stat.st_mode.into());
            let expected = match removal.kind {
                RemovalKind::File => {
                    file_type != FileType::Directory && file_type != FileType::Symlink
                }
                RemovalKind::Symlink => file_type == FileType::Symlink,
                RemovalKind::Directory => file_type == FileType::Directory,
            };
            if !expected {
                return Err(changed_since_planning());
            }
            verify(&stat, removal.id)?;
            let flags = if removal.kind == RemovalKind::Directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            };
            unlinkat(directory, name.as_str(), flags)?;
            Ok(())
        }
    }
}
