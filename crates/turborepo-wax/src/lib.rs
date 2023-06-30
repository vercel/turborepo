//! Wax provides opinionated and portable globs that can be matched against file
//! paths and directory trees. Globs use a familiar syntax and support
//! expressive features with semantics that emphasize component boundaries.
//!
//! See the [repository
//! documentation](https://github.com/olson-sean-k/wax/blob/master/README.md)
//! for details about glob expressions and patterns.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/olson-sean-k/wax/master/doc/wax-favicon.svg?sanitize=true"
)]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/olson-sean-k/wax/master/doc/wax.svg?sanitize=true"
)]
#![allow(clippy::all)]
#![deny(
    clippy::cast_lossless,
    clippy::checked_conversions,
    clippy::cloned_instead_of_copied,
    clippy::explicit_into_iter_loop,
    clippy::filter_map_next,
    clippy::flat_map_option,
    clippy::from_iter_instead_of_collect,
    clippy::if_not_else,
    clippy::manual_ok_or,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_else,
    clippy::unreadable_literal,
    clippy::unused_self
)]

mod capture;
mod diagnostics;
mod encode;
mod rule;
mod token;
mod walk;

use std::{
    borrow::{Borrow, Cow},
    convert::Infallible,
    ffi::OsStr,
    fmt::{self, Debug, Display, Formatter},
    path::{Path, PathBuf},
    str::{self, FromStr},
};

use itertools::Position;
#[cfg(feature = "miette")]
use miette::Diagnostic;
use regex::Regex;
#[cfg(feature = "miette")]
use tardar::{DiagnosticResult, DiagnosticResultExt as _, IteratorExt as _, ResultExt as _};
use thiserror::Error;

#[cfg(feature = "walk")]
pub use crate::walk::{
    FileIterator, FilterTarget, FilterTree, LinkBehavior, Negation, Walk, WalkBehavior, WalkEntry,
    WalkError,
};
pub use crate::{
    capture::MatchedText,
    diagnostics::{LocatedError, Span},
};
use crate::{
    encode::CompileError,
    rule::{Checked, RuleError},
    token::{InvariantText, ParseError, Token, TokenTree, Tokenized},
};

#[cfg(windows)]
const PATHS_ARE_CASE_INSENSITIVE: bool = true;
#[cfg(not(windows))]
const PATHS_ARE_CASE_INSENSITIVE: bool = false;

trait CharExt: Sized {
    /// Returns `true` if the character (code point) has casing.
    fn has_casing(self) -> bool;
}

impl CharExt for char {
    fn has_casing(self) -> bool {
        self.is_lowercase() != self.is_uppercase()
    }
}

trait StrExt {
    /// Returns `true` if any characters in the string have casing.
    fn has_casing(&self) -> bool;
}

impl StrExt for str {
    fn has_casing(&self) -> bool {
        self.chars().any(CharExt::has_casing)
    }
}

trait PositionExt<T> {
    fn as_tuple(&self) -> (Position<()>, &T);

    fn map<U, F>(self, f: F) -> Position<U>
    where
        F: FnMut(T) -> U;

    fn interior_borrow<B>(&self) -> Position<&B>
    where
        T: Borrow<B>;
}

impl<T> PositionExt<T> for Position<T> {
    fn as_tuple(&self) -> (Position<()>, &T) {
        match *self {
            Position::First(ref item) => (Position::First(()), item),
            Position::Middle(ref item) => (Position::Middle(()), item),
            Position::Last(ref item) => (Position::Last(()), item),
            Position::Only(ref item) => (Position::Only(()), item),
        }
    }

    fn map<U, F>(self, mut f: F) -> Position<U>
    where
        F: FnMut(T) -> U,
    {
        match self {
            Position::First(item) => Position::First(f(item)),
            Position::Middle(item) => Position::Middle(f(item)),
            Position::Last(item) => Position::Last(f(item)),
            Position::Only(item) => Position::Only(f(item)),
        }
    }

    fn interior_borrow<B>(&self) -> Position<&B>
    where
        T: Borrow<B>,
    {
        match *self {
            Position::First(ref item) => Position::First(item.borrow()),
            Position::Middle(ref item) => Position::Middle(item.borrow()),
            Position::Last(ref item) => Position::Last(item.borrow()),
            Position::Only(ref item) => Position::Only(item.borrow()),
        }
    }
}

/// Token that captures matched text in a glob expression.
///
/// # Examples
///
/// `CapturingToken`s can be used to isolate sub-expressions.
///
/// ```rust
/// use wax::Glob;
///
/// let expression = "**/*.txt";
/// let glob = Glob::new(expression).unwrap();
/// for token in glob.captures() {
///     let (start, n) = token.span();
///     println!("capturing sub-expression: {}", &expression[start..][..n]);
/// }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct CapturingToken {
    index: usize,
    span: Span,
}

impl CapturingToken {
    /// Gets the index of the capture.
    ///
    /// Captures are one-indexed and the index zero always represents the
    /// implicit capture of the complete match, so the index of
    /// `CapturingToken`s is always one or greater. See [`MatchedText`].
    ///
    /// [`MatchedText`]: crate::MatchedText
    pub fn index(&self) -> usize {
        self.index
    }

    /// Gets the span of the token's sub-expression.
    pub fn span(&self) -> Span {
        self.span
    }
}

// This type is similar to `token::Variance<InvariantText<'_>>`, but is
// simplified for the public API. Invariant text is always expressed as a path
// and no variant bounds are provided.
/// Variance of a [`Pattern`].
///
/// The variance of a pattern describes the kinds of paths it can match with
/// respect to the platform file system APIs. [`Pattern`]s are either variant or
/// invariant.
///
/// An invariant [`Pattern`] can be represented and completely described by an
/// equivalent path using the platform's file system APIs. For example, the glob
/// expression `path/to/file.txt` resolves identically to the paths
/// `path/to/file.txt` and `path\to\file.txt` on Unix and Windows, respectively.
///
/// A variant [`Pattern`] resolves differently than any particular path used
/// with the platform's file system APIs. Such an expression cannot be
/// represented by a single path. This is typically because the expression
/// matches multiple texts using a regular pattern, such as in the glob
/// expression `**/*.rs`.
///
/// [`Pattern`]: crate::Pattern
/// [`Variance`]: crate::Variance
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Variance {
    /// A [`Pattern`] is invariant and equivalent to a path.
    ///
    /// Some non-literal expressions may be invariant, such as in the expression
    /// `path/[t][o]/{file,file}.txt`, which is invariant on Unix (but not on
    /// Windows, because the character class expressions do not match with case
    /// folding).
    ///
    /// [`Pattern`]: crate::Pattern
    Invariant(
        /// An equivalent path that completely describes the invariant
        /// [`Pattern`] with respect to platform file system APIs.
        ///
        /// [`Pattern`]: crate::Pattern
        PathBuf,
    ),
    /// A [`Pattern`] is variant and cannot be completely described by a path.
    ///
    /// Variant expressions may be formed from literals or other **seemingly**
    /// invariant expressions. For example, the variance of literals considers
    /// the case sensitivity of the platform's file system APIs, so the
    /// expression `(?i)path/to/file.txt` is variant on Unix but not on Windows.
    /// Similarly, the expression `path/[t][o]/file.txt` is variant on Windows
    /// but not on Unix.
    ///
    /// [`Pattern`]: crate::Pattern
    Variant,
}

impl Variance {
    /// Gets the equivalent native path if invariant.
    ///
    /// Returns `None` if variant.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Variance::Invariant(ref path) => Some(path),
            Variance::Variant => None,
        }
    }

    /// Returns `true` if invariant.
    pub fn is_invariant(&self) -> bool {
        matches!(self, Variance::Invariant(_))
    }

    /// Returns `true` if variant.
    pub fn is_variant(&self) -> bool {
        matches!(self, Variance::Variant)
    }
}

impl From<token::Variance<InvariantText<'_>>> for Variance {
    fn from(variance: token::Variance<InvariantText<'_>>) -> Self {
        match variance {
            token::Variance::Invariant(text) => {
                Variance::Invariant(PathBuf::from(text.to_string().into_owned()))
            }
            token::Variance::Variant(_) => Variance::Variant,
        }
    }
}

/// A compiled glob expression that can be inspected and matched against paths.
///
/// Matching is a logical operation and does **not** interact with a file
/// system. To handle path operations, use [`Path`] and/or [`PathBuf`] and their
/// associated functions. See [`Glob::partition`] for more about globs and path
/// operations.
///
/// [`Glob::partition`]: crate::Glob::partition
/// [`Path`]: std::path::Path
/// [`PathBuf`]: std::path::PathBuf
pub trait Pattern<'t>: Compose<'t, Error = Infallible> {
    /// Returns `true` if a path matches the pattern.
    ///
    /// The given path must be convertible into a [`CandidatePath`].
    ///
    /// [`CandidatePath`]: crate::CandidatePath
    fn is_match<'p>(&self, path: impl Into<CandidatePath<'p>>) -> bool;

    /// Gets [matched text][`MatchedText`] in a [`CandidatePath`].
    ///
    /// Returns `None` if the [`CandidatePath`] does not match the pattern.
    ///
    /// [`CandidatePath`]: crate::CandidatePath
    /// [`MatchedText`]: crate::MatchedText
    fn matched<'p>(&self, path: &'p CandidatePath<'_>) -> Option<MatchedText<'p>>;

    /// Gets the variance of the pattern.
    ///
    /// The variance of a pattern describes the kinds of paths it can match with
    /// respect to the platform file system APIs.
    fn variance(&self) -> Variance;

    /// Returns `true` if the pattern is exhaustive.
    ///
    /// A glob expression is exhaustive if its terminating component matches any
    /// and all sub-trees, such as in the expressions `/home/**` and
    /// `local/<<?>/>*`.
    fn is_exhaustive(&self) -> bool;
}

/// A glob expression representation that can be composed into a combinator like
/// [`Any`].
///
/// See implementors and the [`any`] function.
///
/// [`any`]: crate::any
/// [`Any`]: crate::Any
pub trait Compose<'t>:
    TryInto<Checked<Self::Tokens>, Error = <Self as Compose<'t>>::Error>
{
    type Tokens: TokenTree<'t>;
    type Error: Into<BuildError>;
}

impl<'t> Compose<'t> for &'t str {
    type Tokens = Tokenized<'t>;
    type Error = BuildError;
}

/// General errors concerning [`Pattern`]s.
///
/// This is the most general error and each of its variants exposes a particular
/// error type that describes the details of its associated error condition.
/// This error is not used in any Wax APIs directly, but can be used to
/// encapsulate the more specific errors that are.
///
/// # Examples
///
/// To encapsulate different errors in the Wax API behind a function, convert
/// them into a `GlobError` via `?`.
///
/// ```rust,no_run,ignore
/// use std::path::Path;
/// use wax::{Glob, GlobError};
///
/// fn read_all(directory: impl AsRef<Path>) -> Result<Vec<u8>, GlobError> {
///     let mut data = Vec::new();
///     let glob = Glob::new("**/*.data.bin")?;
///     for entry in glob.walk(directory) {
///         let entry = entry?;
///         // ...
///     }
///     Ok(data)
/// }
/// ```
///
/// [`Pattern`]: crate::Pattern
#[cfg_attr(feature = "miette", derive(Diagnostic))]
#[derive(Debug, Error)]
#[error(transparent)]
pub enum GlobError {
    #[cfg_attr(feature = "miette", diagnostic(transparent))]
    Build(BuildError),
    #[cfg(feature = "walk")]
    #[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
    #[cfg_attr(feature = "miette", diagnostic(code = "wax::glob::walk"))]
    Walk(WalkError),
}

impl From<BuildError> for GlobError {
    fn from(error: BuildError) -> Self {
        GlobError::Build(error)
    }
}

#[cfg(feature = "walk")]
impl From<WalkError> for GlobError {
    fn from(error: WalkError) -> Self {
        GlobError::Walk(error)
    }
}

// TODO: `Diagnostic` is implemented with macros for brevity and to ensure
//       complete coverage of features. However, this means that documentation
//       does not annotate the implementation with a feature flag requirement.
//       If possible, perhaps in a later version of Rust, close this gap.
/// Describes errors that occur when building a [`Pattern`] from a glob
/// expression.
///
/// Glob expressions may fail to build if they cannot be parsed, violate rules,
/// or cannot be compiled. Parsing errors occur when a glob expression has
/// invalid syntax. Patterns must also follow rules as described in the
/// [repository
/// documentation](https://github.com/olson-sean-k/wax/blob/master/README.md),
/// which are designed to avoid nonsense expressions and ambiguity. Lastly,
/// compilation errors occur **only if the size of the compiled program is too
/// large** (all other compilation errors are considered internal bugs and will
/// panic).
///
/// When the `miette` feature is enabled, this and other error types implement
/// the [`Diagnostic`] trait. Due to a technical limitation, this may not be
/// properly annotated in API documentation.
///
/// [`Diagnostic`]: miette::Diagnostic
/// [`Pattern`]: crate::Pattern
#[cfg_attr(feature = "miette", derive(Diagnostic))]
#[cfg_attr(feature = "miette", diagnostic(transparent))]
#[derive(Debug, Error)]
#[error(transparent)]
pub struct BuildError {
    kind: BuildErrorKind,
}

impl BuildError {
    /// Gets [`LocatedError`]s detailing the errors within a glob expression.
    ///
    /// This function returns an [`Iterator`] over the [`LocatedError`]s that
    /// detail where and why an error occurred when the error has associated
    /// [`Span`]s within a glob expression. For errors with no such associated
    /// information, the [`Iterator`] yields no items, such as compilation
    /// errors.
    ///
    /// # Examples
    ///
    /// [`LocatedError`]s can be used to provide information to users about
    /// which parts of a glob expression are associated with an error.
    ///
    /// ```rust
    /// use wax::Glob;
    ///
    /// // This glob expression violates rules. The error handling code prints details
    /// // about the alternative where the violation occurred.
    /// let expression = "**/{foo,**/bar,baz}";
    /// match Glob::new(expression) {
    ///     Ok(glob) => {
    ///         // ...
    ///     },
    ///     Err(error) => {
    ///         eprintln!("{}", error);
    ///         for error in error.locations() {
    ///             let (start, n) = error.span();
    ///             let fragment = &expression[start..][..n];
    ///             eprintln!("in sub-expression `{}`: {}", fragment, error);
    ///         }
    ///     },
    /// }
    /// ```
    ///
    /// [`Glob`]: crate::Glob
    /// [`Glob::partition`]: crate::Glob::partition
    /// [`Iterator`]: std::iter::Iterator
    /// [`LocatedError`]: crate::LocatedError
    /// [`Span`]: crate::Span
    pub fn locations(&self) -> impl Iterator<Item = &dyn LocatedError> {
        let locations: Vec<_> = match self.kind {
            BuildErrorKind::Parse(ref error) => error
                .locations()
                .iter()
                .map(|location| location as &dyn LocatedError)
                .collect(),
            BuildErrorKind::Rule(ref error) => error
                .locations()
                .iter()
                .map(|location| location as &dyn LocatedError)
                .collect(),
            _ => vec![],
        };
        locations.into_iter()
    }
}

impl From<BuildErrorKind> for BuildError {
    fn from(kind: BuildErrorKind) -> Self {
        BuildError { kind }
    }
}

impl From<CompileError> for BuildError {
    fn from(error: CompileError) -> Self {
        BuildError {
            kind: BuildErrorKind::Compile(error),
        }
    }
}

impl From<Infallible> for BuildError {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}

impl<'t> From<ParseError<'t>> for BuildError {
    fn from(error: ParseError<'t>) -> Self {
        BuildError {
            kind: BuildErrorKind::Parse(error.into_owned()),
        }
    }
}

impl<'t> From<RuleError<'t>> for BuildError {
    fn from(error: RuleError<'t>) -> Self {
        BuildError {
            kind: BuildErrorKind::Rule(error.into_owned()),
        }
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
#[cfg_attr(feature = "miette", derive(Diagnostic))]
enum BuildErrorKind {
    #[error(transparent)]
    #[cfg_attr(feature = "miette", diagnostic(transparent))]
    Compile(CompileError),
    #[error(transparent)]
    #[cfg_attr(feature = "miette", diagnostic(transparent))]
    Parse(ParseError<'static>),
    #[error(transparent)]
    #[cfg_attr(feature = "miette", diagnostic(transparent))]
    Rule(RuleError<'static>),
}

/// Path that can be matched against a [`Pattern`].
///
/// `CandidatePath`s are always UTF-8 encoded. On some platforms this requires a
/// lossy conversion that uses Unicode replacement codepoints `�` whenever a
/// part of a path cannot be represented as valid UTF-8 (such as Windows). This
/// means that some byte sequences cannot be matched, though this is uncommon in
/// practice.
///
/// [`Pattern`]: crate::Pattern
#[derive(Clone)]
pub struct CandidatePath<'b> {
    text: Cow<'b, str>,
}

impl<'b> CandidatePath<'b> {
    /// Clones any borrowed data into an owning instance.
    pub fn into_owned(self) -> CandidatePath<'static> {
        CandidatePath {
            text: self.text.into_owned().into(),
        }
    }
}

impl AsRef<str> for CandidatePath<'_> {
    fn as_ref(&self) -> &str {
        self.text.as_ref()
    }
}

impl Debug for CandidatePath<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.text)
    }
}

impl Display for CandidatePath<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.text)
    }
}

impl<'b> From<&'b OsStr> for CandidatePath<'b> {
    fn from(text: &'b OsStr) -> Self {
        CandidatePath {
            text: text.to_string_lossy(),
        }
    }
}

impl<'b> From<&'b Path> for CandidatePath<'b> {
    fn from(path: &'b Path) -> Self {
        CandidatePath::from(path.as_os_str())
    }
}

impl<'b> From<&'b str> for CandidatePath<'b> {
    fn from(text: &'b str) -> Self {
        CandidatePath { text: text.into() }
    }
}

/// Pattern that can be matched against paths and directory trees.
///
/// `Glob`s are constructed from strings called glob expressions that resemble
/// Unix paths consisting of nominal components delimited by separators. Glob
/// expressions support various patterns that match and capture specified text
/// in a path. These patterns can be used to logically match individual paths
/// and to semantically match and walk directory trees.
///
/// # Examples
///
/// A `Glob` can be used to determine if a path matches a pattern via the
/// [`Pattern`] trait.
///
/// ```rust
/// use wax::{Glob, Pattern};
///
/// let glob = Glob::new("*.png").unwrap();
/// assert!(glob.is_match("apple.png"));
/// ```
///
/// Patterns form captures, which can be used to isolate matching sub-text.
///
/// ```rust
/// use wax::{CandidatePath, Glob, Pattern};
///
/// let glob = Glob::new("**/{*.{go,rs}}").unwrap();
/// let candidate = CandidatePath::from("src/lib.rs");
/// assert_eq!("lib.rs", glob.matched(&candidate).unwrap().get(2).unwrap());
/// ```
///
/// To match a `Glob` against a directory tree, the [`walk`] function can be
/// used to get an iterator over matching paths.
///
/// ```rust,no_run,ignore
/// use wax::Glob;
///
/// let glob = Glob::new("**/*.(?i){jpg,jpeg}").unwrap();
/// for entry in glob.walk("./Pictures") {
///     let entry = entry.unwrap();
///     println!("JPEG: {:?}", entry.path());
/// }
/// ```
///
/// [`Pattern`]: crate::Pattern
/// [`walk`]: crate::Glob::walk
#[derive(Clone, Debug)]
pub struct Glob<'t> {
    tree: Checked<Tokenized<'t>>,
    pattern: Regex,
}

impl<'t> Glob<'t> {
    fn compile<T>(tokens: impl IntoIterator<Item = T>) -> Result<Regex, CompileError>
    where
        T: Borrow<Token<'t>>,
    {
        encode::compile(tokens)
    }

    // TODO: Document pattern syntax in the crate documentation and refer to it
    //       here.
    /// Constructs a [`Glob`] from a glob expression.
    ///
    /// A glob expression is UTF-8 encoded text that resembles a Unix path
    /// consisting of nominal components delimited by separators and patterns
    /// that can be matched against native paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the glob expression fails to build. See
    /// [`BuildError`].
    ///
    /// [`Glob`]: crate::Glob
    /// [`BuildError`]: crate::BuildError
    pub fn new(expression: &'t str) -> Result<Self, BuildError> {
        let tree = parse_and_check(expression)?;
        let pattern = Glob::compile(tree.as_ref().tokens())?;
        Ok(Glob { tree, pattern })
    }

    /// Constructs a [`Glob`] from a glob expression with diagnostics.
    ///
    /// This function is the same as [`Glob::new`], but additionally returns
    /// detailed diagnostics on both success and failure.
    ///
    /// See [`Glob::diagnose`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tardar::DiagnosticResultExt as _;
    /// use wax::Glob;
    ///
    /// let result = Glob::diagnosed("(?i)readme.{md,mkd,markdown}");
    /// for diagnostic in result.diagnostics() {
    ///     eprintln!("{}", diagnostic);
    /// }
    /// if let Some(glob) = result.ok_output() { /* ... */ }
    /// ```
    ///
    /// [`Glob`]: crate::Glob
    /// [`Glob::diagnose`]: crate::Glob::diagnose
    /// [`Glob::new`]: crate::Glob::new
    #[cfg(feature = "miette")]
    #[cfg_attr(docsrs, doc(cfg(feature = "miette")))]
    pub fn diagnosed(expression: &'t str) -> DiagnosticResult<'t, Self> {
        parse_and_diagnose(expression).and_then_diagnose(|tree| {
            Glob::compile(tree.as_ref().tokens())
                .into_error_diagnostic()
                .map_output(|pattern| Glob { tree, pattern })
        })
    }

    /// Partitions a [`Glob`] into an invariant [`PathBuf`] prefix and variant
    /// [`Glob`] postfix.
    ///
    /// The invariant prefix contains no glob patterns nor other variant
    /// components and therefore can be interpreted as a native path. The
    /// [`Glob`] postfix is variant and contains the remaining components that
    /// follow the prefix. For example, the glob expression `.local/**/*.log`
    /// would produce the path `.local` and glob `**/*.log`. It is possible for
    /// either partition to be empty.
    ///
    /// Literal components may be considered variant if they contain characters
    /// with casing and the configured case sensitivity differs from the target
    /// platform's file system. For example, the case-insensitive literal
    /// expression `(?i)photos` is considered variant on Unix and invariant on
    /// Windows, because the literal `photos` resolves differently in Unix file
    /// system APIs.
    ///
    /// Partitioning a [`Glob`] allows any invariant prefix to be used as a
    /// native path to establish a working directory or to interpret semantic
    /// components that are not recognized by globs, such as parent directory
    /// `..` components.
    ///
    /// Partitioned [`Glob`]s are never rooted. If the glob expression has a
    /// root component, then it is always included in the invariant [`PathBuf`]
    /// prefix.
    ///
    /// # Examples
    ///
    /// To match paths against a [`Glob`] while respecting semantic components,
    /// the invariant prefix and candidate path can be canonicalized. The
    /// following example canonicalizes both the working directory joined with
    /// the prefix as well as the candidate path and then attempts to match the
    /// [`Glob`] if the candidate path contains the prefix.
    ///
    /// ```rust,no_run
    /// use dunce; // Avoids UNC paths on Windows.
    /// use std::path::Path;
    /// use wax::{Glob, Pattern};
    ///
    /// let path: &Path = /* ... */ // Candidate path.
    /// # Path::new("");
    ///     
    /// let directory = Path::new("."); // Working directory.
    /// let (prefix, glob) = Glob::new("../../src/**").unwrap().partition();
    /// let prefix = dunce::canonicalize(directory.join(&prefix)).unwrap();
    /// if dunce::canonicalize(path)
    ///     .unwrap()
    ///     .strip_prefix(&prefix)
    ///     .map(|path| glob.is_match(path))
    ///     .unwrap_or(false)
    /// {
    ///     // ...
    /// }
    /// ```
    ///
    /// [`Glob`]: crate::Glob
    /// [`ParseError`]: crate::ParseError
    /// [`PathBuf`]: std::path::PathBuf
    /// [`RuleError`]: crate::RuleError
    /// [`walk`]: crate::Glob::walk
    pub fn partition(self) -> (PathBuf, Self) {
        let Glob { tree, .. } = self;
        let (prefix, tree) = tree.partition();
        let pattern =
            Glob::compile(tree.as_ref().tokens()).expect("failed to compile partitioned glob");
        (prefix, Glob { tree, pattern })
    }

    /// Clones any borrowed data into an owning instance.
    ///
    /// # Examples
    ///
    /// `Glob`s borrow data in the corresponding glob expression. To move a
    /// `Glob` beyond the scope of a glob expression, clone the data with this
    /// function.
    ///
    /// ```rust
    /// use wax::{BuildError, Glob};
    ///
    /// fn local() -> Result<Glob<'static>, BuildError> {
    ///     let expression = String::from("**/*.txt");
    ///     Glob::new(&expression).map(Glob::into_owned)
    /// }
    /// ```
    pub fn into_owned(self) -> Glob<'static> {
        let Glob { tree, pattern } = self;
        Glob {
            tree: tree.into_owned(),
            pattern,
        }
    }

    /// Gets an iterator over matching files in a directory tree.
    ///
    /// This function matches a [`Glob`] against a directory tree, returning
    /// each matching file as a [`WalkEntry`]. [`Glob`]s are the only patterns
    /// that support this semantic operation; it is not possible to match
    /// combinators over directory trees.
    ///
    /// As with [`Path::join`] and [`PathBuf::push`], the base directory can be
    /// escaped or overridden by rooted [`Glob`]s. In many cases, the current
    /// working directory `.` is an appropriate base directory and will be
    /// intuitively ignored if the [`Glob`] is rooted, such as in
    /// `/mnt/media/**/*.mp4`. The [`has_root`] function can be used to check if
    /// a [`Glob`] is rooted and the [`Walk::root`] function can be used to get
    /// the resulting root directory of the traversal.
    ///
    /// The [root directory][`Walk::root`] is established via the [invariant
    /// prefix][`Glob::partition`] of the [`Glob`]. **The prefix and any
    /// [semantic literals][`Glob::has_semantic_literals`] in this prefix are
    /// interpreted semantically as a path**, so components like `.` and `..`
    /// that precede variant patterns interact with the base directory
    /// semantically. This means that expressions like `../**` escape the base
    /// directory as expected on Unix and Windows, for example.
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
    /// iterator adaptor can be used when walking a directory tree to filter
    /// [`WalkEntry`]s using arbitary patterns. **This should generally be
    /// preferred over functions like [`Iterator::filter`], because it avoids
    /// unnecessary reads of directory trees when matching [exhaustive
    /// negations][`Pattern::is_exhaustive`].**
    ///
    /// ```rust,no_run
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
    /// [`Glob`]: crate::Glob
    /// [`Glob::walk_with_behavior`]: crate::Glob::walk_with_behavior
    /// [`has_root`]: crate::Glob::has_root
    /// [`Iterator::filter`]: std::iter::Iterator::filter
    /// [`not`]: crate::Walk::not
    /// [`Path::join`]: std::path::Path::join
    /// [`PathBuf::push`]: std::path::PathBuf::push
    /// [`Pattern`]: crate::Pattern
    /// [`Pattern::is_exhaustive`]: crate::Pattern::is_exhaustive
    /// [`Walk::root`]: crate::Walk::root
    /// [`WalkBehavior`]: crate::WalkBehavior
    /// [`WalkEntry`]: crate::WalkEntry
    #[cfg(feature = "walk")]
    #[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
    pub fn walk(&self, directory: impl AsRef<Path>) -> Walk {
        self.walk_with_behavior(directory, WalkBehavior::default())
    }

    /// Gets an iterator over matching files in a directory tree.
    ///
    /// This function is the same as [`Glob::walk`], but it additionally accepts
    /// a [`WalkBehavior`]. This can be used to configure how the traversal
    /// interacts with symbolic links, the maximum depth from the root, etc.
    ///
    /// Depth is relative to the [root directory][`Walk::root`] of the
    /// traversal, which is determined by joining the given path and any
    /// [invariant prefix][`Glob::partition`] of the [`Glob`].
    ///
    /// See [`Glob::walk`] for more information.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use wax::{Glob, WalkBehavior};
    ///
    /// let glob = Glob::new("**/*.(?i){jpg,jpeg}").unwrap();
    /// for entry in glob.walk_with_behavior("./Pictures", WalkBehavior::default()) {
    ///     let entry = entry.unwrap();
    ///     println!("JPEG: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// By default, symbolic links are read as normal files and their targets
    /// are ignored. To follow symbolic links and traverse any directories that
    /// they reference, specify a [`LinkBehavior`].
    ///
    /// ```rust,no_run
    /// use wax::{Glob, LinkBehavior};
    ///
    /// let glob = Glob::new("**/*.txt").unwrap();
    /// for entry in glob.walk_with_behavior("/var/log", LinkBehavior::ReadTarget) {
    ///     let entry = entry.unwrap();
    ///     println!("Log: {:?}", entry.path());
    /// }
    /// ```
    ///
    /// [`Glob`]: crate::Glob
    /// [`Glob::partition`]: crate::Glob::partition
    /// [`Glob::walk`]: crate::Glob::walk
    /// [`LinkBehavior`]: crate::LinkBehavior
    /// [`Walk::root`]: crate::Walk::root
    /// [`WalkBehavior`]: crate::WalkBehavior
    #[cfg(feature = "walk")]
    #[cfg_attr(docsrs, doc(cfg(feature = "walk")))]
    pub fn walk_with_behavior(
        &self,
        directory: impl AsRef<Path>,
        behavior: impl Into<WalkBehavior>,
    ) -> Walk {
        walk::walk(self, directory, behavior)
    }

    /// Gets **non-error** [`Diagnostic`]s.
    ///
    /// This function requires a receiving [`Glob`] and so does not report
    /// error-level [`Diagnostic`]s. It can be used to get non-error diagnostics
    /// after constructing or [partitioning][`Glob::partition`] a [`Glob`].
    ///
    /// See [`Glob::diagnosed`].
    ///
    /// [`Diagnostic`]: miette::Diagnostic
    /// [`Glob`]: crate::Glob
    /// [`Glob::diagnosed`]: crate::Glob::diagnosed
    /// [`Glob::partition`]: crate::Glob::partition
    #[cfg(feature = "miette")]
    #[cfg_attr(docsrs, doc(cfg(feature = "miette")))]
    pub fn diagnose(&self) -> impl Iterator<Item = Box<dyn Diagnostic + '_>> {
        diagnostics::diagnose(self.tree.as_ref())
    }

    /// Gets metadata for capturing sub-expressions.
    ///
    /// This function returns an iterator over capturing tokens, which describe
    /// the index and location of sub-expressions that capture [matched
    /// text][`MatchedText`]. For example, in the expression `src/**/*.rs`, both
    /// `**` and `*` form
    /// captures.
    ///
    /// [`MatchedText`]: crate::MatchedText
    pub fn captures(&self) -> impl '_ + Clone + Iterator<Item = CapturingToken> {
        self.tree
            .as_ref()
            .tokens()
            .iter()
            .filter(|token| token.is_capturing())
            .enumerate()
            .map(|(index, token)| CapturingToken {
                index: index + 1,
                span: *token.annotation(),
            })
    }

    /// Returns `true` if the glob has a root.
    ///
    /// As with Unix paths, a glob expression has a root if it begins with a
    /// separator `/`. Patterns other than separators may also root an
    /// expression, such as `/**` or `</root:1,>`.
    pub fn has_root(&self) -> bool {
        self.tree
            .as_ref()
            .tokens()
            .first()
            .map_or(false, Token::has_root)
    }

    /// Returns `true` if the glob has literals that have non-nominal semantics
    /// on the target platform.
    ///
    /// The most notable semantic literals are the relative path components `.`
    /// and `..`, which refer to a current and parent directory on Unix and
    /// Windows operating systems, respectively. These are interpreted as
    /// literals in glob expressions, and so only logically match paths that
    /// contain these exact nominal components (semantic meaning is lost).
    ///
    /// See [`Glob::partition`].
    ///
    /// [`Glob::partition`]: crate::Glob::partition
    pub fn has_semantic_literals(&self) -> bool {
        token::literals(self.tree.as_ref().tokens())
            .any(|(_, literal)| literal.is_semantic_literal())
    }
}

impl Display for Glob<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}", self.tree.as_ref().expression())
    }
}

impl FromStr for Glob<'static> {
    type Err = BuildError;

    fn from_str(expression: &str) -> Result<Self, Self::Err> {
        Glob::new(expression).map(Glob::into_owned)
    }
}

impl<'t> Pattern<'t> for Glob<'t> {
    fn is_match<'p>(&self, path: impl Into<CandidatePath<'p>>) -> bool {
        let path = path.into();
        self.pattern.is_match(path.as_ref())
    }

    fn matched<'p>(&self, path: &'p CandidatePath<'_>) -> Option<MatchedText<'p>> {
        self.pattern.captures(path.as_ref()).map(From::from)
    }

    fn variance(&self) -> Variance {
        self.tree.as_ref().variance().into()
    }

    fn is_exhaustive(&self) -> bool {
        token::is_exhaustive(self.tree.as_ref().tokens())
    }
}

impl<'t> TryFrom<&'t str> for Glob<'t> {
    type Error = BuildError;

    fn try_from(expression: &'t str) -> Result<Self, Self::Error> {
        Glob::new(expression)
    }
}

impl<'t> Compose<'t> for Glob<'t> {
    type Tokens = Tokenized<'t>;
    type Error = Infallible;
}

/// Combinator that matches any of its component [`Pattern`]s.
///
/// An instance of `Any` is constructed using the [`any`] function, which
/// composes multiple [`Pattern`]s for more ergonomic and efficient matching.
///
/// [`any`]: crate::any
/// [`Pattern`]: crate::Pattern
#[derive(Clone, Debug)]
pub struct Any<'t> {
    tree: Checked<Token<'t, ()>>,
    pattern: Regex,
}

impl<'t> Any<'t> {
    fn compile(token: &Token<'t, ()>) -> Result<Regex, CompileError> {
        encode::compile([token])
    }
}

impl<'t> Pattern<'t> for Any<'t> {
    fn is_match<'p>(&self, path: impl Into<CandidatePath<'p>>) -> bool {
        let path = path.into();
        self.pattern.is_match(path.as_ref())
    }

    fn matched<'p>(&self, path: &'p CandidatePath<'_>) -> Option<MatchedText<'p>> {
        self.pattern.captures(path.as_ref()).map(From::from)
    }

    fn variance(&self) -> Variance {
        self.tree.as_ref().variance::<InvariantText>().into()
    }

    fn is_exhaustive(&self) -> bool {
        token::is_exhaustive(Some(self.tree.as_ref()))
    }
}

impl<'t> Compose<'t> for Any<'t> {
    type Tokens = Token<'t, ()>;
    type Error = Infallible;
}

// TODO: It may be useful to use dynamic dispatch via trait objects instead.
//       This would allow for a variety of types to be composed in an `any` call
//       and would be especially useful if additional combinators are
//       introduced.
/// Composes glob expressions into a combinator that matches if any of its input
/// [`Pattern`]s match.
///
/// This function accepts an [`IntoIterator`] with items that implement the
/// [`Compose`] trait such as [`Glob`] and `&str`. The output [`Any`] implements
/// [`Pattern`] by matching any of its component [`Pattern`]s. [`Any`] is often
/// more ergonomic and efficient than matching individually against multiple
/// [`Pattern`]s.
///
/// [`Any`] groups all captures and therefore only exposes the complete text of
/// a match. It is not possible to index a particular capturing token in the
/// component patterns. [`Any`] only supports logical matching and cannot be
/// used to semantically match a directory tree.
///
/// # Examples
///
/// To match a path against multiple patterns, the patterns can first be
/// composed into an [`Any`].
///
/// ```rust
/// use wax::{Glob, Pattern};
///
/// let any = wax::any([
///     "src/**/*.rs",
///     "tests/**/*.rs",
///     "doc/**/*.md",
///     "pkg/**/PKGBUILD",
/// ])
/// .unwrap();
/// assert!(any.is_match("src/lib.rs"));
/// ```
///
/// [`Glob`]s and other compiled [`Pattern`]s can also be composed into an
/// [`Any`].
///
/// ```rust
/// use wax::{Glob, Pattern};
///
/// let red = Glob::new("**/red/**/*.txt").unwrap();
/// let blue = Glob::new("**/*blue*.txt").unwrap();
/// assert!(wax::any([red, blue]).unwrap().is_match("red/potion.txt"));
/// ```
///
/// # Errors
///
/// Returns an error if any of the inputs fail to build. If the inputs are a
/// compiled [`Pattern`] type such as [`Glob`], then this only occurs if the
/// compiled program is too large.
///
/// [`Any`]: crate::Any
/// [`Glob`]: crate::Glob
/// [`IntoIterator`]: std::iter::IntoIterator
/// [`Pattern`]: crate::Pattern
pub fn any<'t, I>(patterns: I) -> Result<Any<'t>, BuildError>
where
    I: IntoIterator,
    I::Item: Compose<'t>,
{
    let tree = Checked::any(
        patterns
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)?,
    );
    let pattern = Any::compile(tree.as_ref())?;
    Ok(Any { tree, pattern })
}

/// Escapes text as a literal glob expression.
///
/// This function escapes any and all meta-characters in the given string, such
/// that all text is interpreted as a literal or separator when read as a glob
/// expression.
///
/// # Examples
///
/// This function can be used to escape opaque strings, such as a string
/// obtained from a user that must be interpreted literally.
///
/// ```rust
/// use wax::Glob;
///
/// // An opaque file name that this code does not construct.
/// let name: String = {
///     /* ... */
///     # String::from("file.txt")
/// };
///
/// // Do not allow patterns in `name`.
/// let expression = format!("{}{}", "**/", wax::escape(&name));
/// if let glob = Glob::new(&expression) { /* ... */ }
/// ```
///
/// Sometimes part of a path contains numerous meta-characters. This function
/// can be used to reliably escape them while making the unescaped part of the
/// expression a bit easier to read.
///
/// ```rust
/// use wax::Glob;
///
/// let expression = format!("{}{}", "logs/**/", wax::escape("ingest[01](L).txt"));
/// let glob = Glob::new(&expression).unwrap();
/// ```
// It is possible to call this function using a mutable reference, which may
// appear to mutate the parameter in place.
#[must_use]
pub fn escape(unescaped: &str) -> Cow<str> {
    const ESCAPE: char = '\\';

    if unescaped.chars().any(is_meta_character) {
        let mut escaped = String::new();
        for x in unescaped.chars() {
            if is_meta_character(x) {
                escaped.push(ESCAPE);
            }
            escaped.push(x);
        }
        escaped.into()
    } else {
        unescaped.into()
    }
}

// TODO: Is it possible for `:` and `,` to be contextual meta-characters?
/// Returns `true` if the given character is a meta-character.
///
/// This function does **not** return `true` for contextual meta-characters that
/// may only be escaped in particular contexts, such as hyphens `-` in character
/// class expressions. To detect these characters, use
/// [`is_contextual_meta_character`].
///
/// [`is_contextual_meta_character`]: crate::is_contextual_meta_character
pub const fn is_meta_character(x: char) -> bool {
    matches!(
        x,
        '?' | '*' | '$' | ':' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ','
    )
}

/// Returns `true` if the given character is a contextual meta-character.
///
/// Contextual meta-characters may only be escaped in particular contexts, such
/// as hyphens `-` in character class expressions. Elsewhere, they are
/// interpreted as literals. To detect non-contextual meta-characters, use
/// [`is_meta_character`].
///
/// [`is_meta_character`]: crate::is_meta_character
pub const fn is_contextual_meta_character(x: char) -> bool {
    matches!(x, '-')
}

fn parse_and_check(expression: &str) -> Result<Checked<Tokenized>, BuildError> {
    let tokenized = token::parse(expression)?;
    let checked = rule::check(tokenized)?;
    Ok(checked)
}

#[cfg(feature = "miette")]
fn parse_and_diagnose(expression: &str) -> DiagnosticResult<Checked<Tokenized>> {
    token::parse(expression)
        .into_error_diagnostic()
        .and_then_diagnose(|tokenized| rule::check(tokenized).into_error_diagnostic())
        .and_then_diagnose(|checked| {
            // TODO: This should accept `&Checked`.
            diagnostics::diagnose(checked.as_ref())
                .into_non_error_diagnostic()
                .map_output(|_| checked)
        })
}

// TODO: Construct paths from components in tests. In practice, using string
//       literals works, but is technically specific to platforms that support
//       `/` as a separator.
#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{BuildError, BuildErrorKind, CandidatePath, Glob, Pattern};

    #[test]
    fn escape() {
        assert_eq!(crate::escape(""), "");
        assert_eq!(
            crate::escape("?*$:<>()[]{},"),
            "\\?\\*\\$\\:\\<\\>\\(\\)\\[\\]\\{\\}\\,",
        );
        assert_eq!(crate::escape("/usr/local/lib"), "/usr/local/lib");
        assert_eq!(
            crate::escape("record[D00,00].txt"),
            "record\\[D00\\,00\\].txt",
        );
        assert_eq!(
            crate::escape("Do You Remember Love?.mp4"),
            "Do You Remember Love\\?.mp4",
        );
        assert_eq!(crate::escape("左{}右"), "左\\{\\}右");
        assert_eq!(crate::escape("*中*"), "\\*中\\*");
    }

    #[test]
    fn build_glob_with_eager_zom_tokens() {
        Glob::new("*").unwrap();
        Glob::new("a/*").unwrap();
        Glob::new("*a").unwrap();
        Glob::new("a*").unwrap();
        Glob::new("a*b").unwrap();
        Glob::new("/*").unwrap();
    }

    #[test]
    fn build_glob_with_lazy_zom_tokens() {
        Glob::new("$").unwrap();
        Glob::new("a/$").unwrap();
        Glob::new("$a").unwrap();
        Glob::new("a$").unwrap();
        Glob::new("a$b").unwrap();
        Glob::new("/$").unwrap();
    }

    #[test]
    fn build_glob_with_one_tokens() {
        Glob::new("?").unwrap();
        Glob::new("a/?").unwrap();
        Glob::new("?a").unwrap();
        Glob::new("a?").unwrap();
        Glob::new("a?b").unwrap();
        Glob::new("??a??b??").unwrap();
        Glob::new("/?").unwrap();
    }

    #[test]
    fn build_glob_with_one_and_zom_tokens() {
        Glob::new("?*").unwrap();
        Glob::new("*?").unwrap();
        Glob::new("*/?").unwrap();
        Glob::new("?*?").unwrap();
        Glob::new("/?*").unwrap();
        Glob::new("?$").unwrap();
    }

    #[test]
    fn build_glob_with_tree_tokens() {
        Glob::new("**").unwrap();
        Glob::new("**/").unwrap();
        Glob::new("/**").unwrap();
        Glob::new("**/a").unwrap();
        Glob::new("a/**").unwrap();
        Glob::new("**/a/**/b/**").unwrap();
        Glob::new("{**/a,b/c}").unwrap();
        Glob::new("{a/b,c/**}").unwrap();
        Glob::new("<**/a>").unwrap();
        Glob::new("<a/**>").unwrap();
    }

    #[test]
    fn build_glob_with_class_tokens() {
        Glob::new("a/[xy]").unwrap();
        Glob::new("a/[x-z]").unwrap();
        Glob::new("a/[xyi-k]").unwrap();
        Glob::new("a/[i-kxy]").unwrap();
        Glob::new("a/[!xy]").unwrap();
        Glob::new("a/[!x-z]").unwrap();
        Glob::new("a/[^xy]").unwrap();
        Glob::new("a/[^x-z]").unwrap();
        Glob::new("a/[xy]b/c").unwrap();
    }

    #[test]
    fn negative_match_does_not_traverse_folders() {
        let glob = Glob::new("a[!b]c").unwrap();
        assert!(glob.is_match(Path::new("adc")));
        assert!(!glob.is_match(Path::new("a/c")));
    }

    #[test]
    fn negative_match_does_not_traverse_folders_2() {
        let glob = Glob::new("a[!b-z]c").unwrap();
        assert!(glob.is_match(Path::new("aac")));
        assert!(!glob.is_match(Path::new("a/c")));
    }

    #[test]
    fn build_glob_with_alternative_tokens() {
        Glob::new("a/{x?z,y$}b*").unwrap();
        Glob::new("a/{???,x$y,frob}b*").unwrap();
        Glob::new("a/{???,x$y,frob}b*").unwrap();
        Glob::new("a/{???,{x*z,y$}}b*").unwrap();
        Glob::new("a{/**/b/,/b/**/}ca{t,b/**}").unwrap();
    }

    #[test]
    fn build_glob_with_repetition_tokens() {
        Glob::new("<a:0,1>").unwrap();
        Glob::new("<a:0,>").unwrap();
        Glob::new("<a:2>").unwrap();
        Glob::new("<a:>").unwrap();
        Glob::new("<a>").unwrap();
        Glob::new("<a<b:0,>:0,>").unwrap();
        // Rooted repetitions are accepted if the lower bound is one or greater.
        Glob::new("</root:1,>").unwrap();
        Glob::new("<[!.]*/:0,>[!.]*").unwrap();
    }

    #[test]
    fn build_glob_with_literal_escaped_wildcard_tokens() {
        Glob::new("a/b\\?/c").unwrap();
        Glob::new("a/b\\$/c").unwrap();
        Glob::new("a/b\\*/c").unwrap();
        Glob::new("a/b\\*\\*/c").unwrap();
    }

    #[test]
    fn build_glob_with_class_escaped_wildcard_tokens() {
        Glob::new("a/b[?]/c").unwrap();
        Glob::new("a/b[$]/c").unwrap();
        Glob::new("a/b[*]/c").unwrap();
        Glob::new("a/b[*][*]/c").unwrap();
    }

    #[test]
    fn build_glob_with_literal_escaped_alternative_tokens() {
        Glob::new("a/\\{\\}/c").unwrap();
        Glob::new("a/{x,y\\,,z}/c").unwrap();
    }

    #[test]
    fn build_glob_with_class_escaped_alternative_tokens() {
        Glob::new("a/[{][}]/c").unwrap();
        Glob::new("a/{x,y[,],z}/c").unwrap();
    }

    #[test]
    fn build_glob_with_literal_escaped_class_tokens() {
        Glob::new("a/\\[a-z\\]/c").unwrap();
        Glob::new("a/[\\[]/c").unwrap();
        Glob::new("a/[\\]]/c").unwrap();
        Glob::new("a/[a\\-z]/c").unwrap();
    }

    #[test]
    fn build_glob_with_flags() {
        Glob::new("(?i)a/b/c").unwrap();
        Glob::new("(?-i)a/b/c").unwrap();
        Glob::new("a/(?-i)b/c").unwrap();
        Glob::new("a/b/(?-i)c").unwrap();
        Glob::new("(?i)a/(?-i)b/(?i)c").unwrap();
    }

    #[test]
    fn build_any_combinator() {
        crate::any([
            Glob::new("src/**/*.rs").unwrap(),
            Glob::new("doc/**/*.md").unwrap(),
            Glob::new("pkg/**/PKGBUILD").unwrap(),
        ])
        .unwrap();
        crate::any(["src/**/*.rs", "doc/**/*.md", "pkg/**/PKGBUILD"]).unwrap();
    }

    #[test]
    fn build_any_nested_combinator() {
        crate::any([
            crate::any(["a/b", "c/d"]).unwrap(),
            crate::any(["{e,f,g}", "{h,i}"]).unwrap(),
        ])
        .unwrap();
    }

    #[test]
    fn reject_glob_with_invalid_separator_tokens() {
        assert!(Glob::new("//a").is_err());
        assert!(Glob::new("a//b").is_err());
        assert!(Glob::new("a/b//").is_err());
        assert!(Glob::new("a//**").is_err());
        assert!(Glob::new("{//}a").is_err());
        assert!(Glob::new("{**//}").is_err());
    }

    #[test]
    fn reject_glob_with_adjacent_tree_or_zom_tokens() {
        assert!(Glob::new("***").is_err());
        assert!(Glob::new("****").is_err());
        assert!(Glob::new("**/**").is_err());
        assert!(Glob::new("a{**/**,/b}").is_err());
        assert!(Glob::new("**/*/***").is_err());
        assert!(Glob::new("**$").is_err());
        assert!(Glob::new("**/$**").is_err());
        assert!(Glob::new("{*$}").is_err());
        assert!(Glob::new("<*$:1,>").is_err());
    }

    #[test]
    fn reject_glob_with_tree_adjacent_literal_tokens() {
        assert!(Glob::new("**a").is_err());
        assert!(Glob::new("a**").is_err());
        assert!(Glob::new("a**b").is_err());
        assert!(Glob::new("a*b**").is_err());
        assert!(Glob::new("**/**a/**").is_err());
    }

    #[test]
    fn reject_glob_with_adjacent_one_tokens() {
        assert!(Glob::new("**?").is_err());
        assert!(Glob::new("?**").is_err());
        assert!(Glob::new("?**?").is_err());
        assert!(Glob::new("?*?**").is_err());
        assert!(Glob::new("**/**?/**").is_err());
    }

    #[test]
    fn reject_glob_with_unescaped_meta_characters_in_class_tokens() {
        assert!(Glob::new("a/[a-z-]/c").is_err());
        assert!(Glob::new("a/[-a-z]/c").is_err());
        assert!(Glob::new("a/[-]/c").is_err());
        // NOTE: Without special attention to escaping and character parsing,
        //       this could be mistakenly interpreted as an empty range over the
        //       character `-`. This should be rejected.
        assert!(Glob::new("a/[---]/c").is_err());
        assert!(Glob::new("a/[[]/c").is_err());
        assert!(Glob::new("a/[]]/c").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_alternative_zom_tokens() {
        assert!(Glob::new("*{okay,*}").is_err());
        assert!(Glob::new("{okay,*}*").is_err());
        assert!(Glob::new("${okay,*error}").is_err());
        assert!(Glob::new("{okay,error*}$").is_err());
        assert!(Glob::new("{*,okay}{okay,*}").is_err());
        assert!(Glob::new("{okay,error*}{okay,*error}").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_alternative_tree_tokens() {
        assert!(Glob::new("{**}").is_err());
        assert!(Glob::new("slash/{**/error}").is_err());
        assert!(Glob::new("{error/**}/slash").is_err());
        assert!(Glob::new("slash/{okay/**,**/error}").is_err());
        assert!(Glob::new("{**/okay,error/**}/slash").is_err());
        assert!(Glob::new("{**/okay,prefix{error/**}}/slash").is_err());
        assert!(Glob::new("{**/okay,slash/{**/error}}postfix").is_err());
        assert!(Glob::new("{error/**}{okay,**/error").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_alternative_separator_tokens() {
        assert!(Glob::new("/slash/{okay,/error}").is_err());
        assert!(Glob::new("{okay,error/}/slash").is_err());
        assert!(Glob::new("slash/{okay,/error/,okay}/slash").is_err());
        assert!(Glob::new("{okay,error/}{okay,/error}").is_err());
    }

    #[test]
    fn reject_glob_with_rooted_alternative_tokens() {
        assert!(Glob::new("{okay,/}").is_err());
        assert!(Glob::new("{okay,/**}").is_err());
        assert!(Glob::new("{okay,/error}").is_err());
        assert!(Glob::new("{okay,/**/error}").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_repetition_bounds_tokens() {
        assert!(Glob::new("<a/:0,0>").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_repetition_zom_tokens() {
        assert!(Glob::new("<*:0,>").is_err());
        assert!(Glob::new("<a/*:0,>*").is_err());
        assert!(Glob::new("*<*a:0,>").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_repetition_tree_tokens() {
        assert!(Glob::new("<**:0,>").is_err());
        assert!(Glob::new("</**/a/**:0,>").is_err());
        assert!(Glob::new("<a/**:0,>/").is_err());
        assert!(Glob::new("/**</a:0,>").is_err());
    }

    #[test]
    fn reject_glob_with_invalid_repetition_separator_tokens() {
        assert!(Glob::new("</:0,>").is_err());
        assert!(Glob::new("</a/:0,>").is_err());
        assert!(Glob::new("<a/:0,>/").is_err());
    }

    // Rooted repetitions are rejected if their lower bound is zero; any other
    // lower bound is accepted.
    #[test]
    fn reject_glob_with_rooted_repetition_tokens() {
        assert!(Glob::new("</root:0,>maybe").is_err());
        assert!(Glob::new("</root>").is_err());
    }

    #[test]
    fn reject_glob_with_oversized_invariant_repetition_tokens() {
        assert!(matches!(
            Glob::new("<a:65536>"),
            Err(BuildError {
                kind: BuildErrorKind::Rule(_),
                ..
            }),
        ));
        assert!(matches!(
            Glob::new("<long:16500>"),
            Err(BuildError {
                kind: BuildErrorKind::Rule(_),
                ..
            }),
        ));
        assert!(matches!(
            Glob::new("a<long:16500>b"),
            Err(BuildError {
                kind: BuildErrorKind::Rule(_),
                ..
            }),
        ));
        assert!(matches!(
            Glob::new("{<a:65536>,<long:16500>}"),
            Err(BuildError {
                kind: BuildErrorKind::Rule(_),
                ..
            }),
        ));
    }

    #[test]
    fn reject_glob_with_invalid_flags() {
        assert!(Glob::new("(?)a").is_err());
        assert!(Glob::new("(?-)a").is_err());
        assert!(Glob::new("()a").is_err());
    }

    #[test]
    fn reject_glob_with_adjacent_tokens_through_flags() {
        assert!(Glob::new("/(?i)/").is_err());
        assert!(Glob::new("$(?i)$").is_err());
        assert!(Glob::new("*(?i)*").is_err());
        assert!(Glob::new("**(?i)?").is_err());
        assert!(Glob::new("a(?i)**").is_err());
        assert!(Glob::new("**(?i)a").is_err());
    }

    #[test]
    fn reject_glob_with_oversized_program() {
        assert!(matches!(
            Glob::new("<a*:1000000>"),
            Err(BuildError {
                kind: BuildErrorKind::Compile(_),
                ..
            }),
        ));
    }

    #[test]
    fn reject_any_combinator() {
        assert!(crate::any(["{a,b,c}", "{d, e}", "f/{g,/error,h}",]).is_err())
    }

    #[test]
    fn match_glob_with_empty_expression() {
        let glob = Glob::new("").unwrap();

        assert!(glob.is_match(Path::new("")));

        assert!(!glob.is_match(Path::new("abc")));
    }

    #[test]
    fn match_glob_with_only_invariant_tokens() {
        let glob = Glob::new("a/b").unwrap();

        assert!(glob.is_match(Path::new("a/b")));

        assert!(!glob.is_match(Path::new("aa/b")));
        assert!(!glob.is_match(Path::new("a/bb")));
        assert!(!glob.is_match(Path::new("a/b/c")));

        // There are no variant tokens with which to capture, but the matched
        // text should always be available.
        assert_eq!(
            "a/b",
            glob.matched(&CandidatePath::from(Path::new("a/b")))
                .unwrap()
                .complete(),
        );
    }

    #[test]
    fn match_glob_with_tree_tokens() {
        let glob = Glob::new("a/**/b").unwrap();

        assert!(glob.is_match(Path::new("a/b")));
        assert!(glob.is_match(Path::new("a/x/b")));
        assert!(glob.is_match(Path::new("a/x/y/z/b")));

        assert!(!glob.is_match(Path::new("a")));
        assert!(!glob.is_match(Path::new("b/a")));

        assert_eq!(
            "x/y/z/",
            glob.matched(&CandidatePath::from(Path::new("a/x/y/z/b")))
                .unwrap()
                .get(1)
                .unwrap(),
        );
    }

    #[test]
    fn match_glob_with_tree_and_zom_tokens() {
        let glob = Glob::new("**/*.ext").unwrap();

        assert!(glob.is_match(Path::new("file.ext")));
        assert!(glob.is_match(Path::new("a/file.ext")));
        assert!(glob.is_match(Path::new("a/b/file.ext")));

        let path = CandidatePath::from(Path::new("a/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("a/", matched.get(1).unwrap());
        assert_eq!("file", matched.get(2).unwrap());
    }

    #[test]
    fn match_glob_with_eager_and_lazy_zom_tokens() {
        let glob = Glob::new("$-*.*").unwrap();

        assert!(glob.is_match(Path::new("prefix-file.ext")));
        assert!(glob.is_match(Path::new("a-b-c.ext")));

        let path = CandidatePath::from(Path::new("a-b-c.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("a", matched.get(1).unwrap());
        assert_eq!("b-c", matched.get(2).unwrap());
        assert_eq!("ext", matched.get(3).unwrap());
    }

    #[test]
    fn match_glob_with_class_tokens() {
        let glob = Glob::new("a/[xyi-k]/**").unwrap();

        assert!(glob.is_match(Path::new("a/x/file.ext")));
        assert!(glob.is_match(Path::new("a/y/file.ext")));
        assert!(glob.is_match(Path::new("a/j/file.ext")));

        assert!(!glob.is_match(Path::new("a/b/file.ext")));

        let path = CandidatePath::from(Path::new("a/i/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("i", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_non_ascii_class_tokens() {
        let glob = Glob::new("a/[金銀]/**").unwrap();

        assert!(glob.is_match(Path::new("a/金/file.ext")));
        assert!(glob.is_match(Path::new("a/銀/file.ext")));

        assert!(!glob.is_match(Path::new("a/銅/file.ext")));

        let path = CandidatePath::from(Path::new("a/金/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("金", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_literal_escaped_class_tokens() {
        let glob = Glob::new("a/[\\[\\]\\-]/**").unwrap();

        assert!(glob.is_match(Path::new("a/[/file.ext")));
        assert!(glob.is_match(Path::new("a/]/file.ext")));
        assert!(glob.is_match(Path::new("a/-/file.ext")));

        assert!(!glob.is_match(Path::new("a/b/file.ext")));

        let path = CandidatePath::from(Path::new("a/[/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("[", matched.get(1).unwrap());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn match_glob_with_empty_class_tokens() {
        // A character class is "empty" if it only matches separators on the
        // target platform. Such a character class only matches `NUL` and so
        // effectively matches nothing.
        let glob = Glob::new("a[/]b").unwrap();

        assert!(!glob.is_match(Path::new("a/b")));
    }

    #[test]
    fn match_glob_with_alternative_tokens() {
        let glob = Glob::new("a/{x?z,y$}b/*").unwrap();

        assert!(glob.is_match(Path::new("a/xyzb/file.ext")));
        assert!(glob.is_match(Path::new("a/yb/file.ext")));

        assert!(!glob.is_match(Path::new("a/xyz/file.ext")));
        assert!(!glob.is_match(Path::new("a/y/file.ext")));
        assert!(!glob.is_match(Path::new("a/xyzub/file.ext")));

        let path = CandidatePath::from(Path::new("a/xyzb/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("xyz", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_nested_alternative_tokens() {
        let glob = Glob::new("a/{y$,{x?z,?z}}b/*").unwrap();

        let path = CandidatePath::from(Path::new("a/xyzb/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("xyz", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_alternative_tree_tokens() {
        let glob = Glob::new("a{/foo,/bar,/**/baz}/qux").unwrap();

        assert!(glob.is_match(Path::new("a/foo/qux")));
        assert!(glob.is_match(Path::new("a/foo/baz/qux")));
        assert!(glob.is_match(Path::new("a/foo/bar/baz/qux")));

        assert!(!glob.is_match(Path::new("a/foo/bar/qux")));
    }

    #[test]
    fn match_glob_with_alternative_repetition_tokens() {
        let glob = Glob::new("log-{<[0-9]:3>,<[0-9]:4>-<[0-9]:2>-<[0-9]:2>}.txt").unwrap();

        assert!(glob.is_match(Path::new("log-000.txt")));
        assert!(glob.is_match(Path::new("log-1970-01-01.txt")));

        assert!(!glob.is_match(Path::new("log-abc.txt")));
        assert!(!glob.is_match(Path::new("log-nope-no-no.txt")));
    }

    #[test]
    fn match_glob_with_repetition_tokens() {
        let glob = Glob::new("a/<[0-9]:6>/*").unwrap();

        assert!(glob.is_match(Path::new("a/000000/file.ext")));
        assert!(glob.is_match(Path::new("a/123456/file.ext")));

        assert!(!glob.is_match(Path::new("a/00000/file.ext")));
        assert!(!glob.is_match(Path::new("a/0000000/file.ext")));
        assert!(!glob.is_match(Path::new("a/bbbbbb/file.ext")));

        let path = CandidatePath::from(Path::new("a/999999/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("999999", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_negative_repetition_tokens() {
        let glob = Glob::new("<[!.]*/>[!.]*").unwrap();

        assert!(glob.is_match(Path::new("a/b/file.ext")));

        assert!(!glob.is_match(Path::new(".a/b/file.ext")));
        assert!(!glob.is_match(Path::new("a/.b/file.ext")));
        assert!(!glob.is_match(Path::new("a/b/.file.ext")));
    }

    #[test]
    fn match_glob_with_nested_repetition_tokens() {
        let glob = Glob::new("log<-<[0-9]:3>:1,2>.txt").unwrap();

        assert!(glob.is_match(Path::new("log-000.txt")));
        assert!(glob.is_match(Path::new("log-123-456.txt")));

        assert!(!glob.is_match(Path::new("log-abc.txt")));
        assert!(!glob.is_match(Path::new("log-123-456-789.txt")));

        let path = CandidatePath::from(Path::new("log-987-654.txt"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("-987-654", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_repeated_alternative_tokens() {
        let glob = Glob::new("<{a,b}:1,>/**").unwrap();

        assert!(glob.is_match(Path::new("a/file.ext")));
        assert!(glob.is_match(Path::new("b/file.ext")));
        assert!(glob.is_match(Path::new("aaa/file.ext")));
        assert!(glob.is_match(Path::new("bbb/file.ext")));

        assert!(!glob.is_match(Path::new("file.ext")));
        assert!(!glob.is_match(Path::new("c/file.ext")));

        let path = CandidatePath::from(Path::new("aa/file.ext"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("aa", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_rooted_tree_token() {
        let glob = Glob::new("/**/{var,.var}/**/*.log").unwrap();

        assert!(glob.is_match(Path::new("/var/log/network.log")));
        assert!(glob.is_match(Path::new("/home/nobody/.var/network.log")));

        assert!(!glob.is_match(Path::new("./var/cron.log")));
        assert!(!glob.is_match(Path::new("mnt/var/log/cron.log")));

        let path = CandidatePath::from(Path::new("/var/log/network.log"));
        let matched = glob.matched(&path).unwrap();
        assert_eq!("/", matched.get(1).unwrap());
    }

    #[test]
    fn match_glob_with_flags() {
        let glob = Glob::new("(?-i)photos/**/*.(?i){jpg,jpeg}").unwrap();

        assert!(glob.is_match(Path::new("photos/flower.jpg")));
        assert!(glob.is_match(Path::new("photos/flower.JPEG")));

        assert!(!glob.is_match(Path::new("Photos/flower.jpeg")));
    }

    #[test]
    fn match_glob_with_escaped_flags() {
        let glob = Glob::new("a\\(b\\)").unwrap();

        assert!(glob.is_match(Path::new("a(b)")));
    }

    #[test]
    fn match_any_combinator() {
        let any = crate::any(["src/**/*.rs", "doc/**/*.md", "pkg/**/PKGBUILD"]).unwrap();

        assert!(any.is_match("src/lib.rs"));
        assert!(any.is_match("doc/api.md"));
        assert!(any.is_match("pkg/arch/lib-git/PKGBUILD"));

        assert!(!any.is_match("img/icon.png"));
        assert!(!any.is_match("doc/LICENSE.tex"));
        assert!(!any.is_match("pkg/lib.rs"));
    }

    #[test]
    fn partition_glob_with_variant_and_invariant_parts() {
        let (prefix, glob) = Glob::new("a/b/x?z/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new("a/b"));

        assert!(glob.is_match(Path::new("xyz/file.ext")));
        assert!(glob.is_match(Path::new("a/b/xyz/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_only_variant_wildcard_parts() {
        let (prefix, glob) = Glob::new("x?z/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new(""));

        assert!(glob.is_match(Path::new("xyz/file.ext")));
        assert!(glob.is_match(Path::new("xyz/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_only_invariant_literal_parts() {
        let (prefix, glob) = Glob::new("a/b").unwrap().partition();

        assert_eq!(prefix, Path::new("a/b"));

        assert!(glob.is_match(Path::new("")));
        assert!(glob.is_match(Path::new("a/b").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_variant_alternative_parts() {
        let (prefix, glob) = Glob::new("{x,z}/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new(""));

        assert!(glob.is_match(Path::new("x/file.ext")));
        assert!(glob.is_match(Path::new("z/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_invariant_alternative_parts() {
        let (prefix, glob) = Glob::new("{a/b}/c").unwrap().partition();

        assert_eq!(prefix, Path::new("a/b/c"));

        assert!(glob.is_match(Path::new("")));
        assert!(glob.is_match(Path::new("a/b/c").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_invariant_repetition_parts() {
        let (prefix, glob) = Glob::new("</a/b:3>/c").unwrap().partition();

        assert_eq!(prefix, Path::new("/a/b/a/b/a/b/c"));

        assert!(glob.is_match(Path::new("")));
        assert!(glob.is_match(Path::new("/a/b/a/b/a/b/c").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_literal_dots_and_tree_tokens() {
        let (prefix, glob) = Glob::new("../**/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new(".."));

        assert!(glob.is_match(Path::new("xyz/file.ext")));
        assert!(glob.is_match(Path::new("../xyz/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_rooted_tree_token() {
        let (prefix, glob) = Glob::new("/**/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new("/"));
        assert!(!glob.has_root());

        assert!(glob.is_match(Path::new("file.ext")));
        assert!(glob.is_match(Path::new("/root/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_rooted_zom_token() {
        let (prefix, glob) = Glob::new("/*/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new("/"));
        assert!(!glob.has_root());

        assert!(glob.is_match(Path::new("root/file.ext")));
        assert!(glob.is_match(Path::new("/root/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_rooted_literal_token() {
        let (prefix, glob) = Glob::new("/root/**/*.ext").unwrap().partition();

        assert_eq!(prefix, Path::new("/root"));
        assert!(!glob.has_root());

        assert!(glob.is_match(Path::new("file.ext")));
        assert!(glob.is_match(Path::new("/root/file.ext").strip_prefix(prefix).unwrap()));
    }

    #[test]
    fn partition_glob_with_invariant_expression_text() {
        let (prefix, glob) = Glob::new("/root/file.ext").unwrap().partition();
        assert_eq!(prefix, Path::new("/root/file.ext"));
        assert_eq!(format!("{}", glob), "");

        let (prefix, glob) = Glob::new("<a:3>/file.ext").unwrap().partition();
        assert_eq!(prefix, Path::new("aaa/file.ext"));
        assert_eq!(format!("{}", glob), "");
    }

    #[test]
    fn partition_glob_with_variant_expression_text() {
        let (prefix, glob) = Glob::new("**/file.ext").unwrap().partition();
        assert_eq!(prefix, Path::new(""));
        assert_eq!(format!("{}", glob), "**/file.ext");

        let (prefix, glob) = Glob::new("/root/**/file.ext").unwrap().partition();
        assert_eq!(prefix, Path::new("/root"));
        assert_eq!(format!("{}", glob), "**/file.ext");

        let (prefix, glob) = Glob::new("/root/**").unwrap().partition();
        assert_eq!(prefix, Path::new("/root"));
        assert_eq!(format!("{}", glob), "**");
    }

    #[test]
    fn repartition_glob_with_variant_tokens() {
        let (prefix, glob) = Glob::new("/root/**/file.ext").unwrap().partition();
        assert_eq!(prefix, Path::new("/root"));
        assert_eq!(format!("{}", glob), "**/file.ext");

        let (prefix, glob) = glob.partition();
        assert_eq!(prefix, Path::new(""));
        assert_eq!(format!("{}", glob), "**/file.ext");
    }

    #[test]
    fn query_glob_has_root() {
        assert!(Glob::new("/root").unwrap().has_root());
        assert!(Glob::new("/**").unwrap().has_root());
        assert!(Glob::new("</root:1,>").unwrap().has_root());

        assert!(!Glob::new("").unwrap().has_root());
        // This is not rooted, because character classes may not match
        // separators. This example compiles an "empty" character class, which
        // attempts to match `NUL` and so effectively matches nothing.
        #[cfg(any(unix, windows))]
        assert!(!Glob::new("[/]root").unwrap().has_root());
        // The leading forward slash in tree tokens is meaningful. When omitted,
        // at the beginning of an expression, the resulting glob is not rooted.
        assert!(!Glob::new("**/").unwrap().has_root());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn query_glob_has_semantic_literals() {
        assert!(Glob::new("../src/**").unwrap().has_semantic_literals());
        assert!(Glob::new("*/a/../b.*").unwrap().has_semantic_literals());
        assert!(Glob::new("{a,..}").unwrap().has_semantic_literals());
        assert!(Glob::new("<a/..>").unwrap().has_semantic_literals());
        assert!(Glob::new("<a/{b,..,c}/d>").unwrap().has_semantic_literals());
        assert!(Glob::new("./*.txt").unwrap().has_semantic_literals());
    }

    #[test]
    fn query_glob_capture_indices() {
        let glob = Glob::new("**/{foo*,bar*}/???").unwrap();
        let indices: Vec<_> = glob.captures().map(|token| token.index()).collect();
        assert_eq!(&indices, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn query_glob_capture_spans() {
        let glob = Glob::new("**/{foo*,bar*}/$").unwrap();
        let spans: Vec<_> = glob.captures().map(|token| token.span()).collect();
        assert_eq!(&spans, &[(0, 3), (3, 11), (15, 1)]);
    }

    #[test]
    fn query_glob_variance() {
        assert!(Glob::new("").unwrap().variance().is_invariant());
        assert!(Glob::new("/a/file.ext").unwrap().variance().is_invariant());
        assert!(Glob::new("/a/{file.ext}")
            .unwrap()
            .variance()
            .is_invariant());
        assert!(Glob::new("{a/b/file.ext}")
            .unwrap()
            .variance()
            .is_invariant());
        assert!(Glob::new("{a,a}").unwrap().variance().is_invariant());
        #[cfg(windows)]
        assert!(Glob::new("{a,A}").unwrap().variance().is_invariant());
        assert!(Glob::new("<a/b:2>").unwrap().variance().is_invariant());
        #[cfg(unix)]
        assert!(Glob::new("/[a]/file.ext")
            .unwrap()
            .variance()
            .is_invariant());
        #[cfg(unix)]
        assert!(Glob::new("/[a-a]/file.ext")
            .unwrap()
            .variance()
            .is_invariant());
        #[cfg(unix)]
        assert!(Glob::new("/[a-aaa-a]/file.ext")
            .unwrap()
            .variance()
            .is_invariant());

        assert!(Glob::new("/a/{b,c}").unwrap().variance().is_variant());
        assert!(Glob::new("<a/b:1,>").unwrap().variance().is_variant());
        assert!(Glob::new("/[ab]/file.ext").unwrap().variance().is_variant());
        assert!(Glob::new("**").unwrap().variance().is_variant());
        assert!(Glob::new("/a/*.ext").unwrap().variance().is_variant());
        assert!(Glob::new("/a/b*").unwrap().variance().is_variant());
        #[cfg(unix)]
        assert!(Glob::new("/a/(?i)file.ext")
            .unwrap()
            .variance()
            .is_variant());
        #[cfg(windows)]
        assert!(Glob::new("/a/(?-i)file.ext")
            .unwrap()
            .variance()
            .is_variant());
    }
}
