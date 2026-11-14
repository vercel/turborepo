use std::{collections::HashMap, sync::Arc};

use super::*;
use crate::{
    change::{AffectedOptions, ChangeKind, ChangeSet, Policy, Verdict},
    mem::{EdgeSpec, GraphBuilder, MemGraph},
    view::Span,
    vocab::RefKind,
};

#[derive(Default)]
pub(super) struct Src {
    pub(super) shapes: HashMap<String, Arc<Shape>>,
    pub(super) texts: HashMap<String, Arc<str>>,
    /// Definition-level facts per file ([`ModuleSource::taint`]); files without
    /// an entry have none.
    pub(super) defs: HashMap<String, Defs>,
}

/// Definition-level facts of a test file: which imported names each export
/// reads, and which ones load-time code reads. A read is `(start of the import,
/// name)`.
#[derive(Default)]
pub(super) struct Defs {
    pub(super) exports: Vec<(String, Vec<(u32, String)>)>,
    pub(super) load: Vec<(u32, String)>,
}

impl ModuleSource for Src {
    fn shape(&self, path: &str) -> Option<Arc<Shape>> {
        self.shapes.get(path).cloned()
    }
    fn text(&self, path: &str) -> Option<Arc<str>> {
        self.texts.get(path).cloned()
    }
    fn reads(&self, path: &str, imports: &[TaintedImport]) -> Option<bool> {
        let t = self.taint(path, imports)?;
        Some(t.evaluation || !t.exports.is_empty())
    }
    fn taint(&self, path: &str, imports: &[TaintedImport]) -> Option<Taint> {
        let d = self.defs.get(path)?;
        let tainted = |r: &(u32, String)| {
            imports
                .iter()
                .any(|i| i.start == r.0 && i.names.as_ref().is_none_or(|n| n.contains(&r.1)))
        };
        Some(Taint {
            evaluation: d.load.iter().any(tainted),
            exports: d
                .exports
                .iter()
                .filter(|(_, reads)| reads.iter().any(tainted))
                .map(|(name, _)| name.clone())
                .collect(),
        })
    }
}

/// A test fixture: files, edges with spans and specifiers, shapes per file.
pub(super) struct Fx {
    pub(super) b: GraphBuilder,
    ids: HashMap<String, NodeId>,
    pub(super) src: Src,
    /// Next span start per file.
    next: HashMap<String, u32>,
    /// The package of the task layer's pseudo-files.
    task_pkg: Option<crate::view::PackageId>,
}

impl Fx {
    pub(super) fn new() -> Self {
        Fx {
            b: MemGraph::builder(),
            ids: HashMap::new(),
            src: Src::default(),
            next: HashMap::new(),
            task_pkg: None,
        }
    }

    pub(super) fn node(&mut self, p: &str) -> NodeId {
        if let Some(&n) = self.ids.get(p) {
            return n;
        }
        // The pseudo-files of the task layer: a `tsc -b` project, a membership hub, a
        // task.
        let lang = [
            ("tsproject:", "task-part"),
            ("members:", "task-members"),
            ("task:", "task"),
        ]
        .iter()
        .find(|(prefix, _)| p.starts_with(prefix))
        .map(|(_, lang)| *lang);
        let n = match lang {
            Some(lang) => {
                let pkg = match self.task_pkg {
                    Some(pkg) => pkg,
                    None => *self
                        .task_pkg
                        .insert(self.b.package("tasks", None, ".tasks")),
                };
                self.b.add_file(p, pkg, Some(lang), false)
            }
            None => self.b.file(p),
        };
        self.ids.insert(p.to_owned(), n);
        n
    }

    fn shape(&mut self, p: &str) -> &mut Shape {
        let s = self.src.shapes.entry(p.to_owned()).or_insert_with(|| {
            Arc::new(Shape {
                understood: true,
                ..Shape::default()
            })
        });
        Arc::get_mut(s).expect("unique")
    }

    pub(super) fn start(&mut self, from: &str) -> u32 {
        let s = self.next.entry(from.to_owned()).or_insert(0);
        let v = *s;
        *s += 100;
        v
    }

    pub(super) fn edge(&mut self, from: &str, to: &str, kind: RefKind, spec: &str, start: u32) {
        let (f, t) = (self.node(from), self.node(to));
        let mut e = EdgeSpec::new(Some(t), kind).specifier(spec);
        e.span = Some(Span {
            start,
            end: start + 10,
        });
        self.b.add_edge(f, e);
    }

    /// `from` imports `names` (`None`: everything) from `to`.
    pub(super) fn import(&mut self, from: &str, to: &str, names: Option<&[&str]>) {
        self.import_at(from, to, names);
    }

    /// [`Fx::import`]; returns the start of the import.
    pub(super) fn import_at(&mut self, from: &str, to: &str, names: Option<&[&str]>) -> u32 {
        let start = self.start(from);
        let spec = format!("./{to}");
        self.edge(from, to, RefKind::Import, &spec, start);
        self.shape(from).uses.push(Use {
            start,
            spec,
            names: names.map(|v| v.iter().map(|s| s.to_string()).collect()),
        });
        start
    }

    /// Definition-level facts of `p`: `exports` as `(export, [(import start,
    /// name)])` and the reads of load-time code.
    pub(super) fn defs(
        &mut self,
        p: &str,
        exports: &[(&str, &[(u32, &str)])],
        load: &[(u32, &str)],
    ) {
        let own = |v: &[(u32, &str)]| v.iter().map(|(s, n)| (*s, (*n).to_owned())).collect();
        self.src.defs.insert(
            p.to_owned(),
            Defs {
                exports: exports
                    .iter()
                    .map(|(e, reads)| ((*e).to_owned(), own(reads)))
                    .collect(),
                load: own(load),
            },
        );
    }

    pub(super) fn reexport(&mut self, from: &str, to: &str, kind: ReExportKind) {
        let start = self.start(from);
        let spec = format!("./{to}");
        self.edge(from, to, RefKind::ReExport, &spec, start);
        self.shape(from)
            .reexports
            .push(ReExport { start, spec, kind });
    }

    /// Makes `t` a jest test with config `jest.config.js` (text `cfg`).
    pub(super) fn jest(&mut self, t: &str, cfg: &str) {
        self.edge(
            t,
            "jest.config.js",
            RefKind::Require,
            "jest:config jest.config.js",
            0,
        );
        self.src.texts.insert("jest.config.js".into(), cfg.into());
    }

    pub(super) fn build(self) -> (MemGraph, HashMap<String, NodeId>, Src) {
        (self.b.build().unwrap(), self.ids, self.src)
    }
}

fn named(pairs: &[(&str, &str)]) -> ReExportKind {
    ReExportKind::Named(
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
    )
}

/// Runs `affected` for a modification of `changed` (a candidate exporting
/// `exports`) with every `*.test.ts` as entry; returns the affected entries and
/// the report.
fn run(
    g: &MemGraph,
    ids: &HashMap<String, NodeId>,
    src: &Src,
    changed: &str,
    exports: Option<&[&str]>,
    extra: &[(ChangeKind, &str)],
) -> (Vec<String>, crate::change::AffectedReport, PruneReport) {
    let engine = Engine::new(g);
    let mut entries = NodeSet::new(g.node_count());
    for (p, &n) in ids {
        if p.ends_with(".test.ts") {
            entries.insert(n);
        }
    }
    let mut cs = ChangeSet::new();
    cs.push(ChangeKind::Modified, changed);
    for (k, p) in extra {
        cs.push(*k, *p);
    }
    let barrels = exports.map(|e| {
        Barrels::new(
            vec![Candidate {
                path: changed.to_owned(),
                exports: e.iter().map(|s| s.to_string()).collect(),
                narrowed: false,
                members: Vec::new(),
                ..Default::default()
            }],
            src,
        )
    });
    let opts = AffectedOptions::default().policy(Policy::Ignore);
    let (r, p) = engine.affected_with(&cs, &entries, &opts, barrels.as_ref());
    let mut aff: Vec<String> = r
        .with_verdict(Verdict::Affected)
        .map(|e| g.path(e.entry).to_owned())
        .collect();
    aff.sort();
    (aff, r, p)
}

/// `t.test.ts` imports `{x}` from the barrel `index.ts`, which re-exports
/// `a.ts` and `b.ts`; `t.test.ts` is a jest test.
fn barrel_fixture(t_names: Option<&[&str]>) -> Fx {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", t_names);
    fx.reexport("index.ts", "a.ts", ReExportKind::Star);
    fx.reexport("index.ts", "b.ts", ReExportKind::Star);
    fx.jest("t.test.ts", "module.exports = {}");
    fx
}

#[test]
fn unused_reexported_file_is_skipped() {
    let (g, ids, src) = barrel_fixture(Some(&["x"])).build();
    let (aff, r, p) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert!(aff.is_empty(), "{aff:?}");
    assert_eq!(p.deferred, 1);
    assert_eq!(p.unaffected, [ids["t.test.ts"]]);
    assert_eq!(p.skipped[0].barrels, [ids["index.ts"]]);
    assert_eq!(p.skipped[0].entries, 1);
    assert_eq!(r.seed_count, 1);
    let reasons = p.reasons(&Engine::new(&g), ids["t.test.ts"]);
    assert_eq!(
        reasons,
        [Reason::Barrel(ids["a.ts"], Some(ids["index.ts"]))]
    );
}

#[test]
fn without_barrel_input_everything_reached_is_affected() {
    let (g, ids, src) = barrel_fixture(Some(&["x"])).build();
    let (aff, _, p) = run(&g, &ids, &src, "a.ts", None, &[]);
    assert_eq!(aff, ["t.test.ts"]);
    assert_eq!(p, PruneReport::default());
}

#[test]
fn used_name_selects_with_a_witness_through_the_barrel() {
    let (g, ids, src) = barrel_fixture(Some(&["x"])).build();
    let (aff, r, _) = run(&g, &ids, &src, "a.ts", Some(&["x", "y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
    let e = &r.entries[0];
    assert_eq!(e.witness, [ids["a.ts"], ids["index.ts"], ids["t.test.ts"]]);
    assert_eq!(
        e.cause,
        Some(Cause::Modified {
            path: "a.ts".into()
        })
    );
}

#[test]
fn whole_module_uses_select() {
    // Namespace import with dynamic use, plain require, import(): no names.
    let (g, ids, src) = barrel_fixture(None).build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn unexplained_edges_select() {
    // An edge the importer's shape doesn't list (a framework edge, a dynamic
    // import).
    let mut fx = barrel_fixture(Some(&["x"]));
    fx.edge(
        "t.test.ts",
        "index.ts",
        RefKind::DynamicImport,
        "./index.ts",
        999,
    );
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
    // A file without a shape (not JS, unreadable).
    let mut fx = barrel_fixture(Some(&["x"]));
    fx.src.shapes.remove("t.test.ts");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
    // A shape that is not understood or uses everything.
    let tweaks: [fn(&mut Shape); 2] = [|s| s.understood = false, |s| s.uses_all = true];
    for f in tweaks {
        let mut fx = barrel_fixture(Some(&["x"]));
        f(fx.shape("t.test.ts"));
        let (g, ids, src) = fx.build();
        let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
        assert_eq!(aff, ["t.test.ts"]);
    }
}

#[test]
fn framework_edge_to_the_reexported_file_selects() {
    let mut fx = barrel_fixture(Some(&["x"]));
    fx.edge(
        "t.test.ts",
        "a.ts",
        RefKind::Require,
        "jest:requireActual a.ts",
        0,
    );
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn renamed_reexports_map_names() {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["x"]));
    fx.reexport("index.ts", "a.ts", named(&[("y", "x")]));
    fx.jest("t.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);

    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["x"]));
    fx.reexport("index.ts", "a.ts", named(&[("x", "z")]));
    fx.jest("t.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["x"]), &[]);
    assert!(aff.is_empty());
}

#[test]
fn nested_barrels() {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["x"]));
    fx.reexport("index.ts", "inner/index.ts", ReExportKind::Star);
    fx.reexport("inner/index.ts", "a.ts", ReExportKind::Star);
    fx.jest("t.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert!(aff.is_empty());
    let (aff, r, _) = run(&g, &ids, &src, "a.ts", Some(&["x"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
    assert_eq!(
        r.entries[0].witness,
        [
            ids["a.ts"],
            ids["inner/index.ts"],
            ids["index.ts"],
            ids["t.test.ts"]
        ]
    );
}

#[test]
fn namespace_reexports() {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["ns"]));
    fx.import("u.test.ts", "index.ts", Some(&["other"]));
    fx.reexport("index.ts", "a.ts", ReExportKind::Namespace("ns".into()));
    fx.jest("t.test.ts", "");
    fx.jest("u.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn star_does_not_forward_default() {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["default"]));
    fx.reexport("index.ts", "a.ts", ReExportKind::Star);
    fx.jest("t.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["default"]), &[]);
    assert!(aff.is_empty());
}

#[test]
fn star_collisions_keep_the_file() {
    // `x` comes from b.ts, but a.ts (newly, or at the base) exports `x` too:
    // ambiguous in ESM, first-wins in CommonJS. The name reaches every `export
    // *` source.
    let (g, ids, src) = barrel_fixture(Some(&["x"])).build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["x"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn importer_of_the_file_inside_the_barrel_makes_dependents_affected() {
    // index.ts re-exports c.ts, and c.ts imports {y} from a.ts: whoever uses c.ts
    // sees a.ts.
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["c"]));
    fx.reexport("index.ts", "c.ts", ReExportKind::Star);
    fx.reexport("index.ts", "a.ts", ReExportKind::Star);
    fx.import("c.ts", "a.ts", Some(&["y"]));
    fx.jest("t.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, r, p) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);
    assert_eq!(r.entries[0].witness[0], ids["a.ts"]);
    assert_eq!(*r.entries[0].witness.last().unwrap(), ids["t.test.ts"]);
    assert_eq!(p.skipped[0].relevant, 1);
    // ... but not when c.ts imports a name a.ts doesn't export.
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["z"]), &[]);
    assert!(aff.is_empty());
}

/// index.ts re-exports a.ts and c.ts; c.ts imports `{y}` from a.ts and is pure
/// (exports `c`); t.test.ts uses `x`, u.test.ts uses `c`.
fn pure_importer_fixture() -> Fx {
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["x"]));
    fx.import("u.test.ts", "index.ts", Some(&["c"]));
    fx.reexport("index.ts", "c.ts", ReExportKind::Star);
    fx.reexport("index.ts", "a.ts", ReExportKind::Star);
    fx.import("c.ts", "a.ts", Some(&["y"]));
    fx.shape("c.ts").pure_exports = Some(vec!["c".into()]);
    fx.jest("t.test.ts", "");
    fx.jest("u.test.ts", "");
    fx
}

#[test]
fn pure_importers_pass_the_change_on_through_their_exports() {
    let (g, ids, src) = pure_importer_fixture().build();
    let (aff, r, p) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["u.test.ts"]);
    assert_eq!(
        r.entries[0].witness,
        [ids["a.ts"], ids["c.ts"], ids["index.ts"], ids["u.test.ts"]]
    );
    assert_eq!(p.unaffected, [ids["t.test.ts"]]);
    // Only index.ts re-exports a.ts; c.ts imports it.
    assert_eq!(p.skipped[0].barrels, [ids["index.ts"]]);
}

#[test]
fn changed_importers_stay_relevant() {
    let (g, ids, src) = pure_importer_fixture().build();
    let (aff, ..) = run(
        &g,
        &ids,
        &src,
        "a.ts",
        Some(&["y"]),
        &[(ChangeKind::Modified, "c.ts")],
    );
    assert_eq!(aff, ["t.test.ts", "u.test.ts"]);
}

#[test]
fn entries_using_the_names_stay_relevant() {
    let mut fx = pure_importer_fixture();
    fx.import("v.test.ts", "a.ts", Some(&["y"]));
    fx.shape("v.test.ts").pure_exports = Some(Vec::new());
    fx.jest("v.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["u.test.ts", "v.test.ts"]);
}

#[test]
fn an_entry_that_reexports_a_tracked_name_uses_it() {
    // A route file `export { GET } from './handlers'`, a config `export { default }
    // from './base'`: what loads an entry may read what it exports, so an entry
    // that forwards a name of the changed file is no barrel on the way to
    // somewhere else.
    for kind in [
        named(&[("y", "handler")]),
        ReExportKind::Star,
        ReExportKind::Namespace("ns".into()),
    ] {
        let mut fx = Fx::new();
        fx.reexport("r.test.ts", "a.ts", kind);
        fx.jest("r.test.ts", "");
        let (g, ids, src) = fx.build();
        let (aff, r, p) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
        assert_eq!(aff, ["r.test.ts"]);
        assert_eq!(r.entries[0].witness, [ids["a.ts"], ids["r.test.ts"]]);
        // The entry is a reader of the file, not a barrel it was skipped through.
        assert!(p.skipped[0].barrels.is_empty(), "{:?}", p.skipped);
        assert_eq!(p.skipped[0].relevant, 1);
    }
    // Through a barrel, only for the names the entry forwards.
    let fixture = || {
        let mut fx = Fx::new();
        fx.reexport("index.ts", "a.ts", ReExportKind::Star);
        fx.reexport("index.ts", "b.ts", ReExportKind::Star);
        fx.reexport("r.test.ts", "index.ts", named(&[("y", "handler")]));
        fx.jest("r.test.ts", "");
        fx.build()
    };
    let (g, ids, src) = fixture();
    let (aff, r, _) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["r.test.ts"]);
    assert_eq!(
        r.entries[0].witness,
        [ids["a.ts"], ids["index.ts"], ids["r.test.ts"]]
    );
    let (aff, _, p) = run(&g, &ids, &src, "b.ts", Some(&["z"]), &[]);
    assert!(aff.is_empty(), "{aff:?}");
    assert_eq!(p.unaffected, [ids["r.test.ts"]]);
    // `export *` forwards no default export.
    let mut fx = Fx::new();
    fx.reexport("r.test.ts", "a.ts", ReExportKind::Star);
    fx.jest("r.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["default"]), &[]);
    assert!(aff.is_empty(), "{aff:?}");
    // The dependents of such an entry are selected with it.
    let mut fx = Fx::new();
    fx.reexport("r.test.ts", "a.ts", ReExportKind::Star);
    fx.import("s.test.ts", "r.test.ts", Some(&["other"]));
    fx.jest("r.test.ts", "");
    fx.jest("s.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["r.test.ts", "s.test.ts"]);
}

#[test]
fn a_changed_entry_runs_even_when_nothing_reads_its_exports() {
    // A route file with nothing but exported functions is side-effect free, so it
    // is a candidate; no module imports it. What reads its exports is whatever
    // loads the entry.
    let mut fx = Fx::new();
    fx.import("r.test.ts", "lib.ts", Some(&["x"]));
    fx.jest("r.test.ts", "");
    let (g, ids, src) = fx.build();
    let (aff, r, p) = run(&g, &ids, &src, "r.test.ts", Some(&["GET"]), &[]);
    assert_eq!(aff, ["r.test.ts"]);
    assert_eq!(r.entries[0].witness, [ids["r.test.ts"]]);
    assert!(p.unaffected.is_empty());
    // An entry that imports it is still decided by the names it uses.
    let mut fx = Fx::new();
    fx.import("s.test.ts", "r.test.ts", Some(&["other"]));
    fx.import("u.test.ts", "r.test.ts", Some(&["GET"]));
    for t in ["r.test.ts", "s.test.ts", "u.test.ts"] {
        fx.jest(t, "");
    }
    let (g, ids, src) = fx.build();
    let (aff, _, p) = run(&g, &ids, &src, "r.test.ts", Some(&["GET"]), &[]);
    assert_eq!(aff, ["r.test.ts", "u.test.ts"]);
    assert_eq!(p.unaffected, [ids["s.test.ts"]]);
}

#[test]
fn other_seed_causes_are_not_deferred() {
    // An added file matching a probe on a.ts re-seeds it: no deferral.
    let mut fx = barrel_fixture(Some(&["x"]));
    let a = fx.node("a.ts");
    fx.b.probe(a, "a/missing.ts");
    let (g, ids, src) = fx.build();
    let (aff, _, p) = run(
        &g,
        &ids,
        &src,
        "a.ts",
        Some(&["y"]),
        &[(ChangeKind::Added, "a/missing.ts")],
    );
    assert_eq!(aff, ["t.test.ts"]);
    assert_eq!(p.deferred, 0);
}

#[test]
fn other_changes_still_select() {
    let (g, ids, src) = barrel_fixture(Some(&["x"])).build();
    let (aff, ..) = run(
        &g,
        &ids,
        &src,
        "a.ts",
        Some(&["y"]),
        &[(ChangeKind::Modified, "b.ts")],
    );
    assert_eq!(aff, ["t.test.ts"]);
}

#[test]
fn only_transpiling_test_runners_prune() {
    // Not a test of a known runner.
    let mut fx = Fx::new();
    fx.import("t.test.ts", "index.ts", Some(&["x"]));
    fx.reexport("index.ts", "a.ts", ReExportKind::Star);
    let (g, ids, src) = fx.build();
    let (aff, ..) = run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]);
    assert_eq!(aff, ["t.test.ts"]);

    // ts-jest type-checks unless diagnostics are off.
    let ts_jest = |cfg: &str| {
        let mut fx = barrel_fixture(Some(&["x"]));
        fx.edge(
            "t.test.ts",
            "ts-jest.js",
            RefKind::Require,
            "jest:transform ts-jest",
            0,
        );
        fx.src.texts.insert("jest.config.js".into(), cfg.into());
        fx.build()
    };
    let (g, ids, src) = ts_jest("module.exports = { preset: 'ts-jest' }");
    assert_eq!(
        run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0,
        ["t.test.ts"]
    );
    let (g, ids, src) = ts_jest(
        "module.exports = { transform: { '^.+\\\\.ts$': ['ts-jest', { diagnostics: false }] } }",
    );
    assert!(run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0.is_empty());
    let (g, ids, src) = ts_jest("{ \"globals\": { \"ts-jest\": { \"isolatedModules\": true } } }");
    assert!(run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0.is_empty());

    // Unknown transforms may type-check.
    let mut fx = barrel_fixture(Some(&["x"]));
    fx.edge(
        "t.test.ts",
        "tx.js",
        RefKind::Require,
        "jest:transform ./tools/tx.js",
        0,
    );
    let (g, ids, src) = fx.build();
    assert_eq!(
        run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0,
        ["t.test.ts"]
    );
    // babel-jest only transpiles.
    let mut fx = barrel_fixture(Some(&["x"]));
    fx.edge(
        "t.test.ts",
        "bj.js",
        RefKind::Require,
        "jest:transform babel-jest",
        0,
    );
    let (g, ids, src) = fx.build();
    assert!(run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0.is_empty());
    // So does the SWC transformer `next/jest` installs.
    let mut fx = barrel_fixture(Some(&["x"]));
    fx.edge(
        "t.test.ts",
        "next-swc.js",
        RefKind::Require,
        "jest:transform next/dist/build/swc/jest-transformer",
        0,
    );
    let (g, ids, src) = fx.build();
    assert!(run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0.is_empty());
}

/// A transformer file of the repository that wraps transpilers (babel with a
/// plugin around next's SWC transformer, plus builtins and a helper file)
/// cannot type-check. One that loads anything else, or has an uncertainty, may.
#[test]
fn first_party_transformers_that_wrap_transpilers() {
    let with = |extra: &[(&str, &str)], uncertain: bool| {
        let mut fx = barrel_fixture(Some(&["x"]));
        fx.edge(
            "t.test.ts",
            "test/tx.cjs",
            RefKind::Require,
            "jest:transform test/tx.cjs",
            0,
        );
        for (i, spec) in [
            "@babel/core",
            "node:crypto",
            "node:fs",
            "next/dist/build/swc/jest-transformer",
            "babel-plugin-react-compiler",
            "babel-plugin-react-compiler/package.json",
            "next/package.json",
        ]
        .iter()
        .enumerate()
        {
            fx.edge(
                "test/tx.cjs",
                &format!("pkg{i}.js"),
                RefKind::Require,
                spec,
                100 * (i as u32 + 1),
            );
        }
        fx.edge(
            "test/tx.cjs",
            "test/key.cjs",
            RefKind::Require,
            "./key.cjs",
            900,
        );
        for (i, (to, spec)) in extra.iter().enumerate() {
            fx.edge(
                "test/key.cjs",
                to,
                RefKind::Require,
                spec,
                100 * (i as u32 + 1),
            );
        }
        if uncertain {
            let n = fx.node("test/key.cjs");
            fx.b.uncertainty(
                n,
                crate::vocab::UncertaintyKind::ComputedSpecifier,
                3,
                "require(name)",
            );
        }
        let (g, ids, src) = fx.build();
        run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0
    };
    assert!(with(&[], false).is_empty());
    assert_eq!(with(&[("ts.js", "typescript")], false), ["t.test.ts"]);
    assert_eq!(
        with(&[("cp.js", "node:child_process")], false),
        ["t.test.ts"]
    );
    assert_eq!(with(&[], true), ["t.test.ts"]);
}

#[test]
fn vitest_typecheck_selects() {
    let vitest = |test: &str, cfg: &str| {
        let mut fx = Fx::new();
        fx.import(test, "index.ts", Some(&["x"]));
        fx.reexport("index.ts", "a.ts", ReExportKind::Star);
        fx.edge(
            test,
            "vitest.config.ts",
            RefKind::Require,
            "vitest:config vitest.config.ts",
            0,
        );
        fx.src.texts.insert("vitest.config.ts".into(), cfg.into());
        fx.build()
    };
    let (g, ids, src) = vitest("t.test.ts", "export default {}");
    assert!(run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0.is_empty());
    let (g, ids, src) = vitest(
        "t.test.ts",
        "export default { test: { typecheck: { enabled: true } } }",
    );
    assert_eq!(
        run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0,
        ["t.test.ts"]
    );
    // Type tests run through tsc.
    let (g, ids, src) = vitest("t.test-d.test.ts", "export default {}");
    assert_eq!(
        run(&g, &ids, &src, "a.ts", Some(&["y"]), &[]).0,
        ["t.test-d.test.ts"]
    );
}

#[test]
fn helpers() {
    assert_eq!(package_of("@swc/jest/x"), "@swc/jest");
    assert_eq!(package_of("babel-jest"), "babel-jest");
    assert_eq!(package_of("ts-jest/legacy"), "ts-jest");
    assert!(diagnostics_off("{ 'diagnostics' : false }"));
    assert!(!diagnostics_off("{ diagnostics: { warnOnly: true } }"));
    assert!(is_type_test("src/a.test-d.ts"));
    assert!(!is_type_test("src/a.test.ts"));
}
