//! Tests for erased-equal files (`erase`) and type-only edges per entry
//! (`modes`).

use std::collections::HashMap;

use super::{
    erase::{Hop, hop},
    tests::{Fx, Src},
    *,
};
use crate::{
    change::{AffectedOptions, ChangeKind, ChangeSet, Policy, Verdict},
    mem::{EdgeSpec, MemGraph},
    view::Span,
    vocab::{EdgeAttr, Phase, RefFlags, RefKind, UncertaintyKind},
};

/// What to evaluate: the changes, the erased-equal files, per-entry edge modes,
/// entries besides `*.test.ts`, and the uncertainty policy.
struct Case<'a> {
    changes: &'a [(ChangeKind, &'a str)],
    erased: &'a [&'a str],
    runtime: bool,
    more: &'a [&'a str],
    policy: Policy,
}

impl Default for Case<'_> {
    fn default() -> Self {
        Case {
            changes: &[(ChangeKind::Modified, "types.ts")],
            erased: &[],
            runtime: false,
            more: &[],
            policy: Policy::Ignore,
        }
    }
}

fn prune(
    g: &MemGraph,
    ids: &HashMap<String, NodeId>,
    src: &Src,
    case: &Case<'_>,
) -> (Vec<String>, PruneReport) {
    let engine = Engine::new(g);
    let mut entries = NodeSet::new(g.node_count());
    for (p, &n) in ids {
        if p.ends_with(".test.ts") || case.more.contains(&p.as_str()) {
            entries.insert(n);
        }
    }
    let mut cs = ChangeSet::new();
    for (k, p) in case.changes {
        cs.push(*k, *p);
    }
    let barrels = Barrels {
        isolated: None,
        candidates: Vec::new(),
        erased: case.erased.iter().map(|s| s.to_string()).collect(),
        runtime_entries: case.runtime,
        mocks: false,
        source: src,
    };
    let opts = AffectedOptions::default().policy(case.policy);
    let (r, p) = engine.affected_with(&cs, &entries, &opts, Some(&barrels));
    let mut aff: Vec<String> = r
        .with_verdict(Verdict::Affected)
        .map(|e| g.path(e.entry).to_owned())
        .collect();
    aff.sort();
    (aff, p)
}

/// The affected entries when `types.ts` is modified and erased-equal.
fn erased(fx: Fx, more: &[&str]) -> (Vec<String>, PruneReport) {
    let (g, ids, src) = fx.build();
    prune(
        &g,
        &ids,
        &src,
        &Case {
            erased: &["types.ts"],
            more,
            ..Case::default()
        },
    )
}

/// `t.test.ts` (jest) imports `lib.ts`, which imports `types.ts`.
fn erased_fixture() -> Fx {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "lib.ts", Some(&["x"]));
    fx.import("lib.ts", "types.ts", Some(&["T"]));
    fx.jest("t.test.ts", "module.exports = {}");
    fx
}

#[test]
fn erased_equal_file_is_skipped_for_transpiled_tests() {
    let (g, ids, src) = erased_fixture().build();
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            erased: &["types.ts"],
            ..Case::default()
        },
    );
    assert!(aff.is_empty(), "{aff:?}");
    assert_eq!(p.deferred, 1);
    assert_eq!(p.unaffected, [ids["t.test.ts"]]);
    assert_eq!(p.erased.len(), 1);
    assert_eq!(
        (p.erased[0].node, p.erased[0].entries),
        (ids["types.ts"], 1)
    );
    let engine = Engine::new(&g);
    assert_eq!(
        p.reasons(&engine, ids["t.test.ts"]),
        [Reason::Erased(ids["types.ts"])]
    );
    let mut out = Vec::new();
    p.write_reasons(&engine, ids["t.test.ts"], &mut out)
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "  types.ts skipped: only types, comments or formatting changed (same JavaScript as at \
         the base revision)\n"
    );
    // Not listed as erased-equal: a change like any other.
    let (aff, _) = prune(&g, &ids, &src, &Case::default());
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn erased_equal_file_selects_entries_that_are_not_transpiled_tests() {
    // A plain entry (a build root, a type-check task) and a test of an unknown
    // runner.
    let mut fx = erased_fixture();
    fx.import("build.ts", "lib.ts", Some(&["x"]));
    fx.import("u.test.ts", "lib.ts", Some(&["x"]));
    let (aff, p) = erased(fx, &["build.ts"]);
    assert_eq!(aff, ["build.ts", "u.test.ts"]);
    assert_eq!(p.unaffected.len(), 1);

    // ts-jest: type-checks by default; with `diagnostics: false` it still compiles
    // with the whole program; only `isolatedModules` transpiles file by file.
    let ts_jest = |cfg: &str| {
        let mut fx = erased_fixture();
        fx.edge(
            "t.test.ts",
            "ts-jest.js",
            RefKind::Require,
            "jest:transform ts-jest",
            0,
        );
        fx.src.texts.insert("jest.config.js".into(), cfg.into());
        erased(fx, &[]).0
    };
    assert_eq!(
        ts_jest("module.exports = { preset: 'ts-jest' }"),
        ["t.test.ts"]
    );
    assert_eq!(
        ts_jest("module.exports = { globals: { 'ts-jest': { diagnostics: false } } }"),
        ["t.test.ts"]
    );
    assert!(
        ts_jest("module.exports = { globals: { 'ts-jest': { isolatedModules: true } } }")
            .is_empty()
    );

    // vitest with typecheck.
    let vitest = |cfg: &str| {
        let mut fx = Fx::new();
        fx.import("t.test.ts", "types.ts", Some(&["T"]));
        fx.edge(
            "t.test.ts",
            "vitest.config.ts",
            RefKind::Require,
            "vitest:config vitest.config.ts",
            0,
        );
        fx.src.texts.insert("vitest.config.ts".into(), cfg.into());
        erased(fx, &[]).0
    };
    assert!(vitest("export default {}").is_empty());
    assert_eq!(
        vitest("export default { test: { typecheck: { enabled: true } } }"),
        ["t.test.ts"]
    );
}

#[test]
fn a_test_of_both_runners_gets_the_weaker_answer() {
    // Matched by a jest config and a vitest config: both transpile.
    let both = |vitest_cfg: &str| {
        let mut fx = erased_fixture();
        fx.edge(
            "t.test.ts",
            "vitest.config.ts",
            RefKind::Require,
            "vitest:config vitest.config.ts",
            0,
        );
        fx.src
            .texts
            .insert("vitest.config.ts".into(), vitest_cfg.into());
        erased(fx, &[]).0
    };
    assert!(both("export default {}").is_empty());
    // vitest type-checks: the change counts.
    assert_eq!(
        both("export default { test: { typecheck: { enabled: true } } }"),
        ["t.test.ts"]
    );
}

#[test]
fn a_modified_entry_always_runs() {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "lib.ts", Some(&["x"]));
    fx.jest("t.test.ts", "module.exports = {}");
    let (g, ids, src) = fx.build();
    let (aff, _) = prune(
        &g,
        &ids,
        &src,
        &Case {
            changes: &[(ChangeKind::Modified, "t.test.ts")],
            erased: &["t.test.ts"],
            ..Case::default()
        },
    );
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn readers_of_the_erased_file_text_select() {
    // Reading the file's bytes sees the change.
    let mut fx = erased_fixture();
    fx.edge("r.test.ts", "types.ts", RefKind::FsRead, "./types.ts", 0);
    fx.jest("r.test.ts", "module.exports = {}");
    let (aff, p) = erased(fx, &[]);
    assert_eq!(aff, ["r.test.ts"]);
    assert_eq!(p.erased[0].relevant, 1);

    // Reading the bytes of a module that imports it does not: that file is
    // unchanged.
    let mut fx = erased_fixture();
    fx.edge("r.test.ts", "lib.ts", RefKind::FsRead, "./lib.ts", 0);
    fx.jest("r.test.ts", "module.exports = {}");
    assert!(erased(fx, &[]).0.is_empty());

    // A helper that reads it makes its importers affected.
    let mut fx = erased_fixture();
    fx.edge("helper.ts", "types.ts", RefKind::Asset, "./types.ts", 0);
    fx.import("r.test.ts", "helper.ts", Some(&["h"]));
    fx.jest("r.test.ts", "module.exports = {}");
    assert_eq!(erased(fx, &[]).0, ["r.test.ts"]);
}

#[test]
fn loads_outside_the_transform_select() {
    // The runner's config, loaded by the runner itself (possibly with type checks).
    let mut fx = erased_fixture();
    fx.import("jest.config.js", "types.ts", Some(&["T"]));
    let (aff, _) = erased(fx, &[]);
    assert_eq!(aff, ["t.test.ts"]);

    // Setup files and mocks go through the transform like the test's own imports.
    for label in [
        "jest:setup_file setup.ts",
        "jest:setup_file_after_env setup.ts",
        "jest:manual_mock setup.ts",
        "jest:require_actual ./setup",
    ] {
        let mut fx = Fx::new();
        fx.edge("t.test.ts", "setup.ts", RefKind::Require, label, 0);
        fx.import("setup.ts", "types.ts", Some(&["T"]));
        fx.jest("t.test.ts", "module.exports = {}");
        assert!(erased(fx, &[]).0.is_empty(), "{label}");
    }
    // Global setup, environments, transformers and resolvers don't.
    for label in [
        "jest:global_setup setup.ts",
        "jest:test_environment setup.ts",
        "jest:resolver setup.ts",
    ] {
        let mut fx = Fx::new();
        fx.edge("t.test.ts", "setup.ts", RefKind::Require, label, 0);
        fx.import("setup.ts", "types.ts", Some(&["T"]));
        fx.jest("t.test.ts", "module.exports = {}");
        assert_eq!(erased(fx, &[]).0, ["t.test.ts"], "{label}");
    }

    // Workers and child processes run under another loader.
    for kind in [RefKind::Worker, RefKind::ChildProcess] {
        let mut fx = erased_fixture();
        fx.edge("pool.ts", "lib.ts", kind, "./lib.ts", 0);
        fx.import("w.test.ts", "pool.ts", Some(&["p"]));
        fx.jest("w.test.ts", "module.exports = {}");
        assert_eq!(erased(fx, &[]).0, ["w.test.ts"], "{kind:?}");
    }

    // A build task that runs a module of the region: tests that use its output.
    let mut fx = erased_fixture();
    fx.edge("task:pkg#gen", "lib.ts", RefKind::Import, "lib.ts", 0);
    fx.edge("gen.ts", "task:pkg#gen", RefKind::Import, "task:pkg#gen", 0);
    fx.import("g.test.ts", "gen.ts", Some(&["g"]));
    fx.jest("g.test.ts", "module.exports = {}");
    assert_eq!(erased(fx, &[]).0, ["g.test.ts"]);

    // An edge without a recorded specifier.
    let mut fx = erased_fixture();
    let (a, b) = (fx.node("x.ts"), fx.node("types.ts"));
    fx.b.edge(a, b, RefKind::Import);
    fx.import("x.test.ts", "x.ts", Some(&["x"]));
    fx.jest("x.test.ts", "module.exports = {}");
    assert_eq!(erased(fx, &[]).0, ["x.test.ts"]);
}

#[test]
fn erased_file_with_another_role_is_not_deferred() {
    // An added file matching a probe of the erased-equal file re-seeds it.
    let mut fx = erased_fixture();
    let t = fx.node("types.ts");
    fx.b.probe(t, "types/missing.ts");
    let (g, ids, src) = fx.build();
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            changes: &[
                (ChangeKind::Modified, "types.ts"),
                (ChangeKind::Added, "types/missing.ts"),
            ],
            erased: &["types.ts"],
            ..Case::default()
        },
    );
    assert_eq!(aff, ["t.test.ts"]);
    assert_eq!(p.deferred, 0);

    // Another change in the closure still selects.
    let (g, ids, src) = erased_fixture().build();
    let (aff, _) = prune(
        &g,
        &ids,
        &src,
        &Case {
            changes: &[
                (ChangeKind::Modified, "types.ts"),
                (ChangeKind::Modified, "lib.ts"),
            ],
            erased: &["types.ts"],
            ..Case::default()
        },
    );
    assert_eq!(aff, ["t.test.ts"]);
}

/// `from` imports `to` with `import type`.
fn import_type(fx: &mut Fx, from: &str, to: &str) {
    let (f, t) = (fx.node(from), fx.node(to));
    let start = fx.start(from);
    let mut e = EdgeSpec::new(Some(t), RefKind::Import)
        .specifier(format!("./{to}"))
        .flags(RefFlags::TYPE_ONLY);
    e.span = Some(Span {
        start,
        end: start + 10,
    });
    fx.b.add_edge(f, e);
}

/// `t.test.ts` (jest) imports `lib.ts`, which has `import type` from
/// `types.ts`; `build.ts` imports `lib.ts` too.
fn type_edge_fixture() -> Fx {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "lib.ts", Some(&["x"]));
    import_type(&mut fx, "lib.ts", "types.ts");
    fx.import("build.ts", "lib.ts", Some(&["x"]));
    fx.jest("t.test.ts", "module.exports = {}");
    fx
}

#[test]
fn type_only_edges_are_not_followed_for_transpiled_tests() {
    let (g, ids, src) = type_edge_fixture().build();
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            runtime: true,
            more: &["build.ts"],
            ..Case::default()
        },
    );
    // The build root (not a transpiled test) follows every edge.
    assert_eq!(aff, ["build.ts"]);
    assert_eq!(p.type_only, [ids["t.test.ts"]]);
    let rt = p.runtime_entries.as_ref().unwrap();
    assert!(rt.contains(ids["t.test.ts"]) && !rt.contains(ids["build.ts"]));
    let engine = Engine::new(&g);
    assert_eq!(
        p.reasons(&engine, ids["t.test.ts"]),
        [Reason::TypeOnly(
            ids["types.ts"],
            Some((ids["lib.ts"], ids["types.ts"]))
        )]
    );
    let mut out = Vec::new();
    p.write_reasons(&engine, ids["t.test.ts"], &mut out)
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "  types.ts not followed: lib.ts imports types.ts only for types, which the test runner \
         does not load\n"
    );
    // JSON results name the mode.
    let mut results = vec![
        serde_json::json!({ "entry": "t.test.ts" }),
        serde_json::json!({ "entry": "build.ts" }),
    ];
    p.annotate(&g, &mut results);
    assert_eq!(results[0]["edges"], "runtime");
    assert_eq!(results[1]["edges"], "all");

    // Switched off: every entry follows type-only edges.
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            more: &["build.ts"],
            ..Case::default()
        },
    );
    assert_eq!(aff, ["build.ts", "t.test.ts"]);
    assert!(p.runtime_entries.is_none() && p.type_only.is_empty());
}

#[test]
fn a_value_import_next_to_a_type_import_selects() {
    let mut fx = type_edge_fixture();
    fx.import("lib.ts", "types.ts", Some(&["value"]));
    let (g, ids, src) = fx.build();
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            runtime: true,
            ..Case::default()
        },
    );
    assert_eq!(aff, ["t.test.ts"]);
    assert!(p.type_only.is_empty());
}

#[test]
fn type_checking_runners_follow_type_only_edges() {
    let mut fx = type_edge_fixture();
    fx.edge(
        "t.test.ts",
        "ts-jest.js",
        RefKind::Require,
        "jest:transform ts-jest",
        0,
    );
    let (g, ids, src) = fx.build();
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            runtime: true,
            ..Case::default()
        },
    );
    assert_eq!(aff, ["t.test.ts"]);
    assert!(p.runtime_entries.is_none());
}

#[test]
fn uncertainty_behind_a_type_only_edge_does_not_select_transpiled_tests() {
    let mut fx = type_edge_fixture();
    let t = fx.node("types.ts");
    fx.b.uncertainty(t, UncertaintyKind::ComputedSpecifier, 3, "require(x)");
    fx.import("other.ts", "types.ts", Some(&["v"]));
    fx.import("o.test.ts", "other.ts", Some(&["o"]));
    fx.jest("o.test.ts", "module.exports = {}");
    fx.node("unrelated.ts");
    let (g, ids, src) = fx.build();
    let (aff, _) = prune(
        &g,
        &ids,
        &src,
        &Case {
            changes: &[(ChangeKind::Modified, "unrelated.ts")],
            runtime: true,
            more: &["build.ts"],
            policy: Policy::Affected,
            ..Case::default()
        },
    );
    // The build root and the test that loads types.ts reach the uncertain site; the
    // test that only has a type-only path to it does not.
    assert_eq!(aff, ["build.ts", "o.test.ts"]);
}

#[test]
fn erased_and_type_only_rules_combine() {
    // t.test.ts loads types.ts (erased-equal), which has a type-only edge to
    // changed.ts.
    let mut fx = Fx::new();
    fx.import("t.test.ts", "types.ts", Some(&["T"]));
    import_type(&mut fx, "types.ts", "changed.ts");
    fx.import("v.test.ts", "changed.ts", Some(&["c"]));
    fx.jest("t.test.ts", "module.exports = {}");
    fx.jest("v.test.ts", "module.exports = {}");
    let (g, ids, src) = fx.build();
    let (aff, p) = prune(
        &g,
        &ids,
        &src,
        &Case {
            changes: &[
                (ChangeKind::Modified, "types.ts"),
                (ChangeKind::Modified, "changed.ts"),
            ],
            erased: &["types.ts"],
            runtime: true,
            ..Case::default()
        },
    );
    assert_eq!(aff, ["v.test.ts"]);
    assert_eq!(p.unaffected, [ids["t.test.ts"]]);
}

#[test]
fn mapper_edges_read_what_the_import_reads() {
    // t.test.ts imports `{x}` from the barrel through a module mapper: the import's
    // own edge plus a labelled framework edge without a span.
    let run = |alias_spec: &str| {
        let mut fx = Fx::new();
        fx.import("t.test.ts", "index.ts", Some(&["x"]));
        fx.edge("t.test.ts", "index.ts", RefKind::Require, alias_spec, 0);
        fx.reexport("index.ts", "a.ts", ReExportKind::Star);
        fx.reexport("index.ts", "b.ts", ReExportKind::Star);
        fx.jest("t.test.ts", "module.exports = {}");
        let (g, ids, src) = fx.build();
        let engine = Engine::new(&g);
        let entries = NodeSet::from_iter_in(g.node_count(), [ids["t.test.ts"]]);
        let mut cs = ChangeSet::new();
        cs.push(ChangeKind::Modified, "a.ts");
        let barrels = Barrels::new(
            vec![Candidate {
                path: "a.ts".to_owned(),
                exports: vec!["y".to_owned()],
                narrowed: false,
                members: Vec::new(),
                ..Default::default()
            }],
            &src,
        );
        let opts = AffectedOptions::default().policy(Policy::Ignore);
        let (r, _) = engine.affected_with(&cs, &entries, &opts, Some(&barrels));
        r.with_verdict(Verdict::Affected).count()
    };
    // The mapper edge reads the names of the import it rewrites.
    assert_eq!(run("jest:moduleNameMapper ./index.ts -> /r/index.ts"), 0);
    assert_eq!(run("vitest:resolve.alias ./index.ts -> /r/index.ts"), 0);
    // A mapper edge for a specifier the file does not import statically reads
    // everything.
    assert_eq!(run("jest:moduleNameMapper @r/other -> /r/index.ts"), 1);
    // Other framework edges read everything.
    assert_eq!(run("jest:setup_file index.ts"), 1);
}

#[test]
fn hops() {
    let e = |kind: RefKind, spec: Option<&'static str>| Edge {
        to: None,
        attr: EdgeAttr::new(kind, Phase::Runtime, RefFlags::NONE),
        line: 0,
        span: None,
        specifier: spec,
    };
    assert_eq!(hop("a.ts", &e(RefKind::Import, Some("./b"))), Hop::Load);
    assert_eq!(
        hop("a.mjs", &e(RefKind::DynamicImport, Some("./b"))),
        Hop::Load
    );
    assert_eq!(hop("a.ts", &e(RefKind::ReExport, Some("./b"))), Hop::Load);
    assert_eq!(hop("a.ts", &e(RefKind::Import, None)), Hop::Other);
    assert_eq!(
        hop("task:a#b", &e(RefKind::Import, Some("./b"))),
        Hop::Other
    );
    assert_eq!(hop("a.py", &e(RefKind::Import, Some("b"))), Hop::Other);
    assert_eq!(
        hop("a.ts", &e(RefKind::Require, Some("vitest:setup_file s.ts"))),
        Hop::Load
    );
    // An import rewritten by a module mapper or alias is still a module load.
    assert_eq!(
        hop(
            "a.ts",
            &e(
                RefKind::Require,
                Some("jest:moduleNameMapper @acme/x -> /repo/packages/x/src/index.ts")
            )
        ),
        Hop::Load
    );
    assert_eq!(
        hop(
            "a.ts",
            &e(
                RefKind::Require,
                Some("vitest:resolve.alias @/x -> ./src/x")
            )
        ),
        Hop::Load
    );
    assert_eq!(
        hop(
            "a.ts",
            &e(RefKind::Require, Some("jest:config jest.config.js"))
        ),
        Hop::Other
    );
    assert_eq!(
        hop("a.ts", &e(RefKind::Require, Some("pytest:setup_file s.ts"))),
        Hop::Other
    );
    assert_eq!(hop("a.ts", &e(RefKind::FsRead, Some("./b"))), Hop::Data);
    assert_eq!(hop("a.ts", &e(RefKind::Worker, Some("./b"))), Hop::Other);
}
