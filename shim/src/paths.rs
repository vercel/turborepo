use std::ffi::OsString;
use std::path;

pub type AbsolutePath = path::Path;

#[derive(Debug, Clone, PartialEq)]
struct RelativePath {
    segments: Vec<OsString>,
}
