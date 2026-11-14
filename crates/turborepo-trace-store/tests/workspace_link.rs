//! Workspace dependencies without installed `node_modules` (lockfile-only
//! resolution): a pnpm `link:` importer entry resolves like a
//! `node_modules/<name>` link to the package directory, through the target's
//! `exports` / `main`, so a source edit in a workspace package selects the
//! tests of the packages that import it (not only manifest edits).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use turborepo_trace_core::Language;
use turborepo_trace_graph::{
    AffectedOptions, ChangeKind, ChangeSet, Engine, GraphView, NodeSet, Verdict,
};
use turborepo_trace_store::{Graph, GraphOptions, build};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

/// `b` imports `a` (no `exports`, `main` to a source file) and `c/util`
/// (`exports` map); nothing is installed.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(&r, "package.json", r#"{"name":"root","private":true}"#);
    write(&r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        &r,
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","main":"./src/index.ts"}"#,
    );
    write(&r, "packages/a/src/index.ts", "export * from './lib';\n");
    write(&r, "packages/a/src/lib.ts", "export const x = 1;\n");
    write(
        &r,
        "packages/c/package.json",
        r#"{"name":"c","version":"1.0.0","exports":{".":"./src/main.ts","./util":"./src/util.ts"}}"#,
    );
    write(&r, "packages/c/src/main.ts", "export const m = 1;\n");
    write(&r, "packages/c/src/util.ts", "export const u = 1;\n");
    write(
        &r,
        "packages/b/package.json",
        r#"{"name":"b","version":"1.0.0","dependencies":{"a":"workspace:*","c":"workspace:*"},"devDependencies":{"jest":"29.0.0"}}"#,
    );
    write(&r, "packages/b/jest.config.js", "module.exports = {};\n");
    write(
        &r,
        "packages/b/src/b.test.ts",
        "import { x } from 'a';\nimport { u } from 'c/util';\ntest('b', () => expect(x + \
         u).toBe(2));\n",
    );
    write(
        &r,
        "pnpm-lock.yaml",
        "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  packages/a: {}\n  packages/c: {}\n  \
         packages/b:\n    dependencies:\n      a:\n        specifier: workspace:*\n        \
         version: link:../a\n      c:\n        specifier: workspace:*\n        version: \
         link:../c\n",
    );
    (t, r)
}

fn affected(g: &Graph, changed: &str, entry: &str) -> Verdict {
    let e = g.node_by_path(entry).expect("entry");
    let mut entries = NodeSet::new(g.node_count());
    entries.insert(e);
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, changed.to_owned());
    let opts = AffectedOptions::default();
    let rep = Engine::new(g).affected(&cs, &entries, &opts);
    rep.entries
        .iter()
        .find(|r| r.entry == e)
        .map_or(Verdict::Unaffected, |r| r.verdict)
}

#[test]
fn source_edit_in_linked_workspace_package_selects_importing_tests() {
    let (_t, r) = fixture();
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
    let test = "packages/b/src/b.test.ts";
    // Through `main` (no `exports`), transitively.
    assert_eq!(
        affected(&g, "packages/a/src/lib.ts", test),
        Verdict::Affected
    );
    // Through an `exports` subpath.
    assert_eq!(
        affected(&g, "packages/c/src/util.ts", test),
        Verdict::Affected
    );
    // The target manifests stay inputs (`exports` / `main` decide the target).
    assert_eq!(
        affected(&g, "packages/c/package.json", test),
        Verdict::Affected
    );
    // A file the import does not reach.
    assert_eq!(
        affected(&g, "packages/c/src/main.ts", test),
        Verdict::Unaffected
    );
}
