//! Manual mocks that other files register.
//!
//! `jest.mock('./db')` / `vi.mock('./db')` without a factory loads nothing. The
//! runner loads the manual mock (`__mocks__/db`) when `./db` is first requested
//! after the call (the runtime mock oracle: with nothing requesting the
//! module, neither the mock nor the module ran in any case of any runner). So
//! when a file that is not a test makes the call (a helper, a file a setup file
//! imports), a test runs the mock when it loads that file **and** something it
//! loads requests the module.
//!
//! The graph has the second half as an edge from the module to the mock, added
//! by the store's derived layer for every registering file:
//!
//! ```text
//! <module> → <mock>      <runner>:manual_mock <mock> registered by <file>
//! ```
//!
//! Everything that follows edges (the uncertainty step, names, queries, other
//! kinds of entries, `TURBO_TRACE_DEBUG_MOCKS=0`) takes it as it is: whatever
//! requests the module may get the mock. [`apply`] adds the first half for jest
//! and vitest tests: a test that reaches the change only through such edges,
//! and loads none of the files that register the mock, gets the real module
//! there and is not selected
//! ([`PruneReport::unregistered`](super::super::PruneReport::unregistered)).
//!
//! The decision is made on sets, not per test:
//!
//! 1. the conditional edges whose mock the change reaches are collected from
//!    the files the change reaches (none: nothing to do);
//! 2. a walk from the change that does not follow them gives the tests selected
//!    whatever the conditions say;
//! 3. for each module with such edges, the tests that reach it and one of the
//!    registering files stay selected; the remaining ones are not selected.
//!
//! Step 3 follows conditional edges on the way to the module and from the mock
//! to the change without checking their own conditions: a superset again.

use std::collections::HashMap;

use super::super::PruneReport;
use crate::{
    change::{Cause, EntryResult},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId},
    vocab::{EdgeFilter, RefKind},
};

/// A test a change did not select because it reaches the change only through a
/// manual mock that no file it loads registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unregistered {
    /// The entry.
    pub entry: NodeId,
    /// The manual mock.
    pub mock: NodeId,
    /// The module the mock stands in for, which the entry reaches.
    pub module: NodeId,
    /// A file that registers the mock.
    pub by: NodeId,
    /// How many other files register it.
    pub others: usize,
}

/// The mock and the registering file of a conditional edge's specifier
/// (`<runner>:manual_mock <mock> registered by <file>`).
pub fn parse(spec: &str) -> Option<(&str, &str)> {
    let (fw, rest) = spec.split_once(':')?;
    if !matches!(fw, "jest" | "vitest") {
        return None;
    }
    rest.strip_prefix("manual_mock ")?
        .rsplit_once(" registered by ")
}

/// Whether `n` is a jest or vitest test: it has an edge of the framework layer.
/// The condition of a conditional edge only holds back tests.
pub fn is_test<G: GraphView>(g: &G, n: NodeId) -> bool {
    g.out_edges(n).any(|e| {
        e.specifier
            .is_some_and(|s| s.starts_with("jest:config") || s.starts_with("vitest:config"))
    })
}

/// The conditional edges under a change, for the walks of [`super::apply`].
pub(crate) struct Conditions {
    /// `(module, mock)` → the module's index in `holds`, for the pairs all of
    /// whose followed edges are conditional.
    pairs: HashMap<(u32, u32), u32>,
    /// The mocks of `pairs` (a cheap test before the lookup).
    mocks: NodeSet,
    /// Per module: the nodes that reach it and one of the files that register
    /// its mocks.
    holds: Vec<NodeSet>,
}

impl Conditions {
    /// Whether the edge `from → to` is followed for `entry`.
    pub(crate) fn follows(&self, entry: NodeId, from: NodeId, to: NodeId) -> bool {
        if !self.mocks.contains(to) {
            return true;
        }
        match self.pairs.get(&(from.0, to.0)) {
            Some(&i) => self.holds[i as usize].contains(entry),
            None => true,
        }
    }
}

/// The nodes that reach `starts` over the edges `filter` accepts.
fn reaching<G: GraphView>(
    engine: &Engine<'_, G>,
    filter: EdgeFilter,
    starts: impl IntoIterator<Item = NodeId>,
) -> NodeSet {
    let g = engine.graph();
    let rev = engine.reverse();
    let mut seen = NodeSet::new(g.node_count());
    let mut stack: Vec<NodeId> = Vec::new();
    for s in starts {
        if seen.insert(s) {
            stack.push(s);
        }
    }
    while let Some(n) = stack.pop() {
        let (srcs, attrs) = rev.neighbors(n);
        for (&s, &a) in srcs.iter().zip(attrs) {
            if filter.accepts(a) && seen.insert(NodeId(s)) {
                stack.push(NodeId(s));
            }
        }
    }
    seen
}

const NONE: u32 = u32::MAX;

/// A breadth-first walk from `seeds` against the edges `filter` accepts that
/// does not follow the pairs of `skip` (`mocks`: their targets): for every node
/// it reaches, the next node towards a seed (a seed: itself), else [`NONE`].
fn without<G: GraphView>(
    engine: &Engine<'_, G>,
    filter: EdgeFilter,
    seeds: &[NodeId],
    skip: &HashMap<(u32, u32), u32>,
    mocks: &NodeSet,
) -> Vec<u32> {
    let rev = engine.reverse();
    let mut parent = vec![NONE; engine.graph().node_count() as usize];
    let mut order: Vec<u32> = Vec::new();
    for s in seeds {
        if parent[s.index()] == NONE {
            parent[s.index()] = s.0;
            order.push(s.0);
        }
    }
    let mut i = 0;
    while i < order.len() {
        let n = order[i];
        i += 1;
        let (srcs, attrs) = rev.neighbors(NodeId(n));
        let conditional = mocks.contains(NodeId(n));
        for (&s, &a) in srcs.iter().zip(attrs) {
            if parent[s as usize] != NONE
                || !filter.accepts(a)
                || (conditional && skip.contains_key(&(s, n)))
            {
                continue;
            }
            parent[s as usize] = n;
            order.push(s);
        }
    }
    parent
}

/// Takes out of `results` the change-driven jest and vitest tests that reach
/// the change only through manual mocks that no file they load registers (see
/// the module documentation). `reached` are the nodes that reach a seed over
/// `filter`, `seeds` the seed nodes of the pass, `causes` the changed nodes
/// among them with what changed. Returns the conditions for the per-entry
/// walks, when the change reaches a conditional edge.
///
/// A test that stays selected through another path gets a chain without the
/// edge that does not hold for it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply<G: GraphView>(
    engine: &Engine<'_, G>,
    filter: EdgeFilter,
    seeds: &[NodeId],
    causes: &[(NodeId, Cause)],
    reached: &[NodeId],
    reached_set: &NodeSet,
    results: &mut Vec<EntryResult>,
    is_affected: &mut NodeSet,
    pruned: &mut PruneReport,
) -> Option<Conditions> {
    let g = engine.graph();
    // The registering file of a conditional edge.
    let registrant = |spec: Option<&str>| {
        spec.and_then(parse)
            .and_then(|(_, file)| g.node_by_path(file))
    };
    // 1. Conditional edges between two files the change reaches.
    let mut pairs: HashMap<(u32, u32), u32> = HashMap::new();
    // Modules, with the files that register the mocks of their conditional edges.
    let mut modules: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
    let mut found: Vec<(NodeId, NodeId)> = Vec::new();
    // The modules with such edges from the derived layer's tags, else every file
    // the change reaches.
    let tagged: Option<Vec<NodeId>> = g.tagged(super::TAG_MODULE).map(|t| {
        t.iter()
            .map(|&n| NodeId(n))
            .filter(|&n| reached_set.contains(n))
            .collect()
    });
    for &n in tagged.as_deref().unwrap_or(reached) {
        found.clear();
        for e in g.out_edges(n) {
            let Some(to) = e.to else { continue };
            if e.kind() != RefKind::Require || !reached_set.contains(to) || !filter.accepts(e.attr)
            {
                continue;
            }
            if let Some(file) = registrant(e.specifier) {
                found.push((to, file));
            }
        }
        if found.is_empty() {
            continue;
        }
        // Another edge to the same mock (an import of it) is followed whoever
        // registers.
        for e in g.out_edges(n) {
            let Some(to) = e.to else { continue };
            if filter.accepts(e.attr)
                && found.iter().any(|f| f.0 == to)
                && registrant(e.specifier).is_none()
            {
                found.retain(|f| f.0 != to);
            }
        }
        if found.is_empty() {
            continue;
        }
        let i = modules.len() as u32;
        let mut files = Vec::new();
        for &(to, file) in &found {
            pairs.entry((n.0, to.0)).or_insert(i);
            if !files.contains(&file) {
                files.push(file);
            }
        }
        modules.push((n, files));
    }
    if pairs.is_empty() {
        return None;
    }
    let mocks = NodeSet::from_iter_in(g.node_count(), pairs.keys().map(|k| NodeId(k.1)));
    // 2. What the change selects without them.
    let base = without(engine, filter, seeds, &pairs, &mocks);
    let open: Vec<usize> = results
        .iter()
        .enumerate()
        .filter(|(_, r)| r.uncertain.is_none() && base[r.entry.index()] == NONE)
        .map(|(i, _)| i)
        .collect();
    // 3. Per module: who requests it and loads a file that registers its mock (also
    //    when
    // every test is selected without the edges: the per-entry walks ask).
    let mut requests: Vec<NodeSet> = Vec::with_capacity(modules.len());
    let mut holds: Vec<NodeSet> = Vec::with_capacity(modules.len());
    // Modules with the same registering files (one helper that registers many
    // mocks) share the walk from them.
    let mut loads: Vec<(&[NodeId], NodeSet)> = Vec::new();
    for (module, files) in &modules {
        let r = reaching(engine, filter, [*module]);
        let li = match loads.iter().position(|(f, _)| *f == files.as_slice()) {
            Some(li) => li,
            None => {
                loads.push((
                    files.as_slice(),
                    reaching(engine, filter, files.iter().copied()),
                ));
                loads.len() - 1
            }
        };
        let mut h = loads[li].1.clone();
        h.intersect_with(&r);
        requests.push(r);
        holds.push(h);
    }
    let mut drop = vec![false; results.len()];
    let mut any = false;
    for i in open {
        let r = &results[i];
        if holds.iter().any(|h| h.contains(r.entry)) {
            continue;
        }
        // The conditional edge on the entry's chain (seed first), else of a module it
        // requests.
        let on_chain = r
            .witness
            .windows(2)
            .find_map(|w| pairs.get(&(w[1].0, w[0].0)).map(|&m| (w[0], m)));
        let Some((mock, m)) = on_chain.or_else(|| {
            let m = requests.iter().position(|s| s.contains(r.entry))?;
            pairs
                .iter()
                .filter(|(_, v)| **v as usize == m)
                .map(|(k, v)| (NodeId(k.1), *v))
                .min_by_key(|x| x.0.0)
        }) else {
            // Selected by something these walks do not follow: kept.
            continue;
        };
        if !is_test(g, r.entry) {
            continue;
        }
        let (module, files) = &modules[m as usize];
        drop[i] = true;
        any = true;
        is_affected.remove(r.entry);
        pruned.unregistered.push(Unregistered {
            entry: r.entry,
            mock,
            module: *module,
            by: files[0],
            others: files.len() - 1,
        });
    }
    if any {
        let mut i = 0;
        results.retain(|_| {
            i += 1;
            !drop[i - 1]
        });
    }
    // A test selected without the edges whose chain uses one that does not hold for
    // it: the chain of the walk without them (when its seed is a changed node
    // with a cause).
    for r in results.iter_mut() {
        if r.uncertain.is_some() || base[r.entry.index()] == NONE {
            continue;
        }
        let wrong = r.witness.windows(2).any(|w| {
            pairs
                .get(&(w[1].0, w[0].0))
                .is_some_and(|&m| !holds[m as usize].contains(r.entry))
        });
        if !wrong {
            continue;
        }
        let mut chain = vec![r.entry];
        let mut n = r.entry;
        while base[n.index()] != n.0 {
            n = NodeId(base[n.index()]);
            chain.push(n);
        }
        chain.reverse();
        if let Some((_, cause)) = causes.iter().find(|(s, _)| *s == chain[0]) {
            r.witness = chain;
            r.cause = Some(cause.clone());
        }
    }
    Some(Conditions {
        pairs,
        mocks,
        holds,
    })
}
