//! Graph mode: per-file analysis with recorded probes,
//! memberships and config reads, as the store and `affected` consume them.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use turborepo_trace_core::{AnalyzeInput, Observations, RefKind, Tracer};
use turborepo_trace_lang_rust::RustLanguage;

fn basic() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit/fixtures/rust/basic")
        .canonicalize()
        .unwrap()
}

/// Per analyzed file: its relative path, all observations, and `(kind, target)`
/// edges.
type FileObs = (String, Observations, Vec<(RefKind, String)>);

fn observations(rels: &[&str]) -> Vec<FileObs> {
    let root = basic();
    let tracer = Tracer::builder()
        .base(&root)
        .cwd(&root)
        .target("x86_64-unknown-linux-gnu")
        .record(true)
        .threads(1)
        .language(RustLanguage::new())
        .build()
        .unwrap();
    let inputs = rels
        .iter()
        .map(|r| AnalyzeInput {
            path: root.join(r),
            cached: None,
        })
        .collect();
    tracer
        .analyze(inputs)
        .into_iter()
        .map(|a| {
            let mut obs = Observations::default();
            for g in &a.groups {
                obs.extend(g.observations.clone());
            }
            let edges = a
                .edges
                .iter()
                .flat_map(|e| {
                    e.succ.iter().map(|s| {
                        let p = tracer.path_of(*s);
                        (
                            e.edge.kind,
                            p.strip_prefix(&root)
                                .map(|r| r.display().to_string())
                                .unwrap_or_default(),
                        )
                    })
                })
                .collect();
            (
                a.path.strip_prefix(&root).unwrap().display().to_string(),
                obs,
                edges,
            )
        })
        .collect()
}

#[test]
fn probes_and_config_reads_are_recorded_per_file() {
    let root = basic();
    // Two files of the same package: the second hits the memoized package lookup,
    // which must replay the observations.
    let res = observations(&["app/src/main.rs", "app/src/cli.rs", "app/src/net/mod.rs"]);
    let (_, main, edges) = &res[0];
    // `mod net;`: `net.rs` was probed before `net/mod.rs` was found.
    assert!(
        main.missing.contains(&root.join("app/src/net.rs")),
        "{:?}",
        main.missing
    );
    assert!(edges.contains(&(RefKind::Mod, "app/src/net/mod.rs".into())));
    // Crate-root edges: build script, dependency crates, manifests.
    assert!(edges.contains(&(RefKind::Package, "app/build.rs".into())));
    assert!(edges.contains(&(RefKind::Package, "util/src/lib.rs".into())));
    assert!(edges.contains(&(RefKind::Package, "vendor/regdep/src/lib.rs".into())));
    assert!(edges.contains(&(RefKind::Manifest, "Cargo.lock".into())));
    for (file, obs, edges) in &res {
        assert!(
            obs.reads.contains(&root.join("app/Cargo.toml")),
            "{file}: {:?}",
            obs.reads
        );
        assert!(
            edges.contains(&(RefKind::Manifest, "app/Cargo.toml".into())),
            "{file}"
        );
    }
    // `cli.rs` is not a mod-rs file: `mod args;` probes below `cli/`.
    let (_, cli, edges) = &res[1];
    assert!(edges.contains(&(RefKind::Mod, "app/src/cli/args.rs".into())));
    assert!(!cli.missing.contains(&root.join("app/src/args.rs")));
}

#[test]
fn analysis_is_deterministic_across_runs() {
    let a = observations(&["app/src/main.rs", "util/src/lib.rs"]);
    let b = observations(&["app/src/main.rs", "util/src/lib.rs"]);
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x.0, y.0);
        assert_eq!(x.2, y.2);
        assert_eq!(x.1.missing, y.1.missing);
    }
}
