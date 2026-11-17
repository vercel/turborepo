//! Third-party bare imports without `node_modules` (precision P12): a package's
//! bare import resolves to the opaque package node of the version its own pnpm
//! lockfile importer pins, not to another importer's (the root's) version of
//! the same name, and not to whatever a `node_modules` directory above the
//! repository root holds (Node's walk-up leaves the repository; checkouts under
//! `~/ws` reached `~/ws/node_modules/async-retry` 1.3.3 instead of
//! `published-packages/cosmosdb`'s 1.3.1). So a lockfile bump of that
//! importer's dependency selects its importers.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use turborepo_trace_core::Language;
use turborepo_trace_graph::{GraphView, NodeId};
use turborepo_trace_store::{Graph, GraphOptions, build};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

fn deps(g: &Graph, from: &str) -> Vec<String> {
    let n = g
        .node_by_path(from)
        .unwrap_or_else(|| panic!("node {from}"));
    let mut v: Vec<String> = g
        .out_edges(n)
        .filter_map(|e| e.to)
        .map(|t: NodeId| g.path(t).to_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn bare_imports_resolve_through_their_own_importer() {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    check(&r);
}

#[test]
fn an_install_above_the_repository_does_not_decide() {
    let t = tempfile::tempdir().unwrap();
    let outer = std::fs::canonicalize(t.path()).unwrap();
    write(
        &outer,
        "node_modules/retry-lib/package.json",
        r#"{"name":"retry-lib","version":"9.9.9","main":"index.js"}"#,
    );
    write(
        &outer,
        "node_modules/retry-lib/index.js",
        "module.exports = 1;\n",
    );
    let r = outer.join("repo");
    std::fs::create_dir_all(&r).unwrap();
    check(&r);
}

/// Writes the fixture repository at `r`, builds it and checks the resolutions.
fn check(r: &Path) {
    let r = r.to_path_buf();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        &r,
        "package.json",
        r#"{"name":"root","private":true,"dependencies":{"retry-lib":"^2.0.0"}}"#,
    );
    write(&r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        &r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","dependencies":{"retry-lib":"1.0.0"}}"#,
    );
    write(
        &r,
        "packages/a/src/index.ts",
        "import retry from 'retry-lib';\nexport const r = retry;\n",
    );
    write(
        &r,
        "src/root.ts",
        "import retry from 'retry-lib';\nexport const r = retry;\n",
    );
    write(
        &r,
        "pnpm-lock.yaml",
        r#"lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      retry-lib:
        specifier: ^2.0.0
        version: 2.0.0
  packages/a:
    dependencies:
      retry-lib:
        specifier: 1.0.0
        version: 1.0.0
packages:
  retry-lib@1.0.0:
    resolution: {integrity: sha512-one}
  retry-lib@2.0.0:
    resolution: {integrity: sha512-two}
snapshots:
  retry-lib@1.0.0: {}
  retry-lib@2.0.0: {}
"#,
    );
    let out = r.join(".turbotrace");
    let opts = GraphOptions {
        root: r.clone(),
        out: Some(out.clone()),
        threads: 2,
        ..Default::default()
    };
    let langs: Vec<Arc<dyn Language>> = vec![Arc::new(turborepo_trace_lang_js::JsLanguage)];
    build(&opts, &langs).unwrap();
    let g = Graph::open(&out).unwrap();
    let a = deps(&g, "packages/a/src/index.ts");
    assert!(
        a.contains(&"node_modules/retry-lib@1.0.0".to_string()),
        "packages/a pins 1.0.0: {a:?}"
    );
    assert!(
        !a.contains(&"node_modules/retry-lib@2.0.0".to_string()),
        "not the root's version: {a:?}"
    );
    let root = deps(&g, "src/root.ts");
    assert!(
        root.contains(&"node_modules/retry-lib@2.0.0".to_string()),
        "the root pins 2.0.0: {root:?}"
    );
    let detail: Vec<String> = g
        .out_edges(g.node_by_path("packages/a/src/index.ts").unwrap())
        .map(|e| {
            format!(
                "{:?} {:?} -> {:?}",
                e.specifier,
                e.attr,
                e.to.map(|n| g.path(n))
            )
        })
        .collect();
    assert!(
        !a.iter().chain(&root).any(|p| p.contains("9.9.9")),
        "an install outside the repository is not used: {a:?} {root:?}; edges of a: {detail:?}"
    );
    let a_node = g.node_by_path("packages/a/src/index.ts").unwrap();
    let unc: Vec<String> = g
        .uncertainties(a_node)
        .map(|u| format!("{:?} {}", u.kind, u.detail))
        .collect();
    assert!(
        !unc.iter().any(|u| u.contains("retry-lib")),
        "no unresolved_specifier for a pinned dependency: {unc:?}"
    );
}
