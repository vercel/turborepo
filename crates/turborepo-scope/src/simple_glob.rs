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
            // Escape everything except `*` so that characters such as `.`, `+`
            // or `(` in package names are matched literally.
            let regex_body = pattern
                .split('*')
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join(".*");
            let regex = Regex::new(&format!("^{regex_body}$"))?;
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

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    #[test_case("*", "anything", true ; "star matches anything")]
    #[test_case("web", "web", true ; "exact name")]
    #[test_case("web", "web2", false ; "exact name is not a prefix")]
    #[test_case("@acme/*", "@acme/ui", true ; "scope wildcard")]
    #[test_case("@acme/*", "@other/ui", false ; "scope wildcard other scope")]
    #[test_case("*-app", "docs-app", true ; "leading wildcard")]
    #[test_case("@acme/config.*", "@acme/config.eslint", true ; "dot matches literal dot")]
    #[test_case("@acme/config.*", "@acme/configs", false ; "dot does not match any char")]
    #[test_case("@acme/config.*", "@acme/config-eslint", false ; "dot does not match dash")]
    #[test_case("docs+v*", "docs+v2", true ; "plus matches literal plus")]
    #[test_case("docs+v*", "docssv2", false ; "plus is not a repetition")]
    #[test_case("pkg(*)", "pkg(a)", true ; "parens match literal parens")]
    #[test_case("pkg(*)", "pkga", false ; "parens are not a group")]
    #[test_case("pkg[*", "pkg[a", true ; "unbalanced bracket is literal")]
    fn test_simple_glob_matches(pattern: &str, name: &str, expected: bool) {
        let glob = SimpleGlob::new(pattern).unwrap();
        assert_eq!(glob.is_match(name), expected);
    }
}
