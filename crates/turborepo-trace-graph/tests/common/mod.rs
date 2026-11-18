//! Shared fixture.

use turborepo_trace_graph::{EdgeSpec, MemGraph, RefFlags, RefKind, UncertaintyKind};

/// ```text
/// test/a.test.ts → src/a.ts → src/shared.ts → src/util.ts → node_modules/lodash/index.js (external)
///                  src/a.ts ⇢ src/types.ts (type-only)
/// test/b.test.ts → src/b.ts → src/shared.ts (require)
///                  src/b.ts → src/x.ts ⇄ src/y.ts (cycle)
/// test/c.test.ts → src/c.ts → data/config.json (fs_read), src/c.ts → ? (unresolved)
/// test/d.test.ts → src/dyn.ts (computed_specifier)
/// d/1.ts → d/2.ts, d/3.ts → d/4.ts (diamond)
/// probes on src/a.ts: src/a-helper.ts, node_modules/left-pad
/// membership on src/c.ts: locales/*.json
/// ```
pub fn graph() -> MemGraph {
    let mut b = MemGraph::builder();
    let ta = b.file("test/a.test.ts");
    let tb = b.file("test/b.test.ts");
    let tc = b.file("test/c.test.ts");
    let td = b.file("test/d.test.ts");
    let a = b.file("src/a.ts");
    let bb = b.file("src/b.ts");
    let c = b.file("src/c.ts");
    let shared = b.file("src/shared.ts");
    let util = b.file("src/util.ts");
    let types = b.file("src/types.ts");
    let x = b.file("src/x.ts");
    let y = b.file("src/y.ts");
    let dynf = b.file("src/dyn.ts");
    let cfg = b.file("data/config.json");
    let lodash_pkg = b.package("lodash", Some("4.17.21"), "node_modules/lodash");
    let lodash = b.add_file("node_modules/lodash/index.js", lodash_pkg, Some("js"), true);
    let d1 = b.file("d/1.ts");
    let d2 = b.file("d/2.ts");
    let d3 = b.file("d/3.ts");
    let d4 = b.file("d/4.ts");

    b.edge(ta, a, RefKind::Import)
        .edge(tb, bb, RefKind::Import)
        .edge(tc, c, RefKind::Import)
        .edge(td, dynf, RefKind::Import)
        .edge(a, shared, RefKind::Import)
        .add_edge(
            a,
            EdgeSpec::new(Some(types), RefKind::Import).flags(RefFlags::TYPE_ONLY),
        )
        .edge(bb, shared, RefKind::Require)
        .edge(bb, x, RefKind::Import)
        .edge(x, y, RefKind::Import)
        .edge(y, x, RefKind::Import)
        .edge(shared, util, RefKind::Import)
        .edge(util, lodash, RefKind::Require)
        .edge(c, cfg, RefKind::FsRead)
        .add_edge(
            c,
            EdgeSpec::new(None, RefKind::Require)
                .specifier("./missing")
                .line(7),
        )
        .edge(d1, d2, RefKind::Import)
        .edge(d1, d3, RefKind::Import)
        .edge(d2, d4, RefKind::Import)
        .edge(d3, d4, RefKind::Import);
    b.uncertainty(dynf, UncertaintyKind::ComputedSpecifier, 3, "require(name)");
    b.uncertainty(lodash, UncertaintyKind::EvalLike, 10, "new Function(...)");
    b.probe(a, "src/a-helper.ts")
        .probe(a, "node_modules/left-pad");
    b.membership(c, "locales", "*.json");
    b.build().unwrap()
}
