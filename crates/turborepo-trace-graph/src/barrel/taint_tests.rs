//! Tests for definition-level tracking: candidates tracked by their changed
//! exports only ([`Candidate::narrowed`]) and importers that pass a change on
//! through some of their exports ([`ModuleSource::taint`]).

use std::collections::HashMap;

use super::{
    tests::{Fx, Src},
    *,
};
use crate::{
    change::{AffectedOptions, ChangeKind, ChangeSet, Policy, Verdict},
    mem::MemGraph,
    vocab::RefKind,
};

/// Runs `affected` for modified candidates `(path, tracked names, narrowed)`
/// and other modified files, with every `*.test.ts` as entry.
fn run(
    g: &MemGraph,
    ids: &HashMap<String, NodeId>,
    src: &Src,
    candidates: &[(&str, &[&str], bool)],
    other: &[&str],
) -> (Vec<String>, PruneReport) {
    let engine = Engine::new(g);
    let mut entries = NodeSet::new(g.node_count());
    for (p, &n) in ids {
        if p.ends_with(".test.ts") {
            entries.insert(n);
        }
    }
    let mut cs = ChangeSet::new();
    for (p, ..) in candidates {
        cs.push(ChangeKind::Modified, *p);
    }
    for p in other {
        cs.push(ChangeKind::Modified, *p);
    }
    let barrels = Barrels::new(
        candidates
            .iter()
            .map(|(p, names, narrowed)| Candidate {
                path: (*p).to_owned(),
                exports: names.iter().map(|s| (*s).to_owned()).collect(),
                narrowed: *narrowed,
                members: Vec::new(),
                ..Default::default()
            })
            .collect(),
        src,
    );
    let opts = AffectedOptions::default().policy(Policy::Ignore);
    let (r, p) = engine.affected_with(&cs, &entries, &opts, Some(&barrels));
    let mut aff: Vec<String> = r
        .with_verdict(Verdict::Affected)
        .map(|e| g.path(e.entry).to_owned())
        .collect();
    aff.sort();
    (aff, p)
}

/// `f.ts` exports `x` and `y`. `m.ts` imports both and exports `ux` (reads
/// `x`), `uy` (reads `y`) and `free` (reads neither); its top level runs code,
/// so it is not pure. `a.test.ts` uses `ux`, `b.test.ts` uses `uy`, `c.test.ts`
/// uses `free`. All are jest tests. Returns the fixture and the start of
/// `m.ts`'s import of `f.ts`.
fn fixture() -> (Fx, u32) {
    let mut fx = Fx::new();
    let at = fx.import_at("m.ts", "f.ts", Some(&["x", "y"]));
    fx.import("a.test.ts", "m.ts", Some(&["ux"]));
    fx.import("b.test.ts", "m.ts", Some(&["uy"]));
    fx.import("c.test.ts", "m.ts", Some(&["free"]));
    for t in ["a.test.ts", "b.test.ts", "c.test.ts"] {
        fx.jest(t, "module.exports = {}");
    }
    (fx, at)
}

#[test]
fn without_facts_a_non_pure_importer_ends_the_walk() {
    let (fx, _) = fixture();
    let (g, ids, src) = fx.build();
    let (aff, p) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts", "b.test.ts", "c.test.ts"]);
    assert_eq!(p.skipped[0].passed, 0);
    // The walk still honours the changed names: `y` only would hit `m.ts` just the
    // same, and a name `m.ts` does not import hits nothing.
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["z"], true)], &[]);
    assert!(aff.is_empty(), "{aff:?}");
}

#[test]
fn an_importer_passes_the_change_on_through_the_exports_that_read_it() {
    let (mut fx, at) = fixture();
    fx.defs(
        "m.ts",
        &[("ux", &[(at, "x")]), ("uy", &[(at, "y")]), ("free", &[])],
        &[],
    );
    let (g, ids, src) = fx.build();
    let (aff, p) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts"]);
    assert_eq!(p.skipped[0].passed, 1);
    assert_eq!(p.skipped[0].changed, Some(vec!["x".to_owned()]));
    assert_eq!(p.unaffected, [ids["b.test.ts"], ids["c.test.ts"]]);
    let engine = Engine::new(&g);
    assert_eq!(
        p.reasons(&engine, ids["b.test.ts"]),
        [Reason::Unused(ids["f.ts"], vec!["x".to_owned()])]
    );
    let mut out = Vec::new();
    p.write_reasons(&engine, ids["b.test.ts"], &mut out)
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "  f.ts skipped: only x changed in it, and nothing this entry loads uses it\n"
    );
    // Both names: both users; the test of `free` stays out.
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x", "y"], true)], &[]);
    assert_eq!(aff, ["a.test.ts", "b.test.ts"]);
    // The witness goes through the importer.
    let engine = Engine::new(&g);
    let entries = NodeSet::from_iter_in(g.node_count(), [ids["a.test.ts"]]);
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, "f.ts");
    let barrels = Barrels::new(
        vec![Candidate {
            path: "f.ts".to_owned(),
            exports: vec!["x".to_owned()],
            narrowed: true,
            members: Vec::new(),
            ..Default::default()
        }],
        &src,
    );
    let opts = AffectedOptions::default().policy(Policy::Ignore);
    let (r, _) = engine.affected_with(&cs, &entries, &opts, Some(&barrels));
    assert_eq!(
        r.entries[0].witness,
        [ids["f.ts"], ids["m.ts"], ids["a.test.ts"]]
    );
}

#[test]
fn a_read_at_load_time_ends_the_walk() {
    let (mut fx, at) = fixture();
    fx.defs("m.ts", &[("ux", &[(at, "x")]), ("free", &[])], &[(at, "y")]);
    let (g, ids, src) = fx.build();
    // `y` is read while `m.ts` loads: every test that loads it.
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["y"], true)], &[]);
    assert_eq!(aff, ["a.test.ts", "b.test.ts", "c.test.ts"]);
    // `x` is not.
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts"]);
}

#[test]
fn the_change_travels_through_several_importers() {
    let (mut fx, at) = fixture();
    fx.defs("m.ts", &[("ux", &[(at, "x")]), ("uy", &[(at, "y")])], &[]);
    // n.ts imports `ux` and `uy` from m.ts; `vx` reads `ux`, `w` reads nothing.
    let at_n = fx.import_at("n.ts", "m.ts", Some(&["ux", "uy"]));
    fx.defs("n.ts", &[("vx", &[(at_n, "ux")]), ("w", &[])], &[]);
    fx.import("d.test.ts", "n.ts", Some(&["vx"]));
    fx.import("e.test.ts", "n.ts", Some(&["w"]));
    fx.jest("d.test.ts", "module.exports = {}");
    fx.jest("e.test.ts", "module.exports = {}");
    let (g, ids, src) = fx.build();
    let (aff, p) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts", "d.test.ts"]);
    assert_eq!(p.skipped[0].passed, 2);
    // `uy` reaches n.ts but nothing there reads it.
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["y"], true)], &[]);
    assert_eq!(aff, ["b.test.ts"]);
}

#[test]
fn reads_the_source_cannot_attribute_end_the_walk() {
    // A dynamic import of f.ts next to the static one: no import declaration to ask
    // about.
    let (mut fx, at) = fixture();
    fx.defs("m.ts", &[("ux", &[(at, "x")])], &[]);
    fx.edge("m.ts", "f.ts", RefKind::DynamicImport, "./f.ts", 999);
    let (g, ids, src) = fx.build();
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts", "b.test.ts", "c.test.ts"]);
    // A namespace import is asked about with every name.
    let mut fx = Fx::new();
    let at = fx.import_at("m.ts", "f.ts", None);
    fx.defs("m.ts", &[("ux", &[(at, "anything")]), ("free", &[])], &[]);
    fx.import("a.test.ts", "m.ts", Some(&["ux"]));
    fx.import("c.test.ts", "m.ts", Some(&["free"]));
    fx.jest("a.test.ts", "module.exports = {}");
    fx.jest("c.test.ts", "module.exports = {}");
    let (g, ids, src) = fx.build();
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts"]);
}

#[test]
fn a_candidate_without_changed_names_selects_nothing_but_itself() {
    // No export changed and the file does the same at load time: not even a
    // namespace import can observe it.
    let mut fx = Fx::new();
    fx.import("a.test.ts", "f.ts", None);
    fx.jest("a.test.ts", "module.exports = {}");
    let (g, ids, src) = fx.build();
    let (aff, p) = run(&g, &ids, &src, &[("f.ts", &[], true)], &[]);
    assert!(aff.is_empty(), "{aff:?}");
    let engine = Engine::new(&g);
    let mut out = Vec::new();
    p.write_reasons(&engine, ids["a.test.ts"], &mut out)
        .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "  f.ts skipped: none of its exports changed, and it does the same when it is loaded\n"
    );
    // A modified test always runs, whatever changed in it.
    let (aff, _) = run(&g, &ids, &src, &[("a.test.ts", &[], true)], &[]);
    assert_eq!(aff, ["a.test.ts"]);
    // A reference that is not an import of the module (the test reads the file)
    // sees the file itself: any change counts.
    let mut fx = Fx::new();
    fx.import("a.test.ts", "f.ts", None);
    fx.edge("b.test.ts", "f.ts", RefKind::FsRead, "./f.ts", 500);
    fx.jest("a.test.ts", "module.exports = {}");
    fx.jest("b.test.ts", "module.exports = {}");
    let (g, ids, src) = fx.build();
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &[], true)], &[]);
    assert_eq!(aff, ["b.test.ts"]);
}

#[test]
fn a_changed_importer_passes_on_only_as_a_candidate() {
    let (mut fx, at) = fixture();
    fx.defs("m.ts", &[("ux", &[(at, "x")]), ("uy", &[(at, "y")])], &[]);
    let (g, ids, src) = fx.build();
    // m.ts is modified too and not a candidate: it is a seed, everything behind it
    // runs.
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &["m.ts"]);
    assert_eq!(aff, ["a.test.ts", "b.test.ts", "c.test.ts"]);
    // m.ts is a candidate whose own change is `uy`: its users, plus the users of
    // what it derives from `x`.
    let (aff, _) = run(
        &g,
        &ids,
        &src,
        &[("f.ts", &["x"], true), ("m.ts", &["uy"], true)],
        &[],
    );
    assert_eq!(aff, ["a.test.ts", "b.test.ts"]);
    let (aff, _) = run(
        &g,
        &ids,
        &src,
        &[("f.ts", &["z"], true), ("m.ts", &["free"], true)],
        &[],
    );
    assert_eq!(aff, ["c.test.ts"]);
}

#[test]
fn an_entry_that_reads_none_of_what_changed_is_not_selected() {
    // Both tests import `x`; the facts of a.test.ts say that no statement reads it
    // (it names `x` in a type, or reads other members of a table of which one
    // member changed). b.test.ts has no facts.
    let mut fx = Fx::new();
    let at = fx.import_at("a.test.ts", "f.ts", Some(&["x"]));
    fx.import("b.test.ts", "f.ts", Some(&["x"]));
    fx.jest("a.test.ts", "module.exports = {}");
    fx.jest("b.test.ts", "module.exports = {}");
    fx.defs("a.test.ts", &[], &[]);
    let (g, ids, src) = fx.build();
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["b.test.ts"]);
    // Its load-time code (the test body) reads `x`.
    let mut fx = Fx::new();
    let at2 = fx.import_at("a.test.ts", "f.ts", Some(&["x"]));
    assert_eq!(at, at2);
    fx.jest("a.test.ts", "module.exports = {}");
    fx.defs("a.test.ts", &[], &[(at2, "x")]);
    let (g, ids, src) = fx.build();
    let (aff, _) = run(&g, &ids, &src, &[("f.ts", &["x"], true)], &[]);
    assert_eq!(aff, ["a.test.ts"]);
}
