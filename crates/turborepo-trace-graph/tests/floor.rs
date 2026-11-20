//! The uncertainty floor report ([`Engine::floor`]).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{
    AffectedOptions, ChangeSet, EdgeFilter, EdgeSpec, Effect, Engine, EngineOptions, FloorOrder,
    FloorReport, GraphBuilder, GraphView, MemGraph, NodeId, NodeSet, Origin, RefFlags, RefKind,
    UncertaintyKind, Verdict,
    floor::{AssumedSite, ClassPattern, FloorAssumptions},
};

fn open(b: &mut GraphBuilder, n: NodeId, kind: UncertaintyKind, line: u32, detail: &str) {
    b.uncertainty(n, kind, line, detail);
}

/// A hub, two smaller sites, a site nothing reaches, and records that aren't
/// open:
///
/// ```text
/// test/t1.test.ts → src/a.ts → src/hub.ts      (two reads of row.path)
/// test/t2.test.ts → src/a.ts
/// test/t3.test.ts → src/b.ts → src/hub.ts, src/plugin.ts   (require(name))
/// test/t4.test.ts → src/cwd.ts                 (a cwd-relative read)
/// test/t5.test.ts → src/quiet.ts               (probed, ignored, scoped and membership records)
///                 → node_modules/dep/index.js  (external: require(x))
/// src/orphan.ts                                (require(name), reached by no test)
/// ```
fn hubs() -> MemGraph {
    let mut b = MemGraph::builder();
    let t: Vec<NodeId> = (1..=5)
        .map(|i| b.file(&format!("test/t{i}.test.ts")))
        .collect();
    let a = b.file("src/a.ts");
    let bb = b.file("src/b.ts");
    let hub = b.file("src/hub.ts");
    let plugin = b.file("src/plugin.ts");
    let cwd = b.file("src/cwd.ts");
    let quiet = b.file("src/quiet.ts");
    let orphan = b.file("src/orphan.ts");
    let dep = b.package("dep", Some("1.0.0"), "node_modules/dep");
    let ext = b.add_file("node_modules/dep/index.js", dep, Some("js"), true);
    b.edge(t[0], a, RefKind::Import)
        .edge(t[1], a, RefKind::Import)
        .edge(a, hub, RefKind::Import)
        .edge(t[2], bb, RefKind::Import)
        .edge(bb, hub, RefKind::Import)
        .edge(bb, plugin, RefKind::Import)
        .edge(t[3], cwd, RefKind::Import)
        .edge(t[4], quiet, RefKind::Import)
        .edge(t[4], ext, RefKind::Import);

    open(
        &mut b,
        hub,
        UncertaintyKind::ComputedSpecifier,
        9,
        "readFileSync(other.path)",
    );
    b.uncertainty_class(Effect::Read, Origin::Data);
    open(
        &mut b,
        hub,
        UncertaintyKind::ComputedSpecifier,
        3,
        "readFileSync(row.path)",
    );
    b.uncertainty_class(Effect::Read, Origin::Data);
    open(
        &mut b,
        plugin,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(name)",
    );
    b.uncertainty_class(Effect::Load, Origin::Unknown);
    open(
        &mut b,
        orphan,
        UncertaintyKind::ComputedSpecifier,
        2,
        "require(name)",
    );
    b.uncertainty_class(Effect::Load, Origin::Unknown);
    open(
        &mut b,
        cwd,
        UncertaintyKind::CwdDependent,
        4,
        "readFileSync('data.json')",
    );
    b.uncertainty_class(Effect::Read, Origin::Cwd);
    open(
        &mut b,
        ext,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    b.uncertainty_class(Effect::Load, Origin::Unknown);

    // Not open by default.
    open(
        &mut b,
        quiet,
        UncertaintyKind::UnresolvedSpecifier,
        1,
        "cannot resolve './missing'",
    );
    b.uncertainty_class(Effect::Load, Origin::Unknown);
    open(&mut b, quiet, UncertaintyKind::EvalLike, 2, "eval(code)");
    open(
        &mut b,
        quiet,
        UncertaintyKind::EnvDependent,
        3,
        "readFileSync(process.env.DATA)",
    );
    b.uncertainty_class(Effect::Read, Origin::Env);
    open(
        &mut b,
        quiet,
        UncertaintyKind::PartialSpecifier,
        4,
        "require('./x/' + n)",
    );
    b.covered_uncertainty(
        quiet,
        UncertaintyKind::PatternTooBroad,
        5,
        "readdir truncated",
        "data",
        "**",
    );
    b.build().unwrap()
}

fn tests(g: &MemGraph) -> NodeSet {
    Engine::new(g).query("test/**").unwrap().nodes
}

fn sites(g: &MemGraph, r: &FloorReport) -> Vec<(String, usize, usize)> {
    r.sites
        .iter()
        .map(|s| (g.path(s.node).to_owned(), s.reached, s.exclusive))
        .collect()
}

fn greedy(g: &MemGraph, r: &FloorReport) -> Vec<(String, usize, usize)> {
    r.greedy
        .iter()
        .map(|p| {
            (
                g.path(r.sites[p.site].node).to_owned(),
                p.freed,
                p.remaining,
            )
        })
        .collect()
}

fn classes(r: &FloorReport) -> Vec<(String, [usize; 5])> {
    r.classes
        .iter()
        .map(|c| {
            (
                c.class.to_string(),
                [c.records, c.files, c.files_reached, c.reached, c.exclusive],
            )
        })
        .collect()
}

fn s(path: &str, a: usize, b: usize) -> (String, usize, usize) {
    (path.to_owned(), a, b)
}

/// The entries `affected` selects for a change that touches nothing.
fn selected_without_a_change(
    g: &MemGraph,
    e: &Engine<'_, MemGraph>,
    opts: &AffectedOptions,
) -> usize {
    let report = e.affected(&ChangeSet::new(), &tests(g), opts);
    report
        .entries
        .iter()
        .filter(|r| r.verdict == Verdict::Affected && r.uncertain.is_some())
        .count()
}

#[test]
fn totals_classes_sites_and_greedy_order() {
    let g = hubs();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();
    let r = e.floor(&tests(&g), &opts, 25);

    assert_eq!((r.entries, r.floor), (5, 4));
    assert_eq!(r.floor, selected_without_a_change(&g, &e, &opts));
    let c = r.records;
    assert_eq!(
        (c.open, c.scoped, c.probed, c.ignored, c.membership),
        (5, 1, 1, 2, 1),
        "the external record is not counted"
    );
    assert_eq!(c.open, r.classes.iter().map(|c| c.records).sum::<usize>());

    // Most entries first; then most exclusive entries.
    assert_eq!(
        classes(&r),
        [
            ("computed_specifier/read/data".to_owned(), [2, 1, 1, 3, 2]),
            ("cwd_dependent/read/cwd".to_owned(), [1, 1, 1, 1, 1]),
            (
                "computed_specifier/load/unknown".to_owned(),
                [2, 2, 1, 1, 0]
            ),
        ]
    );
    // Every site, the unreached one included; ties by path.
    assert_eq!(
        sites(&g, &r),
        [
            s("src/hub.ts", 3, 2),
            s("src/cwd.ts", 1, 1),
            s("src/plugin.ts", 1, 0),
            s("src/orphan.ts", 0, 0),
        ]
    );
    assert_eq!(r.sites_reached(), 3);
    // Records by line.
    let hub = &r.sites[0].records;
    assert_eq!(
        hub.iter()
            .map(|r| (r.line, r.detail.as_str()))
            .collect::<Vec<_>>(),
        [
            (3, "readFileSync(row.path)"),
            (9, "readFileSync(other.path)")
        ]
    );
    // The hub frees t1 and t2; then t3 only has the plugin left, and the two
    // remaining sites tie (one entry freed, one entry reaching): the smaller
    // path goes first. The site nothing reaches is never picked.
    assert_eq!(
        greedy(&g, &r),
        [
            s("src/hub.ts", 2, 2),
            s("src/cwd.ts", 1, 1),
            s("src/plugin.ts", 1, 0),
        ]
    );

    // `picks` only limits the greedy order.
    let one = e.floor(&tests(&g), &opts, 1);
    assert_eq!(greedy(&g, &one), [s("src/hub.ts", 2, 2)]);
    assert_eq!(one.sites, r.sites);
    assert_eq!(one.classes, r.classes);
    assert!(e.floor(&tests(&g), &opts, 0).greedy.is_empty());
}

#[test]
fn strict_options_open_more_records() {
    let g = hubs();
    let e = Engine::new(&g);

    // --strict-eval: the eval in src/quiet.ts counts, and puts t5 on the floor.
    let opts = AffectedOptions::default().strict_eval(true);
    let r = e.floor(&tests(&g), &opts, 25);
    assert_eq!((r.entries, r.floor), (5, 5));
    assert_eq!(r.floor, selected_without_a_change(&g, &e, &opts));
    let c = r.records;
    assert_eq!(
        (c.open, c.scoped, c.probed, c.ignored, c.membership),
        (6, 1, 1, 1, 1)
    );
    assert!(sites(&g, &r).contains(&s("src/quiet.ts", 1, 1)));
    assert!(
        classes(&r).contains(&("eval_like/none/unknown".to_owned(), [1, 1, 1, 1, 1])),
        "{:?}",
        classes(&r)
    );

    // --strict: probed and ignored records count too; scoped ones and memberships
    // don't.
    let opts = AffectedOptions::default().strict(true);
    let r = e.floor(&tests(&g), &opts, 25);
    assert_eq!(r.floor, 5);
    assert_eq!(r.floor, selected_without_a_change(&g, &e, &opts));
    let c = r.records;
    assert_eq!(
        (c.open, c.scoped, c.probed, c.ignored, c.membership),
        (8, 1, 0, 0, 1)
    );
    let quiet = r
        .sites
        .iter()
        .find(|s| g.path(s.node) == "src/quiet.ts")
        .unwrap();
    assert_eq!(quiet.records.len(), 3);
    // t5 reaches three classes, so it is exclusive to none.
    assert_eq!(
        classes(&r)
            .iter()
            .filter(|(c, _)| c.contains("env_dependent")
                || c.contains("eval")
                || c.contains("unresolved"))
            .map(|(_, n)| (n[3], n[4]))
            .collect::<Vec<_>>(),
        [(1, 0); 3]
    );
}

#[test]
fn follows_the_traversal_filter() {
    let mut b = MemGraph::builder();
    let typed = b.file("test/typed.test.ts");
    let direct = b.file("test/direct.test.ts");
    let site = b.file("src/site.ts");
    b.add_edge(
        typed,
        EdgeSpec::new(Some(site), RefKind::Import).flags(RefFlags::TYPE_ONLY),
    );
    b.edge(direct, site, RefKind::Import);
    open(
        &mut b,
        site,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    let g = b.build().unwrap();
    let opts = AffectedOptions::default();

    let all = Engine::new(&g);
    let r = all.floor(&tests(&g), &opts, 25);
    assert_eq!((r.floor, r.sites[0].reached), (2, 2));

    // --runtime-only: the type-only import doesn't reach the site.
    let runtime = Engine::with_options(
        &g,
        EngineOptions {
            traversal: EdgeFilter::ALL.without_flags(RefFlags::TYPE_ONLY),
            ..EngineOptions::default()
        },
    );
    let r = runtime.floor(&tests(&g), &opts, 25);
    assert_eq!(
        (r.floor, r.sites[0].reached, r.sites[0].exclusive),
        (1, 1, 1)
    );
    assert_eq!(r.floor, selected_without_a_change(&g, &runtime, &opts));
    assert_eq!(greedy(&g, &r), [s("src/site.ts", 1, 0)]);
}

#[test]
fn runtime_entries_skip_type_only_edges() {
    // Two tests import the site's types only; one of them runs through a
    // type-checking runner. A third imports it as a module.
    let mut b = MemGraph::builder();
    let isolated = b.file("test/isolated.test.ts");
    let checked = b.file("test/checked.test.ts");
    let direct = b.file("test/direct.test.ts");
    let site = b.file("src/site.ts");
    for t in [isolated, checked] {
        b.add_edge(
            t,
            EdgeSpec::new(Some(site), RefKind::Import).flags(RefFlags::TYPE_ONLY),
        );
    }
    b.edge(direct, site, RefKind::Import);
    open(
        &mut b,
        site,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();
    let none = FloorAssumptions::default();
    let entries = tests(&g);
    let run = |runtime: &[NodeId]| {
        let runtime = NodeSet::from_iter_in(g.node_count(), runtime.iter().copied());
        e.floor_for_runners(&entries, &runtime, &opts, FloorOrder::Freed, 25, &none)
    };

    let r = run(&[]);
    assert_eq!((r.floor, r.runtime_entries), (3, 0));
    assert_eq!(r, e.floor(&entries, &opts, 25));
    // The transpile-only tests don't follow the type-only import; the other entry
    // does.
    let r = run(&[isolated, direct]);
    assert_eq!((r.entries, r.floor, r.runtime_entries), (3, 2, 2));
    assert_eq!(sites(&g, &r), [s("src/site.ts", 2, 2)]);
    assert_eq!(greedy(&g, &r), [s("src/site.ts", 2, 0)]);
    assert_eq!(r.to_value(&g)["runtime_only_entries"], 2);
    // Every entry a runtime entry: the floor of `--runtime-only`.
    let r = run(&[isolated, checked, direct]);
    assert_eq!((r.floor, r.runtime_entries), (1, 3));
    let runtime_only = Engine::with_options(
        &g,
        EngineOptions {
            traversal: EdgeFilter::ALL.without_flags(RefFlags::TYPE_ONLY),
            ..EngineOptions::default()
        },
    );
    assert_eq!(runtime_only.floor(&entries, &opts, 25).floor, 1);
    // Nodes outside the entries don't matter, and the what-if baseline uses the
    // same modes.
    let assume = FloorAssumptions {
        cut: vec!["src/site.ts".to_owned()],
        ..FloorAssumptions::default()
    };
    let runtime = NodeSet::from_iter_in(g.node_count(), [isolated, site]);
    let r = e.floor_for_runners(&entries, &runtime, &opts, FloorOrder::Freed, 25, &assume);
    let a = r.assumed.unwrap();
    assert_eq!(
        (r.floor, r.runtime_entries, a.floor_before, a.freed),
        (0, 1, 2, 2)
    );
}

#[test]
fn external_nodes_are_not_sites_but_are_traversed() {
    let mut b = MemGraph::builder();
    let through = b.file("test/through.test.ts");
    let only = b.file("test/only.test.ts");
    let deep = b.file("src/deep.ts");
    let dep = b.package("dep", Some("1.0.0"), "node_modules/dep");
    let ext = b.add_file("node_modules/dep/index.js", dep, Some("js"), true);
    let leaf = b.add_file("node_modules/dep/leaf.js", dep, Some("js"), true);
    b.edge(through, ext, RefKind::Import)
        .edge(ext, deep, RefKind::Import)
        .edge(only, leaf, RefKind::Import);
    open(
        &mut b,
        ext,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    open(
        &mut b,
        leaf,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    open(
        &mut b,
        deep,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();
    let r = e.floor(&tests(&g), &opts, 25);
    assert_eq!((r.entries, r.floor, r.records.open), (2, 1, 1));
    assert_eq!(sites(&g, &r), [s("src/deep.ts", 1, 1)]);
    assert_eq!(r.floor, selected_without_a_change(&g, &e, &opts));
}

#[test]
fn cycles() {
    // t1 → a ⇄ b → s1 → a (the site is on the cycle), b → s3
    // t2 ⇄ c → s2 → s2
    // t3 → d → d
    let mut b = MemGraph::builder();
    let t1 = b.file("test/t1.test.ts");
    let t2 = b.file("test/t2.test.ts");
    let t3 = b.file("test/t3.test.ts");
    let a = b.file("src/a.ts");
    let bb = b.file("src/b.ts");
    let c = b.file("src/c.ts");
    let d = b.file("src/d.ts");
    let s1 = b.file("src/s1.ts");
    let s2 = b.file("src/s2.ts");
    let s3 = b.file("src/s3.ts");
    for (from, to) in [
        (t1, a),
        (a, bb),
        (bb, a),
        (bb, s1),
        (s1, a),
        (bb, s3),
        (t2, c),
        (c, t2),
        (c, s2),
        (s2, s2),
        (t3, d),
        (d, d),
    ] {
        b.edge(from, to, RefKind::Import);
    }
    for site in [s1, s2, s3] {
        open(
            &mut b,
            site,
            UncertaintyKind::ComputedSpecifier,
            1,
            "require(x)",
        );
    }
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();

    let r = e.floor(&tests(&g), &opts, 25);
    assert_eq!((r.entries, r.floor), (3, 2));
    assert_eq!(
        sites(&g, &r),
        [
            s("src/s1.ts", 1, 0),
            s("src/s2.ts", 1, 1),
            s("src/s3.ts", 1, 0)
        ]
    );
    // s2 frees t2 at once. s1 and s3 are both needed for t1: the first frees
    // nothing.
    assert_eq!(
        greedy(&g, &r),
        [
            s("src/s2.ts", 1, 1),
            s("src/s1.ts", 0, 1),
            s("src/s3.ts", 1, 0)
        ]
    );

    // Entries on a cycle, one of them a site.
    let mut entries = NodeSet::new(g.node_count());
    for n in [a, bb, s1, c] {
        entries.insert(n);
    }
    let r = e.floor(&entries, &opts, 25);
    assert_eq!((r.entries, r.floor), (4, 4));
    assert_eq!(
        sites(&g, &r),
        [
            s("src/s1.ts", 3, 0),
            s("src/s3.ts", 3, 0),
            s("src/s2.ts", 1, 1)
        ]
    );
}

#[test]
fn greedy_ties_and_picks_that_free_nothing() {
    // t1, t2 → x, y;  t3 → y, z;  t4 → x, z
    let mut b = MemGraph::builder();
    let t: Vec<NodeId> = (1..=4)
        .map(|i| b.file(&format!("test/t{i}.test.ts")))
        .collect();
    // Declared out of path order: ties follow the path, not the node id.
    let z = b.file("src/z.ts");
    let y = b.file("src/y.ts");
    let x = b.file("src/x.ts");
    for (from, to) in [
        (t[0], x),
        (t[0], y),
        (t[1], x),
        (t[1], y),
        (t[2], y),
        (t[2], z),
        (t[3], x),
        (t[3], z),
    ] {
        b.edge(from, to, RefKind::Import);
    }
    for site in [x, y, z] {
        open(
            &mut b,
            site,
            UncertaintyKind::ComputedSpecifier,
            1,
            "require(x)",
        );
    }
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let r = e.floor(&tests(&g), &AffectedOptions::default(), 25);
    assert_eq!(r.floor, 4);
    assert_eq!(
        sites(&g, &r),
        [
            s("src/x.ts", 3, 0),
            s("src/y.ts", 3, 0),
            s("src/z.ts", 2, 0)
        ]
    );
    // No site frees an entry alone, so the first pick is the site most entries
    // reach: x and y tie, x has the smaller path. Then y frees t1 and t2, and z
    // the rest.
    assert_eq!(
        greedy(&g, &r),
        [
            s("src/x.ts", 0, 4),
            s("src/y.ts", 2, 2),
            s("src/z.ts", 2, 0)
        ]
    );
    // One class: every floored entry is exclusive to it.
    assert_eq!(
        classes(&r),
        [(
            "computed_specifier/none/unknown".to_owned(),
            [3, 3, 3, 4, 4]
        )]
    );
}

#[test]
fn weighted_order_ranks_sites_that_free_entries_together() {
    // t1, t2, t3 → p, q (both must be resolved);  t4 → a-small
    let mut b = MemGraph::builder();
    let t: Vec<NodeId> = (1..=4)
        .map(|i| b.file(&format!("test/t{i}.test.ts")))
        .collect();
    let p = b.file("src/p.ts");
    let q = b.file("src/q.ts");
    let small = b.file("src/a-small.ts");
    for &e in &t[..3] {
        b.edge(e, p, RefKind::Import).edge(e, q, RefKind::Import);
    }
    b.edge(t[3], small, RefKind::Import);
    for site in [p, q, small] {
        open(
            &mut b,
            site,
            UncertaintyKind::ComputedSpecifier,
            1,
            "require(x)",
        );
    }
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();
    let entries = tests(&g);

    // Freed: at first only the small site frees an entry. p frees nothing alone.
    let freed = e.floor(&entries, &opts, 25);
    assert_eq!(
        greedy(&g, &freed),
        [
            s("src/a-small.ts", 1, 3),
            s("src/p.ts", 0, 3),
            s("src/q.ts", 3, 0)
        ]
    );
    assert_eq!(e.floor_with(&entries, &opts, FloorOrder::Freed, 25), freed);
    // Weighted: p and q each hold half of three entries, more than the small site's
    // one entry. p and q tie: the smaller path goes first.
    let weighted = e.floor_with(&entries, &opts, FloorOrder::Weighted, 25);
    assert_eq!(
        greedy(&g, &weighted),
        [
            s("src/p.ts", 0, 4),
            s("src/q.ts", 3, 1),
            s("src/a-small.ts", 1, 0)
        ]
    );
    // Only the order differs.
    assert_eq!(
        (weighted.floor, &weighted.sites, &weighted.classes),
        (freed.floor, &freed.sites, &freed.classes)
    );
    let two = e.floor_with(&entries, &opts, FloorOrder::Weighted, 2);
    assert_eq!(two.greedy, weighted.greedy[..2]);

    // An entry that one site holds counts fully in both orders: on the hub graph
    // they agree.
    let g = hubs();
    let e = Engine::new(&g);
    let weighted = e.floor_with(&tests(&g), &opts, FloorOrder::Weighted, 25);
    assert_eq!(
        greedy(&g, &weighted),
        [
            s("src/hub.ts", 2, 2),
            s("src/cwd.ts", 1, 1),
            s("src/plugin.ts", 1, 0),
        ]
    );
}

fn assuming(g: &MemGraph, sites: &[&str], classes: &[&str]) -> FloorReport {
    let assume = FloorAssumptions {
        sites: sites.iter().map(|s| AssumedSite::parse(s)).collect(),
        classes: classes
            .iter()
            .map(|c| ClassPattern::parse(c).unwrap())
            .collect(),
        cut: Vec::new(),
    };
    Engine::new(g).floor_assuming(
        &tests(g),
        &AffectedOptions::default(),
        FloorOrder::Freed,
        25,
        &assume,
    )
}

#[test]
fn what_if_records_were_resolved() {
    let g = hubs();
    let base = Engine::new(&g).floor(&tests(&g), &AffectedOptions::default(), 25);
    assert_eq!((base.floor, base.assumed.as_ref()), (4, None));

    // The whole hub file: t1 and t2 leave the floor, t3 still reaches the plugin.
    let r = assuming(&g, &["src/hub.ts"], &[]);
    let a = r.assumed.clone().unwrap();
    assert_eq!((r.floor, r.records.open), (2, 3));
    assert_eq!((a.records, a.files, a.floor_before, a.freed), (2, 1, 4, 2));
    assert!(a.unmatched.is_empty());
    assert_eq!(
        sites(&g, &r),
        [
            s("src/cwd.ts", 1, 1),
            s("src/plugin.ts", 1, 1),
            s("src/orphan.ts", 0, 0)
        ]
    );
    assert_eq!(
        greedy(&g, &r),
        [s("src/cwd.ts", 1, 1), s("src/plugin.ts", 1, 0)]
    );
    // The other counts don't move.
    assert_eq!(
        (r.records.scoped, r.records.probed, r.records.ignored),
        (
            base.records.scoped,
            base.records.probed,
            base.records.ignored
        )
    );
    let mut text = Vec::new();
    r.write_text(&g, &mut text).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(
        text.starts_with(
            "uncertainty floor: 2 of 5 entries (40.0%) reach open uncertainty and are selected on \
             every change\nwhat if: 2 open record(s) in 1 file(s) assumed resolved; without that \
             the floor is 4 (2 more entries)\n"
        ),
        "{text}"
    );
    assert_eq!(
        r.to_value(&g)["assumed"],
        serde_json::json!({
            "records": 2, "files": 1, "cut": 0, "unmatched": [], "floor_before": 4, "freed": 2,
        })
    );

    // One line of it: the other record keeps the file a site.
    let r = assuming(&g, &["src/hub.ts:3"], &[]);
    let a = r.assumed.clone().unwrap();
    assert_eq!((r.floor, a.records, a.files, a.freed), (4, 1, 1, 0));
    assert_eq!(r.sites[0].records.len(), 1);
    assert_eq!(r.sites[0].records[0].line, 9);

    // A class, with and without its effect and origin.
    let r = assuming(&g, &[], &["computed_specifier/read"]);
    assert_eq!((r.floor, r.assumed.as_ref().unwrap().records), (2, 2));
    let r = assuming(&g, &[], &["computed_specifier"]);
    let a = r.assumed.clone().unwrap();
    assert_eq!((r.floor, a.records, a.files, a.freed), (1, 4, 3, 3));
    let r = assuming(&g, &["src/cwd.ts"], &["*/*/data", "*/load"]);
    let a = r.assumed.clone().unwrap();
    assert_eq!((r.floor, a.records, a.files, a.freed), (0, 5, 4, 4));
    assert!(r.sites.is_empty() && r.classes.is_empty() && r.greedy.is_empty());

    // Assumptions that match no open record are reported: a missing file, a file
    // whose records aren't open, a line without a record, and a class nothing
    // has.
    let r = assuming(
        &g,
        &[
            "src/nope.ts",
            "src/quiet.ts",
            "src/hub.ts:77",
            "src/plugin.ts",
        ],
        &["aliased_loader"],
    );
    let a = r.assumed.clone().unwrap();
    assert_eq!(
        a.unmatched,
        [
            "src/nope.ts",
            "src/quiet.ts",
            "src/hub.ts:77",
            "aliased_loader/*/*"
        ]
    );
    assert_eq!((r.floor, a.records, a.freed), (4, 1, 0));
    let mut text = Vec::new();
    r.write_text(&g, &mut text).unwrap();
    assert!(
        String::from_utf8(text)
            .unwrap()
            .contains("warning: no open record matches the assumption src/hub.ts:77\n")
    );
}

#[test]
fn what_if_a_hub_were_cut() {
    // t1, t2 → jest.config.ts → task:.#build → s1, s2;  t1 → own
    let mut b = MemGraph::builder();
    let t1 = b.file("test/t1.test.ts");
    let t2 = b.file("test/t2.test.ts");
    let config = b.file("jest.config.ts");
    let hub = b.file("task:.#build");
    let s1 = b.file("src/s1.ts");
    let s2 = b.file("src/s2.ts");
    let own = b.file("src/own.ts");
    for (from, to) in [
        (t1, config),
        (t2, config),
        (config, hub),
        (hub, s1),
        (hub, s2),
        (t1, own),
    ] {
        b.edge(from, to, RefKind::Import);
    }
    for site in [s1, s2, own] {
        open(
            &mut b,
            site,
            UncertaintyKind::ComputedSpecifier,
            1,
            "require(x)",
        );
    }
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();
    let cutting = |cut: &[&str]| {
        let assume = FloorAssumptions {
            cut: cut.iter().map(|s| (*s).to_owned()).collect(),
            ..FloorAssumptions::default()
        };
        e.floor_assuming(&tests(&g), &opts, FloorOrder::Freed, 25, &assume)
    };
    assert_eq!(e.floor(&tests(&g), &opts, 25).floor, 2);

    // Without the hub, t2 reaches no site, and t1 only its own.
    let r = cutting(&["task:.#build"]);
    let a = r.assumed.clone().unwrap();
    assert_eq!(r.floor, 1);
    assert_eq!(
        (a.records, a.files, a.cut, a.floor_before, a.freed),
        (0, 0, 1, 2, 1)
    );
    assert!(a.unmatched.is_empty());
    assert_eq!(
        sites(&g, &r),
        [
            s("src/own.ts", 1, 1),
            s("src/s1.ts", 0, 0),
            s("src/s2.ts", 0, 0)
        ]
    );
    assert_eq!(r.records.open, 3, "the records are still open");
    assert_eq!(greedy(&g, &r), [s("src/own.ts", 1, 0)]);
    let mut text = Vec::new();
    r.write_text(&g, &mut text).unwrap();
    assert!(String::from_utf8(text).unwrap().contains(
        "what if: 0 open record(s) in 0 file(s) assumed resolved and 1 node(s) not traversed; \
         without that the floor is 2 (1 more entries)\n"
    ));
    // The node before the hub works as well; a site can be cut too.
    assert_eq!(cutting(&["jest.config.ts"]).floor, 1);
    assert_eq!(cutting(&["src/s1.ts"]).floor, 2);
    assert_eq!(cutting(&["src/s1.ts", "src/s2.ts"]).floor, 1);
    // An entry that is named is still evaluated; only references to it are cut.
    assert_eq!(cutting(&["test/t2.test.ts"]).floor, 2);
    // A node that isn't in the graph is reported.
    let r = cutting(&["task:nope"]);
    let a = r.assumed.unwrap();
    assert_eq!((r.floor, a.cut, a.freed), (2, 0, 0));
    assert_eq!(a.unmatched, ["task:nope"]);
    // Globs cut every node they match.
    let r = cutting(&["task:**", "nope/**"]);
    let a = r.assumed.unwrap();
    assert_eq!((r.floor, a.cut, a.freed), (1, 1, 1));
    assert_eq!(a.unmatched, ["nope/**"]);
    let r = cutting(&["src/s*.ts"]);
    assert_eq!((r.floor, r.assumed.unwrap().cut), (1, 2));

    // `prefix**` matches across `/`: every task, whatever its package.
    let mut b = MemGraph::builder();
    let t = b.file("test/t.test.ts");
    let task = b.file("task:apps/web#build");
    let site = b.file("src/site.ts");
    b.edge(t, task, RefKind::Import)
        .edge(task, site, RefKind::Import);
    open(
        &mut b,
        site,
        UncertaintyKind::ComputedSpecifier,
        1,
        "require(x)",
    );
    let g = b.build().unwrap();
    let assume = FloorAssumptions {
        cut: vec!["task:**".to_owned()],
        ..FloorAssumptions::default()
    };
    let r = Engine::new(&g).floor_assuming(&tests(&g), &opts, FloorOrder::Freed, 25, &assume);
    assert_eq!((r.floor, r.assumed.unwrap().cut), (0, 1));

    // `prefix**` is the prefix form even when a node has that very name: the
    // membership hub of a whole repository is `members:**`, and `members:**`
    // means every hub.
    let mut b = MemGraph::builder();
    let t1 = b.file("test/t1.test.ts");
    let t2 = b.file("test/t2.test.ts");
    let root_hub = b.file("members:**");
    let pkg_hub = b.file("members:packages/lib/**");
    let s1 = b.file("scripts/s1.ts");
    let s2 = b.file("packages/lib/s2.ts");
    for (from, to) in [(t1, root_hub), (root_hub, s1), (t2, pkg_hub), (pkg_hub, s2)] {
        b.edge(from, to, RefKind::Import);
    }
    for site in [s1, s2] {
        open(
            &mut b,
            site,
            UncertaintyKind::ComputedSpecifier,
            1,
            "require(x)",
        );
    }
    let g = b.build().unwrap();
    let assume = FloorAssumptions {
        cut: vec!["members:**".to_owned()],
        ..FloorAssumptions::default()
    };
    let r = Engine::new(&g).floor_assuming(&tests(&g), &opts, FloorOrder::Freed, 25, &assume);
    assert_eq!((r.floor, r.assumed.unwrap().cut), (0, 2));
}

/// Uncertainty does not propagate along data edges: a task that only reads a
/// hub's members (declared hash inputs, a type-check) is not floored by what a
/// member would load when run, and neither are the consumers of its outputs. A
/// change of the member still reaches them, and a task that runs the file (a
/// plain edge) keeps its uncertainty.
#[test]
fn data_edges_carry_changes_not_uncertainty() {
    let mut b = MemGraph::builder();
    let reader = b.file("test/reader.test.ts");
    let runner = b.file("test/runner.test.ts");
    let check = b.file("task:packages/lib#type-check");
    let build = b.file("task:packages/lib#build");
    let hub = b.file("members:packages/lib/**");
    let script = b.file("packages/lib/scripts/other.ts");
    b.edge(reader, check, RefKind::Import);
    b.edge(runner, build, RefKind::Import);
    b.add_edge(
        check,
        EdgeSpec::new(Some(hub), RefKind::FsRead).flags(RefFlags::DATA),
    );
    b.edge(hub, script, RefKind::FsRead);
    // The build runs the script.
    b.edge(build, script, RefKind::Import);
    open(
        &mut b,
        script,
        UncertaintyKind::ComputedSpecifier,
        1,
        "readFileSync(p)",
    );
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();

    let r = e.floor(&tests(&g), &opts, 25);
    assert_eq!((r.floor, r.sites[0].reached), (1, 1));
    assert_eq!(r.floor, selected_without_a_change(&g, &e, &opts));
    let selected = |changes: &ChangeSet| -> Vec<(String, bool)> {
        e.affected(changes, &tests(&g), &opts)
            .entries
            .iter()
            .filter(|x| x.verdict == Verdict::Affected)
            .map(|x| (g.path(x.entry).to_owned(), x.uncertain.is_some()))
            .collect()
    };
    assert_eq!(
        selected(&ChangeSet::new()),
        [("test/runner.test.ts".to_owned(), true)]
    );

    // An edit of the member selects both: the type-check reads it.
    let mut changes = ChangeSet::new();
    changes.push(
        turborepo_trace_graph::ChangeKind::Modified,
        "packages/lib/scripts/other.ts",
    );
    assert_eq!(
        selected(&changes),
        [
            ("test/reader.test.ts".to_owned(), false),
            ("test/runner.test.ts".to_owned(), false)
        ]
    );

    // The flag has a name in outputs and in the query language.
    assert_eq!(RefFlags::DATA.names().collect::<Vec<_>>(), ["data"]);
    assert_eq!(RefFlags::parse("data"), Some(RefFlags::DATA));
}

#[test]
fn nothing_open() {
    let mut b = MemGraph::builder();
    let t = b.file("test/t.test.ts");
    let a = b.file("src/a.ts");
    b.edge(t, a, RefKind::Import);
    open(&mut b, a, UncertaintyKind::EvalLike, 1, "eval(x)");
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let opts = AffectedOptions::default();
    let r = e.floor(&tests(&g), &opts, 25);
    assert_eq!((r.entries, r.floor, r.records.ignored), (1, 0, 1));
    assert!(r.classes.is_empty() && r.sites.is_empty() && r.greedy.is_empty());
    let mut text = Vec::new();
    r.write_text(&g, &mut text).unwrap();
    assert_eq!(
        String::from_utf8(text).unwrap(),
        "uncertainty floor: 0 of 1 entries (0.0%) reach open uncertainty and are selected on \
         every change\nrecords (first-party): 0 open, 0 scoped, 0 probed, 1 ignored by \
         default\nopen sites: 0 file(s), 0 reached by entries\n"
    );
    // No entries at all.
    let r = e.floor(&NodeSet::new(g.node_count()), &opts.strict(true), 25);
    assert_eq!((r.entries, r.floor, r.sites.len()), (0, 0, 1));
    assert_eq!(r.sites[0].reached, 0);
}

#[test]
fn text_and_json() {
    let g = hubs();
    let e = Engine::new(&g);
    let r = e.floor(&tests(&g), &AffectedOptions::default(), 2);
    let mut text = Vec::new();
    r.write_text(&g, &mut text).unwrap();
    let text = String::from_utf8(text).unwrap();
    let expected = "\
uncertainty floor: 4 of 5 entries (80.0%) reach open uncertainty and are selected on every change
records (first-party): 5 open, 1 scoped, 1 probed, 2 ignored by default
open sites: 4 file(s), 3 reached by entries

open records by class (reached: entries that reach one; exclusive: entries with no open record of \
                    another class)
 records    files  reached  exclusive  kind/effect/origin
       2        1        3          2  computed_specifier/read/data
       1        1        1          1  cwd_dependent/read/cwd
       2        2        1          0  computed_specifier/load/unknown

top sites, in the order to resolve them (freed: entries that leave the floor with this site; left: \
                    the floor after it)
   #    freed     left  reached  site
   1        2        2        3  src/hub.ts
                                   line 3 computed_specifier/read/data: readFileSync(row.path)
                                   line 9 computed_specifier/read/data: readFileSync(other.path)
   2        1        1        1  src/cwd.ts
                                   line 4 cwd_dependent/read/cwd: readFileSync('data.json')
1 more reached site(s) not shown (--top N; --format json lists every site)
";
    assert_eq!(text, expected, "\n{text}");

    let v = r.to_value(&g);
    assert_eq!(v["version"], 1);
    assert_eq!(v["entries"], 5);
    assert_eq!(v["floor"], 4);
    assert_eq!(
        v["records"],
        serde_json::json!({"open": 5, "scoped": 1, "probed": 1, "ignored": 2, "membership": 1})
    );
    assert_eq!(v["open_sites"], 4);
    assert_eq!(v["open_sites_reached"], 3);
    assert_eq!(
        v["classes"][0],
        serde_json::json!({
            "kind": "computed_specifier", "effect": "read", "origin": "data",
            "records": 2, "files": 1, "files_reached": 1, "reached": 3, "exclusive": 2,
        })
    );
    // Every site, whatever the number of picks.
    assert_eq!(v["sites"].as_array().unwrap().len(), 4);
    assert_eq!(
        v["sites"][0],
        serde_json::json!({
            "path": "src/hub.ts", "reached": 3, "exclusive": 2,
            "records": [
                {"line": 3, "kind": "computed_specifier", "effect": "read", "origin": "data",
                 "detail": "readFileSync(row.path)"},
                {"line": 9, "kind": "computed_specifier", "effect": "read", "origin": "data",
                 "detail": "readFileSync(other.path)"},
            ],
        })
    );
    assert_eq!(v["sites"][3]["path"], "src/orphan.ts");
    assert_eq!(v["sites"][3]["reached"], 0);
    assert_eq!(
        v["greedy"],
        serde_json::json!([
            {"path": "src/hub.ts", "freed": 2, "remaining": 2, "reached": 3},
            {"path": "src/cwd.ts", "freed": 1, "remaining": 1, "reached": 1},
        ])
    );
}
