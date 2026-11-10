//! A jest project's discovery as its config writes it, without the place of the
//! checkout (`turborepo_trace_frameworks::location::DiscoverySpec`), for
//! configs that compute their patterns and directories from their own location.
//!
//! `testMatch: [path.join(__dirname, 'src/**/*.test.js')]` evaluates to a
//! pattern with the checkout's absolute path in it, like a pattern that spells
//! that path out. The adapter loads such a config once more as if the
//! repository were at a neutral root: what the config computed from its
//! location has that root in its text, what it spelled has not.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::DirCache;
use turborepo_trace_frameworks::{
    FrameworkInputs, framework, fsx,
    location::{ProjectSpec, Tree, verdict},
};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Loads the jest config at `rel` as the graph build does: confined to the
/// repository.
fn load(repo: &Path, rel: &str) -> FrameworkInputs {
    let fs = Arc::new(DirCache::new());
    fs.confine(repo, []);
    let jest = framework("jest").unwrap();
    fsx::with_fs(&fs, || jest.load(&repo.join(rel)))
}

fn repo(name: &str) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root =
        turborepo_trace_core::fs::normalize(&std::fs::canonicalize(d.path()).unwrap()).join(name);
    std::fs::create_dir_all(&root).unwrap();
    (d, root)
}

const HEAD: &str = "const path = require('node:path');\n";

/// Patterns, directories and a root directory computed from `__dirname`, in a
/// config of the root and in one of a package that spreads a shared base: kept
/// exactly, and the place decides nothing.
#[test]
fn what_a_config_computes_from_its_own_location_is_kept_exactly() {
    let (_d, root) = repo("checkout");
    write(
        &root,
        "jest.config.js",
        &format!(
            "{HEAD}module.exports = {{\n  rootDir: __dirname,\n  roots: [path.join(__dirname, \
             'src'), path.resolve(__dirname, 'lib')],\n  testMatch: [path.join(__dirname, \
             'src/**/*.test.js'), `${{__dirname}}/lib/**/*.spec.js`],\n  testPathIgnorePatterns: \
             [path.join(__dirname, 'src', 'skip'), '/node_modules/'],\n  \
             modulePathIgnorePatterns: [`${{__dirname}}/lib/gen/`],\n}};\n"
        ),
    );
    // A shared base, and a package config that adds what it computes from its own
    // place.
    write(
        &root,
        "jest.base.js",
        "module.exports = { testPathIgnorePatterns: ['/node_modules/'] };\n",
    );
    write(
        &root,
        "packages/a/jest.config.js",
        &format!(
            "{HEAD}const base = require('../../jest.base');\nmodule.exports = {{\n  ...base,\n  \
             rootDir: __dirname,\n  testRegex: `${{__dirname}}/checks/.*[.]js$`,\n  roots: \
             [path.join(__dirname, 'checks'), path.resolve(__dirname, '..', 'shared')],\n}};\n"
        ),
    );
    for f in [
        "src/a.test.js",
        "src/skip/b.test.js",
        "lib/c.spec.js",
        "lib/gen/d.spec.js",
        "packages/a/checks/one.js",
        "packages/shared/x.js",
    ] {
        write(&root, f, "test('t', () => {});\n");
    }
    let files: Vec<String> = [
        "lib/c.spec.js",
        "lib/gen/d.spec.js",
        "packages/a/checks/one.js",
        "packages/shared/x.js",
        "src/a.test.js",
        "src/skip/b.test.js",
    ]
    .map(str::to_owned)
    .to_vec();
    for (config, tests) in [
        ("jest.config.js", vec!["lib/c.spec.js", "src/a.test.js"]),
        (
            "packages/a/jest.config.js",
            vec!["packages/a/checks/one.js"],
        ),
    ] {
        let inputs = load(&root, config);
        let d = &inputs.projects[0].discovery;
        let (spec, fell_back) = ProjectSpec::of(d).expect("jest keeps a spec");
        assert!(spec.exact(), "{config}: {:?}", spec.configured.inexact);
        // No text of the place is in it.
        let text = spec.to_text();
        assert!(
            !text.contains(root.to_str().unwrap()) && !text.contains("checkout"),
            "{text}"
        );
        // Taken at the place it was loaded at, it is the discovery as loaded; at
        // another place it finds the same files.
        let m = d.matcher().rooted(&root);
        let found: Vec<&str> = files
            .iter()
            .map(String::as_str)
            .filter(|f| m.is_test(&root.join(f)))
            .collect();
        assert_eq!(found, tests, "{config}");
        for other in ["/ci/x", "/home/runner/work/app/app", "/a"] {
            let other = Path::new(other);
            let there = spec.applying(fell_back).at(other);
            let m = there.matcher().rooted(other);
            let found: Vec<&str> = files
                .iter()
                .map(String::as_str)
                .filter(|f| m.is_test(&other.join(f)))
                .collect();
            assert_eq!(found, tests, "{config} at {}", other.display());
            let v = verdict(&spec, fell_back, other, &Tree::new(&files));
            assert!(!v.dependent, "{config} at {}", other.display());
        }
        assert!(!verdict(&spec, fell_back, &root, &Tree::new(&files)).dependent);
    }
}

/// A pattern that names the checkout's directory, by spelling its path or by
/// stepping out and back in through its name, is not what the config computed
/// from its location alone: such a discovery is not kept exactly, and at this
/// place it finds its tests as jest does.
#[test]
fn a_pattern_through_the_checkouts_name_is_not_kept_exactly() {
    let (_d, root) = repo("checkout");
    let at = root.display();
    write(
        &root,
        "spelled/jest.config.js",
        &format!("module.exports = {{ testRegex: '{at}/spelled/checks/.*[.]js$' }};\n"),
    );
    write(
        &root,
        "reenter/jest.config.js",
        &format!(
            "{HEAD}module.exports = {{\n  testMatch: [path.join(__dirname, '..', '..', \
             'checkout', 'reenter', 'checks/**/*.js')],\n}};\n"
        ),
    );
    write(
        &root,
        "named/jest.config.js",
        &format!(
            "{HEAD}module.exports = {{\n  testRegex: path.basename(path.dirname(__dirname)) + \
             '/named/checks/.*[.]js$',\n}};\n"
        ),
    );
    for dir in ["spelled", "reenter", "named"] {
        write(
            &root,
            &format!("{dir}/checks/one.js"),
            "test('t', () => {});\n",
        );
        let inputs = load(&root, &format!("{dir}/jest.config.js"));
        let d = &inputs.projects[0].discovery;
        // jest finds the test in this checkout.
        assert!(
            d.matcher()
                .rooted(&root)
                .is_test(&root.join(format!("{dir}/checks/one.js"))),
            "{dir}"
        );
        let (spec, fell_back) = ProjectSpec::of(d).expect("jest keeps a spec");
        let files = vec![format!("{dir}/checks/one.js")];
        let exact = spec.exact();
        let dependent = verdict(&spec, fell_back, &root, &Tree::new(&files)).dependent;
        // Either the spec says that the place decides (the path is the config's own
        // text, matched as written everywhere), or it is not kept exactly: in
        // both cases the root is recorded.
        assert!(
            !exact || dependent,
            "{dir}: exact {exact}, dependent {dependent}"
        );
    }
}
