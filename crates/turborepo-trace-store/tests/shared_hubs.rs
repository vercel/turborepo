//! Membership hubs shared by several packages: two packages whose tasks have
//! the same membership (`knip --directory ../..` lists the whole repository)
//! each store a hub pseudo-file with the same node name. The composed graph
//! keeps one hub node, and every task's edge to its own copy must resolve to
//! that node (not only the first package's).

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

#[test]
fn every_task_reaches_a_shared_hub() {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(&r, "package.json", r#"{"name":"root","private":true}"#);
    write(&r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    for p in ["a", "b", "c"] {
        write(
            &r,
            &format!("packages/{p}/package.json"),
            &format!(
                r#"{{"name":"{p}","version":"1.0.0","scripts":{{"clean-code":"knip --directory ../.. --workspace packages/{p}"}}}}"#
            ),
        );
        write(
            &r,
            &format!("packages/{p}/src/index.ts"),
            "export const x = 1;\n",
        );
    }
    write(&r, "docs/notes.ts", "export const n = 1;\n");
    // Not traced (no language): reached only through the hub.
    write(&r, "docs/guide.md", "# guide\n");
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
    let tasks: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|p| {
            g.node_by_path(&format!("task:packages/{p}#clean-code"))
                .unwrap_or_else(|| panic!("task of {p}"))
        })
        .collect();
    let mut entries = NodeSet::new(g.node_count());
    for &n in &tasks {
        entries.insert(n);
    }
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, "docs/notes.ts".to_owned());
    let rep = Engine::new(&g).affected(&cs, &entries, &AffectedOptions::default());
    for (p, n) in ["a", "b", "c"].iter().zip(&tasks) {
        let v = rep
            .entries
            .iter()
            .find(|e| e.entry == *n)
            .map_or(Verdict::Unaffected, |e| e.verdict);
        assert_eq!(v, Verdict::Affected, "task of {p}");
    }

    // The hub's edges are expanded from the walked files when the graph is
    // composed: one hub node with an edge to every walked file, traced or not.
    let hubs: Vec<_> = (0..g.node_count())
        .map(turborepo_trace_graph::NodeId)
        .filter(|&n| {
            let p = g.path(n);
            p.starts_with("members:") && !p.starts_with("members:packages/")
        })
        .collect();
    assert_eq!(hubs.len(), 1, "one shared repo-wide hub node");
    let targets: Vec<&str> = g
        .out_edges(hubs[0])
        .filter_map(|e| e.to)
        .map(|n| g.path(n))
        .collect();
    for f in [
        "docs/guide.md",
        "docs/notes.ts",
        "package.json",
        "packages/a/src/index.ts",
        "packages/c/package.json",
    ] {
        assert!(targets.contains(&f), "hub edge to {f}: {targets:?}");
    }
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, "docs/guide.md".to_owned());
    let rep = Engine::new(&g).affected(&cs, &entries, &AffectedOptions::default());
    for (p, n) in ["a", "b", "c"].iter().zip(&tasks) {
        let v = rep
            .entries
            .iter()
            .find(|e| e.entry == *n)
            .map_or(Verdict::Unaffected, |e| e.verdict);
        assert_eq!(
            v,
            Verdict::Affected,
            "task of {p} after an untraced file edit"
        );
    }
}
