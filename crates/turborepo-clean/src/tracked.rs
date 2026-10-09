//! Files git tracks. They are source, never build output.

use std::collections::{HashMap, HashSet};

use turbopath::AbsoluteSystemPath;
use turborepo_scm::{GitEnvironment, IndexPaths};

use crate::{
    Error, SkipReason,
    ids::{self, FileId, Identities},
    names::{fold, skeleton},
};

/// A path git owns, and how many of its leading components to report: the
/// whole path for a tracked file, the directory for a submodule or sparse
/// directory.
pub(crate) type Protection = (SkipReason, usize);

pub(crate) struct TrackedFiles {
    /// The repository root relative to the git root, one name per component.
    prefix: Vec<String>,
    index: IndexPaths,
    folded_files: HashSet<String>,
    /// Submodules and sparse directories, folded.
    folded_directories: HashSet<String>,
    /// The identities of the submodule and sparse directories present on
    /// disk.
    directory_ids: HashSet<FileId>,
    /// Tracked names by their directory in the index (`""` for the top).
    by_parent: HashMap<String, Vec<String>>,
    /// Index directories by the skeleton of their path.
    parents_by_skeleton: HashMap<String, Vec<String>>,
    /// The identity of each index directory looked up so far.
    index_directory_ids: HashMap<String, Option<FileId>>,
    /// The identities of the tracked files in each directory checked so far,
    /// keyed by its stored components below the repository root.
    tracked_ids: HashMap<Vec<String>, HashSet<FileId>>,
}

impl TrackedFiles {
    /// Fails closed: outside git, when the index is missing or unreadable,
    /// or when the index does not track the repository's own root manifest
    /// (so it may belong to another repository, such as a dotfiles `~/.git`),
    /// the tracked set is unknown and nothing may be deleted.
    pub(crate) fn load(
        real_repo_root: &AbsoluteSystemPath,
        git: &GitEnvironment,
    ) -> Result<Self, Error> {
        let index = match IndexPaths::read_with(real_repo_root, git) {
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
        let prefix: Vec<String> = index
            .git_root
            .anchor(real_repo_root)?
            .to_unix()
            .as_str()
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        let folded_files: HashSet<String> = index.files.iter().map(|path| fold(path)).collect();
        let folded_directories = index.directories.iter().map(|path| fold(path)).collect();
        let directory_ids = index
            .directories
            .iter()
            .filter_map(|directory| ids::stat(&index.git_root.as_std_path().join(directory)))
            .collect();
        let mut by_parent: HashMap<String, Vec<String>> = HashMap::new();
        for path in &index.files {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            by_parent
                .entry(parent.to_owned())
                .or_default()
                .push(name.to_owned());
        }
        let mut parents_by_skeleton: HashMap<String, Vec<String>> = HashMap::new();
        for parent in by_parent.keys() {
            parents_by_skeleton
                .entry(skeleton(parent))
                .or_default()
                .push(parent.clone());
        }

        let tracked = Self {
            prefix,
            index,
            folded_files,
            folded_directories,
            directory_ids,
            by_parent,
            parents_by_skeleton,
            index_directory_ids: HashMap::new(),
            tracked_ids: HashMap::new(),
        };
        let owns_root = ["package.json", "turbo.json"].iter().any(|manifest| {
            let path = tracked.git_path(&[(*manifest).to_owned()]);
            tracked.index.contains_file(&path) || tracked.folded_files.contains(&fold(&path))
        });
        if !owns_root {
            return Err(Error::TrackedFilesUnknown {
                reason: format!(
                    "the git index {} tracks neither package.json nor turbo.json at \
                     {real_repo_root}, so it may belong to another repository",
                    tracked.index.index_file.display()
                ),
            });
        }
        Ok(tracked)
    }

    /// `components` (below the repository root) relative to the git root.
    fn git_path(&self, components: &[String]) -> String {
        self.prefix
            .iter()
            .chain(components)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Why the stored path at `components` (relative to the repository root)
    /// must be kept, if git owns it.
    ///
    /// Names are compared exactly and, conservatively, folded. Then the
    /// filesystem decides: the path is protected when it is the same file as
    /// a tracked name in its directory, or lies in the same directory as a
    /// submodule or sparse directory, by (device, inode). That follows the
    /// filesystem's own case and Unicode rules, whatever they are.
    pub(crate) fn protects(
        &mut self,
        components: &[String],
        identities: &mut Identities,
    ) -> Option<Protection> {
        let path = self.git_path(components);
        if self.index.contains_file(&path) || self.folded_files.contains(&fold(&path)) {
            return Some((SkipReason::TrackedByGit, components.len()));
        }
        for depth in 1..=components.len() {
            let directory = fold(&self.git_path(&components[..depth]));
            if self.folded_directories.contains(&directory) {
                return Some((SkipReason::InSubmodule, depth));
            }
        }
        if !self.directory_ids.is_empty() {
            for depth in 1..=components.len() {
                if identities
                    .of(&components[..depth])
                    .is_some_and(|id| self.directory_ids.contains(&id))
                {
                    return Some((SkipReason::InSubmodule, depth));
                }
            }
        }
        let id = identities.of(components)?;
        let (_, parent) = components.split_last()?;
        self.tracked_ids_in(parent, identities)
            .contains(&id)
            .then_some((SkipReason::TrackedByGit, components.len()))
    }

    /// The identities of the tracked files in the directory at `parent`:
    /// every index directory that is the same directory on disk, however it
    /// is spelled, contributes the no-follow identity of each of its names.
    fn tracked_ids_in(
        &mut self,
        parent: &[String],
        identities: &mut Identities,
    ) -> &HashSet<FileId> {
        if !self.tracked_ids.contains_key(parent) {
            let mut found = HashSet::new();
            if let Some(parent_id) = identities.of(parent) {
                let mut directory = identities.root().to_owned();
                directory.extend(parent);
                let git_root = self.index.git_root.as_std_path();
                let index_directories = self
                    .parents_by_skeleton
                    .get(&skeleton(&self.git_path(parent)))
                    .into_iter()
                    .flatten();
                for index_directory in index_directories {
                    let id = *self
                        .index_directory_ids
                        .entry(index_directory.clone())
                        .or_insert_with(|| ids::stat(&git_root.join(index_directory)));
                    if id != Some(parent_id) {
                        continue;
                    }
                    for name in &self.by_parent[index_directory] {
                        found.extend(ids::lstat(&directory.join(name)));
                    }
                }
            }
            self.tracked_ids.insert(parent.to_vec(), found);
        }
        &self.tracked_ids[parent]
    }
}
