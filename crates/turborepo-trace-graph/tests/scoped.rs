//! Scoped (covered) uncertainty and the `eval_like` default.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{
    AffectedOptions, Cause, ChangeKind, ChangeSet, Engine, GraphView, MemGraph, NodeSet, Policy,
    RefKind, UncertaintyKind, Verdict,
};

/// ```text
/// test/scan.test.ts → src/scan.ts   (covered pattern_too_broad, scope data/**)
/// test/open.test.ts → src/open.ts   (open computed_specifier)
/// test/eval.test.ts → src/eval.ts   (eval_like)
/// test/plain.test.ts → src/plain.ts
/// data/a.json, other/b.json: files (not edges of src/scan.ts: beyond the match cap)
/// ```
fn graph() -> MemGraph {
    let mut b = MemGraph::builder();
    let t_scan = b.file("test/scan.test.ts");
    let t_open = b.file("test/open.test.ts");
    let t_eval = b.file("test/eval.test.ts");
    let t_plain = b.file("test/plain.test.ts");
    let scan = b.file("src/scan.ts");
    let open = b.file("src/open.ts");
    let ev = b.file("src/eval.ts");
    let plain = b.file("src/plain.ts");
    b.file("data/a.json");
    b.file("other/b.json");
    b.edge(t_scan, scan, RefKind::Import)
        .edge(t_open, open, RefKind::Import)
        .edge(t_eval, ev, RefKind::Import)
        .edge(t_plain, plain, RefKind::Import);
    b.covered_uncertainty(
        scan,
        UncertaintyKind::PatternTooBroad,
        4,
        "data/**: more than 1000 files; truncated",
        "data",
        "**",
    );
    b.membership(scan, "data", "**");
    b.uncertainty(
        open,
        UncertaintyKind::ComputedSpecifier,
        2,
        "readFileSync(p)",
    );
    b.uncertainty(ev, UncertaintyKind::EvalLike, 1, "new Function('x')");
    b.build().unwrap()
}

fn entries(g: &MemGraph) -> NodeSet {
    let e = Engine::new(g);
    e.query("test/**").unwrap().nodes
}

fn run(g: &MemGraph, cs: &ChangeSet, opts: &AffectedOptions) -> Vec<(String, Verdict)> {
    let e = Engine::new(g);
    let mut v: Vec<(String, Verdict)> = e
        .affected(cs, &entries(g), opts)
        .entries
        .iter()
        .map(|r| (g.path(r.entry).to_owned(), r.verdict))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn one(kind: ChangeKind, path: &str) -> ChangeSet {
    let mut cs = ChangeSet::new();
    cs.push(kind, path);
    cs
}

fn selected(g: &MemGraph, cs: &ChangeSet, opts: &AffectedOptions) -> Vec<String> {
    run(g, cs, opts)
        .into_iter()
        .filter(|(_, v)| *v == Verdict::Affected)
        .map(|(p, _)| p)
        .collect()
}

const A: Verdict = Verdict::Affected;

#[test]
fn covered_uncertainty_selects_on_changes_inside_its_scope() {
    let g = graph();
    let opts = AffectedOptions::default();
    // An edit outside every scope: only the open uncertainty selects (eval is
    // ignored by default), not the covered one.
    assert_eq!(
        run(&g, &one(ChangeKind::Modified, "other/b.json"), &opts),
        [("test/open.test.ts".to_owned(), A)]
    );
    // Edit, addition and deletion inside the scope select the covered test (and the
    // open one, as on any change).
    for kind in [ChangeKind::Modified, ChangeKind::Added, ChangeKind::Deleted] {
        let path = if kind == ChangeKind::Added {
            "data/new/c.json"
        } else {
            "data/a.json"
        };
        assert_eq!(
            selected(&g, &one(kind, path), &opts),
            ["test/open.test.ts", "test/scan.test.ts"],
            "{kind:?} {path}"
        );
    }
    // The cause names the scope.
    let e = Engine::new(&g);
    let rep = e.affected(
        &one(ChangeKind::Modified, "data/a.json"),
        &entries(&g),
        &opts,
    );
    let scan = rep
        .entries
        .iter()
        .find(|r| g.path(r.entry) == "test/scan.test.ts")
        .unwrap();
    assert_eq!(
        scan.cause,
        Some(Cause::Scope {
            path: "data/a.json".into(),
            kind: ChangeKind::Modified,
            dir: "data".into(),
            pattern: "**".into(),
        })
    );
    assert!(scan.uncertain.is_none());
    // A path that only shares the directory's name prefix is outside.
    assert_eq!(
        selected(&g, &one(ChangeKind::Added, "data2/x.json"), &opts),
        ["test/open.test.ts"]
    );
}

#[test]
fn open_uncertainty_selects_on_any_change() {
    let g = graph();
    for policy in [Policy::Affected, Policy::Unknown] {
        let opts = AffectedOptions::default().policy(policy);
        let v = run(&g, &one(ChangeKind::Modified, "src/plain.ts"), &opts);
        let want = if policy == Policy::Affected {
            Verdict::Affected
        } else {
            Verdict::Unknown
        };
        assert!(v.contains(&("test/open.test.ts".to_owned(), want)), "{v:?}");
        assert!(!v.iter().any(|(p, _)| p == "test/scan.test.ts"), "{v:?}");
    }
    // Policy `ignore` keeps the scope's seeds: it only drops open uncertainty.
    let opts = AffectedOptions::default().policy(Policy::Ignore);
    assert_eq!(
        selected(&g, &one(ChangeKind::Modified, "data/a.json"), &opts),
        ["test/scan.test.ts"]
    );
}

#[test]
fn eval_is_ignored_by_default_and_counted_when_strict() {
    let g = graph();
    let cs = one(ChangeKind::Modified, "src/plain.ts");
    let default = selected(&g, &cs, &AffectedOptions::default());
    assert!(!default.contains(&"test/eval.test.ts".to_owned()));
    let strict = selected(&g, &cs, &AffectedOptions::default().strict_eval(true));
    assert!(strict.contains(&"test/eval.test.ts".to_owned()));

    // Reported: counts and per-entry notes.
    let e = Engine::new(&g);
    let c = e.uncertainty_counts(&AffectedOptions::default());
    assert_eq!((c.open, c.covered, c.ignored), (1, 1, 1));
    let c = e.uncertainty_counts(&AffectedOptions::default().strict_eval(true));
    assert_eq!((c.open, c.covered, c.ignored), (2, 1, 0));
    let t_eval = g.node_by_path("test/eval.test.ts").unwrap();
    let notes = e.uncertainty_notes(t_eval, &AffectedOptions::default());
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, UncertaintyKind::EvalLike);
    assert!(notes[0].scope.is_none());
    assert!(
        e.uncertainty_notes(t_eval, &AffectedOptions::default().strict_eval(true))
            .is_empty()
    );
    let t_scan = g.node_by_path("test/scan.test.ts").unwrap();
    let notes = e.uncertainty_notes(t_scan, &AffectedOptions::default());
    assert_eq!(notes[0].scope, Some(("data".into(), "**".into())));
}

#[test]
fn scopes_round_trip_through_json() {
    let g = graph();
    let mut buf = Vec::new();
    turborepo_trace_graph::json::write_json(&g, None, false, &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    assert!(text.contains("\"scope\""), "{text}");
    let back = MemGraph::from_json_str(&text).unwrap();
    let scan = back.node_by_path("src/scan.ts").unwrap();
    let u: Vec<_> = back.uncertainties(scan).collect();
    assert_eq!(u.len(), 1);
    let sc = u[0].scope.unwrap();
    assert_eq!((sc.dir, sc.pattern), ("data", "**"));
    let open = back.node_by_path("src/open.ts").unwrap();
    assert!(back.uncertainties(open).all(|u| u.scope.is_none()));
}
