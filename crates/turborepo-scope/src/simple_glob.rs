//! Simple glob matching for package names.
//!
//! This module provides glob matching that works on package names
//! rather than file paths.

use regex::Regex;

/// A simple glob-like pattern that supports a subset of
/// glob syntax for the purposes of string matching.
/// If you are matching paths, use `turborepo_wax::glob::Glob` instead.
pub enum SimpleGlob {
    Regex(Regex),
    String(String),
    Any,
}

pub trait Match {
    fn is_match(&self, s: &str) -> bool;
}

impl SimpleGlob {
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        if pattern == "*" {
            Ok(SimpleGlob::Any)
        } else if pattern.contains('*') {
            let regex = Regex::new(&format!("^{}$", pattern.replace('*', ".*")))?;
            Ok(SimpleGlob::Regex(regex))
        } else {
            Ok(SimpleGlob::String(pattern.to_string()))
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Self::String(_))
    }
}

impl Match for SimpleGlob {
    fn is_match(&self, s: &str) -> bool {
        match self {
            SimpleGlob::Regex(regex) => regex.is_match(s),
            SimpleGlob::String(string) => string == s,
            SimpleGlob::Any => true,
        }
    }
}
