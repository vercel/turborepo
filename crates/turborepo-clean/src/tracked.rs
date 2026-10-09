//! Files git tracks. They are source, never build output.

use std::collections::HashSet;

use turbopath::AbsoluteSystemPath;
use turborepo_scm::IndexPaths;

use crate::{Error, SkipReason, names::fold};

pub(crate) struct TrackedFiles {
    /// The repository root relative to the git root, one name per component.
    prefix: Vec<String>,
    index: IndexPaths,
    folded_files: HashSet<String>,
    folded_directories: Vec<String>,
}

impl TrackedFiles {
    /// Fails closed: outside git, or when the index is missing or unreadable,
    /// the tracked set is unknown and nothing may be deleted.
    pub(crate) fn load(real_repo_root: &AbsoluteSystemPath) -> Result<Self, Error> {
        let index = match IndexPaths::read(real_repo_root) {
            Ok(Some(index)) => index,
            Ok(None) => {
                return Err(Error::TrackedFilesUnknown {
                    reason: format!("{real_repo_root} is not inside a git repository"),
                });
            }
            Err(error) => {
                return Err(Error::TrackedFilesUnknown {
                    reason: error.to_string(),
                });
            }
        };
        let prefix = index
            .git_root
            .anchor(real_repo_root)?
            .to_unix()
            .as_str()
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        let folded_files = index.files.iter().map(|path| fold(path)).collect();
        let folded_directories = index.directories.iter().map(|path| fold(path)).collect();
        Ok(Self {
            prefix,
            index,
            folded_files,
            folded_directories,
        })
    }

    /// Why the stored path at `components` (relative to the repository root)
    /// must be kept, if git owns it. Compares exactly and, to be safe on
    /// case- or normalization-insensitive filesystems, folded.
    pub(crate) fn protects(&self, components: &[String]) -> Option<SkipReason> {
        let path = self
            .prefix
            .iter()
            .chain(components)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("/");
        let folded = fold(&path);
        if self.index.contains_file(&path) || self.folded_files.contains(&folded) {
            return Some(SkipReason::TrackedByGit);
        }
        let in_submodule = self.folded_directories.iter().any(|directory| {
            folded == *directory
                || folded
                    .strip_prefix(directory.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        in_submodule.then_some(SkipReason::InSubmodule)
    }
}
