use anyhow::Result;
use glob::Pattern;
use std::ffi::OsString;
use std::path;
use std::path::Path;
use walkdir::{DirEntry, IntoIter, WalkDir};

pub type AbsolutePath = path::Path;

#[derive(Debug, Clone, PartialEq)]
struct RelativePath {
    segments: Vec<OsString>,
}

pub struct GlobWalker {
    file_walker: IntoIter,
    inclusions: Vec<Pattern>,
    exclusions: Vec<Pattern>,
}

impl GlobWalker {
    pub fn new(
        dir: impl AsRef<Path>,
        inclusions: Vec<Pattern>,
        exclusions: Vec<Pattern>,
    ) -> GlobWalker {
        GlobWalker {
            file_walker: WalkDir::new(dir).into_iter(),
            inclusions,
            exclusions,
        }
    }
}

impl Iterator for GlobWalker {
    type Item = Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.file_walker.next()?;

        if let Ok(entry) = entry {
            let matches = self
                .inclusions
                .iter()
                .any(|inclusion| inclusion.matches_path(entry.path()));

            if matches {
                let is_excluded = self
                    .exclusions
                    .iter()
                    .any(|exclusion| exclusion.matches_path(entry.path()));

                if is_excluded {
                    None
                } else {
                    Some(Ok(entry))
                }
            } else {
                None
            }
        } else {
            Some(entry.map_err(|err| err.into()))
        }
    }
}
