//! Mock-aware loading per entry in `affected`.
//!
//! A jest or vitest test that replaces a module with a mock (`jest.mock('./db',
//! () => fake)`) never runs the module's code: every load of it in the test
//! file's module registry returns the mock. A change the test reaches only
//! through the replaced module therefore cannot change its outcome, and
//! [`apply`] takes such entries out of the change-driven results (the
//! uncertainty step still sees them).
//!
//! What a file replaces is decided at trace time
//! (`turborepo_trace_frameworks::mocks`, from the per-file facts of
//! `turborepo_trace_lang_js::mocks`) and travels on marker edges: unresolved
//! edges whose specifier starts with `mock:`.
//!
//! | Marker | On | Meaning |
//! |---|---|---|
//! | `mock:replaces <runner> <path\|name> <module> @ <file>:<line>` | a test (its own calls and those of its project's setup files) | every load of `<module>` returns a mock that does not load it; `name`: the mock names it by a bare request (a package name), and only imports by a bare request are known to get the mock |
//! | `mock:actual <module>` | any file | the file loads the real `<module>` whatever is mocked (`requireActual`, an automock, `unmock`) |
//! | `mock:hazard <what> @ <file>:<line>` | any file | the file may load any replaced module for real; on a test also: the project's replacements do not hold (unknown transform, ES modules, shared registry) |
//!
//! A manual mock that a file other than a test registers is an edge from the
//! module to the mock with a condition (`<runner>:manual_mock <mock> registered
//! by <file>`): [`lazy`] runs first and decides which tests it holds for; the
//! walks below follow it for those tests only.
//!
//! # The cut
//!
//! For an entry `T` selected by a change, with replaced modules `R`:
//!
//! 1. No cut when `T` reaches a file with a hazard, when its runner is not one
//!    that transpiles file by file ([`Runner::Isolated`]: a type check or a
//!    whole-program transpile reads the replaced module's types), or when `T`
//!    itself is a seed.
//! 2. A module of `R` that a file `T` reaches loads for real (`mock:actual`) is
//!    not cut.
//! 3. **Before the mocks.** What the runner loads before the test file
//!    registers its mocks stays loaded: the closure of `T`'s framework edges
//!    for the config, the setup files, the environment and the transform is
//!    walked without a cut, and a module of `R` in it is not cut (a setup file
//!    that imported the module, or something that imports it, keeps the real
//!    one). The targets of `requireActual`, of an automock and of a manual mock
//!    are loaded by the test file: their own imports see the mocks.
//! 4. **The test's own imports** are walked forward; a module-loading edge
//!    (`import`, `require`, `import()`, `export from`) into a module that is
//!    still cut is not followed. Other edges into it (a file read of its
//!    source) are. In vitest a plain `require` is Node's and is followed; in
//!    jest an importer that is an ES module file (`.mjs`, `.mts`) is not
//!    intercepted; a request with a query names another module; and a module
//!    the mock names by a bare request is only cut for imports by a bare
//!    request (the runner resolves a package through its manifest and may pick
//!    another file than the tracer, which a relative import of the traced file
//!    would then load).
//! 5. `T` stays selected when either walk reaches a seed of the change set.
//!    Otherwise the change reaches `T` only through replaced modules: it is not
//!    selected, and [`MockSkip`] names the module and the mock for `--explain`.
//!
//! Both walks stay inside the reverse closure of the seeds, so their cost is
//! bounded by the part of the graph the change reaches.

use std::collections::HashMap;

use super::{Barrels, PruneReport, Runner, bfs, entry_runner};
use crate::{
    change::{Cause, EntryResult},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId},
    vocab::{EdgeAttr, EdgeFilter, RefKind},
};

pub mod lazy;

pub use lazy::Unregistered;

/// The prefix of a marker edge's specifier.
pub const MARK: &str = "mock:";

/// Tags of the store's derived layer ([`GraphView::tagged`]), which looks at
/// every node's markers once per graph, so that a query does not: this one on
/// some node says that the layer ran.
pub const TAG_INDEX: &str = "mock:index";
/// The files with a `mock:hazard` marker (see [`TAG_INDEX`]). The files with a
/// `mock:actual <module>` marker carry that specifier as a tag.
pub const TAG_HAZARD: &str = "mock:hazard";
/// The modules with an edge to a manual mock that another file registers (see
/// [`TAG_INDEX`]).
pub const TAG_MODULE: &str = "mock:module";

/// A change that did not select an entry because the entry replaces the module
/// it comes through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockSkip {
    /// The entry.
    pub entry: NodeId,
    /// The replaced module.
    pub module: NodeId,
    /// The file whose load of it returns the mock.
    pub importer: NodeId,
    /// Where the mock is declared: `<file>:<line>`.
    pub at: String,
}

/// What [`apply`] did with the change-selected entries that replace a module
/// (counts of entries; an entry is counted at the first reason that keeps it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MockStats {
    /// Entries with a `mock:replaces` marker.
    pub entries: usize,
    /// Kept: the entry, its project or a setup file has a hazard.
    pub own_hazard: usize,
    /// Kept: no replaced module is reached by the change.
    pub unrelated: usize,
    /// Kept: the entry reaches a file with a hazard.
    pub hazard: usize,
    /// Kept: every replaced module the change reaches is loaded for real by a
    /// file the entry reaches.
    pub real: usize,
    /// Kept: the runner does not transpile file by file.
    pub runner: usize,
    /// Kept: the change reaches what the runner loads before the mocks.
    pub before: usize,
    /// Kept: the change reaches the entry through an import no mock replaces.
    pub other_path: usize,
    /// Not selected.
    pub freed: usize,
    /// Not selected, of any test: the change reaches it only through manual
    /// mocks that no file it loads registers ([`lazy`]).
    pub unregistered: usize,
}

impl MockStats {
    /// Adds the counts of another pass.
    pub fn add(&mut self, o: &MockStats) {
        self.entries += o.entries;
        self.own_hazard += o.own_hazard;
        self.unrelated += o.unrelated;
        self.hazard += o.hazard;
        self.real += o.real;
        self.runner += o.runner;
        self.before += o.before;
        self.other_path += o.other_path;
        self.freed += o.freed;
        self.unregistered += o.unregistered;
    }
}

/// A marker of a node (see the module documentation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marker<'a> {
    /// `mock:replaces <runner> <path|name> <module> @ <file>:<line>`.
    Replaces {
        /// `jest` or `vitest`.
        runner: &'a str,
        /// The mock names the module by path (else by a bare request).
        by_path: bool,
        /// The module's path.
        module: &'a str,
        /// `<file>:<line>` of the call.
        at: &'a str,
    },
    /// `mock:actual <module>`.
    Actual(&'a str),
    /// `mock:hazard <what> @ <file>:<line>`.
    Hazard(&'a str),
}

impl<'a> Marker<'a> {
    /// Parses the specifier of an unresolved edge.
    pub fn parse(spec: &'a str) -> Option<Marker<'a>> {
        let rest = spec.strip_prefix(MARK)?;
        let (what, rest) = rest.split_once(' ')?;
        match what {
            "replaces" => {
                let (runner, rest) = rest.split_once(' ')?;
                let (how, rest) = rest.split_once(' ')?;
                let (module, at) = rest.rsplit_once(" @ ")?;
                Some(Marker::Replaces {
                    runner,
                    by_path: how == "path",
                    module,
                    at,
                })
            }
            "actual" => Some(Marker::Actual(rest)),
            "hazard" => Some(Marker::Hazard(rest)),
            _ => None,
        }
    }
}

/// The markers of `n`.
pub fn markers<G: GraphView>(g: &G, n: NodeId) -> impl Iterator<Item = Marker<'_>> + '_ {
    g.out_edges(n)
        .filter(|e| e.to.is_none())
        .filter_map(|e| Marker::parse(e.specifier?))
}

/// Whether the specifier is the label of an edge the framework layer added for
/// something the runner loads before the test file registers its mocks: the
/// config, setup files, the environment, the transform. Not: an import of the
/// file resolved through an alias or a haste mock, and what the test file
/// itself asks the runner for (`requireActual`, the source of an automock, a
/// manual mock), whose own imports go through the registry with the mocks in
/// place.
fn before_mocks(spec: &str) -> bool {
    let Some((fw, rest)) = spec.split_once(':') else {
        return false;
    };
    if !matches!(fw, "jest" | "vitest") || rest.contains(" -> ") {
        return false;
    }
    !matches!(
        rest.split(' ').next().unwrap_or(rest),
        "require_actual" | "automocked" | "manual_mock" | "haste_mock" | "snapshot"
    )
}

/// Whether the specifier is a label of the framework layer (not a request as
/// written).
fn labelled(spec: &str) -> bool {
    spec.split_once(':')
        .is_some_and(|(fw, _)| matches!(fw, "jest" | "vitest"))
}

fn loads_module(attr: EdgeAttr) -> bool {
    matches!(
        attr.kind(),
        RefKind::Import | RefKind::Require | RefKind::DynamicImport | RefKind::ReExport
    )
}

/// Files with a hazard, and `(file, module)` for every real load.
#[derive(Default)]
struct Index {
    hazards: Vec<NodeId>,
    actual: Vec<(NodeId, NodeId)>,
    /// The view has the tags of the derived layer: `actual` is empty, and the
    /// files that load a module for real are the nodes tagged `mock:actual
    /// <module>`.
    tagged: bool,
}

impl Index {
    /// The files that load `module` for real.
    fn loading<G: GraphView>(&self, g: &G, module: NodeId) -> Vec<NodeId> {
        if self.tagged {
            return g
                .tagged(&format!("{MARK}actual {}", g.path(module)))
                .unwrap_or_default()
                .iter()
                .map(|&n| NodeId(n))
                .collect();
        }
        self.actual
            .iter()
            .filter(|a| a.1 == module)
            .map(|a| a.0)
            .collect()
    }
}

fn index<G: GraphView>(g: &G) -> Index {
    let mut ix = Index::default();
    // From the derived layer's tags (the real loads are asked for per module,
    // below), else from every node.
    if let Some(hazards) = g.tagged(TAG_HAZARD) {
        ix.hazards = hazards.iter().map(|&n| NodeId(n)).collect();
        ix.tagged = true;
        return ix;
    }
    for i in 0..g.node_count() {
        let n = NodeId(i);
        for m in markers(g, n) {
            match m {
                Marker::Hazard(_) => {
                    if ix.hazards.last() != Some(&n) {
                        ix.hazards.push(n);
                    }
                }
                Marker::Actual(p) => {
                    if let Some(m) = g.node_by_path(p) {
                        ix.actual.push((n, m));
                    }
                }
                Marker::Replaces { .. } => {}
            }
        }
    }
    ix
}

struct Candidate {
    /// Index into the results.
    result: usize,
    entry: NodeId,
    /// `(module, at, jest, by path)`.
    replaced: Vec<(NodeId, String, bool, bool)>,
}

/// Takes out of `results` the change-driven entries that reach every seed of
/// the change set only through manual mocks that no file they load registers
/// ([`lazy`], [`PruneReport::unregistered`]) or through modules they replace
/// with a mock (see the module documentation, [`PruneReport::mocked`]). `seeds`
/// are the seed nodes of the pass (deferred candidates included), `causes` the
/// changed ones with what changed, `filter` the pass's traversal filter.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply<G: GraphView>(
    engine: &Engine<'_, G>,
    b: &Barrels<'_>,
    filter: EdgeFilter,
    seeds: &[NodeId],
    causes: &[(NodeId, Cause)],
    results: &mut Vec<EntryResult>,
    is_affected: &mut NodeSet,
    pruned: &mut PruneReport,
) {
    if !results.iter().any(|r| r.uncertain.is_none()) {
        return;
    }
    let g = engine.graph();
    // `TURBO_TRACE_DEBUG_MOCKS_TIMING=1`: where the pass spends its time (stderr).
    let timing =
        turborepo_trace_core::debug::var_os("MOCKS_TIMING").map(|_| std::time::Instant::now());
    let lap = |what: &str| {
        if let Some(t) = timing {
            eprintln!(
                "mocks pass: {what} at {:.1} ms",
                t.elapsed().as_secs_f64() * 1e3
            );
        }
    };
    // Only what the change reaches matters to any of the walks.
    let order = bfs(engine, filter, seeds.iter().copied()).1;
    let reached = NodeSet::from_iter_in(g.node_count(), order.iter().copied());
    lap("what the change reaches");
    let before = pruned.unregistered.len();
    let conditions = lazy::apply(
        engine,
        filter,
        seeds,
        causes,
        &order,
        &reached,
        results,
        is_affected,
        pruned,
    );
    pruned.mock_stats.unregistered += pruned.unregistered.len() - before;
    drop(order);
    lap("mocks other files register");
    let follows = |entry: NodeId, from: NodeId, to: NodeId| {
        conditions
            .as_ref()
            .is_none_or(|c| c.follows(entry, from, to))
    };
    let targets = NodeSet::from_iter_in(g.node_count(), seeds.iter().copied());
    let mut cands: Vec<Candidate> = Vec::new();
    for (i, r) in results.iter().enumerate() {
        if r.uncertain.is_some() || targets.contains(r.entry) {
            continue;
        }
        let mut replaced = Vec::new();
        let mut hazard = false;
        for m in markers(g, r.entry) {
            match m {
                Marker::Replaces {
                    runner,
                    by_path,
                    module,
                    at,
                } => {
                    // A manifest is never a module (the trace side does not name one).
                    if let Some(n) = g.node_by_path(module)
                        && !module.ends_with("package.json")
                    {
                        replaced.push((n, at.to_owned(), runner == "jest", by_path));
                    }
                }
                Marker::Hazard(_) => hazard = true,
                Marker::Actual(_) => {}
            }
        }
        if replaced.is_empty() {
            continue;
        }
        pruned.mock_stats.entries += 1;
        if hazard {
            pruned.mock_stats.own_hazard += 1;
        } else {
            cands.push(Candidate {
                result: i,
                entry: r.entry,
                replaced,
            });
        }
    }
    if cands.is_empty() {
        return;
    }
    let st = &mut pruned.mock_stats;
    cands.retain_mut(|c| {
        c.replaced.retain(|r| reached.contains(r.0));
        !c.replaced.is_empty()
    });
    st.unrelated += st.entries - st.own_hazard - cands.len();
    if cands.is_empty() {
        return;
    }
    let mut left = cands.len();
    // Hazards and real loads in the files an entry reaches (over every edge the
    // engine follows: a superset of what the test loads).
    let all = engine.options().traversal;
    lap("candidates");
    let ix = index(g);
    lap("index of hazards and real loads");
    if !ix.hazards.is_empty() {
        let tainted = NodeSet::from_iter_in(
            g.node_count(),
            bfs(engine, all, ix.hazards.iter().copied()).1,
        );
        cands.retain(|c| !tainted.contains(c.entry));
        st.hazard += left - cands.len();
        left = cands.len();
    }
    let mut real: HashMap<NodeId, Option<NodeSet>> = HashMap::new();
    for c in &mut cands {
        c.replaced.retain(|r| {
            real.entry(r.0)
                .or_insert_with(|| {
                    let files = ix.loading(g, r.0);
                    (!files.is_empty())
                        .then(|| NodeSet::from_iter_in(g.node_count(), bfs(engine, all, files).1))
                })
                .as_ref()
                .is_none_or(|s| !s.contains(c.entry))
        });
    }
    cands.retain(|c| !c.replaced.is_empty());
    st.real += left - cands.len();
    left = cands.len();
    cands.retain(|c| entry_runner(g, b.source, c.entry) == Runner::Isolated);
    st.runner += left - cands.len();
    lap("hazards, real loads, runners");
    if cands.is_empty() {
        return;
    }

    let fwd = engine.forward();
    let n = g.node_count() as usize;
    // Generation stamps: visited nodes and cut modules of the current candidate.
    let mut seen = vec![0u32; n];
    let mut cut = vec![0u32; n];
    // Cut modules that a mock names by path (every load of the file gets the mock).
    let mut pathed = vec![0u32; n];
    let mut stack: Vec<NodeId> = Vec::new();
    let mut freed: Vec<(usize, MockSkip)> = Vec::new();
    for (generation, c) in (1u32..).zip(&cands) {
        let mut before: Vec<NodeId> = Vec::new();
        let mut own: Vec<NodeId> = Vec::new();
        for e in g.out_edges(c.entry) {
            let Some(to) = e.to else { continue };
            if !filter.accepts(e.attr) || !reached.contains(to) || !follows(c.entry, c.entry, to) {
                continue;
            }
            let spec = e.specifier.unwrap_or("");
            if before_mocks(spec) {
                before.push(to);
            } else {
                own.push(to);
            }
        }
        seen[c.entry.index()] = generation;
        let mut selected = false;
        // Before the mocks: no cut.
        stack.clear();
        for &t in &before {
            if seen[t.index()] != generation {
                seen[t.index()] = generation;
                stack.push(t);
            }
        }
        while let Some(x) = stack.pop() {
            if targets.contains(x) {
                selected = true;
                break;
            }
            let (tos, attrs) = fwd.neighbors(x);
            for (&t, &a) in tos.iter().zip(attrs) {
                if filter.accepts(a)
                    && reached.contains(NodeId(t))
                    && seen[t as usize] != generation
                    && follows(c.entry, x, NodeId(t))
                {
                    seen[t as usize] = generation;
                    stack.push(NodeId(t));
                }
            }
        }
        if selected {
            st.before += 1;
            continue;
        }
        let mut any = false;
        for r in &c.replaced {
            if seen[r.0.index()] != generation {
                cut[r.0.index()] = generation;
                if r.3 {
                    pathed[r.0.index()] = generation;
                }
                any = true;
            }
        }
        if !any {
            st.before += 1;
            continue;
        }
        let jest = c.replaced.iter().all(|r| r.2);
        // Whether the load `from → to` returns the mock.
        // Every edge of `from` to the cut module must be a load the runner answers with
        // the mock.
        let intercepted = |from: NodeId, to: NodeId| {
            if jest {
                // An ES module file is not loaded through `require`.
                let p = g.path(from);
                if p.ends_with(".mjs") || p.ends_with(".mts") {
                    return false;
                }
            }
            let mut any = false;
            for e in g.out_edges(from) {
                if e.to != Some(to) || !filter.accepts(e.attr) {
                    continue;
                }
                let spec = e.specifier.unwrap_or("");
                // Not a load (a read of the source); a request with a query, which names
                // another module (vite's `?raw`, `?url`); in vitest Node's own `require`
                // (an import resolved through an alias is a labelled edge of that kind);
                // an import by path of a module the mock names by a bare request.
                if !loads_module(e.attr)
                    || spec.contains('?')
                    || (!jest && e.kind() == RefKind::Require && !labelled(spec))
                    || (pathed[to.index()] != generation
                        && (spec.starts_with("./")
                            || spec.starts_with("../")
                            || spec.starts_with('/')))
                {
                    return false;
                }
                any = true;
            }
            any
        };
        // The test's own imports.
        let mut skip: Option<(NodeId, NodeId)> = None;
        stack.clear();
        for &t in &own {
            if cut[t.index()] == generation && intercepted(c.entry, t) {
                skip.get_or_insert((c.entry, t));
            } else if seen[t.index()] != generation {
                seen[t.index()] = generation;
                stack.push(t);
            }
        }
        while let Some(x) = stack.pop() {
            if targets.contains(x) {
                selected = true;
                break;
            }
            let (tos, attrs) = fwd.neighbors(x);
            for (&t, &a) in tos.iter().zip(attrs) {
                let to = NodeId(t);
                if !filter.accepts(a)
                    || !reached.contains(to)
                    || seen[t as usize] == generation
                    || !follows(c.entry, x, to)
                {
                    continue;
                }
                if cut[t as usize] == generation && intercepted(x, to) {
                    skip.get_or_insert((x, to));
                    continue;
                }
                seen[t as usize] = generation;
                stack.push(to);
            }
        }
        if selected {
            st.other_path += 1;
            continue;
        }
        // Nothing was cut on the way (the change reaches the entry through edges the
        // walks do not follow, such as a deferred candidate's own rule): keep it.
        let Some((importer, module)) = skip else {
            st.other_path += 1;
            continue;
        };
        let at = c
            .replaced
            .iter()
            .find(|r| r.0 == module)
            .map(|r| r.1.clone())
            .unwrap_or_default();
        freed.push((
            c.result,
            MockSkip {
                entry: c.entry,
                module,
                importer,
                at,
            },
        ));
    }
    pruned.mock_stats.freed += freed.len();
    lap("walks per entry");
    if freed.is_empty() {
        return;
    }
    let mut drop = vec![false; results.len()];
    for (i, m) in freed {
        drop[i] = true;
        is_affected.remove(m.entry);
        pruned.mocked.push(m);
    }
    let mut i = 0;
    results.retain(|_| {
        i += 1;
        !drop[i - 1]
    });
}

#[cfg(test)]
mod tests {
    use super::{super::tests::Fx, *};
    use crate::{
        change::{AffectedOptions, ChangeKind, ChangeSet, Policy},
        mem::EdgeSpec,
    };

    /// `t.test.ts → user.ts → m.ts → deep.ts`, `t.test.ts` a jest test
    /// (babel-jest).
    fn fixture() -> Fx {
        let mut f = Fx::new();
        f.import("t.test.ts", "user.ts", None);
        f.import("user.ts", "m.ts", None);
        f.import("m.ts", "deep.ts", None);
        f.jest("t.test.ts", "module.exports = {};");
        f
    }

    fn mark(f: &mut Fx, on: &str, spec: &str) {
        let n = f.node(on);
        f.b.add_edge(n, EdgeSpec::new(None, RefKind::Manifest).specifier(spec));
    }

    /// The entries a modification of `changed` selects, and what `--explain`
    /// says for `t.test.ts`.
    fn run(f: Fx, changed: &str, mocks: bool) -> (Vec<String>, String) {
        let (g, ids, src) = f.build();
        let engine = Engine::new(&g);
        let mut entries = NodeSet::new(g.node_count());
        for (p, &n) in &ids {
            if p.ends_with(".test.ts") {
                entries.insert(n);
            }
        }
        let mut cs = ChangeSet::new();
        cs.push(ChangeKind::Modified, changed);
        let mut b = Barrels::new(Vec::new(), &src);
        b.runtime_entries = true;
        b.mocks = mocks;
        let (report, pruned) = engine.affected_with(
            &cs,
            &entries,
            &AffectedOptions::default().policy(Policy::Affected),
            Some(&b),
        );
        let mut why = Vec::new();
        pruned
            .write_reasons(&engine, ids["t.test.ts"], &mut why)
            .unwrap();
        (
            report
                .entries
                .iter()
                .map(|e| g.path(e.entry).to_owned())
                .collect(),
            String::from_utf8(why).unwrap(),
        )
    }

    const REPLACES: &str = "mock:replaces jest path m.ts @ t.test.ts:3";

    #[test]
    fn a_replaced_module_and_what_only_it_reaches_do_not_select() {
        for changed in ["m.ts", "deep.ts"] {
            let mut f = fixture();
            mark(&mut f, "t.test.ts", REPLACES);
            let (sel, why) = run(f, changed, true);
            assert!(sel.is_empty(), "{changed}: {sel:?}");
            assert_eq!(
                why,
                "  not selected through m.ts: replaced by the mock at t.test.ts:3 (loaded by \
                 user.ts)\n"
            );
            // Off: selected.
            let mut f = fixture();
            mark(&mut f, "t.test.ts", REPLACES);
            assert_eq!(run(f, changed, false).0, ["t.test.ts"]);
        }
        // What the test loads in front of the module still selects it.
        for changed in ["user.ts", "t.test.ts"] {
            let mut f = fixture();
            mark(&mut f, "t.test.ts", REPLACES);
            assert_eq!(run(f, changed, true).0, ["t.test.ts"], "{changed}");
        }
        // Without the marker nothing is cut.
        assert_eq!(run(fixture(), "deep.ts", true).0, ["t.test.ts"]);
    }

    #[test]
    fn another_path_keeps_the_test_selected() {
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        f.import("t.test.ts", "other.ts", None);
        f.import("other.ts", "deep.ts", None);
        assert_eq!(run(f, "deep.ts", true).0, ["t.test.ts"]);
        // ... but not for the module itself.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        f.import("t.test.ts", "other.ts", None);
        f.import("other.ts", "deep.ts", None);
        assert!(run(f, "m.ts", true).0.is_empty());
    }

    #[test]
    fn what_the_runner_loads_before_the_mocks_is_not_cut() {
        // A setup file that imports the module (or what imports it).
        for via in ["m.ts", "user.ts"] {
            let mut f = fixture();
            mark(&mut f, "t.test.ts", REPLACES);
            f.edge(
                "t.test.ts",
                "setup.ts",
                RefKind::Require,
                "jest:setup_file ./setup.ts",
                900,
            );
            f.import("setup.ts", via, None);
            assert_eq!(
                run(f, "deep.ts", true).0,
                ["t.test.ts"],
                "setup imports {via}"
            );
        }
        // `requireActual` of the module in the test.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        mark(&mut f, "t.test.ts", "mock:actual m.ts");
        f.edge(
            "t.test.ts",
            "m.ts",
            RefKind::Require,
            "jest:require_actual ./m",
            900,
        );
        assert_eq!(run(f, "deep.ts", true).0, ["t.test.ts"]);
        // `requireActual` of something else: its imports still get the mock.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        mark(&mut f, "t.test.ts", "mock:actual user.ts");
        f.edge(
            "t.test.ts",
            "user.ts",
            RefKind::Require,
            "jest:require_actual ./user",
            900,
        );
        assert!(run(f, "deep.ts", true).0.is_empty());
        // An import through an alias is an import: cut.
        let mut f = Fx::new();
        f.import("t.test.ts", "user.ts", None);
        f.edge(
            "user.ts",
            "m.ts",
            RefKind::Require,
            "jest:moduleNameMapper @/m -> ./m",
            0,
        );
        f.jest("t.test.ts", "module.exports = {};");
        mark(&mut f, "t.test.ts", REPLACES);
        assert!(run(f, "m.ts", true).0.is_empty());
    }

    #[test]
    fn real_loads_and_hazards_in_the_files_the_test_reaches() {
        // A helper loads the real module.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        f.import("t.test.ts", "helper.ts", None);
        mark(&mut f, "helper.ts", "mock:actual m.ts");
        assert_eq!(run(f, "deep.ts", true).0, ["t.test.ts"]);
        // A helper nobody of this test loads does not count.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        mark(&mut f, "elsewhere.ts", "mock:actual m.ts");
        assert!(run(f, "deep.ts", true).0.is_empty());
        // A hazard in a helper, or on the test (its project).
        for on in ["helper.ts", "t.test.ts"] {
            let mut f = fixture();
            mark(&mut f, "t.test.ts", REPLACES);
            f.import("t.test.ts", "helper.ts", None);
            mark(
                &mut f,
                on,
                "mock:hazard jest.unmock(...) of a computed module @ helper.ts:4",
            );
            assert_eq!(run(f, "deep.ts", true).0, ["t.test.ts"], "hazard on {on}");
        }
    }

    #[test]
    fn only_loads_are_intercepted_and_only_by_a_runner_that_transpiles() {
        // A file read of the module's source is not a load.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        f.edge("user.ts", "m.ts", RefKind::FsRead, "./m.ts", 500);
        assert_eq!(run(f, "m.ts", true).0, ["t.test.ts"]);
        // vitest: a plain `require` is Node's.
        let vitest = |kind: RefKind| {
            let mut f = Fx::new();
            f.import("t.test.ts", "user.ts", None);
            f.edge("user.ts", "m.ts", kind, "./m", 0);
            f.edge(
                "t.test.ts",
                "vitest.config.ts",
                RefKind::Require,
                "vitest:config vitest.config.ts",
                0,
            );
            f.src
                .texts
                .insert("vitest.config.ts".into(), "export default {};".into());
            mark(
                &mut f,
                "t.test.ts",
                "mock:replaces vitest path m.ts @ t.test.ts:3",
            );
            run(f, "m.ts", true).0
        };
        assert!(vitest(RefKind::Import).is_empty());
        assert_eq!(vitest(RefKind::Require), ["t.test.ts"]);
        // jest: an ES module importer is not intercepted.
        let mut f = Fx::new();
        f.import("t.test.ts", "user.mjs", None);
        f.import("user.mjs", "m.ts", None);
        f.jest("t.test.ts", "module.exports = {};");
        mark(&mut f, "t.test.ts", REPLACES);
        assert_eq!(run(f, "m.ts", true).0, ["t.test.ts"]);
        // A module the mock names by a bare request: imports by a bare request (also
        // through an alias) get the mock, an import by path may load another file.
        let named = |spec: &str| {
            let mut f = Fx::new();
            f.import("t.test.ts", "user.ts", None);
            f.edge("user.ts", "lib/index.ts", RefKind::Import, spec, 0);
            f.jest("t.test.ts", "module.exports = {};");
            mark(
                &mut f,
                "t.test.ts",
                "mock:replaces jest name lib/index.ts @ t.test.ts:3",
            );
            run(f, "lib/index.ts", true).0
        };
        assert!(named("@acme/lib").is_empty());
        assert!(named("jest:moduleNameMapper @acme/lib -> ./lib").is_empty());
        assert_eq!(named("./lib/index"), ["t.test.ts"]);
        // ts-jest with type checks reads the module's types.
        let mut f = fixture();
        mark(&mut f, "t.test.ts", REPLACES);
        f.edge(
            "t.test.ts",
            "ts-jest",
            RefKind::Require,
            "jest:transform ts-jest",
            950,
        );
        assert_eq!(run(f, "m.ts", true).0, ["t.test.ts"]);
    }

    const REGISTERED: &str = "jest:manual_mock __mocks__/n.ts registered by helper.ts";

    /// `user.ts → n.ts`, and `helper.ts` registers `__mocks__/n.ts` as the mock
    /// of `n.ts` without a factory (the derived edge `n.ts →
    /// __mocks__/n.ts`); the mock imports `mockdep.ts`. The jest test
    /// `t.test.ts` imports `imports`.
    fn lazy_fixture(imports: &[&str]) -> Fx {
        let mut f = Fx::new();
        for i in imports {
            f.import("t.test.ts", i, None);
        }
        f.import("user.ts", "n.ts", None);
        f.edge("n.ts", "__mocks__/n.ts", RefKind::Require, REGISTERED, 900);
        f.import("__mocks__/n.ts", "mockdep.ts", None);
        f.node("helper.ts");
        f.jest("t.test.ts", "module.exports = {};");
        f
    }

    #[test]
    fn a_manual_mock_counts_for_tests_that_load_a_file_that_registers_it() {
        // The test requests `n.ts` and loads no file that registers the mock: it gets
        // the real module.
        for changed in ["__mocks__/n.ts", "mockdep.ts"] {
            let (sel, why) = run(lazy_fixture(&["user.ts"]), changed, true);
            assert!(sel.is_empty(), "{changed}: {sel:?}");
            assert_eq!(
                why,
                "  not selected through __mocks__/n.ts: the mock of n.ts for tests that load \
                 helper.ts, which the test does not\n"
            );
            // Off: an edge like any other.
            assert_eq!(
                run(lazy_fixture(&["user.ts"]), changed, false).0,
                ["t.test.ts"]
            );
        }
        // The module itself still selects the test.
        assert_eq!(
            run(lazy_fixture(&["user.ts"]), "n.ts", true).0,
            ["t.test.ts"]
        );
        // It loads the helper too: the mock runs.
        assert_eq!(
            run(lazy_fixture(&["user.ts", "helper.ts"]), "mockdep.ts", true).0,
            ["t.test.ts"]
        );
        // The helper alone, with nothing that requests the module: no path at all.
        assert!(
            run(lazy_fixture(&["helper.ts"]), "mockdep.ts", false)
                .0
                .is_empty()
        );
        // Another path to the change; an import of the mock file itself.
        for (from, to) in [("t.test.ts", "mockdep.ts"), ("n.ts", "__mocks__/n.ts")] {
            let mut f = lazy_fixture(&["user.ts"]);
            f.import(from, to, None);
            assert_eq!(
                run(f, "mockdep.ts", true).0,
                ["t.test.ts"],
                "{from} -> {to}"
            );
        }
        // Two files register it: one of them is enough.
        let second = "jest:manual_mock __mocks__/n.ts registered by other.ts";
        let mut f = lazy_fixture(&["user.ts", "other.ts"]);
        f.edge("n.ts", "__mocks__/n.ts", RefKind::Require, second, 910);
        assert_eq!(run(f, "mockdep.ts", true).0, ["t.test.ts"]);
        let mut f = lazy_fixture(&["user.ts"]);
        f.edge("n.ts", "__mocks__/n.ts", RefKind::Require, second, 910);
        f.node("other.ts");
        let (sel, why) = run(f, "mockdep.ts", true);
        assert!(sel.is_empty());
        assert!(
            why.contains("(or one of 1 other file(s) that register it)"),
            "{why}"
        );
        // A registering file that is not in the graph: no condition to check.
        let mut f = Fx::new();
        f.import("t.test.ts", "n.ts", None);
        f.edge(
            "n.ts",
            "__mocks__/n.ts",
            RefKind::Require,
            "jest:manual_mock __mocks__/n.ts registered by gone.ts",
            900,
        );
        f.jest("t.test.ts", "module.exports = {};");
        assert_eq!(run(f, "__mocks__/n.ts", true).0, ["t.test.ts"]);
    }

    #[test]
    fn a_test_selected_by_another_path_gets_that_chain() {
        // `t.test.ts → n.ts → __mocks__/n.ts → mockdep.ts` is the shortest chain, and
        // the test does not load the helper; it is selected through `long.ts →
        // far.ts → mockdep.ts`.
        let mut f = Fx::new();
        f.import("t.test.ts", "n.ts", None);
        f.edge("n.ts", "__mocks__/n.ts", RefKind::Require, REGISTERED, 900);
        f.import("__mocks__/n.ts", "mockdep.ts", None);
        f.import("t.test.ts", "long.ts", None);
        f.import("long.ts", "far.ts", None);
        f.import("far.ts", "mockdep.ts", None);
        f.node("helper.ts");
        f.jest("t.test.ts", "module.exports = {};");
        let chain = |f: Fx, mocks: bool| {
            let (g, ids, src) = f.build();
            let engine = Engine::new(&g);
            let entries = NodeSet::from_iter_in(g.node_count(), [ids["t.test.ts"]]);
            let mut cs = ChangeSet::new();
            cs.push(ChangeKind::Modified, "mockdep.ts");
            let mut b = Barrels::new(Vec::new(), &src);
            b.mocks = mocks;
            let (report, _) = engine.affected_with(
                &cs,
                &entries,
                &AffectedOptions::default().policy(Policy::Affected),
                Some(&b),
            );
            report.entries[0]
                .witness
                .iter()
                .map(|&n| g.path(n).to_owned())
                .collect::<Vec<_>>()
        };
        let mut with_helper = Fx::new();
        with_helper.import("t.test.ts", "n.ts", None);
        with_helper.edge("n.ts", "__mocks__/n.ts", RefKind::Require, REGISTERED, 900);
        with_helper.import("__mocks__/n.ts", "mockdep.ts", None);
        with_helper.import("t.test.ts", "helper.ts", None);
        with_helper.jest("t.test.ts", "module.exports = {};");
        assert_eq!(
            chain(with_helper, true),
            ["mockdep.ts", "__mocks__/n.ts", "n.ts", "t.test.ts"]
        );
        assert_eq!(
            chain(f, true),
            ["mockdep.ts", "far.ts", "long.ts", "t.test.ts"]
        );
    }

    #[test]
    fn a_replaced_module_and_an_unregistered_mock_together() {
        // The change reaches the test through a module it replaces and through a manual
        // mock it does not get: neither path counts.
        let with = |helper: bool| {
            let mut f = fixture();
            mark(&mut f, "t.test.ts", REPLACES);
            f.import("t.test.ts", "n.ts", None);
            f.edge("n.ts", "__mocks__/n.ts", RefKind::Require, REGISTERED, 900);
            f.import("__mocks__/n.ts", "deep.ts", None);
            f.node("helper.ts");
            if helper {
                f.import("t.test.ts", "helper.ts", None);
            }
            run(f, "deep.ts", true).0
        };
        assert!(with(false).is_empty());
        assert_eq!(with(true), ["t.test.ts"]);
    }

    #[test]
    fn conditional_edges_parse() {
        assert_eq!(
            lazy::parse("jest:manual_mock src/__mocks__/a b.ts registered by test/setup a.ts"),
            Some(("src/__mocks__/a b.ts", "test/setup a.ts"))
        );
        assert_eq!(
            lazy::parse("vitest:manual_mock __mocks__/n.ts registered by helper.ts"),
            Some(("__mocks__/n.ts", "helper.ts"))
        );
        // A test's own manual mock, other labels, other layers.
        assert_eq!(lazy::parse("jest:manual_mock src/__mocks__/a.ts"), None);
        assert_eq!(
            lazy::parse("jest:require_actual a.ts registered by b.ts"),
            None
        );
        assert_eq!(lazy::parse("./manual_mock a registered by b"), None);
    }

    #[test]
    fn markers_parse() {
        assert_eq!(
            Marker::parse("mock:replaces vitest name src/a b.ts @ src/a.test.ts:12"),
            Some(Marker::Replaces {
                runner: "vitest",
                by_path: false,
                module: "src/a b.ts",
                at: "src/a.test.ts:12"
            })
        );
        assert_eq!(
            Marker::parse("mock:actual src/m.ts"),
            Some(Marker::Actual("src/m.ts"))
        );
        assert!(matches!(
            Marker::parse("mock:hazard x @ a.ts:1"),
            Some(Marker::Hazard(_))
        ));
        assert_eq!(Marker::parse("jest:config jest.config.js"), None);
    }
}
