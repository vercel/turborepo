//! Real-world validation, gated on environment variables (inputs are generated
//! in the sandbox, not stored in the repo):
//!
//! - `TURBO_TRACE_DEBUG_LOCKFILE_RW`: output dir of the generator of real
//!   package-manager lockfiles in the trace's development repository
//!   (`<pm>/{v1,v2,v3}.<lockfile>`). v1→v2 bumps `express` 4.21.1 → 4.21.2
//!   directly in `packages/a`; v2→v3 overrides the transitive `agent-base`
//!   7.1.1 → 7.1.3.
//! - `TURBO_TRACE_DEBUG_LOCKFILE_API`: a large lockfile (for example a large
//!   monorepo's `pnpm-lock.yaml`) that must parse with no dangling edges; with
//!   `TURBO_TRACE_DEBUG_LOCKFILE_API_OLD`, it is also diffed.
//!
//! Without the variables the tests pass trivially and print why.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use turborepo_trace_lockfile::{Lockfile, Target, diff, parse_path};

fn env_dir(var: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(v) => Some(PathBuf::from(v)),
        None => {
            eprintln!("skipping: {var} not set");
            None
        }
    }
}

fn lockfile_in(dir: &Path, version: &str) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.starts_with(&format!("{version}.")) && !name.ends_with(".log")
        })
        .unwrap_or_else(|| panic!("no {version} lockfile in {}", dir.display()))
}

/// Parses `<dir>/<v>.<lockfile>`, giving it its real file name for format
/// detection.
fn load(dir: &Path, version: &str) -> Lockfile {
    let path = lockfile_in(dir, version);
    let name = path.file_name().unwrap().to_string_lossy();
    let real = name.split_once('.').unwrap().1;
    let src = std::fs::read_to_string(&path).unwrap();
    let mut lock = turborepo_trace_lockfile::parse_named(real, &src)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if lock.format() == turborepo_trace_lockfile::LockfileFormat::YarnClassic {
        // yarn.lock v1 has no importers: rebuild them from the manifests the script
        // writes.
        let (express, _) = versions(version);
        let a = format!(
            r#"{{"name": "@rw/a", "dependencies": {{"express": "{express}", "@rw/b": "*"}}}}"#
        );
        let b = r#"{"name": "@rw/b", "dependencies": {"https-proxy-agent": "7.0.5", "socks-proxy-agent": "8.0.4"}}"#;
        let root = r#"{"name": "rw-root", "dependencies": {"next": "14.2.15", "react": "18.3.1", "react-dom": "18.3.1"}}"#;
        lock.add_manifest_importers(&[(".", root), ("packages/a", &a), ("packages/b", b)])
            .unwrap();
    }
    lock
}

fn versions(v: &str) -> (&'static str, &'static str) {
    match v {
        "v1" => ("4.21.1", "7.1.1"),
        "v2" => ("4.21.2", "7.1.1"),
        _ => ("4.21.2", "7.1.3"),
    }
}

fn names(lock: &Lockfile, ids: &BTreeSet<String>) -> BTreeSet<String> {
    ids.iter()
        .map(|id| lock.package(id).unwrap().name.clone())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn assert_resolves_everything(lock: &Lockfile, what: &str) {
    for p in lock.packages() {
        for e in &p.dependencies {
            if let Target::Package(id) = &e.target {
                assert!(
                    lock.package(id).is_some(),
                    "{what}: {} -> {id} dangling",
                    p.id
                );
            }
        }
    }
}

#[test]
fn package_manager_bumps() {
    let Some(root) = env_dir("TURBO_TRACE_DEBUG_LOCKFILE_RW") else {
        return;
    };
    let mut checked = Vec::new();
    for pm in ["npm3", "npm2", "pnpm10", "pnpm8", "yarn1", "berry", "bun"] {
        let dir = root.join(pm);
        if !dir.exists() {
            eprintln!("{pm}: missing, skipped");
            continue;
        }
        let (v1, v2, v3) = (load(&dir, "v1"), load(&dir, "v2"), load(&dir, "v3"));
        for (l, v) in [(&v1, "v1"), (&v2, "v2"), (&v3, "v3")] {
            assert_resolves_everything(l, &format!("{pm} {v}"));
            assert_eq!(l.importers().len(), 3, "{pm} {v} importers");
            let express = l
                .resolve("packages/a", "express")
                .unwrap_or_else(|| panic!("{pm} {v}: express"));
            let turborepo_trace_lockfile::Resolved::Package(express) = express else {
                panic!("{pm} {v}: express unresolved")
            };
            assert_eq!(express.version, versions(v).0, "{pm} {v}");
        }

        // v1 -> v2: direct bump of express; path-to-regexp moves with it. Nothing
        // depends on express, so the closure set equals the identity set.
        let d = diff(&v1, &v2);
        let a = d.affected_packages();
        assert_eq!(
            names(&v2, &a.identity_changed),
            set(&["express", "path-to-regexp"]),
            "{pm} v1->v2 identity"
        );
        assert_eq!(
            names(&v2, &a.closure_changed),
            set(&["express", "path-to-regexp"]),
            "{pm} v1->v2 closure"
        );
        let imp: Vec<(&str, Vec<&str>)> = d
            .importers
            .iter()
            .map(|i| {
                (
                    i.path.as_str(),
                    i.changed.iter().map(|c| c.name.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            imp,
            [("packages/a", vec!["express"])],
            "{pm} v1->v2 importers"
        );

        // v2 -> v3: transitive override of agent-base; its two dependents are
        // closure-changed.
        let d = diff(&v2, &v3);
        let a = d.affected_packages();
        assert_eq!(
            names(&v3, &a.identity_changed),
            set(&["agent-base"]),
            "{pm} v2->v3 identity"
        );
        assert_eq!(
            names(&v3, &a.closure_changed),
            set(&["agent-base", "https-proxy-agent", "socks-proxy-agent"]),
            "{pm} v2->v3 closure"
        );
        let b = d
            .importers
            .iter()
            .find(|i| i.path == "packages/b")
            .unwrap_or_else(|| panic!("{pm}: packages/b"));
        assert!(
            b.changed.is_empty(),
            "{pm}: direct deps of packages/b did not change"
        );
        assert_eq!(
            b.affected,
            ["https-proxy-agent", "socks-proxy-agent"],
            "{pm}"
        );
        assert_eq!(d.importers.len(), 1, "{pm}: only packages/b is affected");

        // Re-diffing a lockfile against itself is empty.
        assert!(diff(&v3, &v3).is_empty());
        checked.push(pm);
    }
    eprintln!("checked: {checked:?}");
}

#[test]
fn big_lockfile_parses_and_diffs() {
    let Some(path) = env_dir("TURBO_TRACE_DEBUG_LOCKFILE_API") else {
        return;
    };
    let t = std::time::Instant::now();
    let lock = parse_path(&path).unwrap();
    eprintln!(
        "parsed {} packages, {} importers in {:?}",
        lock.packages().len(),
        lock.importers().len(),
        t.elapsed()
    );
    assert_resolves_everything(&lock, "api");
    for imp in lock.importers() {
        for e in &imp.dependencies {
            if let Target::Importer(p) = &e.target {
                // Links may point outside the workspace (`link:../outside`); inside ones must
                // exist.
                if !p.starts_with("..") {
                    assert!(
                        lock.importer(p).is_some(),
                        "{} -> {p} missing importer",
                        imp.path
                    );
                }
            }
        }
    }
    if let Some(old) = std::env::var_os("TURBO_TRACE_DEBUG_LOCKFILE_API_OLD") {
        let old = parse_path(old).unwrap();
        let t = std::time::Instant::now();
        let d = diff(&old, &lock);
        let a = d.affected_packages();
        eprintln!(
            "diff in {:?}: +{} -{} ~{} identity {} closure {}",
            t.elapsed(),
            d.added.len(),
            d.removed.len(),
            d.changed.len(),
            a.identity_changed.len(),
            a.closure_changed.len()
        );
        assert!(a.identity_changed.is_subset(&a.closure_changed));
        // Stability: diff of the reverse direction mirrors identity changes.
        let r = diff(&lock, &old);
        assert_eq!(r.affected_packages().removed_identities, a.identity_changed);
    }
}
