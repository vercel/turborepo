//! Bounded loads: what a module load with an unknown target can bring in.
//!
//! A module load whose target cannot be evaluated (`require(x)`, `import(spec)`
//! with a value the analysis does not follow) was an open uncertainty: nothing
//! says which file it loads, so under the default policy every entry that
//! reaches it was selected on every change. The record now needs no
//! `--on-uncertain` verdict ([`Disposition::Declared`]):
//!
//! > **The rule.** You must not require, import, or read files outside of the
//! > packages
//! > you depend on: a file loads only files of its own package and of the
//! > packages that
//! > package declares ([`LoadReach::of`]: one step).
//!
//! Whatever lies further away is reached through edges the graph knows, or
//! through another such load, which again takes one step. So a load with an
//! unknown target, in whatever file it is (the test, a shared helper, a
//! framework), selects an entry that reaches it only when the change *affects*
//! a package that one step from the load reaches:
//!
//! 1. A bounded load *fires* when a package it can reach in one step is
//!    affected. Every file that reaches a fired load is affected in turn, and
//!    so is its package (a fixpoint: the induction over a chain of loads).
//! 2. An entry is selected when it reaches a load that fires for a package its
//!    own package depends on (the declared closure, [`LoadReach::closure`]).
//!    The package of a helper does not widen the bound of an entry: what the
//!    helper's package declares counts only as far as the entry's package
//!    depends on it.
//! 3. An entry that reaches a bounded load is also selected when a package that
//!    its own package reaches in one step is affected: a load whose base is the
//!    test's package rather than the loading file (a runner API, the working
//!    directory).
//!
//! A file is affected when it reaches a seed of the change through the graph's
//! edges; a package is affected when it holds a changed path or an affected
//! file, or is the importer a third-party package changed for. Only files
//! count: a build task or a membership hub is a node of its package too, but no
//! load brings it in (the walks still cross them: a file that imports what a
//! task generates is affected through the task).
//!
//! Edges are never cut. A file that is known to require, import or read a file
//! of a package its package does not declare keeps its edge, and the edge is a
//! finding ([`Engine::undeclared_dependencies`]): the rule is broken where the
//! graph can see it, and the selection assumes it where the graph cannot.
//!
//! # What is proved and what is assumed
//!
//! > **Resolution.** With a strict `node_modules` layout (one directory of
//! > links per
//! > package, as pnpm installs it), a specifier resolved from a file of package
//! > `P` names
//! > a file of `P` itself, of a package that `P` or a package around `P`
//! > declares (any
//! > dependency kind that is installed; the root's dependencies resolve from
//! > every
//! > package by walking up), or of a package nested in one of those. This holds
//! > whatever
//! > the string is, so also for a computed specifier.
//!
//! So for a load *by specifier* in such a layout the rule is a theorem, and by
//! induction over a chain of loads, each resolved from the file the previous
//! one brought in, so is everything the three points above conclude from it.
//!
//! A load by a *path* is not module resolution: an absolute path, a path joined
//! from the working directory, a string that a module mapper or an alias
//! pattern rewrites to a path, a loader of a third-party package that resolves
//! from somewhere else. For those the rule is an assumption
//! ([`AffectedOptions::loads_declared`]): the path stays inside the package of
//! the loading file or of the entry, or a package one of them declares. It
//! is wrong for a harness that is handed the directory of a package that
//! neither its own package nor the test's declares, when nothing in the graph
//! says so. Loads whose target the code locates are not concerned: they are
//! edges (a literal, a path followed from the working directory of the test).
//! `--strict` does not make the assumption: it opens these loads altogether.
//!
//! The step starts where the load happens. A derived layer can move a load's
//! record to the file that supplies its value (cross-file binding: the record
//! then sits on the caller and says `a value passed to <file>:<line>`,
//! [`PASSED_TO`]); the module system still resolves the value from the file
//! with the load, so that file's package is the one the step starts at
//! ([`Engine::loading_files`]).
//!
//! # The declared closure
//!
//! Without [`AffectedOptions::loads_step`] a load is held only to what the
//! entry's package depends on, however many steps away: an entry that reaches a
//! bounded load is selected when the change affects any package in the declared
//! closure of its package. That is the weaker reading of the rule ("the
//! packages you depend on" as everything reachable through declarations) and
//! assumes less for a computed path; it frees little, because most changes
//! affect some package that a test's package depends on somewhere. It is the
//! A/B switch of the step (`TURBO_TRACE_DEBUG_LOADS_STEP=0`). It does not look
//! at where a record sits.
//!
//! # A change of the bound itself
//!
//! What a package declares is read from the manifests of the tree that is
//! evaluated. A change of a manifest is a changed path of its package, so that
//! package is affected: the loads in its files fire, and so do the loads in
//! files of the packages that declare it (with the declared closure: every
//! entry whose package depends on it is selected when it reaches a bounded
//! load). That covers an added or a removed declaration, which changes what one
//! step reaches only from the declaring package, and the closure only of
//! packages that depend on it. Two changes reach further, and the caller names
//! them ([`Engine::bounded_loads`], `lent`): the manifest of a package in whose
//! directory other packages live lends its declarations to them (resolution
//! walks up; the root lends to every package) without being one of their
//! dependencies, and the install layout decides what is linked for every
//! package. A third one the caller names the same way: a package
//! that the change removes. The tree has no manifest that can name it and its
//! files lie in no package (a deleted path counts for the nearest package that
//! is left), so the packages that could resolve it before the change count as
//! affected by its removal.
//!
//! # What the bound does not cover
//!
//! A load by a computed path into a package that neither the loading file's
//! package nor the entry's declares (the assumption above).
//!
//! A load that the graph has no record of: a specifier handed to a third-party
//! loader resolves from that loader's place in the package store, where pnpm
//! links every workspace package (the hidden hoist), so it can name any of
//! them. That is a hole of the load model with or without the bound; but an
//! entry that also reached an open load used to be selected on every change and
//! so was covered by accident, and no longer is. The runtime oracle counts this
//! case apart in every run ("lost with the floor").
//!
//! A runner that finds packages by name below its root directory whatever is
//! declared (jest's haste packages, for unscoped names), from a file of a
//! package other than the root package. A layout in which packages resolve what
//! they do not declare: the caller decides whether the bound applies at all (a
//! hoisted workspace keeps these loads open).

use std::collections::HashMap;

use crate::{
    change::{
        AffectedOptions, Change, ChangeSet, Disposition, EntryResult, Policy, UncertainSite,
        Verdict,
    },
    engine::Engine,
    set::NodeSet,
    view::{GraphView, NodeId, PackageId, PackageVersion},
    vocab::{EdgeFilter, RefFlags, RefKind, UncertaintyKind},
};

const NONE: u32 = u32::MAX;

/// How the detail of a record ends that a derived layer put on the file that
/// *supplies* the value of a load in another file (cross-file binding: `…; a
/// value passed to <file>:<line>`). The module system resolves the load from
/// the file named here, not from the file with the record
/// ([`Engine::loading_files`]).
pub const PASSED_TO: &str = "; a value passed to ";

/// What a load from a file of each first-party package can reach in one step:
/// the package itself, the packages nested in it, the packages that it or a
/// package around it declares, and what is nested in those. Built by the caller
/// from the manifests (the query crate reads no files).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadReach {
    reach: Vec<Vec<u32>>,
}

impl LoadReach {
    /// No package reaches another one.
    pub fn new(package_count: u32) -> Self {
        LoadReach {
            reach: vec![Vec::new(); package_count as usize],
        }
    }

    /// A load from a file of `from` can reach `to`.
    pub fn add(&mut self, from: PackageId, to: PackageId) {
        let n = self.reach.len();
        if let Some(v) = self.reach.get_mut(from.0 as usize)
            && (to.0 as usize) < n
        {
            v.push(to.0);
        }
    }

    /// Sorts and deduplicates; every package reaches itself.
    pub fn finish(&mut self) {
        for (p, v) in self.reach.iter_mut().enumerate() {
            v.push(p as u32);
            v.sort_unstable();
            v.dedup();
        }
    }

    /// The packages a load from a file of `p` can reach (`p` included after
    /// [`LoadReach::finish`]).
    pub fn of(&self, p: PackageId) -> &[u32] {
        self.reach.get(p.0 as usize).map_or(&[], Vec::as_slice)
    }

    /// The declared closure of `p`: every package reachable from it step by
    /// step, `p` included (`true` at the package's id). This is what the
    /// package depends on.
    pub fn closure(&self, p: PackageId) -> Vec<bool> {
        let mut seen = vec![false; self.reach.len()];
        let mut stack = Vec::new();
        if let Some(s) = seen.get_mut(p.0 as usize) {
            *s = true;
            stack.push(p.0);
        }
        while let Some(x) = stack.pop() {
            for &q in &self.reach[x as usize] {
                if !seen[q as usize] {
                    seen[q as usize] = true;
                    stack.push(q);
                }
            }
        }
        seen
    }

    /// The packages whose declared closure holds a package of `hit` (`true` at
    /// its id).
    pub fn dependents(&self, hit: &[bool]) -> Vec<bool> {
        let n = self.reach.len();
        let mut rev: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (p, v) in self.reach.iter().enumerate() {
            for &q in v {
                rev[q as usize].push(p as u32);
            }
        }
        let mut seen = vec![false; n];
        let mut stack: Vec<u32> = Vec::new();
        for (p, &h) in hit.iter().enumerate().take(n) {
            if h {
                seen[p] = true;
                stack.push(p as u32);
            }
        }
        while let Some(x) = stack.pop() {
            for &p in &rev[x as usize] {
                if !seen[p as usize] {
                    seen[p as usize] = true;
                    stack.push(p);
                }
            }
        }
        seen
    }

    /// `(from << 32) | to` for every pair, for a cache.
    pub fn to_pairs(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for (p, v) in self.reach.iter().enumerate() {
            for &q in v {
                out.push(((p as u64) << 32) | u64::from(q));
            }
        }
        out
    }

    /// From [`LoadReach::to_pairs`]. `None`: a pair names a package the graph
    /// does not have.
    pub fn from_pairs(package_count: u32, pairs: &[u64]) -> Option<Self> {
        let mut out = LoadReach::new(package_count);
        for &x in pairs {
            let (p, q) = ((x >> 32) as u32, x as u32);
            if p >= package_count || q >= package_count {
                return None;
            }
            out.reach[p as usize].push(q);
        }
        out.finish();
        Some(out)
    }
}

/// Why a unit is affected.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Why {
    /// A changed path (index into the paths) lies in it.
    Changed(u32),
    /// A third-party package changed for it.
    Package,
    /// A changed path (index into the paths) alters what it resolves: the
    /// manifest of a package above it, whose declarations it resolves
    /// through, or the install layout.
    Resolves(u32),
    /// One of its files reaches a seed of the change.
    Reaches(u32),
    /// One of its files (the first id) reaches a fired load (the second).
    Fired(u32, u32),
}

/// One bounded load site with the entries that reach it, for the uncertainty
/// report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedSite {
    /// The file with the load.
    pub node: NodeId,
    /// Its bounded records: line, kind, detail.
    pub records: Vec<(u32, UncertaintyKind, String)>,
    /// Entries that reach it.
    pub reached: usize,
    /// What one resolution step reaches from the site: first the package of the
    /// file where the load happens ([`Engine::loading_files`]).
    pub reach: Vec<PackageId>,
    /// The packages of the entries that reach it.
    pub entry_packages: usize,
    /// That package, when it is one (the root package's entries are counted
    /// apart: `root_entries`).
    pub entry_package: Option<PackageId>,
    /// The first-party packages those depend on (their declared closures
    /// together, the root package's left out): a change has to affect one
    /// of them for the site to select an entry.
    pub covers: usize,
    /// Some of the entries are files of the root package, whose directory is
    /// the repository: they depend on every package and are not part of
    /// `covers`.
    pub root_entries: bool,
}

/// The bounded loads of a graph for a set of entries
/// ([`Engine::bounded_sites`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundedSites {
    /// Bounded records in first-party files.
    pub records: usize,
    /// The sites that at least one entry reaches, most reached first.
    pub sites: Vec<BoundedSite>,
    /// Entries that reach at least one bounded load.
    pub entries: usize,
    /// Entries that reach a bounded load or an open site: the floor without the
    /// bound.
    pub floor_without: usize,
    /// First-party packages of the graph.
    pub packages: usize,
    /// Whether the entries were walked ([`Engine::bounded_sites`]): without it
    /// only the records are known ([`Engine::bounded_records`]).
    pub walked: bool,
}

/// Statistics of one [`Engine::bounded_loads`] evaluation.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadStats {
    /// Files with a bounded load.
    pub sites: usize,
    /// Of those, the ones the change can reach.
    pub fired: usize,
    /// Entries that reach a bounded load and that nothing else selected.
    pub candidates: usize,
    /// Of those, selected through a fired load.
    pub by_site: usize,
    /// Of those, selected through the entry part.
    pub by_entry: usize,
    /// Affected units.
    pub affected_units: usize,
}

impl<'g, G: GraphView> Engine<'g, G> {
    /// The first-party nodes with a load that the declared packages bound under
    /// `opts` ([`Disposition::Declared`]).
    pub fn declared_sites(&self, opts: &AffectedOptions) -> Vec<NodeId> {
        let g = self.graph();
        let mut out = Vec::new();
        for &n in self.uncertain_nodes() {
            if g.is_external(n) {
                continue;
            }
            if g.uncertainties(n)
                .any(|u| self.disposition_of(n, &u, opts) == Disposition::Declared)
            {
                out.push(n);
            }
        }
        out
    }

    /// The files where the bounded loads of `site` happen: `site` itself, or,
    /// for a record that sits on the file that supplies the value
    /// ([`PASSED_TO`]), the file with the load. One resolution step starts
    /// at the package of these files; where the record sits says who passes
    /// the value, not where it is resolved.
    pub fn loading_files(&self, site: NodeId, opts: &AffectedOptions) -> Vec<NodeId> {
        let g = self.graph();
        let mut out = Vec::new();
        for u in g.uncertainties(site) {
            if self.disposition_of(site, &u, opts) != Disposition::Declared {
                continue;
            }
            let sink = u
                .detail
                .rsplit_once(PASSED_TO)
                .and_then(|(_, at)| at.rsplit_once(':'))
                .and_then(|(path, _)| g.node_by_path(path));
            let f = sink.unwrap_or(site);
            if !out.contains(&f) {
                out.push(f);
            }
        }
        if out.is_empty() {
            out.push(site);
        }
        out
    }

    /// Reverse reachability from `starts` over the edges `filter` accepts: the
    /// parent of every reached node (a start is its own parent), [`NONE`]
    /// elsewhere, and the reached nodes in visit order.
    fn load_reach_up(&self, starts: &[NodeId], filter: EdgeFilter) -> (Vec<u32>, Vec<u32>) {
        let rev = self.reverse();
        let every = filter.is_all();
        let mut parent = vec![NONE; self.graph().node_count() as usize];
        let mut queue: Vec<u32> = Vec::with_capacity(starts.len());
        for s in starts {
            if parent[s.index()] == NONE {
                parent[s.index()] = s.0;
                queue.push(s.0);
            }
        }
        let mut head = 0;
        while head < queue.len() {
            let n = queue[head];
            head += 1;
            let (srcs, attrs) = rev.neighbors(NodeId(n));
            for (&s, &a) in srcs.iter().zip(attrs) {
                if parent[s as usize] == NONE && (every || filter.accepts(a)) {
                    parent[s as usize] = n;
                    queue.push(s);
                }
            }
        }
        (parent, queue)
    }

    /// The package of a changed path: of its node, or of the nearest
    /// first-party package directory around it (`roots`: package root → id;
    /// the root package is `""`).
    fn package_of_path(&self, roots: &HashMap<&str, u32>, path: &str) -> Option<u32> {
        let g = self.graph();
        if let Some(n) = g.node_by_path(path)
            && !g.is_external(n)
        {
            return Some(g.package(n).0);
        }
        let mut dir = path.trim_matches('/');
        loop {
            dir = dir.rsplit_once('/').map_or("", |x| x.0);
            if let Some(&p) = roots.get(dir) {
                return Some(p);
            }
            if dir.is_empty() {
                return None;
            }
        }
    }

    /// The entries that the change selects through loads with an unknown target
    /// (see the [module documentation](self)), among the entries that are
    /// not in `selected`. One result per entry, in node order, with the
    /// policy's verdict and the load as the uncertain site; its text says
    /// which package the change affects and why.
    ///
    /// `changes` is what the engine evaluated (its seeds are the change);
    /// `paths` are the changed paths that count for the bound, whatever a
    /// caller did with them (a manifest applied field by field is still a
    /// change in its package). `lent`: `(index into paths, package)` for a
    /// path that changes what a package it does not lie in can
    /// resolve (`reach` is the head's: a declaration that a package above
    /// lends, the install layout); that package counts as affected too.
    /// `runtime_entries`: the entries evaluated without type-only edges.
    #[allow(clippy::too_many_arguments)]
    pub fn bounded_loads(
        &self,
        reach: &LoadReach,
        changes: &ChangeSet,
        paths: &[Change],
        lent: &[(u32, PackageId)],
        entries: &NodeSet,
        runtime_entries: Option<&NodeSet>,
        selected: &NodeSet,
        opts: &AffectedOptions,
    ) -> (Vec<EntryResult>, LoadStats) {
        let g = self.graph();
        let mut stats = LoadStats::default();
        let mut out = Vec::new();
        if opts.policy == Policy::Ignore {
            return (out, stats);
        }
        let sites = self.declared_sites(opts);
        stats.sites = sites.len();
        if sites.is_empty() {
            return (out, stats);
        }
        let verdict = match opts.policy {
            Policy::Affected => Verdict::Affected,
            _ => Verdict::Unknown,
        };
        let np = g.package_count() as usize;
        let mut roots: HashMap<&str, u32> = HashMap::new();
        for p in 0..np as u32 {
            let info = g.package_info(PackageId(p));
            if matches!(info.version, PackageVersion::Workspace) {
                let r = info.root.trim_matches('/');
                roots.entry(if r == "." { "" } else { r }).or_insert(p);
            }
        }
        // What a load can target: a first-party file. A build task or a membership hub
        // is a node of its package too, but no load brings it in; the walks
        // still cross them (a file that imports what a task generates is
        // affected through the task).
        let is_file =
            |n: NodeId| !g.is_external(n) && !g.lang(n).is_some_and(|l| l.starts_with("task"));
        let base = self.options().traversal;
        let runtime = base.without_flags(RefFlags::TYPE_ONLY);
        let empty = NodeSet::new(g.node_count());
        let rt = runtime_entries.unwrap_or(&empty);
        // How an entry walks, as in the engine's passes: without type-only edges, with
        // every edge, or as a build task (`crate::taskmode`: no byte reads, no
        // listing-only links). What a change affects is taken with the wider
        // filter for tasks.
        let mode_of = |n: NodeId| -> u8 {
            if crate::taskmode::is_task(g, n) {
                2
            } else if rt.contains(n) {
                0
            } else {
                1
            }
        };
        let modes = [
            (runtime, runtime, 0u8),
            (base, base, 1),
            (crate::taskmode::filter(base), base, 2),
        ];
        for (walk, filter, mode) in modes {
            if !entries
                .iter()
                .any(|n| mode_of(n) == mode && !selected.contains(n))
            {
                continue;
            }
            // Entries of this mode that reach a bounded load and that nothing selected.
            let (to_site, reached) = self.load_reach_up(&sites, walk.for_uncertainty());
            let candidates: Vec<NodeId> = reached
                .iter()
                .map(|&n| NodeId(n))
                .filter(|&n| entries.contains(n) && !selected.contains(n) && mode_of(n) == mode)
                .collect();
            if candidates.is_empty() {
                continue;
            }
            stats.candidates += candidates.len();
            // Affected units.
            let mut why: Vec<Option<Why>> = vec![None; np];
            let every = !changes.packages.is_empty();
            for (i, c) in paths.iter().enumerate() {
                if let Some(p) = self.package_of_path(&roots, &c.path) {
                    why[p as usize].get_or_insert(Why::Changed(i as u32));
                }
            }
            for &(i, p) in lent {
                if let Some(w) = why.get_mut(p.0 as usize)
                    && (i as usize) < paths.len()
                {
                    w.get_or_insert(Why::Resolves(i));
                }
            }
            for s in &changes.scoped {
                for i in &s.importers {
                    if let Some(w) = why.get_mut(i.0 as usize) {
                        w.get_or_insert(Why::Package);
                    }
                }
            }
            let (seeds, _) = self.seeds(changes, opts.after_change, filter);
            let seed_nodes: Vec<NodeId> = seeds.iter().map(|s| s.0).collect();
            let (_, changed) = self.load_reach_up(&seed_nodes, filter);
            for &n in &changed {
                let n = NodeId(n);
                if is_file(n) {
                    why[g.package(n).0 as usize].get_or_insert(Why::Reaches(n.0));
                }
            }
            // The loads that fire, to a fixpoint.
            let hit = |why: &[Option<Why>], p: PackageId| -> Option<u32> {
                reach
                    .of(p)
                    .iter()
                    .copied()
                    .chain(std::iter::once(p.0))
                    .find(|&q| why[q as usize].is_some())
            };
            // Where each site's loads are resolved from (the step only).
            let loaders: Vec<Vec<NodeId>> = if opts.loads_step {
                sites.iter().map(|&s| self.loading_files(s, opts)).collect()
            } else {
                Vec::new()
            };
            let mut fired = NodeSet::new(g.node_count());
            let mut fired_by: HashMap<u32, u32> = HashMap::new();
            let mut to_fired = vec![NONE; g.node_count() as usize];
            let rev = self.reverse();
            let unc = walk.for_uncertainty();
            let all_edges = unc.is_all();
            // The packages that depend on an affected one, before any load fires: the rule.
            let affected: Vec<bool> = why.iter().map(Option::is_some).collect();
            let dependents = reach.dependents(&affected);
            loop {
                if !opts.loads_step {
                    break;
                }
                let mut new: Vec<u32> = Vec::new();
                for (&s, from) in sites.iter().zip(&loaders) {
                    if fired.contains(s) {
                        continue;
                    }
                    let q = if every {
                        Some(g.package(s).0)
                    } else {
                        from.iter().find_map(|&f| hit(&why, g.package(f)))
                    };
                    if let Some(q) = q {
                        fired.insert(s);
                        fired_by.insert(s.0, q);
                        new.push(s.0);
                    }
                }
                if new.is_empty() {
                    break;
                }
                // Files that reach a newly fired load, and their units.
                let mut queue: Vec<u32> = Vec::new();
                for &s in &new {
                    if to_fired[s as usize] == NONE {
                        to_fired[s as usize] = s;
                        queue.push(s);
                    }
                }
                let mut head = 0;
                while head < queue.len() {
                    let n = queue[head];
                    head += 1;
                    let (srcs, attrs) = rev.neighbors(NodeId(n));
                    for (&s, &a) in srcs.iter().zip(attrs) {
                        if to_fired[s as usize] == NONE && (all_edges || unc.accepts(a)) {
                            to_fired[s as usize] = n;
                            queue.push(s);
                        }
                    }
                }
                for &n in &queue {
                    let node = NodeId(n);
                    if !is_file(node) {
                        continue;
                    }
                    let slot = &mut why[g.package(node).0 as usize];
                    if slot.is_none() {
                        let mut site = n;
                        while to_fired[site as usize] != site {
                            site = to_fired[site as usize];
                        }
                        *slot = Some(Why::Fired(n, site));
                    }
                }
            }
            stats.fired = stats.fired.max(fired.len());
            stats.affected_units = stats
                .affected_units
                .max(why.iter().filter(|w| w.is_some()).count());
            let describe = |q: u32| -> String {
                let info = g.package_info(PackageId(q));
                let name = if info.root.is_empty() || info.root == "." {
                    "the root package".to_owned()
                } else {
                    format!("{}/", info.root.trim_matches('/'))
                };
                match why[q as usize] {
                    Some(Why::Changed(i)) => {
                        format!("{name} ({} changed)", paths[i as usize].path)
                    }
                    Some(Why::Resolves(i)) => format!(
                        "{name} ({} changed what it resolves)",
                        paths[i as usize].path
                    ),
                    Some(Why::Package) => {
                        format!("{name} (a third-party package it installs changed)")
                    }
                    Some(Why::Reaches(n)) => {
                        format!("{name} ({} depends on the change)", g.path(NodeId(n)))
                    }
                    Some(Why::Fired(n, site)) => format!(
                        "{name} ({} reaches the load in {}, which the change can reach)",
                        g.path(NodeId(n)),
                        g.path(NodeId(site))
                    ),
                    None => name,
                }
            };
            let chain = |parent: &[u32], n: NodeId| -> Vec<NodeId> {
                let mut c = vec![n];
                let mut n = n.0;
                while parent[n as usize] != n {
                    n = parent[n as usize];
                    c.push(NodeId(n));
                }
                c.reverse();
                c
            };
            // Per entry package: its declared closure (what it depends on).
            let mut closures: HashMap<u32, Vec<bool>> = HashMap::new();
            for e in candidates {
                let p = g.package(e);
                let closure = closures.entry(p.0).or_insert_with(|| reach.closure(p));
                let (witness, text) = if every {
                    stats.by_entry += 1;
                    (
                        chain(&to_site, e),
                        "a third-party package changed for the whole repository".to_owned(),
                    )
                } else if g.is_external(e) {
                    continue;
                } else if !opts.loads_step {
                    // The declared closure: the entry's package depends on an affected
                    // package.
                    if !dependents.get(p.0 as usize).copied().unwrap_or(false) {
                        continue;
                    }
                    let Some(q) =
                        (0..np as u32).find(|&q| closure[q as usize] && affected[q as usize])
                    else {
                        continue;
                    };
                    stats.by_entry += 1;
                    (
                        chain(&to_site, e),
                        format!("this entry's package depends on {}", describe(q)),
                    )
                } else if to_fired[e.index()] != NONE
                    && fired_by
                        .get(&chain(&to_fired, e)[0].0)
                        .is_some_and(|&q| closure[q as usize])
                {
                    // A load that fires for a package the entry's package depends on (a
                    // helper's own declarations do not widen what the entry may load).
                    stats.by_site += 1;
                    let witness = chain(&to_fired, e);
                    let q = fired_by.get(&witness[0].0).copied().unwrap_or(0);
                    (witness, format!("the load can reach {}", describe(q)))
                } else if let Some(q) = reach
                    .of(p)
                    .iter()
                    .copied()
                    .chain(std::iter::once(p.0))
                    .find(|&q| match why[q as usize] {
                        // Affected through a load that fired: for a package the entry's
                        // package depends on.
                        Some(Why::Fired(_, site)) => {
                            fired_by.get(&site).is_some_and(|&by| closure[by as usize])
                        }
                        Some(_) => true,
                        None => false,
                    })
                {
                    stats.by_entry += 1;
                    (
                        chain(&to_site, e),
                        format!("a load of this entry's package can reach {}", describe(q)),
                    )
                } else {
                    continue;
                };
                let site = witness[0];
                let reasons = g
                    .uncertainties(site)
                    .filter(|u| self.disposition_of(site, u, opts) == Disposition::Declared)
                    .map(|u| (u.kind, u.line, u.detail.to_owned()))
                    .collect();
                out.push(EntryResult {
                    entry: e,
                    verdict,
                    witness,
                    cause: None,
                    uncertain: Some(UncertainSite {
                        node: site,
                        reasons,
                        bound: Some(text),
                    }),
                });
            }
        }
        out.sort_unstable_by_key(|r| r.entry.0);
        (out, stats)
    }

    /// The bounded loads of the graph with their records, by path, without any
    /// walk: what the uncertainty report shows by default
    /// ([`BoundedSites::walked`] is false and the counts of entries are
    /// zero).
    pub fn bounded_records(&self, opts: &AffectedOptions) -> BoundedSites {
        let g = self.graph();
        let mut report = BoundedSites::default();
        for s in self.declared_sites(opts) {
            let records: Vec<(u32, UncertaintyKind, String)> = g
                .uncertainties(s)
                .filter(|u| self.disposition_of(s, u, opts) == Disposition::Declared)
                .map(|u| (u.line, u.kind, u.detail.to_owned()))
                .collect();
            report.records += records.len();
            report.sites.push(BoundedSite {
                node: s,
                records,
                reached: 0,
                reach: vec![g.package(s)],
                entry_packages: 0,
                entry_package: None,
                covers: 0,
                root_entries: false,
            });
        }
        report
            .sites
            .sort_by(|a, b| g.path(a.node).cmp(g.path(b.node)));
        report
    }

    /// The bounded loads that `entries` reach, with how many entries reach each
    /// and which packages a change has to affect for it to select them, and
    /// the floor these entries would have without the bound (the
    /// uncertainty report with `--loads`). One reachability walk per site
    /// and mode over the files that reach it, on the reverse adjacency: a
    /// report on request, not part of a query.
    pub fn bounded_sites(
        &self,
        reach: &LoadReach,
        entries: &NodeSet,
        runtime_entries: Option<&NodeSet>,
        opts: &AffectedOptions,
    ) -> BoundedSites {
        let g = self.graph();
        let mut report = BoundedSites {
            walked: true,
            ..BoundedSites::default()
        };
        let sites = self.declared_sites(opts);
        if sites.is_empty() {
            return report;
        }
        let base = self.options().traversal;
        let empty = NodeSet::new(g.node_count());
        let rt = runtime_entries.unwrap_or(&empty);
        let mode_of = |n: NodeId| -> usize {
            if crate::taskmode::is_task(g, n) {
                2
            } else if rt.contains(n) {
                0
            } else {
                1
            }
        };
        let modes = [
            base.without_flags(RefFlags::TYPE_ONLY).for_uncertainty(),
            base.for_uncertainty(),
            crate::taskmode::filter(base).for_uncertainty(),
        ];
        let mut has = [false; 3];
        for e in entries.iter() {
            has[mode_of(e)] = true;
        }
        // One stamp array for every walk: a walk costs what it visits.
        let rev = self.reverse();
        let mut stamp = vec![0u32; g.node_count() as usize];
        let mut tick = 0u32;
        let mut queue: Vec<u32> = Vec::new();
        let mut walk = |starts: &[NodeId], f: &mut dyn FnMut(NodeId)| {
            for (mode, filter) in modes.iter().enumerate() {
                if !has[mode] {
                    continue;
                }
                tick += 1;
                queue.clear();
                for s in starts {
                    if stamp[s.index()] != tick {
                        stamp[s.index()] = tick;
                        queue.push(s.0);
                    }
                }
                let every = filter.is_all();
                let mut head = 0;
                while head < queue.len() {
                    let n = NodeId(queue[head]);
                    head += 1;
                    if entries.contains(n) && mode_of(n) == mode {
                        f(n);
                    }
                    let (srcs, attrs) = rev.neighbors(n);
                    for (&x, &a) in srcs.iter().zip(attrs) {
                        if stamp[x as usize] != tick && (every || filter.accepts(a)) {
                            stamp[x as usize] = tick;
                            queue.push(x);
                        }
                    }
                }
            }
        };
        let mut floor = NodeSet::new(g.node_count());
        walk(&sites, &mut |e| {
            floor.insert(e);
        });
        report.entries = floor.len();
        // The floor without the bound: these entries and the ones on the open floor.
        let exempt = g.resolved_entries();
        let (mut open, mut per_entry): (Vec<NodeId>, Vec<NodeId>) = (Vec::new(), Vec::new());
        for &n in self.uncertain_nodes() {
            if g.is_external(n) {
                continue;
            }
            let (mut is_open, mut resolved) = (false, false);
            for u in g.uncertainties(n) {
                if self.disposition_of(n, &u, opts) != Disposition::Open {
                    continue;
                }
                if exempt.is_some() && g.is_entry_resolved(n, &u) {
                    resolved = true;
                } else {
                    is_open = true;
                    break;
                }
            }
            if is_open {
                open.push(n);
            } else if resolved {
                per_entry.push(n);
            }
        }
        walk(&open, &mut |e| {
            floor.insert(e);
        });
        walk(&per_entry, &mut |e| {
            if !exempt.is_some_and(|x| x.contains(e)) {
                floor.insert(e);
            }
        });
        report.floor_without = floor.len();
        report.packages = (0..g.package_count())
            .filter(|&q| {
                matches!(
                    g.package_info(PackageId(q)).version,
                    PackageVersion::Workspace
                )
            })
            .count();
        for &s in &sites {
            let records: Vec<(u32, UncertaintyKind, String)> = g
                .uncertainties(s)
                .filter(|u| self.disposition_of(s, u, opts) == Disposition::Declared)
                .map(|u| (u.line, u.kind, u.detail.to_owned()))
                .collect();
            report.records += records.len();
            let mut reached = 0;
            let mut of: Vec<u32> = Vec::new();
            walk(&[s], &mut |e| {
                reached += 1;
                of.push(g.package(e).0);
            });
            if reached == 0 {
                continue;
            }
            of.sort_unstable();
            of.dedup();
            let is_root = |p: u32| {
                let r = g.package_info(PackageId(p)).root.trim_matches('/');
                r.is_empty() || r == "."
            };
            let root_entries = of.iter().any(|&p| is_root(p));
            of.retain(|&p| !is_root(p));
            let mut covered = vec![false; g.package_count() as usize];
            for &p in &of {
                if !covered[p as usize] {
                    for (q, c) in reach.closure(PackageId(p)).into_iter().enumerate() {
                        covered[q] |= c;
                    }
                }
            }
            let covers = (0..g.package_count())
                .filter(|&q| {
                    covered[q as usize]
                        && matches!(
                            g.package_info(PackageId(q)).version,
                            PackageVersion::Workspace
                        )
                })
                .count();
            // One step from where the loads are resolved: the loading files' packages
            // first.
            let mut pkgs: Vec<PackageId> = Vec::new();
            for f in self.loading_files(s, opts) {
                let own = g.package(f);
                if !pkgs.contains(&own) {
                    pkgs.push(own);
                }
            }
            for own in pkgs.clone() {
                for &q in reach.of(own) {
                    if !pkgs.contains(&PackageId(q)) {
                        pkgs.push(PackageId(q));
                    }
                }
            }
            report.sites.push(BoundedSite {
                node: s,
                records,
                reached,
                reach: pkgs,
                entry_packages: of.len(),
                entry_package: match of[..] {
                    [p] if !root_entries => Some(PackageId(p)),
                    _ => None,
                },
                covers,
                root_entries,
            });
        }
        report
            .sites
            .sort_by(|a, b| (b.reached, g.path(a.node)).cmp(&(a.reached, g.path(b.node))));
        report
    }
}

/// How an edge that leaves the declared packages is written.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Written {
    /// A package name (`@scope/pkg/sub`): resolves only through hoisting or a
    /// link that no manifest declares.
    PackageName,
    /// A relative path that leaves the package (`../../other/src/x`).
    RelativePath,
    /// An absolute path, or a path the analysis evaluated (`join(__dirname,
    /// '../..')`).
    Path,
    /// A specifier that a module mapper, an alias or `paths` rewrites.
    Mapped,
}

impl Written {
    /// `package name`, `relative path`, `path`, `mapper or alias`.
    pub fn as_str(self) -> &'static str {
        match self {
            Written::PackageName => "package name",
            Written::RelativePath => "relative path",
            Written::Path => "path",
            Written::Mapped => "mapper or alias",
        }
    }
}

/// A certain edge that breaks the rule "a file requires, imports or reads only
/// files of its own package and of the packages that package declares": the
/// lint finding ([`Engine::undeclared_dependencies`]). Such an edge is never
/// cut; it is reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndeclaredDependency {
    /// The file with the reference.
    pub from: NodeId,
    /// The file it requires, imports or reads.
    pub to: NodeId,
    /// 1-based line of the reference; 0 if unknown.
    pub line: u32,
    /// The specifier as written, if recorded.
    pub specifier: String,
    /// A read of the file's contents (`fs` read, asset, embed), not a module
    /// load.
    pub read: bool,
    /// A type-only import: erased before the code runs.
    pub type_only: bool,
    /// How the reference is written.
    pub written: Written,
    /// The target's package is declared further away (by a package that the
    /// source's package depends on) but not by the source's package itself:
    /// importable only by accident of the layout. `false`: no package the
    /// source depends on declares it.
    pub transitive: bool,
}

/// The findings of [`Engine::undeclared_dependencies`] with what was left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndeclaredDependencies {
    /// The findings, by file and line.
    pub findings: Vec<UndeclaredDependency>,
    /// Edges between first-party files of two packages that were checked.
    pub checked: usize,
    /// Left out: edges into the root package's own files (the root cannot be
    /// declared).
    pub into_root: usize,
    /// Left out: edges that a test runner draws for a test (its config, setup
    /// files, environment, mocks): not a reference of the file.
    pub runner: usize,
}

const RUNNERS: [&str; 6] = [
    "jest:",
    "vitest:",
    "pytest:",
    "gotest:",
    "cargo-test:",
    "playwright:",
];

impl<'g, G: GraphView> Engine<'g, G> {
    /// The certain edges that break the rule: a first-party JS/TS file that
    /// requires, imports or reads a file of a first-party package that its
    /// own package cannot reach in one step (`reach`: itself, what is
    /// nested in it, what it or a package around it declares). Input: the
    /// graph and the declared packages. Output: one finding per edge, by
    /// file and line, with how it is written and whether the target is at
    /// least declared further away; and the counts of what is left out
    /// (edges into the root package, edges a test runner draws, manifests that
    /// a resolution consulted).
    pub fn undeclared_dependencies(&self, reach: &LoadReach) -> UndeclaredDependencies {
        let g = self.graph();
        let mut out = UndeclaredDependencies::default();
        let is_file = |n: NodeId| {
            !g.is_external(n)
                && !g.lang(n).is_some_and(|l| l.starts_with("task"))
                && !g.path(n).split('/').next().is_some_and(|s| s.contains(':'))
        };
        let first_party =
            |p: PackageId| matches!(g.package_info(p).version, PackageVersion::Workspace);
        let is_root = |p: PackageId| {
            let r = g.package_info(p).root.trim_matches('/');
            r.is_empty() || r == "."
        };
        let mut closures: HashMap<u32, Vec<bool>> = HashMap::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            if g.lang(n) != Some("js") || !is_file(n) {
                continue;
            }
            let p = g.package(n);
            if !first_party(p) || is_root(p) {
                continue;
            }
            let one_step = reach.of(p);
            for e in g.out_edges(n) {
                let Some(t) = e.to else { continue };
                if t == n || !is_file(t) || e.kind() == RefKind::Manifest {
                    continue;
                }
                let q = g.package(t);
                if q == p || !first_party(q) {
                    continue;
                }
                out.checked += 1;
                if one_step.binary_search(&q.0).is_ok() {
                    continue;
                }
                let spec = e.specifier.unwrap_or("");
                if RUNNERS.iter().any(|r| spec.starts_with(r))
                    && !spec.contains("moduleNameMapper")
                    && !spec.contains(".alias")
                {
                    out.runner += 1;
                    continue;
                }
                if is_root(q) {
                    out.into_root += 1;
                    continue;
                }
                let written = if RUNNERS.iter().any(|r| spec.starts_with(r)) {
                    Written::Mapped
                } else if spec.starts_with('.') {
                    Written::RelativePath
                } else if spec.starts_with('/') || spec.is_empty() {
                    Written::Path
                } else {
                    let name = g.package_info(q).name;
                    if spec == name || spec.strip_prefix(name).is_some_and(|r| r.starts_with('/')) {
                        Written::PackageName
                    } else {
                        Written::Mapped
                    }
                };
                let closure = closures.entry(p.0).or_insert_with(|| reach.closure(p));
                out.findings.push(UndeclaredDependency {
                    from: n,
                    to: t,
                    line: e.line,
                    specifier: spec.to_owned(),
                    read: matches!(
                        e.kind(),
                        RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset | RefKind::Embed
                    ),
                    type_only: e.flags().contains(RefFlags::TYPE_ONLY),
                    written,
                    transitive: closure[q.0 as usize],
                });
            }
        }
        out.findings.sort_by(|a, b| {
            (g.path(a.from), a.line, g.path(a.to)).cmp(&(g.path(b.from), b.line, g.path(b.to)))
        });
        out.findings.dedup();
        out
    }
}
