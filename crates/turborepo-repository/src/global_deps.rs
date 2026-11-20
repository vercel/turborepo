//! Matching changed files against user-configured global dependency globs.
//!
//! Negated globs exclude files from the union of positive globs, as they do
//! when the global hash collects files.

use wax::{BuildError, Glob, Program};

pub struct GlobalDepsMatcher<'a> {
    inclusions: wax::Any<'a>,
    exclusions: wax::Any<'a>,
}

impl<'a> GlobalDepsMatcher<'a> {
    /// Compile global dependency globs, returning an error for invalid
    /// patterns.
    pub fn new(globs: impl IntoIterator<Item = &'a str>) -> Result<Self, BuildError> {
        Self::compile(globs, |_, error| Err(error))
    }

    /// Compile valid globs and report invalid patterns without discarding the
    /// remaining patterns. Used for task-level affected detection.
    pub fn new_ignoring_invalid(
        globs: impl IntoIterator<Item = &'a str>,
        mut on_invalid: impl FnMut(&str, &BuildError),
    ) -> Result<Self, BuildError> {
        Self::compile(globs, |glob, error| {
            on_invalid(glob, &error);
            Ok(())
        })
    }

    fn compile(
        globs: impl IntoIterator<Item = &'a str>,
        mut on_invalid: impl FnMut(&str, BuildError) -> Result<(), BuildError>,
    ) -> Result<Self, BuildError> {
        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();
        for raw_glob in globs {
            let (glob, destination) = if let Some(exclusion) = raw_glob.strip_prefix('!') {
                (exclusion, &mut exclusions)
            } else {
                (raw_glob, &mut inclusions)
            };
            match Glob::new(glob) {
                Ok(glob) => destination.push(glob),
                Err(error) => on_invalid(raw_glob, error)?,
            }
        }
        Ok(Self {
            inclusions: wax::any(inclusions)?,
            exclusions: wax::any(exclusions)?,
        })
    }

    pub fn is_match(&self, path: &str) -> bool {
        self.inclusions.is_match(path) && !self.exclusions.is_match(path)
    }
}

#[cfg(test)]
mod tests {
    use super::GlobalDepsMatcher;

    #[test]
    fn excluded_files_do_not_match() {
        let matcher = GlobalDepsMatcher::new(["ci/**", "!ci/test/**"]).unwrap();
        assert!(matcher.is_match("ci/plan.ts"));
        assert!(!matcher.is_match("ci/test/plan.test.ts"));
        assert!(!matcher.is_match("docs/notes.md"));
    }

    #[test]
    fn invalid_patterns_can_be_skipped() {
        let mut invalid = Vec::new();
        let matcher = GlobalDepsMatcher::new_ignoring_invalid(
            ["[invalid", "ci/**", "!ci/test/**"],
            |glob, _| invalid.push(glob.to_string()),
        )
        .unwrap();
        assert_eq!(invalid, ["[invalid"]);
        assert!(matcher.is_match("ci/plan.ts"));
        assert!(!matcher.is_match("ci/test/plan.test.ts"));
    }
}
