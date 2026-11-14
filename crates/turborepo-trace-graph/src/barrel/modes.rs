//! Type-only edges per entry in `affected`.
//!
//! By default `affected` follows type-only edges (`import type`, `export type
//! ... from`, `typeof import()`, references in `.d.ts` files), because
//! type-check and build steps read them. A test whose runner transpiles file by
//! file ([`Runner::Isolated`]) never loads the target of such an edge: the
//! transpiler erases the import, whatever its configuration (`import type` and
//! `export type` are erased even under `verbatimModuleSyntax`; an `import {type
//! A}` with only inline type specifiers is *not* a type-only edge, because it
//! stays a module load under that option). So for those entries a change (or an
//! uncertain site) that is reached only through type-only edges cannot be
//! observed when the test runs.
//!
//! [`Engine::affected_with`] therefore evaluates the entries in two passes over
//! the shared reverse adjacency when [`Barrels::runtime_entries`] is set:
//! [`Runner::Isolated`] entries with the engine's traversal filter minus
//! `TYPE_ONLY` edges, every other entry (tasks, type-checking runners, entries
//! of other frameworks) with the engine's filter. Seeds, barrel pruning,
//! erased-equal files and the uncertainty step all use the pass's filter:
//! an uncertain site that a transpile-only test reaches only through type-only
//! edges does not select it. Mock-aware loading ([`super::mocks`]) runs in each
//! pass with the pass's filter.

use super::{Barrels, PruneReport};
use crate::{
    change::{AffectedOptions, AffectedReport, ChangeSet, EntryResult},
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId},
    vocab::{EdgeFilter, RefFlags},
};

/// How the runner of a test entry treats TypeScript, weakest guarantee first.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Runner {
    /// Not a jest or vitest test, a runner that type-checks (`ts-jest` with
    /// diagnostics, vitest `typecheck`, type tests), or an unknown
    /// transform: every edge and every change counts.
    Checks,
    /// Transpiles without reporting type errors but with the whole program at
    /// hand (`ts-jest` with `diagnostics: false`): the emitted code of a
    /// file may depend on other files' types (`const enum` inlining, elided
    /// re-exports).
    Transpiles,
    /// Transpiles each file on its own (`@swc/jest`, `babel-jest`, esbuild,
    /// `ts-jest` with `isolatedModules`, vitest without `typecheck`): the
    /// emitted code of a file depends on that file's text only.
    Isolated,
}

/// Whether every edge `a → b` the engine follows is type-only (and there is
/// one).
pub(crate) fn type_only_hop<G: GraphView>(g: &G, a: NodeId, b: NodeId) -> bool {
    let mut any = false;
    for e in g.out_edges(a) {
        if e.to != Some(b) {
            continue;
        }
        if !e.flags().contains(RefFlags::TYPE_ONLY) {
            return false;
        }
        any = true;
    }
    any
}

/// Records in `pruned` the entries of a runtime pass that the change reaches
/// only through type-only edges: reached from `seeds` over `all` edges, but
/// neither in `results` nor skipped by another rule.
pub(crate) fn type_only_skips<G: GraphView>(
    engine: &Engine<'_, G>,
    all: EdgeFilter,
    seeds: &[NodeId],
    entries: &NodeSet,
    results: &[EntryResult],
    pruned: &mut PruneReport,
) {
    let g = engine.graph();
    let (_, order) = super::bfs(engine, all, seeds.iter().copied());
    let mut done = NodeSet::from_iter_in(g.node_count(), results.iter().map(|r| r.entry));
    for &n in &pruned.unaffected {
        done.insert(n);
    }
    for m in &pruned.mocked {
        done.insert(m.entry);
    }
    for u in &pruned.unregistered {
        done.insert(u.entry);
    }
    pruned.type_only = order
        .into_iter()
        .filter(|&n| entries.contains(n) && !done.contains(n))
        .collect();
    pruned
        .type_only
        .sort_unstable_by(|a, b| g.path(*a).cmp(g.path(*b)));
    pruned.seeds = seeds.to_vec();
}

/// Both passes' reports as one.
fn merge(
    g: &impl GraphView,
    (mut r, mut p): (AffectedReport, PruneReport),
    (r2, p2): (AffectedReport, PruneReport),
) -> (AffectedReport, PruneReport) {
    r.entry_count += r2.entry_count;
    r.entries.extend(r2.entries);
    r.entries
        .sort_unstable_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
    r.seed_count = r.seed_count.max(r2.seed_count);
    r.reached_count = r.reached_count.max(r2.reached_count);
    p.deferred = p.deferred.max(p2.deferred);
    for (mine, theirs) in [(&mut p.skipped, p2.skipped), (&mut p.erased, p2.erased)] {
        for s in theirs {
            match mine.iter_mut().find(|m| m.node == s.node) {
                Some(m) => {
                    m.entries += s.entries;
                    m.relevant = m.relevant.max(s.relevant);
                    for b in s.barrels {
                        if !m.barrels.contains(&b) {
                            m.barrels.push(b);
                        }
                    }
                }
                None => mine.push(s),
            }
        }
    }
    p.mocked.extend(p2.mocked);
    p.unregistered.extend(p2.unregistered);
    p.mock_stats.add(&p2.mock_stats);
    p.unaffected.extend(p2.unaffected);
    p.unaffected
        .sort_unstable_by(|a, b| g.path(*a).cmp(g.path(*b)));
    (r, p)
}

impl<G: GraphView> Engine<'_, G> {
    /// [`Engine::affected`] with the pruning rules of [`crate::barrel`]:
    ///
    /// - export-level tracing through barrel files: modified
    ///   [`super::Candidate`]s that nothing else seeds are skipped for entries
    ///   that don't use their exports;
    /// - erased-equal files ([`Barrels::erased`]) are skipped for entries that
    ///   only load them as modules through a per-file transpiler
    ///   ([`super::erase`]);
    /// - with [`Barrels::runtime_entries`], entries whose runner transpiles
    ///   file by file are evaluated without type-only edges (see the module
    ///   documentation); [`PruneReport::runtime_entries`] lists them.
    pub fn affected_with(
        &self,
        changes: &ChangeSet,
        entries: &NodeSet,
        opts: &AffectedOptions,
        barrels: Option<&Barrels<'_>>,
    ) -> (AffectedReport, PruneReport) {
        let all = self.options().traversal;
        let g = self.graph();
        // Build tasks are evaluated in task mode (`crate::taskmode`), everything else
        // below.
        if let Some((tasks, rest)) = crate::taskmode::split(g, entries) {
            let out = self.affected_pass(changes, &tasks, opts, barrels, all, true);
            if rest.is_empty() {
                return out;
            }
            return merge(g, self.affected_with(changes, &rest, opts, barrels), out);
        }
        let runtime = all.without_flags(RefFlags::TYPE_ONLY);
        let Some(b) = barrels.filter(|b| b.runtime_entries && runtime != all) else {
            return self.affected_pass(changes, entries, opts, barrels, all, false);
        };
        let mut isolated = NodeSet::new(g.node_count());
        let mut rest = NodeSet::new(g.node_count());
        for n in entries.iter() {
            if b.runner(g, n) == Runner::Isolated {
                isolated.insert(n);
            } else {
                rest.insert(n);
            }
        }
        if isolated.is_empty() {
            return self.affected_pass(changes, entries, opts, barrels, all, false);
        }
        let mut out = self.affected_pass(changes, &isolated, opts, barrels, runtime, false);
        if !rest.is_empty() {
            out = merge(
                g,
                out,
                self.affected_pass(changes, &rest, opts, barrels, all, false),
            );
        }
        out.1.runtime_entries = Some(isolated);
        out
    }
}
