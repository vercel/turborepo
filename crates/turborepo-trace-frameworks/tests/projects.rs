//! Lists of projects (jest `projects`, vitest `test.projects` and workspace
//! files): which paths a list names, and what a load of one asks of the file
//! system ([`turborepo_trace_frameworks::fsx::with_log`]).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_frameworks::{Framework, fsx, jest::Jest, vitest::Vitest};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

fn tmp() -> (tempfile::TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let r = t.path().canonicalize().unwrap();
    (t, r)
}

fn roots(inputs: &turborepo_trace_frameworks::FrameworkInputs, r: &Path) -> Vec<String> {
    inputs
        .projects
        .iter()
        .map(|p| {
            p.root_dir
                .strip_prefix(r)
                .unwrap_or(&p.root_dir)
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// `projects: ['<rootDir>', '<rootDir>/examples/*']`: the first entry is the
/// directory of the config itself. As a project the config is its own options
/// without the list (jest runs it so); a load that followed the list again
/// would never end.
#[test]
fn a_jest_config_that_lists_its_own_directory_is_a_project_of_its_own_options() {
    let (_t, r) = tmp();
    write(
        &r,
        "jest.config.js",
        "module.exports = { testMatch: ['**/*.own.js'], projects: ['<rootDir>', \
         '<rootDir>/examples/*'] };\n",
    );
    write(&r, "examples/one/package.json", "{}");
    write(&r, "examples/two/jest.config.js", "module.exports = {};\n");
    let inputs = Jest.load(&r.join("jest.config.js"));
    assert_eq!(roots(&inputs, &r), ["", "examples/one", "examples/two"]);
    // The project of the config itself has the config's options.
    let own = serde_json::to_string(&inputs.projects[0].discovery).unwrap();
    assert!(own.contains("*.own.js"), "{own}");
}

/// A glob that matches the config's own directory, in a config below the root.
#[test]
fn a_projects_glob_that_matches_its_own_config_ends() {
    let (_t, r) = tmp();
    write(
        &r,
        "packages/c/vitest.config.ts",
        "export default { test: { projects: ['../*'] } };\n",
    );
    write(&r, "packages/a/src/a.ts", "export {};\n");
    let inputs = Vitest.load(&r.join("packages/c/vitest.config.ts"));
    assert_eq!(roots(&inputs, &r), ["packages/a", "packages/c"]);
    write(
        &r,
        "packages/c/jest.config.js",
        "module.exports = { projects: ['<rootDir>/../*'] };\n",
    );
    let inputs = Jest.load(&r.join("packages/c/jest.config.js"));
    assert_eq!(roots(&inputs, &r), ["packages/a", "packages/c"]);
}

/// Two configs that list each other.
#[test]
fn configs_that_list_each_other_end() {
    let (_t, r) = tmp();
    write(
        &r,
        "a/jest.config.js",
        "module.exports = { projects: ['<rootDir>/../b'] };\n",
    );
    write(
        &r,
        "b/jest.config.js",
        "module.exports = { projects: ['<rootDir>/../a'] };\n",
    );
    let inputs = Jest.load(&r.join("a/jest.config.js"));
    assert_eq!(roots(&inputs, &r), ["a"]);
}

/// What the load of a config with a `projects` glob asks: the listing of the
/// glob's directory by the glob (nothing of what lies deeper), and every path
/// it looked at in the directories the glob matched, found or not.
#[test]
fn the_log_of_a_projects_glob() {
    let (_t, r) = tmp();
    write(
        &r,
        "jest.config.js",
        "module.exports = { projects: ['<rootDir>/packages/*'] };\n",
    );
    write(&r, "packages/a/jest.config.js", "module.exports = {};\n");
    write(&r, "packages/b/package.json", r#"{"name":"b"}"#);
    write(&r, "packages/b/src/deep/x.js", "");
    let (inputs, log) = fsx::with_log(|| Jest.load(&r.join("jest.config.js")));
    assert_eq!(roots(&inputs, &r), ["packages/a", "packages/b"]);
    let rel = |p: &Path| p.strip_prefix(&r).unwrap().to_string_lossy().into_owned();
    let listed: Vec<(String, &str)> = log
        .listed
        .iter()
        .map(|(d, p)| (rel(d), p.as_str()))
        .collect();
    // The glob, for directories and for files; the haste map of each project.
    assert!(listed.contains(&("packages".into(), "*")), "{listed:?}");
    assert!(
        listed.contains(&("packages/b".into(), "**/__mocks__/**")),
        "{listed:?}"
    );
    assert!(
        !listed.iter().any(|(_, p)| *p == "**"),
        "no listing of everything: {listed:?}"
    );
    let asked: Vec<String> = log.asked.iter().map(|p| rel(p)).collect();
    for p in [
        // Found: the directories, the config of one, the manifest of the other.
        "packages/a",
        "packages/a/jest.config.js",
        "packages/b",
        "packages/b/package.json",
        // Not found.
        "packages/b/jest.config.js",
        "packages/b/jest.config.ts",
    ] {
        assert!(asked.iter().any(|a| a == p), "{p} in {asked:?}");
    }
}

/// A config that is not read exactly: which discovery applies is asked of every
/// file below the roots, as a part of the load of its own, named by the
/// discovery.
#[test]
fn the_choice_of_a_discovery_is_a_part_of_the_log() {
    let (_t, r) = tmp();
    write(
        &r,
        "jest.config.js",
        "const shared = require(process.env.SHARED);\nmodule.exports = { testMatch: \
         ['**/*.spec.js'], verbose: shared.verbose };\n",
    );
    write(&r, "src/a.test.js", "");
    let load = || fsx::with_log(|| Jest.load(&r.join("jest.config.js")));
    let (inputs, log) = load();
    let choice = inputs.projects[0]
        .discovery
        .choice
        .as_deref()
        .expect("a choice");
    assert!(choice.fell_back, "no file matches the configured patterns");
    assert!(
        !log.listed.iter().any(|(_, p)| p == "**"),
        "the walks are no part of the load itself: {:?}",
        log.listed
    );
    let (_, part) = log
        .parts
        .iter()
        .find(|(name, _)| *name == choice.part)
        .expect("the part");
    assert!(
        part.listed.contains(&(r.clone(), "**".to_owned())),
        "{part:?}"
    );
    // The file is a test by the default patterns only: the choice decides for it.
    let m = inputs.projects[0].discovery.matcher();
    let other = choice.other.matcher();
    let t = r.join("src/a.test.js");
    assert!(m.is_test(&t) && !other.is_test(&t));
    assert!(choice.decides(m.is_test(&t), other.is_test(&t)));
    // A file the configured patterns match is there: they apply, and for that file
    // the choice decides nothing (it is itself a file they match).
    write(&r, "src/b.spec.js", "");
    let (inputs, _) = load();
    let choice = inputs.projects[0]
        .discovery
        .choice
        .as_deref()
        .expect("a choice");
    assert!(
        !choice.fell_back,
        "{}\n{}",
        serde_json::to_string(&inputs.projects[0].discovery).unwrap(),
        serde_json::to_string(&choice.other).unwrap()
    );
    let m = inputs.projects[0].discovery.matcher();
    let other = choice.other.matcher();
    let (spec, test) = (r.join("src/b.spec.js"), r.join("src/a.test.js"));
    assert!(m.is_test(&spec) && !m.is_test(&test));
    assert!(!choice.decides(m.is_test(&spec), other.is_test(&spec)));
    assert!(choice.decides(m.is_test(&test), other.is_test(&test)));
}

/// A config whose object is not known as a whole has a configured discovery
/// without roots: it can match no file, whatever appears, so the default
/// patterns always stand in and the choice decides for no file (a file the
/// default patterns match is itself the file that makes them apply).
#[test]
fn a_discovery_that_can_match_nothing_is_no_choice() {
    let (_t, r) = tmp();
    write(
        &r,
        "jest.config.js",
        "module.exports = { ...require(process.env.SHARED), testMatch: ['**/*.spec.js'] };\n",
    );
    write(&r, "src/a.test.js", "");
    write(&r, "src/b.spec.js", "");
    let inputs = Jest.load(&r.join("jest.config.js"));
    let d = &inputs.projects[0].discovery;
    let choice = d.choice.as_deref().expect("a choice");
    assert!(choice.fell_back && !choice.configured_can_match);
    let (m, other) = (d.matcher(), choice.other.matcher());
    for f in ["src/a.test.js", "src/b.spec.js", "src/c.js"] {
        let f = r.join(f);
        assert!(!choice.decides(m.is_test(&f), other.is_test(&f)), "{f:?}");
    }
}

/// A config that asks for the real path of a directory which is a link: the
/// link it crossed is in the log, and where it leads decides the mapper's
/// target.
#[cfg(unix)]
#[test]
fn the_log_has_the_links_a_real_path_crossed() {
    let (_t, r) = tmp();
    write(
        &r,
        "jest.config.js",
        "const path = require('path');\nmodule.exports = { moduleNameMapper: { '^@lib/(.*)$': \
         path.resolve(require('fs').realpathSync(path.resolve(__dirname, 'src/linked')), '$1') } \
         };\n",
    );
    write(&r, "src/real/x.js", "");
    turborepo_trace_core::os::symlink("real", r.join("src/linked")).unwrap();
    // Through a cache, as in a graph build: it knows the links of a real path.
    let fs = std::sync::Arc::new(turborepo_trace_core::DirCache::new());
    let (inputs, log) = fsx::with_fs(&fs, || {
        fsx::with_log(|| Jest.load(&r.join("jest.config.js")))
    });
    let p = &inputs.projects[0];
    assert!(p.uncertainties.is_empty(), "{:?}", p.uncertainties);
    assert_eq!(
        p.resolution.apply("@lib/x").expect("mapped").requests,
        [r.join("src/real/x").to_string_lossy().into_owned()]
    );
    assert_eq!(log.links, [r.join("src/linked")]);
}

/// A tree for the globs through links: `packages/y` is a directory,
/// `packages/x` a link to a directory outside `packages` that has a config of
/// its own and one two levels below.
#[cfg(unix)]
fn linked_tree(r: &Path, config: &str) {
    for dir in ["packages/y", "elsewhere/x", "elsewhere/x/deep/z"] {
        write(r, &format!("{dir}/{config}"), "module.exports = {};\n");
        write(r, &format!("{dir}/package.json"), "{}");
    }
    turborepo_trace_core::os::symlink("../elsewhere/x", r.join("packages/x")).unwrap();
}

/// What the runners do with a link to a directory on the way of a `projects`
/// glob, measured (jest 29.7 and 30.5, vitest 2.1 and 3.2, Python 3.14):
/// node-glob, which expands jest's list, enters a link that a plain segment
/// names and ends a `**` at one; tinyglobby and Python's `glob` follow links at
/// every depth; jest's file map enters none.
#[cfg(unix)]
#[test]
fn a_glob_goes_through_a_link_as_the_runner_s_library_does() {
    use fsx::Links;
    let (_t, r) = tmp();
    linked_tree(&r, "jest.config.js");
    let fs = std::sync::Arc::new(turborepo_trace_core::DirCache::new());
    let rel = |ps: Vec<PathBuf>| -> Vec<String> {
        let mut out: Vec<String> = ps
            .iter()
            .map(|p| p.strip_prefix(&r).unwrap().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    };
    let glob = |pattern: &str, dirs: bool, links: Links| {
        let abs = r.join(pattern);
        fsx::with_fs(&fs, || {
            rel(fsx::expand_glob(&abs.to_string_lossy(), dirs, links))
        })
    };
    // A plain segment names the link: entered by node-glob and by the libraries
    // that follow links, not by a walk that skips them.
    let files = "packages/*/jest.config.js";
    let both = ["packages/x/jest.config.js", "packages/y/jest.config.js"];
    assert_eq!(glob(files, false, Links::NodeGlob), both);
    assert_eq!(glob(files, false, Links::Follow), both);
    assert_eq!(
        glob(files, false, Links::Skip),
        ["packages/y/jest.config.js"]
    );
    // A `**`: node-glob matches the link and what the next segment names below it,
    // and does not go on below it; the others do.
    let deep = "pack*/**/jest.config.js";
    assert_eq!(glob(deep, false, Links::NodeGlob), both);
    assert_eq!(
        glob(deep, false, Links::Follow),
        [
            "packages/x/deep/z/jest.config.js",
            "packages/x/jest.config.js",
            "packages/y/jest.config.js"
        ]
    );
    assert_eq!(
        glob(deep, false, Links::Skip),
        ["packages/y/jest.config.js"]
    );
    // Directories: the link itself is an entry of the listing in every mode.
    for links in [Links::NodeGlob, Links::Follow, Links::Skip] {
        assert_eq!(
            glob("packages/*", true, links),
            ["packages/x", "packages/y"],
            "{links:?}"
        );
    }
    // Below the link with a `**`.
    assert_eq!(
        glob("packages/**/z", true, Links::Follow),
        ["packages/x/deep/z"]
    );
    assert!(glob("packages/**/z", true, Links::NodeGlob).is_empty());
    assert_eq!(
        glob("packages/*/deep/**", true, Links::NodeGlob),
        ["packages/x/deep/z"],
        "a plain segment entered the link: the `**` after it is below real directories"
    );
}

/// The log of a walk that went through a link: the link was asked about (where
/// it leads is no part of a listing), and every directory behind it is logged
/// by its own entries.
#[cfg(unix)]
#[test]
fn a_walk_through_a_link_logs_the_link_and_the_directories_behind_it() {
    let (_t, r) = tmp();
    linked_tree(&r, "jest.config.js");
    let fs = std::sync::Arc::new(turborepo_trace_core::DirCache::new());
    let pattern = r.join("pack*/**/jest.config.js");
    let (found, log) = fsx::with_fs(&fs, || {
        fsx::with_log(|| fsx::expand_glob(&pattern.to_string_lossy(), false, fsx::Links::Follow))
    });
    assert_eq!(found.len(), 3, "{found:?}");
    let rel = |p: &Path| p.strip_prefix(&r).unwrap().to_string_lossy().into_owned();
    let asked: Vec<String> = log.asked.iter().map(|p| rel(p)).collect();
    assert!(asked.iter().any(|a| a == "packages/x"), "{asked:?}");
    let listed: Vec<(String, &str)> = log
        .listed
        .iter()
        .map(|(d, p)| (rel(d), p.as_str()))
        .collect();
    // (By the real directories: a listing is kept for a directory of the tree.)
    for dir in ["elsewhere/x", "elsewhere/x/deep", "elsewhere/x/deep/z"] {
        assert!(listed.contains(&(dir.to_owned(), "*")), "{dir}: {listed:?}");
    }
    assert!(log.links.contains(&r.join("packages/x")), "{:?}", log.links);
    assert!(
        !listed.contains(&("packages/y".to_owned(), "*")),
        "a real directory is covered by the listing of the pattern: {listed:?}"
    );
}

/// jest: a config file behind a linked directory is a project of the list; the
/// linked directory itself, matched as a project directory, is none (jest drops
/// what `lstat` says is no directory and has no config file's extension).
#[cfg(unix)]
#[test]
fn a_linked_directory_in_a_jest_projects_glob() {
    let (_t, r) = tmp();
    linked_tree(&r, "jest.config.js");
    let load = |projects: &str| {
        write(
            &r,
            "jest.config.js",
            &format!("module.exports = {{ projects: ['<rootDir>/{projects}'] }};\n"),
        );
        let fs = std::sync::Arc::new(turborepo_trace_core::DirCache::new());
        fsx::with_fs(&fs, || roots(&Jest.load(&r.join("jest.config.js")), &r))
    };
    assert_eq!(
        load("packages/*/jest.config.js"),
        ["packages/x", "packages/y"]
    );
    assert_eq!(load("packages/*"), ["packages/y"]);
    assert_eq!(
        load("packages/**/jest.config.js"),
        ["packages/x", "packages/y"],
        "a `**` ends at the link"
    );
}

/// vitest: links are followed at every depth, and a linked directory that the
/// pattern names is taken as a project (vitest 2 does, 3.2 does not).
#[cfg(unix)]
#[test]
fn a_linked_directory_in_a_vitest_projects_glob() {
    let (_t, r) = tmp();
    linked_tree(&r, "vitest.config.js");
    let load = |projects: &str| {
        write(
            &r,
            "vitest.config.js",
            &format!("export default {{ test: {{ projects: ['{projects}'] }} }};\n"),
        );
        let fs = std::sync::Arc::new(turborepo_trace_core::DirCache::new());
        let mut found = fsx::with_fs(&fs, || roots(&Vitest.load(&r.join("vitest.config.js")), &r));
        found.sort();
        found
    };
    assert_eq!(
        load("packages/*/vitest.config.js"),
        ["packages/x", "packages/y"]
    );
    assert_eq!(load("packages/*"), ["packages/x", "packages/y"]);
    assert_eq!(
        load("packages/**/vitest.config.js"),
        ["packages/x", "packages/x/deep/z", "packages/y"]
    );
}
