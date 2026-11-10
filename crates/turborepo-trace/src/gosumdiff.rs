//! When a changed `go.sum` stops being a file change.
//!
//! Every Go file has a manifest edge to its module's `go.mod` and `go.sum`, so
//! a change to either selects every test of the module. For `go.mod` that is
//! the rule: the build list is a function of `go.mod`, and without the `go.mod`
//! files of the dependencies (the module cache) nothing says which module
//! depends on which, so a bump of any module, direct or indirect, can change
//! what any third-party import compiles against.
//!
//! `go.sum` only verifies the build list. The lockfile diff turns its changes
//! into changed packages, which reaches the files that link a changed module,
//! but not the files that link a module that *depends* on it: `go.sum` has no
//! dependency graph, and an indirect module is not a node of the graph at all.
//! So a `go.sum` whose module hashes changed stays a whole-file change. It is
//! dropped only when both sides list the same lines (reordered or reformatted),
//! which can change no build.

use std::collections::BTreeSet;

/// Whether two `go.sum` texts list the same module hashes: the same set of
/// lines after trimming, ignoring blank lines and order. A missing side equals
/// an empty file.
pub fn same_lines(old: Option<&str>, new: Option<&str>) -> bool {
    let lines = |t: Option<&str>| -> BTreeSet<String> {
        t.unwrap_or("")
            .lines()
            .map(|l| l.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect()
    };
    lines(old) == lines(new)
}

#[cfg(test)]
mod tests {
    use super::same_lines;

    const A: &str = "a.test/m v1.0.0 h1:AAA=\na.test/m v1.0.0/go.mod h1:BBB=\n";

    #[test]
    fn order_and_whitespace_do_not_count() {
        let reordered = "a.test/m v1.0.0/go.mod  h1:BBB=\n\na.test/m v1.0.0 h1:AAA=\n";
        assert!(same_lines(Some(A), Some(reordered)));
        assert!(same_lines(None, Some("\n")));
    }

    #[test]
    fn a_changed_added_or_removed_hash_counts() {
        assert!(!same_lines(Some(A), Some(&A.replace("AAA", "CCC"))));
        assert!(!same_lines(
            Some(A),
            Some(&format!("{A}b.test/n v0.1.0 h1:DDD=\n"))
        ));
        assert!(!same_lines(Some(A), None));
    }
}
