use std::{fmt, path::Path};

use camino::{Utf8Component, Utf8Path};
use path_clean::PathClean;
use serde::Serialize;

use crate::{AnchoredSystemPathBuf, PathError, PathRelation, RelativeUnixPathBuf};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Hash)]
#[serde(transparent)]
pub struct AnchoredSystemPath(Utf8Path);

impl ToOwned for AnchoredSystemPath {
    type Owned = AnchoredSystemPathBuf;

    fn to_owned(&self) -> Self::Owned {
        AnchoredSystemPathBuf(self.0.to_owned())
    }
}

impl AsRef<AnchoredSystemPath> for AnchoredSystemPath {
    fn as_ref(&self) -> &AnchoredSystemPath {
        self
    }
}

impl fmt::Display for AnchoredSystemPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<Utf8Path> for AnchoredSystemPath {
    fn as_ref(&self) -> &Utf8Path {
        &self.0
    }
}

impl AsRef<Path> for AnchoredSystemPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

const EMPTY: &str = "";

impl AnchoredSystemPath {
    pub(crate) unsafe fn new_unchecked<'a>(path: impl AsRef<Path> + 'a) -> &'a Self {
        let path = path.as_ref();
        unsafe { &*(path as *const Path as *const Self) }
    }

    pub fn new<'a>(path: impl AsRef<str> + 'a) -> Result<&'a Self, PathError> {
        let path_str = path.as_ref();
        let path = Path::new(path_str);
        if path.is_absolute() {
            return Err(PathError::NotRelative(path_str.to_string()));
        }

        Ok(unsafe { &*(path as *const Path as *const Self) })
    }

    pub fn empty() -> &'static Self {
        unsafe { Self::new_unchecked(EMPTY) }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn parent(&self) -> Option<&AnchoredSystemPath> {
        self.0
            .parent()
            .map(|path| unsafe { AnchoredSystemPath::new_unchecked(path) })
    }

    pub fn ancestors(&self) -> impl Iterator<Item = &AnchoredSystemPath> {
        self.0
            .ancestors()
            .map(|path| unsafe { AnchoredSystemPath::new_unchecked(path) })
    }

    pub fn components(&self) -> impl Iterator<Item = Utf8Component> {
        self.0.components()
    }

    pub fn as_path(&self) -> &Path {
        self.0.as_std_path()
    }

    pub fn to_unix(&self) -> RelativeUnixPathBuf {
        #[cfg(unix)]
        let buf = RelativeUnixPathBuf::new(self.0.as_str());

        #[cfg(not(unix))]
        let buf = {
            use crate::IntoUnix;
            let unix_buf = self.0.into_unix();
            RelativeUnixPathBuf::new(unix_buf)
        };

        buf.unwrap_or_else(|_| panic!("anchored system path is relative: {}", self.0.as_str()))
    }

    pub fn join_component(&self, segment: &str) -> AnchoredSystemPathBuf {
        debug_assert!(!segment.contains(std::path::MAIN_SEPARATOR));
        AnchoredSystemPathBuf(self.0.join(segment))
    }

    pub fn join_components(&self, segments: &[&str]) -> AnchoredSystemPathBuf {
        debug_assert!(!segments
            .iter()
            .any(|segment| segment.contains(std::path::MAIN_SEPARATOR)));
        AnchoredSystemPathBuf(
            self.0
                .join(segments.join(std::path::MAIN_SEPARATOR_STR))
                .as_std_path()
                .clean()
                .try_into()
                .unwrap(),
        )
    }

    pub fn clean(&self) -> AnchoredSystemPathBuf {
        AnchoredSystemPathBuf(self.0.as_std_path().clean().try_into().unwrap())
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

    // Note that this defers to Path::strip_prefix, which operates on components,
    // and therefore enforces boundaries at path dividers.
    pub fn strip_prefix(&self, other: &Self) -> Option<AnchoredSystemPathBuf> {
        self.0
            .strip_prefix(&other.0)
            .ok()
            .map(|path| AnchoredSystemPathBuf(path.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::{AnchoredSystemPathBuf, PathRelation};

    #[test_case(&["a", "b"], &["a", "b"], PathRelation::Parent ; "equal paths return parent")]
    #[test_case(&["a"], &["a", "b"], PathRelation::Parent ; "a is a parent of a/b")]
    #[test_case(&["a", "b"], &["a"], PathRelation::Child ; "a/b is a child of a")]
    #[test_case(&["a", "b"], &["a", "c"], PathRelation::Divergent ; "a/b and a/c are divergent")]
    fn test_path_relation(
        abs_path_components: &[&str],
        other_components: &[&str],
        expected: PathRelation,
    ) {
        let abs_path = AnchoredSystemPathBuf::try_from("")
            .unwrap()
            .join_components(abs_path_components);
        let other_path = AnchoredSystemPathBuf::try_from("")
            .unwrap()
            .join_components(other_components);

        let relation = abs_path.relation_to_path(&other_path);
        assert_eq!(relation, expected);
    }
}
