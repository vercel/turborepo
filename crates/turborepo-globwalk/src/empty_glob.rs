//! A simple `wax` combinator that unconditionally matches if the set of globs
//! is empty.

use wax::{Any, CandidatePath, Glob, Pattern};

use crate::{any_with_contextual_error, WalkError};

pub struct InclusiveEmptyAny<'a>(Option<Any<'a>>);

impl<'a> InclusiveEmptyAny<'a> {
    pub fn new(patterns: Vec<Glob<'static>>, text: Vec<String>) -> Result<Self, WalkError> {
        let iter = patterns.into_iter().collect::<Vec<_>>();
        if iter.is_empty() {
            Ok(Self(None))
        } else {
            Ok(Self(Some(any_with_contextual_error(iter, text)?)))
        }
    }
}

impl<'t> InclusiveEmptyAny<'t> {
    pub fn is_match(&self, path: impl Into<CandidatePath<'t>>) -> bool {
        self.0.as_ref().map_or(true, |any| any.is_match(path))
    }
}
