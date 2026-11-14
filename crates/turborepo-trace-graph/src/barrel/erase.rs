//! Erased-equal changes in `affected`.
//!
//! A modified file `f` is *erased-equal* when the JavaScript a transpiler emits
//! for it is the same at the base and at the head revision: only type
//! annotations, interfaces, type aliases, `import type` / `export type`,
//! comments or formatting changed. The caller decides that
//! ([`super::Barrels::erased`]; the CLI compares both versions after erasing
//! types with a real TypeScript transform, see `turborepo-trace`'s `erase`
//! module).
//!
//! **Why skipping is sound.** A test runner that transpiles file by file
//! (`@swc/jest`, `babel-jest`, esbuild, `ts-jest` with `isolatedModules`,
//! vitest without `typecheck`: [`super::Runner::Isolated`]) turns each module
//! into JavaScript from that module's text and the transform configuration
//! alone. When that JavaScript is the same at both revisions, a test that only
//! *loads* `f` as a module executes the same code before and after the change,
//! so the change cannot alter its outcome.
//!
//! **What still counts.** The text of `f` did change, so everything that can
//! observe the text other than through the transpiler stays affected:
//!
//! 1. entries whose runner type-checks or sees the whole program
//!    ([`super::Runner::Checks`], [`super::Runner::Transpiles`]), build tasks,
//!    type-check tasks and every entry that is not a jest or vitest test;
//! 2. a modified entry itself (a changed test file always runs);
//! 3. anything else that seeds `f` (config inputs, probes, memberships, scopes
//!    of covered uncertainties, packages): the engine only defers a file
//!    nothing else seeds;
//! 4. readers of `f` or of a module that loads `f`, found by [`readers`]: let
//!    the *module region* be `f` plus every JS file that reaches `f` through
//!    plain module loads (imports, requires, dynamic imports, re-exports, and
//!    the test runner's own loads through the transform: setup files, manual
//!    mocks, `requireActual`, automocks, imports rewritten by
//!    `moduleNameMapper` or `resolve.alias`). An edge into the region that is
//!    not such a load makes its source a *reader*, and every entry that reaches
//!    a reader is affected:
//!    - data reads of `f` itself (`fs.readFileSync`, asset paths, directory
//!      listings, embeds, manifests): they see the changed text. Data reads of
//!      other region files don't count, because those files did not change;
//!    - loads by something that is not the test's transform pipeline: workers,
//!      child processes, native addons, the runner's config, global setup,
//!      environment, custom transformers and resolvers (jest may load them
//!      through `ts-node`, which type-checks), build tasks whose commands run a
//!      region file, edges without a recorded specifier, and edges from files
//!      that are not JS modules.

use std::collections::HashMap;

use crate::{
    engine::Engine,
    set::NodeSet,
    view::{Edge, GraphView, NodeId},
    vocab::{EdgeFilter, RefKind},
};

/// File extensions of JS modules a transpiling runner loads.
const JS_EXTS: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"];

fn is_js(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| JS_EXTS.contains(&ext))
}

/// Roles of framework edges (`<framework>:<role> <target>`) that the test
/// runner loads as modules of the test, through its transform: setup files,
/// mocks, and imports that a module mapper or alias rewrites
/// (`jest:moduleNameMapper <spec> -> <request>`).
const RUNNER_LOADS: &[&str] = &[
    "setup_file",
    "setup_file_after_env",
    "require_actual",
    "automocked",
    "manual_mock",
    "haste_mock",
    "moduleNameMapper",
    "resolve.alias",
];

/// How an edge reads its target.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Hop {
    /// Loads it as a module through the test's transform.
    Load,
    /// Reads its bytes.
    Data,
    /// Anything else.
    Other,
}

/// Classifies an edge of the JS or non-JS file at `from`.
pub(crate) fn hop(from: &str, e: &Edge<'_>) -> Hop {
    match e.kind() {
        RefKind::Import | RefKind::Require | RefKind::DynamicImport | RefKind::ReExport => {
            let Some(spec) = e.specifier else {
                return Hop::Other;
            };
            if !is_js(from) {
                return Hop::Other;
            }
            // Framework edges are labelled `<framework>:<role> <target>`; a plain module
            // specifier has no space.
            match spec.split_once(' ') {
                None => Hop::Load,
                Some((label, _)) => match label.split_once(':') {
                    Some(("jest" | "vitest", role)) if RUNNER_LOADS.contains(&role) => Hop::Load,
                    _ => Hop::Other,
                },
            }
        }
        RefKind::FsRead
        | RefKind::FsReadDir
        | RefKind::Asset
        | RefKind::Embed
        | RefKind::Manifest => Hop::Data,
        _ => Hop::Other,
    }
}

/// The readers of the erased-equal files `cands` (see the module documentation)
/// that some entry of `pending` reaches, each with a path from a candidate to
/// the reader (candidate first, reader last).
pub(crate) fn readers<G: GraphView>(
    engine: &Engine<'_, G>,
    filter: EdgeFilter,
    cands: &[NodeId],
    pending: &[NodeId],
) -> Vec<(NodeId, Vec<NodeId>)> {
    let g = engine.graph();
    let is_cand = NodeSet::from_iter_in(g.node_count(), cands.iter().copied());
    // Everything that reaches a candidate over any edge.
    let (_, order) = super::bfs(engine, filter, cands.iter().copied());
    let reach = NodeSet::from_iter_in(g.node_count(), order.iter().copied());
    // Of those, what the pending entries depend on: only these can stand between an
    // entry and a candidate, and only their edges are read below (the expensive
    // part).
    let fwd = engine.forward();
    let mut order: Vec<NodeId> = Vec::new();
    let mut seen = NodeSet::new(g.node_count());
    for &e in pending {
        if reach.contains(e) && seen.insert(e) {
            order.push(e);
        }
    }
    let mut i = 0;
    while i < order.len() {
        let n = order[i];
        i += 1;
        let (targets, attrs) = fwd.neighbors(n);
        for (&t, &a) in targets.iter().zip(attrs) {
            let t = NodeId(t);
            if filter.accepts(a) && reach.contains(t) && seen.insert(t) {
                order.push(t);
            }
        }
    }
    // Per target in `reach`: the sources that load it as a module, and the edges
    // that read it another way.
    let mut loaders: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut others: Vec<(NodeId, NodeId)> = Vec::new();
    let mut per: Vec<(NodeId, bool, bool)> = Vec::new();
    for &s in &order {
        per.clear();
        let from = g.path(s);
        for e in g.out_edges(s) {
            let Some(t) = e.to else { continue };
            if !reach.contains(t) || !filter.accepts(e.attr) {
                continue;
            }
            let (load, other) = match hop(from, &e) {
                Hop::Load => (true, false),
                // The bytes of an unchanged module are the same.
                Hop::Data => (false, is_cand.contains(t)),
                Hop::Other => (false, true),
            };
            match per.iter_mut().find(|p| p.0 == t) {
                Some(p) => {
                    p.1 |= load;
                    p.2 |= other;
                }
                None => per.push((t, load, other)),
            }
        }
        for &(t, load, other) in &per {
            if other {
                others.push((s, t));
            } else if load {
                loaders.entry(t).or_default().push(s);
            }
        }
    }
    // The module region, with the hop towards a candidate.
    let mut towards: HashMap<NodeId, NodeId> = HashMap::new();
    let mut region = is_cand.clone();
    let mut work: Vec<NodeId> = cands.to_vec();
    let mut i = 0;
    while i < work.len() {
        let n = work[i];
        i += 1;
        for &s in loaders.get(&n).map_or(&[][..], Vec::as_slice) {
            if region.insert(s) {
                towards.insert(s, n);
                work.push(s);
            }
        }
    }
    let mut out: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
    let mut seen = NodeSet::new(g.node_count());
    for (s, t) in others {
        if !region.contains(t) || !seen.insert(s) {
            continue;
        }
        let mut path = vec![s, t];
        let mut cur = t;
        while let Some(&next) = towards.get(&cur) {
            path.push(next);
            cur = next;
        }
        path.reverse();
        out.push((s, path));
    }
    out
}
