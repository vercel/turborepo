use std::{
    borrow::Borrow,
    fmt,
    ops::Deref,
    path::{Path, PathBuf},
};

use camino::{Utf8Component, Utf8Components, Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    check_path, AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, PathError,
    PathValidation,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct AnchoredSystemPathBuf(pub(crate) Utf8PathBuf);

impl TryFrom<&str> for AnchoredSystemPathBuf {
    type Error = PathError;

    fn try_from(path: &str) -> Result<Self, Self::Error> {
        let path = Utf8Path::new(path);
        if path.is_absolute() {
            return Err(PathError::NotRelative(path.to_string()));
        }

        Ok(AnchoredSystemPathBuf(path.into()))
    }
}

impl Borrow<AnchoredSystemPath> for AnchoredSystemPathBuf {
    fn borrow(&self) -> &AnchoredSystemPath {
        unsafe { AnchoredSystemPath::new_unchecked(self.0.as_path()) }
    }
}

impl AsRef<AnchoredSystemPath> for AnchoredSystemPathBuf {
    fn as_ref(&self) -> &AnchoredSystemPath {
        self.borrow()
    }
}

impl Deref for AnchoredSystemPathBuf {
    type Target = AnchoredSystemPath;

    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl TryFrom<&Path> for AnchoredSystemPathBuf {
    type Error = PathError;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        let path = path
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(path.to_string_lossy().to_string()))?;

        Self::try_from(path)
    }
}

impl fmt::Display for AnchoredSystemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// TODO: perhaps we ought to be converting to a unix path?
impl<'a> From<&'a AnchoredSystemPathBuf> for wax::CandidatePath<'a> {
    fn from(path: &'a AnchoredSystemPathBuf) -> Self {
        path.0.as_std_path().into()
    }
}

impl AnchoredSystemPathBuf {
    pub fn new(
        root: impl AsRef<AbsoluteSystemPath>,
        path: impl AsRef<AbsoluteSystemPath>,
    ) -> Result<Self, PathError> {
        let root = root.as_ref();
        let path = path.as_ref();
        let stripped_path = path
            .as_path()
            .strip_prefix(root.as_path())
            .map_err(|_| PathError::NotParent(root.to_string(), path.to_string()))?
            .into();

        Ok(AnchoredSystemPathBuf(stripped_path))
    }

    pub fn pop(&mut self) -> bool {
        self.0.pop()
    }

    // Produces a path from start to end, which may include directory traversal
    // tokens. Given that both parameters are absolute, we _should_ always be
    // able to produce such a path. The exception is when crossing drive letters
    // on Windows, where no such path is possible. Since a repository is
    // expected to only reside on a single drive, this shouldn't be an issue.
    pub fn relative_path_between(start: &AbsoluteSystemPath, end: &AbsoluteSystemPath) -> Self {
        // Filter the implicit "RootDir" component that exists for unix paths.
        // For windows paths, we may want an assertion that we aren't crossing drives
        let these_components = start
            .components()
            .skip_while(|c| *c == Utf8Component::RootDir)
            .collect::<Vec<_>>();
        let other_components = end
            .components()
            .skip_while(|c| *c == Utf8Component::RootDir)
            .collect::<Vec<_>>();
        let prefix_len = these_components
            .iter()
            .zip(other_components.iter())
            .take_while(|(a, b)| a == b)
            .count();
        #[cfg(windows)]
        debug_assert!(
            prefix_len >= 1,
            "Cannot traverse drives between {} and {}",
            start,
            end
        );

        let traverse_count = these_components.len() - prefix_len;
        // For every remaining non-matching segment in self, add a directory traversal
        // Then, add every non-matching segment from other
        let path = std::iter::repeat(Utf8Component::ParentDir)
            .take(traverse_count)
            .chain(other_components.into_iter().skip(prefix_len))
            .collect::<Utf8PathBuf>();

        let path: Utf8PathBuf = path_clean::clean(path)
            .try_into()
            .expect("clean should preserve utf8");

        Self(path)
    }

    pub fn from_raw(raw: impl AsRef<str>) -> Result<Self, PathError> {
        let system_path = raw.as_ref();
        Ok(Self(system_path.into()))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    // Takes in a path, validates that it is anchored and constructs an
    // `AnchoredSystemPathBuf` with no trailing slashes.
    pub fn from_system_path(path: &Path) -> Result<Self, PathError> {
        let path = path
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(path.to_string_lossy().to_string()))?;

        #[allow(unused_variables)]
        let PathValidation {
            well_formed,
            windows_safe,
        } = check_path(path);

        if !well_formed {
            return Err(PathError::MalformedPath(path.to_string()));
        }

        #[cfg(windows)]
        {
            if !windows_safe {
                return Err(PathError::WindowsUnsafePath(path.to_string()));
            }
        }

        // Remove trailing slash
        let stripped_path = path.strip_suffix('/').unwrap_or(path);

        let path;
        #[cfg(windows)]
        {
            let windows_path = stripped_path.replace('/', std::path::MAIN_SEPARATOR_STR);
            path = Utf8PathBuf::from(windows_path);
        }
        #[cfg(unix)]
        {
            path = Utf8PathBuf::from(stripped_path);
        }

        Ok(AnchoredSystemPathBuf(path))
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_std_path()
    }

    pub fn push(&mut self, path: impl AsRef<Utf8Path>) {
        self.0.push(path.as_ref());
    }

    pub fn components(&self) -> Utf8Components {
        self.0.components()
    }

    pub fn join(&self, other: &AnchoredSystemPathBuf) -> AnchoredSystemPathBuf {
        Self(self.0.join(other.0.to_owned()))
    }

    pub fn restore_anchor(&self, path: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        path.join_unix_path(self.to_unix().unwrap()).unwrap()
    }
}

impl From<AnchoredSystemPathBuf> for PathBuf {
    fn from(path: AnchoredSystemPathBuf) -> PathBuf {
        path.0.into_std_path_buf()
    }
}

impl AsRef<Utf8Path> for AnchoredSystemPathBuf {
    fn as_ref(&self) -> &Utf8Path {
        self.0.as_ref()
    }
}

impl AsRef<Path> for AnchoredSystemPathBuf {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    #[test_case(&["a", "b", "c", "..", "c"], &["..", "c"] ; "re-entry self")]
    #[test_case(&["a", "b", "c", "d", "..", "d"], &["d"] ; "re-entry child")]
    // TODO reorder
    #[test_case(&["a"], &["..", ".."] ; "parent")]
    #[test_case(&["a", "b", "c"], &["."] ; "empty self")]
    #[test_case(&["a", "b", "d"], &["..", "d"] ; "sibling")]
    #[test_case(&["a", "b", "c", "d"], &["d"] ; "child")]
    #[test_case(&["e", "f"], &["..", "..", "..", "e", "f"] ; "ancestor sibling")]
    #[test_case(&[], &["..", "..", ".."] ; "root")]
    fn test_relative_path_to(input: &[&str], expected: &[&str]) {
        #[cfg(unix)]
        let root_token = "/";
        #[cfg(windows)]
        let root_token = "C:\\";

        let root = AbsoluteSystemPathBuf::new(
            [root_token, "a", "b", "c"].join(std::path::MAIN_SEPARATOR_STR),
        )
        .unwrap();
        let mut parts = vec![root_token];
        parts.extend_from_slice(input);
        let target = AbsoluteSystemPathBuf::new(parts.join(std::path::MAIN_SEPARATOR_STR)).unwrap();
        let expected =
            AnchoredSystemPathBuf::from_raw(expected.join(std::path::MAIN_SEPARATOR_STR)).unwrap();

        let result = AnchoredSystemPathBuf::relative_path_between(&root, &target);

        assert_eq!(result, expected);
    }

    #[test_case(Path::new("test.txt"), Ok("test.txt"), Ok("test.txt") ; "hello world")]
    #[test_case(Path::new("something/"), Ok("something"), Ok("something") ; "Unix directory")]
    #[test_case(Path::new("something\\"), Ok("something\\"), Err("Path is not safe for windows: something\\".to_string()) ; "Windows unsafe")]
    #[test_case(Path::new("//"), Err("path is malformed: //".to_string()), Err("path is malformed: //".to_string()) ; "malformed name")]
    fn test_from_system_path(
        file_name: &Path,
        expected_unix: Result<&'static str, String>,
        expected_windows: Result<&'static str, String>,
    ) {
        let result = AnchoredSystemPathBuf::from_system_path(file_name).map_err(|e| e.to_string());
        let expected = if cfg!(windows) {
            expected_windows
        } else {
            expected_unix
        };
        match (result, expected) {
            (Ok(result), Ok(expected)) => {
                assert_eq!(result.as_str(), expected)
            }
            (Err(result), Err(expected)) => assert_eq!(result, expected),
            (result, expected) => panic!("Expected {:?}, got {:?}", expected, result),
        }
    }
}
