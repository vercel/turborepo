//! Acting on a plan.
//!
//! Every removal starts again from the real repository root. On unix the
//! walk uses directory handles opened with `O_NOFOLLOW`, and the final
//! `unlinkat` never follows a link, so a directory replaced by a symlink
//! after planning makes that removal fail instead of reaching outside the
//! repository. Elsewhere each component is re-checked immediately before
//! the removal.

use crate::plan::{CleanPlan, RemovalKind, counts};

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

fn changed_since_planning() -> std::io::Error {
    std::io::Error::other("it changed since it was planned for removal")
}

impl CleanPlan {
    /// Removes everything in the plan. A failed removal is reported and the
    /// rest still run; directories above a failure are left in place.
    pub fn execute(&self) -> Report {
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
            match remover.remove(&removal.components, removal.kind) {
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

    use rustix::fs::{AtFlags, FileType, Mode, OFlags, open, openat, statat, unlinkat};

    use super::changed_since_planning;
    use crate::plan::RemovalKind;

    const DIRECTORY: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

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

        fn open_parent(&mut self, components: &[String]) -> std::io::Result<&OwnedFd> {
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
                self.parent = Some((components.to_vec(), directory));
            }
            match &self.parent {
                Some((_, directory)) => Ok(directory),
                None => Err(changed_since_planning()),
            }
        }

        pub(super) fn remove(
            &mut self,
            components: &[String],
            kind: RemovalKind,
        ) -> std::io::Result<()> {
            let Some((name, parents)) = components.split_last() else {
                return Err(changed_since_planning());
            };
            let directory = self.open_parent(parents)?;
            let stat = statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)?;
            #[allow(clippy::useless_conversion)]
            let file_type = FileType::from_raw_mode(stat.st_mode.into());
            let expected = match kind {
                RemovalKind::File => {
                    file_type != FileType::Directory && file_type != FileType::Symlink
                }
                RemovalKind::Symlink => file_type == FileType::Symlink,
                RemovalKind::Directory => file_type == FileType::Directory,
            };
            if !expected {
                return Err(changed_since_planning());
            }
            let flags = if kind == RemovalKind::Directory {
                AtFlags::REMOVEDIR
            } else {
                AtFlags::empty()
            };
            unlinkat(directory, name.as_str(), flags)?;
            Ok(())
        }
    }
}

#[cfg(not(unix))]
mod sys {
    use std::path::{Path, PathBuf};

    use super::changed_since_planning;
    use crate::plan::RemovalKind;

    pub(super) struct Remover {
        root: PathBuf,
    }

    fn is_link(metadata: &std::fs::Metadata) -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return true;
            }
        }
        metadata.file_type().is_symlink()
    }

    impl Remover {
        pub(super) fn new(root: &Path) -> Self {
            Self {
                root: root.to_owned(),
            }
        }

        /// Re-verifies every component just before removing, as close to
        /// the removal as path-based APIs allow.
        pub(super) fn remove(
            &mut self,
            components: &[String],
            kind: RemovalKind,
        ) -> std::io::Result<()> {
            let Some((name, parents)) = components.split_last() else {
                return Err(changed_since_planning());
            };
            let mut path = self.root.clone();
            for component in parents {
                path.push(component);
                let metadata = std::fs::symlink_metadata(&path)?;
                if is_link(&metadata) || !metadata.is_dir() {
                    return Err(changed_since_planning());
                }
            }
            path.push(name);
            let metadata = std::fs::symlink_metadata(&path)?;
            match kind {
                RemovalKind::File if !is_link(&metadata) && !metadata.is_dir() => {
                    std::fs::remove_file(&path)
                }
                // A directory link (symlink or junction) needs `remove_dir`,
                // which never recurses into the target.
                RemovalKind::Symlink if is_link(&metadata) => {
                    std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path))
                }
                RemovalKind::Directory if !is_link(&metadata) && metadata.is_dir() => {
                    std::fs::remove_dir(&path)
                }
                _ => Err(changed_since_planning()),
            }
        }
    }
}
