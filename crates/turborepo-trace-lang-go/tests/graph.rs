//! Graph mode on the fixture through the library API:
//! per-reference observations (probes, directory memberships, reads) and opaque
//! module identities.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_core::{AnalyzeInput, Observations, RefKind, Tracer, fs::normalize};
use turborepo_trace_lang_go::{GoLanguage, GoOptions, GoRoot, identity};

fn basic() -> PathBuf {
    normalize(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/go/basic"),
    )
}

fn tracer(root: &Path) -> Tracer {
    Tracer::builder()
        .base(root)
        .cwd(root)
        .target("linux/amd64")
        .record(true)
        .language(GoLanguage::new(GoOptions {
            modcache: Some(root.join("modcache")),
            goroot: GoRoot::None,
            cgo: Some(true),
            workspace: Some(true),
            ..GoOptions::default()
        }))
        .build()
        .unwrap()
}

/// Observations of every reference whose edges include one of `kind`, merged.
fn observed(fa: &turborepo_trace_core::FileAnalysis, spec: &str) -> Observations {
    let mut o = Observations::default();
    for g in &fa.groups {
        if fa.edges[g.edges.clone()]
            .iter()
            .any(|e| &*e.edge.spec == spec)
        {
            o.extend(g.observations.clone());
        }
    }
    o
}

#[test]
fn records_probes_memberships_and_identities() {
    let root = basic();
    let t = tracer(&root);
    let out = t.analyze(vec![AnalyzeInput {
        path: root.join("main.go"),
        cached: None,
    }]);
    let fa = &out[0];

    // The import of a first-party package: its directory is a membership, and the
    // module lookup probed for go.mod files below the module root.
    let util = observed(fa, "example.com/basic/internal/util");
    assert!(
        util.listings
            .contains(&(root.join("internal/util"), "*".to_string())),
        "{util:?}"
    );
    assert!(util.reads.contains(&root.join("go.mod")), "{util:?}");
    // Files excluded by a build constraint are read inputs (their constraint may
    // change).
    assert!(util.reads.contains(&root.join("internal/util/tagged.go")));
    assert!(util.reads.contains(&root.join("internal/util/ignored.go")));

    // A module cache package: probes for the vendor directory (not vendored) and
    // the package's directory membership.
    let dep = observed(fa, "github.com/Upper/dep/sub");
    assert!(
        dep.missing.contains(&root.join("vendor/modules.txt")),
        "{dep:?}"
    );
    let sub_dir = root.join("modcache/github.com/!upper/dep@v1.2.0/sub");
    assert!(dep.listings.contains(&(sub_dir.clone(), "*".to_string())));

    // Embed patterns are memberships relative to the package directory, matched
    // directories are `**` memberships.
    let tmpl = observed(fa, "tmpl");
    assert!(tmpl.listings.contains(&(root.clone(), "tmpl".to_string())));
    assert!(
        tmpl.listings
            .contains(&(root.join("tmpl"), "**".to_string()))
    );
    let missing = observed(fa, "missing/*.txt");
    assert!(
        missing
            .listings
            .contains(&(root.clone(), "missing/*.txt".to_string()))
    );

    // The same observations are reported again by a second file using the memoized
    // steps.
    let out2 = t.analyze(vec![AnalyzeInput {
        path: root.join("version.go"),
        cached: None,
    }]);
    let pkg = &out2[0].groups.last().unwrap().observations;
    assert!(
        pkg.listings.contains(&(root.clone(), "*".to_string())),
        "{pkg:?}"
    );

    // Opaque identity of the module cache target, with its go.sum hash.
    let sums = identity::load_sums(t.fs(), &root);
    let sub = fa
        .edges
        .iter()
        .find(|e| &*e.edge.spec == "github.com/Upper/dep/sub" && e.edge.kind == RefKind::Import)
        .unwrap();
    let file = t.path_of(sub.succ[0]);
    // The fixture's module cache is not below `pkg/mod`, so name it.
    assert!(identity::module_identity(t.fs(), &file, Some(&sums)).is_none());
    let mc = root.join("modcache");
    let id = identity::module_identity_in(t.fs(), &file, Some(&mc), Some(&sums)).unwrap();
    assert_eq!(id.key(), "github.com/Upper/dep@v1.2.0");
    assert_eq!(id.root, mc.join("github.com/!upper/dep@v1.2.0"));
    assert_eq!(
        id.sum.as_deref(),
        Some("h1:UPPERDEPZIPHASH0000000000000000000000000000=")
    );
    // First-party files have no module identity.
    assert!(identity::module_identity(t.fs(), &root.join("main.go"), Some(&sums)).is_none());
}

#[test]
fn vendored_identity_from_trace() {
    let root = normalize(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../turborepo-trace-testkit/fixtures/go/vendored"),
    );
    let t = tracer(&root);
    let trace = t.trace(&[root.join("main.go")]).unwrap();
    let pkg = root.join("vendor/github.com/v/dep/pkg/pkg.go");
    assert!(trace.files().any(|f| f == pkg));
    let sums = identity::load_sums(t.fs(), &root);
    let id = identity::module_identity(t.fs(), &pkg, Some(&sums)).unwrap();
    assert_eq!(id.key(), "github.com/v/dep@v1.0.0");
    assert_eq!(
        id.sum.as_deref(),
        Some("h1:VENDORDEPHASH000000000000000000000000000000=")
    );
    assert!(trace.files().any(|f| f == root.join("vendor/modules.txt")));
}
