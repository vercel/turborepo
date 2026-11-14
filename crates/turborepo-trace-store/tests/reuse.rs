//! Shard reuse on a synthetic monorepo (`support::monorepo`): a refresh
//! re-traces only the packages whose inputs changed. Regression test for keys
//! invalidated by something global (the framework config listing, task
//! discovery, options).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

mod support;

use support::monorepo::{Monorepo, MonorepoStats, backdate};
use turborepo_trace_core::Language;
use turborepo_trace_store::{BuildReport, GraphOptions, build};

fn langs() -> Vec<Arc<dyn Language>> {
    vec![Arc::new(turborepo_trace_lang_js::JsLanguage)]
}

/// The store lives next to the repository, so writing it does not touch the
/// walked tree.
fn rebuild(root: &Path) -> BuildReport {
    let opts = GraphOptions {
        root: root.to_path_buf(),
        out: Some(root.with_file_name("store")),
        threads: 2,
        ..Default::default()
    };
    build(&opts, &langs()).unwrap()
}

fn traced_roots(r: &BuildReport, before: &BuildReport) -> Vec<String> {
    let old: std::collections::HashSet<&str> = before
        .manifest
        .shards
        .iter()
        .map(|s| s.key.as_str())
        .collect();
    let mut v: Vec<String> = r
        .manifest
        .shards
        .iter()
        .filter(|s| s.kind == "first-party" && !old.contains(s.key.as_str()))
        .map(|s| s.root.clone())
        .collect();
    v.sort();
    v
}

fn shape() -> Monorepo {
    Monorepo {
        packages: 24,
        files_per_package: 6,
        third_party: 12,
        // Enough build-output directories that root-level deep listings enter more than a
        // shard's tree part keeps (256).
        dist_dirs: 12,
        ..Default::default()
    }
}

fn setup() -> (tempfile::TempDir, std::path::PathBuf, MonorepoStats) {
    let t = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(t.path()).unwrap().join("repo");
    std::fs::create_dir(&root).unwrap();
    let st = shape().generate_in(&root).unwrap();
    backdate(&root, 60).unwrap();
    (t, root, st)
}

fn edit(root: &Path, rel: &str, append: &str) {
    let p = root.join(rel);
    let mut s = std::fs::read_to_string(&p).unwrap_or_default();
    s.push_str(append);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn pkg_of(rel: &str) -> String {
    rel.split('/').take(2).collect::<Vec<_>>().join("/")
}

#[test]
fn untouched_tree_reuses_every_shard() {
    let (_t, root, _) = setup();
    let a = rebuild(&root);
    assert!(a.traced > 24);
    let b = rebuild(&root);
    assert_eq!(b.traced, 0, "{:?}", traced_roots(&b, &a));
    assert_eq!(b.reused, a.traced);
}

#[test]
fn one_edited_file_retraces_one_shard() {
    let (_t, root, st) = setup();
    let a = rebuild(&root);
    edit(&root, &st.sample_source, "\nexport const edited = 1;\n");
    let b = rebuild(&root);
    assert_eq!(traced_roots(&b, &a), [pkg_of(&st.sample_source)]);
    assert_eq!(b.traced, 1);
}

#[test]
fn added_file_retraces_its_package_only() {
    let (_t, root, st) = setup();
    let a = rebuild(&root);
    let pkg = pkg_of(&st.sample_source);
    edit(
        &root,
        &format!("{pkg}/src/added.ts"),
        "export const added = 1;\n",
    );
    let b = rebuild(&root);
    assert_eq!(traced_roots(&b, &a), [pkg]);
}

/// A jest config added in one package re-keys that package and the root package
/// (whose tree contains it), not every package of the repository.
#[test]
fn added_jest_config_retraces_governed_packages_only() {
    let (_t, root, st) = setup();
    let a = rebuild(&root);
    let pkg = pkg_of(&st.sample_source);
    edit(
        &root,
        &format!("{pkg}/src/__tests__/jest.config.js"),
        "module.exports = {};\n",
    );
    let b = rebuild(&root);
    assert_eq!(traced_roots(&b, &a), ["".to_owned(), pkg]);
}

/// Several files edited across packages: exactly those packages.
#[test]
fn spread_edits_retrace_their_packages() {
    let (_t, root, st) = setup();
    let a = rebuild(&root);
    let mut want: Vec<String> = Vec::new();
    for f in &st.spread_sources {
        edit(&root, f, "\nexport const spread = 1;\n");
        want.push(pkg_of(f));
    }
    want.sort();
    want.dedup();
    assert!(want.len() > 2);
    let b = rebuild(&root);
    assert_eq!(traced_roots(&b, &a), want);
}

/// A no-op refresh reuses every shard's observed tree state, including shards
/// whose deep listings (a root `**/__mocks__/**`, task discovery) enter many
/// gitignored directories: nothing is listed or probed again.
#[test]
fn noop_refresh_reuses_tree_parts() {
    let (_t, root, _) = setup();
    let a = rebuild(&root);
    let b = rebuild(&root);
    assert!(b.walk_cached);
    assert_eq!(b.traced, 0);
    assert_eq!(b.tree_reused, [a.first_party, 0], "{:?}", b.tree_reused);
    // And again: tree parts and listings reused without being recomputed are kept.
    let c = rebuild(&root);
    assert_eq!(c.tree_reused, [a.first_party, 0]);
}

/// A version bump in a low-level package's manifest re-keys the package and the
/// packages that resolve imports through it (they read its `package.json`), not
/// every package that depends on it transitively through turbo's `^build`.
#[test]
fn manifest_bump_does_not_retrace_transitive_dependents() {
    let (_t, root, _) = setup();
    let deps = shape().workspace_deps();
    let a = rebuild(&root);
    let target = "packages/p0000";
    let pj = root.join(target).join("package.json");
    let s = std::fs::read_to_string(&pj).unwrap();
    std::fs::write(
        &pj,
        s.replace("\"version\": \"1.0.0\"", "\"version\": \"1.0.1\""),
    )
    .unwrap();
    let b = rebuild(&root);
    let traced = traced_roots(&b, &a);
    assert!(traced.iter().any(|r| r == target), "{traced:?}");
    let direct: std::collections::HashSet<&str> = deps
        .iter()
        .filter(|(_, d)| d.iter().any(|x| x == target))
        .map(|(p, _)| p.as_str())
        .collect();
    for r in &traced {
        assert!(
            r == target || r.is_empty() || direct.contains(r.as_str()),
            "{r} depends on {target} only transitively but was re-traced: {traced:?}"
        );
    }
    // The fixture has transitive dependents (so the check above means something).
    assert!(deps.iter().filter(|(_, d)| !d.is_empty()).count() > direct.len() + 1);
}

/// After a tree change, only the tree parts that depend on a changed directory
/// are recomputed: an added source file in one package leaves the observed tree
/// state of most other packages reusable (per-directory reuse, not
/// all-or-nothing on the walk).
#[test]
fn added_file_rechecks_few_tree_parts() {
    let (_t, root, st) = setup();
    let a = rebuild(&root);
    let _ = rebuild(&root);
    let pkg = pkg_of(&st.sample_source);
    edit(
        &root,
        &format!("{pkg}/src/added.ts"),
        "export const added = 1;\n",
    );
    let b = rebuild(&root);
    eprintln!(
        "tree parts after an add: {:?} of {}",
        b.tree_reused, a.first_party
    );
    assert_eq!(traced_roots(&b, &a), [pkg]);
    assert!(
        b.tree_reused[1] * 2 < a.first_party,
        "{:?} of {}",
        b.tree_reused,
        a.first_party
    );
}

/// A file replaced through a rename (as `git checkout` writes files) changes
/// its directory's mtime but not its entries: with unchanged contents, every
/// tree part is reused, even for the repository root, which nearly every
/// package's observations depend on.
#[test]
fn rename_rewrite_keeps_tree_parts() {
    let (_t, root, _) = setup();
    let a = rebuild(&root);
    let _ = rebuild(&root);
    let p = root.join("jest.config.js");
    let tmp = root.join("jest.config.js.tmp");
    std::fs::copy(&p, &tmp).unwrap();
    std::fs::rename(&tmp, &p).unwrap();
    let b = rebuild(&root);
    assert_eq!(b.traced, 0, "{:?}", traced_roots(&b, &a));
    assert_eq!(b.tree_reused, [a.first_party, 0]);
}

/// The options digest covers the build identity (a hash of the sources that
/// decide shard contents): two identities never share a digest.
#[test]
fn build_identity_enters_the_options_digest() {
    let tracer = turborepo_trace_core::Tracer::builder()
        .language(turborepo_trace_lang_js::JsLanguage)
        .build()
        .unwrap();
    let opts = GraphOptions::default();
    let d = |id: &str| turborepo_trace_store::build::options_digest_for(&opts, &tracer, id);
    assert_ne!(d("a"), d("b"));
    assert_eq!(
        turborepo_trace_store::build::options_digest(&opts, &tracer),
        d(turborepo_trace_store::build::build_id())
    );
    assert!(!turborepo_trace_store::build::build_id().is_empty());
}

/// A store written by a binary with another build identity (another options
/// digest) is not reused: every first-party shard is traced again, task records
/// included.
#[test]
fn changed_build_identity_invalidates_every_shard() {
    let (_t, root, _) = setup();
    let a = rebuild(&root);
    let manifest = root.with_file_name("store").join("graph.json");
    let mut m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    m["options_digest"] = serde_json::Value::String("another-build".into());
    std::fs::write(&manifest, serde_json::to_vec(&m).unwrap()).unwrap();
    let b = rebuild(&root);
    assert_eq!(b.traced, a.first_party, "{b:?}");
    assert_eq!(
        b.reused, b.third_party,
        "only opaque shards (keyed by name and version)"
    );
}
