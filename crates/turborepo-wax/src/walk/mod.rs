//! Traversal and matching of files and directory trees.
//!
//! This module provides APIs for walking directory trees and matching files in
//! a directory tree against [`Program`]s using [`Iterator`]s. These iterators
//! implement [`FileIterator`], which supports efficient filtering that can
//! cancel traversal into sub-trees that are discarded by combinators.
//!
//! # Examples
//!
//! To iterate over the files in a directory tree, use the [`PathExt`] trait.
//!
//! ```rust,no_run
//! use std::path::Path;
//! use wax::walk::{Entry, PathExt as _};
//!
//! let root = Path::new(".config");
//! for entry in root.walk() {
//!     let entry = entry.unwrap();
//!     println!("{:?}", entry.path());
//! }
//! ```
//!
//! To match a [`Glob`] against a directory tree, use [`Glob::walk`]. This
//! function constructs an iterator that efficiently matches a [`Glob`] against
//! the paths of files read from a directory tree.
//!
//! ```rust,no_run
//! use wax::walk::Entry;
//! use wax::Glob;
//!
//! let glob = Glob::new("**/src/**").unwrap();
//! for entry in glob.walk("projects") {
//!     let entry = entry.unwrap();
//!     println!("{:?}", entry.path());
//! }
//! ```
//!
//! Any [`FileIterator`] (the iterators constructed by [`Glob::walk`],
//! [`PathExt::walk`], etc.) can be efficiently filtered. This filtering can
//! cancel traversal into sub-trees that are discarded. To filter files using
//! [`Program`]s, use the [`not`] combinator.
//!
//! ```rust,no_run
//! use std::path::Path;
//! use wax::walk::{Entry, FileIterator, PathExt as _};
//!
//! let root = Path::new(".config");
//! for entry in root.walk().not(["**/*.xml"]).unwrap() {
//!     let entry = entry.unwrap();
//!     println!("{:?}", entry.path());
//! }
//! ```
//!
//! More arbitrary (non-nominal) filtering is also possible via the
//! [`filter_entry`] combinator.
//!
//! [`FileIterator`]: crate::walk::FileIterator
//! [`filter_entry`]: crate::walk::FileIterator::filter_entry
//! [`Glob`]: crate::Glob
//! [`Glob::walk`]: crate::Glob::walk
//! [`Iterator`]: std::iter::Iterator
//! [`not`]: crate::walk::FileIterator::not
//! [`PathExt`]: crate::walk::PathExt
//! [`PathExt::walk`]: crate::walk::PathExt::walk
//! [`Program`]: crate::Program

#![cfg(feature = "walk")]
#![cfg_attr(docsrs, doc(cfg(feature = "walk")))]

mod filter;
mod glob;

use std::{
    fs,
    fs::{FileType, Metadata},
    io,
    io::ErrorKind,
    path::{Path, PathBuf},
    rc::Rc,
};

use thiserror::Error;
use walkdir::{DirEntry, Error, WalkDir};

pub use crate::walk::glob::{GlobEntry, GlobWalker};
use crate::{
    walk::{
        filter::{
            CancelWalk, HierarchicalIterator, Isomeric, SeparatingFilter, SeparatingFilterInput,
            Separation, TreeResidue, WalkCancellation,
        },
        glob::FilterAny,
    },
    BuildError, Pattern,
};

type FileFiltrate<T> = Result<T, WalkError>;
type FileResidue<R> = TreeResidue<R>;
type FileFeed<T, R> = (FileFiltrate<T>, FileResidue<R>);

impl<T, R> Isomeric for (T, FileResidue<R>)
where
    T: Entry,
    R: Entry,
{
    // TODO: Using a trait object here is very flexible, but incurs a slight
    // performance penalty.       At time of writing, there are no public APIs
    // that allow mapping of the entry types of       separating filters, so
    // this flexibility may not be worth its cost. The alternative is
    //       to use `TreeEntry` as the substituent and require that `T` is
    // `AsRef<TreeEntry>` or       similar. This does not require dynamic
    // dispatch, but places more restrictive       constraints on entry types.
    // Revisit this.
    type Substituent<'a> = &'a dyn Entry
    where
        Self: 'a;

    fn substituent(separation: &Separation<Self>) -> Self::Substituent<'_> {
        match separation {
            Separation::Filtrate(ref filtrate) => filtrate.get(),
            Separation::Residue(ref residue) => residue.get().get(),
        }
    }
}

trait SplitAtDepth {
    fn split_at_depth(&self, depth: usize) -> (&Path, &Path);
}

impl SplitAtDepth for Path {
    fn split_at_depth(&self, depth: usize) -> (&Path, &Path) {
        let ancestor = self.ancestors().nth(depth).unwrap_or(Path::new(""));
        let descendant = self.strip_prefix(ancestor).unwrap();
        (ancestor, descendant)
    }
}

trait JoinAndGetDepth {
    fn join_and_get_depth(&self, path: impl AsRef<Path>) -> (PathBuf, usize);
}

impl JoinAndGetDepth for Path {
    fn join_and_get_depth(&self, path: impl AsRef<Path>) -> (PathBuf, usize) {
        let path = path.as_ref();
        let joined = self.join(path);
        let depth = joined.components().count();
        let depth = if path.is_absolute() {
            // If `path` is absolute, then it replaces `self` (`joined` and `path` are the
            // same). In this case, the depth of the join is the depth of
            // `joined` (there is no root sub-path).
            depth
                .checked_add(1)
                .expect("overflow determining join depth")
        } else if path.has_root() {
            depth
        } else {
            depth.saturating_sub(self.components().count())
        };
        (joined, depth)
    }
}

/// Describes errors that occur when walking a directory tree.
///
/// `WalkError` implements conversion into [`io::Error`].
///
/// [`io::Error`]: std::io::Error
#[derive(Debug, Error)]
#[error("failed to match directory tree: {kind}")]
pub struct WalkError {
    depth: usize,
    kind: WalkErrorKind,
}

impl WalkError {
    /// Gets the path at which the error occurred, if any.
    ///
    /// Returns `None` if there is no path associated with the error.
    pub fn path(&self) -> Option<&Path> {
        self.kind.path()
    }

    /// Gets the depth at which the error occurred from the root directory of
    /// the traversal.
    pub fn depth(&self) -> usize {
        self.depth
    }
}

impl From<walkdir::Error> for WalkError {
    fn from(error: walkdir::Error) -> Self {
        let depth = error.depth();
        let path = error.path().map(From::from);
        if error.io_error().is_some() {
            WalkError {
                depth,
                kind: WalkErrorKind::Io {
                    path,
                    error: error.into_io_error().expect("incongruent error kind"),
                },
            }
        } else {
            WalkError {
                depth,
                kind: WalkErrorKind::LinkCycle {
                    root: error
                        .loop_ancestor()
                        .expect("incongruent error kind")
                        .into(),
                    leaf: path.expect("incongruent error kind"),
                },
            }
        }
    }
}

impl From<WalkError> for io::Error {
    fn from(error: WalkError) -> Self {
        let kind = match error.kind {
            WalkErrorKind::Io { ref error, .. } => error.kind(),
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, error)
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
enum WalkErrorKind {
    #[error("failed to read file at `{path:?}`: {error}")]
    Io {
        path: Option<PathBuf>,
        error: io::Error,
    },
    #[error("symbolic link cycle detected from `{root}` to `{leaf}`")]
    LinkCycle { root: PathBuf, leaf: PathBuf },
}

impl WalkErrorKind {
    pub fn path(&self) -> Option<&Path> {
        match self {
            WalkErrorKind::Io { ref path, .. } => path.as_ref().map(PathBuf::as_ref),
            WalkErrorKind::LinkCycle { ref leaf, .. } => Some(leaf.as_ref()),
        }
    }
}

/// Functions for walking a directory tree at a [`Path`].
///
/// [`Path`]: std::path::Path
pub trait PathExt {
    /// Gets an iterator over files in the directory tree at the path.
    ///
    /// If the path refers to a regular file, then only its path is yielded by
    /// the iterator.
    ///
    /// This function uses the default [`WalkBehavior`]. To configure the
    /// behavior of the traversal, see [`PathExt::walk_with_behavior`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::path::Path;
    /// use wax::walk::{Entry, PathExt};
    ///
    /// for entry in Path::new(".").walk() {
    ///     let entry = entry.unwrap();
    ///     println!("{:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`PathExt::walk_with_behavior`]: crate::walk::PathExt::walk_with_behavior
    /// [`WalkBehavior`]: crate::walk::WalkBehavior
    fn walk(&self) -> WalkTree {
        self.walk_with_behavior(WalkBehavior::default())
    }

    /// Gets an iterator over files in the directory tree at the path.
    ///
    /// This function is the same as [`PathExt::walk`], but it additionally
    /// accepts a [`WalkBehavior`] that configures how the traversal
    /// interacts with symbolic links, the maximum depth from the root, etc.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::path::Path;
    /// use wax::walk::{Entry, LinkBehavior, PathExt};
    ///
    /// // Read the target of symbolic links (follow links).
    /// for entry in Path::new("/home").walk_with_behavior(LinkBehavior::ReadTarget) {
    ///     let entry = entry.unwrap();
    ///     println!("{:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`PathExt::walk`]: crate::walk::PathExt::walk
    /// [`WalkBehavior`]: crate::walk::WalkBehavior
    fn walk_with_behavior(&self, behavior: impl Into<WalkBehavior>) -> WalkTree;
}

impl PathExt for Path {
    fn walk_with_behavior(&self, behavior: impl Into<WalkBehavior>) -> WalkTree {
        WalkTree::with_behavior(self, behavior)
    }
}

/// Configuration for interpreting symbolic links.
///
/// Determines how symbolic links are interpreted when walking directory trees
/// using functions like [`Glob::walk_with_behavior`]. **By default, symbolic
/// links are read as regular files and their targets are ignored.**
///
/// [`Glob::walk_with_behavior`]: crate::Glob::walk_with_behavior
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LinkBehavior {
    /// Read the symbolic link file itself.
    ///
    /// This behavior reads the symbolic link as a regular file. The
    /// corresponding entry uses the path of the link file and its metadata
    /// describes the link file itself. The target is effectively ignored
    /// and traversal does **not** follow the link.
    #[default]
    ReadFile,
    /// Read the target of the symbolic link.
    ///
    /// This behavior reads the target of the symbolic link. The corresponding
    /// entry uses the path of the link file and its metadata describes the
    /// target. If the target is a directory, then traversal follows the
    /// link and descend into the target.
    ///
    /// If a link is reentrant and forms a cycle, then an error will be emitted
    /// instead of an entry and traversal does not follow the link.
    ReadTarget,
}

/// Configuration for walking directory trees.
///
/// Determines the behavior of the traversal within a directory tree when using
/// functions like [`Glob::walk_with_behavior`]. `WalkBehavior` can be
/// constructed via conversions from types representing its fields. APIs
/// generally accept `impl Into<WalkBehavior>`, so these conversion can be used
/// implicitly. When constructed using such a conversion, `WalkBehavior` will
/// use defaults for any remaining fields.
///
/// # Examples
///
/// By default, symbolic links are interpreted as regular files and targets are
/// ignored. To read linked targets, use [`LinkBehavior::ReadTarget`].
///
/// ```rust,no_run
/// use wax::walk::LinkBehavior;
/// use wax::Glob;
///
/// for entry in Glob::new("**")
///     .unwrap()
///     .walk_with_behavior(".", LinkBehavior::ReadTarget)
/// {
///     let entry = entry.unwrap();
///     // ...
/// }
/// ```
///
/// [`Glob::walk_with_behavior`]: crate::Glob::walk_with_behavior
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WalkBehavior {
    // TODO: Consider using a dedicated type for this field. Using primitive types does not
    //       interact well with conversions used in `walk` APIs. For example, if another `usize`
    //       field is introduced, then the conversions become ambiguous and confusing.
    /// Maximum depth.
    ///
    /// Determines the maximum depth to which a directory tree will be traversed
    /// relative to the root. A depth of zero corresponds to the root and so
    /// using such a depth will yield at most one file entry that refers to
    /// the root.
    ///
    /// For [`PathExt::walk`], this depth is relative to the [`Path`] receiver.
    /// For [`Glob::walk`], this depth is relative to the `directory` path
    /// parameter.
    ///
    /// The default value is [`usize::MAX`].
    ///
    /// [`Glob::walk`]: crate::Glob::walk
    /// [`Path`]: std::path::Path
    /// [`PathExt::walk`]: crate::walk::PathExt::walk
    /// [`usize::MAX`]: usize::MAX
    pub depth: usize,
    /// Interpretation of symbolic links.
    ///
    /// Determines how symbolic links are interpreted when walking a directory
    /// tree. See [`LinkBehavior`].
    ///
    /// The default value is [`LinkBehavior::ReadFile`].
    ///
    /// [`LinkBehavior`]: crate::walk::LinkBehavior
    /// [`LinkBehavior::ReadFile`]: crate::walk::LinkBehavior::ReadFile
    pub link: LinkBehavior,
}

/// Constructs a `WalkBehavior` using the following defaults:
///
/// | Field     | Description                       | Value                      |
/// |-----------|-----------------------------------|----------------------------|
/// | [`depth`] | Maximum depth.                    | [`usize::MAX`]             |
/// | [`link`]  | Interpretation of symbolic links. | [`LinkBehavior::ReadFile`] |
///
/// [`depth`]: crate::walk::WalkBehavior::depth
/// [`link`]: crate::walk::WalkBehavior::link
/// [`LinkBehavior::ReadFile`]: crate::walk::LinkBehavior::ReadFile
/// [`usize::MAX`]: usize::MAX
impl Default for WalkBehavior {
    fn default() -> Self {
        WalkBehavior {
            depth: usize::MAX,
            link: LinkBehavior::default(),
        }
    }
}

impl From<()> for WalkBehavior {
    fn from(_: ()) -> Self {
        Default::default()
    }
}

impl From<LinkBehavior> for WalkBehavior {
    fn from(link: LinkBehavior) -> Self {
        WalkBehavior {
            link,
            ..Default::default()
        }
    }
}

impl From<usize> for WalkBehavior {
    fn from(depth: usize) -> Self {
        WalkBehavior {
            depth,
            ..Default::default()
        }
    }
}

/// Describes a file yielded from a [`FileIterator`].
///
/// [`FileIterator`]: crate::walk::FileIterator
pub trait Entry {
    /// Converts the entry into its file path.
    fn into_path(self) -> PathBuf
    where
        Self: Sized;

    /// Gets the path of the file.
    fn path(&self) -> &Path;

    /// Gets the root and relative paths.
    ///
    /// The root path is the path to the walked directory from which the file
    /// entry has been read. The relative path is the remainder of the file
    /// path of the entry (the path relative to the root directory). Both the
    /// root and relative paths may be empty.
    ///
    /// The root and relative paths can differ significantly depending on the
    /// way a directory is walked, in particular when using a [`Glob`]. The
    /// following table describes some example paths when using
    /// [`Glob::walk`].
    ///
    /// | Glob Expression           | Directory    | Entry Path                               | Root         | Relative                         |
    /// |---------------------------|--------------|------------------------------------|--------------|----------------------------------|
    /// | `**/*.txt`                | `/home/user` | `/home/user/notes.txt`             | `/home/user` | `notes.txt`                      |
    /// | `projects/**/src/**/*.rs` | `.`          | `./projects/fibonacci/src/main.rs` | `.`          | `projects/fibonacci/src/main.rs` |
    /// | `/var/log/**/*.log`       | `.`          | `/var/log/pacman.log`              |              | `/var/log/pacman.log`            |
    ///
    /// See also [`GlobWalker::root_prefix_paths`]
    ///
    /// [`Glob`]: crate::Glob
    /// [`Glob::walk`]: crate::Glob::walk
    /// [`GlobWalker::root_prefix_paths`]: crate::walk::GlobWalker::root_prefix_paths
    fn root_relative_paths(&self) -> (&Path, &Path);

    /// Gets the [`Metadata`] of the file.
    ///
    /// On some platforms, this requires an additional read from the file
    /// system.
    ///
    /// [`Metadata`]: std::fs::Metadata
    fn metadata(&self) -> Result<Metadata, WalkError>;

    /// Gets the type of the file (regular vs. directory).
    ///
    /// Prefer this function over [`metadata`] if only the file type is needed,
    /// as this information is cached.
    ///
    /// [`metadata`]: crate::walk::Entry::metadata
    fn file_type(&self) -> FileType;

    /// Gets the depth of the file path from the root.
    ///
    /// The root path is the path to the walked directory from which the file
    /// entry has been read. Use [`root_relative_paths`] to get the root
    /// path.
    ///
    /// [`root_relative_paths`]: crate::walk::Entry::root_relative_paths
    fn depth(&self) -> usize;
}

/// Describes a file yielded from a [`WalkTree`] iterator.
///
/// [`WalkTree`]: crate::walk::WalkTree
#[derive(Clone, Debug)]
pub struct TreeEntry {
    entry: WaxDirEntry,
    prefix: usize,
}

/// A light wrapper around DirEntry that allows
/// us to reconstruct virtual ones if needed
#[derive(Clone, Debug)]
pub enum WaxDirEntry {
    DirEntry(DirEntry),
    /// Dead symlinks will yield errors from walkdir, but we may want them
    DeadSymlink {
        path: PathBuf,
        file_type: FileType,
        depth: usize,
        error: Rc<Error>,
    },
}

impl From<DirEntry> for WaxDirEntry {
    fn from(inner: DirEntry) -> Self {
        WaxDirEntry::DirEntry(inner)
    }
}

impl TryFrom<walkdir::Error> for WaxDirEntry {
    type Error = walkdir::Error;

    fn try_from(error: Error) -> Result<Self, walkdir::Error> {
        if error
            .io_error()
            .filter(|e| e.kind() == ErrorKind::NotFound)
            .is_some()
        {
            let path = error.path().expect("not found errors always have paths");

            if let Some(symlink_meta) = std::fs::symlink_metadata(path)
                .ok()
                .filter(Metadata::is_symlink)
            {
                return Ok(WaxDirEntry::DeadSymlink {
                    path: path.to_path_buf(),
                    file_type: symlink_meta.file_type(),
                    depth: error.depth(),
                    error: Rc::new(error),
                });
            }
        }

        Err(error)
    }
}

impl WaxDirEntry {
    pub fn path(&self) -> &Path {
        match self {
            WaxDirEntry::DirEntry(inner) => inner.path(),
            WaxDirEntry::DeadSymlink { path, .. } => path.as_path(),
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self {
            WaxDirEntry::DirEntry(inner) => inner.into_path(),
            WaxDirEntry::DeadSymlink { path, .. } => path,
        }
    }

    pub fn file_type(&self) -> FileType {
        match self {
            WaxDirEntry::DirEntry(inner) => inner.file_type(),
            WaxDirEntry::DeadSymlink { file_type, .. } => *file_type,
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            WaxDirEntry::DirEntry(inner) => inner.depth(),
            WaxDirEntry::DeadSymlink { depth, .. } => *depth,
        }
    }

    pub fn metadata(&self) -> Result<Metadata, WalkError> {
        match self {
            WaxDirEntry::DirEntry(inner) => inner.metadata().map_err(From::from),
            WaxDirEntry::DeadSymlink { path, error, .. } => {
                fs::symlink_metadata(path).map_err(|e| WalkError {
                    depth: error.depth(),
                    kind: WalkErrorKind::Io {
                        path: Some(path.to_path_buf()),
                        error: e,
                    },
                })
            }
        }
    }
}

impl Entry for TreeEntry {
    fn into_path(self) -> PathBuf {
        self.entry.into_path()
    }

    fn path(&self) -> &Path {
        self.entry.path()
    }

    fn root_relative_paths(&self) -> (&Path, &Path) {
        self.path().split_at_depth(
            self.depth()
                .checked_add(self.prefix)
                .expect("overflow determining root-relative paths"),
        )
    }

    fn metadata(&self) -> Result<Metadata, WalkError> {
        self.entry.metadata().map_err(From::from)
    }

    fn file_type(&self) -> FileType {
        self.entry.file_type()
    }

    fn depth(&self) -> usize {
        self.entry.depth()
    }
}

/// A [`FileIterator`] over files in a directory tree.
///
/// This iterator is constructed from [`Path`]s via extension functions in
/// [`PathExt`], such as [`PathExt::walk`].
///
/// # Examples
///
/// ```rust,no_run
/// use std::path::Path;
/// use wax::walk::{Entry, PathExt};
///
/// for entry in Path::new(".").walk() {
///     let entry = entry.unwrap();
///     println!("{:?}", entry.path());
/// }
/// ```
///
/// [`FileIterator`]: crate::walk::FileIterator
/// [`Path`]: std::path::Path
/// [`PathExt`]: crate::walk::PathExt
/// [`PathExt::walk`]: crate::walk::PathExt::walk
#[derive(Debug)]
pub struct WalkTree {
    prefix: usize,
    is_dir: bool,
    input: walkdir::IntoIter,
}

impl WalkTree {
    fn with_behavior(root: impl Into<PathBuf>, behavior: impl Into<WalkBehavior>) -> Self {
        WalkTree::with_prefix_and_behavior(root, 0, behavior)
    }

    fn with_prefix_and_behavior(
        root: impl Into<PathBuf>,
        prefix: usize,
        behavior: impl Into<WalkBehavior>,
    ) -> Self {
        let root = root.into();
        let WalkBehavior { link, depth } = behavior.into();
        let builder = WalkDir::new(root.as_path());
        WalkTree {
            prefix,
            is_dir: false,
            input: builder
                .follow_links(match link {
                    LinkBehavior::ReadFile => false,
                    LinkBehavior::ReadTarget => true,
                })
                .max_depth(depth)
                .into_iter(),
        }
    }
}

impl CancelWalk for WalkTree {
    fn cancel_walk_tree(&mut self) {
        // `IntoIter::skip_current_dir` discards the least recently yielded directory,
        // but `cancel_walk_tree` must act upon the most recently yielded node
        // regardless of its topology (leaf vs. branch).
        if self.is_dir {
            self.input.skip_current_dir();
        }
    }
}

impl Iterator for WalkTree {
    type Item = Result<TreeEntry, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        let (is_dir, next) = match self.input.next() {
            Some(result) => match result {
                Ok(entry) => (
                    entry.file_type().is_dir(),
                    Some(Ok(TreeEntry {
                        entry: entry.into(),
                        prefix: self.prefix,
                    })),
                ),
                Err(error) => match WaxDirEntry::try_from(error) {
                    Ok(entry) => (
                        false,
                        Some(Ok(TreeEntry {
                            entry,
                            prefix: self.prefix,
                        })),
                    ),
                    Err(error) => (false, Some(Err(error.into()))),
                },
            },
            _ => (false, None),
        };
        self.is_dir = is_dir;
        next
    }
}

impl SeparatingFilterInput for WalkTree {
    type Feed = (Result<TreeEntry, WalkError>, TreeResidue<TreeEntry>);
}

/// An [`Iterator`] over files in a directory tree.
///
/// This iterator is aware of its hierarchical structure and can cancel
/// traversal into directories that are discarded by filter combinators to avoid
/// unnecessary work. The contents of discarded directories are not read from
/// the file system.
///
/// The iterators constructed by [`PathExt::walk`], [`Glob::walk`], etc.
/// implement this trait.
///
/// [`Glob::walk`]: crate::Glob::walk
/// [`PathExt::walk`]: crate::walk::PathExt::walk
/// [`Iterator`]: std::iter::Iterator
pub trait FileIterator:
    HierarchicalIterator<Feed = FileFeed<Self::Entry, Self::Residue>>
    + Iterator<Item = FileFiltrate<Self::Entry>>
{
    /// The file entry type yielded by the iterator.
    ///
    /// `FileIterator`s implement [`Iterator`] where the associated `Item` type
    /// is `Result<Self::Entry, WalkError>`.
    ///
    /// [`Result`]: std::result::Result
    type Entry: Entry;
    type Residue: Entry + From<Self::Entry>;

    /// Filters file entries and controls the traversal of the directory tree.
    ///
    /// This function constructs a combinator that filters file entries and
    /// furthermore specifies how iteration proceeds to traverse the
    /// directory tree. It accepts a function that, when discarding an
    /// entry, returns an [`EntryResidue`]. If an entry refers to a directory
    /// and the filtering function returns [`EntryResidue::Tree`], then
    /// iteration does **not** descend into that directory and the tree is
    /// **not** read from the file system.
    ///
    /// The filtering function is called even when a composing filter has
    /// already discarded a file entry. This allows filtering combinators to
    /// observe previously filtered entries and potentially discard a
    /// directory tree regardless of how they are composed. Filtering is
    /// monotonic, meaning that filtered entries can only progress forward from
    /// unfiltered `None` to filtered file `Some(EntryResidue::File)` to
    /// filtered tree `Some(EntryResidue::Tree)`. An entry cannot be
    /// "unfiltered" and if a subsequent combinator specifies a lesser filter,
    /// then it has no effect.
    ///
    /// **Prefer this combinator over functions like [`Iterator::filter`] when
    /// discarded directories need not be read.**
    ///
    /// # Examples
    ///
    /// The [`FilterEntry`] combinator can apply arbitrary and non-nominal
    /// filtering that avoids unnecessary directory reads. The following
    /// example filters out hidden files on Unix and Windows. On Unix,
    /// hidden files are filtered out nominally via [`not`]. On Windows,
    /// `filter_entry` instead detects the [hidden attribute][attributes]. In
    /// both cases, the combinator does not read conventionally hidden
    /// directory trees.
    ///
    /// ```rust,no_run
    /// use wax::walk::{Entry, FileIterator};
    /// use wax::Glob;
    ///
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg}").unwrap();
    /// let walk = glob.walk("./Pictures");
    /// // Filter out nominally hidden files on Unix. Like `filter_entry`, `not` does not perform
    /// // unnecessary reads of directory trees.
    /// #[cfg(unix)]
    /// let walk = walk.not(["**/.*/**"]).unwrap();
    /// // Filter out files with the hidden attribute on Windows.
    /// #[cfg(windows)]
    /// let walk = walk.filter_entry(|entry| {
    ///     use std::os::windows::fs::MetadataExt as _;
    ///     use wax::walk::EntryResidue;
    ///
    ///     const ATTRIBUTE_HIDDEN: u32 = 0x2;
    ///
    ///     let attributes = entry.metadata().unwrap().file_attributes();
    ///     if (attributes & ATTRIBUTE_HIDDEN) == ATTRIBUTE_HIDDEN {
    ///         // Do not read hidden directory trees.
    ///         Some(EntryResidue::Tree)
    ///     }
    ///     else {
    ///         None
    ///     }
    /// });
    /// for entry in walk {
    ///     let entry = entry.unwrap();
    ///     println!("JPEG: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`EntryResidue`]: crate::walk::EntryResidue
    /// [`EntryResidue::Tree`]: crate::walk::EntryResidue::Tree
    /// [`FilterEntry`]: crate::walk::FilterEntry
    /// [`Iterator::filter`]: std::iter::Iterator::filter
    /// [`not`]: crate::walk::FileIterator::not
    ///
    /// [attributes]: https://docs.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants
    fn filter_entry<F>(self, f: F) -> FilterEntry<Self, F>
    where
        Self: Sized,
        F: FnMut(&dyn Entry) -> Option<EntryResidue>,
    {
        FilterEntry { input: self, f }
    }

    /// Filters file entries against negated glob expressions.
    ///
    /// This function constructs a combinator that discards files with paths
    /// that match **any** of the given glob expressions. When matching a
    /// [`Glob`] against a directory tree, this allows for broad negations
    /// that cannot be achieved using a positive glob expression alone.
    ///
    /// The combinator does **not** read directory trees from the file system
    /// when a directory matches an [exhaustive glob
    /// expression][`Program::is_exhaustive`] such as `**/private/**`
    /// or `hidden/<<?>/>*`.
    ///
    /// **Prefer this combinator over matching each file entry against
    /// [`Program`]s, since it avoids potentially large and unnecessary
    /// reads.**
    ///
    /// # Errors
    ///
    /// Returns an error if any of the inputs fail to build. If the inputs are a
    /// compiled [`Program`] type such as [`Glob`], then this only occurs if
    /// the compiled program is too large (i.e., there are too many
    /// component patterns).
    ///
    /// # Examples
    ///
    /// Because glob expressions do not support general negations, it is
    /// sometimes impossible to express patterns that deny particular paths.
    /// In such cases, `not` can be used to apply additional patterns as a
    /// filter.
    ///
    /// ```rust,no_run
    /// use wax::walk::FileIterator;
    /// use wax::Glob;
    ///
    /// // Find image files, but not if they are beneath a directory with a name that suggests that
    /// // they are private.
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg,png}").unwrap();
    /// for entry in glob.walk(".").not(["**/(?i)<.:0,1>private/**"]).unwrap() {
    ///     let entry = entry.unwrap();
    ///     // ...
    /// }
    /// ```
    ///
    /// [`Glob`]: crate::Glob
    /// [`Iterator::filter`]: std::iter::Iterator::filter
    /// [`Program`]: crate::Program
    /// [`Program::is_exhaustive`]: crate::Program::is_exhaustive
    fn not<'t, I>(self, patterns: I) -> Result<Not<Self>, BuildError>
    where
        Self: Sized,
        I: IntoIterator,
        I::Item: Pattern<'t>,
    {
        FilterAny::any(patterns).map(|filter| Not {
            input: self,
            filter,
        })
    }
}

impl<T, R, I> FileIterator for I
where
    T: Entry,
    R: Entry + From<T>,
    I: HierarchicalIterator<Feed = FileFeed<T, R>> + Iterator<Item = FileFiltrate<T>>,
{
    type Entry = T;
    type Residue = R;
}

// TODO: Implement this using combinators provided by the `filter` module and
// RPITIT once it lands       in stable Rust. Remove any use of
// `WalkCancellation::unchecked`.
/// Iterator combinator that filters file entries and controls the traversal of
/// directory trees.
///
/// This combinator is returned by [`FileIterator::filter_entry`] and implements
/// [`FileIterator`].
///
/// [`FileIterator`]: crate::walk::FileIterator
/// [`FileIterator::filter_entry`]: crate::walk::FileIterator::filter_entry
#[derive(Clone, Debug)]
pub struct FilterEntry<I, F> {
    input: I,
    f: F,
}

impl<I, F> CancelWalk for FilterEntry<I, F>
where
    I: CancelWalk,
{
    fn cancel_walk_tree(&mut self) {
        self.input.cancel_walk_tree()
    }
}

impl<T, R, I, F> SeparatingFilter for FilterEntry<I, F>
where
    T: 'static + Entry,
    R: 'static + Entry + From<T>,
    I: FileIterator<Entry = T, Residue = R>,
    F: FnMut(&dyn Entry) -> Option<EntryResidue>,
{
    type Feed = I::Feed;

    fn feed(&mut self) -> Option<Separation<Self::Feed>> {
        self.input
            .feed()
            .map(|separation| match separation.transpose_filtrate() {
                Ok(separation) => separation
                    .filter_tree_by_substituent(
                        WalkCancellation::unchecked(&mut self.input),
                        |substituent| (self.f)(substituent).map(From::from),
                    )
                    .map_filtrate(Ok),
                Err(error) => error.map(Err).into(),
            })
    }
}

impl<T, R, I, F> Iterator for FilterEntry<I, F>
where
    T: 'static + Entry,
    R: 'static + Entry + From<T>,
    I: FileIterator<Entry = T, Residue = R>,
    F: FnMut(&dyn Entry) -> Option<EntryResidue>,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        filter::filtrate(self)
    }
}

// TODO: Implement this using combinators provided by the `filter` module and
// RPITIT once it lands       in stable Rust. Remove any use of
// `WalkCancellation::unchecked`.
/// Iterator combinator that filters file entries with paths that match
/// patterns.
///
/// This combinator is returned by [`FileIterator::not`] and implements
/// [`FileIterator`].
///
/// [`FileIterator`]: crate::walk::FileIterator
/// [`FileIterator::not`]: crate::walk::FileIterator::not
#[derive(Clone, Debug)]
pub struct Not<I> {
    input: I,
    filter: FilterAny,
}

impl<I> CancelWalk for Not<I>
where
    I: CancelWalk,
{
    fn cancel_walk_tree(&mut self) {
        self.input.cancel_walk_tree()
    }
}

impl<T, R, I> SeparatingFilter for Not<I>
where
    T: 'static + Entry,
    R: 'static + Entry + From<T>,
    I: FileIterator<Entry = T, Residue = R>,
{
    type Feed = I::Feed;

    fn feed(&mut self) -> Option<Separation<Self::Feed>> {
        self.input
            .feed()
            .map(|separation| match separation.transpose_filtrate() {
                Ok(separation) => separation
                    .filter_tree_by_substituent(
                        WalkCancellation::unchecked(&mut self.input),
                        |substituent| self.filter.residue(substituent).map(From::from),
                    )
                    .map_filtrate(Ok),
                Err(error) => error.map(Err).into(),
            })
    }
}

impl<T, R, I> Iterator for Not<I>
where
    T: 'static + Entry,
    R: 'static + Entry + From<T>,
    I: FileIterator<Entry = T, Residue = R>,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        filter::filtrate(self)
    }
}

/// Describes how file entries are read and discarded by
/// [`FileIterator::filter_entry`].
///
/// [`FileIterator::filter_entry`]: crate::walk::FileIterator::filter_entry
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EntryResidue {
    /// Discard the file.
    ///
    /// The entry for the given file is discarded. Only this particular file is
    /// ignored and if the entry refers to a directory, then its tree is
    /// still read from the file system.
    File,
    /// Discard the file **and its directory tree**, if any.
    ///
    /// The entry for the given file is discarded. If the entry refers to a
    /// directory, then its entire tree is ignored and is **not** read from
    /// the file system.
    ///
    /// If the entry refers to a normal file (not a directory), then this is the
    /// same as [`EntryResidue::File`].
    ///
    /// [`EntryResidue::File`]: crate::walk::EntryResidue::File
    Tree,
}

impl From<EntryResidue> for TreeResidue<()> {
    fn from(residue: EntryResidue) -> Self {
        match residue {
            EntryResidue::File => TreeResidue::Node(()),
            EntryResidue::Tree => TreeResidue::Tree(()),
        }
    }
}

// TODO: Rust's testing framework does not provide a mechanism for maintaining
// shared state. This       means that tests that write to the file system must
// do so individually rather than writing       before and after all tests have
// run. This should probably be avoided.
#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use build_fs_tree::{dir, file, Build, FileSystemTree};
    use path_slash::PathBufExt;
    use tempfile::TempDir;

    use crate::{
        walk::{
            filter::{HierarchicalIterator, Separation, TreeResidue},
            Entry, FileIterator, LinkBehavior, PathExt, WalkBehavior,
        },
        Glob,
    };

    macro_rules! assert_set_eq {
        ($left:expr, $right:expr $(,)?) => {{
            match (&$left, &$right) {
                (left, right) if !(*left == *right) => {
                    let lrdiff: Vec<_> = left.difference(right).collect();
                    let rldiff: Vec<_> = right.difference(left).collect();
                    panic!(
                        "assertion `left == right` failed\nleft: {:#?}\nright: {:#?}\nleft - \
                         right: {:#?}\nright - left: {:#?}",
                        left, right, lrdiff, rldiff,
                    )
                }
                _ => {}
            }
        }};
    }

    /// Writes a testing directory tree to a temporary location on the file
    /// system.
    fn temptree() -> (TempDir, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let tree: FileSystemTree<&str, &str> = dir! {
            "doc" => dir! {
                "guide.md" => file!(""),
            },
            "src" => dir! {
                "glob.rs" => file!(""),
                "lib.rs" => file!(""),
            },
            "tests" => dir! {
                "harness" => dir! {
                    "mod.rs" => file!(""),
                },
                "walk.rs" => file!(""),
            },
            "README.md" => file!(""),
        };
        let path = root.path().join("project");
        tree.build(&path).unwrap();
        (root, path)
    }

    /// Writes a testing directory tree that includes a reentrant symbolic link
    /// to a temporary location on the file system.
    #[cfg(any(unix, windows))]
    fn temptree_with_cyclic_link() -> (TempDir, PathBuf) {
        use std::{io, path::Path};

        #[cfg(unix)]
        fn link(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
            std::os::unix::fs::symlink(target, link)
        }

        #[cfg(windows)]
        fn link(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
            std::os::windows::fs::symlink_dir(target, link)
        }

        // Get a temporary tree and create a reentrant symbolic link.
        let (root, path) = temptree();
        link(path.as_path(), path.join("tests/cycle")).unwrap();
        (root, path)
    }

    #[test]
    fn walk_tree() {
        let (_root, path) = temptree();

        let paths: HashSet<_> = path
            .walk()
            .flatten()
            .map(|entry| entry.root_relative_paths().1.to_path_buf())
            .collect();
        assert_set_eq!(
            paths,
            [
                PathBuf::from(""),
                PathBuf::from("doc"),
                PathBuf::from("doc/guide.md"),
                PathBuf::from("src"),
                PathBuf::from("src/glob.rs"),
                PathBuf::from("src/lib.rs"),
                PathBuf::from("tests"),
                PathBuf::from("tests/harness"),
                PathBuf::from("tests/harness/mod.rs"),
                PathBuf::from("tests/walk.rs"),
                PathBuf::from("README.md"),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    fn walk_tree_with_not() {
        let (_root, path) = temptree();

        let paths: HashSet<_> = path
            .walk()
            .not(["tests/**"])
            .unwrap()
            .flatten()
            .map(Entry::into_path)
            .collect();
        assert_set_eq!(
            paths,
            [
                #[allow(clippy::redundant_clone)]
                path.to_path_buf(),
                path.join("doc"),
                path.join("doc/guide.md"),
                path.join("src"),
                path.join("src/glob.rs"),
                path.join("src/lib.rs"),
                path.join("README.md"),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    fn walk_tree_with_empty_not() {
        let (_root, path) = temptree();

        let paths: HashSet<_> = path
            .walk()
            .not([""])
            .unwrap()
            .flatten()
            .map(Entry::into_path)
            .collect();
        assert_set_eq!(
            paths,
            // The root directory (`path.join("")` or `path.to_path_buf()`) must not be present,
            // because the empty `not` pattern matches the empty relative path at the root.
            [
                path.join("doc"),
                path.join("doc/guide.md"),
                path.join("src"),
                path.join("src/glob.rs"),
                path.join("src/lib.rs"),
                path.join("tests"),
                path.join("tests/harness"),
                path.join("tests/harness/mod.rs"),
                path.join("tests/walk.rs"),
                path.join("README.md"),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    fn walk_glob_with_unbounded_tree() {
        let (_root, path) = temptree();

        let glob = Glob::new("**").unwrap();
        let paths: HashSet<_> = glob.walk(&path).flatten().map(Entry::into_path).collect();
        assert_set_eq!(
            paths,
            [
                #[allow(clippy::redundant_clone)]
                path.to_path_buf(),
                path.join("doc"),
                path.join("doc/guide.md"),
                path.join("src"),
                path.join("src/glob.rs"),
                path.join("src/lib.rs"),
                path.join("tests"),
                path.join("tests/harness"),
                path.join("tests/harness/mod.rs"),
                path.join("tests/walk.rs"),
                path.join("README.md"),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    fn walk_glob_with_invariant_terminating_component() {
        let (_root, path) = temptree();

        let glob = Glob::new("**/*.md").unwrap();
        let paths: HashSet<_> = glob.walk(&path).flatten().map(Entry::into_path).collect();
        assert_set_eq!(
            paths,
            [path.join("doc/guide.md"), path.join("README.md"),]
                .into_iter()
                .collect(),
        );
    }

    #[test]
    fn walk_glob_with_invariant_intermediate_component() {
        let (_root, path) = temptree();

        let glob = Glob::new("**/src/**/*.rs").unwrap();
        let paths: HashSet<_> = glob.walk(&path).flatten().map(Entry::into_path).collect();
        assert_set_eq!(
            paths,
            [path.join("src/glob.rs"), path.join("src/lib.rs"),]
                .into_iter()
                .collect(),
        );
    }

    #[test]
    fn walk_with_anchored_glob() {
        let (_root, path) = temptree();
        let slash_path = path.to_slash().unwrap();

        // on windows the slash path doesn't escape the colon. to make
        // it a valid glob, we must
        #[cfg(windows)]
        let slash_path = {
            let regex = regex::Regex::new("([A-Z]):").unwrap();
            regex.replace(&slash_path, "$1\\:")
        };

        let glob_exp = format!("{}/{}", slash_path, "**/*.rs");

        let glob = Glob::new(&glob_exp).unwrap();
        let paths: HashSet<_> = glob.walk(&path).flatten().map(Entry::into_path).collect();
        assert_set_eq!(
            paths,
            [
                "src/glob.rs",
                "src/lib.rs",
                "tests/harness/mod.rs",
                "tests/walk.rs"
            ]
            .into_iter()
            .map(|c| path.join(c))
            .collect(),
        );
    }

    #[test]
    fn walk_glob_with_only_invariant() {
        let (_root, path) = temptree();

        let glob = Glob::new("src/lib.rs").unwrap();
        let paths: HashSet<_> = glob.walk(&path).flatten().map(Entry::into_path).collect();
        assert_set_eq!(paths, [path.join("src/lib.rs")].into_iter().collect());
    }

    #[test]
    fn walk_glob_with_only_invariant_partitioned() {
        let (_root, path) = temptree();

        let (prefix, glob) = Glob::new("src/lib.rs").unwrap().partition();
        let paths: HashSet<_> = glob
            .walk(path.join(prefix))
            .flatten()
            .map(Entry::into_path)
            .collect();
        assert_set_eq!(paths, [path.join("src/lib.rs")].into_iter().collect());
    }

    #[test]
    fn walk_glob_with_not() {
        #[derive(Debug, Eq, Hash, PartialEq)]
        enum TestSeparation<T> {
            Filtrate(T),
            Residue(TreeResidue<T>),
        }
        use TestSeparation::{Filtrate, Residue};
        use TreeResidue::{Node, Tree};

        let (_root, path) = temptree();

        let glob = Glob::new("**/*.{md,rs}").unwrap();
        let mut paths = HashSet::new();
        glob.walk(&path)
            .not(["**/harness/**"])
            .unwrap()
            // Inspect the feed rather than the `Iterator` output (filtrate). While it is trivial
            // to provide a way to collect the feed, it is difficult to inspect its contents. In
            // particular, it is not possible to construct `Product`s outside of the `filter`
            // module (by design). Instead, the feed is collected into a simpler format in
            // `filter_map_tree`.
            .filter_map_tree(|_, separation| {
                paths.insert(match separation.as_ref() {
                    Separation::Filtrate(ref filtrate) => Filtrate(
                        filtrate
                            .get()
                            .as_ref()
                            .expect("failed to read file")
                            .path()
                            .to_path_buf(),
                    ),
                    Separation::Residue(ref residue) => Residue(
                        residue
                            .get()
                            .as_ref()
                            .map(|residue| residue.path().to_path_buf()),
                    ),
                });
                separation
            })
            .for_each(drop);
        assert_set_eq!(
            paths,
            [
                Residue(Node(path.to_path_buf())),
                Residue(Node(path.join("doc"))),
                Filtrate(path.join("doc/guide.md")),
                Residue(Node(path.join("src"))),
                Filtrate(path.join("src/glob.rs")),
                Filtrate(path.join("src/lib.rs")),
                Residue(Node(path.join("tests"))),
                // This entry is important. The glob does **not** match this path and will separate
                // it into node residue (not tree residue). The glob **does** match paths beneath
                // it. The `not` iterator must subsequently observe the residue and map it from
                // node to tree and cancel the walk. Nothing beneath this directory must be present
                // at all, even as residue.
                Residue(Tree(path.join("tests/harness"))),
                Filtrate(path.join("tests/walk.rs")),
                Filtrate(path.join("README.md")),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    fn walk_glob_with_depth() {
        let (_root, path) = temptree();

        let glob = Glob::new("**").unwrap();
        let paths: HashSet<_> = glob
            .walk_with_behavior(
                &path,
                WalkBehavior {
                    depth: 1,
                    ..Default::default()
                },
            )
            .flatten()
            .map(Entry::into_path)
            .collect();
        assert_set_eq!(
            paths,
            [
                #[allow(clippy::redundant_clone)]
                path.to_path_buf(),
                path.join("doc"),
                path.join("src"),
                path.join("tests"),
                path.join("README.md"),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn walk_glob_with_cyclic_link_file() {
        let (_root, path) = temptree_with_cyclic_link();

        let glob = Glob::new("**").unwrap();
        let paths: HashSet<_> = glob
            .walk_with_behavior(&path, LinkBehavior::ReadFile)
            .flatten()
            .map(Entry::into_path)
            .collect();
        assert_set_eq!(
            paths,
            [
                #[allow(clippy::redundant_clone)]
                path.to_path_buf(),
                path.join("README.md"),
                path.join("doc"),
                path.join("doc/guide.md"),
                path.join("src"),
                path.join("src/glob.rs"),
                path.join("src/lib.rs"),
                path.join("tests"),
                path.join("tests/cycle"),
                path.join("tests/harness"),
                path.join("tests/harness/mod.rs"),
                path.join("tests/walk.rs"),
            ]
            .into_iter()
            .collect(),
        );
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn walk_glob_with_cyclic_link_target() {
        let (_root, path) = temptree_with_cyclic_link();

        // Collect paths into `Vec`s so that duplicates can be detected.
        let expected = vec![
            #[allow(clippy::redundant_clone)]
            path.to_path_buf(),
            path.join("README.md"),
            path.join("doc"),
            path.join("doc/guide.md"),
            path.join("src"),
            path.join("src/glob.rs"),
            path.join("src/lib.rs"),
            path.join("tests"),
            path.join("tests/harness"),
            path.join("tests/harness/mod.rs"),
            path.join("tests/walk.rs"),
        ];
        let glob = Glob::new("**").unwrap();
        let mut paths: Vec<_> = glob
            .walk_with_behavior(&path, LinkBehavior::ReadTarget)
            .flatten()
            // Take an additional item. This prevents an infinite loop if there is a problem with
            // detecting the cycle while also introducing unexpected files so that the error can be
            // detected.
            .take(expected.len() + 1)
            .map(Entry::into_path)
            .collect();
        paths.sort_unstable();
        assert_eq!(paths, expected);
    }
}
