#![cfg(feature = "walk")]

use std::{
    borrow::Cow,
    fs::{FileType, Metadata},
    io,
    path::{Component, Path, PathBuf},
};

use itertools::Itertools as _;
use regex::Regex;
use thiserror::Error;
use walkdir::{self, DirEntry, WalkDir};

use crate::{
    capture::MatchedText,
    encode::CompileError,
    token::{self, Token, TokenTree},
    BuildError, CandidatePath, Compose, Glob, PositionExt as _,
};

pub type WalkItem<'e> = Result<WalkEntry<'e>, WalkError>;

/// Describes errors that occur when matching a [`Glob`] against a directory
/// tree.
///
/// `WalkError` implements conversion into [`io::Error`].
///
/// [`Glob`]: crate::Glob
/// [`io::Error`]: std::io::Error
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Debug, Error)]
#[error("failed to match directory tree: {kind}")]
pub struct WalkError {
    depth: usize,
    kind: WalkErrorKind,
}

impl WalkError {
    /// Gets the path at which the error occurred.
    ///
    /// Returns `None` if there is no path associated with the error.
    pub fn path(&self) -> Option<&Path> {
        self.kind.path()
    }

    /// Gets the depth from [the root][`Walk::root`] at which the error
    /// occurred.
    ///
    /// [`Walk::root`]: crate::Walk::root
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

/// Traverses a directory tree via a `Walk` instance.
///
/// This macro emits an interruptable loop that executes a block of code
/// whenever a `WalkEntry` or error is encountered while traversing a directory
/// tree. The block may return from its function or otherwise interrupt and
/// subsequently resume the loop.
///
/// Note that if the block attempts to emit a `WalkEntry` across a function
/// boundary, then the entry contents must be copied via `into_owned`.
macro_rules! walk {
    ($state:expr => |$entry:ident| $f:block) => {
        use itertools::EitherOrBoth::{Both, Left, Right};
        use itertools::Position::{First, Last, Middle, Only};

        // `while-let` avoids a mutable borrow of `walk`, which would prevent a
        // subsequent call to `skip_current_dir` within the loop body.
        #[allow(clippy::while_let_on_iterator)]
        #[allow(unreachable_code)]
        'walk: while let Some(entry) = $state.walk.next() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let $entry = Err(error.into());
                    $f
                    continue; // May be unreachable.
                }
            };
            let path = entry
                .path()
                .strip_prefix(&$state.prefix)
                .expect("path is not in tree");
            let depth = entry.depth().saturating_sub(1);
            // Globs don't include the root token, but absolute paths do.
            // Skip that token so that matching up components will work below.
            for candidate in path
                .components()
                .filter(|c| !matches!(c, Component::RootDir))
                .skip(depth)
                .filter_map(|component| match component {
                    Component::Normal(component) => Some(CandidatePath::from(component)),
                    Component::Prefix(component) => Some(CandidatePath::from(component.as_os_str())),
                    _ => None,
                })
                .zip_longest($state.components.iter().skip(depth))
                .with_position()
            {
                match candidate.as_tuple() {
                    (First(_) | Middle(_), Both(component, pattern)) => {
                        if !pattern.is_match(component.as_ref()) {
                            // Do not descend into directories that do not match
                            // the corresponding component pattern.
                            if entry.file_type().is_dir() {
                                $state.walk.skip_current_dir();
                            }
                            continue 'walk;
                        }
                    }
                    (Last(_) | Only(_), Both(component, pattern)) => {
                        if pattern.is_match(component.as_ref()) {
                            let path = CandidatePath::from(path);
                            if let Some(matched) =
                                $state.pattern.captures(path.as_ref()).map(MatchedText::from)
                            {
                                let $entry = Ok(WalkEntry {
                                    entry: Cow::Borrowed(&entry),
                                    matched,
                                });
                                $f
                            }
                        }
                        else {
                            // Do not descend into directories that do not match
                            // the corresponding component pattern.
                            if entry.file_type().is_dir() {
                                $state.walk.skip_current_dir();
                            }
                        }
                        continue 'walk;
                    }
                    (_, Left(_component)) => {
                        let path = CandidatePath::from(path);
                        if let Some(matched) =
                            $state.pattern.captures(path.as_ref()).map(MatchedText::from)
                        {
                            let $entry = Ok(WalkEntry {
                                entry: Cow::Borrowed(&entry),
                                matched,
                            });
                            $f
                        }
                        continue 'walk;
                    }
                    (_, Right(_pattern)) => {
                        continue 'walk;
                    }
                }
            }
            // If the loop is not entered, check for a match. This may indicate
            // that the `Glob` is empty and a single invariant path may be
            // matched.
            let path = CandidatePath::from(path);
            if let Some(matched) = $state.pattern.captures(path.as_ref()).map(MatchedText::from) {
                let $entry = Ok(WalkEntry {
                    entry: Cow::Borrowed(&entry),
                    matched,
                });
                $f
            }
        }
    };
}

/// An [`Iterator`] over [`WalkEntry`]s that can filter directory trees.
///
/// A `FileIterator` is a `TreeIterator` that yields [`WalkEntry`]s. This trait
/// is implemented by [`Walk`] and adaptors like [`FilterTree`]. A
/// `TreeIterator` is an iterator that reads its items from a tree and therefore
/// can meaningfully filter not only items but their corresponding sub-trees to
/// avoid unnecessary work. To that end, this trait provides the `filter_tree`
/// function, which allows directory trees to be discarded (not read from the
/// file system) when matching [`Glob`]s against directory trees.
///
/// [`filter_tree`]: crate::FileIterator::filter_tree
/// [`Glob`]: crate::Glob
/// [`Iterator`]: std::iter::Iterator
/// [`WalkEntry`]: crate::WalkEntry
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
pub trait FileIterator: Sized + TreeIterator<Item = WalkItem<'static>> {
    /// Filters [`WalkEntry`]s and controls the traversal of directory trees.
    ///
    /// This function creates an adaptor that filters [`WalkEntry`]s and
    /// furthermore specifies how iteration proceeds to traverse directory
    /// trees. The adaptor accepts a function that, when discarding a
    /// [`WalkEntry`], yields a [`FilterTarget`]. **If the entry refers to a
    /// directory and [`FilterTarget::Tree`] is returned by the function, then
    /// iteration will not descend into that directory and the tree will not be
    /// read from the file system.** Therefore, this adaptor should be preferred
    /// over functions like [`Iterator::filter`] when discarded directories do
    /// not need to be read.
    ///
    /// Errors are not filtered, so if an error occurs reading a file at a path
    /// that would have been discarded, then that error is still yielded by the
    /// iterator.
    ///
    /// # Examples
    ///
    /// The [`FilterTree`] adaptor can be used to apply additional custom
    /// filtering that avoids unnecessary directory reads. The following example
    /// filters out hidden files on Unix and Windows. On Unix, hidden files are
    /// filtered out nominally via [`not`]. On Windows, `filter_tree` is used to
    /// detect the [hidden attribute][attributes]. In both cases, the adaptor
    /// does not read conventionally hidden directory trees.
    ///
    /// ```rust,no_run
    /// use wax::Glob;
    /// #[cfg(windows)]
    /// use wax::{FileIterator, FilterTarget};
    ///
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg}").unwrap();
    /// let walk = glob.walk("./Pictures");
    /// // Filter out nominally hidden files on Unix. Like `filter_tree`, `not`
    /// // does not perform unnecessary reads of directory trees.
    /// #[cfg(unix)]
    /// let walk = walk.not(["**/.*/**"]).unwrap();
    /// // Filter out files with the hidden attribute on Windows.
    /// #[cfg(windows)]
    /// let walk = walk.filter_tree(|entry| {
    ///     use std::os::windows::fs::MetadataExt as _;
    ///
    ///     const ATTRIBUTE_HIDDEN: u32 = 0x2;
    ///
    ///     let attributes = entry.metadata().unwrap().file_attributes();
    ///     if (attributes & ATTRIBUTE_HIDDEN) == ATTRIBUTE_HIDDEN {
    ///         // Do not read hidden directory trees.
    ///         Some(FilterTarget::Tree)
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
    /// [`FilterTree`]: crate::FilterTree
    /// [`Iterator`]: std::iter::Iterator
    /// [`Iterator::filter`]: std::iter::Iterator::filter
    /// [`not`]: crate::Walk::not
    /// [`Walk`]: crate::Walk
    /// [`WalkEntry`]: crate::WalkEntry
    ///
    /// [attributes]: https://docs.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants
    fn filter_tree<F>(self, f: F) -> FilterTree<Self, F>
    where
        F: FnMut(&WalkEntry<'static>) -> Option<FilterTarget>;
}

impl<I> FileIterator for I
where
    I: TreeIterator<Item = WalkItem<'static>> + Sized,
{
    fn filter_tree<F>(self, f: F) -> FilterTree<Self, F>
    where
        F: FnMut(&WalkEntry<'static>) -> Option<FilterTarget>,
    {
        FilterTree { input: self, f }
    }
}

pub trait TreeIterator: Iterator {
    fn skip_tree(&mut self);
}

impl TreeIterator for walkdir::IntoIter {
    fn skip_tree(&mut self) {
        self.skip_current_dir();
    }
}

/// Negated combinator that efficiently filters [`WalkEntry`]s.
///
/// Determines an appropriate [`FilterTarget`] for a [`WalkEntry`] based on the
/// [exhaustiveness][`Pattern::is_exhaustive`] of its component [`Pattern`]s.
/// This can be used with [`FilterTree`] to efficiently filter [`WalkEntry`]s
/// without reading directory trees from the file system when not necessary.
///
/// [`FilterTarget`]: crate::FilterTarget
/// [`FilterTree`]: crate::FilterTree
/// [`Pattern`]: crate::Pattern
/// [`Pattern::is_exhaustive`]: crate::Pattern::is_exhaustive
/// [`WalkEntry`]: crate::WalkEntry
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Clone, Debug)]
pub struct Negation {
    exhaustive: Regex,
    nonexhaustive: Regex,
}

impl Negation {
    /// Composes glob expressions into a `Negation`.
    ///
    /// This function accepts an [`IntoIterator`] with items that implement the
    /// [`Compose`] trait such as [`Glob`] and `&str`.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the inputs fail to build. If the inputs are a
    /// compiled [`Pattern`] type such as [`Glob`], then this only occurs if the
    /// compiled program is too large.
    ///
    /// [`Glob`]: crate::Glob
    /// [`Pattern`]: crate::Pattern
    /// [`IntoIterator`]: std::iter::IntoIterator
    pub fn any<'t, I>(patterns: I) -> Result<Self, BuildError>
    where
        I: IntoIterator,
        I::Item: Compose<'t>,
    {
        let (exhaustive, nonexhaustive) = patterns
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)?
            .into_iter()
            .partition::<Vec<_>, _>(|tree| token::is_exhaustive(tree.as_ref().tokens()));
        let negation = Negation {
            exhaustive: crate::any(exhaustive)?.pattern,
            nonexhaustive: crate::any(nonexhaustive)?.pattern,
        };
        Ok(negation)
    }

    /// Gets the appropriate [`FilterTarget`] for the given [`WalkEntry`].
    ///
    /// This function can be used with [`FileIterator::filter_tree`] to
    /// effeciently filter [`WalkEntry`]s without reading directory trees from
    /// the file system when not necessary.
    ///
    /// Returns [`FilterTarget::Tree`] if the [`WalkEntry`] matches an
    /// [exhaustive glob expression][`Pattern::is_exhaustive`], such as
    /// `secret/**`.
    ///
    /// [`FileIterator::filter_tree`]: crate::FileIterator::filter_tree
    /// [`FilterTarget`]: crate::FilterTarget
    /// [`FilterTarget::Tree`]: crate::FilterTarget::Tree
    /// [`Pattern::is_exhaustive`]: crate::Pattern::is_exhaustive
    /// [`WalkEntry`]: crate::WalkEntry
    pub fn target(&self, entry: &WalkEntry) -> Option<FilterTarget> {
        let path = entry.to_candidate_path();
        if self.exhaustive.is_match(path.as_ref()) {
            // Do not descend into directories that match the exhaustive
            // negation.
            Some(FilterTarget::Tree)
        } else if self.nonexhaustive.is_match(path.as_ref()) {
            Some(FilterTarget::File)
        } else {
            None
        }
    }
}

/// Configuration for interpreting symbolic links.
///
/// Determines how symbolic links are interpreted when traversing directory
/// trees using functions like [`Glob::walk`]. **By default, symbolic links are
/// read as regular files and their targets are ignored.**
///
/// [`Glob::walk`]: crate::Glob::walk
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Clone, Copy, Debug)]
pub enum LinkBehavior {
    /// Read the symbolic link file itself.
    ///
    /// This behavior reads the symbolic link as a regular file. The
    /// corresponding [`WalkEntry`] uses the path of the link file and its
    /// metadata describes the link file itself. The target is effectively
    /// ignored and traversal will **not** follow the link.
    ///
    /// [`WalkEntry`]: crate::WalkEntry
    ReadFile,
    /// Read the target of the symbolic link.
    ///
    /// This behavior reads the target of the symbolic link. The corresponding
    /// [`WalkEntry`] uses the path of the link file and its metadata describes
    /// the target. If the target is a directory, then traversal will follow the
    /// link and descend into the target.
    ///
    /// If a link is reentrant and forms a cycle, then an error will be emitted
    /// instead of a [`WalkEntry`] and traversal will not follow the link.
    ///
    /// [`WalkEntry`]: crate::WalkEntry
    ReadTarget,
}

impl Default for LinkBehavior {
    fn default() -> Self {
        LinkBehavior::ReadFile
    }
}

/// Configuration for matching [`Glob`]s against directory trees.
///
/// Determines the behavior of the traversal within a directory tree when using
/// functions like [`Glob::walk`]. `WalkBehavior` can be constructed via
/// conversions from types representing its fields. APIs generally accept `impl
/// Into<WalkBehavior>`, so these conversion can be used implicitly. When
/// constructed using such a conversion, `WalkBehavior` will use defaults for
/// any remaining fields.
///
/// # Examples
///
/// By default, symbolic links are interpreted as regular files and targets are
/// ignored. To read linked targets, use [`LinkBehavior::ReadTarget`].
///
/// ```rust
/// use wax::{Glob, LinkBehavior};
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
/// [`Glob`]: crate::Glob
/// [`Glob::walk`]: crate::Glob::walk
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Clone, Copy, Debug)]
pub struct WalkBehavior {
    // TODO: Consider using a dedicated type for this field. Using primitive
    //       types does not interact well with conversions used in `walk` APIs.
    //       For example, if another `usize` field is introduced, then the
    //       conversions become ambiguous and confusing.
    /// Maximum depth.
    ///
    /// Determines the maximum depth to which a directory tree will be traversed
    /// relative to [the root][`Walk::root`]. A depth of zero corresponds to the
    /// root and so using such a depth will yield at most one entry for the
    /// root.
    ///
    /// The default value is [`usize::MAX`].
    ///
    /// [`usize::MAX`]: usize::MAX
    /// [`Walk::root`]: crate::Walk::root
    pub depth: usize,
    /// Interpretation of symbolic links.
    ///
    /// Determines how symbolic links are interpreted when traversing a
    /// directory tree. See [`LinkBehavior`].
    ///
    /// The default value is [`LinkBehavior::ReadFile`].
    ///
    /// [`LinkBehavior`]: crate::LinkBehavior
    /// [`LinkBehavior::ReadFile`]: crate::LinkBehavior::ReadFile
    pub link: LinkBehavior,
}

/// Constructs a `WalkBehavior` using the following defaults:
///
/// | Field     | Description                       | Value
/// |
/// |-----------|-----------------------------------|----------------------------|
/// | [`depth`] | Maximum depth.                    | [`usize::MAX`]
/// | | [`link`]  | Interpretation of symbolic links. |
/// [`LinkBehavior::ReadFile`] |
///
/// [`depth`]: crate::WalkBehavior::depth
/// [`link`]: crate::WalkBehavior::link
/// [`LinkBehavior::ReadFile`]: crate::LinkBehavior::ReadFile
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

/// Iterator over files matching a [`Glob`] in a directory tree.
///
/// `Walk` is a `TreeIterator` and supports [`FileIterator::filter_tree`].
///
/// [`FileIterator::filter_tree`]: crate::FileIterator::filter_tree
/// [`Glob`]: crate::Glob
#[derive(Debug)]
// This type is principally an iterator and is therefore lazy.
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[must_use]
pub struct Walk<'g> {
    pattern: Cow<'g, Regex>,
    components: Vec<Regex>,
    root: PathBuf,
    prefix: PathBuf,
    walk: walkdir::IntoIter,
    // This is a hack to express an empty iterator
    is_empty: bool,
}

impl<'g> Walk<'g> {
    fn empty() -> Self {
        Self {
            pattern: Cow::Owned(Regex::new("").unwrap()),
            components: vec![],
            root: PathBuf::new(),
            prefix: PathBuf::new(),
            walk: walkdir::WalkDir::new(PathBuf::new()).into_iter(),
            is_empty: true,
        }
    }

    fn compile<'t, I>(tokens: I) -> Result<Vec<Regex>, CompileError>
    where
        I: IntoIterator<Item = &'t Token<'t>>,
        I::IntoIter: Clone,
    {
        let mut regexes = Vec::new();
        for component in token::components(tokens) {
            if component
                .tokens()
                .iter()
                .any(|token| token.has_component_boundary())
            {
                // Stop at component boundaries, such as tree wildcards or any
                // boundary within a group token.
                break;
            }
            regexes.push(Glob::compile(component.tokens().iter().copied())?);
        }
        Ok(regexes)
    }

    /// Clones any borrowed data into an owning instance.
    pub fn into_owned(self) -> Walk<'static> {
        let Walk {
            pattern,
            components,
            root,
            prefix,
            walk,
            is_empty,
        } = self;
        Walk {
            pattern: Cow::Owned(pattern.into_owned()),
            components,
            root,
            prefix,
            walk,
            is_empty,
        }
    }

    /// Calls a closure on each matched file or error.
    ///
    /// This function is similar to [`for_each`], but does not clone paths and
    /// [matched text][`MatchedText`] and so may be somewhat more efficient.
    /// Note that the closure receives borrowing [`WalkEntry`]s rather than
    /// `'static` items.
    ///
    /// [`for_each`]: std::iter::Iterator::for_each
    /// [`WalkEntry`]: crate::WalkEntry
    pub fn for_each_ref(mut self, mut f: impl FnMut(WalkItem)) {
        walk!(self => |entry| {
            f(entry);
        });
    }

    /// Filters [`WalkEntry`]s against negated glob expressions.
    ///
    /// This function creates an adaptor that discards [`WalkEntry`]s that match
    /// any of the given glob expressions. This allows for broad negations while
    /// matching a [`Glob`] against a directory tree that cannot be achieved
    /// using a single glob expression alone.
    ///
    /// The adaptor is constructed via [`FilterTree`] and [`Negation`] and
    /// therefore does not read directory trees from the file system when a
    /// directory matches an [exhaustive glob
    /// expression][`Pattern::is_exhaustive`] such as `**/private/**` or
    /// `hidden/<<?>/>*`. **This function should be preferred when filtering
    /// [`WalkEntry`]s against [`Glob`]s, since this avoids potentially large
    /// and unnecessary reads**.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the inputs fail to build. If the inputs are a
    /// compiled [`Pattern`] type such as [`Glob`], then this only occurs if the
    /// compiled program is too large.
    ///
    /// # Examples
    ///
    /// Because glob expressions do not support general negations, it is
    /// sometimes impossible to express patterns that deny particular text. In
    /// such cases, `not` can be used to apply additional patterns as a filter.
    ///
    /// ```rust,no_run
    /// use wax::Glob;
    ///
    /// // Find image files, but not if they are beneath a directory with a name that
    /// // suggests that they are private.
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg,png}").unwrap();
    /// for entry in glob.walk(".").not(["**/(?i)<.:0,1>private/**"]).unwrap() {
    ///     let entry = entry.unwrap();
    ///     // ...
    /// }
    /// ```
    ///
    /// [`FileIterator::filter_tree`]: crate::FileIterator::filter_tree
    /// [`Glob`]: crate::Glob
    /// [`Iterator::filter`]: std::iter::Iterator::filter
    /// [`Negation`]: crate::Negation
    /// [`Pattern`]: crate::Pattern
    /// [`Pattern::is_exhaustive`]: crate::Pattern::is_exhaustive
    /// [`WalkEntry`]: crate::WalkEntry
    pub fn not<'t, I>(self, patterns: I) -> Result<impl 'g + FileIterator, BuildError>
    where
        I: IntoIterator,
        I::Item: Compose<'t>,
    {
        Negation::any(patterns)
            .map(|negation| self.filter_tree(move |entry| negation.target(entry)))
    }

    /// Gets the root directory of the traversal.
    ///
    /// The root directory is determined by joining the directory path in
    /// functions like [`Glob::walk`] with any [invariant
    /// prefix](`Glob::partition`) of the [`Glob`]. When a [`Glob`] is rooted,
    /// the root directory is the same as the invariant prefix.
    ///
    /// The depth specified via [`WalkBehavior`] is relative to this path.
    ///
    /// [`Glob`]: crate::Glob
    /// [`Glob::partition`]: crate::Glob::partition
    /// [`Glob::walk`]: crate::Glob::walk
    /// [`WalkBehavior`]: crate::WalkBehavior
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Iterator for Walk<'_> {
    type Item = WalkItem<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_empty {
            return None;
        }
        walk!(self => |entry| {
            return Some(entry.map(WalkEntry::into_owned));
        });
        None
    }
}

impl TreeIterator for Walk<'_> {
    fn skip_tree(&mut self) {
        self.walk.skip_tree();
    }
}

/// Describes how files are read and discarded by [`FilterTree`].
///
/// [`FilterTree`]: crate::FilterTree
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FilterTarget {
    /// Discard the file.
    ///
    /// The [`WalkEntry`] for the given file is discarded by the [`FilterTree`]
    /// adaptor. Only this particular file is ignored and if the entry
    /// represents a directory, then its tree is still read from the file
    /// system.
    ///
    /// [`FilterTree`]: crate::FilterTree
    /// [`WalkEntry`]: crate::WalkEntry
    File,
    /// Discard the file and its directory tree, if any.
    ///
    /// The [`WalkEntry`] for the given file is discarded by the [`FilterTree`]
    /// adaptor. If the entry represents a directory, then its entire tree is
    /// ignored and is not read from the file system.
    ///
    /// When the [`WalkEntry`] represents a normal file (not a directory), then
    /// this is the same as [`FilterTarget::File`].
    ///
    /// [`FilterTarget::File`]: crate::FilterTarget::File
    /// [`FilterTree`]: crate::FilterTree
    /// [`WalkEntry`]: crate::WalkEntry
    Tree,
}

/// Iterator adaptor that filters [`WalkEntry`]s and controls the traversal of
/// directory trees.
///
/// This adaptor is returned by [`FileIterator::filter_tree`] and in addition to
/// filtering [`WalkEntry`]s also determines how `TreeIterator`s traverse
/// directory trees. If discarded directories do not need to be read from the
/// file system, then **this adaptor should be preferred over functions like
/// [`Iterator::filter`], because it can avoid potentially large and unnecessary
/// reads.**
///
/// `FilterTree` is a `TreeIterator` and supports [`FileIterator::filter_tree`]
/// so `filter_tree` may be chained.
///
/// [`FileIterator::filter_tree`]: crate::FileIterator::filter_tree
/// [`WalkEntry`]: crate::WalkEntry
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Clone, Debug)]
pub struct FilterTree<I, F> {
    input: I,
    f: F,
}

impl<I, F> Iterator for FilterTree<I, F>
where
    I: FileIterator,
    F: FnMut(&WalkEntry<'static>) -> Option<FilterTarget>,
{
    type Item = WalkItem<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(result) = self.input.next() {
                if let Ok(entry) = result.as_ref() {
                    match (self.f)(entry) {
                        None => {
                            return Some(result);
                        }
                        Some(FilterTarget::File) => {
                            continue;
                        }
                        Some(FilterTarget::Tree) => {
                            if entry.file_type().is_dir() {
                                self.input.skip_tree();
                            }
                            continue;
                        }
                    }
                }
                return Some(result);
            }
            return None;
        }
    }
}

impl<I, F> TreeIterator for FilterTree<I, F>
where
    Self: Iterator,
    I: TreeIterator,
{
    fn skip_tree(&mut self) {
        self.input.skip_tree();
    }
}

/// Describes a file matching a [`Glob`] in a directory tree.
///
/// [`Glob`]: crate::Glob
#[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
#[derive(Debug)]
pub struct WalkEntry<'e> {
    entry: Cow<'e, DirEntry>,
    matched: MatchedText<'e>,
}

impl<'e> WalkEntry<'e> {
    /// Clones any borrowed data into an owning instance.
    pub fn into_owned(self) -> WalkEntry<'static> {
        let WalkEntry { entry, matched } = self;
        WalkEntry {
            entry: Cow::Owned(entry.into_owned()),
            matched: matched.into_owned(),
        }
    }

    pub fn into_path(self) -> PathBuf {
        match self.entry {
            Cow::Borrowed(entry) => entry.path().to_path_buf(),
            Cow::Owned(entry) => entry.into_path(),
        }
    }

    /// Gets the path of the matched file.
    pub fn path(&self) -> &Path {
        self.entry.path()
    }

    /// Converts the entry to the relative [`CandidatePath`].
    ///
    /// **This differs from [`path`] and [`into_path`], which are natively
    /// encoded and may be absolute.** The [`CandidatePath`] is always relative
    /// to [the root][`Walk::root`] of the directory tree.
    ///
    /// [`CandidatePath`]: crate::CandidatePath
    /// [`into_path`]: crate::WalkEntry::into_path
    /// [`matched`]: crate::WalkEntry::matched
    /// [`path`]: crate::WalkEntry::path
    pub fn to_candidate_path(&self) -> CandidatePath<'_> {
        self.matched.to_candidate_path()
    }

    pub fn file_type(&self) -> FileType {
        self.entry.file_type()
    }

    pub fn metadata(&self) -> Result<Metadata, WalkError> {
        self.entry.metadata().map_err(WalkError::from)
    }

    /// Gets the depth of the file from [the root][`Walk::root`] of the
    /// directory tree.
    ///
    /// [`Walk::root`]: crate::Walk::root
    pub fn depth(&self) -> usize {
        self.entry.depth()
    }

    /// Gets the matched text in the path of the file.
    pub fn matched(&self) -> &MatchedText<'e> {
        &self.matched
    }
}

pub fn walk<'g>(
    glob: &'g Glob<'_>,
    directory: impl AsRef<Path>,
    behavior: impl Into<WalkBehavior>,
) -> Walk<'g> {
    let directory = directory.as_ref();
    let WalkBehavior { depth, link } = behavior.into();
    // The directory tree is traversed from `root`, which may include an
    // invariant prefix from the glob pattern. `Walk` patterns are only applied
    // to path components following this prefix in `root`.
    let (root, prefix) = invariant_path_prefix(glob.tree.as_ref().tokens()).map_or_else(
        || {
            let root = Cow::from(directory);
            (root.clone(), root)
        },
        |prefix| {
            let root = directory.join(&prefix).into();
            if prefix.is_absolute() {
                // Absolute paths replace paths with which they are joined,
                // in which case there is no prefix.
                (root, PathBuf::new().into())
            } else {
                (root, directory.into())
            }
        },
    );
    if matches!(link, LinkBehavior::ReadFile) {
        if let Ok(tail) = root.strip_prefix(directory) {
            let found = tail
                .components()
                .try_fold(directory.to_path_buf(), |accum, c| {
                    let candidate = accum.join(c);
                    if candidate.is_symlink() {
                        None
                    } else {
                        Some(candidate)
                    }
                })
                .is_none();
            if found {
                return Walk::empty();
            }
        }
    }
    let components =
        Walk::compile(glob.tree.as_ref().tokens()).expect("failed to compile glob sub-expressions");
    Walk {
        pattern: Cow::Borrowed(&glob.pattern),
        components,
        root: root.clone().into_owned(),
        prefix: prefix.into_owned(),
        walk: WalkDir::new(root.clone())
            .follow_links(match link {
                LinkBehavior::ReadFile => false,
                LinkBehavior::ReadTarget => true,
            })
            .max_depth(depth)
            .into_iter(),
        is_empty: false,
    }
}

fn invariant_path_prefix<'t, A, I>(tokens: I) -> Option<PathBuf>
where
    A: 't,
    I: IntoIterator<Item = &'t Token<'t, A>>,
{
    let prefix = token::invariant_text_prefix(tokens);
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.into())
    }
}
