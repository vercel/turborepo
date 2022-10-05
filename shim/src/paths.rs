use anyhow::Result;
use globset::GlobSet;
use std::ffi::OsString;
use std::path::Path;
use walkdir::{DirEntry, IntoIter, WalkDir};

#[derive(Debug, Clone, PartialEq)]
struct RelativePath {
    segments: Vec<OsString>,
}

pub struct GlobWalker {
    file_walker: IntoIter,
    inclusions: GlobSet,
    exclusions: GlobSet,
}

impl GlobWalker {
    pub fn new(dir: impl AsRef<Path>, inclusions: GlobSet, exclusions: GlobSet) -> GlobWalker {
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
        loop {
            let entry = self.file_walker.next()?;
            match entry {
                Ok(entry) => {
                    let matches = self.inclusions.is_match(entry.path());

                    if matches {
                        let is_excluded = self.exclusions.is_match(entry.path());

                        if is_excluded {
                            continue;
                        } else {
                            return Some(Ok(entry));
                        }
                    } else {
                        continue;
                    }
                }
                Err(err) => {
                    return Some(Err(err.into()));
                }
            }
        }
    }
}
