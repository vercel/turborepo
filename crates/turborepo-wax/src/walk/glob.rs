use std::{
    fs::{FileType, Metadata},
    path::{Component, Path, PathBuf},
};

use itertools::Itertools;
use regex::Regex;

use super::SplitAtDepth;
use crate::{
    capture::MatchedText,
    encode::CompileError,
    token::{self, Token, TokenTree},
    walk::{
        filter::{HierarchicalIterator, Separation},
        Entry, EntryResidue, FileIterator, JoinAndGetDepth, TreeEntry, WalkBehavior, WalkError,
        WalkTree,
    },
    BuildError, CandidatePath, Glob, Pattern,
};

/// APIs for matching globs against directory trees.
impl<'t> Glob<'t> {
    /// Gets an iterator over matching file paths in a directory tree.
    ///
    /// This function matches a `Glob` against a directory tree, returning a
    /// [`FileIterator`] that yields a [`GlobEntry`] for each matching file.
    /// `Glob`s are the only [`Pattern`]s that support this semantic
    /// operation; it is not possible to match combinators ([`Any`]) against
    /// directory trees.
    ///
    /// As with [`Path::join`] and [`PathBuf::push`], the base directory can be
    /// escaped or overridden by rooted `Glob`s. In many cases, the current
    /// working directory `.` is an appropriate base directory and will be
    /// intuitively ignored if the `Glob` is rooted, such as in `/mnt/media/
    /// **/*.mp4`. The [`has_root`] function can be used to check if a `Glob` is
    /// rooted.
    ///
    /// The root directory is either the given directory or, if rooted, the
    /// [invariant prefix][`Glob::partition`] of the `Glob`. Either way,
    /// this function joins the given directory with any invariant prefix to
    /// potentially begin the walk as far down the tree as possible. **The
    /// prefix and any [semantic literals][`Glob::has_semantic_literals`] in
    /// this prefix are interpreted semantically as a path**, so components
    /// like `.` and `..` that precede variant patterns interact with the
    /// base directory semantically. This means that expressions like
    /// `../**` escape the base directory as expected on Unix and Windows, for
    /// example. To query the root directory of the walk, see [`Glob::walker`].
    ///
    /// This function uses the default [`WalkBehavior`]. To configure the
    /// behavior of the traversal, see [`Glob::walk_with_behavior`].
    ///
    /// Unlike functions in [`Pattern`], **this operation is semantic and
    /// interacts with the file system**.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wax::walk::Entry;
    /// use wax::Glob;
    ///
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg}").unwrap();
    /// for entry in glob.walk("./Pictures") {
    ///     let entry = entry.unwrap();
    ///     println!("JPEG: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// Glob expressions do not support general negations, but the [`not`]
    /// combinator can be used when walking a directory tree to filter
    /// entries using patterns. **This should generally be preferred over
    /// functions like [`Iterator::filter`], because it avoids unnecessary reads
    /// of directory trees when matching [exhaustive
    /// negations][`Pattern::is_exhaustive`].**
    ///
    /// ```rust,no_run
    /// use wax::walk::{Entry, FileIterator};
    /// use wax::Glob;
    ///
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg,png}").unwrap();
    /// for entry in glob
    ///     .walk("./Pictures")
    ///     .not(["**/(i?){background<s:0,1>,wallpaper<s:0,1>}/**"])
    ///     .unwrap()
    /// {
    ///     let entry = entry.unwrap();
    ///     println!("{:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`Any`]: crate::Any
    /// [`Glob::walk_with_behavior`]: crate::Glob::walk_with_behavior
    /// [`Glob::walker`]: crate::Glob::walker
    /// [`GlobEntry`]: crate::walk::GlobEntry
    /// [`has_root`]: crate::Glob::has_root
    /// [`FileIterator`]: crate::walk::FileIterator
    /// [`Iterator::filter`]: std::iter::Iterator::filter
    /// [`not`]: crate::walk::FileIterator::not
    /// [`Path::join`]: std::path::Path::join
    /// [`PathBuf::push`]: std::path::PathBuf::push
    /// [`Program`]: crate::Program
    /// [`Program::is_exhaustive`]: crate::Program::is_exhaustive
    /// [`WalkBehavior`]: crate::walk::WalkBehavior
    pub fn walk(
        &self,
        directory: impl Into<PathBuf>,
    ) -> impl 'static + FileIterator<Entry = GlobEntry> {
        self.walk_with_behavior(directory, WalkBehavior::default())
    }

    /// Gets an iterator over matching files in a directory tree.
    ///
    /// This function is the same as [`Glob::walk`], but it additionally accepts
    /// a [`WalkBehavior`] that configures how the traversal interacts with
    /// symbolic links, the maximum depth from the root, etc.
    ///
    /// Depth is relative to the root directory of the traversal, which is
    /// determined by joining the given path and any [invariant
    /// prefix][`Glob::partition`] of the `Glob`.
    ///
    /// See [`Glob::walk`] for more information.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wax::walk::{Entry, WalkBehavior};
    /// use wax::Glob;
    ///
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg}").unwrap();
    /// for entry in glob.walk_with_behavior("./Pictures", WalkBehavior::default()) {
    ///     let entry = entry.unwrap();
    ///     println!("JPEG: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// By default, symbolic links are read as normal files and their targets
    /// are ignored. To follow symbolic links and traverse any directories
    /// that they reference, specify a [`LinkBehavior`].
    ///
    /// ```rust,no_run
    /// use wax::walk::{Entry, LinkBehavior};
    /// use wax::Glob;
    ///
    /// let glob = Glob::new("**/*.txt").unwrap();
    /// for entry in glob.walk_with_behavior("/var/log", LinkBehavior::ReadTarget) {
    ///     let entry = entry.unwrap();
    ///     println!("Log: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`Glob::partition`]: crate::Glob::partition
    /// [`Glob::walk`]: crate::Glob::walk
    /// [`LinkBehavior`]: crate::walk::LinkBehavior
    /// [`WalkBehavior`]: crate::walk::WalkBehavior
    pub fn walk_with_behavior(
        &self,
        directory: impl Into<PathBuf>,
        behavior: impl Into<WalkBehavior>,
    ) -> impl 'static + FileIterator<Entry = GlobEntry> {
        self.walker(directory).walk_with_behavior(behavior)
    }

    /// Gets an iterator builder over matching files in a directory tree.
    ///
    /// This function gets an intermediate walker that describes iteration over
    /// matching files and provides paths prior to iteration. In particular,
    /// `walker` can be used when the root directory of the walk is needed.
    /// **The root directory may differ from the directory passed to walking
    /// functions.**
    ///
    /// See [`Glob::walk`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wax::walk::Entry;
    /// use wax::Glob;
    ///
    /// let glob = Glob::new("**/*.{log,txt}").unwrap();
    /// let walker = glob.walker("/var/log");
    /// let root = walker.root_prefix_paths().0.to_path_buf();
    /// for entry in walker.walk() {
    ///     let entry = entry.unwrap();
    ///     println!("Log: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`Glob::walk`]: crate::Glob::walk
    pub fn walker(&self, directory: impl Into<PathBuf>) -> GlobWalker {
        GlobWalker {
            anchor: self.anchor(directory),
            program: WalkProgram::from_glob(self),
        }
    }

    fn anchor(&self, directory: impl Into<PathBuf>) -> Anchor {
        fn invariant_path_prefix<'t, A, I>(tokens: I, root: &Path) -> Option<PathBuf>
        where
            A: 't,
            I: IntoIterator<Item = &'t Token<'t, A>>,
        {
            let prefix = token::invariant_text_prefix(tokens);
            if prefix.is_empty() {
                None
            } else {
                // here, we don't know if the glob will be walked with or without symlinks,
                // so we need to ensure that the invariant prefix optimisation doesn't cross a
                // symlink todo: `anchor` knows nothing about the walk behaviour. if it did, we
                // could probably skip this conditionally for a small perf bonus
                let prefix: PathBuf = prefix.into();
                let mut curr_prefix = prefix.as_path();
                let mut last_symlink = None;
                while let Some(parent) = curr_prefix.parent() {
                    // make sure we don't traverse out of the root
                    if curr_prefix == root {
                        break;
                    }

                    if parent.is_symlink() {
                        last_symlink = Some(parent);
                    }
                    curr_prefix = parent;
                }
                // we found the last symlink, but we need the chance to
                // filter it, so take the parent one more time
                Some(
                    last_symlink
                        .and_then(Path::parent)
                        .map(Into::into)
                        .unwrap_or(prefix),
                )
            }
        }

        let directory = directory.into();
        // Establish the root directory and any prefix in that root path that is not a
        // part of the glob expression. The directory tree is traversed from
        // `root`, which may include an invariant prefix from the glob. The
        // `prefix` is an integer that specifies how many components from the
        // end of the root path must be popped to get the portion of the root
        // path that is not present in the glob. The prefix may be empty or may be the
        // entirety of `root` depending on `directory` and the glob.
        //
        // Note that a rooted glob, like in `Path::join`, replaces `directory` when
        // establishing the root path. In this case, there is no prefix, as the
        // entire root path is present in the glob expression.
        let (root, prefix) = match invariant_path_prefix(self.tree.as_ref().tokens(), &directory) {
            Some(prefix) => directory.join_and_get_depth(prefix),
            _ => (directory, 0),
        };
        Anchor { root, prefix }
    }
}

/// Root path and prefix of a `Glob` when walking a particular path.
#[derive(Clone, Debug)]
struct Anchor {
    /// The root (starting) directory of the walk.
    root: PathBuf,
    // TODO: Is there a better name for this? This is a prefix w.r.t. a glob but is a suffix w.r.t.
    //       the root directory. This can be a bit confusing since either perspective is reasonable
    //       (and in some contexts one may be more intuitive than the other).
    /// The number of components from the end of `root` that are present in the
    /// `Glob`'s expression.
    prefix: usize,
}

impl Anchor {
    pub fn root_prefix_paths(&self) -> (&Path, &Path) {
        self.root.split_at_depth(self.prefix)
    }

    pub fn walk_with_behavior(self, behavior: impl Into<WalkBehavior>) -> WalkTree {
        WalkTree::with_prefix_and_behavior(self.root, self.prefix, behavior)
    }
}

#[derive(Clone, Debug)]
struct WalkProgram {
    complete: Regex,
    components: Vec<Regex>,
}

impl WalkProgram {
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
                // Stop at component boundaries, such as tree wildcards or any boundary within a
                // group token.
                break;
            }
            regexes.push(Glob::compile(component.tokens().iter().copied())?);
        }
        Ok(regexes)
    }

    fn from_glob(glob: &Glob<'_>) -> Self {
        WalkProgram {
            complete: glob.program.clone(),
            components: WalkProgram::compile(glob.tree.as_ref().tokens())
                .expect("failed to compile glob sub-expressions"),
        }
    }
}

/// Describes iteration over matching files in a directory tree.
///
/// A walker provides the paths walked by a [`Glob`] prior to iteration, most
/// notably the [root path][`GlobWalker::root_prefix_paths`], which may differ
/// from the directory passed to walking functions. When ready, it can be
/// converted into an iterator over matching files.
///
/// See [`Glob::walker`].
///
/// [`Glob`]: crate::Glob
/// [`Glob::walker`]: crate::Glob::walker
/// [`GlobWalker::root_prefix_paths`]: crate::walk::GlobWalker::root_prefix_paths
#[derive(Clone, Debug)]
pub struct GlobWalker {
    anchor: Anchor,
    program: WalkProgram,
}

impl GlobWalker {
    /// Gets the root and prefix paths.
    ///
    /// The root path is the path to the walked directory tree. **This path may
    /// differ from the directory passed to walking functions like
    /// [`Glob::walk`]**, because it may incorporate an invariant path
    /// prefix from the glob expression.
    ///
    /// The prefix path is the invariant path prefix of the glob expression.
    /// This path may be empty and is always a suffix of the root path.
    ///
    /// The following table describes some example paths when using
    /// [`Glob::walk`].
    ///
    /// | Glob Expression           | Directory    | Root         | Prefix     |
    /// |---------------------------|--------------|--------------|------------|
    /// | `**/*.txt`                | `/home/user` | `/home/user` |            |
    /// | `projects/**/src/**/*.rs` | `.`          | `./projects` | `projects` |
    /// | `/var/log/**/*.log`       | `.`          | `/var/log`   | `/var/log` |
    ///
    /// See also [`Entry::root_relative_paths`].
    ///
    /// [`Entry::root_relative_paths`]: crate::walk::Entry::root_relative_paths
    /// [`Glob::walk`]: crate::Glob::walk
    pub fn root_prefix_paths(&self) -> (&Path, &Path) {
        self.anchor.root_prefix_paths()
    }

    /// Converts a walker into an iterator over matching files in its directory
    /// tree.
    ///
    /// See [`Glob::walk`].
    ///
    /// [`Glob::walk`]: crate::Glob::walk
    pub fn walk(self) -> impl 'static + FileIterator<Entry = GlobEntry> {
        self.walk_with_behavior(WalkBehavior::default())
    }

    /// Converts a walker into an iterator over matching files in its directory
    /// tree.
    ///
    /// See [`Glob::walk_with_behavior`].
    ///
    /// [`Glob::walk_with_behavior`]: crate::Glob::walk_with_behavior
    pub fn walk_with_behavior(
        self,
        behavior: impl Into<WalkBehavior>,
    ) -> impl 'static + FileIterator<Entry = GlobEntry, Residue = TreeEntry> {
        self.anchor
            .walk_with_behavior(behavior)
            .filter_map_tree(move |cancellation, separation| {
                use itertools::{
                    EitherOrBoth::{Both, Left, Right},
                    Position::{First, Last, Middle, Only},
                };

                let filtrate = match separation.filtrate() {
                    Some(filtrate) => match filtrate.transpose() {
                        Ok(filtrate) => filtrate,
                        Err(error) => {
                            return Separation::from(error.map(Err));
                        }
                    },
                    // `Path::walk_with_behavior` yields no residue.
                    _ => unreachable!(),
                };
                let entry = filtrate.as_ref();
                let (_, path) = entry.root_relative_paths();
                let depth = entry.depth().saturating_sub(1);
                for (position, candidate) in path
                    .components()
                    .filter_map(|component| match component {
                        Component::Normal(component) => Some(CandidatePath::from(component)),
                        Component::Prefix(prefix) => Some(CandidatePath::from(prefix.as_os_str())),
                        _ => None,
                    })
                    .skip(depth)
                    .zip_longest(self.program.components.iter().skip(depth))
                    .with_position()
                {
                    match (position, candidate) {
                        (First | Middle, Both(candidate, program)) => {
                            if !program.is_match(candidate.as_ref()) {
                                // Do not walk directories that do not match the corresponding
                                // component program.
                                return filtrate.filter_tree(cancellation).into();
                            }
                        }
                        (Last | Only, Both(candidate, program)) => {
                            return if program.is_match(candidate.as_ref()) {
                                let candidate = CandidatePath::from(path);
                                if let Some(matched) = self
                                    .program
                                    .complete
                                    .captures(candidate.as_ref())
                                    .map(MatchedText::from)
                                    .map(MatchedText::into_owned)
                                {
                                    filtrate
                                        .map(|entry| Ok(GlobEntry { entry, matched }))
                                        .into()
                                } else {
                                    filtrate.filter_node().into()
                                }
                            } else {
                                // Do not walk directories that do not match the corresponding
                                // component program.
                                filtrate.filter_tree(cancellation).into()
                            };
                        }
                        (_, Left(_candidate)) => {
                            let candidate = CandidatePath::from(path);
                            return if let Some(matched) = self
                                .program
                                .complete
                                .captures(candidate.as_ref())
                                .map(MatchedText::from)
                                .map(MatchedText::into_owned)
                            {
                                filtrate
                                    .map(|entry| Ok(GlobEntry { entry, matched }))
                                    .into()
                            } else {
                                filtrate.filter_node().into()
                            };
                        }
                        (_, Right(_program)) => {
                            return filtrate.filter_node().into();
                        }
                    }
                }
                // If the component loop is not entered, then check for a match. This may
                // indicate that the `Glob` is empty and a single invariant path
                // may be matched.
                let candidate = CandidatePath::from(path);
                if let Some(matched) = self
                    .program
                    .complete
                    .captures(candidate.as_ref())
                    .map(MatchedText::from)
                    .map(MatchedText::into_owned)
                {
                    return filtrate
                        .map(|entry| Ok(GlobEntry { entry, matched }))
                        .into();
                }
                filtrate.filter_node().into()
            })
    }
}

#[derive(Clone, Debug)]
enum FilterAnyProgram {
    Empty,
    Exhaustive(Regex),
    Nonexhaustive(Regex),
    Partitioned {
        exhaustive: Regex,
        nonexhaustive: Regex,
    },
}

impl FilterAnyProgram {
    fn compile<'t, I>(tokens: I) -> Result<Option<Regex>, BuildError>
    where
        I: IntoIterator,
        I::Item: Pattern<'t>,
        I::IntoIter: ExactSizeIterator,
    {
        let tokens = tokens.into_iter();
        if 0 == tokens.len() {
            Ok(None)
        } else {
            crate::any(tokens).map(|any| Some(any.program))
        }
    }

    fn from_partitions<'t, I>(exhaustive: I, nonexhaustive: I) -> Result<Self, BuildError>
    where
        I: IntoIterator,
        I::Item: Pattern<'t>,
        I::IntoIter: ExactSizeIterator,
    {
        use FilterAnyProgram::{Empty, Exhaustive, Nonexhaustive, Partitioned};

        // It is important to distinguish between empty _partitions_ and empty
        // _expressions_ here. `FilterAnyProgram::compile` discards empty
        // partitions. When matching against an empty path, an explicit empty
        // _expression_ must match but an empty _partition_ must not (such
        // a partition must never match anything).
        Ok(
            match (
                FilterAnyProgram::compile(exhaustive)?,
                FilterAnyProgram::compile(nonexhaustive)?,
            ) {
                (Some(exhaustive), Some(nonexhaustive)) => Partitioned {
                    exhaustive,
                    nonexhaustive,
                },
                (Some(exhaustive), None) => Exhaustive(exhaustive),
                (None, Some(nonexhaustive)) => Nonexhaustive(nonexhaustive),
                (None, None) => Empty,
            },
        )
    }

    pub fn residue(&self, candidate: CandidatePath<'_>) -> Option<EntryResidue> {
        use FilterAnyProgram::{Exhaustive, Nonexhaustive, Partitioned};

        match self {
            Exhaustive(ref exhaustive) | Partitioned { ref exhaustive, .. }
                if exhaustive.is_match(candidate.as_ref()) =>
            {
                Some(EntryResidue::Tree)
            }
            Nonexhaustive(ref nonexhaustive)
            | Partitioned {
                ref nonexhaustive, ..
            } if nonexhaustive.is_match(candidate.as_ref()) => Some(EntryResidue::File),
            _ => None,
        }
    }
}

/// Negated glob combinator that efficiently filters file entries against
/// patterns.
#[derive(Clone, Debug)]
pub struct FilterAny {
    program: FilterAnyProgram,
}

impl FilterAny {
    /// Combines patterns into a `FilterAny`.
    ///
    /// This function accepts an [`IntoIterator`] with items that implement
    /// [`Pattern`], such as [`Glob`] and `&str`.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the inputs fail to build. If the inputs are a
    /// compiled [`Program`] type such as [`Glob`], then this only occurs if
    /// the compiled program is too large.
    ///
    /// [`Glob`]: crate::Glob
    /// [`IntoIterator`]: std::iter::IntoIterator
    /// [`Pattern`]: crate::Pattern
    /// [`Program`]: crate::Program
    pub fn any<'t, I>(patterns: I) -> Result<Self, BuildError>
    where
        I: IntoIterator,
        I::Item: Pattern<'t>,
    {
        let (exhaustive, nonexhaustive) = patterns
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)?
            .into_iter()
            .partition::<Vec<_>, _>(|tree| token::is_exhaustive(tree.as_ref().tokens()));
        Ok(FilterAny {
            program: FilterAnyProgram::from_partitions(exhaustive, nonexhaustive)?,
        })
    }

    /// Gets the appropriate [`EntryResidue`] for the given [`Entry`].
    ///
    /// Notably, this function returns [`EntryResidue::Tree`] if the [`Entry`]
    /// matches an [exhaustive glob expression][`Program::is_exhaustive`],
    /// such as `secret/**`.
    ///
    /// [`Entry`]: crate::walk::Entry
    /// [`EntryResidue`]: crate::walk::EntryResidue
    /// [`EntryResidue::Tree`]: crate::walk::EntryResidue::Tree
    /// [`Program::is_exhaustive`]: crate::Program::is_exhaustive
    pub fn residue(&self, entry: &dyn Entry) -> Option<EntryResidue> {
        let candidate = CandidatePath::from(entry.root_relative_paths().1);
        self.program.residue(candidate)
    }
}

/// Describes a file with a path matching a [`Glob`] in a directory tree.
///
/// See [`Glob::walk`].
///
/// [`Glob`]: crate::Glob
/// [`Glob::walk`]: crate::Glob::walk
#[derive(Debug)]
pub struct GlobEntry {
    entry: TreeEntry,
    matched: MatchedText<'static>,
}

impl GlobEntry {
    /// Converts the entry to the relative [`CandidatePath`].
    ///
    /// **This differs from [`Entry::path`] and [`Entry::into_path`], which are
    /// native paths and typically include the root path.** The
    /// [`CandidatePath`] is always relative to [the root
    /// path][`Entry::root_relative_paths`].
    ///
    /// [`CandidatePath`]: crate::CandidatePath
    /// [`Entry::into_path`]: crate::walk::Entry::into_path
    /// [`Entry::path`]: crate::walk::Entry::path
    /// [`matched`]: crate::walk::GlobEntry::matched
    pub fn to_candidate_path(&self) -> CandidatePath<'_> {
        self.matched.to_candidate_path()
    }

    /// Gets the matched text in the path of the file.
    pub fn matched(&self) -> &MatchedText<'static> {
        &self.matched
    }
}

impl Entry for GlobEntry {
    fn into_path(self) -> PathBuf {
        self.entry.into_path()
    }

    fn path(&self) -> &Path {
        self.entry.path()
    }

    fn root_relative_paths(&self) -> (&Path, &Path) {
        self.entry.root_relative_paths()
    }

    fn file_type(&self) -> FileType {
        self.entry.file_type()
    }

    fn metadata(&self) -> Result<Metadata, WalkError> {
        self.entry.metadata().map_err(WalkError::from)
    }

    // TODO: This needs some work and requires some explanation when applied to
    // globs.
    fn depth(&self) -> usize {
        self.entry.depth()
    }
}

impl From<GlobEntry> for TreeEntry {
    fn from(entry: GlobEntry) -> Self {
        entry.entry
    }
}
