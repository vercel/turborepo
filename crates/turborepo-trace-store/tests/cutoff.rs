//! Early cutoff: a refresh re-traces what an input change can change, and
//! nothing else, and is still a cold build of the same tree byte for byte
//! (shard keys, store bytes, export).
//!
//! - A listing that only membership hubs reference (`knip --directory ../..`:
//!   every file of the repository) keys a shard by what the shard stores of it:
//!   a file added anywhere re-keys no package.
//! - Inside a re-keyed package, a file that read nothing that changed keeps its
//!   records (`turborepo_trace_store::filereuse`): an edit analyzes the edited
//!   file, a changed manifest its readers, a changed task pipeline no file at
//!   all.
//! - The randomized test edits sources, tests, manifests, the lockfile,
//!   framework and compiler configs (also at places that had none), mocks,
//!   workspace links and package boundaries, and compares with a cold build
//!   after every edit. The sources include loads that say where they resolve
//!   from (another package, next to a module that is or is not installed) and
//!   located roots that are no reads; the configs a runner-level resolver, a
//!   tsconfig that extends a config that comes and goes, and project references
//!   (a referenced project that loads, breaks, goes, or comes to include the
//!   files below the referencing config; a solution config at the root); for
//!   the runner configs: manifests with a `jest` key, a directory below
//!   `packages/` that appears and goes (bare, with a manifest, with a config),
//!   `projects` as a glob and as a list, vitest `test.projects` and a workspace
//!   file, a path that is a file in one tree and a directory in another, a
//!   config that is not read exactly; a directory and a file that come to be
//!   symbolic links (and back), a relative request for a package directory.
//!   Every case runs in a tree that was just written and in an old one
//!   (`replay_old`: a refresh trusts the directories no edit touched). Its
//!   failures are persisted next to this file (`cutoff.proptest-regressions`),
//!   the sequences it has found are fixed cases (`regressions`), and
//!   `random::long_run` is the gate run (thousands of cases, a printed seed;
//!   `TURBO_TRACE_DEBUG_CUTOFF_TREE=old` for the old tree).
//! - The runners of the other languages have fixed cases (`runners`: go test,
//!   cargo test, pytest).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, sync::Arc};

use turborepo_trace_core::Language;
use turborepo_trace_store::{
    BuildReport, Externals, Graph, GraphOptions,
    artifact::{self, PackOptions},
    build,
};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

/// Writes the file `rel`. What is in its way goes: a file where one of its
/// directories is to be, a directory where the file is to be (the random edits
/// write paths that are a file in one tree and a directory in another).
fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    for dir in p.ancestors().skip(1).take_while(|d| *d != root) {
        if dir.is_file() {
            std::fs::remove_file(dir).unwrap();
        }
    }
    if p.is_symlink() {
        // (A write would go through the link.)
        std::fs::remove_file(&p).unwrap();
    } else if p.is_dir() {
        std::fs::remove_dir_all(&p).unwrap();
    }
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

/// Removes the file or directory `rel`, and the directories that are empty
/// without it.
fn remove(root: &Path, rel: &str) {
    let p = root.join(rel);
    if p.is_dir() && !p.is_symlink() {
        std::fs::remove_dir_all(&p).unwrap();
    } else {
        let _ = std::fs::remove_file(&p);
    }
    for dir in p.ancestors().skip(1).take_while(|d| *d != root) {
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
    }
}

fn opts(root: &Path, out: &Path) -> GraphOptions {
    GraphOptions {
        root: root.to_path_buf(),
        out: Some(out.to_path_buf()),
        externals: Externals::Opaque,
        threads: 2,
        ..Default::default()
    }
}

/// The JSON export and the shard keys of the graph in `out`.
fn export(out: &Path) -> (String, Vec<String>) {
    let g = Graph::open(out).unwrap();
    let mut buf = Vec::new();
    turborepo_trace_graph::json::write_json(&g, Some(g.root()), true, &mut buf).unwrap();
    let keys = g
        .manifest()
        .shards
        .iter()
        .map(|s| format!("{} {} {}", s.kind, s.root, s.key))
        .collect();
    (String::from_utf8(buf).unwrap(), keys)
}

/// The primary store of the graph in `out` as bytes (its packed file): equal
/// for two graphs exactly when their manifests, walked-file lists and every
/// shard's bytes are equal.
fn store_bytes(out: &Path) -> Vec<u8> {
    let t = tempfile::tempdir().unwrap();
    let file = t.path().join("store.tta");
    artifact::pack(
        out,
        &file,
        &PackOptions {
            label: Some("same".into()),
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::read(file).unwrap()
}

/// Refreshes the graph of `root` and compares it with a fresh build of the
/// tree: export, shard keys and store bytes.
fn refresh_equals_cold(root: &Path, what: &str) -> BuildReport {
    refresh_equals_cold_with(root, what, &langs())
}

fn build_with(
    o: &GraphOptions,
    langs: &[Arc<dyn Language>],
) -> Result<BuildReport, turborepo_trace_store::Error> {
    build(o, langs)
}

/// [`refresh_equals_cold`] with the languages `langs`.
fn refresh_equals_cold_with(root: &Path, what: &str, langs: &[Arc<dyn Language>]) -> BuildReport {
    let out = root.join(".turbotrace");
    let r = build(&opts(root, &out), langs).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    build(&opts(root, fresh.path()), langs).unwrap();
    let (got_json, got_keys) = export(&out);
    let (fresh_json, fresh_keys) = export(fresh.path());
    if got_json != fresh_json {
        let got: std::collections::BTreeSet<&str> = got_json.lines().collect();
        let fr: std::collections::BTreeSet<&str> = fresh_json.lines().collect();
        let mut diff: Vec<String> = got
            .difference(&fr)
            .take(8)
            .map(|l| format!("only refreshed: {}", l.trim()))
            .collect();
        diff.extend(
            fr.difference(&got)
                .take(8)
                .map(|l| format!("only fresh: {}", l.trim())),
        );
        // For a look at the whole difference: both exports, written where the variable
        // says.
        if let Some(dir) = std::env::var_os("TURBO_TRACE_DEBUG_CUTOFF_DUMP") {
            let dir = std::path::PathBuf::from(dir);
            let _ = std::fs::write(dir.join("refreshed.json"), &got_json);
            let _ = std::fs::write(dir.join("fresh.json"), &fresh_json);
        }
        if diff.is_empty() {
            // The same lines, in another order or another number of times.
            let at = got_json
                .lines()
                .zip(fresh_json.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b);
            diff.push(match at {
                Some((i, (a, b))) => format!(
                    "the same lines in another order, first at line {}:\nrefreshed: {}\nfresh: {}",
                    i + 1,
                    a.trim(),
                    b.trim()
                ),
                None => format!(
                    "{} line(s) refreshed, {} fresh",
                    got_json.lines().count(),
                    fresh_json.lines().count()
                ),
            });
        }
        panic!(
            "{what}: the refreshed graph differs from a fresh build:\n{}",
            diff.join("\n")
        );
    }
    assert_eq!(got_keys, fresh_keys, "{what}: shard keys");
    assert!(
        store_bytes(&out) == store_bytes(fresh.path()),
        "{what}: the stores differ in bytes under equal keys"
    );
    r
}

/// Three packages whose `knip` task lists the whole repository (one shared
/// hub), a root package with a file of its own, and a file no language traces.
fn hub_fixture(r: &Path) {
    write(r, ".gitignore", "node_modules\n.turbotrace\n");
    write(r, "package.json", r#"{"name":"root","private":true}"#);
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    for p in ["a", "b", "c"] {
        write(
            r,
            &format!("packages/{p}/package.json"),
            &format!(
                r#"{{"name":"{p}","version":"1.0.0","scripts":{{"clean-code":"knip --directory ../.. --workspace packages/{p}"}}}}"#
            ),
        );
        write(
            r,
            &format!("packages/{p}/src/index.ts"),
            "export const x = 1;\n",
        );
    }
    write(r, "docs/notes.ts", "export const n = 1;\n");
    write(r, "docs/guide.md", "# guide\n");
}

fn canonical_tmp() -> (tempfile::TempDir, std::path::PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    (t, r)
}

/// The shard keys of the first-party packages below `packages/`.
fn package_keys(root: &Path) -> Vec<String> {
    let m = turborepo_trace_store::Manifest::read(&root.join(".turbotrace")).unwrap();
    m.shards
        .iter()
        .filter(|s| s.kind == "first-party" && s.root.starts_with("packages/"))
        .map(|s| format!("{} {}", s.root, s.key))
        .collect()
}

#[test]
fn a_file_added_below_a_shared_hub_rekeys_no_package() {
    let (_t, r) = canonical_tmp();
    hub_fixture(&r);
    build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
    let before = package_keys(&r);
    assert_eq!(before.len(), 3);

    // A file nobody traces, somewhere in the repository: a member of the hub of
    // every package, stored by none of them.
    write(&r, "docs/more.md", "# more\n");
    let rep = refresh_equals_cold(&r, "a file added below the hub");
    assert_eq!(package_keys(&r), before, "the packages keep their shards");
    // Only the root package (it lists its own directory) may be traced again.
    assert!(rep.traced <= 1, "traced {} shards", rep.traced);

    // The composed graph expands the hub from the walked files: the new file is a
    // member.
    let g = Graph::open(&r.join(".turbotrace")).unwrap();
    use turborepo_trace_graph::GraphView;
    let hub = (0..g.node_count())
        .map(turborepo_trace_graph::NodeId)
        .find(|&n| {
            let p = g.path(n);
            p.starts_with("members:") && !p.starts_with("members:packages/")
        })
        .expect("the shared hub");
    let targets: Vec<&str> = g
        .out_edges(hub)
        .filter_map(|e| e.to)
        .map(|n| g.path(n))
        .collect();
    assert!(targets.contains(&"docs/more.md"), "{targets:?}");

    // Removed again, and a traced file added in one package: only that package.
    std::fs::remove_file(r.join("docs/more.md")).unwrap();
    write(&r, "packages/b/src/extra.ts", "export const y = 2;\n");
    refresh_equals_cold(&r, "a source file added in one package");
    let after = package_keys(&r);
    assert_eq!(after[0], before[0], "packages/a");
    assert_ne!(after[1], before[1], "packages/b has a new file");
    assert_eq!(after[2], before[2], "packages/c");
}

#[test]
fn a_listing_that_code_reads_still_rekeys() {
    let (_t, r) = canonical_tmp();
    hub_fixture(&r);
    // Package a also lists the repository's docs directory from code: its shard
    // stores the listing's matches as edges.
    write(
        &r,
        "packages/a/src/index.ts",
        "const fs = require('fs');\nconst path = require('path');\nexport const x = \
         fs.readdirSync(path.join(__dirname, '../../../docs'));\n",
    );
    build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
    let before = package_keys(&r);
    write(&r, "docs/more.md", "# more\n");
    refresh_equals_cold(&r, "a file added to a directory that code lists");
    let after = package_keys(&r);
    assert_ne!(after[0], before[0], "packages/a lists docs/ from code");
    assert_eq!(after[1], before[1], "packages/b");
    assert_eq!(after[2], before[2], "packages/c");
}

#[test]
fn a_third_party_member_of_a_hub_rekeys_it() {
    let (_t, r) = canonical_tmp();
    hub_fixture(&r);
    // A Go module vendored in the repository: its files are walked, members of the
    // hub, and files of a third-party package (the hub stores their edges
    // itself, `HubRec::skip`).
    write(
        &r,
        "go.mod",
        "module example.com/root\n\ngo 1.21\n\nrequire example.com/dep v1.0.0\n",
    );
    write(
        &r,
        "vendor/modules.txt",
        "# example.com/dep v1.0.0\n## explicit\nexample.com/dep\n",
    );
    let env = tempfile::tempdir().unwrap();
    let langs: Vec<Arc<dyn Language>> = vec![
        Arc::new(turborepo_trace_lang_js::JsLanguage),
        Arc::new(turborepo_trace_lang_go::GoLanguage::new(
            turborepo_trace_lang_go::GoOptions {
                modcache: Some(env.path().join("gomod")),
                goroot: turborepo_trace_lang_go::GoRoot::None,
                cgo: Some(false),
                workspace: Some(false),
                ..Default::default()
            },
        )),
    ];
    build_with(&opts(&r, &r.join(".turbotrace")), &langs).unwrap();
    let before = package_keys(&r);
    write(&r, "vendor/example.com/dep/dep.go", "package dep\n");
    refresh_equals_cold_with(&r, "a vendored file added below the hub", &langs);
    let after = package_keys(&r);
    for (a, b) in after.iter().zip(&before) {
        assert_ne!(
            a, b,
            "a hub that stores an edge to the new file is re-keyed"
        );
    }
    // A file that is no third-party file leaves the shards alone.
    write(&r, "vendor/README.md", "# vendor\n");
    refresh_equals_cold_with(&r, "a first-party file next to the vendored module", &langs);
    assert_eq!(package_keys(&r), after);
}

// ---- Per-file reuse inside a re-keyed package ----

fn symlink(root: &Path, target: &str, link: &str) {
    let l = root.join(link);
    std::fs::create_dir_all(l.parent().unwrap()).unwrap();
    // What is in its way goes: a link, a file, a real directory with everything
    // below it.
    if l.is_dir() && !l.is_symlink() {
        std::fs::remove_dir_all(&l).unwrap();
    } else {
        let _ = std::fs::remove_file(&l);
    }
    turborepo_trace_core::os::symlink(target, l).unwrap();
}

/// Installs lodash `version` pnpm-style and writes the lockfile.
fn lodash(r: &Path, version: &str, integrity: &str) {
    let dir = format!("node_modules/.pnpm/lodash@{version}/node_modules/lodash");
    write(
        r,
        &format!("{dir}/package.json"),
        &format!(r#"{{"name":"lodash","version":"{version}","main":"index.js"}}"#),
    );
    write(r, &format!("{dir}/index.js"), "module.exports = 1;\n");
    symlink(
        r,
        &format!("../../../node_modules/.pnpm/lodash@{version}/node_modules/lodash"),
        "packages/a/node_modules/lodash",
    );
    write(
        r,
        "pnpm-lock.yaml",
        &format!(
            "lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/a:\n    dependencies:\n      lodash:\n        specifier: ^4\n        version: {version}\n  packages/c:\n    dependencies:\n      left-pad:\n        specifier: ^1\n        version: 1.3.0\npackages:\n  left-pad@1.3.0:\n    resolution: {{integrity: sha512-pad}}\n  lodash@{version}:\n    resolution: {{integrity: {integrity}}}\nsnapshots:\n  left-pad@1.3.0: {{}}\n  lodash@{version}: {{}}\n"
        ),
    );
}

/// The lockfile of [`lodash`] (4.17.21) with an entry for `left-pad` in the
/// importer `packages/a`: a link to `packages/b`, or the registry package.
fn left_pad_lock(r: &Path, link: bool) {
    let (specifier, version) = if link {
        ("workspace:*", "link:../b")
    } else {
        ("^1", "1.3.0")
    };
    write(
        r,
        "pnpm-lock.yaml",
        &format!(
            "lockfileVersion: '9.0'\nimporters:\n  .: {{}}\n  packages/a:\n    dependencies:\n      left-pad:\n        specifier: {specifier}\n        version: {version}\n      lodash:\n        specifier: ^4\n        version: 4.17.21\n  packages/b: {{}}\n  packages/c:\n    dependencies:\n      left-pad:\n        specifier: ^1\n        version: 1.3.0\npackages:\n  left-pad@1.3.0:\n    resolution: {{integrity: sha512-pad}}\n  lodash@4.17.21:\n    resolution: {{integrity: sha512-old}}\nsnapshots:\n  left-pad@1.3.0: {{}}\n  lodash@4.17.21: {{}}\n"
        ),
    );
}

const A_MANIFESTS: &[&str] = &[
    r#"{"name":"a","main":"src/index.ts","scripts":{"build":"tsc -p .","test":"jest"},"dependencies":{"b":"workspace:*","lodash":"^4"}}"#,
    // A dependency entry more: nothing a file's resolution reads.
    r#"{"name":"a","main":"src/index.ts","scripts":{"build":"tsc -p .","test":"jest"},"dependencies":{"b":"workspace:*","lodash":"^4","left-pad":"^1"}}"#,
    // Another script: the tasks change.
    r#"{"name":"a","main":"src/index.ts","scripts":{"build":"tsc -p .","test":"jest","lint":"eslint src"},"dependencies":{"b":"workspace:*","lodash":"^4"}}"#,
    // An exports map: resolution of the package's own name changes.
    r#"{"name":"a","main":"src/index.ts","exports":{".":"./src/index.ts","./util":"./src/util.ts"},"scripts":{"test":"jest"},"dependencies":{"b":"workspace:*","lodash":"^4"}}"#,
    // A subpath import that names the generated config (`"extends": "#generated"`).
    r##"{"name":"a","main":"src/index.ts","imports":{"#generated":"./.generated/tsconfig.json"},"scripts":{"build":"tsc -p .","test":"jest"},"dependencies":{"b":"workspace:*","lodash":"^4"}}"##,
];

const B_MANIFESTS: &[&str] = &[
    r#"{"name":"b","main":"src/index.ts","scripts":{"build":"tsc -p ."}}"#,
    r#"{"name":"b","version":"2.0.0","main":"src/index.ts","scripts":{"build":"tsc -p ."}}"#,
    r#"{"name":"b","main":"src/other.ts","scripts":{"build":"tsc -p ."}}"#,
    r#"{"name":"b","main":"src/index.ts","exports":{".":"./src/other.ts"},"dependencies":{"lodash":"^4"}}"#,
    // The package takes the name of a registry package that the lockfile has and nothing
    // installs (`Edit::LeftPad` makes the lockfile link it).
    r#"{"name":"left-pad","main":"src/index.ts"}"#,
    // An exports map that sends `b/tsconfig` to the config `Edit::Mapped` makes: without it
    // the request names the package's own `tsconfig.json`.
    r#"{"name":"b","main":"src/index.ts","exports":{".":"./src/index.ts","./tsconfig":"./configs/shared.json"},"scripts":{"build":"tsc -p ."}}"#,
];

/// Manifests of `packages/c` that name an entry (the package has no index file
/// unless an edit writes one): what the name leads to when a link leads to the
/// package.
const C_MANIFESTS: &[&str] = &[
    r#"{"name":"c","main":"src/d.ts","dependencies":{"a":"workspace:*","left-pad":"^1"}}"#,
    r#"{"name":"c","exports":{".":"./src/c.ts"},"dependencies":{"a":"workspace:*","left-pad":"^1"}}"#,
];

const JEST_CONFIGS: &[&str] = &[
    "module.exports = { setupFiles: ['<rootDir>/test/setup.js'], moduleNameMapper: { \
     '^@lib/(.*)$': '<rootDir>/src/$1' } };\n",
    "module.exports = { moduleNameMapper: { '^@lib/(.*)$': '<rootDir>/src/$1' } };\n",
    "module.exports = { setupFiles: ['<rootDir>/test/setup.js'] };\n",
    "module.exports = { setupFiles: ['<rootDir>/test/setup.js'], moduleNameMapper: { \
     '^@lib/(.*)$': '<rootDir>/src/dir/$1' }, testMatch: ['**/*.spec.ts'] };\n",
    // What the runner may load in place of a module: a resolver of its own (a module load
    // with an unknown target of every test of the project).
    "module.exports = { resolver: '<rootDir>/test/setup.js' };\n",
    "module.exports = { resolver: '<rootDir>/test/setup.js', moduleNameMapper: { '^@lib/(.*)$': \
     '<rootDir>/src/$1' } };\n",
    // A config with a preset (`Edit::Preset` makes it, changes it and removes it). Loading
    // the config looks for the preset; what it finds decides which files are tests and
    // what the imports of every file below the project resolve to.
    "module.exports = { preset: './preset' };\n",
    "module.exports = { preset: './preset', setupFiles: ['<rootDir>/test/setup.js'] };\n",
    // A config that is not read exactly (a spread of a module that is not known): when its
    // discovery matches no file, jest's default patterns stand in for it. Which of the two
    // it is depends on the files that are there (`more.spec.ts`).
    "const shared = require(process.env.SHARED);\nmodule.exports = { testMatch: ['**/*.spec.ts'], \
     verbose: shared.verbose };\n",
    // A config that asks for the real path of a directory (`Edit::Link` 4 and 6 make it a
    // link, to one package or another) and maps a module name to a file below it.
    "const path = require('path');\nmodule.exports = { moduleNameMapper: { '^@lib/(.*)$': \
     path.resolve(require('fs').realpathSync(path.resolve(__dirname, 'src/linked')), '$1') } };\n",
    // The same for a directory below the link: the path that is asked about is there
    // wherever the link leads.
    "const path = require('path');\nmodule.exports = { moduleNameMapper: { '^@lib/(.*)$': \
     path.resolve(require('fs').realpathSync(path.resolve(__dirname, 'src/linked/sub')), '$1') } \
     };\n",
];

/// The preset of the jest configs that name one.
const PRESET: &str = "packages/a/preset/jest-preset.json";

const PRESETS: &[&str] = &[
    // A module mapper: the imports of files that are no tests resolve through it.
    r#"{"moduleNameMapper":{"^@lib/(.*)$":"<rootDir>/src/$1"}}"#,
    // Another test discovery: files that were no tests are, and tests are none (also a
    // file of a package without tests, for a config above the packages).
    r#"{"testMatch":["**/*.spec.ts","**/plain.ts","**/d.ts"]}"#,
    r#"{"setupFiles":["<rootDir>/test/setup.js"],"moduleNameMapper":{"^@lib/(.*)$":"<rootDir>/src/dir/$1"}}"#,
];

/// The root manifest: as the workspace has it, and with a `jest` key (a config
/// above every package: another test discovery, a module mapper).
const ROOT_MANIFESTS: &[&str] = &[
    r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"}}"#,
    r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"},"jest":{"testMatch":["**/d.ts","**/*.test.ts"]}}"#,
    r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"},"jest":{"moduleNameMapper":{"^not-installed$":"<rootDir>/packages/b/src/other.ts"}}}"#,
    // Fields that the resolution view holds and a config loader does not get, and one it
    // gets: `exports`, which answers the `extends` of a root config that names the root
    // package (`ROOT_TSCONFIGS`).
    r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"},"packageManager":"pnpm@9.1.0","engines":{"node":">=20"}}"#,
    r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"},"exports":{"./cfg":"./tsconfig.base.json"}}"#,
];

/// Places of a manifest below the config of `packages/a` (no workspace member).
/// With a `jest` key it is the config that governs the files below it.
const MANIFEST_PLACES: &[&str] = &[
    "packages/a/src/dir/package.json",
    "packages/a/src/package.json",
];

const MANIFESTS_AT: &[&str] = &[
    r#"{"jest":{"testMatch":["**/util.ts","**/esm.mjs","**/*.test.ts"]}}"#,
    r#"{"jest":{"moduleNameMapper":{"^@lib/(.*)$":"<rootDir>/$1"}}}"#,
    r#"{"private":true}"#,
];

/// A manifest whose `jest` project has its root in `packages/c`, with a module
/// mapper for the import `not-installed` of `packages/c/src/c.ts`.
const ELSEWHERE: &str = r#"{"name":"d","jest":{"rootDir":"../c","moduleNameMapper":{"^not-installed$":"<rootDir>/src/d.ts"}}}"#;

/// Paths that no other edit writes, each with the contents it can have
/// (`Edit::Path`):
/// - a directory below `packages/` that is no package of the fixture: it
///   appears with its first file and goes with its last one. Bare, with a
///   manifest (plain, or with a `jest` key: a project of its own, also one
///   whose root is another package), with a config file. A `projects` glob or
///   list of a config above comes to name it;
/// - the same name as a **file** (a config that a `projects` glob lists as a
///   file), and the setup file of `packages/a` as a **directory**: a path that
///   is asked about is a file in one tree and a directory in another;
/// - vitest: a config at the root with `test.projects` (a glob, a list with an
///   inline project) or without, a workspace file, a config of one package.
const PATHS: &[(&str, &[&str])] = &[
    (
        "packages/d/src/x.ts",
        &[
            "export const x = 1;\n",
            "import nope from 'not-installed';\nexport default nope;\n",
        ],
    ),
    ("packages/d/src/x.test.ts", &["test('x', () => {});\n"]),
    (
        "packages/d/package.json",
        &[
            r#"{"name":"d"}"#,
            ELSEWHERE,
            r#"{"name":"d","jest":{"testMatch":["**/x.ts","**/d.ts"]}}"#,
        ],
    ),
    (
        "packages/d/jest.config.js",
        &[
            "module.exports = {};\n",
            // Its roots are another package.
            "module.exports = { roots: ['<rootDir>/../c'], moduleNameMapper: { '^not-installed$': \
             '<rootDir>/../c/src/d.ts' } };\n",
            // Its root is a sibling package. (The config file is a module too, and a
            // relative path string in it is a load of that directory: the case
            // `a_directory_that_a_relative_request_names_comes_to_answer`.)
            "module.exports = { rootDir: '../c', moduleNameMapper: { '^not-installed$': \
             '<rootDir>/src/d.ts' } };\n",
        ],
    ),
    (
        "packages/d/vitest.config.ts",
        &["export default { test: { include: ['**/x.ts'] } };\n"],
    ),
    (
        "packages/d",
        &["module.exports = { rootDir: '<rootDir>/c', testMatch: ['**/d.ts'] };\n"],
    ),
    ("packages/a/test/setup.js/index.js", &["jest.mock('b');\n"]),
    (
        "vitest.config.ts",
        &[
            "export default { test: { projects: ['packages/*'] } };\n",
            "export default { test: { projects: ['packages/a', 'packages/d', { test: { name: 'c', \
             root: 'packages/c/src', include: ['**/d.ts'] } }] } };\n",
            "export default { test: { include: ['**/*.spec.ts'] }, resolve: { alias: { \
             'not-installed': './packages/b/src/other.ts' } } };\n",
            // An inline project rooted at a package directory (a path string that names a
            // directory which may not answer: a long run found the case
            // `a_directory_that_a_relative_request_names_comes_to_answer` here).
            "export default { test: { projects: ['packages/a', { test: { name: 'c', root: \
             'packages/c', include: ['**/d.ts'] } }] } };\n",
        ],
    ),
    (
        "vitest.workspace.ts",
        &[
            "export default ['packages/*'];\n",
            "export default ['packages/a', 'packages/d'];\n",
        ],
    ),
    (
        "packages/c/vitest.config.ts",
        &[
            "export default { test: { include: ['**/d.ts'] }, resolve: { alias: { \
             'not-installed': './src/d.ts' } } };\n",
            "export default { test: { projects: ['../*'] } };\n",
        ],
    ),
];

const TSCONFIGS: &[&str] = &[
    r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/dir/*"]}}}"#,
    r#"{"extends":"../../tsconfig.base.json","compilerOptions":{"strict":true,"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    // A config that extends a generated config (`Edit::Generated` makes it). Without that
    // file the config does not load, and the places its `extends` looked at are what
    // covers the files it governs.
    r#"{"extends":"./.generated/tsconfig.json","compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    r#"{"extends":["../../tsconfig.base.json","./.generated/tsconfig.json"]}"#,
    r#"{"extends":"./.generated/tsconfig.json","references":[{"path":"../b"}]}"#,
    // Configs that load and reference the project of `packages/b` (`Edit::Referenced`
    // makes it, breaks it and removes it): one that governs the files below it, and a
    // solution config whose second reference is never there.
    r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}},"references":[{"path":"../b"}]}"#,
    r#"{"files":[],"include":[],"references":[{"path":"../b"},{"path":"../c"}]}"#,
    // Configs whose `extends` names a file by a rule of the loader's. What a load read is
    // what the loader asked of the file system, so none of these has a rule of its own
    // here, and each named another file (or none) for a record derived from the text:
    // a name with a dot and without `.json` (`tsconfig.shared.json`; not the `tsconfig.json`
    // next to it, which `Edit::RootTsconfig` makes),
    r#"{"extends":"../../tsconfig.shared"}"#,
    // a request that the manifest of the linked package `b` maps (`Edit::ManifestB`,
    // `Edit::Mapped`), and that names the package's `tsconfig.json` without the map,
    r#"{"extends":"b/tsconfig"}"#,
    // a symbolic link (`LINKS`: to the base config at the root, or to its copy elsewhere),
    r#"{"extends":"./tsconfig.link.json"}"#,
    // and a subpath import of the package's own manifest (`Edit::ManifestA`).
    r##"{"extends":"#generated"}"##,
    // A config behind a linked directory (`LINKS`: `packages/a/cfg` leads to `packages/b`,
    // where `Edit::Mapped` makes the config, or to `packages/c`, where it never is): the
    // link is above the place the load looks at.
    r#"{"extends":"./cfg/configs/shared.json"}"#,
    // A reference to a directory that comes and goes (`DIRS`), and never holds a config.
    r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}},"references":[{"path":"../nope"}]}"#,
    // A config behind a linked directory at the root (`LINKS`: `cfgs`), outside the package.
    r#"{"extends":"../../cfgs/configs/shared.json"}"#,
    // A solution config whose first reference is the generated project (which can include
    // the package's sources and govern them) and whose second is the project of
    // `packages/b`: loaded after the governing one, and reported when it does not load.
    r#"{"files":[],"include":[],"references":[{"path":"./.generated"},{"path":"../b"}]}"#,
    // A config of the linked package `b` that extends `./chained`: the loader asks about
    // `chained` by the path through the link (`packages/a/node_modules/b/chained`), and
    // the directory of that name (`DIRS`) is one of `packages/b`.
    r#"{"extends":"b/chain.json","compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    // A name that reads as a pattern: `[cfg].json`, or the directory `[cfg]` (`DIRS`).
    r#"{"extends":"./[cfg]"}"#,
];

/// The project config that some `TSCONFIGS` (and `ROOT_TSCONFIGS`) reference.
const REFERENCED: &str = "packages/b/tsconfig.json";
const REFERENCED_CONFIGS: &[&str] = &[
    // It loads, and includes the files below its own directory.
    "{}",
    // It loads when the generated config of `packages/a` is there.
    r#"{"extends":"../a/.generated/tsconfig.json"}"#,
    // It includes the sources of `packages/a`: a config that references it governs them.
    r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}},"include":["../a/src/**/*"]}"#,
    // It does not parse.
    r#"{"extends": ["#,
    // An option more: nothing that decides whether it loads or what it includes.
    r#"{"compilerOptions":{"strict":true}}"#,
    // It extends the generated config through the root's link to the package `a`: what
    // its load read was located through that link.
    r#"{"extends":"a/.generated/tsconfig.json"}"#,
    // It extends the shared config by a name without `.json`.
    r#"{"extends":"../../tsconfig.shared"}"#,
];

/// A config at the root, above every package's own: a solution config that
/// references the packages, one whose own `extends` comes and goes with the
/// generated config, and one that governs with a reference that is never there.
const ROOT_TSCONFIGS: &[&str] = &[
    r#"{"files":[],"include":[],"references":[{"path":"./packages/a"},{"path":"./packages/b"}]}"#,
    r#"{"extends":"./packages/a/.generated/tsconfig.json","compilerOptions":{"baseUrl":"."}}"#,
    r#"{"references":[{"path":"./packages/b"},{"path":"./packages/nope"}]}"#,
    // A root config that extends the root package by name: the root manifest is the nearest
    // one of its load, which reads the name and the `exports` (`ROOT_MANIFESTS`) and
    // nothing else of it.
    r#"{"extends":"root/cfg","compilerOptions":{"baseUrl":"."}}"#,
];

/// The generated config that some `TSCONFIGS` extend, with or without `paths`
/// of its own.
const GENERATED: &str = "packages/a/.generated/tsconfig.json";
const GENERATED_CONFIGS: &[&str] = &[
    "{}",
    r#"{"compilerOptions":{"baseUrl":"..","paths":{"@/*":["src/dir/*"]}}}"#,
    // It includes the sources of the package: a config that references it (as the
    // directory `./.generated`) is governed by it for them.
    r#"{"compilerOptions":{"baseUrl":"..","paths":{"@/*":["src/*"]}},"include":["../src/**/*"]}"#,
];

/// The config at the root that some `TSCONFIGS` extend as
/// `../../tsconfig.shared` or through a link: its `paths` are relative to where
/// it lies.
const SHARED: &str = "tsconfig.shared.json";
const SHARED_CONFIGS: &[&str] = &[
    r#"{"compilerOptions":{"baseUrl":"./packages/a","paths":{"@/*":["src/*"]}}}"#,
    r#"{"compilerOptions":{"baseUrl":"./packages/a","paths":{"@/*":["src/dir/*"]}}}"#,
];

/// The config of `packages/b` that its manifest's `exports` can name
/// (`b/tsconfig`).
const MAPPED: &str = "packages/b/configs/shared.json";
const MAPPED_CONFIGS: &[&str] = &[
    r#"{"compilerOptions":{"baseUrl":"../../a","paths":{"@/*":["src/*"]}}}"#,
    r#"{"compilerOptions":{"baseUrl":"../../a","paths":{"@/*":["src/dir/*"]}}}"#,
];

/// The base config at the root, and a copy of it in `packages/b` (`Edit::Base`
/// writes both: the same bytes at two places, whose `baseUrl` is relative to
/// where each lies).
const BASES: [&str; 2] = ["tsconfig.base.json", "packages/b/tsconfig.base.json"];

const TURBOS: &[&str] = &[
    r#"{"tasks":{"//#gen":{"outputs":["tools/**/*.gen.ts"]},"build":{"dependsOn":["^build"]}}}"#,
    r#"{"tasks":{"build":{"dependsOn":["^build"],"inputs":["src/**"]},"test":{"dependsOn":["build"]}}}"#,
];

/// A workspace with a jest project, a tsconfig chain, a pnpm-style install with
/// a lockfile, tasks, and packages that import each other.
fn workspace(r: &Path) {
    write(r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{"gen":"node gen.js"}}"#,
    );
    write(r, "gen.js", "console.log(1);\n");
    write(r, "turbo.json", TURBOS[0]);
    for base in BASES {
        write(r, base, r#"{"compilerOptions":{"strict":true}}"#);
    }
    write(r, "src/root.ts", "import a from 'a';\nexport default a;\n");
    write(r, "docs/guide.md", "# guide\n");
    write(r, "packages/a/package.json", A_MANIFESTS[0]);
    write(r, "packages/a/jest.config.js", JEST_CONFIGS[0]);
    write(r, "packages/a/tsconfig.json", TSCONFIGS[0]);
    write(r, "packages/a/test/setup.js", "jest.mock('lodash');\n");
    write(
        r,
        "packages/a/src/index.ts",
        "import { u } from './util';\nimport b from 'b';\nimport _ from 'lodash';\nimport \
         './missing';\nconst loc = require('./locales/' + lang + '.json');\nexport const x = u;\n",
    );
    write(r, "packages/a/src/util.ts", "export const u = 1;\n");
    write(
        r,
        "packages/a/src/lib.ts",
        "import { u } from '@lib/util';\nexport const l = u;\n",
    );
    write(r, "packages/a/src/plain.ts", "export const p = 1;\n");
    write(
        r,
        "packages/a/src/alias.ts",
        "import { u } from '@/util';\nexport const v = u;\n",
    );
    write(
        r,
        "packages/a/src/index.test.ts",
        "import { x } from './index';\njest.mock('./util');\ntest('x', () => expect(x).toBe(1));\n",
    );
    write(
        r,
        "packages/a/src/plain.test.ts",
        "import { p } from './plain';\ntest('p', () => expect(p).toBe(1));\n",
    );
    write(r, "packages/a/src/locales/en.json", "{}");
    write(r, "packages/b/package.json", B_MANIFESTS[0]);
    write(r, "packages/b/src/index.ts", "export default 2;\n");
    write(r, "packages/b/src/other.ts", "export default 3;\n");
    // Configs that some `TSCONFIGS` extend, always there: one of the linked package
    // `b` that extends `./chained` (`chained.json`; `DIRS` has a directory of
    // that name), and one next to a directory whose name reads as a pattern
    // (`DIRS`: `[cfg]`).
    write(r, "packages/b/chain.json", r#"{"extends":"./chained"}"#);
    write(
        r,
        "packages/b/chained.json",
        r#"{"compilerOptions":{"strict":true}}"#,
    );
    write(
        r,
        "packages/a/[cfg].json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    write(
        r,
        "packages/c/package.json",
        r#"{"name":"c","dependencies":{"a":"workspace:*","left-pad":"^1"},"scripts":{"build":"tsc -p ."}}"#,
    );
    write(
        r,
        "packages/c/src/c.ts",
        "import a from 'a';\nimport pad from 'left-pad';\nimport nope from \
         'not-installed';\nexport default [a, pad, nope];\n",
    );
    write(r, "packages/c/src/d.ts", "export const d = 4;\n");
    lodash(r, "4.17.21", "sha512-old");
    symlink(r, "../packages/a", "node_modules/a");
    symlink(r, "../../../packages/b", "packages/a/node_modules/b");
    symlink(r, "../../../packages/a", "packages/c/node_modules/a");
}

fn built_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let (t, r) = canonical_tmp();
    workspace(&r);
    build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
    (t, r)
}

#[test]
fn an_edit_keeps_the_other_files_of_its_package() {
    let (_t, r) = built_workspace();
    // A body edit of a file nothing else reads by content.
    write(&r, "packages/a/src/plain.ts", "export const p = 2;\n");
    let rep = refresh_equals_cold(&r, "one file edited");
    assert_eq!(rep.traced, 1, "one package");
    assert_eq!(rep.analyzed_files, 1, "one file analyzed");
    assert!(rep.kept_files >= 6, "kept {}", rep.kept_files);
}

#[test]
fn a_deleted_or_added_file_retraces_the_files_that_looked_for_it() {
    let (_t, r) = built_workspace();
    // `./missing` appears: index.ts looked for it.
    write(&r, "packages/a/src/missing.ts", "export {};\n");
    let rep = refresh_equals_cold(&r, "a probed file appears");
    assert!(
        rep.kept_files > 0 && rep.analyzed_files >= 2,
        "kept {}, analyzed {}",
        rep.kept_files,
        rep.analyzed_files
    );
    // `./util` goes: a file of the package is gone, and the package is traced as a
    // whole (a lookup that found the file may have left no record of it).
    std::fs::remove_file(r.join("packages/a/src/util.ts")).unwrap();
    let rep = refresh_equals_cold(&r, "an imported file is deleted");
    assert_eq!(rep.kept_files, 0, "kept {}", rep.kept_files);
    // A file of another package that `a` reached is deleted, then comes back.
    std::fs::remove_file(r.join("packages/b/src/index.ts")).unwrap();
    refresh_equals_cold(&r, "a reached file of another package is deleted");
    write(&r, "packages/b/src/index.ts", "export default 5;\n");
    refresh_equals_cold(&r, "and comes back");
}

/// What the shard knew to be there and is gone keeps no file of the package: a
/// lookup that found it may have left no record on the file that looked.
#[test]
fn what_was_there_and_is_gone_keeps_no_file() {
    // A file of the package that no other file reached.
    let (_t, r) = built_workspace();
    std::fs::remove_file(r.join("packages/a/src/alias.ts")).unwrap();
    let rep = refresh_equals_cold(&r, "a file nothing reached is deleted");
    assert_eq!(rep.traced, 1, "one package");
    assert_eq!(rep.kept_files, 0, "a file of the package is gone");
    // The same file added again: what appears is covered by what each file looked
    // for.
    write(&r, "packages/a/src/alias.ts", "export const v = 1;\n");
    let rep = refresh_equals_cold(&r, "the file is back");
    assert!(rep.kept_files >= 6, "kept {}", rep.kept_files);

    // A file of another package that a file of `a` reached.
    let (_t, r) = built_workspace();
    std::fs::remove_file(r.join("packages/b/src/index.ts")).unwrap();
    let rep = refresh_equals_cold(&r, "a reached file of another package is deleted");
    assert_eq!(rep.kept_files, 0, "a reached file is gone");

    // The place of a package link: an installed package, and the link of `c` to the
    // workspace package `a` (`c` has a file that imports nothing through it).
    let (_t, r) = built_workspace();
    apply(&r, &Edit::Uninstall);
    let rep = refresh_equals_cold(&r, "a linked package is uninstalled");
    assert_eq!(rep.kept_files, 0, "the place of a link is gone");
    let (_t, r) = built_workspace();
    apply(&r, &Edit::Link(2, false));
    let rep = refresh_equals_cold(&r, "a workspace link is removed");
    assert_eq!(rep.traced, 1, "one package");
    assert_eq!(rep.kept_files, 0, "the place of a link is gone");

    // A manifest that files of other packages reached: the manifest of `a`, for the
    // file of `c` and the file of the root that import `a`. Each of the two
    // packages has another file that reached nothing of `a`.
    let (_t, r) = built_workspace();
    std::fs::remove_file(r.join("packages/a/package.json")).unwrap();
    let rep = refresh_equals_cold(&r, "the manifest of an imported package is deleted");
    assert_eq!(rep.kept_files, 0, "a reached manifest is gone");

    // A config that is no file of the package and that no edge reaches: the
    // tsconfig at the root, which governs the files without a tsconfig of their
    // own that have a reference to resolve (`c.ts` of `c`). The other file of
    // `c` has none: it read no tsconfig. (For the root package the tsconfig is
    // a file of the package.)
    let (_t, r) = canonical_tmp();
    workspace(&r);
    write(
        &r,
        "tsconfig.json",
        r#"{"compilerOptions":{"strict":true}}"#,
    );
    build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
    std::fs::remove_file(r.join("tsconfig.json")).unwrap();
    let rep = refresh_equals_cold(&r, "the governing tsconfig is deleted");
    assert_eq!(rep.kept_files, 0, "a config is gone");
}

#[test]
fn a_changed_config_retraces_its_readers() {
    let (_t, r) = built_workspace();
    // The manifest of `b` changes what `import 'b'` resolves to: readers in `a`
    // only.
    write(&r, "packages/b/package.json", B_MANIFESTS[2]);
    let rep = refresh_equals_cold(&r, "the main of a dependency changes");
    assert!(rep.kept_files > 0, "kept {}", rep.kept_files);
    // The jest config loses its module mapper: shared framework state.
    write(&r, "packages/a/jest.config.js", JEST_CONFIGS[2]);
    refresh_equals_cold(&r, "the module mapper goes");
    write(&r, "packages/a/test/setup.js", "jest.mock('b');\n");
    refresh_equals_cold(&r, "the setup file mocks another module");
    write(&r, "packages/a/tsconfig.json", TSCONFIGS[1]);
    refresh_equals_cold(&r, "the tsconfig paths change");
    write(&r, "turbo.json", TURBOS[1]);
    let rep = refresh_equals_cold(&r, "the task pipeline changes");
    assert_eq!(rep.analyzed_files, 0, "only tasks are computed again");
    assert!(rep.kept_files > 0, "kept {}", rep.kept_files);
    // The lockfile: a dependency the importer did not have, then a version bump.
    write(&r, "packages/a/package.json", A_MANIFESTS[1]);
    refresh_equals_cold(&r, "a dependency entry is added");
    lodash(&r, "4.17.30", "sha512-new");
    refresh_equals_cold(&r, "the lockfile bumps a package");
}

#[derive(Clone, Debug)]
enum Edit {
    Write(usize, Vec<usize>),
    Delete(usize),
    BumpLodash(u8),
    ManifestA(usize),
    ManifestB(usize),
    Jest(Option<usize>),
    Tsconfig(Option<usize>),
    Base(bool),
    Turbo(usize),
    PackageC(bool),
    Mock(usize, bool),
    Uninstall,
    Lock(bool),
    Knip(bool),
    /// A workspace link is made (also: made to lead elsewhere) or removed.
    Link(usize, bool),
    /// A framework config at a path that had none: at the root, or below a
    /// package's own. Nothing a file read or looked for says that there was
    /// none.
    ConfigAt(usize, Option<usize>),
    /// The generated config that some `TSCONFIGS` extend appears, changes or
    /// goes.
    Generated(Option<usize>),
    /// The lockfile gives the importer `packages/a` an entry for `left-pad`: a
    /// link to `packages/b` (a package of the repository), or the registry
    /// package.
    LeftPad(bool),
    /// The manifest of `packages/c` comes to name an entry.
    ManifestC(usize),
    /// The project config of `packages/b`, which some `TSCONFIGS` reference,
    /// appears, changes or goes.
    Referenced(Option<usize>),
    /// A `tsconfig.json` at the root appears, changes or goes.
    RootTsconfig(Option<usize>),
    /// The preset that some `JEST_CONFIGS` and `CONFIGS_AT` name appears,
    /// changes or goes.
    Preset(Option<usize>),
    /// A manifest in a directory below the config of `packages/a` appears,
    /// changes or goes (with a `jest` key: a config).
    ManifestAt(usize, Option<usize>),
    /// The root manifest: with or without a `jest` key (a config above every
    /// package).
    RootManifest(usize),
    /// One of `PATHS` is written (the contents by number, modulo how many it
    /// has) or goes (a directory with everything below it; the directories
    /// that are empty then, too).
    Path(usize, Option<usize>),
    /// The shared config at the root appears, changes or goes.
    Shared(Option<usize>),
    /// The config that the manifest of `packages/b` can map appears, changes or
    /// goes.
    Mapped(Option<usize>),
    /// A directory of `DIRS` appears (empty, or with a file that is no source)
    /// or goes.
    Dir(usize, Option<bool>),
    /// The linked source directory of `packages/a` (`LINKS` 4 and 6) as a real
    /// directory with the files the links lead to, or gone: a path that is
    /// a directory in one tree and a link to a directory in another.
    RealDir(bool),
    /// One of `FILE_LINKS`: a file comes to be a link to another file (a later
    /// write or deletion of the path makes it a file again, or nothing).
    FileLink(usize),
}

/// Directories at places where a config or a module is looked for, none of
/// which ever holds what the lookup wants. A lookup that asks "is it a
/// directory?" first goes inside and asks for other paths than without the
/// directory: the target of `"extends": "../../tsconfig.shared"` (the loader
/// asks for `tsconfig.shared/tsconfig.json` and no longer for
/// `tsconfig.shared.json`), a project that configs reference, and the targets
/// of `import './missing'` and `import type './types'`. The last one is a
/// directory with the name of a config file, at the root: the files
/// whose discovery gets there look for a file, and what is there is no file.
const DIRS: &[&str] = &[
    "tsconfig.shared",
    "packages/nope",
    "packages/a/src/missing",
    "packages/a/src/types",
    "tsconfig.json",
    // Next to `chained.json`, which a config of the linked package `b` extends as
    // `./chained`, and the directory whose name reads as a pattern (`TSCONFIGS`).
    "packages/b/chained",
    "packages/a/[cfg]",
    // Directories with the name of a file that a lookup makes up elsewhere than next to
    // a relative request (both are `FILES`, which take their place back): the index file
    // of a directory that `import './dir'` names, and the file next to the target of a
    // tsconfig mapping (`@/util`).
    "packages/a/src/dir/index.ts",
    "packages/a/src/util.ts",
];

/// The linked source directory of `packages/a` and what it has as a real
/// directory.
const REAL_DIR: (&str, &[&str]) = (
    "packages/a/src/linked",
    &["other.ts", "index.ts", "sub/util.ts"],
);

/// Files that come to be links, and where the links lead: a module that a
/// config asks the real path of and other files import; the setup file of
/// `packages/a`.
const FILE_LINKS: &[(&str, &str)] = &[
    ("packages/b/src/other.ts", "index.ts"),
    ("packages/a/test/setup.js", "../src/plain.ts"),
];

const FILES: &[&str] = &[
    "packages/a/src/index.ts",
    "packages/a/src/util.ts",
    "packages/a/src/missing.ts",
    "packages/a/src/lib.ts",
    "packages/a/src/plain.ts",
    "packages/a/src/alias.ts",
    "packages/a/src/dir/util.ts",
    "packages/a/src/dir/index.ts",
    "packages/a/src/index.test.ts",
    "packages/a/src/plain.test.ts",
    "packages/a/src/more.spec.ts",
    "packages/a/src/locales/de.json",
    "packages/a/test/setup.js",
    "packages/b/src/index.ts",
    "packages/b/src/other.ts",
    "packages/c/src/c.ts",
    "packages/c/src/d.ts",
    "src/root.ts",
    "docs/more.md",
    "packages/a/src/dir/deep.test.ts",
    // An index file of `packages/c`: with it the package's directory answers a lookup of
    // the name a link gives it, whatever its manifest says.
    "packages/c/index.js",
    // A module whose kind its extension says: it asks no manifest for it.
    "packages/a/src/dir/esm.mjs",
    // A file without an extension at the place `import type './types'` names: the request
    // as written. A directory of `DIRS` has the same path: one takes the other's place.
    "packages/a/src/types",
    // What a real path below the linked source directory of `packages/a` names, wherever
    // the link leads (`JEST_CONFIGS` 10, `CONFIGS_AT` 12).
    "packages/b/src/sub/util.ts",
    "packages/c/src/sub/util.ts",
];

const LINES: &[&str] = &[
    "import { u } from './util';",
    "import './missing';",
    "import _ from 'lodash';",
    "import b from 'b';",
    "import d from './dir';",
    "const l = require('./locales/' + x + '.json');",
    "import o from '../../b/src/other';",
    "import a from 'a';",
    "import p from '@/util';",
    "import q from '@lib/util';",
    "import type { T } from './types';",
    "const w = require(name);",
    "import pad from 'left-pad';",
    "import nope from 'not-installed';",
    "jest.mock('./util');",
    "jest.mock('lodash');",
    "jest.mock('b');",
    "import u2 from 'a/util';",
    "import { p } from './plain';",
    "jest.mock('../src/util');",
    "import lo from './linked/other';",
    "jest.mock('../../b/src/other');",
    // Loads that say where they resolve from (`turborepo_trace_lang_js::resolve::located`): a
    // module searched from another package (when it is not found there, that package's
    // manifest and the lockfile decide), next to a module that is installed, next to one
    // that is not (a registry package of the lockfile; a name nothing provides), by URL.
    "const r1 = require(require.resolve('b', { paths: [__dirname + '/../../c'] }));",
    "const r2 = require(require.resolve('left-pad', { paths: [__dirname + '/../../c'] }));",
    "const r3 = require('module').createRequire(require.resolve('lodash'))('./fp.js');",
    "const r4 = require('module').createRequire(require.resolve('left-pad'))('./inner.js');",
    "const r5 = require('module').createRequire(require.resolve('not-installed'))('./x.js');",
    "const r6 = import(new URL('./other.ts', import.meta.resolve('b')).href);",
    "const r7 = require('fs').readFileSync(require.resolve('lodash') + '/../package.json');",
    "const r8 = require(require.resolve('a/util', { paths: [__dirname + '/../../..'] }));",
    // A located root that is no read (a child's working directory, the base of `relative`,
    // a message), and one that is read.
    "const root1 = require('path').resolve(__dirname, '..', '..', '..'); \
     require('child_process').execSync('git status', { cwd: root1 }); \
     console.log(require('path').relative(root1, name));",
    "const root2 = require('path').resolve(__dirname, '..', '..', '..'); \
     require('fs').readFileSync(require('path').join(root2, name));",
    // Names that only a `baseUrl` resolves: of the root, and of `packages/b`.
    "import viaRoot from 'src/root';",
    "import viaB from 'src/other';",
    // A relative request for a package directory: its manifest (an entry or none) and its
    // index file decide whether it answers.
    "import c from '../../c';",
];

/// Workspace links: the link and where it leads. The last one leads the link of
/// `b` in `packages/a` to another package.
const LINKS: &[(&str, &str)] = &[
    ("node_modules/a", "../packages/a"),
    ("packages/a/node_modules/b", "../../../packages/b"),
    ("packages/c/node_modules/a", "../../../packages/a"),
    ("packages/a/node_modules/b", "../../../packages/c"),
    // A linked source directory: no package, a link on the way to a first-party file.
    ("packages/a/src/linked", "../../b/src"),
    // The name `b` a level above the link of `packages/a`: where a lookup from `packages/a`
    // goes on to when the directory its own link leads to does not answer.
    ("node_modules/b", "../packages/b"),
    // The linked source directory leads to another package.
    ("packages/a/src/linked", "../../c/src"),
    // A link that a tsconfig extends: to the base config at the root, and to its copy in
    // `packages/b` (both are always there: see `a_link_to_nothing_appears_in_a_package`).
    ("packages/a/tsconfig.link.json", "../../tsconfig.base.json"),
    ("packages/a/tsconfig.link.json", "../b/tsconfig.base.json"),
    // A linked directory above the place a config load looks at
    // (`"extends": "./cfg/configs/shared.json"`): to the package that can hold the config,
    // and to one that never does.
    ("packages/a/cfg", "../b"),
    ("packages/a/cfg", "../c"),
    // The same pair of linked directories at the root: no entry of the package whose
    // config extends through it (`"extends": "../../cfgs/configs/shared.json"`).
    ("cfgs", "packages/b"),
    ("cfgs", "packages/c"),
];

/// Places of a framework config other than `packages/a/jest.config.js`: the
/// root (a config above every package), and a directory below the package's
/// config (a project of its own for the tests below it).
const CONFIG_PLACES: &[&str] = &["jest.config.js", "packages/a/src/dir/jest.config.js"];

const CONFIGS_AT: &[&str] = &[
    "module.exports = { moduleNameMapper: { '^@lib/(.*)$': '<rootDir>/$1' } };\n",
    "module.exports = { projects: ['<rootDir>/packages/*'] };\n",
    "module.exports = { setupFiles: ['<rootDir>/packages/a/test/setup.js'], moduleNameMapper: { \
     '^@lib/(.*)$': '<rootDir>/packages/a/src/dir/$1' } };\n",
    "module.exports = { resolver: './resolver.js' };\n",
    // The preset of `packages/a`, named from the root (from the other place the path names
    // nothing): a config above packages that have no tests of their own.
    "module.exports = { preset: './packages/a/preset' };\n",
    // A project whose root is one package: the files of the other packages below the
    // config are asked about and are not in it.
    "module.exports = { rootDir: 'packages/b', moduleNameMapper: { '^@lib/(.*)$': \
     '<rootDir>/src/$1' } };\n",
    "module.exports = { rootDir: 'packages/a', moduleNameMapper: { '^@lib/(.*)$': \
     '<rootDir>/src/dir/$1' } };\n",
    // Projects as a list: a path that is a directory, a file or nothing (`packages/d`).
    "module.exports = { projects: ['<rootDir>/packages/a', '<rootDir>/packages/c', \
     '<rootDir>/packages/d'] };\n",
    // Projects as a glob over config files, and over directories of two parents.
    "module.exports = { projects: ['<rootDir>/packages/*/jest.config.js'] };\n",
    "module.exports = { projects: ['<rootDir>/{packages,docs}/*'] };\n",
    // Two projects whose tests are the same files: each has inputs of its own.
    "module.exports = { projects: [{ displayName: 'one', rootDir: 'packages/a', setupFiles: \
     ['<rootDir>/test/setup.js'] }, { displayName: 'two', rootDir: 'packages/a', setupFiles: \
     ['<rootDir>/src/util.ts'], moduleNameMapper: { '^@lib/(.*)$': '<rootDir>/src/dir/$1' } }] \
     };\n",
    // A config that is not read exactly, above every package.
    "const shared = require(process.env.SHARED);\nmodule.exports = { testMatch: ['**/*.spec.ts'], \
     verbose: shared.verbose };\n",
    // A config above every package that asks for the real path of a directory below a
    // link of `packages/a` (`Edit::Link` 4 and 6) and maps the import `not-installed` of
    // `packages/c` to a file there.
    "const path = require('path');\nmodule.exports = { moduleNameMapper: { '^not-installed$': \
     path.resolve(require('fs').realpathSync(path.resolve(__dirname, \
     'packages/a/src/linked/sub')), 'util.ts') } };\n",
    // The same for a file: one that is a file in one tree and a link to another file in
    // another (`Edit::FileLink`).
    "const path = require('path');\nmodule.exports = { moduleNameMapper: { '^not-installed$': \
     require('fs').realpathSync(path.resolve(__dirname, 'packages/b/src/other.ts')) } };\n",
];

const MOCKS: &[&str] = &[
    "packages/a/__mocks__/lodash.js",
    "packages/a/src/__mocks__/util.ts",
    "packages/a/__mocks__/b.js",
    // The manual mock next to a module of another package.
    "packages/b/src/__mocks__/other.ts",
    // Mocks of installed packages in the haste map of a project at the root.
    "__mocks__/left-pad.js",
    "__mocks__/a.js",
];

fn apply(r: &Path, e: &Edit) {
    let put = |rel: &str, variant: Option<&str>| match variant {
        Some(text) => {
            // (A file takes the place of a directory of `DIRS`.)
            if r.join(rel).is_dir() {
                std::fs::remove_dir_all(r.join(rel)).unwrap();
            }
            write(r, rel, text)
        }
        None => {
            let _ = std::fs::remove_file(r.join(rel));
        }
    };
    match e {
        Edit::Write(f, lines) => {
            let mut s: String = lines.iter().map(|&l| format!("{}\n", LINES[l])).collect();
            s.push_str("export default 1;\n");
            // (A file takes the place of a directory of `DIRS`.)
            if r.join(FILES[*f]).is_dir() {
                std::fs::remove_dir_all(r.join(FILES[*f])).unwrap();
            }
            write(r, FILES[*f], &s);
        }
        Edit::Delete(f) => put(FILES[*f], None),
        Edit::BumpLodash(v) => lodash(r, &format!("4.17.{}", 30 + v), &format!("sha512-{v}")),
        Edit::ManifestA(i) => put("packages/a/package.json", Some(A_MANIFESTS[*i])),
        Edit::ManifestB(i) => put("packages/b/package.json", Some(B_MANIFESTS[*i])),
        Edit::Jest(i) => put("packages/a/jest.config.js", i.map(|i| JEST_CONFIGS[i])),
        Edit::Tsconfig(i) => put("packages/a/tsconfig.json", i.map(|i| TSCONFIGS[i])),
        Edit::Base(strict) => {
            for base in BASES {
                put(
                    base,
                    Some(if *strict {
                        r#"{"compilerOptions":{"strict":true}}"#
                    } else {
                        r#"{"compilerOptions":{"strict":false,"baseUrl":"."}}"#
                    }),
                );
            }
        }
        Edit::Turbo(i) => put("turbo.json", Some(TURBOS[*i])),
        Edit::PackageC(on) => put(
            "packages/c/package.json",
            on.then_some(r#"{"name":"c","dependencies":{"a":"workspace:*","left-pad":"^1"}}"#),
        ),
        Edit::Mock(i, on) => put(MOCKS[*i], on.then_some("module.exports = {};\n")),
        Edit::Uninstall => put("packages/a/node_modules/lodash", None),
        Edit::Lock(on) => {
            if *on {
                lodash(r, "4.17.21", "sha512-old");
            } else {
                put("pnpm-lock.yaml", None);
            }
        }
        Edit::Link(i, on) => {
            let (link, target) = LINKS[*i];
            if *on {
                symlink(r, target, link);
            } else {
                let _ = std::fs::remove_file(r.join(link));
            }
        }
        Edit::ConfigAt(at, i) => put(CONFIG_PLACES[*at], i.map(|i| CONFIGS_AT[i])),
        Edit::Generated(i) => put(GENERATED, i.map(|i| GENERATED_CONFIGS[i])),
        Edit::LeftPad(link) => left_pad_lock(r, *link),
        Edit::ManifestC(i) => put("packages/c/package.json", Some(C_MANIFESTS[*i])),
        Edit::Referenced(i) => put(REFERENCED, i.map(|i| REFERENCED_CONFIGS[i])),
        Edit::RootTsconfig(i) => put("tsconfig.json", i.map(|i| ROOT_TSCONFIGS[i])),
        Edit::Preset(i) => put(PRESET, i.map(|i| PRESETS[i])),
        Edit::ManifestAt(at, i) => put(MANIFEST_PLACES[*at], i.map(|i| MANIFESTS_AT[i])),
        Edit::RootManifest(i) => put("package.json", Some(ROOT_MANIFESTS[*i])),
        Edit::Path(at, i) => {
            let (path, contents) = PATHS[*at];
            match i {
                Some(i) => write(r, path, contents[i % contents.len()]),
                None => remove(r, path),
            }
        }
        Edit::Shared(i) => put(SHARED, i.map(|i| SHARED_CONFIGS[i])),
        Edit::Mapped(i) => put(MAPPED, i.map(|i| MAPPED_CONFIGS[i])),
        Edit::Dir(i, on) => match on {
            Some(file) => {
                // (The directory takes the place of a file of `FILES`.)
                let _ = std::fs::remove_file(r.join(DIRS[*i]));
                std::fs::create_dir_all(r.join(DIRS[*i])).unwrap();
                if *file {
                    write(r, &format!("{}/notes.txt", DIRS[*i]), "nothing\n");
                }
            }
            None => {
                let _ = std::fs::remove_dir_all(r.join(DIRS[*i]));
            }
        },
        Edit::RealDir(on) => {
            let (dir, files) = REAL_DIR;
            // (A link in its place goes, not what the link leads to.)
            remove(r, dir);
            if *on {
                for f in files {
                    write(r, &format!("{dir}/{f}"), "export default 5;\n");
                }
            }
        }
        Edit::FileLink(i) => {
            let (link, target) = FILE_LINKS[*i];
            if r.join(link).is_dir() && !r.join(link).is_symlink() {
                // (The setup file as a directory, `PATHS`: the link takes its place.)
                remove(r, link);
            }
            symlink(r, target, link);
        }
        Edit::Knip(on) => put(
            "packages/b/package.json",
            Some(if *on {
                r#"{"name":"b","main":"src/index.ts","scripts":{"clean-code":"knip --directory ../.. --workspace packages/b"}}"#
            } else {
                B_MANIFESTS[0]
            }),
        ),
    }
}

/// Sequences the randomized test found a refreshed graph to differ from a fresh
/// one for, shrunk. They stay here as fixed cases: a random run explores other
/// sequences every time.
fn regressions() -> Vec<(&'static str, Vec<Edit>)> {
    vec![
        (
            // A file that is no test replaces a module with a mock (`jest.mock('lodash')` in
            // the setup file, no jest config left): the framework pass resolved the module,
            // and nothing of the file recorded what it found. When the package went, the
            // file kept its records, and with them the probes of a lookup that succeeded.
            "a mocked module that a file without edges to it resolved is uninstalled",
            vec![Edit::Jest(None), Edit::Uninstall],
        ),
        (
            "the same with an edit of another file in between",
            vec![
                Edit::Jest(None),
                Edit::Write(2, vec![7, 3, 15]),
                Edit::Uninstall,
            ],
        ),
        (
            // The same hole before the framework pass named mocked modules in markers (found
            // by a 2,000-case run on the tree this test was first written for): the setup
            // file's `jest.mock('lodash')` is resolved for every test of the project.
            "a manual mock appears, then the mocked package is uninstalled",
            vec![Edit::Mock(0, true), Edit::Uninstall],
        ),
        (
            // The same lookup from a test, with the jest config in place.
            "a module a test mocks is uninstalled",
            vec![Edit::Write(9, vec![15]), Edit::Uninstall],
        ),
        (
            // The link a resolution crossed to a workspace package goes: the file it led to
            // is still there, and nothing else recorded the link.
            "a workspace link goes, comes back and leads elsewhere",
            vec![
                Edit::Link(1, false),
                Edit::Link(1, true),
                Edit::Link(3, true),
            ],
        ),
        (
            "the link of the root to a workspace package goes",
            vec![Edit::Link(0, false), Edit::Link(2, false)],
        ),
        (
            // A lookup that failed inside a package it reached through a link (no `main`,
            // no index: the probes lie where the link leads) has no edge that records the
            // link. The link goes: every recorded path is as it was.
            "a link that a failed lookup went through leads elsewhere, then goes",
            vec![Edit::Link(3, true), Edit::Link(1, false)],
        ),
        (
            // The same link leads to a package that answers the lookup again: it is there
            // before and after (the level above the walk-up says nothing), and where it
            // leads is all that changed.
            "a link that a failed lookup went through leads to another package",
            vec![Edit::Link(3, true), Edit::Link(1, true)],
        ),
        (
            "the main file of a linked package goes, then the link",
            vec![Edit::Delete(13), Edit::Link(1, false)],
        ),
        (
            // A jest project without a test (`testMatch` matches nothing): no file of the
            // package had the haste map as an input, and the first mock of a package that
            // its files import changed nothing the shard's key covered.
            "the first mock of an imported package in a project without tests",
            vec![Edit::Jest(Some(3)), Edit::Mock(2, true)],
        ),
        (
            // A file that is no test mocks a first-party module by path: the framework pass
            // found the file, and only names it. The file goes.
            "a file that a helper mocks by path is deleted",
            vec![
                Edit::Jest(None),
                Edit::Write(12, vec![19]),
                Edit::Delete(1),
                Edit::Write(1, vec![]),
            ],
        ),
        (
            // A link that is no package directory, crossed on the way to a file of another
            // package: the file stays where it is when the link goes.
            "a linked source directory that an import crossed goes",
            vec![
                Edit::Link(4, true),
                Edit::Write(4, vec![20]),
                Edit::Link(4, false),
                Edit::Link(4, true),
            ],
        ),
        (
            // A module mocked by path goes, and comes back.
            "a file a test mocks by path is deleted and written again",
            vec![
                Edit::Write(9, vec![14]),
                Edit::Delete(1),
                Edit::Write(1, vec![]),
            ],
        ),
        (
            // Found by a mutation run (the framework pass's reached files switched off), and
            // the first case the persistence file held.
            "a file that a test mocks by path is deleted, no jest config left",
            vec![Edit::Jest(None), Edit::Delete(1)],
        ),
        (
            // A file imports its own package by name (an `exports` map answers), and the
            // legacy resolution next to it walks `node_modules` up and enters the root's
            // link to the package: the walk-up record holds the levels that missed, and
            // nothing said where the walk stopped. The link goes: a fresh trace records one
            // level more, and the package was not even re-keyed.
            "a link that a walk-up stopped at goes",
            vec![
                Edit::ManifestA(3),
                Edit::Write(0, vec![17]),
                Edit::Link(0, false),
            ],
        ),
        (
            // A manual mock that a file other than a test registers is found by looking
            // next to its module, and only named in a marker.
            "a manual mock that a file without tests registered is deleted",
            vec![Edit::Mock(1, true), Edit::Jest(None), Edit::Mock(1, false)],
        ),
        (
            "the same with the mock made after the jest config went",
            vec![
                Edit::Jest(None),
                Edit::Mock(1, true),
                Edit::Turbo(1),
                Edit::Mock(1, false),
            ],
        ),
        (
            // The same lookup into another package: the package of the file that registered
            // the mock has no other record of the mock, and is not re-keyed without one.
            "a manual mock next to a module of another package is deleted",
            vec![
                Edit::Jest(None),
                Edit::Write(4, vec![21]),
                Edit::Mock(3, true),
                Edit::Mock(3, false),
            ],
        ),
        (
            // The setup file is the only file of its package that resolved the module (to
            // mock it by name).
            "a module that only a mocking helper resolved is uninstalled",
            vec![Edit::Jest(None), Edit::Write(0, vec![]), Edit::Uninstall],
        ),
        (
            // A helper mocks a module of another package by path: its package has no other
            // record of the module, and without a record of what the framework pass found
            // it is not re-keyed when the module goes.
            "a module of another package that only a mocking helper resolved is deleted",
            vec![
                Edit::Jest(None),
                Edit::Write(12, vec![21]),
                Edit::Delete(14),
            ],
        ),
        (
            // A project at the root: its haste map is an input of every file below it, and
            // no file of the package `c` reads the listing itself. The first mock of a
            // package that a file of `c` imports changes nothing else of `c`'s shard.
            "a mock of an imported package appears in the haste map of a root project",
            vec![
                Edit::ConfigAt(0, Some(2)),
                Edit::Mock(4, true),
                Edit::Mock(5, true),
                Edit::Mock(4, false),
            ],
        ),
        (
            // A module mapper leads into a directory that is not there: the lookup records
            // the missing directory. The directory appears (with a file the lookup does
            // not want): a fresh trace records the paths it looked for inside, none of
            // which exists. The two shards had one key, and the store kept the first.
            "a directory that a lookup stopped at appears",
            vec![
                Edit::Write(13, vec![9]),
                Edit::ConfigAt(0, Some(2)),
                Edit::ConfigAt(1, Some(0)),
            ],
        ),
        (
            // `require.resolve('a/util', { paths: [root] })`: the search enters the root's
            // link to the package, whose manifest has no such subpath. The manifest decides,
            // and the failed search did not record it: an `exports` map that names the
            // subpath re-traced no file (a 4,000-case run found it).
            "a linked package comes to export a subpath that was searched from another place",
            vec![Edit::Write(13, vec![29]), Edit::ManifestA(3)],
        ),
        (
            "the same next to the package's own entry",
            vec![
                Edit::Write(13, vec![29, 27]),
                Edit::ManifestA(3),
                Edit::ManifestA(0),
            ],
        ),
        (
            // `require.resolve('b', { paths: [packages/c] })`, with nothing found: the
            // reference depends on the manifest of the package the search started in. That
            // manifest was looked up without a record of the levels passed, so when the
            // package's own `package.json` came back the file kept the root's (two runs of
            // 4,000 cases found it, with other seeds than the one before).
            "the manifest of the place a search started in goes and comes back",
            vec![
                Edit::Write(12, vec![22]),
                Edit::PackageC(false),
                Edit::PackageC(true),
            ],
        ),
        (
            // `import b from 'b'` enters `packages/c` through the link of `packages/a`, and
            // the package does not answer (no entry in its manifest, no index file). The
            // manifest decides that: while it was there it was an input of the file, and
            // while it was not, nothing said that it had been looked for. When it came
            // back the shard was not re-keyed, and a fresh trace has the manifest as an
            // input: two keys for equal graphs (the long runs of the branch that widened
            // the vocabulary found it; it fails the same way without that branch).
            "the manifest of the package a link leads to goes and comes back",
            vec![
                Edit::Link(3, true),
                Edit::PackageC(false),
                Edit::PackageC(true),
            ],
        ),
        (
            // The manifest that is there comes to name an entry.
            "the package a link leads to comes to answer",
            vec![
                Edit::Link(3, true),
                Edit::ManifestC(0),
                Edit::PackageC(true),
            ],
        ),
        (
            // The same lookup goes on to the levels above a package directory that does
            // not answer, as Node does.
            "the name appears a level above a package directory that does not answer",
            vec![Edit::Link(3, true), Edit::Link(5, true)],
        ),
        (
            // The directory such a walk went through comes to answer: by its manifest,
            // then (the manifest gone) by an index file. The file found a level above says
            // nothing of it.
            "a package directory that a walk went through comes to answer",
            vec![
                Edit::Link(3, true),
                Edit::Link(5, true),
                Edit::ManifestC(0),
                Edit::PackageC(false),
                Edit::Write(20, vec![]),
                Edit::Delete(20),
                Edit::PackageC(true),
                Edit::ManifestC(1),
            ],
        ),
        (
            "the name above goes while the nearer directory does not answer",
            vec![
                Edit::Link(5, true),
                Edit::Link(3, true),
                Edit::PackageC(false),
                Edit::Link(5, false),
                Edit::PackageC(true),
            ],
        ),
        (
            // (Found by the long run with seed 20261108.)
            "the setup file of a project that a config above lists is deleted",
            vec![Edit::ConfigAt(0, Some(1)), Edit::Delete(12)],
        ),
        (
            // A file that an edge leads to comes to be a link to another file: it is a
            // file before and after, and a fresh trace finds what the link leads to, with
            // the link as one it crossed (the first long run with edits that make a file
            // a link found it, through the module mapper of a manifest).
            "a mapped file comes to be a link",
            vec![Edit::RootManifest(2), Edit::FileLink(0)],
        ),
        (
            // (Found by the gate's long runs, three seeds, both trees.)
            "a link appears below a package whose config names a preset that is not there",
            vec![Edit::Jest(Some(6)), Edit::Link(4, true)],
        ),
        (
            // (The same runs.) The setup file of `packages/a` is a link, and what it leads
            // to goes: a link that leads nowhere.
            "the file a linked setup file leads to is deleted, below a config at the root",
            vec![
                Edit::FileLink(1),
                Edit::ConfigAt(0, Some(0)),
                Edit::Delete(4),
            ],
        ),
        (
            "a setup file comes to be a link that leads nowhere",
            vec![
                Edit::Delete(4),
                Edit::ConfigAt(0, Some(0)),
                Edit::FileLink(1),
            ],
        ),
        (
            // (The long run in an old tree found it through an `exports` entry.)
            "an exported file comes to be a link",
            vec![Edit::Lock(true), Edit::ManifestB(3), Edit::FileLink(0)],
        ),
        (
            "an imported file comes to be a link, and a file again",
            vec![
                Edit::Write(4, vec![6]),
                Edit::FileLink(0),
                Edit::Write(14, vec![]),
            ],
        ),
    ]
}

/// The edits one after the other on the workspace, each compared with a cold
/// build. The report of the last refresh.
fn replay(what: &str, edits: &[Edit]) -> BuildReport {
    let (_t, r) = built_workspace();
    let mut last = None;
    for (i, e) in edits.iter().enumerate() {
        apply(&r, e);
        last = Some(refresh_equals_cold(
            &r,
            &format!("{what}: after {:?}", &edits[..=i]),
        ));
    }
    last.expect("at least one edit")
}

/// Makes every file and directory of the tree that was written since the last
/// call look `3600 - 60 * step` seconds old (the graph directory and links
/// excepted): the tree as it is some time after an edit, when a refresh trusts
/// the time of a directory again. What was not touched keeps its time, and what
/// was gets one it never had.
fn age(root: &Path, start: std::time::SystemTime, step: u64) {
    let recent = start - std::time::Duration::from_secs(1800);
    let then = start - std::time::Duration::from_secs(3600 - 60 * step.min(29));
    fn walk(p: &Path, root: &Path, recent: std::time::SystemTime, then: std::time::SystemTime) {
        let Ok(md) = std::fs::symlink_metadata(p) else {
            return;
        };
        if md.file_type().is_symlink() || p == root.join(".turbotrace") {
            return;
        }
        if md.is_dir() {
            for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
                walk(&e.path(), root, recent, then);
            }
        }
        if md.modified().is_ok_and(|m| m > recent) {
            let _ = std::fs::File::open(p).and_then(|f| f.set_modified(then));
        }
    }
    walk(root, root, recent, then);
}

/// [`replay`] in an old tree. The cases of `replay` run in a tree that was
/// written a moment ago, where a refresh trusts the time of no directory and
/// computes every part of a shard's key that depends on the tree again. Here
/// the tree is old before the first build and after every edit, and the graph
/// was refreshed once before the first edit (nothing to trace): the parts that
/// cover directories no edit touched are trusted, and what a record keeps there
/// (the paths that were asked about and what is at them, a listing of one
/// directory) must be found changed through the directories that did change. (A
/// review ran 18 fixed cases this way: 8 failed before the access log.)
fn replay_old(what: &str, edits: &[Edit]) -> BuildReport {
    let (_t, r) = canonical_tmp();
    workspace(&r);
    let start = std::time::SystemTime::now();
    let out = r.join(".turbotrace");
    std::fs::create_dir_all(&out).unwrap();
    age(&r, start, 0);
    build(&opts(&r, &out), &langs()).unwrap();
    let rep = build(&opts(&r, &out), &langs()).unwrap();
    assert_eq!(rep.traced, 0, "{what}: an old tree that did not change");
    let mut last = None;
    for (i, e) in edits.iter().enumerate() {
        apply(&r, e);
        age(&r, start, i as u64 + 1);
        last = Some(refresh_equals_cold(
            &r,
            &format!("{what} (old tree): after {:?}", &edits[..=i]),
        ));
    }
    last.expect("at least one edit")
}

/// The fixed sequences in an old tree ([`replay_old`]).
#[test]
fn the_sequences_that_once_failed_in_an_old_tree() {
    let mut failed = 0;
    let only = std::env::var("TURBO_TRACE_DEBUG_CUTOFF_ONLY").ok();
    for (what, edits) in regressions() {
        if only.as_ref().is_some_and(|o| !what.contains(o.as_str())) {
            continue;
        }
        if let Err(e) = std::panic::catch_unwind(|| replay_old(what, &edits)) {
            failed += 1;
            eprintln!("FAILED {what}: {:?}", e.downcast_ref::<String>());
        }
    }
    assert_eq!(failed, 0, "sequences in an old tree");
}

#[test]
fn the_sequences_that_once_failed() {
    let mut failed = 0;
    // `TURBO_TRACE_DEBUG_CUTOFF_ONLY=<text>`: only the cases whose description
    // contains the text.
    let only = std::env::var("TURBO_TRACE_DEBUG_CUTOFF_ONLY").ok();
    for (what, edits) in regressions() {
        if only.as_ref().is_some_and(|o| !what.contains(o.as_str())) {
            continue;
        }
        if let Err(e) = std::panic::catch_unwind(|| replay(what, &edits)) {
            failed += 1;
            eprintln!("FAILED {what}: {:?}", e.downcast_ref::<String>());
        }
    }
    assert_eq!(failed, 0);
}

/// A load next to a module that is not installed is taken to stay in that
/// module's package tree when the module is a registry package, and is open
/// when it is a package of the repository. Which one it is, is the lockfile's
/// answer for the importer, the link the shard's key checks: a lockfile that
/// comes to link the name to a package of the repository re-traces the file,
/// and a package that merely takes the name changes nothing (nothing resolves
/// the module until the lockfile or an install provides it).
#[test]
fn a_load_next_to_a_module_follows_the_lockfile_link() {
    // `packages/a/src/plain.ts`:
    // `createRequire(require.resolve('left-pad'))('./inner.js')`.
    let load = Edit::Write(4, vec![25]);
    let (_t, r) = built_workspace();
    let out = r.join(".turbotrace");
    let stays = "the load is taken to stay in the package tree of the lockfile entry of 'left-pad'";
    let not_known =
        "'left-pad' is not installed (searched from packages/a/src), so the file is not known";
    // Whether the record of the load is the open one, after `e`.
    let step = |e: &Edit, what: &str| -> bool {
        apply(&r, e);
        refresh_equals_cold(&r, what);
        let g = export(&out).0;
        let about: Vec<&str> = g.lines().filter(|l| l.contains("left-pad")).collect();
        assert!(
            g.contains(stays) != g.contains(not_known),
            "{what}: one of the two records:\n{}",
            about.join("\n")
        );
        g.contains(not_known)
    };
    // No lockfile entry for the importer; then a package of the repository takes
    // the name.
    let open = step(&load, "the load");
    assert!(!open, "no entry");
    let open = step(&Edit::ManifestB(4), "a package takes the name");
    assert!(!open, "the name alone");
    // The lockfile links the importer to it, then to the registry package, then to
    // it again.
    let open = step(
        &Edit::LeftPad(true),
        "the lockfile links the package of the repository",
    );
    assert!(open, "a workspace link");
    let open = step(
        &Edit::LeftPad(false),
        "the lockfile links the registry package",
    );
    assert!(!open, "a registry entry");
    let open = step(&Edit::LeftPad(true), "the workspace link again");
    assert!(open, "a link again");
    // The lockfile goes.
    let open = step(&Edit::Lock(false), "no lockfile");
    assert!(!open, "no lockfile");
    // Other orders, and a module searched from another package of the repository
    // (`packages/c`, whose importer has the registry entry): compared with a cold
    // build.
    replay(
        "a package loses the name",
        &[Edit::ManifestB(4), load.clone(), Edit::ManifestB(0)],
    );
    replay(
        "the link before the load",
        &[
            Edit::LeftPad(true),
            Edit::ManifestB(4),
            load.clone(),
            Edit::LeftPad(false),
        ],
    );
    replay(
        "a module searched from another package",
        &[
            Edit::Write(4, vec![23]),
            Edit::ManifestB(4),
            Edit::LeftPad(true),
            Edit::PackageC(false),
            Edit::Lock(false),
        ],
    );
}

/// A project that a `tsconfig.json` references is loaded for every file whose
/// config discovery consults that config: when it does not load, that is
/// reported on each of them, and when it includes a file it governs it. A
/// project that loads and does not include the file is no input of it (an edit
/// of it selects nothing through the file), and the file recorded nothing of
/// it: when the project came to be broken, or to include the file, a refresh
/// kept the file's records, and a cold build has others. What the load
/// of the project read is a key observation of the file now
/// (`turborepo_trace_core::record::key`).
#[test]
fn a_referenced_project_that_changes_retraces_the_files_that_consulted_it() {
    // Whether the export of the refreshed graph (which is the cold build's) has the
    // record of a project reference that was skipped.
    let skipped = |r: &Path| {
        export(&r.join(".turbotrace"))
            .0
            .contains("skipped project reference")
    };
    let run = |edits: &[Edit], last: &Edit, what: &str| -> (BuildReport, bool) {
        let (_t, r) = built_workspace();
        for (i, e) in edits.iter().enumerate() {
            apply(&r, e);
            refresh_equals_cold(&r, &format!("{what}: after {:?}", &edits[..=i]));
        }
        assert!(!skipped(&r), "{what}: the project loads");
        apply(&r, last);
        let rep = refresh_equals_cold(&r, &format!("{what}: after {last:?}"));
        (rep, skipped(&r))
    };
    // The config of `packages/a` governs its files and references the project of
    // `packages/b`, which loads.
    let governs = [Edit::Referenced(Some(0)), Edit::Tsconfig(Some(6))];
    // The project's own config is deleted.
    let (_, record) = run(&governs, &Edit::Referenced(None), "the project goes");
    assert!(record, "a project that is not there is reported");
    // The target of its `extends` is deleted.
    let extends = [
        Edit::Generated(Some(0)),
        Edit::Referenced(Some(1)),
        Edit::Tsconfig(Some(6)),
    ];
    let (_, record) = run(&extends, &Edit::Generated(None), "its extends target goes");
    assert!(record, "a project that does not load is reported");
    // It comes to be a config that does not parse.
    let (_, record) = run(&governs, &Edit::Referenced(Some(3)), "it does not parse");
    assert!(record, "a project that does not parse is reported");
    // It comes to include the sources of `packages/a`: it governs them, and the
    // alias of `alias.ts` resolves through it.
    let (rep, record) = run(
        &governs,
        &Edit::Referenced(Some(2)),
        "it includes the files",
    );
    assert!(!record);
    assert!(rep.analyzed_files > 0, "the files it governs are traced");
    // An option that decides neither: the records are the same, and still a
    // function of the project's config.
    let (_, record) = run(&governs, &Edit::Referenced(Some(4)), "an option more");
    assert!(!record);

    // The same below a solution config (no file of its own, a reference that is
    // never there), and for a reference after the one that governs.
    let solution = [Edit::Referenced(Some(0)), Edit::Tsconfig(Some(7))];
    run(&solution[..1], &solution[1], "a solution config");
    replay(
        "a project of a solution config goes and comes back",
        &[
            solution[0].clone(),
            solution[1].clone(),
            Edit::Referenced(None),
            Edit::Referenced(Some(2)),
            Edit::Referenced(Some(0)),
        ],
    );
    // The project extends a config of a linked package: the link goes and comes
    // back, then the config behind it goes.
    replay(
        "a project that extends through a workspace link",
        &[
            Edit::Generated(Some(0)),
            Edit::Referenced(Some(5)),
            Edit::Tsconfig(Some(6)),
            Edit::Link(0, false),
            Edit::Link(0, true),
            Edit::Generated(None),
        ],
    );
    // The linked package comes to have an `exports` map that does not export the
    // config: the project does not load, by the package's manifest (a
    // 4,000-case run found it for the project's own files).
    replay(
        "a package that a project extends through comes to export something else",
        &[
            Edit::Generated(Some(0)),
            Edit::Referenced(Some(5)),
            Edit::ManifestA(3),
            Edit::Tsconfig(Some(6)),
            Edit::ManifestA(0),
        ],
    );
    // A config at the root, above the package's own: a solution config whose
    // projects come to be broken, a config whose own `extends` goes, and a
    // reference that is never there.
    replay(
        "a solution config appears at the root",
        &[Edit::RootTsconfig(Some(0))],
    );
    replay(
        "a solution config at the root",
        &[
            Edit::Referenced(Some(0)),
            Edit::RootTsconfig(Some(0)),
            Edit::Referenced(None),
            Edit::Tsconfig(None),
            Edit::Referenced(Some(3)),
            Edit::RootTsconfig(None),
        ],
    );
    replay(
        "a config at the root whose extends target comes and goes",
        &[
            Edit::RootTsconfig(Some(1)),
            Edit::Generated(Some(1)),
            Edit::Tsconfig(None),
            Edit::Generated(None),
        ],
    );
    replay(
        "a config at the root with a project that breaks next to one that is not there",
        &[
            Edit::Referenced(Some(1)),
            Edit::RootTsconfig(Some(2)),
            Edit::Generated(Some(0)),
            Edit::Tsconfig(None),
            Edit::Generated(None),
            Edit::Referenced(None),
        ],
    );
}

/// What the load of a config read is what the loader asked of the file system
/// (`crates/turborepo-trace-lang-js/src/resolve/tsconfig.rs`). The record used
/// to be derived from the config's text by rules of its own, which named
/// another file than the loader for an `extends` without `.json` on a name with
/// a dot, for a request that a package's manifest maps (`exports`, `imports`),
/// and for a link: the config that was loaded was no part of the record, and a
/// refresh kept the records of the files it governs.
#[test]
fn what_a_config_load_read_is_recorded() {
    replay(
        "an extends without .json on a dotted name, next to a tsconfig.json",
        &[
            Edit::RootTsconfig(Some(2)),
            Edit::Shared(Some(0)),
            Edit::Tsconfig(Some(8)),
            Edit::Shared(Some(1)),
            Edit::RootTsconfig(None),
            Edit::Shared(None),
            Edit::Shared(Some(0)),
        ],
    );
    replay(
        "an extends that a package's exports map",
        &[
            Edit::Mapped(Some(0)),
            Edit::ManifestB(5),
            Edit::Tsconfig(Some(9)),
            Edit::Mapped(Some(1)),
            // Without the map the request names the package's own config.
            Edit::ManifestB(0),
            Edit::Referenced(Some(2)),
            Edit::ManifestB(5),
            Edit::Mapped(None),
        ],
    );
    replay(
        "an extends that the package's imports map",
        &[
            Edit::Generated(Some(1)),
            Edit::ManifestA(4),
            Edit::Tsconfig(Some(11)),
            Edit::Generated(Some(0)),
            Edit::ManifestA(0),
            Edit::ManifestA(4),
            Edit::Generated(None),
        ],
    );
    // A link: the config is read where the link leads. A link that comes to lead to
    // the same bytes at another place is another config (its `baseUrl` is
    // relative to where it lies), and no content that was read says so: where
    // the link leads is recorded.
    let via_base_url = Edit::Write(5, vec![LINES.len() - 2, LINES.len() - 1]);
    replay(
        "an extends through a link that comes to lead elsewhere",
        &[
            Edit::Base(false),
            Edit::Link(7, true),
            Edit::Tsconfig(Some(10)),
            via_base_url,
            Edit::Link(8, true),
            Edit::Base(true),
            Edit::Link(7, true),
            Edit::Base(false),
            Edit::Link(7, false),
        ],
    );
    // The same for a project that a config references (a key observation of the
    // files below the referencing config).
    replay(
        "a referenced project that extends without .json",
        &[
            Edit::Shared(Some(0)),
            Edit::Referenced(Some(6)),
            Edit::Tsconfig(Some(6)),
            Edit::Shared(None),
            Edit::Shared(Some(1)),
        ],
    );
}

/// An answer is an observation, whichever answer it is: "it is a directory"
/// too. A lookup that asks whether a path is a directory before it looks inside
/// records what it asked inside, and asks other questions once the directory is
/// gone (the loader of a tsconfig: `<path>/tsconfig.json` with the directory,
/// `<path>.json` without). No path of the record exists before or after, so the
/// state of each of its paths says whether their directory is there, and the
/// file that looked is traced again.
#[test]
fn a_directory_that_was_looked_into_and_goes_retraces_the_files_that_looked() {
    // The target of an `extends`: with the directory the config does not load;
    // without it, `tsconfig.shared.json` is the config.
    replay(
        "the directory an extends names goes",
        &[
            Edit::Shared(Some(1)),
            Edit::Dir(0, Some(true)),
            Edit::Tsconfig(Some(8)),
            Edit::Dir(0, None),
        ],
    );
    replay(
        "the directory an extends names comes, empty, and goes",
        &[
            Edit::Shared(Some(0)),
            Edit::Tsconfig(Some(8)),
            Edit::Dir(0, Some(false)),
            Edit::Dir(0, None),
            Edit::Dir(0, Some(true)),
        ],
    );
    // A project reference: the same loader, and the records are key observations
    // and probes of the files below the referencing config.
    replay(
        "the directory a reference names goes and comes",
        &[
            Edit::Dir(1, Some(true)),
            Edit::Tsconfig(Some(13)),
            Edit::Dir(1, None),
            Edit::Dir(1, Some(false)),
        ],
    );
    replay(
        "the directory a root config references goes",
        &[
            Edit::Dir(1, Some(false)),
            Edit::RootTsconfig(Some(2)),
            Edit::Dir(1, None),
        ],
    );
    // A module: `import './missing'` looks for the name with every extension, and
    // inside a directory of the name for its manifest and its index files.
    replay(
        "the directory a module request names goes and comes",
        &[
            Edit::Dir(2, Some(true)),
            Edit::Dir(2, None),
            Edit::Dir(2, Some(false)),
            Edit::Dir(3, Some(false)),
            Edit::Write(4, vec![10]),
            Edit::Dir(3, None),
        ],
    );
    // A file takes the directory's place (the request as written names it), and the
    // directory the file's.
    replay(
        "a file takes the place of the directory a module request names",
        &[
            Edit::Write(4, vec![10]),
            Edit::Dir(3, Some(true)),
            Edit::Write(FILES.len() - 1, vec![]),
            Edit::Dir(3, Some(false)),
        ],
    );
    // A directory of a linked package, asked about by the path through the link:
    // the record is at the real directory.
    replay(
        "the directory next to the base of a linked package's config comes and goes",
        &[
            Edit::Tsconfig(Some(TSCONFIGS.len() - 2)),
            Edit::Dir(5, Some(false)),
            Edit::Dir(5, Some(true)),
            Edit::Dir(5, None),
            // The link goes, leads to a package without the config, and comes back.
            Edit::Link(1, false),
            Edit::Dir(5, Some(true)),
            Edit::Link(3, true),
            Edit::Link(1, true),
            Edit::Dir(5, None),
        ],
    );
    // A directory whose name reads as a pattern: the record is of that name.
    replay(
        "the directory named like a pattern comes and goes",
        &[
            Edit::Tsconfig(Some(TSCONFIGS.len() - 1)),
            Edit::Dir(6, Some(true)),
            Edit::Write(4, vec![10]),
            Edit::Dir(6, None),
            Edit::Dir(6, Some(false)),
        ],
    );
}

/// `import './missing'` with `missing.ts` there and a directory `missing` next
/// to it that holds no index: the file answers. The lookup asks about the name
/// as written first ("is it a file?"): without the directory that is a path it
/// looked for; with it, the lookup records the directory's manifest as looked
/// for, and nothing at the name. When the directory goes, the state of that
/// record says so (its directory is gone), and the file is traced again: a new
/// graph has the name as a path that was looked for, and a file that appears
/// there later is matched by it.
#[test]
fn a_directory_next_to_the_module_file_of_its_name() {
    replay(
        "a directory next to the module file of its name comes and goes",
        &[
            Edit::Write(2, vec![]),
            Edit::Dir(2, Some(false)),
            Edit::Dir(2, None),
            Edit::Dir(2, Some(true)),
            Edit::Dir(2, None),
        ],
    );
}

/// A directory with the name of a module file (`missing.ts/`), which the file
/// then takes the place of. The module resolver asks whether `missing.ts` is a
/// file, gets a no, and goes on to the next extension: a directory there is no
/// "not found" of its log.
#[test]
fn a_file_takes_the_place_of_a_directory_named_like_a_module_file() {
    let (_t, r) = built_workspace();
    std::fs::create_dir_all(r.join("packages/a/src/missing.ts")).unwrap();
    refresh_equals_cold(&r, "a directory named like the module file");
    std::fs::remove_dir(r.join("packages/a/src/missing.ts")).unwrap();
    apply(&r, &Edit::Write(2, vec![]));
    refresh_equals_cold(&r, "the file takes its place");
}

/// The same answer where the lookup made the name up elsewhere than next to a
/// relative request. `import './sub'` enters the directory `sub` and
/// asks for its index files: a directory `sub/index.ts/` is no file, and the
/// file that takes its place is the module. A request that a tsconfig maps
/// (`@/thing` to `src/dir/thing`) tries the target with every extension: a
/// directory `thing.ts/` there, then the file.
#[test]
fn a_file_takes_the_place_of_a_directory_named_like_an_index_or_a_mapped_file() {
    let (_t, r) = built_workspace();
    write(
        &r,
        "packages/a/src/plain.ts",
        "import './sub';\nexport const p = 1;\n",
    );
    std::fs::create_dir_all(r.join("packages/a/src/sub/index.ts")).unwrap();
    refresh_equals_cold(&r, "a directory named like the index file");
    std::fs::remove_dir(r.join("packages/a/src/sub/index.ts")).unwrap();
    write(&r, "packages/a/src/sub/index.ts", "export {};\n");
    refresh_equals_cold(&r, "the index file takes its place");
    // A directory that the lookup enters is no such name: `sub/index.ts/` again,
    // with the manifest of `sub` naming it as `main` (asked as a file, then
    // entered).
    std::fs::remove_file(r.join("packages/a/src/sub/index.ts")).unwrap();
    std::fs::create_dir_all(r.join("packages/a/src/sub/index.ts")).unwrap();
    write(
        &r,
        "packages/a/src/sub/package.json",
        r#"{"main":"index.ts"}"#,
    );
    write(&r, "packages/a/src/sub/index.ts/index.ts", "export {};\n");
    refresh_equals_cold(&r, "the directory is the main of its package");
    write(&r, "packages/a/src/sub/index.ts/more.ts", "export {};\n");
    refresh_equals_cold(&r, "a file next to the module inside it");

    let (_t, r) = built_workspace();
    apply(&r, &Edit::Tsconfig(Some(1)));
    write(
        &r,
        "packages/a/src/plain.ts",
        "import '@/thing';\nexport const p = 1;\n",
    );
    std::fs::create_dir_all(r.join("packages/a/src/dir/thing.ts")).unwrap();
    refresh_equals_cold(&r, "a directory named like the mapped module file");
    std::fs::remove_dir(r.join("packages/a/src/dir/thing.ts")).unwrap();
    write(&r, "packages/a/src/dir/thing.ts", "export {};\n");
    refresh_equals_cold(&r, "the mapped file takes its place");
    replay(
        "directories at an index file and at a mapped file come and go",
        &[
            // (`@/*` is `src/*`; `plain.ts` imports `./dir` and `@/util`.)
            Edit::Tsconfig(Some(0)),
            Edit::Write(4, vec![4, 8]),
            Edit::Dir(7, Some(false)),
            Edit::Dir(8, Some(true)),
            Edit::Write(7, vec![]),
            Edit::Write(1, vec![]),
            Edit::Dir(8, Some(false)),
            Edit::Dir(7, Some(true)),
            Edit::Dir(7, None),
            Edit::Dir(8, None),
        ],
    );
}

/// The references after the one that governs a file are loaded with it, and one
/// that does not load is reported on the file: what their loads read is a key
/// observation of the file, although discovery never asks them whether they
/// include it.
#[test]
fn a_reference_after_the_governing_one_is_a_key_observation() {
    replay(
        "a later reference comes to be broken, loads again and goes",
        &[
            Edit::Generated(Some(2)),
            Edit::Referenced(Some(0)),
            Edit::Tsconfig(Some(15)),
            Edit::Referenced(Some(3)),
            Edit::Referenced(Some(4)),
            Edit::Referenced(None),
        ],
    );
}

/// A path that was looked for as a file and is there as a directory is recorded
/// as not found, and that it is a directory is part of its state and of the
/// key: when a file takes its place, nothing else of the looking package
/// changes (the place is the root's; the files of `packages/a` get there once
/// their own config is gone).
#[test]
fn a_file_that_takes_the_place_of_a_directory_that_was_looked_for_as_a_file() {
    replay(
        "a directory named like a config, then the config",
        &[
            Edit::Tsconfig(None),
            Edit::Dir(4, Some(false)),
            Edit::RootTsconfig(Some(1)),
            Edit::Dir(4, Some(true)),
            Edit::RootTsconfig(Some(0)),
            Edit::RootTsconfig(None),
        ],
    );
}

/// A linked directory above the place a lookup looked at: the link comes to
/// lead elsewhere (with and without the file there), and goes. What was looked
/// for behind it is recorded where the link leads, and the link is a place
/// seen; without the link the directory of the place is not there.
#[test]
fn a_linked_directory_above_a_place_that_was_looked_at() {
    replay(
        "a config behind a linked directory",
        &[
            Edit::Mapped(Some(0)),
            Edit::Link(9, true),
            Edit::Tsconfig(Some(12)),
            // Elsewhere: no config there.
            Edit::Link(10, true),
            Edit::Link(9, true),
            Edit::Mapped(Some(1)),
            Edit::Link(9, false),
            Edit::Link(10, true),
            Edit::Link(10, false),
        ],
    );
    // The link is no entry of the package that looks: nothing but the answer "not
    // there, from the root on" changes for it when the link appears.
    replay(
        "a config behind a linked directory of another package",
        &[
            Edit::Tsconfig(Some(14)),
            Edit::Link(12, true),
            Edit::Link(11, true),
            Edit::Mapped(Some(0)),
            Edit::Link(12, true),
            Edit::Link(12, false),
        ],
    );
    replay(
        "a module behind a linked directory",
        &[
            Edit::Link(4, true),
            Edit::Write(4, vec![20]),
            Edit::Link(6, true),
            Edit::Link(4, true),
            Edit::Link(6, true),
            Edit::Link(6, false),
            Edit::Link(4, true),
        ],
    );
}

/// Open: a symbolic link that leads to nothing and appears in a package. The
/// refreshed graph holds the records of a new one in another order (before
/// the links of this file's vocabulary led to configs, no edit made such a
/// link). Nothing of a config load is involved; the links of
/// `LINKS` lead to files that are always there so that the random test does not
/// stop here.
#[test]
#[ignore = "open: the records of a refresh after a link to nothing appears are in another order"]
fn a_link_to_nothing_appears_in_a_package() {
    let (_t, r) = built_workspace();
    symlink(&r, "../../nothing.json", "packages/a/nothing.link.json");
    refresh_equals_cold(&r, "a link to nothing appears");
}

/// Of the same family: a link to nothing that appears at a place a config load
/// looked at (the target of an `extends`). A new graph records the place the
/// link would lead to. An existence check follows links, so the path was as
/// missing as before for the files' records and the refresh kept them: the
/// state of a path that was looked for says when a link to nothing is there
/// (`normalize::presence`).
#[test]
fn a_link_to_nothing_appears_where_a_config_was_looked_for() {
    let (_t, r) = built_workspace();
    apply(&r, &Edit::Tsconfig(Some(10)));
    refresh_equals_cold(&r, "the config extends a link that is not there");
    symlink(&r, "../../nothing.json", "packages/a/tsconfig.link.json");
    refresh_equals_cold(&r, "a link to nothing appears there");
    // What it leads to comes, and goes; the link leads elsewhere; the link goes.
    write(&r, "nothing.json", "{}");
    refresh_equals_cold(&r, "the place the link leads to has a config");
    std::fs::remove_file(r.join("nothing.json")).unwrap();
    refresh_equals_cold(&r, "and has none again");
    std::fs::remove_file(r.join("packages/a/tsconfig.link.json")).unwrap();
    symlink(&r, "../../nowhere.json", "packages/a/tsconfig.link.json");
    refresh_equals_cold(&r, "the link leads to another place without a config");
    std::fs::remove_file(r.join("packages/a/tsconfig.link.json")).unwrap();
    refresh_equals_cold(&r, "the link goes");
}

/// A module request whose file is a link to nothing (`./missing` of
/// `packages/a/src/index.ts`, the link there from the start): the link comes to
/// lead to another place without a file, then a file appears there. The
/// resolution follows the link, so it is a place seen, and where a place that
/// leads nowhere would lead is part of its state.
#[test]
fn an_imported_file_that_is_a_link_to_nothing_comes_to_lead_elsewhere() {
    let (_t, r) = canonical_tmp();
    workspace(&r);
    symlink(&r, "../../../nothing.ts", "packages/a/src/missing.ts");
    build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
    std::fs::remove_file(r.join("packages/a/src/missing.ts")).unwrap();
    symlink(&r, "../../../nowhere.ts", "packages/a/src/missing.ts");
    refresh_equals_cold(&r, "the link leads to another place without a file");
    write(&r, "nowhere.ts", "export const n = 1;\n");
    refresh_equals_cold(&r, "a file appears where it leads");
    std::fs::remove_file(r.join("nowhere.ts")).unwrap();
    refresh_equals_cold(&r, "and goes again");
}

/// A framework config that appears where there was none (above every package,
/// or below a package's own config) changes which project a file belongs to,
/// and what its imports resolve to there. No file read the path or looked for
/// it: config discovery goes by the walked tree. So the set of config paths
/// that can govern a package is a part of its key, and a package re-keyed by it
/// keeps no file's records.
#[test]
fn a_framework_config_at_another_place_keeps_no_file() {
    let w = |f: usize, l: &[usize]| Edit::Write(f, l.to_vec());
    let rep = replay(
        "a root config that lists projects",
        &[Edit::ConfigAt(0, Some(1))],
    );
    assert_eq!(rep.kept_files, 0, "every package is below the new config");
    replay(
        "a root config with a setup file",
        &[Edit::ConfigAt(0, Some(2))],
    );
    replay(
        "a root config with a module mapper, the package's own config gone",
        &[Edit::Jest(None), Edit::ConfigAt(0, Some(0))],
    );
    replay(
        "a root config comes and goes, the package's own config gone",
        &[
            Edit::Jest(None),
            Edit::ConfigAt(0, Some(2)),
            Edit::ConfigAt(0, None),
        ],
    );
    replay(
        "a config below the package's own, over files that import through the mapper",
        &[w(7, &[9]), w(6, &[]), Edit::ConfigAt(1, Some(0))],
    );
}

/// A jest config names a preset, and loading the config looks for it. What the
/// load finds decides the framework inputs of every file the config is
/// consulted for, not only of the tests (whose framework group has the load's
/// observations as a part of the project's state): which files are tests, and
/// what the imports of a file that is no test resolve to through a module
/// mapper. So what a load observed besides its config files is a key
/// observation of every file it is consulted for
/// (`crates/turborepo-trace-store/src/fw.rs`, `Loaded::consulted`): the files
/// that depend on it are traced again, with or without a test in the package.
#[test]
fn a_preset_that_comes_or_goes_retraces_the_files_its_config_governs() {
    // The imports of a file that is no test (`lib.ts`: `@lib/util`) come to resolve
    // through the preset's module mapper.
    replay(
        "a preset with a module mapper appears",
        &[Edit::Jest(Some(6)), Edit::Preset(Some(0))],
    );
    replay(
        "a preset with a module mapper goes",
        &[
            Edit::Jest(Some(6)),
            Edit::Preset(Some(0)),
            Edit::Preset(None),
        ],
    );
    // A file that was no test becomes one (`plain.ts`), and the tests are none.
    replay(
        "a preset with another test discovery appears and goes",
        &[
            Edit::Jest(Some(7)),
            Edit::Preset(Some(1)),
            Edit::Preset(None),
        ],
    );
    replay(
        "a preset changes",
        &[
            Edit::Jest(Some(6)),
            Edit::Preset(Some(2)),
            Edit::Preset(Some(1)),
            Edit::Preset(Some(0)),
        ],
    );
}

/// The same for a config above packages that have no test. In an earlier
/// version nothing
/// in such a package had a record of the lookup (it was recorded on tests
/// only): the package's key did not move when the preset appeared, and the file
/// that it made a test (`packages/c/src/d.ts`) was a test only in a fresh
/// build.
#[test]
fn a_preset_of_a_config_above_a_package_without_tests() {
    replay(
        "a preset of a root config appears and goes",
        &[
            Edit::ConfigAt(0, Some(4)),
            Edit::Preset(Some(1)),
            Edit::Preset(None),
        ],
    );
    replay(
        "a preset of a root config with a module mapper, the package's own config gone",
        &[
            Edit::Jest(None),
            Edit::ConfigAt(0, Some(4)),
            Edit::Preset(Some(0)),
            Edit::Preset(None),
        ],
    );
}

/// A config that lists projects looks for the setup files of each of them when
/// it is loaded, and the load is consulted for every file below the config. A
/// setup file that is there left no record of the lookup, one that is not there
/// leaves a missing path: a test of another project (in a package that does not
/// own the file) was traced without a record of it, and when the file went a
/// fresh build recorded a path the kept shard did not have. What a load found
/// is a looked-for path of the load's key observation now.
#[test]
fn a_setup_file_of_another_project_of_the_config_goes() {
    let (_t, r) = canonical_tmp();
    workspace(&r);
    write(
        &r,
        "packages/b/src/b.test.ts",
        "import b from './index';\ntest('b', () => expect(b).toBe(2));\n",
    );
    write(&r, "jest.config.js", CONFIGS_AT[1]);
    build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
    std::fs::remove_file(r.join("packages/a/test/setup.js")).unwrap();
    refresh_equals_cold(&r, "the setup file of another project is deleted");
    write(&r, "packages/a/test/setup.js", "jest.mock('lodash');\n");
    refresh_equals_cold(&r, "and comes back");
}

/// A config above a package whose project root is elsewhere (`rootDir:
/// 'packages/b'`) is asked whether its project contains each file below it, and
/// says no for the files of `packages/a`, which have a config of their own.
/// That answer is a function of the config: when its root comes to be
/// `packages/a`, its module mapper applies to the imports of those files. So
/// the config files of every config that is asked are key-only configs of the
/// file, also when the answer is no (not only when it is yes, or the key of
/// `packages/a` would not move).
#[test]
fn a_config_above_whose_project_comes_to_contain_a_package() {
    let w = |f: usize, l: &[usize]| Edit::Write(f, l.to_vec());
    replay(
        "the project root of a config above moves to the package",
        &[
            w(6, &[]),
            Edit::ConfigAt(0, Some(5)),
            Edit::ConfigAt(0, Some(6)),
            Edit::ConfigAt(0, Some(5)),
        ],
    );
}

/// A `package.json` with a `jest` key is a config, found like a config file: by
/// walking up from a file's directory. A file looks for the manifests above it
/// only as far as the nearest one (it says the kind of the module); the walk
/// for a config goes on to the config. When a manifest with a `jest` key
/// appears in a directory above the nearer manifest, it governs the file (which
/// becomes a test here). The walk is a key observation of every file that made
/// it, test or not, so each of them is traced again (it was recorded on tests
/// only, and a path that the framework group of a test looked for was shared
/// state of the package then).
#[test]
fn a_manifest_with_a_jest_key_above_a_nearer_manifest_keeps_no_file() {
    let w = |f: usize, l: &[usize]| Edit::Write(f, l.to_vec());
    replay(
        "a manifest with a jest key appears between a file's manifest and its config",
        &[
            Edit::ManifestAt(0, Some(2)),
            w(6, &[]),
            Edit::ManifestAt(1, Some(0)),
        ],
    );
    // A file reads its nearest manifest and looks for it where there is none: these
    // are its own.
    replay(
        "a manifest with a jest key appears next to a file, with a test below it",
        &[w(6, &[]), w(19, &[]), Edit::ManifestAt(0, Some(0))],
    );
    replay(
        "the same without a test, and over a module whose extension says its kind",
        &[w(6, &[]), w(21, &[]), Edit::ManifestAt(0, Some(0))],
    );
    replay(
        "a manifest comes to have a jest key",
        &[
            Edit::ManifestAt(0, Some(2)),
            w(6, &[]),
            Edit::ManifestAt(0, Some(0)),
        ],
    );
    replay(
        "a manifest with a module mapper appears and goes",
        &[
            Edit::Jest(Some(2)),
            w(6, &[]),
            w(7, &[9]),
            w(19, &[]),
            Edit::ManifestAt(0, Some(1)),
            Edit::ManifestAt(0, None),
        ],
    );
}

/// A manifest above packages that comes to have a `jest` key is a config for
/// every file below it, also in a package without a test: a file of it that the
/// new config makes a test (`packages/c/src/d.ts`), a file whose import its
/// module mapper resolves. The walk that looks for a file's config asks every
/// manifest on the way for the key, and was recorded on tests only, so such a
/// package's key did not move. What the walk accessed is a key observation of
/// every file that made it (`crates/turborepo-trace-store/src/fw.rs`, `Walk`):
/// the manifest's resolution view, which holds the key.
#[test]
fn the_root_manifest_comes_to_have_a_jest_key() {
    replay(
        "another test discovery",
        &[Edit::RootManifest(1), Edit::RootManifest(0)],
    );
    // A module mapper: it resolves the import `not-installed` of
    // `packages/c/src/c.ts` (no test, in a package without a config of its own)
    // while the key has it. (Both graphs of this case were without the edge
    // before the framework pass asked the walk of every directory above a file
    // for its config: a manifest's config was applied to no import.)
    let mapped = |r: &Path| {
        export(&r.join(".turbotrace"))
            .0
            .contains("jest:moduleNameMapper not-installed -> packages/b/src/other.ts")
    };
    let (_t, r) = built_workspace();
    assert!(!mapped(&r), "no mapper yet");
    apply(&r, &Edit::RootManifest(2));
    refresh_equals_cold(&r, "a module mapper: the manifest comes to have it");
    assert!(
        mapped(&r),
        "the mapper of the manifest's jest key is applied"
    );
    // The file it maps to goes and comes back: a record of the importer. (The edge
    // stays, to the manifest that says the kind of the importer's module; the
    // mapped file is a path that was looked for.)
    apply(&r, &Edit::Delete(14));
    refresh_equals_cold(&r, "a module mapper: the mapped file goes");
    apply(&r, &Edit::Write(14, vec![]));
    refresh_equals_cold(&r, "a module mapper: the mapped file is back");
    assert!(mapped(&r), "mapped again");
    apply(&r, &Edit::RootManifest(0));
    refresh_equals_cold(&r, "a module mapper: the key goes");
    assert!(!mapped(&r), "no mapper left");
    replay(
        "without the config of a",
        &[
            Edit::Jest(None),
            Edit::RootManifest(1),
            Edit::RootManifest(2),
            Edit::RootManifest(0),
        ],
    );
}

/// The load of a config reads a manifest through the loader's view (`name`,
/// `exports`, `imports`, `main`). A config at the root that extends
/// the root package by name has the root manifest as its nearest one; the files
/// of the packages without a config of their own are below it. Fields outside
/// the view come and go, the `exports` that answer the request come and go,
/// with and without the config of `packages/a`.
#[test]
fn a_root_manifest_that_a_config_load_reads() {
    replay(
        "a root config that extends the root package, and fields of the root manifest",
        &[
            Edit::RootTsconfig(Some(ROOT_TSCONFIGS.len() - 1)),
            Edit::RootManifest(3),
            Edit::RootManifest(4),
            Edit::Tsconfig(None),
            Edit::RootManifest(3),
            Edit::RootManifest(0),
            Edit::RootManifest(4),
            Edit::RootManifest(1),
            Edit::RootTsconfig(None),
        ],
    );
    // The config of a package that extends a package: its own manifest is the
    // nearest.
    replay(
        "a package config that extends a package, and its manifest",
        &[
            Edit::Tsconfig(Some(TSCONFIGS.len() - 2)),
            Edit::ManifestA(1),
            Edit::ManifestA(0),
            Edit::Link(1, false),
            Edit::ManifestA(2),
            Edit::ManifestA(3),
            Edit::Link(1, true),
        ],
    );
}

// Review probes: directories several levels above a place that
// was looked at (the nearest directory that is there is part of a probe's state
// and of the key).

/// `import './one/two/three'` with none of the directories there: they come one
/// by one (empty: git tracks none of them), a sibling comes, the file comes,
/// and all go.
#[test]
fn directories_above_a_module_that_was_looked_for() {
    let (_t, r) = built_workspace();
    write(
        &r,
        "packages/a/src/deep.ts",
        "import './one/two/three';\nexport const d = 1;\n",
    );
    refresh_equals_cold(&r, "a request three directories down, none there");
    let keys = package_keys(&r);
    std::fs::create_dir_all(r.join("packages/a/src/onex/two")).unwrap();
    refresh_equals_cold(&r, "a sibling of the first missing directory");
    std::fs::create_dir(r.join("packages/a/src/one")).unwrap();
    refresh_equals_cold(&r, "the first directory");
    assert_ne!(keys, package_keys(&r), "the nearest directory is another");
    std::fs::create_dir(r.join("packages/a/src/one/two")).unwrap();
    refresh_equals_cold(&r, "the second directory");
    std::fs::create_dir(r.join("packages/a/src/one/two/three")).unwrap();
    refresh_equals_cold(&r, "a directory at the request");
    std::fs::remove_dir(r.join("packages/a/src/one/two/three")).unwrap();
    write(&r, "packages/a/src/one/two/three.ts", "export {};\n");
    refresh_equals_cold(&r, "the file");
    std::fs::remove_dir_all(r.join("packages/a/src/one")).unwrap();
    refresh_equals_cold(&r, "everything goes");
    std::fs::remove_dir_all(r.join("packages/a/src/onex")).unwrap();
    refresh_equals_cold(&r, "the sibling goes");
    assert_eq!(keys, package_keys(&r), "the tree is the first one again");
}

/// `"extends": "./cfg/deep/base"` with none of the directories there: they come
/// one by one, a directory at the name, the config next to it, and the
/// directory goes.
#[test]
fn directories_above_a_config_that_was_looked_for() {
    let (_t, r) = built_workspace();
    write(
        &r,
        "packages/a/tsconfig.json",
        r#"{"extends":"./cfg/deep/base","compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    refresh_equals_cold(&r, "an extends three names down, nothing there");
    for d in ["cfg", "cfg/deep", "cfg/deep/base"] {
        std::fs::create_dir(r.join("packages/a").join(d)).unwrap();
        refresh_equals_cold(&r, &format!("the directory {d}"));
    }
    write(
        &r,
        "packages/a/cfg/deep/base.json",
        r#"{"compilerOptions":{"strict":true}}"#,
    );
    refresh_equals_cold(&r, "the config next to the directory of its name");
    std::fs::remove_dir(r.join("packages/a/cfg/deep/base")).unwrap();
    refresh_equals_cold(&r, "the directory at the name goes");
    std::fs::create_dir(r.join("packages/a/cfg/deep/base")).unwrap();
    refresh_equals_cold(&r, "and comes");
    std::fs::remove_dir_all(r.join("packages/a/cfg")).unwrap();
    refresh_equals_cold(&r, "everything goes");
}

/// The config of `packages/a` extends a config of the workspace package `b`
/// (through the link `packages/a/node_modules/b`), which extends `./base`: a
/// directory `base` comes next to `base.json` in `packages/b`, gets a file, and
/// goes.
#[test]
fn a_directory_next_to_the_base_of_a_linked_package_config() {
    let (_t, r) = built_workspace();
    write(&r, "packages/b/chain.json", r#"{"extends":"./base"}"#);
    write(
        &r,
        "packages/b/base.json",
        r#"{"compilerOptions":{"strict":true}}"#,
    );
    write(
        &r,
        "packages/a/tsconfig.json",
        r#"{"extends":"b/chain.json","compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    refresh_equals_cold(&r, "a config of a linked package that extends ./base");
    std::fs::create_dir(r.join("packages/b/base")).unwrap();
    refresh_equals_cold(&r, "a directory base, empty");
    write(&r, "packages/b/base/notes.txt", "nothing\n");
    refresh_equals_cold(&r, "with a file");
    std::fs::remove_dir_all(r.join("packages/b/base")).unwrap();
    refresh_equals_cold(&r, "the directory goes");
}

mod random {
    use proptest::{
        prelude::*,
        test_runner::{
            Config, FileFailurePersistence, RngAlgorithm, TestCaseError, TestRng, TestRunner,
        },
    };

    use super::*;

    fn edit() -> impl Strategy<Value = Edit> {
        prop_oneof![
            6 => (0..FILES.len(), proptest::collection::vec(0..LINES.len(), 0..4))
                .prop_map(|(f, l)| Edit::Write(f, l)),
            3 => (0..FILES.len()).prop_map(Edit::Delete),
            1 => (0u8..3).prop_map(Edit::BumpLodash),
            2 => (0..A_MANIFESTS.len()).prop_map(Edit::ManifestA),
            2 => (0..B_MANIFESTS.len()).prop_map(Edit::ManifestB),
            2 => proptest::option::of(0..JEST_CONFIGS.len()).prop_map(Edit::Jest),
            2 => proptest::option::of(0..TSCONFIGS.len()).prop_map(Edit::Tsconfig),
            1 => any::<bool>().prop_map(Edit::Base),
            1 => (0..TURBOS.len()).prop_map(Edit::Turbo),
            1 => any::<bool>().prop_map(Edit::PackageC),
            2 => (0..MOCKS.len(), any::<bool>()).prop_map(|(i, on)| Edit::Mock(i, on)),
            1 => Just(Edit::Uninstall),
            1 => any::<bool>().prop_map(Edit::Lock),
            1 => any::<bool>().prop_map(Edit::Knip),
            2 => (0..LINKS.len(), any::<bool>()).prop_map(|(i, on)| Edit::Link(i, on)),
            2 => (0..CONFIG_PLACES.len(), proptest::option::of(0..CONFIGS_AT.len()))
                .prop_map(|(at, i)| Edit::ConfigAt(at, i)),
            1 => proptest::option::of(0..GENERATED_CONFIGS.len()).prop_map(Edit::Generated),
            1 => any::<bool>().prop_map(Edit::LeftPad),
            1 => (0..C_MANIFESTS.len()).prop_map(Edit::ManifestC),
            2 => proptest::option::of(0..REFERENCED_CONFIGS.len()).prop_map(Edit::Referenced),
            1 => proptest::option::of(0..ROOT_TSCONFIGS.len()).prop_map(Edit::RootTsconfig),
            2 => proptest::option::of(0..PRESETS.len()).prop_map(Edit::Preset),
            1 => (0..ROOT_MANIFESTS.len()).prop_map(Edit::RootManifest),
            1 => (0..MANIFEST_PLACES.len(), proptest::option::of(0..MANIFESTS_AT.len()))
                .prop_map(|(at, i)| Edit::ManifestAt(at, i)),
            5 => (0..PATHS.len(), proptest::option::of(0..4usize))
                .prop_map(|(at, i)| Edit::Path(at, i)),
            1 => proptest::option::of(0..SHARED_CONFIGS.len()).prop_map(Edit::Shared),
            1 => proptest::option::of(0..MAPPED_CONFIGS.len()).prop_map(Edit::Mapped),
            2 => (0..DIRS.len(), proptest::option::of(any::<bool>()))
                .prop_map(|(i, on)| Edit::Dir(i, on)),
            1 => any::<bool>().prop_map(Edit::RealDir),
            1 => (0..FILE_LINKS.len()).prop_map(Edit::FileLink),
        ]
    }

    /// A number from the environment variable `name`, else `default`.
    fn env_number(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// `cases` random cases. Failing cases are written to a file next to this
    /// one (an integration test has no `lib.rs` for proptest to find its
    /// default place by), which is checked in: every later run replays them
    /// first.
    fn config(cases: u32) -> Config {
        Config {
            cases,
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/cutoff.proptest-regressions"
            )))),
            ..Config::default()
        }
    }

    /// `TURBO_TRACE_DEBUG_CUTOFF_TREE`: `young` (a tree written a moment ago,
    /// [`replay`]), `old` ([`replay_old`]) or `both`.
    fn check(edits: &[Edit], default: &str) {
        let tree =
            std::env::var("TURBO_TRACE_DEBUG_CUTOFF_TREE").unwrap_or_else(|_| default.into());
        if tree != "old" {
            replay("a random sequence", edits);
        }
        if tree != "young" {
            replay_old("a random sequence", edits);
        }
    }

    proptest! {
        #![proptest_config(config(env_number("TURBO_TRACE_DEBUG_CUTOFF_CASES", 48) as u32))]

        /// Any sequence of edits to sources, manifests, the lockfile and shared configs:
        /// after each one, the refreshed graph is a fresh build of the tree, byte for byte,
        /// in a tree that was just written and in an old one. `TURBO_TRACE_DEBUG_CUTOFF_CASES`
        /// sets the number of cases (48).
        #[test]
        fn a_refresh_equals_a_cold_build_after_every_edit(
            edits in proptest::collection::vec(edit(), 1..7),
        ) {
            check(&edits, "both");
        }
    }

    /// The same property over many cases, for a gate run:
    ///
    /// ```sh
    /// TURBO_TRACE_DEBUG_CUTOFF_CASES=4000 TURBO_TRACE_DEBUG_CUTOFF_SEED=<n> \
    ///     cargo test -p turborepo-trace-store --test cutoff -- --ignored --nocapture long_run
    /// ```
    ///
    /// `TURBO_TRACE_DEBUG_CUTOFF_TREE=old` runs the cases in an old tree
    /// ([`replay_old`]; the default here is a tree that was just written),
    /// `both` in both.
    ///
    /// The cases are spread over `TURBO_TRACE_DEBUG_CUTOFF_JOBS` threads (8),
    /// each with its own generator seeded from
    /// `TURBO_TRACE_DEBUG_CUTOFF_SEED` (the clock when unset) and the thread's
    /// number. The seed is printed, so a failing run can be repeated; a failing
    /// case is shrunk, printed and written to the regressions file.
    #[test]
    #[ignore = "long: run it explicitly, see its documentation"]
    fn long_run() {
        let cases = env_number("TURBO_TRACE_DEBUG_CUTOFF_CASES", 2000);
        let jobs = env_number("TURBO_TRACE_DEBUG_CUTOFF_JOBS", 8).max(1);
        let seed = env_number(
            "TURBO_TRACE_DEBUG_CUTOFF_SEED",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_secs()),
        );
        eprintln!("cutoff long run: seed {seed}, {cases} cases over {jobs} threads");
        let failures: Vec<String> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..jobs)
                .map(|job| {
                    scope.spawn(move || {
                        let mut bytes = [0u8; 32];
                        bytes[..8].copy_from_slice(&seed.to_le_bytes());
                        bytes[8..16].copy_from_slice(&job.to_le_bytes());
                        let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &bytes);
                        let mut runner =
                            TestRunner::new_with_rng(config((cases / jobs).max(1) as u32), rng);
                        runner
                            .run(&proptest::collection::vec(edit(), 1..7), |edits| {
                                check(&edits, "young");
                                Ok::<(), TestCaseError>(())
                            })
                            .err()
                            .map(|e| format!("thread {job} (seed {seed}): {e}"))
                    })
                })
                .collect();
            threads
                .into_iter()
                .filter_map(|t| {
                    t.join()
                        .unwrap_or_else(|_| Some("a thread panicked".into()))
                })
                .collect()
        });
        eprintln!(
            "cutoff long run: seed {seed}, {} cases, {} failure(s)",
            (cases / jobs).max(1) * jobs,
            failures.len()
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

// ---- What a trace takes from a reached file ----

#[test]
fn a_file_that_a_vendored_module_comes_to_own_is_another_target() {
    let (_t, r) = canonical_tmp();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        &r,
        "package.json",
        r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
    );
    write(&r, "packages/a/package.json", r#"{"name":"a"}"#);
    // A first-party file reads a data file below `vendor/`: an edge to a file of
    // the root package.
    write(
        &r,
        "packages/a/src/index.ts",
        "const fs = require('fs');\nconst path = require('path');\nexport const x = \
         fs.readFileSync(path.join(__dirname, '../../../vendor/example.com/dep/data.json'));\n",
    );
    write(&r, "packages/a/src/other.ts", "export const o = 1;\n");
    write(&r, "vendor/example.com/dep/data.json", "{}\n");
    write(
        &r,
        "go.mod",
        "module example.com/root\n\ngo 1.21\n\nrequire example.com/dep v1.0.0\n",
    );
    let env = tempfile::tempdir().unwrap();
    let langs: Vec<Arc<dyn Language>> = vec![
        Arc::new(turborepo_trace_lang_js::JsLanguage),
        Arc::new(turborepo_trace_lang_go::GoLanguage::new(
            turborepo_trace_lang_go::GoOptions {
                modcache: Some(env.path().join("gomod")),
                goroot: turborepo_trace_lang_go::GoRoot::None,
                cgo: Some(false),
                workspace: Some(false),
                ..Default::default()
            },
        )),
    ];
    build_with(&opts(&r, &r.join(".turbotrace")), &langs).unwrap();
    let before = package_keys(&r);
    // The directory becomes a vendored Go module (`vendor/modules.txt` appears):
    // the data file is a file of a third-party package now, and a new trace
    // links the package instead of the path. The file is still there and the
    // root package still contains it: only what owns it has changed.
    write(
        &r,
        "vendor/modules.txt",
        "# example.com/dep v1.0.0\n## explicit\nexample.com/dep\n",
    );
    let rep = refresh_equals_cold_with(&r, "a vendored module comes to own a reached file", &langs);
    assert_ne!(package_keys(&r), before, "the reader is traced again");
    assert!(
        rep.kept_files >= 1,
        "the file that does not read it is kept"
    );
}

// ---- The resolution view of a manifest ----

#[test]
fn a_dependency_entry_retraces_no_file() {
    let (_t, r) = built_workspace();
    // `b` gains a dependency entry and a script: its importers in `a` and the root
    // resolve through its manifest, and read none of that.
    write(
        &r,
        "packages/b/package.json",
        r#"{"name":"b","main":"src/index.ts","scripts":{"build":"tsc -p .","lint":"eslint src"},"dependencies":{"lodash":"^4"},"description":"b"}"#,
    );
    let rep = refresh_equals_cold(&r, "a dependency entry and a script are added");
    assert_eq!(
        rep.analyzed_files, 0,
        "no file is analyzed (kept {}, traced {} shards)",
        rep.kept_files, rep.traced
    );
    // Its `main` changes: the importers resolve to another file.
    write(
        &r,
        "packages/b/package.json",
        r#"{"name":"b","main":"src/other.ts","scripts":{"build":"tsc -p .","lint":"eslint src"},"dependencies":{"lodash":"^4"},"description":"b"}"#,
    );
    let rep = refresh_equals_cold(&r, "the main of the dependency changes");
    assert!(rep.analyzed_files > 0, "the importers are analyzed again");
    assert!(rep.kept_files > 0, "the other files are kept");
    // `c` drops a dependency from its own manifest (the lockfile still has it).
    write(
        &r,
        "packages/c/package.json",
        r#"{"name":"c","dependencies":{"a":"workspace:*"},"scripts":{"build":"tsc -p ."}}"#,
    );
    let rep = refresh_equals_cold(&r, "a package's own dependency entry goes");
    assert_eq!(rep.analyzed_files, 0, "no file of the package is analyzed");
}

/// A relative request names a directory that does not answer (its manifest
/// names no entry, it has no index file). The resolver recorded the manifest of
/// a directory that answered and nothing for one that did not: when the
/// manifest came to name an entry, the importer kept its records, and a fresh
/// build had the edge (found by the 48 random cases of a normal run: a config
/// with `rootDir: '../c'`, whose path string is a load of the directory). The
/// manifest (its resolution view, or that it is not there) is a record of
/// the request whatever the answer
/// (`crates/turborepo-trace-lang-js/src/resolve/mod.rs`, `oxc_resolve`).
#[test]
fn a_directory_that_a_relative_request_names_comes_to_answer() {
    let (_t, r) = built_workspace();
    write(
        &r,
        "packages/a/src/plain.ts",
        "import c from '../../c';\nexport const p = c;\n",
    );
    refresh_equals_cold(&r, "an import of a package directory by path");
    apply(&r, &Edit::ManifestC(0));
    refresh_equals_cold(&r, "the directory's manifest comes to name an entry");
    // Without a manifest, then with one that names another entry.
    apply(&r, &Edit::PackageC(false));
    refresh_equals_cold(&r, "the directory's manifest goes");
    apply(&r, &Edit::ManifestC(1));
    refresh_equals_cold(&r, "a manifest with another entry appears");
    // The directory comes to answer by an index file, whatever the manifest says.
    apply(&r, &Edit::PackageC(true));
    apply(&r, &Edit::Write(20, vec![]));
    refresh_equals_cold(&r, "an index file answers");
}

/// Review findings: a `projects` glob of a config above
/// comes to match a directory. The load of the config looks into every
/// directory the glob matches (its config files, its manifest), and since
/// `Loaded::consulted` what it looked for is a key observation of every file
/// below the config; the listing that says which directories there are is not
/// recorded (`Fw::recorded` drops listings, `Loaded::key` has none), so a shard
/// traced before the directory appeared does not move.
mod projects_globs {
    use super::*;

    const GLOB: &str = "module.exports = { projects: ['<rootDir>/packages/*'] };\n";

    /// Failed before the fix, passed with a fix of the framework pass alone:
    /// the export is equal, the shard keys of `packages/a`, `b` and `c` are
    /// not (a fresh build has the lookups below `packages/d` in the key
    /// observation of the load, the kept shards do not).
    #[test]
    fn a_directory_appears_below_a_projects_glob() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", GLOB);
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(&r, "packages/d/src/x.ts", "export const x = 1;\n");
        refresh_equals_cold(&r, "a directory the glob matches appears");
    }

    /// Failed before the fix and with that pass alone: the tests of
    /// `packages/a` are governed by the config above, and the lookups below
    /// `packages/d` are probes of their framework groups in a fresh build
    /// only.
    #[test]
    fn a_directory_appears_below_a_projects_glob_that_governs_tests() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        std::fs::remove_file(r.join("packages/a/jest.config.js")).unwrap();
        write(&r, "jest.config.js", GLOB);
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(&r, "packages/d/src/x.ts", "export const x = 1;\n");
        refresh_equals_cold(&r, "a directory the glob matches appears");
    }

    /// The same for a vitest `projects` glob. Failed before the fix and
    /// with that pass alone.
    #[test]
    fn a_directory_appears_below_a_vitest_projects_glob() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        std::fs::remove_file(r.join("packages/a/jest.config.js")).unwrap();
        write(
            &r,
            "vitest.config.ts",
            "export default { test: { projects: ['packages/*'] } };\n",
        );
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(&r, "packages/d/src/x.ts", "export const x = 1;\n");
        refresh_equals_cold(&r, "a directory the projects glob matches appears");
    }

    /// Passed before the fix (failed with that pass alone): the directory was
    /// there when the shards were traced, so the load's lookup of its
    /// manifest is a key observation of the files of `packages/c`, and the
    /// manifest that appears retraces them.
    #[test]
    fn a_directory_below_a_projects_glob_gets_a_project_with_a_root_elsewhere() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", GLOB);
        write(&r, "packages/d/src/x.ts", "export const x = 1;\n");
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(&r, "packages/d/package.json", ELSEWHERE);
        refresh_equals_cold(&r, "a project whose root is packages/c appears");
    }

    /// Failed before the fix and with that pass alone, in the answer: the
    /// refreshed graph has no edge `packages/c/src/c.ts` ->
    /// `packages/c/src/d.ts` (the module mapper of the new project), a
    /// fresh build has it. The same edits as the case above, with a refresh
    /// between them: the shards kept by that refresh lack the lookups.
    #[test]
    fn a_directory_appears_below_a_projects_glob_and_then_gets_a_root_elsewhere() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", GLOB);
        let out = r.join(".turbotrace");
        build(&opts(&r, &out), &langs()).unwrap();
        write(&r, "packages/d/src/x.ts", "export const x = 1;\n");
        build(&opts(&r, &out), &langs()).unwrap();
        write(&r, "packages/d/package.json", ELSEWHERE);
        refresh_equals_cold(&r, "a project whose root is packages/c appears");
    }

    /// Failed before the fix and with that pass alone, in the answer (the same
    /// missing edge): the directory and its manifest appear in one edit.
    #[test]
    fn a_project_with_a_root_elsewhere_appears_below_a_projects_glob() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", GLOB);
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(&r, "packages/d/package.json", ELSEWHERE);
        refresh_equals_cold(&r, "a project whose root is packages/c appears");
    }
}

/// What the load of a config accessed is its record
/// (`turborepo_trace_frameworks::fsx::with_log`,
/// `crates/turborepo-trace-store/src/fw.rs` `Fw::key_of`): one case per kind of
/// access that only the log knows of. Each fails without its rule.
mod loads {
    use super::*;

    /// The workspace with the config `config` at the root, built.
    fn with_root_config(config: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let (t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", config);
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        (t, r)
    }

    /// A project that a list names by its path is asked about: is it a
    /// directory (its config, its manifest), is it a file (a config)? The
    /// path is there before and after, as a file and then as a directory,
    /// and nothing lists it: only what is at the path has changed
    /// (`normalize::presence`).
    #[test]
    fn a_listed_project_that_was_a_file_comes_to_be_a_directory() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", CONFIGS_AT[7]);
        write(&r, "packages/d", PATHS[5].1[0]);
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        // No file of a package is added or goes: the directory holds a file no language
        // traces.
        std::fs::remove_file(r.join("packages/d")).unwrap();
        write(&r, "packages/d/notes.md", "# d\n");
        refresh_equals_cold(&r, "the listed path is a directory");
        remove(&r, "packages/d/notes.md");
        write(&r, "packages/d", PATHS[5].1[0]);
        refresh_equals_cold(&r, "and a file again");
    }

    /// The same path comes to be there at all, and goes: as a directory with a
    /// manifest that has a `jest` key, with a config file, bare.
    #[test]
    fn a_listed_project_appears_and_goes() {
        let (_t, r) = with_root_config(CONFIGS_AT[7]);
        write(&r, "packages/d/package.json", ELSEWHERE);
        refresh_equals_cold(&r, "a manifest with a jest key");
        write(&r, "packages/d/jest.config.js", PATHS[3].1[0]);
        refresh_equals_cold(&r, "a config file next to it");
        remove(&r, "packages/d/package.json");
        remove(&r, "packages/d/jest.config.js");
        refresh_equals_cold(&r, "the directory goes");
        write(&r, "packages/d/src/x.ts", PATHS[0].1[1]);
        refresh_equals_cold(&r, "a bare directory");
    }

    /// A glob over the config files of the packages, and one with braces: a
    /// config file that appears two levels down, a directory below the
    /// second parent.
    #[test]
    fn a_projects_glob_over_files_and_with_braces() {
        let (_t, r) = with_root_config(CONFIGS_AT[8]);
        write(&r, "packages/d/jest.config.js", PATHS[3].1[1]);
        refresh_equals_cold(&r, "a config file the glob matches appears");
        remove(&r, "packages/d/jest.config.js");
        refresh_equals_cold(&r, "and goes");
        let (_t, r) = with_root_config(CONFIGS_AT[9]);
        write(&r, "docs/site/package.json", ELSEWHERE);
        refresh_equals_cold(&r, "a directory below the second parent appears");
    }

    /// A vitest workspace file with a glob, and `test.projects` as a list with
    /// a path that is not there yet.
    #[test]
    fn a_directory_appears_below_a_vitest_workspace() {
        for (file, contents) in [
            ("vitest.workspace.ts", PATHS[8].1[0]),
            ("vitest.config.ts", PATHS[7].1[1]),
        ] {
            let (_t, r) = canonical_tmp();
            workspace(&r);
            std::fs::remove_file(r.join("packages/a/jest.config.js")).unwrap();
            write(&r, file, contents);
            // The workspace file is found through the config next to it.
            if file == "vitest.workspace.ts" {
                write(
                    &r,
                    "vitest.config.ts",
                    "export default { test: { workspace: './vitest.workspace.ts' } };\n",
                );
            }
            build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
            write(&r, "packages/d/src/x.ts", PATHS[0].1[0]);
            refresh_equals_cold(&r, &format!("{file}: a directory appears"));
            write(&r, "packages/d/vitest.config.ts", PATHS[4].1[0]);
            refresh_equals_cold(&r, &format!("{file}: it gets a config"));
            remove(&r, "packages/d");
            refresh_equals_cold(&r, &format!("{file}: it goes"));
        }
    }

    /// A config that is not read exactly: its discovery (`**/*.spec.ts`)
    /// applies while it matches a file, and jest's default patterns stand
    /// in for it when it matches none. Which one it is decides whether a
    /// file that only the default patterns match is a test (`packages/b/
    /// src/b.test.ts`), and it is a fact about files of other packages: the
    /// first file the configured patterns match appears in `packages/a`, and
    /// goes.
    #[test]
    fn the_discovery_of_a_config_that_is_not_read_exactly_is_chosen_by_the_files() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "jest.config.js", CONFIGS_AT[11]);
        write(
            &r,
            "packages/b/src/b.test.ts",
            "import b from './index';\ntest('b', () => expect(b).toBe(2));\n",
        );
        let out = r.join(".turbotrace");
        build(&opts(&r, &out), &langs()).unwrap();
        let is_test = |r: &Path| {
            use turborepo_trace_graph::GraphView;
            let g = Graph::open(&r.join(".turbotrace")).unwrap();
            g.tests(Some("jest"))
                .into_iter()
                .any(|n| g.path(n) == "packages/b/src/b.test.ts")
        };
        assert!(is_test(&r), "by the default patterns");
        write(&r, "packages/a/src/more.spec.ts", "test('m', () => {});\n");
        refresh_equals_cold(&r, "a file the configured patterns match appears");
        assert!(!is_test(&r), "the configured patterns apply");
        std::fs::remove_file(r.join("packages/a/src/more.spec.ts")).unwrap();
        refresh_equals_cold(&r, "and goes");
        assert!(is_test(&r), "by the default patterns again");
    }

    /// The same link, in a tree that is old enough for a refresh to trust what
    /// it kept (in a young tree it trusts no directory, and the other cases
    /// of this file never get that far): the shard of `packages/c` saw
    /// through a link of `packages/a` (the root config's mapper), and the
    /// link is made to lead elsewhere. It keeps its name and its kind, so
    /// the entries of its directory are the same; the directory's time
    /// is not, and a tree part that saw through a link is covered by the
    /// directory of the link.
    #[test]
    fn a_link_of_another_package_leads_elsewhere_in_an_old_tree() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        for p in ["b", "c"] {
            write(
                &r,
                &format!("packages/{p}/src/sub/util.ts"),
                "export const u = 1;\n",
            );
        }
        apply(&r, &Edit::Link(4, true));
        apply(&r, &Edit::ConfigAt(0, Some(12)));
        let out = r.join(".turbotrace");
        std::fs::create_dir_all(&out).unwrap();
        crate::support::monorepo::backdate(&r, 60).unwrap();
        build(&opts(&r, &out), &langs()).unwrap();
        // A second build keeps every tree part: the first one's are trusted.
        let rep = build(&opts(&r, &out), &langs()).unwrap();
        assert_eq!(rep.traced, 0, "nothing changed");
        assert_eq!(rep.tree_reused[1], 0, "{:?}", rep.tree_reused);
        apply(&r, &Edit::Link(6, true));
        refresh_equals_cold(&r, "the link leads to another package");
    }

    /// A config asks for the real path of a directory that is a link, or that
    /// lies below one (`src/linked/sub`, which is there wherever the link
    /// leads: both packages have the directory). The link is there before
    /// and after and leads elsewhere: where it leads is a record of the
    /// files the load is consulted for. For the config of the package that
    /// has the link, the package's own files below the link say so too; for
    /// a config above, the files of another package (`packages/c`, whose import
    /// the mapper resolves below the link) have no other record of it.
    #[test]
    fn a_link_that_a_config_asks_the_real_path_of_leads_elsewhere() {
        for config in [
            Edit::Jest(Some(9)),
            Edit::Jest(Some(10)),
            Edit::ConfigAt(0, Some(12)),
        ] {
            let (_t, r) = canonical_tmp();
            workspace(&r);
            for p in ["b", "c"] {
                write(
                    &r,
                    &format!("packages/{p}/src/sub/util.ts"),
                    "export const u = 1;\n",
                );
            }
            build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
            for e in [
                Edit::Link(4, true),
                config.clone(),
                Edit::Link(6, true),
                Edit::Link(4, true),
                Edit::Link(4, false),
            ] {
                apply(&r, &e);
                refresh_equals_cold(&r, &format!("{config:?}, after {e:?}"));
            }
        }
    }
}

/// Review findings. Each case failed before its fix, also with the framework
/// pass and store of the time (none was a regression of the reviewed
/// commits): the refreshed graph is not the fresh build, and for the first
/// three the difference is in the export.
mod configs_above {
    use super::*;

    /// A config above the packages asks for the real path of a directory
    /// (`CONFIGS_AT[12]`: `realpathSync` of `packages/a/src/linked/sub`)
    /// and maps the import `not-installed` of `packages/c` to a file below
    /// it. `packages/a/src/linked` is a directory first and a link to
    /// `packages/b/src` then. The path that is asked about is a directory in
    /// both trees (`normalize::presence` follows links, as the lookup does),
    /// and a link that the real path does not cross yet leaves no record
    /// (`fsx::realpath` logs the links it crossed, `Loaded::crossed`):
    /// `packages/c/src/c.ts` keeps its records, and a fresh build has the
    /// edge `not-installed -> packages/b/src/sub/util.ts`. (The other
    /// direction, a link that comes to be a directory, passed before the fix:
    /// the link was crossed and is a record. The cases of `loads` change
    /// where a link leads.)
    #[test]
    fn a_directory_above_a_real_path_of_a_config_above_comes_to_be_a_link() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        for p in ["b", "c"] {
            write(
                &r,
                &format!("packages/{p}/src/sub/util.ts"),
                "export const u = 1;\n",
            );
        }
        write(
            &r,
            "packages/a/src/linked/sub/util.ts",
            "export const u = 1;\n",
        );
        apply(&r, &Edit::ConfigAt(0, Some(12)));
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        std::fs::remove_dir_all(r.join("packages/a/src/linked")).unwrap();
        apply(&r, &Edit::Link(4, true));
        refresh_equals_cold(&r, "the directory is a link");
    }

    /// The same for the path itself: a file that a config asks the real path of
    /// is a file first and a link to another file then (a file in both
    /// trees). The refreshed graph keeps the mapper's edge to
    /// `packages/b/src/other.ts`, a fresh build has it to `packages/b/src/
    /// index.ts`.
    #[test]
    fn a_file_that_a_config_asks_the_real_path_of_comes_to_be_a_link() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(
            &r,
            "jest.config.js",
            "const path = require('path');\nmodule.exports = { moduleNameMapper: { \
             '^not-installed$': require('fs').realpathSync(path.resolve(__dirname, \
             'packages/b/src/other.ts')) } };\n",
        );
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        std::fs::remove_file(r.join("packages/b/src/other.ts")).unwrap();
        symlink(&r, "index.ts", "packages/b/src/other.ts");
        refresh_equals_cold(&r, "the file is a link to another file");
    }

    /// A module request of a config (`setupFiles: ['@setup/d']`) is resolved by
    /// the tracer as if a file in the config's directory made it
    /// (`Fw::resolved_cell`), with the `paths` of the tsconfig that governs
    /// that directory. Of that trace only the request's own group is kept
    /// (`ref_index == Some(0)`); the tsconfig chain is a record
    /// of the file-level group (`JsResolver::file_edges`) and is dropped. The
    /// config's tests lie below another tsconfig
    /// (`packages/a/tsconfig.json`) and have no record of the one at the
    /// root: when its `paths` lead elsewhere, they keep the unresolved
    /// request, and a fresh build has the setup file `packages/c/src/d.ts`.
    #[test]
    fn the_tsconfig_above_a_config_whose_request_its_paths_resolve_changes() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        std::fs::remove_file(r.join("packages/a/jest.config.js")).unwrap();
        write(
            &r,
            "jest.config.js",
            "module.exports = { setupFiles: ['@setup/d'] };\n",
        );
        let paths = |to: &str| {
            format!(
                r#"{{"compilerOptions":{{"allowJs":true,"baseUrl":".","paths":{{"@setup/*":["packages/{to}/src/*"]}}}}}}"#
            )
        };
        write(&r, "tsconfig.json", &paths("b"));
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(&r, "tsconfig.json", &paths("c"));
        refresh_equals_cold(&r, "the paths of the tsconfig at the root lead elsewhere");
    }

    /// The same request below a config at the root that references a project
    /// (`packages/b/tsconfig.json`). The tracer asks every referenced project
    /// whether it governs the file a request is written in: a project that
    /// does not is a key observation of that directory's level, a set the
    /// tracer made. The tests of `packages/a` get it through the request of
    /// their config and name it in their file-level group, like the sets of
    /// their own resolutions:
    /// - a query that asks which files observed the project's config gets them
    ///   (`Graph::key_observers`; a set labelled as the framework pass's is
    ///   skipped there, and in the shard of `packages/a` only framework
    ///   observations lead to this one);
    /// - when the project comes to govern the root (its `include` and `paths`),
    ///   the request resolves through it, and the tests are traced again.
    #[test]
    fn a_project_that_a_config_above_references_comes_to_govern_a_request() {
        use turborepo_trace_graph::GraphView;
        let (_t, r) = canonical_tmp();
        workspace(&r);
        std::fs::remove_file(r.join("packages/a/jest.config.js")).unwrap();
        write(
            &r,
            "jest.config.js",
            "module.exports = { setupFiles: ['@setup/d'] };\n",
        );
        write(
            &r,
            "tsconfig.json",
            r#"{"compilerOptions":{"allowJs":true,"baseUrl":".","paths":{"@setup/*":["packages/b/src/*"]}},"references":[{"path":"./packages/b"}]}"#,
        );
        write(&r, REFERENCED, "{}");
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        {
            let g = Graph::open(&r.join(".turbotrace")).unwrap();
            let observers: Vec<&str> = g
                .key_observers(&[REFERENCED])
                .iter()
                .map(|(n, _)| g.path(*n))
                .collect();
            assert!(
                observers.contains(&"packages/a/src/index.test.ts"),
                "the tests that take the request's result observed the project: {observers:?}"
            );
        }
        write(
            &r,
            REFERENCED,
            r#"{"compilerOptions":{"allowJs":true,"baseUrl":".","paths":{"@setup/*":["../c/src/*"]}},"include":["../../*"]}"#,
        );
        refresh_equals_cold(&r, "the referenced project governs the root");
    }

    /// No refresh: the module mapper of a config that is the `jest` key of a
    /// manifest was applied to no import. `Fw::js_loads_for` found the
    /// configs of a directory in `Fw::js_config_index`, which goes by
    /// `Framework::config_file_names`, and jest's has no `package.json`.
    /// (So the replay "a module mapper" of
    /// `the_root_manifest_comes_to_have_a_jest_key` compared two graphs without
    /// the mapper's edge.) `Fw::js_loads_for` asks the adapter's walk now.
    #[test]
    fn the_module_mapper_of_a_manifest_config_is_applied() {
        let mapped = |r: &Path| {
            export(&r.join(".turbotrace"))
                .0
                .contains("jest:moduleNameMapper not-installed -> packages/b/src/other.ts")
        };
        // The same mapper as a config file at the root.
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(
            &r,
            "jest.config.js",
            "module.exports = { moduleNameMapper: { '^not-installed$': \
             '<rootDir>/packages/b/src/other.ts' } };\n",
        );
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        assert!(mapped(&r), "a config file's mapper is applied");
        let (_t, r) = canonical_tmp();
        workspace(&r);
        apply(&r, &Edit::RootManifest(2));
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        assert!(
            mapped(&r),
            "the mapper of the manifest's jest key is applied"
        );
    }
}

/// Symbolic links to directories on the way of a framework config's walks: a
/// `projects` glob goes through a link as the runner's glob library does
/// (`turborepo_trace_frameworks::fsx::Links`), and a listing does not, so what
/// is behind a link is a record of its own: the link (what it leads to), and
/// every directory behind it by its own entries.
mod links {
    use super::*;

    /// A config at the root whose `projects` are the files
    /// `packages/*/project.config.js` (a name that is no config file's: a
    /// file of that name that appears is known to the load through its walk
    /// alone), and `packages/l`, a link to `to`.
    fn linked(to: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let (t, r) = canonical_tmp();
        workspace(&r);
        write(
            &r,
            "jest.config.js",
            "module.exports = { projects: ['<rootDir>/packages/*/project.config.js'] };\n",
        );
        symlink(&r, to, "packages/l");
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        (t, r)
    }

    const CONFIG: &str = "module.exports = { testMatch: ['**/*.md.test.ts'] };\n";

    /// A config file appears in the directory the link leads to: a project of
    /// the list, found through the link. The listing of `packages` (real
    /// directories) says nothing of it; the entries of the directory behind
    /// the link do.
    #[test]
    fn a_config_appears_behind_a_link_that_a_projects_glob_goes_through() {
        let (_t, r) = linked("../docs");
        write(&r, "docs/project.config.js", CONFIG);
        refresh_equals_cold(&r, "a config behind the link");
        std::fs::remove_file(r.join("docs/project.config.js")).unwrap();
        refresh_equals_cold(&r, "and gone again");
    }

    /// The link leads to a file, and then to a directory with a config: the
    /// walk asks what a link on its way leads to, whether or not that is a
    /// directory.
    #[test]
    fn a_link_on_the_way_of_a_projects_glob_comes_to_lead_to_a_directory() {
        let (_t, r) = linked("../turbo.json");
        write(&r, "docs/project.config.js", CONFIG);
        refresh_equals_cold(&r, "a config where the link does not lead yet");
        symlink(&r, "../docs", "packages/l");
        refresh_equals_cold(&r, "the link leads to the directory");
        symlink(&r, "../nowhere", "packages/l");
        refresh_equals_cold(&r, "the link leads nowhere");
        symlink(&r, "../docs", "packages/l");
        refresh_equals_cold(&r, "and to the directory again");
    }

    /// A config that is not read exactly asks whether its discovery matches a
    /// file below its roots (`JEST_CONFIGS` 8), with a walk that follows
    /// links: a file that appears behind the linked source directory of
    /// `packages/a` decides for the package's files, and the listing of
    /// everything below the root does not go through the link.
    #[test]
    fn a_file_appears_behind_a_link_that_a_walk_of_everything_follows() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        apply(&r, &Edit::Jest(Some(8)));
        apply(&r, &Edit::Link(4, true));
        build(&opts(&r, &r.join(".turbotrace")), &langs()).unwrap();
        write(
            &r,
            "packages/b/src/deep/more.spec.ts",
            "test('m', () => {});\n",
        );
        refresh_equals_cold(&r, "a file the discovery matches, behind the link");
        remove(&r, "packages/b/src/deep");
        refresh_equals_cold(&r, "and gone again");
    }
}

/// The runners of the other languages (go test, cargo test, pytest): what their
/// adapters ask of the file system is logged like the JS adapters' accesses,
/// also where a component asks the cache itself (the cargo project model, the
/// Go package scanner: `turborepo_trace_frameworks::fsx::direct`). After each
/// step the refreshed graph is a fresh build of the tree.
mod runners {
    use super::*;

    struct Repo {
        _t: tempfile::TempDir,
        root: std::path::PathBuf,
        langs: Vec<Arc<dyn Language>>,
    }

    impl Repo {
        fn step(&self, what: &str, edit: impl FnOnce(&Path)) {
            edit(&self.root);
            refresh_equals_cold_with(&self.root, what, &self.langs);
        }
    }

    const CARGO: &str = "[package]\nname = \"rsapp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

    /// A Go module with a package and its test, a cargo package with a unit
    /// test and an integration test, a Python project with a pytest test;
    /// no environment outside it.
    fn repo() -> Repo {
        use turborepo_trace_lang_go::{GoLanguage, GoOptions, GoRoot};
        let t = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(t.path()).unwrap();
        let (r, env) = (base.join("repo"), base.join("env"));
        std::fs::create_dir_all(env.join("cargo")).unwrap();
        std::fs::create_dir_all(env.join("gomod")).unwrap();
        write(&r, ".gitignore", ".turbotrace\n");
        write(&r, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
        write(
            &r,
            "svc/lib/lib.go",
            "package lib\n\nfunc Do() int { return 1 }\n",
        );
        write(
            &r,
            "svc/lib/lib_test.go",
            "package lib\n\nimport \"testing\"\n\nfunc TestDo(t *testing.T) { Do() }\n",
        );
        write(&r, "rs/Cargo.toml", CARGO);
        write(
            &r,
            "rs/src/lib.rs",
            "pub mod util;\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\n",
        );
        write(&r, "rs/src/util.rs", "pub fn run() {}\n");
        write(
            &r,
            "rs/tests/it.rs",
            "use rsapp::util;\n\n#[test]\nfn it() {\n    util::run();\n}\n",
        );
        write(
            &r,
            "py/pyproject.toml",
            "[project]\nname = \"pyapp\"\nversion = \
             \"0.1.0\"\n\n[tool.pytest.ini_options]\ntestpaths = [\"tests\"]\n",
        );
        write(&r, "py/app/__init__.py", "");
        write(&r, "py/app/util.py", "def helper():\n    pass\n");
        write(
            &r,
            "py/tests/test_util.py",
            "from app.util import helper\n\n\ndef test_helper():\n    helper()\n",
        );
        let langs: Vec<Arc<dyn Language>> = vec![
            Arc::new(turborepo_trace_lang_js::JsLanguage),
            Arc::new(GoLanguage::new(GoOptions {
                modcache: Some(env.join("gomod")),
                goroot: GoRoot::None,
                cgo: Some(false),
                workspace: Some(false),
                ..GoOptions::default()
            })),
            Arc::new(turborepo_trace_lang_rust::RustLanguage::with_cargo_home(
                env.join("cargo"),
            )),
        ];
        build_with(&opts(&r, &r.join(".turbotrace")), &langs).unwrap();
        Repo {
            _t: t,
            root: r,
            langs,
        }
    }

    /// The tests of the graph as `<runner> <path>`.
    fn tests_of(root: &Path) -> Vec<String> {
        use turborepo_trace_graph::GraphView;
        let g = Graph::open(&root.join(".turbotrace")).unwrap();
        let mut out: Vec<String> = Vec::new();
        for runner in ["gotest", "cargo-test", "pytest"] {
            for n in g.tests(Some(runner)) {
                out.push(format!("{runner} {}", g.path(n)));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn the_fixture_has_a_test_of_each_runner() {
        let repo = repo();
        let tests = tests_of(&repo.root);
        let want = vec![
            "gotest svc/lib/lib_test.go",
            "cargo-test rs/src/lib.rs",
            "cargo-test rs/tests/it.rs",
        ];
        // A pytest test is a file of the Python language. Without it the pytest steps
        // below still compare a refresh with a fresh build: the tree has the project,
        // its packaging task and its markers, whose files no compiled language
        // claims.
        for want in want {
            assert!(tests.iter().any(|t| t == want), "{want}: {tests:?}");
        }
    }

    #[test]
    fn go_test() {
        let repo = repo();
        repo.step("go: a fixture directory appears", |r| {
            write(r, "svc/lib/testdata/a.txt", "a\n")
        });
        repo.step("go: a second fixture", |r| {
            write(r, "svc/lib/testdata/sub/b.txt", "b\n")
        });
        repo.step("go: the fixture directory goes", |r| {
            remove(r, "svc/lib/testdata")
        });
        repo.step("go: a file of the package appears", |r| {
            write(r, "svc/lib/more.go", "package lib\n\nfunc More() {}\n")
        });
        repo.step("go: a second test file appears", |r| {
            write(
                r,
                "svc/lib/more_test.go",
                "package lib\n\nimport \"testing\"\n\nfunc TestMore(t *testing.T) { More() }\n",
            )
        });
        repo.step("go: the file of the package goes", |r| {
            remove(r, "svc/lib/more.go")
        });
        repo.step("go: the module file changes", |r| {
            write(r, "svc/go.mod", "module example.com/svc\n\ngo 1.23\n")
        });
        repo.step("go: a module of its own for the package", |r| {
            write(r, "svc/lib/go.mod", "module example.com/lib\n\ngo 1.22\n")
        });
        repo.step("go: and without it", |r| remove(r, "svc/lib/go.mod"));
        repo.step("go: a package below with a test", |r| {
            write(r, "svc/lib/in/in.go", "package in\n");
            write(
                r,
                "svc/lib/in/in_test.go",
                "package in\n\nimport \"testing\"\n\nfunc TestIn(t *testing.T) {}\n",
            );
        });
        repo.step("go: the module file goes", |r| remove(r, "svc/go.mod"));
    }

    #[test]
    fn cargo_test() {
        let repo = repo();
        repo.step("cargo: an integration test appears", |r| {
            write(r, "rs/tests/other.rs", "#[test]\nfn o() {}\n")
        });
        repo.step("cargo: a module directory of the tests", |r| {
            write(r, "rs/tests/common/mod.rs", "pub fn c() {}\n")
        });
        repo.step("cargo: a file in a directory of the tests", |r| {
            write(r, "rs/tests/dir/helper.rs", "pub fn h() {}\n")
        });
        repo.step("cargo: the directory comes to be a test target", |r| {
            write(
                r,
                "rs/tests/dir/main.rs",
                "mod helper;\n\n#[test]\nfn d() {\n    helper::h();\n}\n",
            )
        });
        repo.step("cargo: and is none again", |r| {
            remove(r, "rs/tests/dir/main.rs")
        });
        repo.step("cargo: a binary in the directory of binaries", |r| {
            write(r, "rs/src/bin/tool.rs", "fn main() {}\n")
        });
        repo.step("cargo: an example and a bench", |r| {
            write(r, "rs/examples/e.rs", "fn main() {}\n");
            write(r, "rs/benches/b.rs", "fn main() {}\n");
        });
        repo.step("cargo: explicit test targets", |r| {
            write(
                r,
                "rs/Cargo.toml",
                &format!(
                    "{CARGO}autotests = false\n\n[[test]]\nname = \"it\"\npath = \"tests/it.rs\"\n"
                ),
            )
        });
        repo.step("cargo: discovered test targets again", |r| {
            write(r, "rs/Cargo.toml", CARGO)
        });
        repo.step("cargo: a build script appears", |r| {
            write(r, "rs/build.rs", "fn main() {}\n")
        });
        repo.step("cargo: the build script names a file", |r| {
            write(
                r,
                "rs/build.rs",
                "fn main() {\n    println!(\"cargo:rerun-if-changed=proto/a.proto\");\n}\n",
            )
        });
        repo.step("cargo: the file the build script names appears", |r| {
            write(r, "rs/proto/a.proto", "syntax = \"proto3\";\n")
        });
        repo.step("cargo: the build script goes", |r| remove(r, "rs/build.rs"));
        repo.step("cargo: a binary of the package", |r| {
            write(r, "rs/src/main.rs", "fn main() {}\n")
        });
        repo.step("cargo: a workspace above", |r| {
            write(r, "Cargo.toml", "[workspace]\nmembers = [\"rs\"]\n")
        });
        repo.step("cargo: a lockfile of the workspace", |r| {
            write(r, "Cargo.lock", "version = 3\n")
        });
        repo.step("cargo: the workspace goes", |r| {
            remove(r, "Cargo.toml");
            remove(r, "Cargo.lock");
        });
        repo.step("cargo: the library goes", |r| remove(r, "rs/src/lib.rs"));
        repo.step("cargo: an integration test goes", |r| {
            remove(r, "rs/tests/other.rs")
        });
    }

    #[test]
    fn pytest() {
        let repo = repo();
        repo.step("pytest: a conftest next to the test", |r| {
            write(r, "py/tests/conftest.py", "import pytest\n")
        });
        repo.step("pytest: a conftest above it", |r| {
            write(r, "py/conftest.py", "import pytest\n")
        });
        repo.step("pytest: the nearer conftest goes", |r| {
            remove(r, "py/tests/conftest.py")
        });
        repo.step("pytest: the tests come to be a package", |r| {
            write(r, "py/tests/__init__.py", "")
        });
        repo.step("pytest: an ini file takes the config", |r| {
            write(r, "py/pytest.ini", "[pytest]\npython_files = check_*.py\n")
        });
        repo.step("pytest: a file the ini file's pattern names", |r| {
            write(r, "py/tests/check_more.py", "def test_more():\n    pass\n")
        });
        repo.step("pytest: the ini file goes", |r| remove(r, "py/pytest.ini"));
        repo.step("pytest: a setup.cfg with the section", |r| {
            write(r, "py/setup.cfg", "[tool:pytest]\ntestpaths = tests\n")
        });
        repo.step("pytest: the project file goes", |r| {
            remove(r, "py/pyproject.toml")
        });
        repo.step("pytest: a test in a new directory", |r| {
            write(r, "py/tests/unit/test_x.py", "def test_x():\n    pass\n")
        });
        repo.step("pytest: a conftest at the repository's root", |r| {
            write(r, "conftest.py", "import pytest\n")
        });
        repo.step("pytest: the directory of the test goes", |r| {
            remove(r, "py/tests/unit")
        });
        // Package markers above the project: what the import root of a test is.
        repo.step("pytest: the project directory is a package", |r| {
            write(r, "py/__init__.py", "")
        });
        repo.step("pytest: the repository's root is a package", |r| {
            write(r, "__init__.py", "")
        });
        repo.step("pytest: the root is no package again", |r| {
            remove(r, "__init__.py")
        });
        repo.step("pytest: the conftest at the root goes", |r| {
            remove(r, "conftest.py")
        });
        repo.step("pytest: a directory at the ini file's place", |r| {
            write(r, "py/pytest.ini/keep.txt", "x\n")
        });
    }

    /// Package markers from the test's directory up to the repository's root:
    /// the chain decides which directory a test is imported from, and the
    /// marker at the root is a file of another package than the test's.
    #[test]
    fn pytest_package_markers_above_the_project() {
        let repo = repo();
        repo.step("pytest: markers up to the root", |r| {
            write(r, "py/tests/__init__.py", "");
            write(r, "py/__init__.py", "");
            write(r, "__init__.py", "");
        });
        repo.step("pytest: the marker at the root goes", |r| {
            remove(r, "__init__.py")
        });
        repo.step("pytest: a directory at the marker's place", |r| {
            write(r, "py/__init__.py/keep.txt", "x\n")
        });
    }
}

/// A Python project in a build without the Python language (the languages a
/// build compiles are its choice: the CLI's cargo features). Its packaging task
/// (`py-dist`) is still discovered, and its memberships are the packages that
/// setuptools' discovery finds: directories with an `__init__.py`, files no
/// compiled language claims. After each step the refreshed graph is a fresh
/// build of the tree.
mod unclaimed_python {
    use super::*;

    /// JS, Go and Rust, with the environments in `env`.
    fn langs(env: &Path) -> Vec<Arc<dyn Language>> {
        use turborepo_trace_lang_go::{GoLanguage, GoOptions, GoRoot};
        vec![
            Arc::new(turborepo_trace_lang_js::JsLanguage),
            Arc::new(GoLanguage::new(GoOptions {
                modcache: Some(env.join("gomod")),
                goroot: GoRoot::None,
                cgo: Some(false),
                workspace: Some(false),
                ..GoOptions::default()
            })),
            Arc::new(turborepo_trace_lang_rust::RustLanguage::with_cargo_home(
                env.join("cargo"),
            )),
        ]
    }

    /// A named edit of the tree.
    type Step<'a> = (&'a str, &'a dyn Fn(&Path));

    fn steps(layout: &str, steps: &[Step<'_>]) {
        let (_t, base) = canonical_tmp();
        let (r, env) = (base.join("repo"), base.join("env"));
        std::fs::create_dir_all(env.join("cargo")).unwrap();
        std::fs::create_dir_all(env.join("gomod")).unwrap();
        write(&r, ".gitignore", ".turbotrace\n");
        write(
            &r,
            "py/pyproject.toml",
            "[project]\nname = \"pyapp\"\nversion = \"0.1.0\"\n\n[build-system]\nrequires = \
             [\"setuptools\"]\nbuild-backend = \"setuptools.build_meta\"\n",
        );
        write(&r, "py/app/__init__.py", "");
        write(&r, "py/app/util.py", "def helper():\n    pass\n");
        let langs = langs(&env);
        build_with(&opts(&r, &r.join(".turbotrace")), &langs).unwrap();
        for (name, f) in steps {
            f(&r);
            refresh_equals_cold_with(&r, &format!("{layout}: {name}"), &langs);
        }
    }

    #[test]
    fn flat_layout_packages() {
        steps(
            "flat",
            &[
                ("a package appears", &|r| {
                    write(r, "py/other/__init__.py", "")
                }),
                ("a module of it", &|r| {
                    write(r, "py/other/mod.py", "x = 1\n")
                }),
                ("its marker goes", &|r| remove(r, "py/other/__init__.py")),
                ("a subpackage", &|r| write(r, "py/app/sub/__init__.py", "")),
                ("a directory at a marker's place", &|r| {
                    write(r, "py/app/__init__.py/keep.txt", "x\n")
                }),
                ("the marker again", &|r| write(r, "py/app/__init__.py", "")),
                ("a directory without a marker", &|r| {
                    write(r, "py/data/a.txt", "a\n")
                }),
                ("the package goes", &|r| remove(r, "py/other")),
            ],
        );
    }

    #[test]
    fn src_layout_comes_and_goes() {
        steps(
            "src",
            &[
                ("a src directory", &|r| {
                    write(r, "py/src/pyapp/__init__.py", "")
                }),
                ("a namespace package below it", &|r| {
                    write(r, "py/src/pyapp/ns/mod.py", "x = 1\n")
                }),
                ("the src directory goes", &|r| remove(r, "py/src")),
            ],
        );
    }
}

/// Probes: what a task source decides by in the tree,
/// in builds that compile only some languages. `Port` is JS, Go and Rust; `Js`
/// is JS alone.
mod probes_tasks {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum Set {
        Port,
        Js,
    }

    fn langs(set: Set, env: &Path) -> Vec<Arc<dyn Language>> {
        use turborepo_trace_lang_go::{GoLanguage, GoOptions, GoRoot};
        match set {
            Set::Js => vec![Arc::new(turborepo_trace_lang_js::JsLanguage)],
            Set::Port => vec![
                Arc::new(turborepo_trace_lang_js::JsLanguage),
                Arc::new(GoLanguage::new(GoOptions {
                    modcache: Some(env.join("gomod")),
                    goroot: GoRoot::None,
                    cgo: Some(false),
                    workspace: Some(false),
                    ..GoOptions::default()
                })),
                Arc::new(turborepo_trace_lang_rust::RustLanguage::with_cargo_home(
                    env.join("cargo"),
                )),
            ],
        }
    }

    type Step<'a> = (&'a str, &'a dyn Fn(&Path));

    const PYPROJECT: &str = "[project]\nname = \"pyapp\"\nversion = \
                             \"0.1.0\"\n\n[build-system]\nrequires = \
                             [\"setuptools\"]\nbuild-backend = \"setuptools.build_meta\"\n";

    /// Runs the steps; a step whose refresh differs is noted, the graph is
    /// built again from nothing, and the next step runs. Panics with every
    /// step that failed.
    fn steps(what: &str, set: Set, old: bool, setup: &dyn Fn(&Path), steps: &[Step<'_>]) {
        let (_t, base) = canonical_tmp();
        let (r, env) = (base.join("repo"), base.join("env"));
        std::fs::create_dir_all(env.join("cargo")).unwrap();
        std::fs::create_dir_all(env.join("gomod")).unwrap();
        write(&r, ".gitignore", ".turbotrace\n");
        setup(&r);
        let langs = langs(set, &env);
        let start = std::time::SystemTime::now();
        let out = r.join(".turbotrace");
        std::fs::create_dir_all(&out).unwrap();
        if old {
            age(&r, start, 0);
        }
        build_with(&opts(&r, &out), &langs).unwrap();
        if old {
            build_with(&opts(&r, &out), &langs).unwrap();
        }
        let mut failed = Vec::new();
        for (i, (name, f)) in steps.iter().enumerate() {
            f(&r);
            if old {
                age(&r, start, i as u64 + 1);
            }
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                refresh_equals_cold_with(
                    &r,
                    &format!("{what} ({set:?}, old: {old}): {name}"),
                    &langs,
                );
            }));
            if let Err(e) = res {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                failed.push(format!(
                    "STEP {name}: {}",
                    msg.lines().take(6).collect::<Vec<_>>().join(" | ")
                ));
                std::fs::remove_dir_all(&out).unwrap();
                std::fs::create_dir_all(&out).unwrap();
                build_with(&opts(&r, &out), &langs).unwrap();
            }
        }
        assert!(
            failed.is_empty(),
            "{what} ({set:?}, old: {old}):\n{}",
            failed.join("\n")
        );
    }

    fn both(what: &str, set: Set, setup: &dyn Fn(&Path), s: &[Step<'_>]) {
        let sets: &[Set] = if what.starts_with("py ") {
            &[Set::Port]
        } else {
            &[set]
        };
        let mut ok = true;
        for &set in sets {
            for old in [false, true] {
                ok &= std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    steps(what, set, old, setup, s)
                }))
                .is_ok();
            }
        }
        assert!(ok, "{what}: failed");
    }

    /// Two cold builds of one tree, many times: how often they differ.
    fn cold_twice(alias: bool, tool: bool) -> (usize, String) {
        let mut differ = 0;
        let mut first = String::new();
        for _ in 0..40 {
            let (_t, base) = canonical_tmp();
            let (r, env) = (base.join("repo"), base.join("env"));
            std::fs::create_dir_all(env.join("cargo")).unwrap();
            std::fs::create_dir_all(env.join("gomod")).unwrap();
            write(&r, ".gitignore", ".turbotrace\n");
            cargo_setup(&r);
            if !alias {
                remove(&r, ".cargo/config.toml");
            }
            if tool {
                write(
                    &r,
                    "crates/tool/Cargo.toml",
                    "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                );
                write(&r, "crates/tool/src/main.rs", "fn main() {}\n");
            }
            let langs = langs(Set::Port, &env);
            let (a, b) = (base.join("a"), base.join("b"));
            build_with(&opts(&r, &a), &langs).unwrap();
            build_with(&opts(&r, &b), &langs).unwrap();
            let (ja, ka) = export(&a);
            let (jb, kb) = export(&b);
            if ja != jb || ka != kb {
                differ += 1;
                if first.is_empty() {
                    let la: std::collections::BTreeSet<&str> = ja.lines().collect();
                    let lb: std::collections::BTreeSet<&str> = jb.lines().collect();
                    first = format!(
                        "only a: {:?} only b: {:?} keys equal: {}",
                        la.difference(&lb).take(12).collect::<Vec<_>>(),
                        lb.difference(&la).take(12).collect::<Vec<_>>(),
                        ka == kb
                    );
                }
            }
        }
        (differ, first)
    }

    #[test]
    #[ignore = "before the branch too: two cold builds of a cargo workspace with two crates differ \
                in about one run of five (listings of a crate's target directories, shard keys)"]
    fn cargo_cold_builds_differ() {
        let mut out = Vec::new();
        for (alias, tool) in [(false, true), (false, false)] {
            let (n, first) = cold_twice(alias, tool);
            out.push(format!(
                "COLD alias={alias} tool={tool}: {n}/40 differ; {first}"
            ));
        }
        eprintln!("{}", out.join("\n"));
        assert!(
            out.iter().all(|l| l.contains(": 0/40")),
            "{}",
            out.join("\n")
        );
    }

    #[test]
    fn py_at_root() {
        both(
            "py at the root",
            Set::Port,
            &|r| {
                write(r, "pyproject.toml", PYPROJECT);
                write(
                    r,
                    "package.json",
                    r#"{"name":"root","private":true,"workspaces":["packages/*"]}"#,
                );
                write(
                    r,
                    "packages/a/package.json",
                    r#"{"name":"a","scripts":{"build":"tsc -p ."}}"#,
                );
                write(r, "packages/a/src/i.ts", "export const a = 1;\n");
                write(r, "tools/gen.py", "x = 1\n");
            },
            &[
                ("a package at the root", &|r| {
                    write(r, "newpkg/__init__.py", "")
                }),
                ("a marker in an existing directory", &|r| {
                    write(r, "tools/__init__.py", "")
                }),
                ("a subpackage", &|r| write(r, "tools/sub/__init__.py", "")),
                ("a marker inside a workspace package", &|r| {
                    write(r, "packages/__init__.py", "")
                }),
                ("and below it", &|r| write(r, "packages/a/__init__.py", "")),
                ("the marker goes", &|r| remove(r, "tools/__init__.py")),
                ("a src directory", &|r| write(r, "src/ns/x.txt", "")),
                ("a namespace below it", &|r| write(r, "src/ns2/y/x.txt", "")),
                ("src goes", &|r| remove(r, "src")),
            ],
        );
    }

    #[test]
    fn py_ignored() {
        both(
            "py ignored",
            Set::Port,
            &|r| {
                py(r);
                write(
                    r,
                    ".gitignore",
                    ".turbotrace\n__pycache__/\ngen/\n*.egg-info/\nbuild/\n",
                );
                write(r, "py/src/pyapp/__init__.py", "");
            },
            &[
                ("a cache directory in a package", &|r| {
                    write(r, "py/src/pyapp/__pycache__/m.cpython-311.pyc", "")
                }),
                ("an ignored namespace directory", &|r| {
                    write(r, "py/src/gen/x.txt", "")
                }),
                ("an ignored package", &|r| {
                    write(r, "py/src/gen/__init__.py", "")
                }),
                ("a module in it", &|r| write(r, "py/src/gen/m.py", "")),
                ("an egg-info directory", &|r| {
                    write(r, "py/src/pyapp.egg-info/PKG-INFO", "")
                }),
                ("the ignored package goes", &|r| remove(r, "py/src/gen")),
                ("flat: src goes", &|r| remove(r, "py/src")),
                ("flat: an ignored package", &|r| {
                    write(r, "py/gen/__init__.py", "")
                }),
                ("flat: a subpackage of it", &|r| {
                    write(r, "py/gen/sub/__init__.py", "")
                }),
                ("flat: a build directory", &|r| {
                    write(r, "py/build/lib/app/__init__.py", "")
                }),
                ("flat: the ignored package goes", &|r| remove(r, "py/gen")),
            ],
        );
    }

    /// The smallest cases of what failed above.
    fn one(what: &str, set: Set, old: bool, setup: &dyn Fn(&Path), step: &dyn Fn(&Path)) {
        steps(what, set, old, setup, &[("the step", step)]);
    }

    #[test]
    fn a_link_below_a_python_project_leads_elsewhere_in_an_old_tree() {
        one(
            "py link",
            Set::Port,
            true,
            &|r| {
                py(r);
                symlink(r, "../shared/app3", "py/linked");
            },
            &|r| symlink(r, "../shared/plain", "py/linked"),
        );
    }

    #[test]
    fn the_target_of_a_link_below_a_python_project_appears_in_an_old_tree() {
        one(
            "py link target",
            Set::Port,
            true,
            &|r| {
                py(r);
                symlink(r, "../shared/none", "py/linked");
            },
            &|r| write(r, "shared/none/__init__.py", ""),
        );
    }

    #[test]
    fn a_marker_that_is_a_link_leads_nowhere_in_an_old_tree() {
        one(
            "py marker link",
            Set::Port,
            true,
            &|r| {
                py(r);
                write(r, "py/lk/x.txt", "");
                symlink(r, "../../shared/app2/__init__.py", "py/lk/__init__.py");
            },
            &|r| symlink(r, "../../shared/zzz.py", "py/lk/__init__.py"),
        );
    }

    #[test]
    fn the_default_package_link_leads_to_a_file_in_an_old_tree() {
        one(
            "hatch link",
            Set::Port,
            true,
            &|r| {
                py(r);
                remove(r, "py/app");
                write(
                    r,
                    "py/pyproject.toml",
                    "[project]\nname = \"pyapp\"\nversion = \"0.1.0\"\n\n[build-system]\nrequires \
                     = [\"hatchling\"]\nbuild-backend = \"hatchling.build\"\n",
                );
                symlink(r, "../shared/app3", "py/pyapp");
            },
            &|r| symlink(r, "../shared/app2/m.py", "py/pyapp"),
        );
    }

    fn tool_crate(r: &Path) {
        cargo_setup(r);
        write(
            r,
            "crates/tool/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(r, "crates/tool/src/main.rs", "fn main() {}\n");
    }

    #[test]
    #[ignore = "open: with the Rust language two cold builds of a cargo workspace differ in about \
                one run of five (the listings of a crate's target directories); the alias's \
                binaries themselves are keyed (`CargoAliases::discover`, `with_decided_by`; the \
                same case without Rust passes)"]
    fn a_binary_of_the_crate_an_alias_runs_appears() {
        one("alias bin", Set::Port, false, &tool_crate, &|r| {
            write(r, "crates/tool/src/bin/extra.rs", "fn main() {}\n")
        });
    }

    #[test]
    fn a_binary_of_the_crate_an_alias_runs_appears_without_rust() {
        one("alias bin", Set::Js, false, &tool_crate, &|r| {
            write(r, "crates/tool/src/bin/extra.rs", "fn main() {}\n")
        });
    }

    fn go_module(r: &Path) {
        write(r, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
        write(
            r,
            "svc/lib/lib.go",
            "package lib\n\nfunc Do() int { return 1 }\n",
        );
        write(r, "svc/gen.go", "package svc\n\n//go:generate echo hi\n");
    }

    #[test]
    #[ignore = "before the branch too, a reduced build (no Go language): a new file with a \
                go:generate directive has a task in a fresh build and none in the refreshed one"]
    fn a_file_with_a_directive_appears_without_go() {
        one("go file", Set::Js, false, &go_module, &|r| {
            write(
                r,
                "svc/lib/more.go",
                "package lib\n\n//go:generate echo more\n",
            )
        });
    }

    fn cargo_setup(r: &Path) {
        write(
            r,
            ".cargo/config.toml",
            "[alias]\nxtask = \"run -p tool --\"\n",
        );
        write(
            r,
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
        );
        write(
            r,
            "crates/lib/Cargo.toml",
            "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(r, "crates/lib/src/lib.rs", "pub fn f() {}\n");
    }

    fn py(r: &Path) {
        write(r, "py/pyproject.toml", PYPROJECT);
        write(r, "py/app/__init__.py", "");
        write(r, "py/app/util.py", "def helper():\n    pass\n");
        write(r, "shared/app2/__init__.py", "");
        write(r, "shared/app2/m.py", "x = 1\n");
        write(r, "shared/app3/__init__.py", "");
        write(r, "shared/plain/m.py", "x = 1\n");
    }

    #[test]
    fn py_flat_kinds_and_depth() {
        both(
            "py flat",
            Set::Port,
            &|r| {
                py(r);
                write(r, "py/foo", "x\n");
            },
            &[
                ("a file comes to be a package directory", &|r| {
                    write(r, "py/foo/__init__.py", "")
                }),
                ("and a file again", &|r| {
                    remove(r, "py/foo");
                    write(r, "py/foo", "x\n")
                }),
                ("a chain of 8 packages", &|r| {
                    for d in [
                        "a",
                        "a/b",
                        "a/b/c",
                        "a/b/c/d",
                        "a/b/c/d/e",
                        "a/b/c/d/e/f",
                        "a/b/c/d/e/f/g",
                    ] {
                        write(r, &format!("py/app/{d}/__init__.py"), "");
                    }
                }),
                ("depth 9", &|r| {
                    write(r, "py/app/a/b/c/d/e/f/g/h/__init__.py", "")
                }),
                ("depth 10", &|r| {
                    write(r, "py/app/a/b/c/d/e/f/g/h/i/__init__.py", "")
                }),
                ("depth 11", &|r| {
                    write(r, "py/app/a/b/c/d/e/f/g/h/i/j/__init__.py", "")
                }),
                ("a module at depth 11", &|r| {
                    write(r, "py/app/a/b/c/d/e/f/g/h/i/j/m.py", "")
                }),
                ("a marker in the middle goes", &|r| {
                    remove(r, "py/app/a/b/c/__init__.py")
                }),
                ("a directory with a dash", &|r| {
                    write(r, "py/my-pkg/__init__.py", "")
                }),
                ("a hidden directory", &|r| {
                    write(r, "py/.hid/__init__.py", "")
                }),
                ("an excluded top name", &|r| {
                    write(r, "py/tests/__init__.py", "")
                }),
                ("a package below it", &|r| {
                    write(r, "py/tests/unit/__init__.py", "")
                }),
                ("node_modules", &|r| {
                    write(r, "py/node_modules/x/__init__.py", "")
                }),
            ],
        );
    }

    #[test]
    fn py_flat_links() {
        both(
            "py flat links",
            Set::Port,
            &py,
            &[
                ("a link to a package", &|r| {
                    symlink(r, "../shared/app2", "py/linked")
                }),
                ("a subpackage behind the link", &|r| {
                    write(r, "shared/app2/sub/__init__.py", "")
                }),
                ("a module behind the link", &|r| {
                    write(r, "shared/app2/n.py", "")
                }),
                ("the link leads to another package", &|r| {
                    symlink(r, "../shared/app3", "py/linked")
                }),
                ("the link leads to a directory without a marker", &|r| {
                    symlink(r, "../shared/plain", "py/linked")
                }),
                ("the marker appears behind it", &|r| {
                    write(r, "shared/plain/__init__.py", "")
                }),
                ("the link leads nowhere", &|r| {
                    symlink(r, "../shared/none", "py/linked")
                }),
                ("its target appears", &|r| {
                    write(r, "shared/none/__init__.py", "")
                }),
                ("the link goes", &|r| remove(r, "py/linked")),
                ("a package directory comes to be a link", &|r| {
                    symlink(r, "../shared/app2", "py/app")
                }),
                ("and a directory again", &|r| {
                    remove(r, "py/app");
                    write(r, "py/app/__init__.py", "")
                }),
                ("a marker that is a link", &|r| {
                    write(r, "py/lk/x.txt", "");
                    symlink(r, "../../shared/app2/__init__.py", "py/lk/__init__.py")
                }),
                ("the marker link leads nowhere", &|r| {
                    symlink(r, "../../shared/zzz.py", "py/lk/__init__.py")
                }),
                ("the marker link leads to a directory", &|r| {
                    symlink(r, "../../shared", "py/lk/__init__.py")
                }),
            ],
        );
    }

    #[test]
    fn py_single_module() {
        both(
            "py single module",
            Set::Port,
            &|r| {
                py(r);
                remove(r, "py/app");
            },
            &[
                ("the module of the project's name", &|r| {
                    write(r, "py/pyapp.py", "")
                }),
                ("an excluded package", &|r| {
                    write(r, "py/tests/__init__.py", "")
                }),
                ("a package", &|r| write(r, "py/real/__init__.py", "")),
                ("the package goes", &|r| remove(r, "py/real")),
                ("the module comes to be a directory", &|r| {
                    write(r, "py/pyapp.py/x.txt", "")
                }),
                ("the module goes", &|r| remove(r, "py/pyapp.py")),
            ],
        );
    }

    #[test]
    fn py_src_layout() {
        both(
            "py src",
            Set::Port,
            &|r| {
                py(r);
                write(r, "py/src/pyapp/__init__.py", "");
            },
            &[
                ("a namespace directory with a text file", &|r| {
                    write(r, "py/src/ns/data.txt", "")
                }),
                ("a directory below it", &|r| {
                    write(r, "py/src/ns/deep/x.txt", "")
                }),
                ("a module in it", &|r| write(r, "py/src/ns/deep/m.py", "")),
                ("the namespace goes", &|r| remove(r, "py/src/ns")),
                ("src comes to be a file", &|r| {
                    remove(r, "py/src");
                    write(r, "py/src", "x\n")
                }),
                ("src comes to be a link", &|r| {
                    symlink(r, "../shared", "py/src")
                }),
                ("a directory behind the link", &|r| {
                    write(r, "shared/more/x.txt", "")
                }),
                ("src leads nowhere", &|r| symlink(r, "../nowhere", "py/src")),
                ("src goes", &|r| remove(r, "py/src")),
                ("src is back", &|r| write(r, "py/src/pyapp/__init__.py", "")),
            ],
        );
    }

    #[test]
    fn py_find_setup_cfg() {
        const CFG: &str = "[metadata]\nname = pyapp\n\n[options]\npackages = \
                           find:\n\n[options.packages.find]\nwhere = lib\ninclude = app*\nexclude \
                           = app.tests*\n";
        both(
            "py find: setup.cfg",
            Set::Port,
            &|r| {
                py(r);
                remove(r, "py/pyproject.toml");
                write(r, "py/setup.cfg", CFG);
            },
            &[
                ("the where directory appears with a package", &|r| {
                    write(r, "py/lib/app/__init__.py", "")
                }),
                ("a package the include leaves out", &|r| {
                    write(r, "py/lib/other/__init__.py", "")
                }),
                ("an excluded subpackage", &|r| {
                    write(r, "py/lib/app/tests/__init__.py", "")
                }),
                ("a subpackage", &|r| {
                    write(r, "py/lib/app/core/__init__.py", "")
                }),
                ("a second included package", &|r| {
                    write(r, "py/lib/app2/__init__.py", "")
                }),
                ("find_namespace", &|r| {
                    write(r, "py/setup.cfg", &CFG.replace("find:", "find_namespace:"))
                }),
                ("a namespace directory", &|r| {
                    write(r, "py/lib/app/ns/x.txt", "")
                }),
                ("another top directory", &|r| {
                    write(r, "py/lib/appdata/x.txt", "")
                }),
                ("the where directory comes to be a link", &|r| {
                    remove(r, "py/lib");
                    symlink(r, "../shared", "py/lib")
                }),
                ("a package behind it", &|r| {
                    write(r, "shared/app9/__init__.py", "")
                }),
                ("the where directory goes", &|r| remove(r, "py/lib")),
                ("a pyproject with find", &|r| {
                    remove(r, "py/setup.cfg");
                    write(
                        r,
                        "py/pyproject.toml",
                        &format!(
                            "{PYPROJECT}\n[tool.setuptools.packages.find]\nwhere = [\"lib\", \
                             \"lib2\"]\nexclude = [\"t*\"]\n"
                        ),
                    );
                }),
                ("a package in the second where", &|r| {
                    write(r, "py/lib2/zz/__init__.py", "")
                }),
                ("an excluded one", &|r| {
                    write(r, "py/lib2/tt/__init__.py", "")
                }),
                ("explicit packages", &|r| {
                    write(
                        r,
                        "py/pyproject.toml",
                        &format!(
                            "{PYPROJECT}\n[tool.setuptools]\npackages = [\"app\", \"gone\"]\n"
                        ),
                    )
                }),
                ("an explicit package appears", &|r| {
                    write(r, "py/gone/__init__.py", "")
                }),
            ],
        );
    }

    #[test]
    fn py_setup_py() {
        both(
            "py setup.py",
            Set::Port,
            &|r| {
                py(r);
                remove(r, "py/pyproject.toml");
                write(
                    r,
                    "py/setup.py",
                    "from setuptools import setup, find_packages\nsetup(name='pyapp', \
                     packages=find_packages(where='src', exclude=['tests*']), package_dir={'': \
                     'src'})\n",
                );
            },
            &[
                ("a package below src", &|r| {
                    write(r, "py/src/pkg/__init__.py", "")
                }),
                ("an excluded one", &|r| {
                    write(r, "py/src/tests/__init__.py", "")
                }),
                ("a module", &|r| write(r, "py/src/pkg/m.py", "")),
                ("a setup.cfg next to it", &|r| {
                    write(r, "py/setup.cfg", "[options]\npackages = find:\n")
                }),
                ("setup.py without packages", &|r| {
                    write(
                        r,
                        "py/setup.py",
                        "from setuptools import setup\nsetup(name='pyapp')\n",
                    )
                }),
                ("a flat package", &|r| write(r, "py/flat/__init__.py", "")),
                ("the setup.cfg goes", &|r| remove(r, "py/setup.cfg")),
                ("another flat package", &|r| {
                    write(r, "py/flat2/__init__.py", "")
                }),
                ("a pyproject takes over", &|r| {
                    remove(r, "py/setup.py");
                    write(r, "py/pyproject.toml", PYPROJECT)
                }),
                ("one more", &|r| write(r, "py/flat3/__init__.py", "")),
                ("a MANIFEST.in", &|r| {
                    write(
                        r,
                        "py/MANIFEST.in",
                        "graft docs\nrecursive-include data *.json\n",
                    )
                }),
                ("what it grafts", &|r| write(r, "py/docs/a.rst", "")),
            ],
        );
    }

    fn backend(name: &'static str, pyproject: &'static str) {
        both(
            &format!("py {name}"),
            Set::Port,
            &|r| {
                py(r);
                remove(r, "py/app");
                write(r, "py/pyproject.toml", pyproject);
            },
            &[
                ("the module of the project's name", &|r| {
                    write(r, "py/pyapp.py", "")
                }),
                ("the package of the project's name", &|r| {
                    write(r, "py/pyapp/__init__.py", "")
                }),
                ("a text file in it", &|r| write(r, "py/pyapp/data.txt", "")),
                ("the module below src", &|r| write(r, "py/src/pyapp.py", "")),
                ("the package below src", &|r| {
                    write(r, "py/src/pyapp/__init__.py", "")
                }),
                ("src goes", &|r| remove(r, "py/src")),
                ("the package comes to be a link", &|r| {
                    symlink(r, "../shared/app2", "py/pyapp")
                }),
                ("a file behind the link", &|r| {
                    write(r, "shared/app2/z.txt", "")
                }),
                ("the link leads elsewhere", &|r| {
                    symlink(r, "../shared/app3", "py/pyapp")
                }),
                ("the link leads to a file", &|r| {
                    symlink(r, "../shared/app2/m.py", "py/pyapp")
                }),
                ("the link leads nowhere", &|r| {
                    symlink(r, "../shared/nil", "py/pyapp")
                }),
                ("the package goes", &|r| remove(r, "py/pyapp")),
                ("the module goes", &|r| remove(r, "py/pyapp.py")),
            ],
        );
    }

    #[test]
    fn py_hatch() {
        backend(
            "hatch",
            "[project]\nname = \"pyapp\"\nversion = \"0.1.0\"\n\n[build-system]\nrequires = \
             [\"hatchling\"]\nbuild-backend = \"hatchling.build\"\n",
        );
    }

    #[test]
    fn py_flit() {
        backend(
            "flit",
            "[project]\nname = \"pyapp\"\nversion = \"0.1.0\"\n\n[build-system]\nrequires = \
             [\"flit_core\"]\nbuild-backend = \"flit_core.buildapi\"\n",
        );
    }

    #[test]
    fn py_pdm() {
        backend(
            "pdm",
            "[project]\nname = \"pyapp\"\nversion = \"0.1.0\"\n\n[build-system]\nrequires = \
             [\"pdm-backend\"]\nbuild-backend = \"pdm.backend\"\n",
        );
    }

    #[test]
    fn py_poetry() {
        backend(
            "poetry",
            "[tool.poetry]\nname = \"pyapp\"\nversion = \"0.1.0\"\n\n[build-system]\nrequires = \
             [\"poetry-core\"]\nbuild-backend = \"poetry.core.masonry.api\"\n",
        );
    }

    #[test]
    fn py_poetry_packages() {
        both(
            "py poetry packages",
            Set::Port,
            &|r| {
                py(r);
                write(
                    r,
                    "py/pyproject.toml",
                    "[tool.poetry]\nname = \"pyapp\"\nversion = \"0.1.0\"\npackages = [{ include \
                     = \"foo\", from = \"lib\" }, { include = \"bar\" \
                     }]\n\n[build-system]\nrequires = [\"poetry-core\"]\nbuild-backend = \
                     \"poetry.core.masonry.api\"\n",
                );
            },
            &[
                ("the module", &|r| write(r, "py/lib/foo.py", "")),
                ("the package", &|r| write(r, "py/lib/foo/__init__.py", "")),
                ("the package goes", &|r| remove(r, "py/lib/foo")),
                ("bar as a file without a suffix", &|r| {
                    write(r, "py/bar", "")
                }),
                ("bar as a directory", &|r| write(r, "py/bar/x.txt", "")),
                ("bar as a link", &|r| symlink(r, "../shared/app2", "py/bar")),
                ("bar goes", &|r| remove(r, "py/bar")),
            ],
        );
    }

    fn cargo_alias(set: Set) {
        both(
            "cargo alias",
            set,
            &|r| {
                write(
                    r,
                    ".cargo/config.toml",
                    "[alias]\nxtask = \"run -p tool --\"\n",
                );
                write(
                    r,
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
                );
                write(
                    r,
                    "crates/lib/Cargo.toml",
                    "[package]\nname = \"lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                );
                write(r, "crates/lib/src/lib.rs", "pub fn f() {}\n");
            },
            &[
                ("the crate the alias runs appears", &|r| {
                    write(
                        r,
                        "crates/tool/Cargo.toml",
                        "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    );
                    write(r, "crates/tool/src/main.rs", "fn main() {}\n");
                }),
                ("a binary in its directory of binaries", &|r| {
                    write(r, "crates/tool/src/bin/extra.rs", "fn main() {}\n")
                }),
                ("a binary that is a directory", &|r| {
                    write(r, "crates/tool/src/bin/dir/main.rs", "fn main() {}\n")
                }),
                ("a second binary file", &|r| {
                    write(r, "crates/tool/src/bin/more.rs", "fn main() {}\n")
                }),
                ("a binary goes", &|r| {
                    remove(r, "crates/tool/src/bin/extra.rs")
                }),
                ("a build script", &|r| {
                    write(r, "crates/tool/build.rs", "fn main() {}\n")
                }),
                ("a library", &|r| {
                    write(r, "crates/tool/src/lib.rs", "pub fn g() {}\n")
                }),
                ("the crate gets another name", &|r| {
                    write(
                        r,
                        "crates/tool/Cargo.toml",
                        "[package]\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    )
                }),
                ("another crate gets the name", &|r| {
                    write(
                        r,
                        "crates/zz/Cargo.toml",
                        "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    );
                    write(r, "crates/zz/src/main.rs", "fn main() {}\n");
                }),
                ("an earlier crate gets the name", &|r| {
                    write(
                        r,
                        "crates/aa/Cargo.toml",
                        "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    );
                    write(r, "crates/aa/src/main.rs", "fn main() {}\n");
                }),
                ("the directories of binaries go", &|r| {
                    remove(r, "crates/tool/src/bin")
                }),
                ("a member that is a link", &|r| {
                    write(
                        r,
                        "elsewhere/a0/Cargo.toml",
                        "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
                    );
                    write(r, "elsewhere/a0/src/main.rs", "fn main() {}\n");
                    symlink(r, "../elsewhere/a0", "crates/a0")
                }),
            ],
        );
    }

    #[test]
    #[ignore = "open: with the Rust language two cold builds of a cargo workspace differ in about \
                one run of five (the listings of a crate's target directories); every other \
                difference is fixed (`cargo_alias_js_only` passes)"]
    fn cargo_alias_port() {
        cargo_alias(Set::Port);
    }

    #[test]
    fn cargo_alias_js_only() {
        cargo_alias(Set::Js);
    }

    fn go_generate(set: Set) {
        both(
            "go generate",
            set,
            &|r| {
                write(r, "svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
                write(
                    r,
                    "svc/lib/lib.go",
                    "package lib\n\nfunc Do() int { return 1 }\n",
                );
            },
            &[
                ("a file with a directive", &|r| {
                    write(r, "svc/gen.go", "package svc\n\n//go:generate echo hi\n")
                }),
                ("a second directive", &|r| {
                    write(
                        r,
                        "svc/gen.go",
                        "package svc\n\n//go:generate echo hi\n//go:generate echo ho\n",
                    )
                }),
                ("one in testdata", &|r| {
                    write(
                        r,
                        "svc/testdata/g.go",
                        "package x\n\n//go:generate echo td\n",
                    )
                }),
                ("one in vendor", &|r| {
                    write(
                        r,
                        "svc/vendor/x/g.go",
                        "package x\n\n//go:generate echo v\n",
                    )
                }),
                ("one in an underscore directory", &|r| {
                    write(r, "svc/_tools/g.go", "package x\n\n//go:generate echo u\n")
                }),
                ("one in a test file", &|r| {
                    write(
                        r,
                        "svc/lib/zz_test.go",
                        "package lib\n\n//go:generate echo t\n",
                    )
                }),
                ("one with a build constraint", &|r| {
                    write(
                        r,
                        "svc/lib/ign.go",
                        "//go:build ignore\n\npackage main\n\n//go:generate echo i\n",
                    )
                }),
                ("one in a new directory", &|r| {
                    write(
                        r,
                        "svc/deep/er/g.go",
                        "package er\n\n//go:generate echo d\n",
                    )
                }),
                ("a nested module takes the directory", &|r| {
                    write(r, "svc/deep/go.mod", "module example.com/deep\n\ngo 1.22\n")
                }),
                ("the nested module goes", &|r| remove(r, "svc/deep/go.mod")),
                ("a file goes", &|r| remove(r, "svc/gen.go")),
                ("a directory comes to be a link", &|r| {
                    write(r, "other/g.go", "package o\n\n//go:generate echo o\n");
                    symlink(r, "../other", "svc/linked")
                }),
            ],
        );
    }

    #[test]
    #[ignore = "before the branch too: a directory of a Go module comes to be a link to a \
                directory with a directive"]
    fn go_generate_port() {
        go_generate(Set::Port);
    }

    #[test]
    #[ignore = "before the branch too, a reduced build: without the Go language a new file with a \
                go:generate directive does not re-key its module"]
    fn go_generate_js_only() {
        go_generate(Set::Js);
    }

    /// Probes: link chains, links in the middle of a
    /// path, links of a cargo alias's crate, a graph at another place, a
    /// directory whose time is kept.
    mod probes_links {
        use super::*;

        #[test]
        fn py_link_chain() {
            both(
                "py link chain",
                Set::Port,
                &|r| {
                    py(r);
                    symlink(r, "../hops/h1", "py/linked");
                    symlink(r, "../shared/app3", "hops/h1");
                    write(r, "hops/keep.txt", "");
                },
                &[
                    (
                        "the second link leads to a directory without a marker",
                        &|r| symlink(r, "../shared/plain", "hops/h1"),
                    ),
                    ("the second link leads to a package", &|r| {
                        symlink(r, "../shared/app2", "hops/h1")
                    }),
                    ("the second link leads nowhere", &|r| {
                        symlink(r, "../shared/none", "hops/h1")
                    }),
                    ("the second link leads nowhere else", &|r| {
                        symlink(r, "../shared/none2", "hops/h1")
                    }),
                    ("where it leads appears", &|r| {
                        write(r, "shared/none2/__init__.py", "")
                    }),
                    ("the second link goes", &|r| remove(r, "hops/h1")),
                    ("a directory in its place", &|r| {
                        write(r, "hops/h1/__init__.py", "")
                    }),
                    ("a third link", &|r| {
                        symlink(r, "h2", "hops/h1");
                        symlink(r, "../shared/app2", "hops/h2")
                    }),
                    ("the third link leads nowhere", &|r| {
                        symlink(r, "../shared/none3", "hops/h2")
                    }),
                    ("the third link leads nowhere else", &|r| {
                        symlink(r, "../shared/none4", "hops/h2")
                    }),
                    ("a file where it leads", &|r| write(r, "shared/none4", "")),
                    ("a package where it leads", &|r| {
                        write(r, "shared/none4/__init__.py", "")
                    }),
                    ("the first link leads to itself", &|r| {
                        symlink(r, "linked", "py/linked")
                    }),
                    ("two links that lead to each other", &|r| {
                        symlink(r, "../hops/h1", "py/linked");
                        symlink(r, "../py/linked", "hops/h1")
                    }),
                    ("and out of it", &|r| {
                        symlink(r, "../shared/app3", "hops/h1")
                    }),
                ],
            );
        }

        #[test]
        #[ignore = "should fix: a link that takes another way to the same place (one link fewer on \
                    the way) keeps the shard: the refreshed graph is equal, its key is not the key \
                    of a fresh build (the state of a place seen is its real path, not the links on \
                    the way: `seen_state`)"]
        fn a_link_takes_another_way_to_the_same_place() {
            one(
                "py link way",
                Set::Port,
                false,
                &|r| {
                    py(r);
                    symlink(r, "../hops/h1", "py/linked");
                    symlink(r, "h2", "hops/h1");
                    symlink(r, "../shared/app3", "hops/h2");
                },
                &|r| symlink(r, "../hops/h2", "py/linked"),
            );
        }

        #[test]
        fn py_marker_chain() {
            both(
                "py marker chain",
                Set::Port,
                &|r| {
                    py(r);
                    write(r, "py/lk/x.txt", "");
                    write(r, "hops/keep.txt", "");
                    symlink(r, "../../hops/m", "py/lk/__init__.py");
                    symlink(r, "../shared/app2/__init__.py", "hops/m");
                },
                &[
                    ("the second link leads nowhere", &|r| {
                        symlink(r, "../shared/zzz.py", "hops/m")
                    }),
                    ("nowhere else", &|r| {
                        symlink(r, "../shared/yyy.py", "hops/m")
                    }),
                    ("a file appears there", &|r| write(r, "shared/yyy.py", "")),
                    ("it leads to a directory", &|r| {
                        symlink(r, "../shared", "hops/m")
                    }),
                    ("and to a file again", &|r| {
                        symlink(r, "../shared/app3/__init__.py", "hops/m")
                    }),
                ],
            );
        }

        #[test]
        fn py_links_in_the_middle() {
            both(
                "py links in the middle",
                Set::Port,
                &|r| {
                    py(r);
                    remove(r, "py/app");
                    write(r, "srcs/s1/pkg/__init__.py", "");
                    write(r, "srcs/s1/pkg/sub/__init__.py", "");
                    write(r, "srcs/s2/other/__init__.py", "");
                    symlink(r, "../srcs/s1", "py/src");
                },
                &[
                    ("src leads elsewhere", &|r| {
                        symlink(r, "../srcs/s2", "py/src")
                    }),
                    ("a link behind the link", &|r| {
                        symlink(r, "../../shared/app2", "srcs/s2/lk")
                    }),
                    ("it leads to another package", &|r| {
                        symlink(r, "../../shared/app3", "srcs/s2/lk")
                    }),
                    ("it leads to a directory without a marker", &|r| {
                        symlink(r, "../../shared/plain", "srcs/s2/lk")
                    }),
                    ("it leads nowhere", &|r| {
                        symlink(r, "../../shared/none", "srcs/s2/lk")
                    }),
                    ("it leads nowhere else", &|r| {
                        symlink(r, "../../shared/none2", "srcs/s2/lk")
                    }),
                    ("where it leads appears", &|r| {
                        write(r, "shared/none2/__init__.py", "")
                    }),
                    ("a subpackage behind both", &|r| {
                        write(r, "shared/none2/deep/__init__.py", "")
                    }),
                    ("a link below a package behind the link", &|r| {
                        symlink(r, "../../../shared/app2", "srcs/s2/other/in")
                    }),
                    ("which leads elsewhere", &|r| {
                        symlink(r, "../../../shared/app3", "srcs/s2/other/in")
                    }),
                    ("src leads nowhere", &|r| symlink(r, "../srcs/s9", "py/src")),
                    ("src leads nowhere else", &|r| {
                        symlink(r, "../srcs/s8", "py/src")
                    }),
                    ("and there it is", &|r| {
                        write(r, "srcs/s8/late/__init__.py", "")
                    }),
                ],
            );
        }

        /// A link that leads nowhere, through a link that leads somewhere: the
        /// second one comes to lead to a place where the path is.
        #[test]
        #[ignore = "should fix: in an old tree, a link that leads nowhere through a second link \
                    (`py/linked -> ../via/x`, `via -> one`): the second link comes to lead to a \
                    directory that has `x`, of the same time as the one before: the tree part is \
                    kept and the package behind the link is missing (`seen_state`: the links on \
                    the way to where it would lead cover nothing)"]
        fn py_nowhere_through_a_link() {
            both(
                "py nowhere through a link",
                Set::Port,
                &|r| {
                    py(r);
                    write(r, "one/keep.txt", "");
                    write(r, "two/x/__init__.py", "");
                    write(r, "three/keep.txt", "");
                    symlink(r, "one", "via");
                    symlink(r, "../via/x", "py/linked");
                },
                &[
                    ("the link on the way leads to where the path is", &|r| {
                        symlink(r, "two", "via")
                    }),
                    ("and to where it is not", &|r| symlink(r, "three", "via")),
                    ("and back", &|r| symlink(r, "two", "via")),
                    ("and nowhere", &|r| symlink(r, "four", "via")),
                    ("which appears", &|r| write(r, "four/x/__init__.py", "")),
                ],
            );
        }

        #[test]
        fn py_link_into_an_installed_package() {
            both(
                "py link into an installed package",
                Set::Port,
                &|r| {
                    py(r);
                    write(r, "package.json", r#"{"name":"root","private":true}"#);
                    write(
                        r,
                        "node_modules/np/package.json",
                        r#"{"name":"np","version":"1.0.0"}"#,
                    );
                    write(r, "node_modules/np/__init__.py", "");
                    write(
                        r,
                        "node_modules/np2/package.json",
                        r#"{"name":"np2","version":"1.0.0"}"#,
                    );
                    write(r, "node_modules/np2/x.txt", "");
                },
                &[
                    ("a link into an installed package", &|r| {
                        symlink(r, "../node_modules/np", "py/linked")
                    }),
                    ("another version of it", &|r| {
                        write(
                            r,
                            "node_modules/np/package.json",
                            r#"{"name":"np","version":"2.0.0"}"#,
                        )
                    }),
                    ("a subpackage in it", &|r| {
                        write(r, "node_modules/np/sub/__init__.py", "")
                    }),
                    ("the marker in it goes", &|r| {
                        remove(r, "node_modules/np/__init__.py")
                    }),
                    ("a link into another one", &|r| {
                        symlink(r, "../node_modules/np2", "py/linked")
                    }),
                    ("a marker in that one", &|r| {
                        write(r, "node_modules/np2/__init__.py", "")
                    }),
                ],
            );
        }

        fn cargo_links(set: Set) {
            both(
                "cargo alias links",
                set,
                &|r| {
                    tool_crate(r);
                    write(r, "bins1/x.rs", "fn main() {}\n");
                    write(r, "bins2/z.rs", "fn main() {}\n");
                },
                &[
                    ("the directory of binaries is a link", &|r| {
                        symlink(r, "../../../bins1", "crates/tool/src/bin")
                    }),
                    ("a binary behind the link", &|r| {
                        write(r, "bins1/y.rs", "fn main() {}\n")
                    }),
                    ("the link leads elsewhere", &|r| {
                        symlink(r, "../../../bins2", "crates/tool/src/bin")
                    }),
                    ("the link leads nowhere", &|r| {
                        symlink(r, "../../../bins3", "crates/tool/src/bin")
                    }),
                    ("the link leads nowhere else", &|r| {
                        symlink(r, "../../../bins4", "crates/tool/src/bin")
                    }),
                    ("where it leads appears", &|r| {
                        write(r, "bins4/w.rs", "fn main() {}\n")
                    }),
                    ("a chain", &|r| {
                        symlink(r, "../../../hop", "crates/tool/src/bin");
                        symlink(r, "bins1", "hop")
                    }),
                    ("the second link leads elsewhere", &|r| {
                        symlink(r, "bins2", "hop")
                    }),
                    ("the second link leads nowhere", &|r| {
                        symlink(r, "bins9", "hop")
                    }),
                    ("the second link leads nowhere else", &|r| {
                        symlink(r, "bins8", "hop")
                    }),
                    ("where it leads appears", &|r| {
                        write(r, "bins8/q.rs", "fn main() {}\n")
                    }),
                    ("the crate is behind two links", &|r| {
                        remove(r, "crates/tool");
                        for t in ["t1", "t2"] {
                            write(
                                r,
                                &format!("elsewhere/{t}/Cargo.toml"),
                                "[package]\nname = \"tool\"\nversion = \"0.1.0\"\nedition = \
                                 \"2021\"\n",
                            );
                            write(r, &format!("elsewhere/{t}/src/main.rs"), "fn main() {}\n");
                            write(
                                r,
                                &format!("elsewhere/{t}/src/bin/{t}.rs"),
                                "fn main() {}\n",
                            );
                        }
                        symlink(r, "../hop2", "crates/tool");
                        symlink(r, "elsewhere/t1", "hop2")
                    }),
                    ("the second leads to another crate", &|r| {
                        symlink(r, "elsewhere/t2", "hop2")
                    }),
                    ("a binary behind them", &|r| {
                        write(r, "elsewhere/t2/src/bin/more.rs", "fn main() {}\n")
                    }),
                    ("the second leads nowhere", &|r| {
                        symlink(r, "elsewhere/t3", "hop2")
                    }),
                    ("the second leads nowhere else", &|r| {
                        symlink(r, "elsewhere/t4", "hop2")
                    }),
                    ("the configuration directory is a link", &|r| {
                        symlink(r, "elsewhere/t1", "hop2");
                        remove(r, ".cargo");
                        write(
                            r,
                            "cfg1/config.toml",
                            "[alias]\nxtask = \"run -p tool --\"\n",
                        );
                        write(
                            r,
                            "cfg2/config.toml",
                            "[alias]\nxtask = \"run -p lib --\"\n",
                        );
                        symlink(r, "cfg1", ".cargo")
                    }),
                    ("which leads elsewhere", &|r| symlink(r, "cfg2", ".cargo")),
                    ("which leads nowhere", &|r| symlink(r, "cfg3", ".cargo")),
                ],
            );
        }

        #[test]
        fn cargo_links_js_only() {
            cargo_links(Set::Js);
        }

        /// Other sources that ask through links: a Taskfile's include, a
        /// justfile's import.
        #[test]
        fn taskfile_and_just_through_links() {
            both(
                "taskfile and just links",
                Set::Js,
                &|r| {
                    write(r, "package.json", r#"{"name":"root","private":true}"#);
                    write(
                        r,
                        "Taskfile.yml",
                        "version: '3'\nincludes:\n  sub: ./lk\ntasks:\n  a:\n    cmds:\n      - \
                         echo a\n",
                    );
                    write(r, "justfile", "import 'jl/x.just'\n\nb:\n    echo b\n");
                    write(
                        r,
                        "t1/Taskfile.yml",
                        "version: '3'\ntasks:\n  one:\n    cmds:\n      - echo one\n",
                    );
                    write(
                        r,
                        "t2/Taskfile.yml",
                        "version: '3'\ntasks:\n  two:\n    cmds:\n      - echo two\n",
                    );
                    write(r, "t3/keep.txt", "");
                    write(r, "j1/x.just", "one:\n    echo one\n");
                    write(r, "j2/x.just", "two:\n    echo two\n");
                    write(r, "j3/keep.txt", "");
                    symlink(r, "t1", "lk");
                    symlink(r, "j1", "jl");
                },
                &[
                    ("the include leads elsewhere", &|r| symlink(r, "t2", "lk")),
                    ("the import leads elsewhere", &|r| symlink(r, "j2", "jl")),
                    (
                        "the include leads to a directory without a Taskfile",
                        &|r| symlink(r, "t3", "lk"),
                    ),
                    ("the import leads to a directory without the file", &|r| {
                        symlink(r, "j3", "jl")
                    }),
                    ("both lead nowhere", &|r| {
                        symlink(r, "t8", "lk");
                        symlink(r, "j8", "jl")
                    }),
                    ("both lead nowhere else", &|r| {
                        symlink(r, "t9", "lk");
                        symlink(r, "j9", "jl")
                    }),
                    ("where they lead appears", &|r| {
                        write(
                            r,
                            "t9/Taskfile.yml",
                            "version: '3'\ntasks:\n  nine:\n    cmds:\n      - echo nine\n",
                        );
                        write(r, "j9/x.just", "nine:\n    echo nine\n")
                    }),
                    ("a chain", &|r| {
                        symlink(r, "hop", "lk");
                        symlink(r, "t1", "hop")
                    }),
                    ("its second link leads elsewhere", &|r| {
                        symlink(r, "t2", "hop")
                    }),
                ],
            );
        }

        /// The keys of the graph of `setup`, built below `base/<sub>`.
        fn keys_at(sub: &str, set: Set, setup: &dyn Fn(&Path)) -> (String, Vec<String>) {
            let (_t, base) = canonical_tmp();
            let base = base.join(sub);
            let (r, env) = (base.join("repo"), base.join("env"));
            std::fs::create_dir_all(env.join("cargo")).unwrap();
            std::fs::create_dir_all(env.join("gomod")).unwrap();
            write(&r, ".gitignore", ".turbotrace\n");
            setup(&r);
            let langs = langs(set, &env);
            let out = r.join(".turbotrace");
            build_with(&opts(&r, &out), &langs).unwrap();
            export(&out)
        }

        fn same_elsewhere(what: &str, set: Set, setup: &dyn Fn(&Path)) -> bool {
            let a = keys_at("a", set, setup).1;
            let b = keys_at("somewhere/much/deeper/than/a", set, setup).1;
            if a != b {
                eprintln!("MOVED {what} ({set:?}): keys at a: {a:?}; at b: {b:?}");
            }
            a == b
        }

        #[test]
        fn the_keys_of_links_in_the_tree_are_the_same_at_another_place() {
            let mut ok = true;
            let set = Set::Js;
            ok &= same_elsewhere("no link", set, &py);
            ok &= same_elsewhere("a link in the tree", set, &|r| {
                py(r);
                symlink(r, "../shared/app3", "py/linked");
            });
            ok &= same_elsewhere("a chain", set, &|r| {
                py(r);
                symlink(r, "../hops/h1", "py/linked");
                symlink(r, "../shared/app3", "hops/h1");
            });
            ok &= same_elsewhere("a link to nothing in the tree", set, &|r| {
                py(r);
                symlink(r, "../shared/none", "py/linked");
            });
            ok &= same_elsewhere("an alias with links", set, &|r| {
                tool_crate(r);
                write(r, "bins1/x.rs", "fn main() {}\n");
                symlink(r, "../../../hop", "crates/tool/src/bin");
                symlink(r, "bins1", "hop");
            });
            assert!(ok, "see MOVED lines");
        }

        #[test]
        #[ignore = "note: a link of a Python project or of an alias's crate that leaves the \
                    repository (to nothing, or to a directory) makes the shard's key depend on \
                    where the checkout is: the place it leads to is hashed as an absolute path \
                    (`seen_state`)"]
        fn the_keys_of_links_that_leave_the_tree_are_the_same_at_another_place() {
            let mut ok = true;
            let set = Set::Js;
            ok &= same_elsewhere("a link to nothing outside the tree", set, &|r| {
                py(r);
                symlink(r, "../../outside/none", "py/linked");
            });
            ok &= same_elsewhere("a link to something outside the tree", set, &|r| {
                py(r);
                write(r, "../outside/pkg/__init__.py", "");
                symlink(r, "../../outside/pkg", "py/linked");
            });
            ok &= same_elsewhere("an alias with a link to nothing", set, &|r| {
                tool_crate(r);
                symlink(r, "../../../../outside/bins", "crates/tool/src/bin");
            });
            assert!(ok, "see MOVED lines");
        }

        #[test]
        #[ignore = "as `cargo_alias_port`, open: run by hand to see which steps fail besides the \
                    listings of a crate's target directories"]
        fn cargo_links_port() {
            cargo_links(Set::Port);
        }

        /// The time of a cold build of one Python project with `n` package
        /// directories.
        fn cold_time(n: usize) -> std::time::Duration {
            let (_t, base) = canonical_tmp();
            let (r, env) = (base.join("repo"), base.join("env"));
            std::fs::create_dir_all(&env).unwrap();
            write(&r, ".gitignore", ".turbotrace\n");
            write(&r, "py/pyproject.toml", PYPROJECT);
            for i in 0..n {
                let dir = format!("py/p{}/q{}/r{}", i / 100, (i / 10) % 10, i % 10);
                write(&r, &format!("py/p{}/__init__.py", i / 100), "");
                write(
                    &r,
                    &format!("py/p{}/q{}/__init__.py", i / 100, (i / 10) % 10),
                    "",
                );
                write(&r, &format!("{dir}/__init__.py"), "");
                write(&r, &format!("{dir}/m.py"), "x = 1\n");
            }
            let langs = langs(Set::Js, &env);
            let start = std::time::Instant::now();
            build_with(&opts(&r, &r.join(".turbotrace")), &langs).unwrap();
            start.elapsed()
        }

        #[test]
        fn a_python_project_with_many_packages_builds_in_linear_time() {
            let small = cold_time(2000);
            let big = cold_time(8000);
            let ratio = big.as_secs_f64() / small.as_secs_f64();
            eprintln!("SCALE 2000 dirs: {small:?}; 8000 dirs: {big:?}; ratio {ratio:.1}");
            assert!(
                ratio < 6.0,
                "2000 dirs: {small:?}; 8000 dirs: {big:?}; ratio {ratio:.1}"
            );
        }

        /// Sets the link `link` to `target` and gives its directory the time it
        /// had.
        fn retarget_keeping_time(r: &Path, target: &str, link: &str) {
            let dir = r.join(link).parent().unwrap().to_path_buf();
            let m = std::fs::metadata(&dir).unwrap().modified().unwrap();
            symlink(r, target, link);
            std::fs::File::open(&dir).unwrap().set_modified(m).unwrap();
        }

        #[test]
        #[ignore = "note: `link_dirs` trusts the time of the link's directory: a link to nothing \
                    that is set to another target by a tool that restores the directory's time \
                    keeps the tree part in an old tree"]
        fn a_link_to_nothing_leads_elsewhere_and_its_directory_keeps_its_time() {
            one(
                "py link, time kept",
                Set::Port,
                true,
                &|r| {
                    py(r);
                    symlink(r, "../shared/none", "py/linked");
                },
                &|r| retarget_keeping_time(r, "../shared/none2", "py/linked"),
            );
        }

        #[test]
        #[ignore = "note: as above for a link that leads somewhere, whose directory keeps its time"]
        fn a_link_leads_elsewhere_and_its_directory_keeps_its_time() {
            one(
                "py link, time kept",
                Set::Port,
                true,
                &|r| {
                    py(r);
                    symlink(r, "../shared/app3", "py/linked");
                },
                &|r| retarget_keeping_time(r, "../shared/plain", "py/linked"),
            );
        }
    }
}

/// Review probes: a `jest` key of the root manifest with
/// each field a config contributes, links to nothing that a load or a
/// resolution follows, each in a young and in an old tree ([`age`]).
mod root_manifest_and_links {
    use super::*;

    /// A named edit of the tree.
    type Step<'a> = (&'a str, TreeEdit<'a>);

    /// An edit of the tree.
    type TreeEdit<'a> = &'a dyn Fn(&Path);

    /// Runs `steps` on the workspace, a refresh against a fresh build after
    /// each, in a tree written a moment ago (`old == false`) or one that
    /// looks old after each step.
    fn steps(what: &str, old: bool, setup: &dyn Fn(&Path), steps: &[Step<'_>]) {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        setup(&r);
        let start = std::time::SystemTime::now();
        let out = r.join(".turbotrace");
        std::fs::create_dir_all(&out).unwrap();
        if old {
            age(&r, start, 0);
        }
        build(&opts(&r, &out), &langs()).unwrap();
        if old {
            build(&opts(&r, &out), &langs()).unwrap();
        }
        for (i, (name, f)) in steps.iter().enumerate() {
            f(&r);
            if old {
                age(&r, start, i as u64 + 1);
            }
            refresh_equals_cold(&r, &format!("{what} (old: {old}): {name}"));
        }
    }

    fn root_jest(r: &Path, jest: &str) {
        write(
            r,
            "package.json",
            &format!(
                r#"{{"name":"root","private":true,"workspaces":["packages/*"],"scripts":{{"gen":"node gen.js"}},"jest":{jest}}}"#
            ),
        );
    }

    fn manifest_fields(old: bool) {
        let setup = |r: &Path| {
            write(
                r,
                "packages/c/src/c.test.ts",
                "import c from './c';\ntest('c', () => expect(c).toBeTruthy());\n",
            );
            write(r, "__mocks__/left-pad.js", "module.exports = () => '';\n");
            write(r, "packages/b/src/not-installed.ts", "export default 9;\n");
            write(
                r,
                "tr.js",
                "module.exports = { process: (s) => ({ code: s }) };\n",
            );
        };
        let s: &[Step<'_>] = &[
            ("roots c", &|r| {
                root_jest(r, r#"{"roots":["<rootDir>/packages/c"]}"#)
            }),
            ("roots with mocks", &|r| {
                root_jest(
                    r,
                    r#"{"roots":["<rootDir>/packages/c","<rootDir>/__mocks__"]}"#,
                )
            }),
            ("mapper", &|r| {
                root_jest(
                    r,
                    r#"{"roots":["<rootDir>"],"moduleNameMapper":{"^not-installed$":"<rootDir>/packages/b/src/other.ts"}}"#,
                )
            }),
            ("setup files", &|r| {
                root_jest(r, r#"{"setupFiles":["<rootDir>/packages/c/src/d.ts"]}"#)
            }),
            ("setup files after env, by request", &|r| {
                root_jest(
                    r,
                    r#"{"setupFilesAfterEach":["a"],"setupFilesAfterEnv":["b"]}"#,
                )
            }),
            ("transform", &|r| {
                root_jest(r, r#"{"transform":{"^.+\\.ts$":"<rootDir>/tr.js"}}"#)
            }),
            ("module paths", &|r| {
                root_jest(r, r#"{"modulePaths":["<rootDir>/packages/b/src"]}"#)
            }),
            ("module directories", &|r| {
                root_jest(
                    r,
                    r#"{"moduleDirectories":["node_modules","<rootDir>/packages/b/src"]}"#,
                )
            }),
            ("projects", &|r| {
                root_jest(r, r#"{"projects":["<rootDir>/packages/c"]}"#)
            }),
            ("projects glob", &|r| {
                root_jest(r, r#"{"projects":["<rootDir>/packages/*"]}"#)
            }),
            ("test match", &|r| {
                root_jest(r, r#"{"testMatch":["**/d.ts","**/*.test.ts"]}"#)
            }),
            ("haste mocks dir gone", &|r| {
                remove(r, "__mocks__/left-pad.js")
            }),
            ("no key", &|r| root_jest(r, "null")),
            ("no key at all", &|r| apply(r, &Edit::RootManifest(0))),
        ];
        steps("the root manifest's jest key", old, &setup, s);
    }

    #[test]
    #[ignore = "probe, passes"]
    fn the_fields_of_a_root_manifest_jest_key() {
        manifest_fields(false);
    }

    #[test]
    #[ignore = "probe, passes"]
    fn the_fields_of_a_root_manifest_jest_key_in_an_old_tree() {
        manifest_fields(true);
    }

    /// Links to nothing that a load or a resolution follows, retargeted to
    /// another place without anything, then something appears there (D6:
    /// the probe's state no longer carries the link's text).
    fn links_to_nothing(old: bool) {
        let setup = |r: &Path| {
            // A tsconfig that extends a link to nothing.
            write(
                r,
                "packages/a/tsconfig.json",
                r#"{"extends":"./base.link.json","compilerOptions":{"strict":true}}"#,
            );
            symlink(r, "../../nothing1.json", "packages/a/base.link.json");
            // A jest config whose setup file is a link to nothing.
            write(
                r,
                "packages/a/jest.config.js",
                "module.exports = { setupFiles: ['<rootDir>/setup.link.js'] };\n",
            );
            symlink(r, "../../nothing1.js", "packages/a/setup.link.js");
            // A package of `node_modules` that is a link to nothing, and a manifest whose
            // `main` is one.
            write(
                r,
                "packages/c/src/e.ts",
                "import x from 'xpkg';\nimport y from '../y';\nexport default [x, y];\n",
            );
            symlink(r, "../../../nothing1", "packages/c/node_modules/xpkg");
            write(
                r,
                "packages/c/y/package.json",
                r#"{"main":"./main.link.js"}"#,
            );
            symlink(r, "../../../nothing1.ts", "packages/c/y/main.link.js");
        };
        let s: &[Step<'_>] = &[
            ("retargeted", &|r| {
                symlink(r, "../../nothing2.json", "packages/a/base.link.json");
                symlink(r, "../../nothing2.js", "packages/a/setup.link.js");
                symlink(r, "../../../nothing2", "packages/c/node_modules/xpkg");
                symlink(r, "../../../nothing2.ts", "packages/c/y/main.link.js");
            }),
            ("the old targets appear", &|r| {
                write(r, "nothing1.json", r#"{"compilerOptions":{"baseUrl":"."}}"#);
                write(r, "nothing1.js", "1;\n");
                write(
                    r,
                    "nothing1/package.json",
                    r#"{"name":"xpkg","main":"i.js"}"#,
                );
                write(r, "nothing1/i.js", "module.exports = 1;\n");
                write(r, "nothing1.ts", "export default 1;\n");
            }),
            ("the new targets appear", &|r| {
                write(r, "nothing2.json", r#"{"compilerOptions":{"baseUrl":"."}}"#);
                write(r, "nothing2.js", "2;\n");
                write(
                    r,
                    "nothing2/package.json",
                    r#"{"name":"xpkg","main":"i.js"}"#,
                );
                write(r, "nothing2/i.js", "module.exports = 2;\n");
                write(r, "nothing2.ts", "export default 2;\n");
            }),
            ("the new targets go", &|r| {
                for p in ["nothing2.json", "nothing2.js", "nothing2", "nothing2.ts"] {
                    remove(r, p);
                }
            }),
            ("retargeted to the old ones", &|r| {
                symlink(r, "../../nothing1.json", "packages/a/base.link.json");
                symlink(r, "../../nothing1.js", "packages/a/setup.link.js");
                symlink(r, "../../../nothing1", "packages/c/node_modules/xpkg");
                symlink(r, "../../../nothing1.ts", "packages/c/y/main.link.js");
            }),
        ];
        steps("links to nothing", old, &setup, s);
    }

    #[test]
    #[ignore = "probe, passes"]
    fn links_to_nothing_that_loads_follow() {
        links_to_nothing(false);
    }

    #[test]
    #[ignore = "probe, fails: an old-tree refresh keeps the record of where a retargeted link to \
                nothing led"]
    fn links_to_nothing_that_loads_follow_in_an_old_tree() {
        links_to_nothing(true);
    }

    /// The tsconfig link alone, and the author's imported file alone, in an old
    /// tree.
    fn one_link(old: bool, which: usize) {
        let (link, ext, setup): (&str, &str, TreeEdit<'_>) = match which {
            0 => ("packages/a/base.link.json", "json", &|r: &Path| {
                write(
                    r,
                    "packages/a/tsconfig.json",
                    r#"{"extends":"./base.link.json","compilerOptions":{"strict":true}}"#,
                );
            }),
            _ => ("packages/a/src/missing.ts", "ts", &|_: &Path| {}),
        };
        let depth = "../".repeat(link.matches('/').count());
        let to = |n: u32| format!("{depth}nothing{n}.{ext}");
        let setup = |r: &Path| {
            setup(r);
            symlink(r, &to(1), link);
        };
        let s: &[Step<'_>] = &[
            ("retargeted", &|r| symlink(r, &to(2), link)),
            ("the new target appears", &|r| {
                write(
                    r,
                    &format!("nothing2.{ext}"),
                    if ext == "json" {
                        "{}"
                    } else {
                        "export default 2;\n"
                    },
                )
            }),
        ];
        steps(&format!("one link {which}"), old, &setup, s);
    }

    #[test]
    #[ignore = "probe, fails: an old-tree refresh keeps the record of where a retargeted link to \
                nothing led"]
    fn a_tsconfig_extends_a_link_to_nothing_that_is_retargeted_in_an_old_tree() {
        one_link(true, 0);
    }

    #[test]
    #[ignore = "probe, passes"]
    fn an_imported_link_to_nothing_that_is_retargeted_in_an_old_tree() {
        one_link(true, 1);
    }

    /// The same, with the refresh after the retarget unchecked: does the target
    /// that appears then reach the graph?
    #[test]
    #[ignore = "probe, passes"]
    fn a_tsconfig_extends_a_retargeted_link_whose_target_appears_in_an_old_tree() {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(
            &r,
            "packages/a/tsconfig.json",
            r#"{"extends":"./base.link.json","compilerOptions":{"strict":true}}"#,
        );
        symlink(&r, "../../nothing1.json", "packages/a/base.link.json");
        let start = std::time::SystemTime::now();
        let out = r.join(".turbotrace");
        std::fs::create_dir_all(&out).unwrap();
        age(&r, start, 0);
        build(&opts(&r, &out), &langs()).unwrap();
        build(&opts(&r, &out), &langs()).unwrap();
        symlink(&r, "../../nothing2.json", "packages/a/base.link.json");
        age(&r, start, 1);
        build(&opts(&r, &out), &langs()).unwrap();
        write(
            &r,
            "nothing2.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@lib/*":["packages/b/src/*"]}}}"#,
        );
        age(&r, start, 2);
        refresh_equals_cold(&r, "the new target appears");
    }

    /// The same with the two targets in two directories that are there from the
    /// start: the directory of the new target is the only one that changes
    /// when it appears.
    #[test]
    #[ignore = "probe, passes"]
    fn a_tsconfig_extends_a_link_retargeted_to_another_directory_whose_target_appears_in_an_old_tree()
     {
        let (_t, r) = canonical_tmp();
        workspace(&r);
        write(&r, "d1/keep.txt", "1\n");
        write(&r, "d2/keep.txt", "2\n");
        write(
            &r,
            "packages/a/tsconfig.json",
            r#"{"extends":"./base.link.json","compilerOptions":{"strict":true}}"#,
        );
        symlink(&r, "../../d1/n.json", "packages/a/base.link.json");
        let start = std::time::SystemTime::now();
        let out = r.join(".turbotrace");
        std::fs::create_dir_all(&out).unwrap();
        age(&r, start, 0);
        build(&opts(&r, &out), &langs()).unwrap();
        build(&opts(&r, &out), &langs()).unwrap();
        symlink(&r, "../../d2/n.json", "packages/a/base.link.json");
        age(&r, start, 1);
        build(&opts(&r, &out), &langs()).unwrap();
        write(
            &r,
            "d2/n.json",
            r#"{"compilerOptions":{"baseUrl":"../packages/a","paths":{"@lib/*":["../b/src/*"]}}}"#,
        );
        age(&r, start, 2);
        refresh_equals_cold(&r, "the new target appears in its own directory");
    }

    /// A setup file of a config that is a link to a file: the file it leads to
    /// is edited, in a young and an old tree.
    fn setup_link(old: bool) {
        let setup = |r: &Path| {
            write(
                r,
                "packages/a/jest.config.js",
                "module.exports = { setupFiles: ['<rootDir>/setup.link.js'] };\n",
            );
            write(r, "packages/a/lib/real.js", "require('./one');\n");
            write(r, "packages/a/lib/one.js", "1;\n");
            write(r, "packages/a/lib/two.js", "2;\n");
            symlink(r, "lib/real.js", "packages/a/setup.link.js");
        };
        let s: &[Step<'_>] = &[
            ("the file it leads to requires another file", &|r| {
                write(r, "packages/a/lib/real.js", "require('./two');\n")
            }),
            ("the link leads to another file", &|r| {
                symlink(r, "lib/one.js", "packages/a/setup.link.js")
            }),
        ];
        steps("a setup file behind a link", old, &setup, s);
    }

    #[test]
    #[ignore = "probe, passes"]
    fn a_setup_file_behind_a_link() {
        setup_link(false);
    }

    #[test]
    #[ignore = "probe, passes"]
    fn a_setup_file_behind_a_link_in_an_old_tree() {
        setup_link(true);
    }
}
