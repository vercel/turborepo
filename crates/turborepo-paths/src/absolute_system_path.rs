#[cfg(not(windows))]
use std::os::unix::fs::symlink as symlink_file;
#[cfg(not(windows))]
use std::os::unix::fs::symlink as symlink_dir;
#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::{
    fmt,
    fs::{File, Metadata, OpenOptions, Permissions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use camino::{Utf8Component, Utf8Components, Utf8Path, Utf8PathBuf};
use fs_err as fs;
use path_clean::PathClean;
use wax::CandidatePath;

use crate::{
    AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, PathError, RelativeUnixPath,
};

/// Models how two paths relate to each other
#[derive(Debug, PartialEq, Eq)]
pub enum PathRelation {
    /// e.g. /a/b vs /a/c
    Divergent,
    /// e.g. /a vs /a/b
    Parent,
    /// e.g. /a/b vs /a
    Child,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AbsoluteSystemPath(Utf8Path);

impl ToOwned for AbsoluteSystemPath {
    type Owned = AbsoluteSystemPathBuf;

    fn to_owned(&self) -> Self::Owned {
        AbsoluteSystemPathBuf(self.0.to_owned())
    }
}

impl AsRef<AbsoluteSystemPath> for AbsoluteSystemPath {
    fn as_ref(&self) -> &AbsoluteSystemPath {
        self
    }
}

impl fmt::Display for AbsoluteSystemPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.as_str())
    }
}

impl AsRef<Path> for AbsoluteSystemPath {
    fn as_ref(&self) -> &Path {
        self.0.as_std_path()
    }
}

impl AbsoluteSystemPath {
    /// Validates that the path is absolute and creates an `AbsoluteSystemPath`.
    /// Does *not* validate that the path is a system path. See module
    /// documentation for an explanation why.
    ///
    /// # Arguments
    ///
    /// * `value`: The path to convert to an absolute system path
    ///
    /// returns: Result<&AbsoluteSystemPath, PathError>
    ///
    /// # Examples
    ///
    /// ```
    /// use turbopath::AbsoluteSystemPath;
    /// #[cfg(unix)]
    /// {
    ///   assert!(AbsoluteSystemPath::new("/foo/bar").is_ok());
    ///   assert!(AbsoluteSystemPath::new("foo/bar").is_err());
    ///   assert!(AbsoluteSystemPath::new("C:\\foo\\bar").is_err());
    /// }
    ///
    /// #[cfg(windows)]
    /// {
    ///   assert!(AbsoluteSystemPath::new("C:\\foo\\bar").is_ok());
    ///   assert!(AbsoluteSystemPath::new("foo\\bar").is_err());
    ///   assert!(AbsoluteSystemPath::new("/foo/bar").is_err());
    /// }
    /// ```
    pub fn new<P: AsRef<str> + ?Sized>(value: &P) -> Result<&Self, PathError> {
        let path: &Utf8Path = value.as_ref().into();
        Self::from_utf8_path(path)
    }

    /// Converts a standard library `Path` to an `AbsoluteSystemPath`
    /// Errors if `Path` is not valid Unicode or if it's relative.
    pub fn from_std_path(path: &Path) -> Result<&Self, PathError> {
        let path: &Utf8Path = path.try_into()?;
        Self::from_utf8_path(path)
    }

    /// Converts a `Utf8Path` to an `AbsoluteSystemPath`.
    /// Errors if `Utf8Path` is relative.
    fn from_utf8_path(path: &Utf8Path) -> Result<&Self, PathError> {
        if path.is_relative() {
            return Err(PathError::NotAbsolute(path.to_string()));
        }
        Ok(Self::new_unchecked(path))
    }

    pub(crate) fn new_unchecked(path: &Utf8Path) -> &Self {
        unsafe { &*(path as *const Utf8Path as *const Self) }
    }

    pub fn as_path(&self) -> &Utf8Path {
        &self.0
    }

    pub fn as_std_path(&self) -> &Path {
        self.0.as_std_path()
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_str().as_bytes()
    }

    pub fn exists(&self) -> bool {
        self.0.exists()
    }

    pub fn ancestors(&self) -> impl Iterator<Item = &AbsoluteSystemPath> {
        self.0.ancestors().map(Self::new_unchecked)
    }

    pub fn create(&self) -> Result<File, io::Error> {
        File::create(&self.0)
    }

    /// Recursively creates a directory and all of its parent components
    /// if they are missing.
    pub fn create_dir_all(&self) -> Result<(), io::Error> {
        fs::create_dir_all(&self.0)
    }

    pub fn create_dir_all_with_permissions(
        &self,
        permissions: Permissions,
    ) -> Result<(), io::Error> {
        let (create, change_perms) = match fs::metadata(&self.0) {
            Ok(info) if info.is_dir() && info.permissions() == permissions => {
                // Directory already exists with correct permissions
                (false, false)
            }
            Ok(info) if info.is_dir() => (false, true),
            Ok(_) => {
                // Path exists as a file
                self.remove_file()?;
                (true, true)
            }
            // If this errors then the path doesn't exist and we can create it as expected
            Err(_) => (true, true),
        };
        if create {
            self.create_dir_all()?;
        }
        if change_perms {
            fs::set_permissions(&self.0, permissions)?;
        }

        Ok(())
    }

    /// Creates or truncates a file, then write the
    /// given contents to it
    pub fn create_with_contents<B: AsRef<[u8]>>(&self, contents: B) -> Result<(), io::Error> {
        let mut f = fs::File::create(&self.0)?;
        f.write_all(contents.as_ref())?;
        Ok(())
    }

    /// Removes a directory at this path, after removing all its contents. Use
    /// carefully! This function does not follow symbolic links and it will
    /// simply remove the symbolic link itself.
    pub fn remove_dir_all(&self) -> Result<(), io::Error> {
        fs::remove_dir_all(&self.0)
    }

    pub fn rename(&self, other: &Self) -> Result<(), io::Error> {
        fs::rename(&self.0, &other.0)
    }

    pub fn try_exists(&self) -> Result<bool, PathError> {
        // try_exists is an experimental API and not yet in fs_err
        Ok(std::fs::try_exists(&self.0)?)
    }

    pub fn extension(&self) -> Option<&str> {
        self.0.extension()
    }

    /// Intended for joining literals or obviously single-token strings
    pub fn join_component(&self, segment: &str) -> AbsoluteSystemPathBuf {
        debug_assert!(!segment.contains(std::path::MAIN_SEPARATOR));
        AbsoluteSystemPathBuf(
            self.0
                .join(segment)
                .as_std_path()
                .clean()
                .try_into()
                .unwrap(),
        )
    }

    /// Intended for joining a path composed of literals
    pub fn join_components(&self, segments: &[&str]) -> AbsoluteSystemPathBuf {
        debug_assert!(!segments
            .iter()
            .any(|segment| segment.contains(std::path::MAIN_SEPARATOR)));
        AbsoluteSystemPathBuf(
            self.0
                .join(segments.join(std::path::MAIN_SEPARATOR_STR))
                .as_std_path()
                .clean()
                .try_into()
                .unwrap(),
        )
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn join_unix_path(
        &self,
        unix_path: impl AsRef<RelativeUnixPath>,
    ) -> Result<AbsoluteSystemPathBuf, PathError> {
        let tail = unix_path.as_ref().to_system_path_buf();
        Ok(AbsoluteSystemPathBuf(
            self.0.join(tail).as_std_path().clean().try_into()?,
        ))
    }

    pub fn anchor(&self, path: &AbsoluteSystemPath) -> Result<AnchoredSystemPathBuf, PathError> {
        AnchoredSystemPathBuf::new(self, path)
    }

    pub fn ensure_dir(&self) -> Result<(), io::Error> {
        if let Some(parent) = self.0.parent() {
            fs::create_dir_all(parent)
        } else {
            Ok(())
        }
    }

    pub fn symlink_to_file<P: AsRef<str>>(&self, to: P) -> Result<(), PathError> {
        let target = to.as_ref();
        symlink_file(target, &self.0)?;
        Ok(())
    }

    pub fn symlink_to_dir<P: AsRef<str>>(&self, to: P) -> Result<(), PathError> {
        let target = to.as_ref();
        symlink_dir(target, &self.0)?;

        Ok(())
    }

    pub fn resolve(&self, path: &AnchoredSystemPath) -> AbsoluteSystemPathBuf {
        let path = self.0.join(path);
        AbsoluteSystemPathBuf(path)
    }

    /// Lexically cleans a path by doing the following:
    /// 1. Reduce multiple slashes to a single slash.
    /// 2. Eliminate . path name elements (the current directory).
    /// 3. Eliminate .. path name elements (the parent directory) and the non-.
    ///    non-.., element that precedes them.
    /// 4. Eliminate .. elements that begin a rooted path, that is, replace /..
    ///    by / at the beginning of a path.
    /// 5. Leave intact .. elements that begin a non-rooted path.
    pub fn clean(&self) -> Result<AbsoluteSystemPathBuf, PathError> {
        let cleaned_path = self
            .0
            .as_std_path()
            .clean()
            .try_into()
            .map_err(|_| PathError::InvalidUnicode(self.0.as_str().to_owned()))?;

        Ok(AbsoluteSystemPathBuf(cleaned_path))
    }

    /// Canonicalizes a path. Uses `dunce` to avoid UNC paths when possible.
    pub fn to_realpath(&self) -> Result<AbsoluteSystemPathBuf, PathError> {
        let realpath = dunce::canonicalize(&self.0)?;
        Ok(AbsoluteSystemPathBuf(Utf8PathBuf::try_from(realpath)?))
    }

    /// Gets metadata on path.
    /// NOTE: This is *not* lstat. If this is a symlink, it
    /// will return metadata for the target.
    pub fn stat(&self) -> Result<Metadata, PathError> {
        Ok(fs::metadata(&self.0)?)
    }

    /// The equivalent of lstat. Returns the metadata for this file,
    /// even if it is a symlink
    pub fn symlink_metadata(&self) -> Result<Metadata, PathError> {
        Ok(fs::symlink_metadata(&self.0)?)
    }

    pub fn read_link(&self) -> Result<Utf8PathBuf, io::Error> {
        self.0.read_link_utf8()
    }

    pub fn remove_file(&self) -> Result<(), io::Error> {
        fs::remove_file(&self.0)
    }

    pub fn remove_dir(&self) -> Result<(), io::Error> {
        fs::remove_dir(&self.0)
    }

    pub fn components(&self) -> Utf8Components<'_> {
        self.0.components()
    }

    pub fn collapse(&self) -> AbsoluteSystemPathBuf {
        let mut stack = vec![];
        for segment in self.0.components() {
            match segment {
                // skip over prefix/root dir
                // we can ignore this
                Utf8Component::CurDir => {
                    continue;
                }
                Utf8Component::ParentDir => {
                    // should error if there's nothing popped
                    stack.pop();
                }
                c => stack.push(c),
            }
        }
        debug_assert!(
            matches!(
                stack.first(),
                Some(Utf8Component::RootDir) | Some(Utf8Component::Prefix(_))
            ),
            "expected absolute path to start with root/prefix"
        );

        AbsoluteSystemPathBuf::new(stack.into_iter().collect::<Utf8PathBuf>())
            .expect("collapsed path should be absolute")
    }

    // TODO: consider consolidating with `relation_to_path` below
    pub fn contains(&self, other: &Self) -> bool {
        // On windows, trying to get a relative path between files on different volumes
        // is an error. We don't care about the error, it's good enough for us to say
        // that one path doesn't contain the other if they're on different volumes.
        #[cfg(windows)]
        if self.components().next() != other.components().next() {
            return false;
        }
        let this = self.collapse();
        let other = other.collapse();
        let rel = AnchoredSystemPathBuf::relative_path_between(&this, &other);
        rel.components().next() != Some(Utf8Component::ParentDir)
    }

    /// relation_to_path does a lexical comparison of path components to
    /// determine how this path relates to the given path. In the event that
    /// the paths are the same, we return `Parent`, much the way that `contains`
    /// would return `true`.
    pub fn relation_to_path(&self, other: &Self) -> PathRelation {
        let mut self_components = self.components();
        let mut other_components = other.components();
        loop {
            match (self_components.next(), other_components.next()) {
                // Non-matching component, the paths diverge
                (Some(self_component), Some(other_component))
                    if self_component != other_component =>
                {
                    return PathRelation::Divergent
                }
                // A matching component, continue iterating
                (Some(_), Some(_)) => {}
                // We've reached the end of a possible parent without hitting a
                // non-matching component. Return Parent.
                (None, _) => return PathRelation::Parent,
                // We've hit the end of the other path without hitting the
                // end of this path. Since we haven't hit a non-matching component,
                // our path must be a child
                (_, None) => return PathRelation::Child,
            }
        }
    }

    pub fn parent(&self) -> Option<&AbsoluteSystemPath> {
        self.0.parent().map(Self::new_unchecked)
    }

    /// Opens file and sets the `FILE_FLAG_SEQUENTIAL_SCAN` flag on Windows to
    /// help with performance
    pub fn open(&self) -> Result<File, io::Error> {
        let mut options = OpenOptions::new();
        options.read(true);

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            use crate::FILE_FLAG_SEQUENTIAL_SCAN;

            options.custom_flags(FILE_FLAG_SEQUENTIAL_SCAN);
        }

        options.open(&self.0)
    }

    pub fn open_with_options(&self, open_options: OpenOptions) -> Result<File, io::Error> {
        open_options.open(&self.0)
    }

    pub fn read(&self) -> Result<Vec<u8>, io::Error> {
        std::fs::read(self.as_path())
    }

    pub fn read_to_string(&self) -> Result<String, io::Error> {
        fs::read_to_string(&self.0)
    }

    /// Attempts to read a file, and:
    /// If the file does not exist it returns the default value.
    /// For all other scenarios passes through the `read_to_string` results.
    pub fn read_existing_to_string_or<I>(
        &self,
        default_value: Result<I, io::Error>,
    ) -> Result<String, io::Error>
    where
        I: Into<String>,
    {
        fs::read_to_string(&self.0).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                default_value.map(|intoable| intoable.into())
            } else {
                Err(e)
            }
        })
    }

    #[cfg(unix)]
    pub fn set_mode(&self, mode: u32) -> Result<(), io::Error> {
        use std::os::unix::fs::PermissionsExt;

        let permissions = Permissions::from_mode(mode);
        fs::set_permissions(&self.0, permissions)?;

        Ok(())
    }
}

impl<'a> From<&'a AbsoluteSystemPath> for CandidatePath<'a> {
    fn from(value: &'a AbsoluteSystemPath) -> Self {
        CandidatePath::from(value.0.as_std_path())
    }
}

impl PartialEq<AbsoluteSystemPath> for Path {
    fn eq(&self, other: &AbsoluteSystemPath) -> bool {
        Utf8Path::from_path(self)
            .map(|path| &other.0 == path)
            .unwrap_or(false)
    }
}

impl PartialEq<AbsoluteSystemPath> for PathBuf {
    fn eq(&self, other: &AbsoluteSystemPath) -> bool {
        self.as_path().eq(other)
    }
}

impl PartialEq<&AbsoluteSystemPath> for Path {
    fn eq(&self, other: &&AbsoluteSystemPath) -> bool {
        Utf8Path::from_path(self)
            .map(|path| &other.0 == path)
            .unwrap_or(false)
    }
}

impl PartialEq<&AbsoluteSystemPath> for PathBuf {
    fn eq(&self, other: &&AbsoluteSystemPath) -> bool {
        self.as_path().eq(other)
    }
}

impl<'a> TryFrom<&'a Path> for &'a AbsoluteSystemPath {
    type Error = PathError;

    fn try_from(value: &'a Path) -> Result<Self, Self::Error> {
        AbsoluteSystemPath::from_std_path(value)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempdir::TempDir;
    use test_case::test_case;

    use super::*;

    #[test]
    fn test_create_absolute_path() -> Result<()> {
        #[cfg(unix)]
        {
            let absolute_path = AbsoluteSystemPath::new("/foo/bar")?;
            assert_eq!(absolute_path.to_string(), "/foo/bar");
        }

        #[cfg(windows)]
        {
            let absolute_path = AbsoluteSystemPath::new(r"C:\foo\bar")?;
            assert_eq!(absolute_path.to_string(), r"C:\foo\bar");
        }

        Ok(())
    }

    #[test]
    fn test_resolve_empty() {
        let root = AbsoluteSystemPathBuf::cwd().unwrap();
        let empty = AnchoredSystemPathBuf::from_raw("").unwrap();
        let result = root.resolve(&empty);
        assert_eq!(result, root);
    }

    #[test_case(&["foo", "bar"], &["foo", "bar"] ; "no collapse")]
    #[test_case(&["foo", "..", "bar"], &["bar"] ; "parent traversal")]
    #[test_case(&["foo", ".", "bar"], &["foo", "bar"] ; "current dir")]
    #[test_case(&["foo", "bar", "..", "bar"], &["foo", "bar"] ; "re-entry")]
    fn test_collapse(input: &[&str], expected: &[&str]) {
        let root = if cfg!(windows) { "C:\\" } else { "/" };

        let path = AbsoluteSystemPathBuf::new(root)
            .unwrap()
            .join_components(input);

        let expected = AbsoluteSystemPathBuf::new(root)
            .unwrap()
            .join_components(expected);

        assert_eq!(path.collapse(), expected);
    }

    #[test_case(&["elsewhere"], false ; "no shared prefix")]
    #[test_case(&["some", "sibling"], false ; "sibling")]
    #[test_case(&["some", "path"], true ; "reflexive")]
    #[test_case(&["some", "path", "..", "path", "inside", "parent"], true ; "re-enters base")]
    #[test_case(&["some", "path", "inside", "..", "inside", "parent"], true ; "re-enters child")]
    #[test_case(&["some", "path", "inside", "..", "..", "outside", "parent"], false ; "exits base")]
    #[test_case(&["some", "path2"], false ; "lexical prefix match")]
    fn test_contains(other: &[&str], expected: bool) {
        let root_token = match cfg!(windows) {
            true => "C:\\",
            false => "/",
        };

        let base = AbsoluteSystemPathBuf::new(
            [root_token, "some", "path"].join(std::path::MAIN_SEPARATOR_STR),
        )
        .unwrap();
        let other = AbsoluteSystemPathBuf::new(
            std::iter::once(root_token)
                .chain(other.iter().copied())
                .collect::<Vec<_>>()
                .join(std::path::MAIN_SEPARATOR_STR),
        )
        .unwrap();

        assert_eq!(base.contains(&other), expected);
    }

    // Constructing a windows permissions struct is only possible by calling
    // fs::metadata so we only run these tests on unix.
    #[cfg(unix)]
    mod unix {
        use std::os::unix::fs::PermissionsExt;

        use test_case::test_case;

        use super::*;
        const PERMISSION_MASK: u32 = 0o777;

        #[test_case(false, None, Permissions::from_mode(0o777) ; "dir doesn't exist")]
        #[test_case(false, Some(Permissions::from_mode(0o666)), Permissions::from_mode(0o755) ; "path exists as file")]
        #[test_case(true, Some(Permissions::from_mode(0o755)), Permissions::from_mode(0o655) ; "dir exists with incorrect mode")]
        #[test_case(false, Some(Permissions::from_mode(0o755)), Permissions::from_mode(0o755) ; "dir exists with correct mode")]
        fn test_mkdir_all_with_perms(
            is_dir: bool,
            mode: Option<Permissions>,
            expected: Permissions,
        ) -> Result<()> {
            let test_dir = TempDir::new("mkdir-all")?;

            let test_path = test_dir.path().join("foo");

            if let Some(perm) = mode {
                if is_dir {
                    fs::create_dir(&test_path)?;
                } else {
                    fs::File::create(&test_path)?;
                }
                fs::set_permissions(&test_path, perm)?;
            }

            let path = AbsoluteSystemPathBuf::new(test_path.to_str().unwrap())?;
            path.create_dir_all_with_permissions(expected.clone())?;

            let actual = fs::metadata(path.as_path())?;

            assert!(actual.is_dir());

            assert_eq!(
                actual.permissions().mode() & PERMISSION_MASK,
                expected.mode()
            );

            Ok(())
        }
    }

    #[test_case(&["a", "b"], &["a", "b"], PathRelation::Parent ; "equal paths return parent")]
    #[test_case(&["a"], &["a", "b"], PathRelation::Parent ; "a is a parent of a/b")]
    #[test_case(&["a", "b"], &["a"], PathRelation::Child ; "a/b is a child of a")]
    #[test_case(&["a", "b"], &["a", "c"], PathRelation::Divergent ; "a/b and a/c are divergent")]
    fn test_path_relation(
        abs_path_components: &[&str],
        other_components: &[&str],
        expected: PathRelation,
    ) {
        #[cfg(windows)]
        let root = "C:\\";
        #[cfg(not(windows))]
        let root = "/";

        let abs_path = AbsoluteSystemPathBuf::try_from(root)
            .unwrap()
            .join_components(abs_path_components);
        let other_path = AbsoluteSystemPathBuf::try_from(root)
            .unwrap()
            .join_components(other_components);

        let relation = abs_path.relation_to_path(&other_path);
        assert_eq!(relation, expected);
    }
}
