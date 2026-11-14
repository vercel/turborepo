//! Bounded loads (`turborepo_trace_graph::loads`): a module load with an
//! unknown target selects the entries that reach it only when the change
//! affects a package the load can reach.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_graph::{
    AffectedOptions, ChangeKind, ChangeSet, Disposition, Effect, Engine, GraphView, LoadReach,
    MemGraph, NodeSet, Origin, PackageId, Policy, RefFlags, RefKind, UncertaintyKind, Verdict,
};

/// The shape of a service repository whose test harness boots a framework that
/// loads handler modules by a string (names changed):
///
/// ```text
/// packages/framework   normalize.ts: import(spec), spec a parameter      declares util
/// packages/util        index.ts (imported by normalize.ts), other.ts, loader.ts: require(x)
///                                                                       declares deep
/// packages/deep        d.ts
/// packages/harness     setup.ts imports the framework                    declares framework
/// packages/shared      s.ts                                              (a declares it)
/// packages/leaf        index.ts                                          (nobody declares it)
/// services/a           a.test.ts -> setup.ts, routes.ts -> handler.ts   declares harness, shared
/// services/a/tools     gen.ts (a package nested in services/a)
/// services/b           b.test.ts -> setup.ts; handler.ts (no edge)      declares harness
/// packages/far         f.ts                                              (shared declares it)
/// services/c           c.test.ts -> setup.ts by a relative path         declares nothing
/// ```
struct Fixture {
    g: MemGraph,
    reach: LoadReach,
    b: PackageId,
    third_party: PackageId,
}

fn fixture() -> Fixture {
    let mut gb = MemGraph::builder();
    let root = gb.package(".", None, "");
    let pkg = |gb: &mut turborepo_trace_graph::GraphBuilder, name: &str, dir: &str| {
        gb.package(name, None, dir)
    };
    let framework = pkg(&mut gb, "@x/framework", "packages/framework");
    let util = pkg(&mut gb, "@x/util", "packages/util");
    let deep = pkg(&mut gb, "@x/deep", "packages/deep");
    let harness = pkg(&mut gb, "@x/harness", "packages/harness");
    let shared = pkg(&mut gb, "@x/shared", "packages/shared");
    let leaf = pkg(&mut gb, "@x/leaf", "packages/leaf");
    let a = pkg(&mut gb, "svc-a", "services/a");
    let tools = pkg(&mut gb, "svc-a-tools", "services/a/tools");
    let b = pkg(&mut gb, "svc-b", "services/b");
    let far = pkg(&mut gb, "@x/far", "packages/far");
    let c = pkg(&mut gb, "svc-c", "services/c");
    let third_party = gb.package("lodash", Some("4.17.21"), "node_modules/lodash");
    let file = |gb: &mut turborepo_trace_graph::GraphBuilder, path: &str, p: PackageId| {
        gb.add_file(path, p, Some("js"), false)
    };
    let normalize = file(&mut gb, "packages/framework/normalize.ts", framework);
    let util_index = file(&mut gb, "packages/util/index.ts", util);
    file(&mut gb, "packages/util/other.ts", util);
    let loader = file(&mut gb, "packages/util/loader.ts", util);
    file(&mut gb, "packages/deep/d.ts", deep);
    let setup = file(&mut gb, "packages/harness/setup.ts", harness);
    file(&mut gb, "packages/shared/s.ts", shared);
    file(&mut gb, "packages/leaf/index.ts", leaf);
    let a_test = file(&mut gb, "services/a/a.test.ts", a);
    let routes = file(&mut gb, "services/a/routes.ts", a);
    let handler = file(&mut gb, "services/a/handler.ts", a);
    file(&mut gb, "services/a/tools/gen.ts", tools);
    let b_test = file(&mut gb, "services/b/b.test.ts", b);
    file(&mut gb, "services/b/handler.ts", b);
    file(&mut gb, "README.md", root);
    file(&mut gb, "packages/far/f.ts", far);
    let c_test = file(&mut gb, "services/c/c.test.ts", c);
    gb.edge(c_test, setup, RefKind::Import);
    gb.edge(normalize, util_index, RefKind::Import);
    gb.edge(setup, normalize, RefKind::Import);
    gb.edge(a_test, setup, RefKind::Import);
    gb.edge(a_test, routes, RefKind::Import);
    gb.add_edge(
        routes,
        turborepo_trace_graph::EdgeSpec::new(Some(handler), RefKind::DynamicImport)
            .flags(RefFlags::LAZY),
    );
    gb.edge(b_test, setup, RefKind::Import);
    gb.uncertainty(
        normalize,
        UncertaintyKind::ComputedSpecifier,
        42,
        "spec; parameter normalize#0",
    )
    .uncertainty_class(Effect::Load, Origin::Param);
    gb.uncertainty(loader, UncertaintyKind::ComputedSpecifier, 3, "x")
        .uncertainty_class(Effect::Load, Origin::Unknown);
    let g = gb.build().unwrap();
    let mut reach = LoadReach::new(g.package_count());
    for (from, to) in [
        (framework, util),
        (util, deep),
        (harness, framework),
        (a, harness),
        (a, shared),
        (a, tools),
        (tools, a),
        (b, harness),
        (shared, far),
    ] {
        reach.add(from, to);
    }
    reach.finish();
    Fixture {
        g,
        reach,
        b,
        third_party,
    }
}

/// The bound with the resolution step (what one load reaches, chains followed).
fn opts() -> AffectedOptions {
    AffectedOptions::default()
        .loads_declared(true)
        .loads_step(true)
}

/// The declared closure alone: what the entry's package depends on, however far
/// away.
fn rule() -> AffectedOptions {
    AffectedOptions::default()
        .loads_declared(true)
        .loads_step(false)
}

fn tests(g: &MemGraph) -> NodeSet {
    Engine::new(g).query("**/*.test.ts").unwrap().nodes
}

/// What the engine selects, then what the bounded loads add: `(path, reason)`.
fn select(f: &Fixture, cs: &ChangeSet, opts: &AffectedOptions) -> (Vec<String>, Vec<String>) {
    let g = &f.g;
    let e = Engine::new(g);
    let entries = tests(g);
    let report = e.affected(cs, &entries, opts);
    let mut selected = NodeSet::new(g.node_count());
    let mut first: Vec<String> = Vec::new();
    for r in &report.entries {
        selected.insert(r.entry);
        first.push(g.path(r.entry).to_owned());
    }
    let (found, _) = e.bounded_loads(
        &f.reach,
        cs,
        &cs.files,
        &[],
        &entries,
        None,
        &selected,
        opts,
    );
    let added = found
        .iter()
        .map(|r| {
            assert_eq!(r.verdict, Verdict::Affected);
            assert!(r.cause.is_none());
            let u = r
                .uncertain
                .as_ref()
                .expect("the load is the uncertain site");
            assert_eq!(r.witness[0], u.node);
            assert_eq!(*r.witness.last().unwrap(), r.entry);
            format!("{}: {}", g.path(r.entry), u.bound.as_deref().unwrap())
        })
        .collect();
    (first, added)
}

fn changed(path: &str) -> ChangeSet {
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, path);
    cs
}

fn paths(v: &[String]) -> Vec<&str> {
    v.iter().map(|s| s.split(": ").next().unwrap()).collect()
}

#[test]
fn a_load_is_bounded_only_when_the_option_is_on() {
    let f = fixture();
    let g = &f.g;
    let e = Engine::new(g);
    let site = g.node_by_path("packages/framework/normalize.ts").unwrap();
    let d = |o: &AffectedOptions| {
        let u = g.uncertainties(site).next().unwrap();
        e.disposition_of(site, &u, o)
    };
    assert_eq!(d(&AffectedOptions::default()), Disposition::Open);
    assert_eq!(d(&opts()), Disposition::Declared);
    assert_eq!(d(&opts().strict(true)), Disposition::Open);
    assert_eq!(e.declared_sites(&opts()).len(), 2);
    assert!(e.declared_sites(&AffectedOptions::default()).is_empty());
}

#[test]
fn an_unrelated_change_selects_nothing() {
    let f = fixture();
    // A package nobody declares, the root package, a path that is no node.
    for path in ["packages/leaf/index.ts", "README.md", "docs/new.md"] {
        let (first, added) = select(&f, &changed(path), &opts());
        assert!(first.is_empty() && added.is_empty(), "{path}: {added:?}");
    }
    // Without the bound both tests are on the floor for the same change.
    let (first, _) = select(
        &f,
        &changed("packages/leaf/index.ts"),
        &AffectedOptions::default(),
    );
    assert_eq!(
        first,
        [
            "services/a/a.test.ts",
            "services/b/b.test.ts",
            "services/c/c.test.ts"
        ]
    );
    // `--strict` assumes nothing.
    let (first, added) = select(&f, &changed("packages/leaf/index.ts"), &opts().strict(true));
    assert_eq!(
        first,
        [
            "services/a/a.test.ts",
            "services/b/b.test.ts",
            "services/c/c.test.ts"
        ]
    );
    assert!(added.is_empty());
}

#[test]
fn a_change_in_a_package_the_load_declares_fires_it() {
    let f = fixture();
    // `other.ts` is a file nothing imports, of a package the framework declares.
    let (first, added) = select(&f, &changed("packages/util/other.ts"), &opts());
    assert!(first.is_empty());
    assert_eq!(
        paths(&added),
        ["services/a/a.test.ts", "services/b/b.test.ts"]
    );
    assert!(
        added[0].contains("the load can reach packages/util/ (packages/util/other.ts changed)"),
        "{}",
        added[0]
    );
    // The package of the loading file itself.
    let (_, added) = select(&f, &changed("packages/framework/new.ts"), &opts());
    assert_eq!(added.len(), 2);
}

#[test]
fn the_entry_part_covers_the_package_of_the_test_and_what_it_declares() {
    let f = fixture();
    // `services/a` declares `shared`; the framework does not.
    let (first, added) = select(&f, &changed("packages/shared/s.ts"), &opts());
    assert!(first.is_empty());
    assert_eq!(paths(&added), ["services/a/a.test.ts"]);
    assert!(
        added[0].contains("a load of this entry's package can reach packages/shared/"),
        "{}",
        added[0]
    );
    // A file of the test's own package that no edge reaches.
    let (_, added) = select(&f, &changed("services/b/handler.ts"), &opts());
    assert_eq!(paths(&added), ["services/b/b.test.ts"]);
    // A package nested in the test's package.
    let (_, added) = select(&f, &changed("services/a/tools/gen.ts"), &opts());
    assert_eq!(paths(&added), ["services/a/a.test.ts"]);
}

#[test]
fn a_change_reached_through_edges_stays_a_change() {
    let f = fixture();
    let (first, added) = select(&f, &changed("services/a/handler.ts"), &opts());
    assert_eq!(first, ["services/a/a.test.ts"]);
    // `services/b` does not declare `services/a`: nothing more.
    assert!(added.is_empty(), "{added:?}");
}

#[test]
fn a_fired_load_makes_the_files_that_reach_it_affected() {
    let f = fixture();
    // `deep` is declared by `util` only. The load in util/loader.ts can reach it,
    // so loader.ts is affected, and with it `util`, which the framework's load
    // can reach.
    let (first, added) = select(&f, &changed("packages/deep/d.ts"), &opts());
    assert!(first.is_empty());
    assert_eq!(
        paths(&added),
        ["services/a/a.test.ts", "services/b/b.test.ts"]
    );
    assert!(
        added[0].contains(
            "packages/util/ (packages/util/loader.ts reaches the load in packages/util/loader.ts, \
             which the change can reach)"
        ),
        "{}",
        added[0]
    );
}

#[test]
fn a_changed_third_party_package_affects_its_importer() {
    let f = fixture();
    let mut cs = ChangeSet::new();
    cs.push_scoped(f.third_party, vec![f.b]);
    let (_, added) = select(&f, &cs, &opts());
    assert_eq!(paths(&added), ["services/b/b.test.ts"]);
    assert!(added[0].contains("a third-party package it installs changed"));
}

#[test]
fn the_rule_alone_is_what_the_entry_package_depends_on() {
    let f = fixture();
    // Declared three steps away from services/a and services/b (harness, framework,
    // util).
    let (first, added) = select(&f, &changed("packages/util/other.ts"), &rule());
    assert!(first.is_empty());
    assert_eq!(
        paths(&added),
        ["services/a/a.test.ts", "services/b/b.test.ts"]
    );
    assert!(
        added[0].contains(
            "this entry's package depends on packages/util/ (packages/util/other.ts changed)"
        ),
        "{}",
        added[0]
    );
    // `far` is declared by `shared`, which only services/a declares: inside the
    // closure of a, and no single resolution step from a or from the framework
    // reaches it.
    let (_, added) = select(&f, &changed("packages/far/f.ts"), &rule());
    assert_eq!(paths(&added), ["services/a/a.test.ts"]);
    let (_, added) = select(&f, &changed("packages/far/f.ts"), &opts());
    assert!(added.is_empty(), "{added:?}");
    // Nobody depends on `leaf`; the test's own package and what is nested in it
    // count.
    let (_, added) = select(&f, &changed("packages/leaf/index.ts"), &rule());
    assert!(added.is_empty());
    let (_, added) = select(&f, &changed("services/b/handler.ts"), &rule());
    assert_eq!(paths(&added), ["services/b/b.test.ts"]);
    let (_, added) = select(&f, &changed("services/a/tools/gen.ts"), &rule());
    assert_eq!(paths(&added), ["services/a/a.test.ts"]);
}

/// services/c reaches the harness by a relative path and declares nothing: what
/// the helper's package declares does not widen what c's tests may load.
#[test]
fn a_helper_does_not_widen_the_bound_of_an_entry() {
    let f = fixture();
    for o in [rule(), opts()] {
        let (_, added) = select(&f, &changed("packages/util/other.ts"), &o);
        assert_eq!(
            paths(&added),
            ["services/a/a.test.ts", "services/b/b.test.ts"],
            "step: {}",
            o.loads_step
        );
        // A change the helper file itself imports is an edge, whoever declares what.
        let (first, _) = select(&f, &changed("packages/util/index.ts"), &o);
        assert_eq!(first.len(), 3);
    }
}

#[test]
fn the_policy_decides_the_verdict() {
    let f = fixture();
    let g = &f.g;
    let e = Engine::new(g);
    let cs = changed("packages/util/other.ts");
    let none = NodeSet::new(g.node_count());
    let run = |p: Policy| {
        e.bounded_loads(
            &f.reach,
            &cs,
            &cs.files,
            &[],
            &tests(g),
            None,
            &none,
            &opts().policy(p),
        )
        .0
    };
    assert!(run(Policy::Ignore).is_empty());
    assert!(
        run(Policy::Unknown)
            .iter()
            .all(|r| r.verdict == Verdict::Unknown)
    );
    assert_eq!(run(Policy::Unknown).len(), 2);
}

#[test]
fn an_entry_that_something_else_selected_is_left_alone() {
    let f = fixture();
    let g = &f.g;
    let e = Engine::new(g);
    let cs = changed("packages/util/other.ts");
    let mut selected = NodeSet::new(g.node_count());
    selected.insert(g.node_by_path("services/a/a.test.ts").unwrap());
    let (found, stats) = e.bounded_loads(
        &f.reach,
        &cs,
        &cs.files,
        &[],
        &tests(g),
        None,
        &selected,
        &opts(),
    );
    let got: Vec<&str> = found.iter().map(|r| g.path(r.entry)).collect();
    assert_eq!(got, ["services/b/b.test.ts"]);
    assert_eq!((stats.sites, stats.candidates, stats.by_site), (2, 2, 1));
}

#[test]
fn the_report_says_for_which_packages_a_load_selects() {
    let f = fixture();
    let g = &f.g;
    let e = Engine::new(g);
    let r = e.bounded_sites(&f.reach, &tests(g), None, &opts());
    assert_eq!((r.records, r.entries, r.floor_without), (2, 3, 3));
    assert!(r.walked);
    // Without a walk: the records by path, no counts.
    let light = e.bounded_records(&opts());
    assert_eq!(
        (light.records, light.sites.len(), light.walked),
        (2, 2, false)
    );
    // util/loader.ts is reached by no test.
    assert_eq!(r.sites.len(), 1);
    let s = &r.sites[0];
    assert_eq!(g.path(s.node), "packages/framework/normalize.ts");
    assert_eq!(s.reached, 3);
    // a, b and c hold the entries; together they depend on everything but `leaf`,
    // the root package and the third-party package.
    assert_eq!((s.entry_packages, s.covers, r.packages), (3, 10, 12));
    assert!(!s.root_entries);
    let roots: Vec<&str> = s.reach.iter().map(|&p| g.package_info(p).root).collect();
    assert_eq!(roots, ["packages/framework", "packages/util"]);
    assert_eq!(s.records[0].0, 42);
}

/// The lint finding: a certain edge into a package the file's package cannot
/// reach.
#[test]
fn certain_edges_outside_the_declared_packages_are_findings_not_cuts() {
    let f = fixture();
    let g = &f.g;
    let e = Engine::new(g);
    let r = e.undeclared_dependencies(&f.reach);
    // services/c imports the harness by a path and declares nothing.
    let got: Vec<(&str, &str, bool)> = r
        .findings
        .iter()
        .map(|d| (g.path(d.from), g.path(d.to), d.transitive))
        .collect();
    assert_eq!(
        got,
        [("services/c/c.test.ts", "packages/harness/setup.ts", false)]
    );
    assert!(!r.findings[0].read && !r.findings[0].type_only);
    // The declared edges were looked at and are fine.
    assert!(r.checked >= 4, "{}", r.checked);
    assert_eq!((r.into_root, r.runner), (0, 0));
    // The edge itself is never cut: a change of the harness selects c's test.
    let (first, _) = select(&f, &changed("packages/harness/setup.ts"), &rule());
    assert!(first.contains(&"services/c/c.test.ts".to_owned()));
}

#[test]
fn findings_say_how_the_edge_is_written_and_what_is_left_out() {
    use turborepo_trace_graph::{EdgeSpec, Written};
    let mut gb = MemGraph::builder();
    let root = gb.package(".", None, "");
    let a = gb.package("a", None, "packages/a");
    let b = gb.package("b", None, "packages/b");
    let c = gb.package("c", None, "packages/c");
    let file = |gb: &mut turborepo_trace_graph::GraphBuilder, path: &str, p: PackageId| {
        gb.add_file(path, p, Some("js"), false)
    };
    let user = file(&mut gb, "packages/a/user.ts", a);
    let b_index = file(&mut gb, "packages/b/index.ts", b);
    let b_data = gb.add_file("packages/b/data.json", b, None, false);
    let c_index = file(&mut gb, "packages/c/index.ts", c);
    let cfg = file(&mut gb, "jest.config.js", root);
    let edge =
        |to, kind, spec: &str, line| EdgeSpec::new(Some(to), kind).specifier(spec).line(line);
    gb.add_edge(user, edge(b_index, RefKind::Import, "../b/index", 1));
    gb.add_edge(
        user,
        edge(b_index, RefKind::Import, "b/index", 2).flags(RefFlags::TYPE_ONLY),
    );
    gb.add_edge(user, edge(b_data, RefKind::FsRead, "", 3));
    gb.add_edge(
        user,
        edge(
            b_index,
            RefKind::Require,
            "jest:moduleNameMapper ~b -> packages/b/index",
            4,
        ),
    );
    gb.add_edge(user, edge(c_index, RefKind::Import, "c", 5));
    gb.add_edge(user, edge(cfg, RefKind::Require, "jest:config", 0));
    gb.add_edge(user, edge(cfg, RefKind::FsRead, "../../jest.config.js", 6));
    let g = gb.build().unwrap();
    // a declares c, and c declares b.
    let mut reach = LoadReach::new(g.package_count());
    reach.add(a, c);
    reach.add(c, b);
    reach.finish();
    let r = Engine::new(&g).undeclared_dependencies(&reach);
    let got: Vec<(u32, Written, bool, bool, bool)> = r
        .findings
        .iter()
        .map(|d| (d.line, d.written, d.read, d.type_only, d.transitive))
        .collect();
    assert_eq!(
        got,
        [
            (1, Written::RelativePath, false, false, true),
            (2, Written::PackageName, false, true, true),
            (3, Written::Path, true, false, true),
            (4, Written::Mapped, false, false, true),
        ]
    );
    // The runner's config edge and the read of a root file are counted, not
    // reported.
    assert_eq!((r.runner, r.into_root), (1, 1));
}
