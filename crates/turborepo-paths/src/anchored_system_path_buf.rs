use std::{
    borrow::Borrow,
    ops::Deref,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{AbsoluteSystemPath, AnchoredSystemPath, IntoSystem, PathError, RelativeUnixPathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct AnchoredSystemPathBuf(pub(crate) PathBuf);

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
        if path.is_absolute() {
            let bad_path = path.display().to_string();
            return Err(PathError::NotRelative(bad_path));
        }

        Ok(AnchoredSystemPathBuf(path.into_system()?))
    }
}

// TODO: perhaps we ought to be converting to a unix path?
impl<'a> From<&'a AnchoredSystemPathBuf> for wax::CandidatePath<'a> {
    fn from(value: &'a AnchoredSystemPathBuf) -> wax::CandidatePath<'a> {
        value.as_path().into()
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
            .to_path_buf();

        Ok(AnchoredSystemPathBuf(stripped_path))
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
            .skip_while(|c| *c == Component::RootDir)
            .collect::<Vec<_>>();
        let other_components = end
            .components()
            .skip_while(|c| *c == Component::RootDir)
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
        let path = std::iter::repeat(Component::ParentDir)
            .take(traverse_count)
            .chain(other_components.into_iter().skip(prefix_len))
            .collect::<PathBuf>();
        Self(path)
    }

    pub fn from_raw(raw: impl AsRef<Path>) -> Result<Self, PathError> {
        let system_path = raw.as_ref();
        let system_path = system_path.into_system()?;
        Ok(Self(system_path))
    }

    // Takes in a path that has already been validated as anchored
    // via `check_name` in `turborepo-cache` and constructs an
    // `AnchoredSystemPathBuf` with no trailing slashes.
    pub fn from_validated_tar_path(path: &Path) -> Self {
        // There's no easier way to remove trailing slashes in Rust
        // because `OsString`s don't allow for manipulation.
        let no_trailing_slash: PathBuf = path.components().collect();

        // We know this is indeed anchored because of `check_name`,
        // and it is indeed system because we just split and combined with the
        // system path separator above
        unsafe { AnchoredSystemPathBuf::new_unchecked(no_trailing_slash) }
    }

    pub(crate) unsafe fn new_unchecked(path: impl Into<PathBuf>) -> Self {
        AnchoredSystemPathBuf(path.into())
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }

    pub fn to_str(&self) -> Result<&str, PathError> {
        self.0
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(self.0.to_string_lossy().to_string()))
    }

    pub fn to_unix(&self) -> Result<RelativeUnixPathBuf, PathError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bytes = self.0.as_os_str().as_bytes();
            RelativeUnixPathBuf::new(bytes)
        }
        #[cfg(not(unix))]
        {
            use crate::IntoUnix;
            let unix_buf = self.0.as_path().into_unix()?;
            let unix_str = unix_buf
                .to_str()
                .ok_or_else(|| PathError::InvalidUnicode(unix_buf.to_string_lossy().to_string()))?;
            RelativeUnixPathBuf::new(unix_str.as_bytes())
        }
    }

    pub fn push(&mut self, path: impl AsRef<Path>) {
        self.0.push(path.as_ref());
    }
}

impl From<AnchoredSystemPathBuf> for PathBuf {
    fn from(path: AnchoredSystemPathBuf) -> PathBuf {
        path.0
    }
}

impl AsRef<Path> for AnchoredSystemPathBuf {
    fn as_ref(&self) -> &Path {
        self.0.as_path()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use test_case::test_case;

    use crate::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    #[test]
    fn test_relative_path_to() {
        #[cfg(unix)]
        let root_token = "/";
        #[cfg(windows)]
        let root_token = "C:\\";

        let root = AbsoluteSystemPathBuf::new(
            [root_token, "a", "b", "c"].join(std::path::MAIN_SEPARATOR_STR),
        )
        .unwrap();

        // /a/b/c
        // vs
        // /a -> ../..
        // /a/b/d -> ../d
        // /a/b/c/d -> d
        // /e/f -> ../../../e/f
        // / -> ../../..
        let test_cases: &[(&[&str], &[&str])] = &[
            (&["a"], &["..", ".."]),
            (&["a", "b", "d"], &["..", "d"]),
            (&["a", "b", "c", "d"], &["d"]),
            (&["e", "f"], &["..", "..", "..", "e", "f"]),
            (&[], &["..", "..", ".."]),
        ];
        for (input, expected) in test_cases {
            let mut parts = vec![root_token];
            parts.extend_from_slice(input);
            let target =
                AbsoluteSystemPathBuf::new(parts.join(std::path::MAIN_SEPARATOR_STR)).unwrap();
            let expected =
                AnchoredSystemPathBuf::from_raw(expected.join(std::path::MAIN_SEPARATOR_STR))
                    .unwrap();
            let result = AnchoredSystemPathBuf::relative_path_between(&root, &target);
            assert_eq!(result, expected);
        }
    }

    #[test_case(Path::new("./foo/bar/"), "./foo/bar" ; "with trailing slash")]
    #[test_case(Path::new("foo/bar"), "foo/bar" ; "no trailing slash")]
    #[test_case(Path::new(""), "" ; "empty")]
    fn test_from_validated_tar_path(path: &Path, expected_path: &str) {
        let path = AnchoredSystemPathBuf::from_validated_tar_path(path);
        assert_eq!(path.to_str().unwrap(), expected_path);
    }
}
