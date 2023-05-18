//! A simple `wax` combinator that unconditionally matches if the set of globs
//! is empty.

use wax::{Any, BuildError, CandidatePath, Compose, Pattern};

pub struct InclusiveEmptyAny<'a>(Option<Any<'a>>);

impl<'a> InclusiveEmptyAny<'a> {
    pub fn new<I>(patterns: I) -> Result<Self, BuildError>
    where
        I: IntoIterator,
        I::Item: Compose<'a>,
    {
        let iter = patterns.into_iter().collect::<Vec<_>>();
        if iter.len() == 0 {
            Ok(Self(None))
        } else {
            Ok(Self(Some(wax::any(iter)?)))
        }
    }
}

impl<'t> InclusiveEmptyAny<'t> {
    pub fn is_match(&self, path: impl Into<CandidatePath<'t>>) -> bool {
        self.0.as_ref().map_or(true, |any| any.is_match(path))
    }
}
