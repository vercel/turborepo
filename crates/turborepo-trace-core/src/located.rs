//! A load next to a module that is not installed: what a resolver hands to the
//! graph store.
//!
//! A JavaScript load can say where it resolves from
//! (`createRequire(require.resolve('pkg'))`, `require.resolve('dep', { paths:
//! [dir] })`). In a checkout without `node_modules` the module is not on disk,
//! so the file the load names is not known. What the module *is* decides what
//! the load can reach: a package of the registry keeps what it loads by itself
//! inside its own installed tree, a package of the repository does not (a path
//! next to it is a file of the repository).
//!
//! Which of the two it is, is the lockfile's answer for the importer, the same
//! answer that links a plain import of the module. A resolver has no lockfile,
//! and a graph must not depend on anything a refresh does not check again. So
//! the resolver emits the edge of the load with a specifier of a fixed form
//! ([`spec`]) and the record that holds for a registry package, and the store,
//! which links through the lockfile and keeps every such link in the shard's
//! key, turns the record into an open one ([`open_detail`]) when the lockfile
//! links the importer to a package of the repository.

use std::path::Path;

const MARK: &str = " (not installed: ";
const FROM: &str = " from ";

/// The specifier of the edge: `<text> (not installed: <package>)`, or with `
/// from <dir>` when the search started in a directory of another package of the
/// repository (whose lockfile importer decides). The text has spaces: no
/// lockfile entry is looked up for the edge itself.
pub fn spec(text: &str, package: &str, from: Option<&Path>) -> String {
    match from {
        Some(dir) => format!("{text}{MARK}{package}{FROM}{})", dir.display()),
        None => format!("{text}{MARK}{package})"),
    }
}

/// The package of a specifier made by [`spec`], and the directory the search
/// started in when it is not the referencing file's package.
pub fn parse(spec: &str) -> Option<(&str, Option<&str>)> {
    let rest = spec.strip_suffix(')')?;
    let rest = &rest[rest.rfind(MARK)? + MARK.len()..];
    Some(match rest.split_once(FROM) {
        Some((package, dir)) => (package, Some(dir)),
        None => (rest, None),
    })
}

/// How the detail of the record continues when the module is taken to be a
/// registry package (after what [`open_detail`] keeps).
pub const STAYS: &str = "; the ";

/// The detail of the open record: what `detail` says up to the assumption, and
/// that the file is not known.
pub fn open_detail(detail: &str) -> String {
    let head = detail.rfind(STAYS).map_or(detail, |i| &detail[..i]);
    format!("{head}, so the file is not known")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_specifier_round_trips() {
        let own = spec("require.resolve('@a/b')/../x.js", "@a/b", None);
        assert_eq!(own, "require.resolve('@a/b')/../x.js (not installed: @a/b)");
        assert_eq!(parse(&own), Some(("@a/b", None)));
        let other = spec(
            "require.resolve('c', from '../p')",
            "c",
            Some(Path::new("/r/packages/p")),
        );
        assert_eq!(parse(&other), Some(("c", Some("/r/packages/p"))));
        assert_eq!(parse("require.resolve('c') (not installed)"), None);
        assert_eq!(parse("left-pad"), None);
    }

    #[test]
    fn the_open_detail_drops_the_assumption() {
        assert_eq!(
            open_detail(
                "x: 'p' is not installed (searched from a/b); the load is taken to stay in the \
                 package tree of the lockfile entry of 'p'"
            ),
            "x: 'p' is not installed (searched from a/b), so the file is not known"
        );
        assert_eq!(open_detail("x"), "x, so the file is not known");
    }
}
