//! The query engine: lazily built indexes, traversal primitives, and evaluation
//! of parsed expressions.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use crate::{
    change::{
        AffectedOptions, AffectedReport, Cause, Change, ChangeKind, ChangeSet, Disposition,
        EntryResult, Policy, UncertainSite, UncertaintyCounts, UncertaintyNote, Verdict,
    },
    glob::{Glob, is_glob},
    index::{InputIndex, MembershipIndex, ProbeIndex},
    parse::{Expr, ExprKind, QueryError, SetOp, SrcSpan, parse},
    set::NodeSet,
    view::{Adjacency, GraphView, NodeId, PackageId, PackageVersion},
    vocab::{EdgeAttr, EdgeFilter, Phase, RefFlags, RefKind, UncertaintyKind},
};

const NONE: u32 = u32::MAX;

/// Engine-wide options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// Edges that traversals (`deps`, `rdeps`, paths, `affected`) follow. The
    /// default follows every resolved edge, type-only included (a
    /// typecheck reads them); use `EdgeFilter::ALL.
    /// without_flags(RefFlags::TYPE_ONLY)` for `--runtime-only`.
    pub traversal: EdgeFilter,
    /// Defaults for `affected` (the query can override the policy).
    pub affected: AffectedOptions,
}

/// A value bound to a `$name` variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A set of nodes.
    Nodes(NodeSet),
    /// A change set (only valid as the first argument of `affected`).
    Changes(ChangeSet),
}

/// The result of a query.
#[derive(Clone, Debug)]
pub struct QueryResult {
    /// The resulting node set.
    pub nodes: NodeSet,
    /// For a top-level `somepath`, the path in order (start first).
    pub path: Option<Vec<NodeId>>,
    /// For a top-level `affected`, the per-entry report.
    pub affected: Option<AffectedReport>,
}

enum Extra {
    None,
    Path(Vec<NodeId>),
    Affected(AffectedReport),
}

/// Evaluates queries against a [`GraphView`].
///
/// Indexes are built on first use and kept for later queries: the forward
/// adjacency (unless the view provides one), the reverse adjacency (CSR
/// transpose), the probe index, the membership index and the list of uncertain
/// nodes. Traversal buffers are pooled. The engine is `Sync` when the view is,
/// so one engine can serve concurrent queries.
pub struct Engine<'g, G: GraphView> {
    g: &'g G,
    opts: EngineOptions,
    fwd: OnceLock<Adjacency>,
    rev: OnceLock<Adjacency>,
    probes: OnceLock<ProbeIndex<'g>>,
    members: OnceLock<MembershipIndex<'g>>,
    inputs: OnceLock<InputIndex<'g>>,
    uncertain: OnceLock<Vec<NodeId>>,
    /// Scopes of covered first-party uncertainties (see
    /// [`Engine::covered_scopes`]).
    #[allow(clippy::type_complexity)]
    covered: OnceLock<Vec<(NodeId, String, String, Option<Glob>)>>,
    queues: Mutex<Vec<Vec<u32>>>,
    parents: Mutex<Vec<Vec<u32>>>,
    bindings: HashMap<String, Value>,
}

impl<'g, G: GraphView> Engine<'g, G> {
    /// An engine with default options.
    pub fn new(g: &'g G) -> Self {
        Self::with_options(g, EngineOptions::default())
    }

    /// An engine with options.
    pub fn with_options(g: &'g G, opts: EngineOptions) -> Self {
        Engine {
            g,
            opts,
            fwd: OnceLock::new(),
            rev: OnceLock::new(),
            probes: OnceLock::new(),
            members: OnceLock::new(),
            inputs: OnceLock::new(),
            uncertain: OnceLock::new(),
            covered: OnceLock::new(),
            queues: Mutex::new(Vec::new()),
            parents: Mutex::new(Vec::new()),
            bindings: HashMap::new(),
        }
    }

    /// The graph.
    pub fn graph(&self) -> &'g G {
        self.g
    }

    /// The options.
    pub fn options(&self) -> &EngineOptions {
        &self.opts
    }

    /// Binds `$name` for later queries, for example `$changes` from `--since`
    /// or `$tests` from framework discovery.
    pub fn bind(&mut self, name: impl Into<String>, value: Value) -> &mut Self {
        self.bindings.insert(name.into(), value);
        self
    }

    /// An empty set over this graph.
    pub fn empty(&self) -> NodeSet {
        NodeSet::new(self.g.node_count())
    }

    /// Resolved outgoing edges (from the view, or built once).
    pub fn forward(&self) -> &Adjacency {
        match self.g.forward_adjacency() {
            Some(a) => a,
            None => self.fwd.get_or_init(|| Adjacency::from_view(self.g)),
        }
    }

    /// Resolved incoming edges (from the view, or the transpose of [`forward`],
    /// built once).
    ///
    /// [`forward`]: Engine::forward
    pub fn reverse(&self) -> &Adjacency {
        match self.g.reverse_adjacency() {
            Some(a) => a,
            None => self.rev.get_or_init(|| self.forward().transpose()),
        }
    }

    /// Builds the lazily computed structures `affected` needs (reverse
    /// adjacency, probe, membership and input indexes, uncertain nodes), so
    /// a caller can overlap them with other work.
    pub fn prepare(&self) {
        self.warm();
    }

    fn probe_index(&self) -> &ProbeIndex<'g> {
        self.probes.get_or_init(|| ProbeIndex::build(self.g))
    }

    fn membership_index(&self) -> &MembershipIndex<'g> {
        self.members.get_or_init(|| MembershipIndex::build(self.g))
    }

    fn input_index(&self) -> &InputIndex<'g> {
        self.inputs.get_or_init(|| InputIndex::build(self.g))
    }

    /// Builds every lazy index now (useful before timing queries or sharing the
    /// engine).
    pub fn warm(&self) {
        self.reverse();
        if !self.g.has_lookups() {
            self.probe_index();
            self.membership_index();
            self.input_index();
        }
        self.uncertain_nodes();
    }

    /// Sizes of the built indexes, for diagnostics: `(reverse edges, probes,
    /// memberships)`.
    pub fn index_stats(&self) -> (usize, usize, usize) {
        (
            self.reverse().edge_count(),
            self.probe_index().len(),
            self.membership_index().len(),
        )
    }

    /// Number of `(input, node)` pairs in the input index (builds it).
    pub fn input_count(&self) -> usize {
        self.input_index().len()
    }

    pub(crate) fn uncertain_nodes(&self) -> &[NodeId] {
        self.uncertain.get_or_init(|| {
            (0..self.g.node_count())
                .map(NodeId)
                .filter(|&n| self.g.has_uncertainty(n))
                .collect()
        })
    }

    /// The scopes of covered first-party uncertainties: `(node, dir, pattern,
    /// glob)`, with `None` for an invalid pattern (it then covers
    /// everything below `dir`, conservative).
    fn covered_scopes(&self) -> &[(NodeId, String, String, Option<Glob>)] {
        self.covered.get_or_init(|| {
            let mut out = Vec::new();
            for &n in self.uncertain_nodes() {
                if self.g.is_external(n) {
                    continue;
                }
                for u in self.g.uncertainties(n) {
                    if let Some(sc) = u.scope {
                        let dir = crate::mem::normalize(sc.dir).to_owned();
                        let pattern = sc.pattern.to_owned();
                        if !out
                            .iter()
                            .any(|(m, d, p, _): &(NodeId, String, String, _)| {
                                *m == n && *d == dir && *p == pattern
                            })
                        {
                            let glob = Glob::new(&pattern).ok();
                            out.push((n, dir, pattern, glob));
                        }
                    }
                }
            }
            out
        })
    }

    /// Calls `f(node, dir, pattern)` for every covered scope that contains
    /// `path`.
    pub fn match_scopes(&self, path: &str, f: &mut dyn FnMut(NodeId, &str, &str)) {
        for (n, dir, pattern, glob) in self.covered_scopes() {
            let rest = if dir.is_empty() {
                Some(path)
            } else {
                path.strip_prefix(dir.as_str())
                    .and_then(|r| r.strip_prefix('/'))
            };
            if let Some(rest) = rest
                && glob.as_ref().is_none_or(|g| g.matches(rest))
            {
                f(*n, dir, pattern);
            }
        }
    }

    /// First-party uncertainty records by how the policy treats them under
    /// `opts`.
    pub fn uncertainty_counts(&self, opts: &AffectedOptions) -> UncertaintyCounts {
        let mut c = UncertaintyCounts::default();
        for &n in self.uncertain_nodes() {
            if self.g.is_external(n) {
                continue;
            }
            for u in self.g.uncertainties(n) {
                match opts.disposition(&u, self.g.lang(n)) {
                    Disposition::Open => c.open += 1,
                    Disposition::Scoped | Disposition::Named | Disposition::Declared => {
                        c.covered += 1
                    }
                    Disposition::Probed => {
                        c.covered += 1;
                        c.probed += 1;
                    }
                    Disposition::Ignored => c.ignored += 1,
                    Disposition::Membership => {}
                }
            }
        }
        c
    }

    /// The first-party uncertainties in `entry`'s closure (along the traversal
    /// filter) that don't trigger the policy under `opts`: covered ones (by
    /// a scope or by their resolution probes) and those ignored by default
    /// ([`AffectedOptions::disposition`]). Sorted by path and line.
    pub fn uncertainty_notes(&self, entry: NodeId, opts: &AffectedOptions) -> Vec<UncertaintyNote> {
        let mut from = self.empty();
        from.insert(entry);
        let closure = self.deps(&from, None);
        let mut out = Vec::new();
        for &n in self.uncertain_nodes() {
            if !closure.contains(n) || self.g.is_external(n) {
                continue;
            }
            for u in self.g.uncertainties(n) {
                let scope = u.scope.map(|s| (s.dir.to_owned(), s.pattern.to_owned()));
                let disposition = opts.disposition(&u, self.g.lang(n));
                if matches!(
                    disposition,
                    Disposition::Scoped
                        | Disposition::Probed
                        | Disposition::Ignored
                        | Disposition::Named
                        | Disposition::Declared
                ) {
                    out.push(UncertaintyNote {
                        node: n,
                        kind: u.kind,
                        line: u.line,
                        detail: u.detail.to_owned(),
                        scope,
                        disposition,
                    });
                }
            }
        }
        out.sort_by(|a, b| {
            (self.g.path(a.node), a.line, a.kind).cmp(&(self.g.path(b.node), b.line, b.kind))
        });
        out.dedup();
        out
    }

    fn take_queue(&self) -> Vec<u32> {
        let mut q = self
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .unwrap_or_default();
        q.clear();
        q
    }

    fn put_queue(&self, q: Vec<u32>) {
        self.queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(q);
    }

    fn take_parents(&self) -> Vec<u32> {
        self.parents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .unwrap_or_else(|| vec![NONE; self.g.node_count() as usize])
    }

    /// Returns a parent array; every entry touched must be listed in `touched`.
    fn put_parents(&self, mut p: Vec<u32>, touched: &[u32]) {
        for &t in touched {
            p[t as usize] = NONE;
        }
        debug_assert!(p.iter().all(|&x| x == NONE));
        self.parents
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(p);
    }

    // ---------------------------------------------------------------------------------
    // Traversal primitives
    // ---------------------------------------------------------------------------------

    fn closure(&self, adj: &Adjacency, seeds: &NodeSet, depth: Option<u32>) -> NodeSet {
        let filter = self.opts.traversal;
        let all = filter.is_all();
        let mut out = seeds.clone();
        let mut q = self.take_queue();
        q.extend(seeds.iter().map(|n| n.0));
        let mut head = 0;
        let mut level = 0;
        while head < q.len() && depth.is_none_or(|d| level < d) {
            let end = q.len();
            while head < end {
                let n = NodeId(q[head]);
                head += 1;
                let (targets, attrs) = adj.neighbors(n);
                for (&t, &a) in targets.iter().zip(attrs) {
                    if (all || filter.accepts(a)) && out.insert(NodeId(t)) {
                        q.push(t);
                    }
                }
            }
            level += 1;
        }
        self.put_queue(q);
        out
    }

    /// Transitive dependencies of `x` (including `x`), up to `depth` edges.
    pub fn deps(&self, x: &NodeSet, depth: Option<u32>) -> NodeSet {
        self.closure(self.forward(), x, depth)
    }

    /// Transitive dependents of `x` (including `x`), up to `depth` edges.
    pub fn rdeps(&self, x: &NodeSet, depth: Option<u32>) -> NodeSet {
        self.closure(self.reverse(), x, depth)
    }

    /// Multi-source BFS over `adj` from `seeds`, recording in `parent` the node
    /// each visited node was reached from (seeds point to themselves).
    /// Stops early when `stop` returns true for a newly reached node,
    /// returning it. `q` ends up holding every visited node.
    fn bfs_parents(
        &self,
        adj: &Adjacency,
        seeds: impl Iterator<Item = NodeId>,
        parent: &mut [u32],
        q: &mut Vec<u32>,
        stop: impl FnMut(NodeId) -> bool,
    ) -> Option<NodeId> {
        self.bfs_parents_with(adj, self.opts.traversal, seeds, parent, q, stop)
    }

    /// [`Engine::bfs_parents`] over the edges `filter` accepts.
    fn bfs_parents_with(
        &self,
        adj: &Adjacency,
        filter: EdgeFilter,
        seeds: impl Iterator<Item = NodeId>,
        parent: &mut [u32],
        q: &mut Vec<u32>,
        mut stop: impl FnMut(NodeId) -> bool,
    ) -> Option<NodeId> {
        let all = filter.is_all();
        for s in seeds {
            if parent[s.index()] == NONE {
                parent[s.index()] = s.0;
                q.push(s.0);
                if stop(s) {
                    return Some(s);
                }
            }
        }
        let mut head = 0;
        while head < q.len() {
            let n = q[head];
            head += 1;
            let (targets, attrs) = adj.neighbors(NodeId(n));
            for (&t, &a) in targets.iter().zip(attrs) {
                if parent[t as usize] == NONE && (all || filter.accepts(a)) {
                    parent[t as usize] = n;
                    q.push(t);
                    if stop(NodeId(t)) {
                        return Some(NodeId(t));
                    }
                }
            }
        }
        None
    }

    fn chain(parent: &[u32], mut n: NodeId) -> Vec<NodeId> {
        let mut out = vec![n];
        while parent[n.index()] != n.0 {
            n = NodeId(parent[n.index()]);
            out.push(n);
        }
        out
    }

    /// A shortest path from some node of `from` to some node of `to`, start
    /// first.
    pub fn somepath(&self, from: &NodeSet, to: &NodeSet) -> Option<Vec<NodeId>> {
        let mut parent = self.take_parents();
        let mut q = self.take_queue();
        let hit = self.bfs_parents(self.forward(), from.iter(), &mut parent, &mut q, |n| {
            to.contains(n)
        });
        let path = hit.map(|h| {
            let mut p = Self::chain(&parent, h);
            p.reverse();
            p
        });
        self.put_parents(parent, &q);
        self.put_queue(q);
        path
    }

    /// Every node on some path from `from` to `to`: `deps(from) ^ rdeps(to)`.
    pub fn allpaths(&self, from: &NodeSet, to: &NodeSet) -> NodeSet {
        let mut d = self.deps(from, None);
        if d.intersects(to) {
            let r = self.rdeps(to, None);
            d.intersect_with(&r);
            d
        } else {
            self.empty()
        }
    }

    // ---------------------------------------------------------------------------------
    // affected
    // ---------------------------------------------------------------------------------

    /// Calls `f(node, probe)` for every probe matched by adding (or deleting)
    /// `path`: the path itself or one of its ancestor directories.
    pub fn match_probes(&self, path: &str, f: &mut dyn FnMut(NodeId, &str)) {
        if self.g.has_lookups() {
            let candidates =
                std::iter::once(path).chain(crate::index::dir_splits(path).map(|(d, _)| d));
            for c in candidates.filter(|c| !c.is_empty()) {
                self.g.probe_lookup(c, f);
            }
        } else {
            self.probe_index().matches(path, f);
        }
    }

    /// Calls `f(node, probe)` for every probe that lies below `path`.
    ///
    /// A path that changes takes every answer below it with it. Below a file
    /// there is nothing, so this is about what a path can lead to: a link
    /// to a directory that goes (a lookup through it now stops at the
    /// link's name, and records the path it asked for), comes or leads
    /// elsewhere, and a file where a directory was.
    pub fn match_probes_below(&self, path: &str, f: &mut dyn FnMut(NodeId, &str)) {
        if path.is_empty() {
            return;
        }
        if self.g.has_lookups() {
            self.g.probe_lookup_below(path, f);
        } else {
            self.probe_index().below(path, f);
        }
    }

    /// Calls `f(node, dir, pattern)` for every node that asked for the
    /// directory `path` by its name: a membership of its parent directory
    /// whose pattern is the literal name.
    ///
    /// That is the record of the answer "it is a directory" (or "there is
    /// nothing of that name") to a lookup that asked for exactly this path.
    /// A directory that a change makes or removes ([`ChangeSet::dirs`]) is
    /// another answer to it. It is not matched against patterns: what a
    /// listing with a wildcard held is a question of the files that came or
    /// went, which the change names.
    ///
    /// The name is the one the record's pattern says, whatever characters it is
    /// made of ([`crate::glob::escape`]): a directory called `[id]` is
    /// asked for by that name, and its record is the pattern `\[id]`.
    pub fn match_dir(&self, path: &str, f: &mut dyn FnMut(NodeId, &str, &str)) {
        let (parent, name) = match path.rfind('/') {
            Some(i) => (&path[..i], &path[i + 1..]),
            None => ("", path),
        };
        if name.is_empty() {
            return;
        }
        let pattern_of_name = crate::glob::escape(name);
        if self.g.has_lookups() {
            self.g.membership_lookup(parent, &mut |n, d, pattern| {
                if pattern == pattern_of_name {
                    f(n, d, pattern);
                }
            });
        } else {
            // (The index matches the path against every pattern of its directories.)
            self.membership_index().matches(path, |n, d, pattern| {
                if d == parent && pattern == pattern_of_name {
                    f(n, d, pattern);
                }
            });
        }
    }

    /// Calls `f(node, dir, pattern)` for every membership matched by adding or
    /// deleting `path`. Invalid patterns match everything below their
    /// directory (conservative).
    pub fn match_memberships(&self, path: &str, f: &mut dyn FnMut(NodeId, &str, &str)) {
        if self.g.has_lookups() {
            let mut globs: HashMap<String, Option<Glob>> = HashMap::new();
            for (dir, rest) in crate::index::dir_splits(path) {
                // Views usually list a directory's memberships grouped by pattern; remember
                // the verdict for the previous pattern so each run is matched once.
                let mut last: Option<(String, bool)> = None;
                self.g.membership_lookup(dir, &mut |n, d, pat| {
                    let hit = match &last {
                        Some((p, hit)) if p == pat => *hit,
                        _ => {
                            let g = globs
                                .entry(pat.to_owned())
                                .or_insert_with(|| Glob::new(pat).ok());
                            let hit = g.as_ref().is_none_or(|g| g.matches(rest));
                            last = Some((pat.to_owned(), hit));
                            hit
                        }
                    };
                    if hit {
                        f(n, d, pat);
                    }
                });
            }
        } else {
            self.membership_index().matches(path, f);
        }
    }

    /// Calls `f(node)` for every node with `path` as an input.
    pub fn match_inputs(&self, path: &str, f: &mut dyn FnMut(NodeId)) {
        if self.g.has_lookups() {
            self.g.input_lookup(path, f);
        } else {
            self.input_index().matches(path, f);
        }
    }

    /// Seeds for a change set: node → first cause. Also returns unmatched
    /// changes. With `after_change`, deleted paths also match probes
    /// ([`AffectedOptions::after_change`]). `filter`: the traversal filter
    /// of the evaluation pass (importers of changed packages are seeded
    /// over the edges it accepts).
    pub(crate) fn seeds(
        &self,
        changes: &ChangeSet,
        after_change: bool,
        filter: EdgeFilter,
    ) -> (Vec<(NodeId, Cause)>, Vec<Change>) {
        let g = self.g;
        let mut seen = NodeSet::new(g.node_count());
        let mut seeds = Vec::new();
        let mut unmatched = Vec::new();
        let mut add = |n: NodeId, cause: &dyn Fn() -> Cause, seeds: &mut Vec<(NodeId, Cause)>| {
            if seen.insert(n) {
                seeds.push((n, cause()));
            }
        };
        for c in &changes.files {
            let mut matched = false;
            let node = g.node_by_path(&c.path);
            match c.kind {
                ChangeKind::Modified | ChangeKind::Added => {
                    // An "added" path that is already a node means the graph is stale; treat
                    // it as modified (conservative).
                    if let Some(n) = node {
                        matched = true;
                        add(
                            n,
                            &|| Cause::Modified {
                                path: c.path.clone(),
                            },
                            &mut seeds,
                        );
                    }
                }
                ChangeKind::Deleted => {
                    if let Some(n) = node {
                        matched = true;
                        add(
                            n,
                            &|| Cause::Deleted {
                                path: c.path.clone(),
                            },
                            &mut seeds,
                        );
                    }
                }
            }
            match c.kind {
                ChangeKind::Added => self.match_probes(&c.path, &mut |n, probe| {
                    matched = true;
                    add(
                        n,
                        &|| Cause::Probe {
                            added: c.path.clone(),
                            probe: probe.to_owned(),
                        },
                        &mut seeds,
                    );
                }),
                // On a graph built after the change, a file that resolved to the deleted path
                // now probes it (or falls back to a later candidate after probing it). On a
                // "before" graph a deleted path was present, so no probe can name it.
                ChangeKind::Deleted if after_change => self.match_probes(&c.path, &mut |n, _| {
                    matched = true;
                    add(
                        n,
                        &|| Cause::Deleted {
                            path: c.path.clone(),
                        },
                        &mut seeds,
                    );
                }),
                ChangeKind::Deleted | ChangeKind::Modified => {}
            }
            // What was looked for below the path ([`Engine::match_probes_below`]).
            if c.kind != ChangeKind::Deleted || after_change {
                self.match_probes_below(&c.path, &mut |n, probe| {
                    matched = true;
                    add(n, &|| below_cause(c, probe), &mut seeds);
                });
            }
            // Config inputs: any change to a file that a resolution read (but for the
            // readers the caller found untouched by this change, `ChangeSet::spared`).
            let spared: Option<&Vec<NodeId>> = changes
                .spared
                .iter()
                .find(|(p, _)| *p == c.path)
                .map(|(_, nodes)| nodes);
            self.match_inputs(&c.path, &mut |n| {
                if spared.is_some_and(|s| s.binary_search(&n).is_ok()) {
                    return;
                }
                matched = true;
                add(
                    n,
                    &|| match c.kind {
                        ChangeKind::Deleted => Cause::Deleted {
                            path: c.path.clone(),
                        },
                        _ => Cause::Modified {
                            path: c.path.clone(),
                        },
                    },
                    &mut seeds,
                );
            });
            // Covered uncertainty: any change inside a scope (its contents, added or
            // deleted files) can change what the reference reads.
            self.match_scopes(&c.path, &mut |n, dir, pattern| {
                matched = true;
                add(
                    n,
                    &|| Cause::Scope {
                        path: c.path.clone(),
                        kind: c.kind,
                        dir: dir.to_owned(),
                        pattern: pattern.to_owned(),
                    },
                    &mut seeds,
                );
            });
            if matches!(c.kind, ChangeKind::Added | ChangeKind::Deleted) {
                self.match_memberships(&c.path, &mut |n, dir, pattern| {
                    matched = true;
                    add(
                        n,
                        &|| Cause::Membership {
                            path: c.path.clone(),
                            kind: c.kind,
                            dir: dir.to_owned(),
                            pattern: pattern.to_owned(),
                        },
                        &mut seeds,
                    );
                });
            }
            if !matched {
                unmatched.push(c.clone());
            }
        }
        // Directories the change made or removed: the lookups that asked for one by
        // name.
        for c in &changes.dirs {
            self.match_dir(&c.path, &mut |n, dir, pattern| {
                add(
                    n,
                    &|| Cause::Membership {
                        path: c.path.clone(),
                        kind: c.kind,
                        dir: dir.to_owned(),
                        pattern: pattern.to_owned(),
                    },
                    &mut seeds,
                );
            });
        }
        // Nodes the caller seeds directly; those only type resolution observes, when
        // the pass follows type-only edges.
        let types = !filter.exclude_flags.contains(RefFlags::TYPE_ONLY);
        let typed = changes.type_seeds.iter().filter(|_| types);
        for (n, cause) in changes.seeds.iter().chain(typed) {
            if n.0 < g.node_count() {
                add(*n, &|| cause.clone(), &mut seeds);
            }
        }
        if !changes.packages.is_empty() {
            let mut pkgs = vec![false; g.package_count() as usize];
            for p in &changes.packages {
                if let Some(slot) = pkgs.get_mut(p.0 as usize) {
                    *slot = true;
                }
            }
            for i in 0..g.node_count() {
                let n = NodeId(i);
                let p = g.package(n);
                if pkgs[p.0 as usize] {
                    add(n, &|| Cause::Package { package: p }, &mut seeds);
                }
            }
        }
        if !changes.scoped.is_empty() {
            // Per changed package: the importers it changed for. A package's own nodes are
            // seeded only when it is traced (more than its opaque node); otherwise the
            // linking files of the listed importers are.
            let np = g.package_count() as usize;
            let mut scope: Vec<Option<Vec<bool>>> = vec![None; np];
            for s in &changes.scoped {
                let Some(slot) = scope.get_mut(s.package.0 as usize) else {
                    continue;
                };
                let set = slot.get_or_insert_with(|| vec![false; np]);
                for i in &s.importers {
                    if let Some(b) = set.get_mut(i.0 as usize) {
                        *b = true;
                    }
                }
            }
            let mut members: Vec<Vec<NodeId>> = vec![Vec::new(); np];
            for i in 0..g.node_count() {
                let n = NodeId(i);
                let p = g.package(n).0 as usize;
                if scope[p].is_some() {
                    members[p].push(n);
                }
            }
            let rev = self.reverse();
            for (p, importers) in scope.iter().enumerate() {
                let Some(importers) = importers else { continue };
                let package = PackageId(p as u32);
                let traced = members[p].iter().any(|&n| g.lang(n).is_some());
                for &n in &members[p] {
                    if traced {
                        add(n, &|| Cause::Package { package }, &mut seeds);
                    }
                    let (srcs, attrs) = rev.neighbors(n);
                    for (&s, &a) in srcs.iter().zip(attrs) {
                        let s = NodeId(s);
                        if filter.accepts(a) && importers[g.package(s).0 as usize] {
                            add(s, &|| Cause::Package { package }, &mut seeds);
                        }
                    }
                }
            }
        }
        (seeds, unmatched)
    }

    /// Evaluates which `entries` a change set affects.
    ///
    /// 1. Seeds: modified and deleted nodes; nodes whose probes match an added
    ///    path; nodes whose directory memberships match an added or deleted
    ///    path; every node of a changed package.
    /// 2. One multi-source reverse BFS from the seeds; entries reached are
    ///    affected, with the BFS tree giving each a shortest witness chain.
    ///    Nodes with a covered uncertainty whose scope contains a changed path
    ///    are seeds too ([`crate::Uncertainty::scope`]).
    /// 3. One reverse BFS from first-party (non-external) nodes with an open
    ///    uncertainty of a counted kind ([`AffectedOptions::triggers`]);
    ///    entries reached but not already affected get the policy's verdict,
    ///    with the nearest uncertain node as the reason. Covered uncertainties
    ///    never trigger it.
    pub fn affected(
        &self,
        changes: &ChangeSet,
        entries: &NodeSet,
        opts: &AffectedOptions,
    ) -> AffectedReport {
        self.affected_with(changes, entries, opts, None).0
    }

    /// One evaluation pass of [`Engine::affected_with`]
    /// (`crate::barrel::modes`) over the edges `filter` accepts:
    /// [`Engine::affected`] with export-level tracing through barrel
    /// files and erased-equal files. Modified [`crate::barrel::Candidate`]s and
    /// [`crate::barrel::Barrels::erased`] files that nothing else seeds are
    /// skipped for entries that cannot observe their change (see
    /// [`crate::barrel`]).
    pub(crate) fn affected_pass(
        &self,
        changes: &ChangeSet,
        entries: &NodeSet,
        opts: &AffectedOptions,
        barrels: Option<&crate::barrel::Barrels<'_>>,
        filter: EdgeFilter,
        task_mode: bool,
    ) -> (AffectedReport, crate::barrel::PruneReport) {
        let g = self.g;
        let rev = self.reverse();
        let (mut seeds, unmatched) = self.seeds(changes, opts.after_change, filter);
        // Task mode (`crate::taskmode`): a byte read carries the read file's own change
        // one hop, and the walk skips byte reads and listing-only links.
        let base = filter;
        let filter = if task_mode {
            let mut seeded = NodeSet::from_iter_in(g.node_count(), seeds.iter().map(|s| s.0));
            let readers = self.byte_readers(changes, &seeds, base, &mut seeded);
            seeds.extend(readers);
            crate::taskmode::filter(base)
        } else {
            base
        };
        let mut deferred = Vec::new();
        if let Some(b) = barrels {
            let (rest, cands) = crate::barrel::split_changes(g, changes, b);
            if !cands.is_empty() {
                let (rest_seeds, _) = self.seeds(&rest, opts.after_change, filter);
                deferred = crate::barrel::defer(&mut seeds, &rest_seeds, cands);
            }
        }
        self.attr_record(opts, filter, entries, &seeds, deferred.iter().map(|d| d.0));
        let mut results = Vec::new();
        let mut is_affected = NodeSet::new(g.node_count());

        let mut parent = self.take_parents();
        let mut q = self.take_queue();
        self.bfs_parents_with(
            rev,
            filter,
            seeds.iter().map(|s| s.0),
            &mut parent,
            &mut q,
            |_| false,
        );
        let reached_count = q.len();
        let cause_of: HashMap<u32, &Cause> = seeds.iter().map(|(n, c)| (n.0, c)).collect();
        for &n in &q {
            let n = NodeId(n);
            if entries.contains(n) {
                is_affected.insert(n);
                let mut witness = Self::chain(&parent, n);
                witness.reverse();
                let cause = cause_of.get(&witness[0].0).map(|c| (*c).clone());
                results.push(EntryResult {
                    entry: n,
                    verdict: Verdict::Affected,
                    witness,
                    cause,
                    uncertain: None,
                });
            }
        }
        self.put_parents(parent, &q);
        q.clear();
        let mut pruned = match barrels {
            Some(b) if !deferred.is_empty() => crate::barrel::resolve(
                self,
                b,
                changes,
                &deferred,
                entries,
                filter,
                &mut is_affected,
                &mut results,
            ),
            _ => crate::barrel::PruneReport::default(),
        };

        // Entries the change reaches only through modules they replace with a mock.
        if let Some(b) = barrels.filter(|b| b.mocks) {
            let nodes: Vec<NodeId> = seeds
                .iter()
                .map(|s| s.0)
                .chain(deferred.iter().map(|d| d.0))
                .collect();
            crate::barrel::mocks::apply(
                self,
                b,
                filter,
                &nodes,
                &seeds,
                &mut results,
                &mut is_affected,
                &mut pruned,
            );
        }

        if opts.policy != Policy::Ignore {
            // Sites whose open records all are entry-resolved
            // (`GraphView::is_entry_resolved`) only count for entries outside
            // `resolved_entries`; every other site counts for every entry that
            // reaches it.
            let exempt = g.resolved_entries();
            let mut uncertain: Vec<NodeId> = Vec::new();
            let mut per_entry: Vec<NodeId> = Vec::new();
            for &n in self.uncertain_nodes() {
                if g.is_external(n) {
                    continue;
                }
                let (mut open, mut resolved) = (false, false);
                for u in g.uncertainties(n) {
                    if self.disposition_of(n, &u, opts) != Disposition::Open {
                        continue;
                    }
                    if exempt.is_some() && g.is_entry_resolved(n, &u) {
                        resolved = true;
                    } else {
                        open = true;
                        break;
                    }
                }
                if open {
                    uncertain.push(n);
                } else if resolved {
                    per_entry.push(n);
                }
            }
            let verdict = match opts.policy {
                Policy::Affected => Verdict::Affected,
                _ => Verdict::Unknown,
            };
            for (sites, exempt) in [(uncertain, None), (per_entry, exempt)] {
                if sites.is_empty() {
                    continue;
                }
                q.clear();
                let mut parent = self.take_parents();
                self.bfs_parents_with(
                    rev,
                    filter.for_uncertainty(),
                    sites.into_iter(),
                    &mut parent,
                    &mut q,
                    |_| false,
                );
                for &n in &q {
                    let n = NodeId(n);
                    if entries.contains(n)
                        && !is_affected.contains(n)
                        && !exempt.is_some_and(|x| x.contains(n))
                    {
                        // An entry both passes reach keeps the first pass's site.
                        is_affected.insert(n);
                        let mut witness = Self::chain(&parent, n);
                        witness.reverse();
                        let site = witness[0];
                        let reasons = g
                            .uncertainties(site)
                            .filter(|u| self.disposition_of(site, u, opts) == Disposition::Open)
                            .map(|u| (u.kind, u.line, u.detail.to_owned()))
                            .collect();
                        results.push(EntryResult {
                            entry: n,
                            verdict,
                            witness,
                            cause: None,
                            uncertain: Some(UncertainSite {
                                node: site,
                                reasons,
                                bound: None,
                            }),
                        });
                    }
                }
                self.put_parents(parent, &q);
            }
        }
        self.put_queue(q);
        // A pass without type-only edges: which entries the change reaches only through
        // them.
        if barrels.is_some() && base != self.opts.traversal {
            let nodes: Vec<NodeId> = seeds
                .iter()
                .map(|s| s.0)
                .chain(deferred.iter().map(|d| d.0))
                .collect();
            crate::barrel::modes::type_only_skips(
                self,
                self.opts.traversal,
                &nodes,
                entries,
                &results,
                &mut pruned,
            );
        }

        results.sort_unstable_by(|a, b| g.path(a.entry).cmp(g.path(b.entry)));
        let report = AffectedReport {
            policy: opts.policy,
            entry_count: entries.len(),
            entries: results,
            seed_count: seeds.len() + deferred.len(),
            reached_count,
            unmatched,
        };
        (report, pruned)
    }

    // ---------------------------------------------------------------------------------
    // Filters
    // ---------------------------------------------------------------------------------

    /// Nodes of `x` that are the target of at least one edge accepted by `f`
    /// whose source is also in `x`. This is how `kind`, `phase` and `flags`
    /// read: `kind(fs_read, deps(main))` is the files main's closure reads
    /// through the fs.
    pub fn filter_by_incoming(&self, x: &NodeSet, f: &EdgeFilter) -> NodeSet {
        let rev = self.reverse();
        let mut out = x.clone();
        out.retain(|n| {
            let (srcs, attrs) = rev.neighbors(n);
            srcs.iter()
                .zip(attrs)
                .any(|(&s, &a)| f.accepts(a) && x.contains(NodeId(s)))
        });
        out
    }

    fn package_matcher(&self, spec: &str) -> Result<Vec<bool>, String> {
        let (name, version) = match spec.rfind('@') {
            Some(i) if i > 0 => (&spec[..i], Some(&spec[i + 1..])),
            _ => (spec, None),
        };
        let glob = if is_glob(name) {
            Some(Glob::new(name).map_err(|e| e.to_string())?)
        } else {
            None
        };
        Ok((0..self.g.package_count())
            .map(|p| {
                let info = self.g.package_info(PackageId(p));
                let name_ok = match &glob {
                    Some(gl) => gl.matches(info.name),
                    None => info.name == name,
                };
                let version_ok = version.is_none_or(|v| match info.version {
                    PackageVersion::Workspace => v == "workspace",
                    PackageVersion::Version(x) => x == v,
                });
                name_ok && version_ok
            })
            .collect())
    }

    // ---------------------------------------------------------------------------------
    // Evaluation
    // ---------------------------------------------------------------------------------

    /// Parses and evaluates a query.
    pub fn query(&self, src: &str) -> Result<QueryResult, QueryError> {
        self.eval(&parse(src)?)
    }

    /// Evaluates a parsed query.
    pub fn eval(&self, e: &Expr) -> Result<QueryResult, QueryError> {
        let mut env = Vec::new();
        let (v, extra) = self.eval_expr(e, &mut env)?;
        let nodes = match v {
            Value::Nodes(n) => n,
            Value::Changes(_) => {
                return Err(
                    QueryError::new(e.span, "this query evaluates to a change set")
                        .with_hint("use change sets as the first argument of affected(...)"),
                );
            }
        };
        let (path, affected) = match extra {
            Extra::None => (None, None),
            Extra::Path(p) => (Some(p), None),
            Extra::Affected(r) => (None, Some(r)),
        };
        Ok(QueryResult {
            nodes,
            path,
            affected,
        })
    }

    fn eval_expr(
        &self,
        e: &Expr,
        env: &mut Vec<(String, Value)>,
    ) -> Result<(Value, Extra), QueryError> {
        match &e.kind {
            ExprKind::Literal { text, .. } => {
                Ok((Value::Nodes(self.literal(text, e.span)?), Extra::None))
            }
            ExprKind::Var(name) => {
                let v = env
                    .iter()
                    .rev()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v)
                    .or_else(|| self.bindings.get(name))
                    .ok_or_else(|| {
                        let mut known: Vec<&str> = env
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .chain(self.bindings.keys().map(String::as_str))
                            .collect();
                        known.sort_unstable();
                        known.dedup();
                        let err = QueryError::new(e.span, format!("unbound variable `${name}`"));
                        if known.is_empty() {
                            err.with_hint("bind it with `let name = ... in ...`")
                        } else {
                            err.with_hint(format!("bound variables: ${}", known.join(", $")))
                        }
                    })?;
                Ok((v.clone(), Extra::None))
            }
            ExprKind::Let { name, value, body } => {
                let (v, _) = self.eval_expr(value, env)?;
                env.push((name.clone(), v));
                let r = self.eval_expr(body, env);
                env.pop();
                r
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let (l, _) = self.eval_expr(lhs, env)?;
                let (r, _) = self.eval_expr(rhs, env)?;
                match (l, r) {
                    (Value::Nodes(mut a), Value::Nodes(b)) => {
                        match op {
                            SetOp::Union => a.union_with(&b),
                            SetOp::Difference => a.difference_with(&b),
                            SetOp::Intersection => a.intersect_with(&b),
                        }
                        Ok((Value::Nodes(a), Extra::None))
                    }
                    (l, r) => {
                        if *op != SetOp::Union {
                            return Err(QueryError::new(
                                e.span,
                                "only `+` is defined on change sets",
                            ));
                        }
                        let mut cs = self.to_changes(l);
                        cs.extend(self.to_changes(r));
                        Ok((Value::Changes(cs), Extra::None))
                    }
                }
            }
            ExprKind::Call {
                name,
                name_span,
                args,
            } => self.call(name, *name_span, e.span, args, env),
        }
    }

    fn to_changes(&self, v: Value) -> ChangeSet {
        match v {
            Value::Changes(c) => c,
            Value::Nodes(n) => ChangeSet {
                files: n
                    .iter()
                    .map(|n| Change::new(ChangeKind::Modified, self.g.path(n)))
                    .collect(),
                ..ChangeSet::default()
            },
        }
    }

    fn nodes(
        &self,
        e: &Expr,
        env: &mut Vec<(String, Value)>,
    ) -> Result<(NodeSet, Extra), QueryError> {
        match self.eval_expr(e, env)? {
            (Value::Nodes(n), x) => Ok((n, x)),
            (Value::Changes(_), _) => Err(QueryError::new(
                e.span,
                "expected a set of files, found a change set",
            )
            .with_hint("change sets are only valid as the first argument of affected(...)")),
        }
    }

    fn literal(&self, text: &str, span: SrcSpan) -> Result<NodeSet, QueryError> {
        let g = self.g;
        let mut out = self.empty();
        if let Some(spec) = text.strip_prefix("pkg:") {
            let m = self
                .package_matcher(spec)
                .map_err(|m| QueryError::new(span, m))?;
            if !m.iter().any(|&b| b) {
                return Err(QueryError::new(
                    span,
                    format!("no package matches `{spec}`"),
                ));
            }
            for i in 0..g.node_count() {
                if m[g.package(NodeId(i)).0 as usize] {
                    out.insert(NodeId(i));
                }
            }
            return Ok(out);
        }
        if is_glob(text) {
            let glob = Glob::new(crate::mem::normalize(text))
                .map_err(|e| QueryError::new(span, format!("invalid glob: {}", e.message)))?;
            g.glob(&glob, &mut out);
            return Ok(out);
        }
        match g.node_by_path(text) {
            Some(n) => {
                out.insert(n);
                Ok(out)
            }
            None => {
                let path = crate::mem::normalize(text);
                let err = QueryError::new(span, format!("no file `{path}` in the graph"));
                let dir_glob = if path.is_empty() {
                    "**".to_owned()
                } else {
                    format!("{path}/**")
                };
                let mut below = self.empty();
                if let Ok(gl) = Glob::new(&dir_glob) {
                    g.glob(&gl, &mut below);
                }
                Err(if !below.is_empty() {
                    err.with_hint(format!(
                        "`{path}` is a directory with {} files; use `{dir_glob}`",
                        below.len()
                    ))
                } else if text.parse::<u64>().is_ok() {
                    err.with_hint("numbers are only valid as depth arguments")
                } else {
                    err.with_hint("for a path that does not exist yet, use added(\"...\")")
                })
            }
        }
    }

    fn word<'e>(&self, e: &'e Expr, what: &str) -> Result<&'e str, QueryError> {
        match &e.kind {
            ExprKind::Literal { text, .. } => Ok(text),
            _ => Err(QueryError::new(e.span, format!("expected {what}"))),
        }
    }

    fn names<T>(
        &self,
        e: &Expr,
        what: &str,
        parse_one: impl Fn(&str) -> Option<T>,
        valid: impl Iterator<Item = &'static str>,
    ) -> Result<Vec<T>, QueryError> {
        let text = self.word(e, what)?;
        let valid: Vec<_> = valid.collect();
        text.split('|')
            .map(|part| {
                parse_one(part).ok_or_else(|| {
                    let mut err = QueryError::new(e.span, format!("unknown {what} `{part}`"));
                    err = match closest(part, &valid) {
                        Some(s) => err.with_hint(format!("did you mean `{s}`?")),
                        None => err.with_hint(format!("expected one of: {}", valid.join(", "))),
                    };
                    err
                })
            })
            .collect()
    }

    // Invariant: the parser checked the function's arity.
    #[allow(clippy::expect_used)]
    fn call(
        &self,
        name: &str,
        name_span: SrcSpan,
        span: SrcSpan,
        args: &[Expr],
        env: &mut Vec<(String, Value)>,
    ) -> Result<(Value, Extra), QueryError> {
        let arity = |lo: usize, hi: usize, sig: &str| -> Result<(), QueryError> {
            if args.len() < lo || args.len() > hi {
                Err(QueryError::new(
                    span,
                    format!(
                        "`{name}` takes {}, got {}",
                        if lo == hi {
                            format!("{lo} argument{}", if lo == 1 { "" } else { "s" })
                        } else if hi == usize::MAX {
                            format!("at least {lo} argument{}", if lo == 1 { "" } else { "s" })
                        } else {
                            format!("{lo} to {hi} arguments")
                        },
                        args.len()
                    ),
                )
                .with_hint(format!("usage: {sig}")))
            } else {
                Ok(())
            }
        };
        let nodes = |v: NodeSet| Ok((Value::Nodes(v), Extra::None));
        match name {
            "deps" | "rdeps" => {
                arity(1, 2, &format!("{name}(set [, depth])"))?;
                let (x, _) = self.nodes(&args[0], env)?;
                let depth = match args.get(1) {
                    None => None,
                    Some(a) => {
                        let w = self.word(a, "a depth (a non-negative integer)")?;
                        Some(w.parse::<u32>().map_err(|_| {
                            QueryError::new(
                                a.span,
                                format!("expected a depth (a non-negative integer), found `{w}`"),
                            )
                        })?)
                    }
                };
                nodes(if name == "deps" {
                    self.deps(&x, depth)
                } else {
                    self.rdeps(&x, depth)
                })
            }
            "somepath" => {
                arity(2, 2, "somepath(from, to)")?;
                let (a, _) = self.nodes(&args[0], env)?;
                let (b, _) = self.nodes(&args[1], env)?;
                match self.somepath(&a, &b) {
                    Some(p) => {
                        let set = NodeSet::from_iter_in(self.g.node_count(), p.iter().copied());
                        Ok((Value::Nodes(set), Extra::Path(p)))
                    }
                    None => nodes(self.empty()),
                }
            }
            "allpaths" => {
                arity(2, 2, "allpaths(from, to)")?;
                let (a, _) = self.nodes(&args[0], env)?;
                let (b, _) = self.nodes(&args[1], env)?;
                nodes(self.allpaths(&a, &b))
            }
            "kind" | "phase" | "flags" => {
                arity(2, 2, &format!("{name}(name[|name...], set)"))?;
                let f = match name {
                    "kind" => EdgeFilter::kinds(self.names(
                        &args[0],
                        "reference kind",
                        RefKind::parse,
                        RefKind::names(),
                    )?),
                    "phase" => EdgeFilter::phases(self.names(
                        &args[0],
                        "phase",
                        Phase::parse,
                        Phase::names(),
                    )?),
                    _ => {
                        let flags =
                            self.names(&args[0], "flag", RefFlags::parse, RefFlags::all_names())?;
                        EdgeFilter {
                            require_any_flag: flags.into_iter().fold(RefFlags::NONE, |a, b| a | b),
                            ..EdgeFilter::ALL
                        }
                    }
                };
                let (x, _) = self.nodes(&args[1], env)?;
                nodes(self.filter_by_incoming(&x, &f))
            }
            "lang" => {
                arity(2, 2, "lang(name[|name...], set)")?;
                let text = self.word(&args[0], "a language id")?;
                let langs: Vec<&str> = text.split('|').collect();
                let (mut x, _) = self.nodes(&args[1], env)?;
                x.retain(|n| self.g.lang(n).is_some_and(|l| langs.contains(&l)));
                nodes(x)
            }
            "pkg" => {
                arity(2, 2, "pkg(name[@version], set)")?;
                let spec = self.word(&args[0], "a package name")?;
                let m = self
                    .package_matcher(spec)
                    .map_err(|m| QueryError::new(args[0].span, m))?;
                let (mut x, _) = self.nodes(&args[1], env)?;
                x.retain(|n| m[self.g.package(n).0 as usize]);
                nodes(x)
            }
            "external" => {
                arity(1, 1, "external(set)")?;
                let (mut x, _) = self.nodes(&args[0], env)?;
                x.retain(|n| self.g.is_external(n));
                nodes(x)
            }
            "uncertain" => {
                arity(1, 2, "uncertain([kind[|kind...],] set)")?;
                let kinds = if args.len() == 2 {
                    Some(self.names(
                        &args[0],
                        "uncertainty kind",
                        UncertaintyKind::parse,
                        UncertaintyKind::names(),
                    )?)
                } else {
                    None
                };
                let (mut x, _) = self.nodes(args.last().expect("arity"), env)?;
                let mut keep = self.empty();
                for &n in self.uncertain_nodes() {
                    if x.contains(n)
                        && kinds
                            .as_ref()
                            .is_none_or(|ks| self.g.uncertainties(n).any(|u| ks.contains(&u.kind)))
                    {
                        keep.insert(n);
                    }
                }
                x.intersect_with(&keep);
                nodes(x)
            }
            "affected" => {
                arity(
                    2,
                    3,
                    "affected(changes, entries [, affected|unknown|ignore])",
                )?;
                let (c, _) = self.eval_expr(&args[0], env)?;
                let changes = self.to_changes(c);
                let (entries, _) = self.nodes(&args[1], env)?;
                let mut opts = self.opts.affected.clone();
                if let Some(p) = args.get(2) {
                    let w = self.word(p, "a policy (affected, unknown or ignore)")?;
                    opts.policy = Policy::parse(w).ok_or_else(|| {
                        QueryError::new(p.span, format!("unknown policy `{w}`"))
                            .with_hint("expected one of: affected, unknown, ignore")
                    })?;
                }
                let report = self.affected(&changes, &entries, &opts);
                let set = NodeSet::from_iter_in(
                    self.g.node_count(),
                    report.with_verdict(Verdict::Affected).map(|r| r.entry),
                );
                Ok((Value::Nodes(set), Extra::Affected(report)))
            }
            "added" | "modified" | "deleted" => {
                arity(1, usize::MAX, &format!("{name}(path, ...)"))?;
                let kind = match name {
                    "added" => ChangeKind::Added,
                    "modified" => ChangeKind::Modified,
                    _ => ChangeKind::Deleted,
                };
                let mut cs = ChangeSet::new();
                for a in args {
                    self.change_arg(kind, a, env, &mut cs)?;
                }
                Ok((Value::Changes(cs), Extra::None))
            }
            _ => {
                let err = QueryError::new(name_span, format!("unknown function `{name}`"));
                Err(match closest(name, FUNCTIONS) {
                    Some(s) => err.with_hint(format!("did you mean `{s}`?")),
                    None => err.with_hint(format!("functions: {}", FUNCTIONS.join(", "))),
                })
            }
        }
    }

    fn change_arg(
        &self,
        kind: ChangeKind,
        a: &Expr,
        env: &mut Vec<(String, Value)>,
        cs: &mut ChangeSet,
    ) -> Result<(), QueryError> {
        if let ExprKind::Literal { text, .. } = &a.kind {
            if let Some(spec) = text.strip_prefix("pkg:") {
                if kind == ChangeKind::Added {
                    return Err(QueryError::new(
                        a.span,
                        "a package cannot be added; use modified(pkg:...)",
                    ));
                }
                let m = self
                    .package_matcher(spec)
                    .map_err(|m| QueryError::new(a.span, m))?;
                let before = cs.packages.len();
                cs.packages.extend(
                    m.iter()
                        .enumerate()
                        .filter(|(_, b)| **b)
                        .map(|(i, _)| PackageId(i as u32)),
                );
                if cs.packages.len() == before {
                    return Err(QueryError::new(
                        a.span,
                        format!("no package matches `{spec}`"),
                    ));
                }
                return Ok(());
            }
            if is_glob(text) {
                if kind == ChangeKind::Added {
                    return Err(QueryError::new(a.span, "added paths cannot be globs")
                        .with_hint("list the added files explicitly"));
                }
            } else {
                cs.push(kind, text.as_str());
                return Ok(());
            }
        }
        let (set, _) = self.nodes(a, env)?;
        for n in set.iter() {
            cs.push(kind, self.g.path(n));
        }
        Ok(())
    }
}

/// Every function name, for suggestions.
pub const FUNCTIONS: &[&str] = &[
    "deps",
    "rdeps",
    "somepath",
    "allpaths",
    "kind",
    "phase",
    "flags",
    "lang",
    "pkg",
    "external",
    "uncertain",
    "affected",
    "added",
    "modified",
    "deleted",
];

/// The cause of a node that looked for `probe` below the changed path of `c`.
fn below_cause(c: &Change, probe: &str) -> Cause {
    match c.kind {
        ChangeKind::Added => Cause::Probe {
            added: c.path.clone(),
            probe: probe.to_owned(),
        },
        ChangeKind::Deleted => Cause::Deleted {
            path: c.path.clone(),
        },
        ChangeKind::Modified => Cause::Modified {
            path: c.path.clone(),
        },
    }
}

fn closest<'a>(s: &str, candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .map(|c| (levenshtein(s, c), *c))
        .filter(|(d, c)| *d <= 2.max(c.len() / 3))
        .min()
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, &cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

const _: () = assert!(std::mem::size_of::<EdgeAttr>() == 2);
