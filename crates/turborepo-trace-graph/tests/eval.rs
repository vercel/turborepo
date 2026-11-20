//! Evaluator tests on hand-built graphs.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::graph;
use turborepo_trace_graph::{
    AffectedOptions, Cause, Change, ChangeKind, ChangeSet, EdgeFilter, Engine, EngineOptions,
    GraphView, MemGraph, NodeId, NodeSet, Policy, RefFlags, RefKind, UncertaintyKind, Verdict,
};

fn paths(g: &MemGraph, s: &NodeSet) -> Vec<String> {
    let mut v: Vec<String> = s.iter().map(|n| g.path(n).to_owned()).collect();
    v.sort();
    v
}

fn q(g: &MemGraph, src: &str) -> Vec<String> {
    let e = Engine::new(g);
    let r = e
        .query(src)
        .unwrap_or_else(|err| panic!("{}", err.render(src)));
    paths(g, &r.nodes)
}

fn qerr(g: &MemGraph, src: &str) -> String {
    Engine::new(g).query(src).expect_err(src).render(src)
}

fn n(g: &MemGraph, p: &str) -> NodeId {
    g.node_by_path(p).unwrap()
}

#[test]
fn literals() {
    let g = graph();
    assert_eq!(q(&g, "src/a.ts"), ["src/a.ts"]);
    assert_eq!(q(&g, "./src/a.ts"), ["src/a.ts"]);
    assert_eq!(q(&g, "test/*.ts").len(), 4);
    assert_eq!(q(&g, "d/{1,4}.ts"), ["d/1.ts", "d/4.ts"]);
    assert_eq!(q(&g, "\"d/[23].ts\""), ["d/2.ts", "d/3.ts"]);
    assert_eq!(q(&g, "pkg:lodash"), ["node_modules/lodash/index.js"]);
    assert_eq!(q(&g, "pkg:lodash@4.17.21").len(), 1);
    assert_eq!(q(&g, "pkg:lod*").len(), 1);
    assert_eq!(q(&g, "nothing/**"), Vec::<String>::new());
    assert_eq!(q(&g, "**").len(), g.node_count() as usize);
}

#[test]
fn deps_and_rdeps() {
    let g = graph();
    assert_eq!(
        q(&g, "deps(test/a.test.ts)"),
        [
            "node_modules/lodash/index.js",
            "src/a.ts",
            "src/shared.ts",
            "src/types.ts",
            "src/util.ts",
            "test/a.test.ts"
        ]
    );
    assert_eq!(q(&g, "deps(test/a.test.ts, 0)"), ["test/a.test.ts"]);
    assert_eq!(
        q(&g, "deps(test/a.test.ts, 1)"),
        ["src/a.ts", "test/a.test.ts"]
    );
    assert_eq!(
        q(&g, "rdeps(src/util.ts)"),
        [
            "src/a.ts",
            "src/b.ts",
            "src/shared.ts",
            "src/util.ts",
            "test/a.test.ts",
            "test/b.test.ts"
        ]
    );
    assert_eq!(
        q(&g, "rdeps(src/util.ts, 2)"),
        ["src/a.ts", "src/b.ts", "src/shared.ts", "src/util.ts"]
    );
    // Unresolved edges are kept but not traversed.
    assert_eq!(
        q(&g, "deps(test/c.test.ts)"),
        ["data/config.json", "src/c.ts", "test/c.test.ts"]
    );
    assert_eq!(
        g.out_edges(n(&g, "src/c.ts"))
            .filter(|e| e.to.is_none())
            .map(|e| (e.specifier, e.line))
            .collect::<Vec<_>>(),
        [(Some("./missing"), 7)]
    );
}

#[test]
fn cycles() {
    let g = graph();
    assert_eq!(q(&g, "deps(src/x.ts)"), ["src/x.ts", "src/y.ts"]);
    assert_eq!(
        q(&g, "rdeps(src/y.ts)"),
        ["src/b.ts", "src/x.ts", "src/y.ts", "test/b.test.ts"]
    );
    assert_eq!(
        q(&g, "somepath(src/y.ts, src/x.ts)"),
        ["src/x.ts", "src/y.ts"]
    );
    assert_eq!(
        q(&g, "allpaths(src/x.ts, src/x.ts)"),
        ["src/x.ts", "src/y.ts"]
    );
}

#[test]
fn diamond_paths() {
    let g = graph();
    assert_eq!(
        q(&g, "allpaths(d/1.ts, d/4.ts)"),
        ["d/1.ts", "d/2.ts", "d/3.ts", "d/4.ts"]
    );
    assert_eq!(q(&g, "allpaths(d/2.ts, d/3.ts)"), Vec::<String>::new());
    let e = Engine::new(&g);
    let r = e.query("somepath(d/1.ts, d/4.ts)").unwrap();
    let p: Vec<_> = r.path.unwrap().iter().map(|&x| g.path(x)).collect();
    assert_eq!(p.len(), 3);
    assert_eq!((p[0], p[2]), ("d/1.ts", "d/4.ts"));
    assert_eq!(q(&g, "somepath(d/4.ts, d/1.ts)"), Vec::<String>::new());
    assert_eq!(q(&g, "somepath(d/1.ts, d/1.ts)"), ["d/1.ts"]);
}

#[test]
fn filters() {
    let g = graph();
    assert_eq!(
        q(&g, "kind(fs_read, deps(test/c.test.ts))"),
        ["data/config.json"]
    );
    assert_eq!(
        q(&g, "kind(require, deps(test/b.test.ts))"),
        ["node_modules/lodash/index.js", "src/shared.ts"]
    );
    assert_eq!(
        q(&g, "flags(type_only, deps(test/a.test.ts))"),
        ["src/types.ts"]
    );
    assert_eq!(q(&g, "phase(build, **)"), Vec::<String>::new());
    assert_eq!(
        q(&g, "phase(runtime, d/**)"),
        ["d/2.ts", "d/3.ts", "d/4.ts"]
    );
    // The source of the edge must be in the set too.
    assert_eq!(q(&g, "kind(import, src/a.ts)"), Vec::<String>::new());
    assert!(!q(&g, "lang(js, **)").contains(&"data/config.json".to_owned()));
    assert_eq!(q(&g, "lang(js|py, data/**)"), Vec::<String>::new());
    assert_eq!(q(&g, "pkg(lodash, **)"), ["node_modules/lodash/index.js"]);
    assert_eq!(q(&g, "pkg(., d/**)").len(), 4);
    assert_eq!(
        q(&g, "external(deps(test/a.test.ts))"),
        ["node_modules/lodash/index.js"]
    );
    assert_eq!(
        q(&g, "uncertain(**)"),
        ["node_modules/lodash/index.js", "src/dyn.ts"]
    );
    assert_eq!(
        q(&g, "uncertain(eval_like, **)"),
        ["node_modules/lodash/index.js"]
    );
    assert_eq!(
        q(&g, "uncertain(computed_specifier|eval_like, src/**)"),
        ["src/dyn.ts"]
    );
}

#[test]
fn set_operations_and_let() {
    let g = graph();
    assert_eq!(
        q(&g, "rdeps(src/shared.ts) ^ test/**"),
        ["test/a.test.ts", "test/b.test.ts"]
    );
    assert_eq!(
        q(&g, "deps(test/b.test.ts) - deps(src/x.ts) - src/**"),
        ["node_modules/lodash/index.js", "test/b.test.ts"]
    );
    assert_eq!(q(&g, "d/1.ts + d/2.ts").len(), 2);
    assert_eq!(
        q(
            &g,
            "let t = test/** in rdeps(src/util.ts) intersect $t except test/b.test.ts"
        ),
        ["test/a.test.ts"]
    );
    let mut e = Engine::new(&g);
    e.bind(
        "tests",
        turborepo_trace_graph::Value::Nodes(e.query("test/**").unwrap().nodes),
    );
    let r = e.query("rdeps(src/c.ts) ^ $tests").unwrap();
    assert_eq!(paths(&g, &r.nodes), ["test/c.test.ts"]);
}

#[test]
fn traversal_filter_runtime_only() {
    let g = graph();
    let e = Engine::with_options(
        &g,
        EngineOptions {
            traversal: EdgeFilter::ALL.without_flags(RefFlags::TYPE_ONLY),
            ..Default::default()
        },
    );
    let r = e.query("deps(test/a.test.ts)").unwrap();
    assert!(!paths(&g, &r.nodes).contains(&"src/types.ts".to_owned()));
    let r = e.query("rdeps(src/types.ts)").unwrap();
    assert_eq!(paths(&g, &r.nodes), ["src/types.ts"]);
}

#[test]
fn evaluation_errors() {
    let g = graph();
    let r = qerr(&g, "deps(src/nope.ts)");
    assert!(r.contains("no file `src/nope.ts` in the graph"), "{r}");
    assert!(r.contains("     ^^^^^^^^^^^"), "{r}");
    let r = qerr(&g, "deps(src)");
    assert!(
        r.contains("`src` is a directory with 9 files; use `src/**`"),
        "{r}"
    );
    let r = qerr(&g, "dep(src/a.ts)");
    assert!(
        r.contains("unknown function `dep`") && r.contains("did you mean `deps`?"),
        "{r}"
    );
    let r = qerr(&g, "kind(imprt, **)");
    assert!(
        r.contains("unknown reference kind `imprt`") && r.contains("`import`"),
        "{r}"
    );
    let r = qerr(&g, "flags(xyzzy, **)");
    assert!(
        r.contains("expected one of: optional, conditional, lazy, type_only"),
        "{r}"
    );
    let r = qerr(&g, "deps(a, b, c)");
    assert!(r.contains("`deps` takes 1 to 2 arguments, got 3"), "{r}");
    let r = qerr(&g, "deps(src/a.ts, -1)");
    assert!(r.contains("expected an expression, found `-`"), "{r}");
    let r = qerr(&g, "deps(src/a.ts, two)");
    assert!(
        r.contains("expected a depth (a non-negative integer), found `two`"),
        "{r}"
    );
    let r = qerr(&g, "$nope");
    assert!(r.contains("unbound variable `$nope`"), "{r}");
    let r = qerr(&g, "pkg:react");
    assert!(r.contains("no package matches `react`"), "{r}");
    let r = qerr(&g, "added(x.ts)");
    assert!(r.contains("this query evaluates to a change set"), "{r}");
    let r = qerr(&g, "deps(added(x.ts))");
    assert!(
        r.contains("expected a set of files, found a change set"),
        "{r}"
    );
    let r = qerr(&g, "affected(added(x.ts) - added(y.ts), **)");
    assert!(r.contains("only `+` is defined on change sets"), "{r}");
    let r = qerr(&g, "affected(added(src/*.ts), **)");
    assert!(r.contains("added paths cannot be globs"), "{r}");
    let r = qerr(&g, "affected(modified(src/a.ts), test/**, maybe)");
    assert!(r.contains("unknown policy `maybe`"), "{r}");
}

fn affected(g: &MemGraph, src: &str) -> Vec<(String, Verdict)> {
    let e = Engine::new(g);
    let r = e
        .query(src)
        .unwrap_or_else(|err| panic!("{}", err.render(src)));
    let rep = r.affected.expect("affected report");
    rep.entries
        .iter()
        .map(|x| (g.path(x.entry).to_owned(), x.verdict))
        .collect()
}

fn s(v: &[(&str, Verdict)]) -> Vec<(String, Verdict)> {
    v.iter().map(|(p, v)| (p.to_string(), *v)).collect()
}

use Verdict::{Affected as A, Unknown as U};

#[test]
fn affected_closure_and_witness() {
    let g = graph();
    let e = Engine::new(&g);
    let r = e
        .query("affected(modified(src/util.ts), test/**, ignore)")
        .unwrap();
    let rep = r.affected.unwrap();
    assert_eq!(rep.entry_count, 4);
    assert_eq!(paths(&g, &r.nodes), ["test/a.test.ts", "test/b.test.ts"]);
    let a = &rep.entries[0];
    let w: Vec<_> = a.witness.iter().map(|&x| g.path(x)).collect();
    assert_eq!(
        w,
        ["src/util.ts", "src/shared.ts", "src/a.ts", "test/a.test.ts"]
    );
    assert_eq!(
        a.cause,
        Some(Cause::Modified {
            path: "src/util.ts".into()
        })
    );
    // A bare set counts as modified.
    assert_eq!(
        affected(&g, "affected(src/util.ts, test/**, ignore)"),
        s(&[("test/a.test.ts", A), ("test/b.test.ts", A)])
    );
    // Deleting a file in the closure.
    assert_eq!(
        affected(&g, "affected(deleted(src/c.ts), test/**, ignore)"),
        s(&[("test/c.test.ts", A)])
    );
    // Entries themselves can be the change.
    assert_eq!(
        affected(&g, "affected(modified(test/c.test.ts), test/**, ignore)"),
        s(&[("test/c.test.ts", A)])
    );
}

#[test]
fn affected_uncertainty_policies() {
    let g = graph();
    let src = |p: &str| format!("affected(modified(src/c.ts), test/**, {p})");
    assert_eq!(
        affected(&g, &src("affected")),
        s(&[("test/c.test.ts", A), ("test/d.test.ts", A)])
    );
    assert_eq!(
        affected(&g, &src("unknown")),
        s(&[("test/c.test.ts", A), ("test/d.test.ts", U)])
    );
    assert_eq!(affected(&g, &src("ignore")), s(&[("test/c.test.ts", A)]));
    // Default policy comes from the engine options (affected).
    assert_eq!(
        affected(&g, "affected(modified(src/c.ts), test/**)"),
        s(&[("test/c.test.ts", A), ("test/d.test.ts", A)])
    );
    // The uncertainty reason is reported; lodash's (external) uncertainty never
    // counts, although test/a's closure contains it.
    let e = Engine::new(&g);
    let r = e
        .query("affected(modified(README.md), test/**, unknown)")
        .unwrap();
    let rep = r.affected.unwrap();
    assert_eq!(rep.entries.len(), 1);
    let d = &rep.entries[0];
    assert_eq!(g.path(d.entry), "test/d.test.ts");
    let site = d.uncertain.as_ref().unwrap();
    assert_eq!(g.path(site.node), "src/dyn.ts");
    assert_eq!(
        site.reasons,
        [(
            UncertaintyKind::ComputedSpecifier,
            3,
            "require(name)".to_owned()
        )]
    );
    assert_eq!(rep.unmatched.len(), 1);
    assert_eq!(rep.unmatched[0].path, "README.md");
    assert!(r.nodes.is_empty());
    // Excluding the kind from the counted set drops the verdict.
    let opts = AffectedOptions::default()
        .policy(Policy::Unknown)
        .count_kind(UncertaintyKind::ComputedSpecifier, false);
    let all = e.query("test/**").unwrap().nodes;
    assert!(
        e.affected(&ChangeSet::new(), &all, &opts)
            .entries
            .is_empty()
    );
}

#[test]
fn affected_probes() {
    let g = graph();
    let e = Engine::new(&g);
    let r = e
        .query("affected(added(src/a-helper.ts), test/**, ignore)")
        .unwrap();
    let rep = r.affected.unwrap();
    assert_eq!(rep.entries.len(), 1);
    assert_eq!(g.path(rep.entries[0].entry), "test/a.test.ts");
    assert_eq!(
        rep.entries[0].cause,
        Some(Cause::Probe {
            added: "src/a-helper.ts".into(),
            probe: "src/a-helper.ts".into()
        })
    );
    // A file below a probed missing directory.
    let rep = e
        .query("affected(added(node_modules/left-pad/index.js), test/**, ignore)")
        .unwrap()
        .affected
        .unwrap();
    assert_eq!(
        rep.entries[0].cause,
        Some(Cause::Probe {
            added: "node_modules/left-pad/index.js".into(),
            probe: "node_modules/left-pad".into()
        })
    );
    // Not a probe: a sibling with a shared name prefix.
    assert_eq!(
        affected(
            &g,
            "affected(added(node_modules/left-pad2/x.js), test/**, ignore)"
        ),
        s(&[])
    );
}

/// A path that changes takes every answer below it with it: a link to a
/// directory that appears or goes above a path that was looked for, and a
/// directory that the change made or removed (`ChangeSet::dirs`).
#[test]
fn affected_probes_below_a_changed_path() {
    let g = graph();
    let e = Engine::new(&g);
    // Added above the probe `node_modules/left-pad` (a link, say).
    let rep = e
        .query("affected(added(node_modules), test/**, ignore)")
        .unwrap()
        .affected
        .unwrap();
    assert_eq!(rep.entries.len(), 1);
    assert_eq!(g.path(rep.entries[0].entry), "test/a.test.ts");
    assert_eq!(
        rep.entries[0].cause,
        Some(Cause::Probe {
            added: "node_modules".into(),
            probe: "node_modules/left-pad".into()
        })
    );
    // Not below: a name that the probe's first name starts with.
    assert_eq!(
        affected(&g, "affected(added(node_mod), test/**, ignore)"),
        s(&[])
    );
    let all = e.query("test/**").unwrap().nodes;
    let ignore = AffectedOptions::default().policy(Policy::Ignore);
    let selected = |cs: &ChangeSet, opts: &AffectedOptions| -> Vec<String> {
        let rep = e.affected(cs, &all, opts);
        rep.entries
            .iter()
            .map(|r| g.path(r.entry).to_owned())
            .collect()
    };
    // Deleted: only a graph of the tree after the change can hold a path that was
    // looked for below it.
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Deleted, "node_modules");
    assert!(selected(&cs, &ignore).is_empty());
    assert_eq!(
        selected(&cs, &ignore.clone().after_change(true)),
        ["test/a.test.ts"]
    );
}

/// A directory that a change makes or removes (`ChangeSet::dirs`) is an answer
/// to the lookups that asked for it by name: a membership of its parent
/// directory under its literal name. It matches nothing else: no pattern, no
/// probe at, above or below it.
#[test]
fn affected_directories_of_a_change() {
    let mut b = MemGraph::builder();
    let t = b.file("test/t.test.ts");
    let asked = b.file("src/asked.ts");
    let listed = b.file("src/listed.ts");
    let probed = b.file("src/probed.ts");
    b.edge(t, asked, RefKind::Import)
        .edge(t, listed, RefKind::Import)
        .edge(t, probed, RefKind::Import);
    // "Is `configs/base` a directory?": the name, in its parent.
    b.membership(asked, "configs", "base");
    // Listings with patterns, and paths that were looked for.
    b.membership(listed, "configs", "*")
        .membership(listed, "", "**");
    b.probe(probed, "configs/base")
        .probe(probed, "configs/base/tsconfig.json")
        .probe(probed, "configs");
    let g = b.build().unwrap();
    let e = Engine::new(&g);
    let all = e.query("**").unwrap().nodes;
    let ignore = AffectedOptions::default()
        .policy(Policy::Ignore)
        .after_change(true);
    for kind in [ChangeKind::Added, ChangeKind::Deleted] {
        let seeds = |path: &str| -> Vec<(String, Option<Cause>)> {
            let cs = ChangeSet {
                dirs: vec![Change::new(kind, path)],
                ..ChangeSet::default()
            };
            let rep = e.affected(&cs, &all, &ignore);
            let mut v: Vec<(String, Option<Cause>)> = rep
                .entries
                .iter()
                .map(|r| (g.path(r.entry).to_owned(), r.cause.clone()))
                .collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        };
        let cause = Cause::Membership {
            path: "configs/base".into(),
            kind,
            dir: "configs".into(),
            pattern: "base".into(),
        };
        assert_eq!(
            seeds("configs/base"),
            [
                ("src/asked.ts".to_owned(), Some(cause.clone())),
                ("test/t.test.ts".to_owned(), Some(cause)),
            ]
        );
        // Another directory there, a directory below it, and its parent.
        assert_eq!(seeds("configs/other"), []);
        assert_eq!(seeds("configs/base/deep"), []);
        assert_eq!(seeds("configs"), []);
    }
}

#[test]
fn affected_memberships() {
    let g = graph();
    assert_eq!(
        affected(&g, "affected(added(locales/fr.json), test/**, ignore)"),
        s(&[("test/c.test.ts", A)])
    );
    assert_eq!(
        affected(&g, "affected(deleted(locales/en.json), test/**, ignore)"),
        s(&[("test/c.test.ts", A)])
    );
    // `*.json` does not cover subdirectories, and modifications don't change
    // listings.
    assert_eq!(
        affected(&g, "affected(added(locales/sub/x.json), test/**, ignore)"),
        s(&[])
    );
    assert_eq!(
        affected(&g, "affected(modified(locales/en.json), test/**, ignore)"),
        s(&[])
    );
    let e = Engine::new(&g);
    let rep = e
        .query("affected(added(locales/fr.json), test/**, ignore)")
        .unwrap()
        .affected
        .unwrap();
    assert_eq!(
        rep.entries[0].cause,
        Some(Cause::Membership {
            path: "locales/fr.json".into(),
            kind: ChangeKind::Added,
            dir: "locales".into(),
            pattern: "*.json".into()
        })
    );
}

#[test]
fn affected_external_package() {
    let g = graph();
    assert_eq!(
        affected(&g, "affected(modified(pkg:lodash), test/**, ignore)"),
        s(&[("test/a.test.ts", A), ("test/b.test.ts", A)])
    );
    let e = Engine::new(&g);
    let lodash = g.package(n(&g, "node_modules/lodash/index.js"));
    let mut cs = ChangeSet::new();
    cs.push_package(lodash);
    let all = e.query("test/**").unwrap().nodes;
    let rep = e.affected(
        &cs,
        &all,
        &AffectedOptions::default().policy(Policy::Ignore),
    );
    assert_eq!(rep.entries.len(), 2);
    assert_eq!(
        rep.entries[0].cause,
        Some(Cause::Package { package: lodash })
    );
    assert_eq!(rep.seed_count, 1);
}

#[test]
fn affected_combined_changes() {
    let g = graph();
    assert_eq!(
        affected(
            &g,
            "affected(modified(src/x.ts) + added(locales/x.json) + src/dyn.ts, test/**, ignore)"
        ),
        s(&[
            ("test/b.test.ts", A),
            ("test/c.test.ts", A),
            ("test/d.test.ts", A)
        ])
    );
    let mut e = Engine::new(&g);
    let cs = ChangeSet::from_name_status("M\tsrc/y.ts\nA\tsrc/a-helper.ts\n").unwrap();
    e.bind("changes", turborepo_trace_graph::Value::Changes(cs));
    let r = e.query("affected($changes, test/**, ignore)").unwrap();
    assert_eq!(paths(&g, &r.nodes), ["test/a.test.ts", "test/b.test.ts"]);
}

#[test]
fn affected_witnesses_are_paths() {
    let g = graph();
    let e = Engine::new(&g);
    let r = e
        .query("affected(modified(src/util.ts + src/y.ts + data/config.json), **)")
        .unwrap();
    for x in &r.affected.unwrap().entries {
        assert_eq!(*x.witness.last().unwrap(), x.entry);
        for w in x.witness.windows(2) {
            assert!(
                g.out_edges(w[1]).any(|e| e.to == Some(w[0])),
                "{} does not depend on {}",
                g.path(w[1]),
                g.path(w[0])
            );
        }
    }
}
