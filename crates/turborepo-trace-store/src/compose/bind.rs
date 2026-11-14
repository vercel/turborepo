//! Bound dependencies: [`Graph::bound_patch`].
//!
//! A language's cross-file binding (for JS / TS:
//! `turborepo_trace_lang_js::xfile`) finds that a value a file reads or loads
//! is supplied by another file: a caller passes the path, or the
//! key that selects it from a table. The dependency then belongs to the
//! supplier, and the record or the over-approximating edges on the file that
//! contains the site are no longer needed. No shard can hold this: the callers
//! of a function are in every shard. Like the per-test working directories
//! ([`super::cwd`]) it is derived from the composed graph, as a [`Patch`]:
//!
//! | Bound dependency of a supplier | Added to the supplier |
//! |---|---|
//! | a path from the repository root | what a reference to it resolves to (as in [`super::cwd`]): an edge, a config input, or a probe when it is missing; module candidates for a module load |
//! | something below a directory | the files below it and a `**` membership (reads); an open record (module loads) |
//! | a module specifier of the site's file | edges to what the site's file resolves that specifier to (its stored edges); relative specifiers are resolved against the site's directory |
//! | relative to the working directory | a `cwd_dependent` record |
//! | a module relative to the working directory, evaluated for the tests' working directories | a `cwd_dependent` record that is entry-resolved ([`super::cwd`]): it counts for entries without a known working directory only |
//! | unknown | an open record with the origin of the unknown part |
//!
//! A closed site with a parameter-origin record loses that record. A closed
//! *resolved* site (its stored references already cover every value, such as
//! every value of a lookup table) loses the edges the bound dependencies
//! replace: the edges of the site itself and of references inside a closed
//! table's literal, for the listed specifiers, to targets that the file does
//! not also reach in another way.
//!
//! "Exists" is decided against the snapshot's walked-file list, so the patch is
//! a function of the composed graph and the bound dependencies.

use std::collections::{HashMap, HashSet};

use turborepo_trace_core::{Phase, RefFlags, RefKind, UncertaintyKind};
use turborepo_trace_graph::{self as q, EdgeAttr, GraphView, NodeId, NodeSet, Patch};

use super::{Graph, NONE, cwd::SiteRef};
use crate::paths::join_repo;

/// What a bound dependency points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundTarget {
    /// A path from the repository root (normalized, no leading slash).
    Path(String),
    /// Something below this directory (repository-relative).
    Below(String),
    /// A path relative to the process working directory.
    Cwd(String),
    /// A module specifier, as the site's file resolves it (or, when it has no
    /// reference with that specifier, as the supplier's own references do).
    Spec(String),
    /// A module relative to the working directory that the binding evaluated
    /// for every working directory tests run in: a record that does not
    /// count for those tests.
    PerTestCwd(String),
    /// Not known.
    Open {
        /// Where the unknown part comes from.
        origin: turborepo_trace_core::Origin,
        /// What is unknown.
        detail: String,
    },
}

/// A dependency of `node` that follows from a value it supplies to a site in
/// another file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundDep {
    /// The file that supplies the value.
    pub node: NodeId,
    /// The line where it does (0: not known).
    pub line: u32,
    /// What the site does with the value.
    pub kind: RefKind,
    /// The file of the site.
    pub sink: NodeId,
    /// The line of the site.
    pub sink_line: u32,
    /// The target.
    pub target: BoundTarget,
}

/// A site whose record or edges the bound dependencies replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedSite {
    /// The file of the site.
    pub node: NodeId,
    /// The span of the site's uncertainty record.
    pub at: [u32; 2],
    /// The span of the site's references.
    pub site: [u32; 2],
    /// Drop the parameter-origin record. Otherwise: cut the site's edges for
    /// `specs`.
    pub open: bool,
    /// Specifiers whose edges are replaced.
    pub specs: Vec<String>,
}

/// A table literal whose string values are only selected by suppliers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTable {
    /// The file.
    pub node: NodeId,
    /// The span of the literal.
    pub at: [u32; 2],
    /// Its string values.
    pub specs: Vec<String>,
}

/// What [`Graph::bound_patch`] did.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundStats {
    /// Dependencies applied.
    pub deps: usize,
    /// Parameter-origin records dropped.
    pub dropped: usize,
    /// Node pairs cut.
    pub cuts: usize,
    /// Open records added to suppliers.
    pub opened: usize,
}

fn is_module_kind(k: RefKind) -> bool {
    matches!(
        k,
        RefKind::Require
            | RefKind::DynamicImport
            | RefKind::Import
            | RefKind::ReExport
            | RefKind::Worker
            | RefKind::ChildProcess
    )
}

/// Where replaced edges are: at a site (its span), or inside a table literal
/// (its span), with the specifiers that are replaced.
type Rule<'a> = (Option<[u32; 2]>, [u32; 2], &'a [String]);

fn loads_module(k: q::RefKind) -> bool {
    matches!(
        k,
        q::RefKind::Require
            | q::RefKind::DynamicImport
            | q::RefKind::Import
            | q::RefKind::ReExport
            | q::RefKind::Worker
            | q::RefKind::ChildProcess
    )
}

/// The import specifier a framework alias edge rewrites (`jest:moduleNameMapper
/// <spec> -> <request>`, see `fw::alias_edges`): such an edge belongs to every
/// reference of the file with that specifier.
fn aliased_spec(label: &str) -> Option<&str> {
    let (left, _) = label.split_once(" -> ")?;
    let (head, spec) = left.rsplit_once(' ')?;
    head.contains(':').then_some(spec)
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Whether a node reaches a file with a `cwd_dependent` record (itself
/// included), answered for the nodes that are asked about: a search of the
/// node's closure, which stops at the first such file. The nodes of a search
/// that found none reach none either and are kept, so the searches together
/// visit each node once. (The set of all nodes that reach such a file takes the
/// reverse of every edge; a patch asks about the modules it binds.)
struct CwdReach<'g> {
    g: &'g Graph,
    cwd_kind: usize,
    /// Nodes known to reach a record, and known not to.
    yes: NodeSet,
    no: NodeSet,
    /// Scratch of one search.
    seen: NodeSet,
}

impl<'g> CwdReach<'g> {
    fn new(g: &'g Graph) -> Self {
        let n = g.nodes.len() as u32;
        CwdReach {
            g,
            cwd_kind: crate::convert::UNCERTAINTY_KINDS
                .iter()
                .position(|k| *k == UncertaintyKind::CwdDependent)
                .unwrap_or(usize::MAX),
            yes: NodeSet::new(n),
            no: NodeSet::new(n),
            seen: NodeSet::new(n),
        }
    }

    /// Whether the file of node `x` has a `cwd_dependent` record.
    fn record(&self, x: u32) -> bool {
        let node = &self.g.nodes[x as usize];
        node.shard != NONE
            && self.g.shards[node.shard as usize].get().files[node.file as usize]
                .uncertainties
                .iter()
                .any(|u| u.kind as usize == self.cwd_kind)
    }

    fn reaches(&mut self, t: u32) -> bool {
        if t as usize >= self.g.nodes.len() {
            return false;
        }
        if self.yes.contains(NodeId(t)) {
            return true;
        }
        if self.no.contains(NodeId(t)) {
            return false;
        }
        let mut visited: Vec<u32> = vec![t];
        let mut stack: Vec<u32> = vec![t];
        self.seen.insert(NodeId(t));
        let mut found = false;
        while let Some(x) = stack.pop() {
            if self.yes.contains(NodeId(x)) || self.record(x) {
                found = true;
                break;
            }
            for &y in self.g.fwd.neighbors(NodeId(x)).0 {
                // A node known to reach none adds nothing; one known to reach one is
                // found when it is popped.
                if !self.no.contains(NodeId(y)) && self.seen.insert(NodeId(y)) {
                    visited.push(y);
                    stack.push(y);
                }
            }
        }
        for x in &visited {
            self.seen.remove(NodeId(*x));
            // A finished search without a record: none of its nodes reaches one.
            if !found {
                self.no.insert(NodeId(*x));
            }
        }
        if found {
            self.yes.insert(NodeId(t));
        }
        found
    }
}

impl Graph {
    /// The patch for bound dependencies (see the module documentation).
    pub fn bound_patch(
        &self,
        deps: &[BoundDep],
        closed: &[ClosedSite],
        tables: &[ClosedTable],
    ) -> (Patch, BoundStats) {
        let mut patch = Patch::new();
        let mut stats = BoundStats::default();
        if deps.is_empty() && closed.is_empty() {
            return (patch, stats);
        }
        let walked_buf = self.walked_files();
        let walked: Vec<&str> = match &walked_buf {
            Some(b) => crate::manifest::files_in(b).collect(),
            None => Vec::new(),
        };
        let has_walked = walked_buf.is_some();
        // Module loads that reach files the supplier did not reach before: per-test
        // resolutions of working-directory-relative references ([`super::cwd`]) were
        // computed without them.
        let mut cwd_reach = CwdReach::new(self);
        let mut reach_of: HashMap<u32, NodeSet> = HashMap::new();
        let mut seen: HashSet<(u32, u32, u8)> = HashSet::new();
        for d in deps {
            stats.deps += 1;
            let effect = if is_module_kind(d.kind) {
                q::Effect::Load
            } else {
                q::Effect::Read
            };
            let from = format!("{}:{}", self.path(d.sink), d.sink_line);
            let open = |patch: &mut Patch,
                        stats: &mut BoundStats,
                        kind: UncertaintyKind,
                        origin: turborepo_trace_core::Origin,
                        detail: &str| {
                stats.opened += 1;
                patch.add_uncertainty_from(
                    d.node,
                    crate::convert::query_unc_kind(
                        crate::convert::UNCERTAINTY_KINDS
                            .iter()
                            .position(|k| *k == kind)
                            .unwrap_or(0) as u8,
                    ),
                    d.line,
                    &format!("{detail}{}{from}", q::loads::PASSED_TO),
                    effect,
                    crate::convert::query_origin(
                        turborepo_trace_core::Origin::ALL
                            .iter()
                            .position(|o| *o == origin)
                            .unwrap_or(0) as u8,
                    ),
                );
            };
            let attr = crate::convert::edge_attr(d.kind, Phase::Runtime, RefFlags::empty());
            let label = format!(
                "bound: {} supplies the value of {from}{}",
                self.path(d.node),
                if d.line > 0 {
                    format!(" at line {}", d.line)
                } else {
                    String::new()
                }
            );
            // A repository path as a reference of the supplier.
            let resolve = |patch: &mut Patch,
                           seen: &mut HashSet<(u32, u32, u8)>,
                           kind: RefKind,
                           path: &str|
             -> Option<Vec<u32>> {
                if !has_walked {
                    return None;
                }
                let r = SiteRef {
                    kind,
                    attr,
                    specs: Some(vec![path.to_owned()]),
                    line: d.line,
                    unc_span: [0, 0],
                    js: true,
                };
                let res = self.resolve_in_cwd(&walked, "", &r)?;
                for &t in &res.edges {
                    if seen.insert((d.node.0, t, d.kind as u8)) {
                        patch.edge(d.node, NodeId(t), EdgeAttr(attr), &label);
                    }
                }
                for p in &res.probes {
                    patch.probe(d.node, p);
                }
                for p in &res.inputs {
                    patch.input(d.node, p);
                }
                for (dir, pat) in &res.members {
                    patch.membership(d.node, dir, pat);
                }
                Some(res.edges)
            };
            let mut targets: Vec<u32> = Vec::new();
            match &d.target {
                BoundTarget::Path(p) => match resolve(&mut patch, &mut seen, d.kind, p) {
                    Some(ts) => targets = ts,
                    None => open(
                        &mut patch,
                        &mut stats,
                        UncertaintyKind::ComputedSpecifier,
                        turborepo_trace_core::Origin::Unknown,
                        &format!("`{p}` cannot be resolved here"),
                    ),
                },
                BoundTarget::Below(dir) if !is_module_kind(d.kind) => {
                    // Reading something below a directory: every file below it.
                    if resolve(&mut patch, &mut seen, RefKind::FsRead, dir).is_none() {
                        open(
                            &mut patch,
                            &mut stats,
                            UncertaintyKind::PatternTooBroad,
                            turborepo_trace_core::Origin::Unknown,
                            &format!("something below `{dir}`"),
                        );
                    }
                }
                BoundTarget::Below(dir) => open(
                    &mut patch,
                    &mut stats,
                    UncertaintyKind::ComputedSpecifier,
                    turborepo_trace_core::Origin::Unknown,
                    &format!("a module below `{dir}`"),
                ),
                BoundTarget::Cwd(p) => open(
                    &mut patch,
                    &mut stats,
                    UncertaintyKind::CwdDependent,
                    turborepo_trace_core::Origin::Cwd,
                    &format!("`{p}` relative to the working directory"),
                ),
                BoundTarget::Spec(s) => {
                    // What the site's file resolves the specifier to.
                    let mut found = false;
                    for e in self.out_edges(d.sink) {
                        let named = e.specifier == Some(s.as_str())
                            || (e.span.is_none()
                                && e.specifier.and_then(aliased_spec) == Some(s.as_str()));
                        if named && loads_module(e.attr.kind()) {
                            found = true;
                            if let Some(t) = e.to
                                && seen.insert((d.node.0, t.0, d.kind as u8))
                            {
                                patch.edge(d.node, t, EdgeAttr(attr), &label);
                                targets.push(t.0);
                            }
                        }
                    }
                    // A specifier the supplier itself lists (a module string of a route
                    // table): its own references already are the dependency.
                    if !found
                        && !(s.starts_with("./") || s.starts_with("../"))
                        && self.out_edges(d.node).any(|e| {
                            loads_module(e.attr.kind())
                                && (e.specifier == Some(s.as_str())
                                    || e.specifier.and_then(aliased_spec) == Some(s.as_str()))
                        })
                    {
                        found = true;
                    }
                    if !found {
                        let rel = (s.starts_with("./") || s.starts_with("../"))
                            .then(|| join_repo(dir_of(self.path(d.sink)), s))
                            .flatten();
                        match rel.and_then(|p| resolve(&mut patch, &mut seen, d.kind, &p)) {
                            Some(ts) => targets = ts,
                            None => open(
                                &mut patch,
                                &mut stats,
                                UncertaintyKind::ComputedSpecifier,
                                turborepo_trace_core::Origin::Unknown,
                                &format!("module `{s}` cannot be resolved here"),
                            ),
                        }
                    }
                }
                BoundTarget::PerTestCwd(p) => {
                    let detail = format!(
                        "module `{p}` relative to the working directory, evaluated for the \
                         working directories of the tests{}{from}",
                        q::loads::PASSED_TO
                    );
                    let kind = crate::convert::query_unc_kind(
                        crate::convert::UNCERTAINTY_KINDS
                            .iter()
                            .position(|k| *k == UncertaintyKind::CwdDependent)
                            .unwrap_or(0) as u8,
                    );
                    stats.opened += 1;
                    patch.add_uncertainty_from(
                        d.node,
                        kind,
                        d.line,
                        &detail,
                        effect,
                        q::Origin::Cwd,
                    );
                    patch.resolve_for_entries(d.node, kind, d.line, None, &detail);
                }
                BoundTarget::Open { origin, detail } => {
                    let kind = if *origin == turborepo_trace_core::Origin::Env {
                        UncertaintyKind::EnvDependent
                    } else {
                        UncertaintyKind::ComputedSpecifier
                    };
                    open(&mut patch, &mut stats, kind, *origin, detail);
                }
            }
            // Loaded modules the supplier did not reach before, that reach a
            // working-directory-relative reference: its per-test resolution does not cover
            // the supplier's tests.
            if is_module_kind(d.kind) && !targets.is_empty() {
                let reaching: Vec<u32> = targets
                    .iter()
                    .copied()
                    .filter(|t| cwd_reach.reaches(*t))
                    .collect();
                if !reaching.is_empty() {
                    let reach = reach_of
                        .entry(d.node.0)
                        .or_insert_with(|| self.closure_of(d.node));
                    if reaching.iter().any(|t| !reach.contains(NodeId(*t))) {
                        open(
                            &mut patch,
                            &mut stats,
                            UncertaintyKind::CwdDependent,
                            turborepo_trace_core::Origin::Cwd,
                            "a bound module reaches working-directory-relative references",
                        );
                    }
                }
            }
        }
        // Closed sites: drop the parameter-origin record, keeping records of other
        // origins at the same place.
        let comp = crate::convert::query_unc_kind(
            crate::convert::UNCERTAINTY_KINDS
                .iter()
                .position(|k| *k == UncertaintyKind::ComputedSpecifier)
                .unwrap_or(0) as u8,
        );
        for c in closed.iter().filter(|c| c.open) {
            let at = q::Span {
                start: c.at[0],
                end: c.at[1],
            };
            let recs: Vec<q::Uncertainty<'_>> = self
                .uncertainties(c.node)
                .filter(|u| u.kind == comp && u.span == Some(at))
                .collect();
            let bound = |o: q::Origin| {
                matches!(o, q::Origin::Param | q::Origin::Call | q::Origin::Imported)
            };
            if !recs.iter().any(|u| bound(u.origin)) {
                continue;
            }
            for u in &recs {
                patch.drop_uncertainty(c.node, u.kind, u.line, u.span);
                if !bound(u.origin) {
                    patch
                        .add_uncertainty_from(c.node, u.kind, u.line, u.detail, u.effect, u.origin);
                }
            }
            stats.dropped += 1;
        }
        // Closed resolved sites and tables: cut the edges the bound dependencies
        // replace.
        let mut by_node: HashMap<u32, Vec<Rule<'_>>> = HashMap::new();
        for c in closed.iter().filter(|c| !c.open) {
            by_node
                .entry(c.node.0)
                .or_default()
                .push((Some(c.site), [0, 0], &c.specs));
        }
        for t in tables {
            // Only together with a closed site of the same file.
            if let Some(v) = by_node.get_mut(&t.node.0) {
                v.push((None, t.at, &t.specs));
            }
        }
        for (node, rules) in by_node {
            let n = NodeId(node);
            let replaced = |e: &q::Edge<'_>| -> bool {
                let (Some(span), Some(spec)) = (e.span, e.specifier) else {
                    return false;
                };
                loads_module(e.attr.kind())
                    && rules.iter().any(|(site, within, specs)| {
                        let place = match site {
                            Some(s) => span.start == s[0] && span.end == s[1],
                            None => span.start >= within[0] && span.end <= within[1],
                        };
                        // Alternatives of one reference are joined with ` | `.
                        place && spec.split(" | ").all(|x| specs.iter().any(|s| s == x))
                    })
            };
            // Specifiers the file also names in a reference that stays: their alias edges
            // stay too.
            let mut kept_specs: HashSet<&str> = HashSet::new();
            let mut replaced_specs: HashSet<&str> = HashSet::new();
            for e in self.out_edges(n) {
                if let (Some(_), Some(spec)) = (e.span, e.specifier) {
                    // Only a module load that stays keeps its specifier's alias edges (the
                    // manifest a site consulted to resolve a bare specifier does not).
                    let set = if replaced(&e) {
                        &mut replaced_specs
                    } else if loads_module(e.attr.kind()) {
                        &mut kept_specs
                    } else {
                        continue;
                    };
                    set.extend(spec.split(" | "));
                }
            }
            let alias_replaced = |e: &q::Edge<'_>| -> bool {
                e.span.is_none()
                    && e.specifier
                        .and_then(aliased_spec)
                        .is_some_and(|s| replaced_specs.contains(s) && !kept_specs.contains(s))
            };
            // A target is cut only when every edge to it is replaced.
            let mut keep: HashSet<u32> = HashSet::new();
            let mut cut: Vec<u32> = Vec::new();
            for e in self.out_edges(n) {
                let Some(t) = e.to else { continue };
                if replaced(&e) || alias_replaced(&e) {
                    if !cut.contains(&t.0) {
                        cut.push(t.0);
                    }
                } else {
                    keep.insert(t.0);
                }
            }
            for t in cut {
                if !keep.contains(&t) {
                    patch.cut(n, NodeId(t));
                    stats.cuts += 1;
                }
            }
        }
        (patch, stats)
    }

    /// What the composed graph is a function of (the binary, the manifest's
    /// shards and its walked files), as one number: the key for caches of
    /// facts derived from the graph, next to the composed graph's own
    /// (`cache/composed`).
    pub fn composed_key(&self) -> u128 {
        super::cache::key(&self.manifest)
    }

    /// The directory for caches derived from this graph ([`crate::cache_dir`]),
    /// if the graph was opened from a directory.
    pub fn cache_dir(&self) -> Option<&std::path::Path> {
        self.tables_cache.as_deref()?.parent()
    }

    /// The working directories the tests of the graph run in
    /// (repository-relative, sorted, distinct).
    pub fn test_cwds(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .tests_with_cwds()
            .values()
            .flatten()
            .map(|c| (*c).to_owned())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The nodes `from` reaches.
    fn closure_of(&self, from: NodeId) -> NodeSet {
        let mut set = NodeSet::new(self.nodes.len() as u32);
        let mut stack = vec![from.0];
        set.insert(from);
        while let Some(x) = stack.pop() {
            for &t in self.fwd.neighbors(NodeId(x)).0 {
                if set.insert(NodeId(t)) {
                    stack.push(t);
                }
            }
        }
        set
    }
}
