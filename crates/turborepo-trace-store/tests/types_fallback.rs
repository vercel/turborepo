//! Type-only bare imports without `node_modules`: `import type {X} from
//! 'semver'` where the importer only declares `@types/semver` resolves to the
//! `@types/semver` package node through the lockfile (as TypeScript resolves
//! it), with no `unresolved_specifier`. A runtime import of the same undeclared
//! name stays unresolved and uncertain. (Left unresolved, one such type import
//! puts every test that reaches its file on the uncertainty floor.)

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use turborepo_trace_core::Language;
use turborepo_trace_graph::GraphView;
use turborepo_trace_store::{Graph, GraphOptions, build};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

#[test]
fn type_only_imports_fall_back_to_definitely_typed() {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(&r, "package.json", r#"{"name":"root","private":true}"#);
    write(&r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        &r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","devDependencies":{"@types/semver":"7.5.8","@types/scope__lib":"1.0.0"}}"#,
    );
    write(
        &r,
        "packages/a/src/types.ts",
        "import type { ReleaseType } from 'semver';\nimport type { L } from '@scope/lib';\nexport \
         type O = ReleaseType | L;\n",
    );
    write(
        &r,
        "packages/a/src/runtime.ts",
        "import semver from 'semver';\nexport const v = semver.valid;\n",
    );
    write(
        &r,
        "pnpm-lock.yaml",
        r#"lockfileVersion: '9.0'
importers:
  .: {}
  packages/a:
    devDependencies:
      '@types/semver':
        specifier: 7.5.8
        version: 7.5.8
      '@types/scope__lib':
        specifier: 1.0.0
        version: 1.0.0
packages:
  '@types/semver@7.5.8':
    resolution: {integrity: sha512-types}
  '@types/scope__lib@1.0.0':
    resolution: {integrity: sha512-scoped}
snapshots:
  '@types/semver@7.5.8': {}
  '@types/scope__lib@1.0.0': {}
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

    let types = g.node_by_path("packages/a/src/types.ts").unwrap();
    let targets: Vec<&str> = g
        .out_edges(types)
        .filter_map(|e| e.to)
        .map(|n| g.path(n))
        .collect();
    assert!(
        targets.contains(&"node_modules/@types/semver@7.5.8"),
        "{targets:?}"
    );
    assert!(
        targets.contains(&"node_modules/@types/scope__lib@1.0.0"),
        "{targets:?}"
    );
    let unc: Vec<String> = g
        .uncertainties(types)
        .map(|u| u.detail.to_owned())
        .collect();
    assert!(unc.is_empty(), "{unc:?}");

    // Runtime imports don't fall back: `semver` itself is not installed.
    let runtime = g.node_by_path("packages/a/src/runtime.ts").unwrap();
    assert!(
        !g.out_edges(runtime)
            .filter_map(|e| e.to)
            .any(|n| g.path(n).contains("@types")),
    );
    assert!(
        g.uncertainties(runtime)
            .any(|u| u.detail.contains("'semver'")),
        "a runtime import of an undeclared package stays uncertain"
    );
}
