//! Generated-file linking through directory listings inside an output
//! (precision P1b).
//!
//! A root task's `apps/**/*.gen.ts` output is scoped to the root package:
//! nested workspace packages (`apps/web`) are excluded. (The pattern keeps the
//! listers' own probes of `apps/tsconfig.json` and the like out of the output,
//! so only the listing rule links.) A file that lists a directory inside such
//! an excluded package must not be linked to the producer (the output never
//! writes there); a listing in a directory the output does cover still links.

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

fn linked_to(g: &Graph, from: &str, task: &str) -> bool {
    let n = g
        .node_by_path(from)
        .unwrap_or_else(|| panic!("node {from}"));
    let t = g
        .node_by_path(task)
        .unwrap_or_else(|| panic!("node {task}"));
    g.out_edges(n).any(|e| e.to == Some(t))
}

#[test]
fn listings_in_excluded_packages_do_not_link_to_the_producer() {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        &r,
        "package.json",
        r#"{"name":"root","private":true,"scripts":{"gen":"node gen.js"}}"#,
    );
    write(&r, "gen.js", "console.log(1);\n");
    write(&r, "pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n");
    write(
        &r,
        "turbo.json",
        r#"{"tasks":{"//#gen":{"outputs":["apps/**/*.gen.ts"]}}}"#,
    );
    let lister = "const fs = require('fs');\nconst path = require('path');\nmodule.exports = \
                  fs.readdirSync(path.join(__dirname, 'fixtures'));\n";
    // A nested package: excluded from the root task's output.
    write(
        &r,
        "apps/web/package.json",
        r#"{"name":"web","version":"1.0.0"}"#,
    );
    write(&r, "apps/web/src/a.js", lister);
    write(&r, "apps/web/src/fixtures/f.json", "{}\n");
    // Not a package: part of the root package, covered by the output.
    write(&r, "apps/tools/b.js", lister);
    write(&r, "apps/tools/fixtures/g.json", "{}\n");
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
    let task = "task:.#gen";
    assert!(
        (0..g.node_count()).any(|i| g.path(NodeId(i)) == task),
        "the root task exists"
    );
    assert!(
        linked_to(&g, "apps/tools/b.js", task),
        "a listing inside the output links to its producer"
    );
    let a = g.node_by_path("apps/web/src/a.js").unwrap();
    let why = || {
        let edges: Vec<String> = g
            .out_edges(a)
            .map(|e| format!("{:?} -> {:?}", e.specifier, e.to.map(|n| g.path(n))))
            .collect();
        let members: Vec<String> = g
            .memberships(a)
            .map(|m| format!("{}/{}", m.dir, m.pattern))
            .collect();
        let probes: Vec<&str> = g.probes(a).collect();
        format!("edges {edges:?}; memberships {members:?}; probes {probes:?}")
    };
    assert!(
        !linked_to(&g, "apps/web/src/a.js", task),
        "a listing inside an excluded nested package does not: {}",
        why()
    );
}
