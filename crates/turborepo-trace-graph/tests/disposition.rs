//! How `affected` treats each class of uncertainty record
//! ([`AffectedOptions::disposition`]).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{
    AffectedOptions, ChangeKind, ChangeSet, Disposition, Effect, Engine, GraphView, MemGraph,
    NodeSet, Origin, RefKind, UncertaintyKind, Verdict,
};

/// One test per uncertain module:
///
/// ```text
/// test/unresolved.test.ts → src/unresolved.ts  (import './missing' doesn't resolve; probes src/missing.ts)
/// test/generated.test.ts  → src/generated.ts   (committed generated file, no producing task)
/// test/envread.test.ts    → src/envread.ts     (readFileSync(process.env.X))
/// test/envload.test.ts    → src/envload.ts     (require(process.env.X))
/// test/param.test.ts      → src/param.ts       (readFileSync(p), p a parameter)
/// test/paramload.test.ts  → src/paramload.ts   (require(p), p a parameter)
/// test/data.test.ts       → src/data.ts        (readFileSync(row.path))
/// svc/app_test.go         → svc/app.go         (import of a module that isn't in the module cache)
/// test/plain.test.ts      → src/plain.ts
/// ```
fn graph() -> MemGraph {
    let mut b = MemGraph::builder();
    let pair = |b: &mut turborepo_trace_graph::GraphBuilder, name: &str| {
        let t = b.file(&format!("test/{name}.test.ts"));
        let s = b.file(&format!("src/{name}.ts"));
        b.edge(t, s, RefKind::Import);
        s
    };
    let unresolved = pair(&mut b, "unresolved");
    b.uncertainty(
        unresolved,
        UncertaintyKind::UnresolvedSpecifier,
        1,
        "cannot resolve './missing'",
    )
    .uncertainty_class(Effect::Load, Origin::Unknown);
    b.probe(unresolved, "src/missing.ts");
    let generated = pair(&mut b, "generated");
    b.uncertainty(
        generated,
        UncertaintyKind::GeneratedFile,
        1,
        "generated, no task produces it",
    );
    let envread = pair(&mut b, "envread");
    b.uncertainty(envread, UncertaintyKind::EnvDependent, 2, "process.env.X")
        .uncertainty_class(Effect::Read, Origin::Env);
    let envload = pair(&mut b, "envload");
    b.uncertainty(envload, UncertaintyKind::EnvDependent, 2, "process.env.X")
        .uncertainty_class(Effect::Load, Origin::Env);
    let param = pair(&mut b, "param");
    b.uncertainty(param, UncertaintyKind::ComputedSpecifier, 3, "p")
        .uncertainty_class(Effect::Read, Origin::Param);
    let paramload = pair(&mut b, "paramload");
    b.uncertainty(paramload, UncertaintyKind::ComputedSpecifier, 3, "p")
        .uncertainty_class(Effect::Load, Origin::Param);
    let data = pair(&mut b, "data");
    b.uncertainty(data, UncertaintyKind::ComputedSpecifier, 4, "row.path")
        .uncertainty_class(Effect::Read, Origin::Data);
    pair(&mut b, "plain");
    let go_test = b.file("svc/app_test.go");
    let go = b.file("svc/app.go");
    b.edge(go_test, go, RefKind::Import);
    b.uncertainty(
        go,
        UncertaintyKind::UnresolvedSpecifier,
        5,
        "no required module provides package example.com/dep",
    )
    .uncertainty_class(Effect::Load, Origin::Unknown);
    b.file("README.md");
    b.build().unwrap()
}

fn entries(g: &MemGraph) -> NodeSet {
    Engine::new(g)
        .query("test/** + svc/app_test.go")
        .unwrap()
        .nodes
}

fn selected(g: &MemGraph, kind: ChangeKind, path: &str, opts: &AffectedOptions) -> Vec<String> {
    let mut cs = ChangeSet::new();
    cs.push(kind, path);
    let e = Engine::new(g);
    let mut v: Vec<String> = e
        .affected(&cs, &entries(g), opts)
        .entries
        .iter()
        .filter(|r| r.verdict == Verdict::Affected)
        .map(|r| g.path(r.entry).to_owned())
        .collect();
    v.sort();
    v
}

fn dispositions(g: &MemGraph, opts: &AffectedOptions) -> Vec<(String, Disposition)> {
    let mut v = Vec::new();
    for i in 0..g.node_count() {
        let n = turborepo_trace_graph::NodeId(i);
        for u in g.uncertainties(n) {
            v.push((g.path(n).to_owned(), opts.disposition(&u, g.lang(n))));
        }
    }
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[test]
fn default_dispositions() {
    let g = graph();
    let d = dispositions(&g, &AffectedOptions::default());
    let expect = [
        ("src/data.ts", Disposition::Open),
        ("src/envload.ts", Disposition::Open),
        ("src/envread.ts", Disposition::Ignored),
        ("src/generated.ts", Disposition::Ignored),
        ("src/param.ts", Disposition::Open),
        ("src/paramload.ts", Disposition::Open),
        ("src/unresolved.ts", Disposition::Probed),
        ("svc/app.go", Disposition::Open),
    ];
    assert_eq!(
        d,
        expect.map(|(p, x)| (p.to_owned(), x)),
        "a read of a path from outside the repository and an unresolved JS import are not open; a \
         module load, a parameter-derived read and an unresolved Go import are"
    );
    let e = Engine::new(&g);
    let c = e.uncertainty_counts(&AffectedOptions::default());
    assert_eq!((c.open, c.covered, c.probed, c.ignored), (5, 1, 1, 2));
}

#[test]
fn only_open_records_select_on_unrelated_changes() {
    let g = graph();
    let opts = AffectedOptions::default();
    // An unrelated change: only the tests that reach an open record.
    assert_eq!(
        selected(&g, ChangeKind::Modified, "README.md", &opts),
        [
            "svc/app_test.go",
            "test/data.test.ts",
            "test/envload.test.ts",
            "test/param.test.ts",
            "test/paramload.test.ts"
        ]
    );
    // The unresolved import is covered by its probes: adding the missing file
    // selects it.
    assert_eq!(
        selected(&g, ChangeKind::Added, "src/missing.ts", &opts),
        [
            "svc/app_test.go",
            "test/data.test.ts",
            "test/envload.test.ts",
            "test/param.test.ts",
            "test/paramload.test.ts",
            "test/unresolved.test.ts"
        ]
    );
    // Edits of the uncertain files themselves select their tests as usual.
    assert_eq!(
        selected(&g, ChangeKind::Modified, "src/generated.ts", &opts),
        [
            "svc/app_test.go",
            "test/data.test.ts",
            "test/envload.test.ts",
            "test/generated.test.ts",
            "test/param.test.ts",
            "test/paramload.test.ts"
        ]
    );
}

#[test]
fn strict_counts_everything() {
    let g = graph();
    let opts = AffectedOptions::default().strict(true);
    assert!(
        dispositions(&g, &opts)
            .iter()
            .all(|(_, d)| *d == Disposition::Open)
    );
    assert_eq!(
        selected(&g, ChangeKind::Modified, "README.md", &opts),
        [
            "svc/app_test.go",
            "test/data.test.ts",
            "test/envload.test.ts",
            "test/envread.test.ts",
            "test/generated.test.ts",
            "test/param.test.ts",
            "test/paramload.test.ts",
            "test/unresolved.test.ts"
        ]
    );
}
