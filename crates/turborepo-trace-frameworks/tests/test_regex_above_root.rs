//! A case found in review.
//!
//! A `testRegex` that names a directory above the repository root was read by
//! splitting it at its `/`. An expression with no `/` between that name and its
//! wildcard, or with the separator written as a class or as `.`, had no such
//! split: jest runs the files in a checkout at that place, and they were no
//! tests. Discovery now matches the absolute path as jest does, so every such
//! expression finds its tests where jest finds them, and
//! `turborepo_trace_frameworks::location` says that the place decides.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use turborepo_trace_frameworks::{JsRegex, TestDiscovery, TestPattern};

fn is_test(regex: &str, root: &str, rel: &str) -> bool {
    let root = Path::new(root);
    let d = TestDiscovery {
        roots: vec![root.to_path_buf()],
        include: vec![TestPattern::Regex(JsRegex::new(regex, ""))],
        absolute: true,
        ..TestDiscovery::default()
    };
    d.matcher().rooted(root).is_test(&root.join(rel))
}

#[test]
fn a_test_regex_without_a_slash_after_a_name_above_the_root_keeps_its_tests() {
    let mut lost = Vec::new();
    for re in [
        // The checkout's own name, then a wildcard.
        "repo.*[.]test[.]js$",
        // The directory above it.
        "work.*[.]test[.]js$",
        // The separator as a class (written so for Windows) and as `.`.
        r"repo[\\/]src[\\/].*\.test\.js$",
        "repo[/]src[/].*[.]test[.]js$",
        "work.repo.src.*[.]test[.]js$",
    ] {
        // jest: `new RegExp(re).test('/work/repo/src/a.test.js')` is true for each.
        assert!(
            regex::Regex::new(&re.replace(r"[\\/]", "[/]"))
                .unwrap()
                .is_match("/work/repo/src/a.test.js"),
            "{re}"
        );
        if !is_test(re, "/work/repo", "src/a.test.js") {
            lost.push(re);
        }
        // And nowhere else, as in jest.
        assert!(!is_test(re, "/ci/checkout", "src/a.test.js"), "{re}");
    }
    assert!(lost.is_empty(), "no test by: {lost:?}");
}
