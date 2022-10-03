use anyhow::Result;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub type AbsolutePath = Path;

#[derive(Debug, Clone, PartialEq)]
struct RelativePath {
    segments: Vec<OsString>,
}

/// An iterator for the ancestor path from a current directory to the root of the file system.
/// Returns any files with a given name in the ancestor path.
pub struct AncestorSearch<'a> {
    current_dir: PathBuf,
    file_name: &'a str,
}

impl<'a> AncestorSearch<'a> {
    pub fn new(current_dir: PathBuf, file_name: &'a str) -> Result<Self> {
        Ok(Self {
            current_dir: fs::canonicalize(current_dir)?,
            file_name,
        })
    }
}

impl<'a> Iterator for AncestorSearch<'a> {
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        while fs::metadata(self.current_dir.join(&self.file_name)).is_err() {
            // Pops off current folder and sets to `current_dir.parent`
            // if false, `current_dir` has no parent
            if !self.current_dir.pop() {
                return None;
            }
        }
        Some(self.current_dir.join(self.file_name))
    }
}
