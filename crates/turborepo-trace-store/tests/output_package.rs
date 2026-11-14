//! Output packages (`TaskSpec::output_package`) in the composed graph: Prisma's
//! default client is the package `@prisma/client`, so its installed opaque node
//! links back to `prisma generate`, and so do the importers of the package.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, sync::Arc};

use turborepo_trace_core::Language;
use turborepo_trace_graph::{GraphView, NodeId};
use turborepo_trace_store::{Externals, Graph, GraphOptions, build};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contents).unwrap();
}

#[test]
fn installed_client_links_its_opaque_node() {
    let t = tempfile::tempdir().unwrap();
    let r = std::fs::canonicalize(t.path()).unwrap();
    write(&r, ".gitignore", "node_modules\n.turbotrace\n");
    write(
        &r,
        "package.json",
        r#"{"name":"app","scripts":{"gen":"prisma generate"},"dependencies":{"@prisma/client":"5.0.0"}}"#,
    );
    write(
        &r,
        "prisma/schema.prisma",
        "generator client {\n  provider = \"prisma-client-js\"\n}\n",
    );
    write(
        &r,
        "src/db.ts",
        "import { PrismaClient } from '@prisma/client';\nexport const db = new PrismaClient();\n",
    );
    let pkg = "node_modules/.pnpm/@prisma+client@5.0.0/node_modules/@prisma/client";
    write(
        &r,
        &format!("{pkg}/package.json"),
        r#"{"name":"@prisma/client","version":"5.0.0","main":"index.js"}"#,
    );
    write(&r, &format!("{pkg}/index.js"), "module.exports = {};\n");
    std::fs::create_dir_all(r.join("node_modules/@prisma")).unwrap();
    turborepo_trace_core::os::symlink(
        "../.pnpm/@prisma+client@5.0.0/node_modules/@prisma/client",
        r.join("node_modules/@prisma/client"),
    )
    .unwrap();
    write(
        &r,
        "pnpm-lock.yaml",
        "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      '@prisma/client':\n        specifier: 5.0.0\n        version: 5.0.0\npackages:\n  '@prisma/client@5.0.0':\n    resolution: {integrity: sha512-x}\nsnapshots:\n  '@prisma/client@5.0.0': {}\n",
    );
    let out = r.join(".turbotrace");
    let langs: Vec<Arc<dyn Language>> = vec![Arc::new(turborepo_trace_lang_js::JsLanguage)];
    let opts = GraphOptions {
        root: r.clone(),
        out: Some(out.clone()),
        externals: Externals::Opaque,
        threads: 2,
        ..Default::default()
    };
    build(&opts, &langs).unwrap();
    let g = Graph::open(&out).unwrap();
    let targets = |n: NodeId| -> Vec<&str> {
        g.out_edges(n)
            .filter_map(|e| e.to)
            .map(|n| g.path(n))
            .collect()
    };
    let client = g
        .node_by_path("node_modules/@prisma/client@5.0.0")
        .expect("opaque node");
    assert!(
        targets(client).contains(&"task:.#gen"),
        "{:?}",
        targets(client)
    );
    let db = g.node_by_path("src/db.ts").unwrap();
    assert!(
        targets(db).contains(&"node_modules/@prisma/client@5.0.0"),
        "{:?}",
        targets(db)
    );
}
