//! A [`GraphView`] with derived facts layered on a base graph: [`Patched`].
//!
//! Some dependencies cannot be stored on the file that contains the reference,
//! because they depend on which entry reaches it or on another file:
//!
//! - a path relative to the process working directory (`require(process.cwd() +
//!   '/src/routes')` in a shared test helper) names a different file for every
//!   test project that runs the helper;
//! - a loader wrapper (`export function load(p) { return require(p) }`) loads
//!   what its callers in other files pass;
//! - a hint declares dependencies of a file, or bounds what a dynamic site can
//!   load;
//! - a stored edge over-approximates what a file loads (every value of a lookup
//!   table) and the patch knows which files select which values: the edge is
//!   *cut* and the patch's edges on those files replace it ([`Patch::cut`]).
//!
//! A [`Patch`] holds such facts as plain graph data on existing nodes: extra
//! edges (with a label that explains them), probes, config inputs and directory
//! memberships; tags (an index of the nodes on which the layer found some fact,
//! [`GraphView::tagged`]); uncertainty records that are dropped (the patch's
//! edges and probes replace them) or added (a caller that passes an unknown
//! value); and *entry-resolved* records. An entry-resolved record stays on its
//! node, but it does not count for the entries in [`Patch::resolved_entries`]:
//! the patch added, on each of those entries, the edges and probes that
//! represent every target the reference can have when that entry
//! runs it ([`GraphView::is_entry_resolved`]). For every other entry the record
//! is as open as before.
//!
//! [`Patched`] implements [`GraphView`] over the base graph plus one patch,
//! with the same node ids, so the engine, the outputs and the exports work
//! unchanged.

use std::collections::{HashMap, HashSet};

use crate::{
    set::NodeSet,
    view::{
        Adjacency, Edge, GraphView, Membership, NodeId, PackageId, PackageInfo, Span, Uncertainty,
    },
    vocab::{EdgeAttr, Effect, Origin, UncertaintyKind},
};

/// Identifies one uncertainty record of a node: kind, line and span.
type UncKey = (u32, UncertaintyKind, u32, Option<Span>);

fn key_of(n: NodeId, u: &Uncertainty<'_>) -> UncKey {
    (n.0, u.kind, u.line, u.span)
}

/// An uncertainty record a patch adds to a node.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Added {
    kind: UncertaintyKind,
    line: u32,
    detail: String,
    effect: Effect,
    origin: Origin,
}

/// Derived facts to layer on a graph (see the module documentation).
#[derive(Clone, Debug, Default)]
pub struct Patch {
    /// Interned labels, paths, directories and patterns.
    strings: Vec<String>,
    string_ix: HashMap<String, u32>,
    /// `(from, to, attr, label)`.
    edges: Vec<(u32, u32, EdgeAttr, u32)>,
    /// `(node, path)`.
    probes: Vec<(u32, u32)>,
    /// `(node, path)`.
    inputs: Vec<(u32, u32)>,
    /// `(node, dir, pattern)`.
    memberships: Vec<(u32, u32, u32)>,
    /// `(node, tag)`.
    tags: Vec<(u32, u32)>,
    dropped: HashSet<UncKey>,
    added: HashMap<u32, Vec<Added>>,
    /// Entry-resolved records with their replacement detail text.
    resolved: HashMap<UncKey, String>,
    resolved_entries: Option<NodeSet>,
    /// `(from, to)`: every base edge between the two nodes is removed.
    cuts: Vec<(u32, u32)>,
}

/// Version of [`Patch::to_bytes`].
const PATCH_MAGIC: &[u8; 8] = b"DTPATCH2";

struct Enc(Vec<u8>);

impl Enc {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }
    fn key(&mut self, k: &UncKey) {
        self.u32(k.0);
        self.str(k.1.as_str());
        self.u32(k.2);
        match k.3 {
            Some(s) => {
                self.u32(1);
                self.u32(s.start);
                self.u32(s.end);
            }
            None => self.u32(0),
        }
    }
}

struct Dec<'a>(&'a [u8]);

impl<'a> Dec<'a> {
    fn u32(&mut self) -> Option<u32> {
        let (head, rest) = self.0.split_first_chunk::<4>()?;
        self.0 = rest;
        Some(u32::from_le_bytes(*head))
    }
    fn str(&mut self) -> Option<&'a str> {
        let n = self.u32()? as usize;
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        std::str::from_utf8(head).ok()
    }
    fn key(&mut self) -> Option<UncKey> {
        let n = self.u32()?;
        let kind = UncertaintyKind::parse(self.str()?)?;
        let line = self.u32()?;
        let span = match self.u32()? {
            0 => None,
            _ => Some(Span {
                start: self.u32()?,
                end: self.u32()?,
            }),
        };
        Some((n, kind, line, span))
    }
}

impl Patch {
    /// An empty patch.
    pub fn new() -> Self {
        Patch::default()
    }

    /// The patch as bytes, for a cache of a derived layer. Node ids are the
    /// graph's: the bytes are only valid for the graph they were computed
    /// from. Deterministic (sets and maps are written in sorted order).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut e = Enc(Vec::new());
        e.0.extend_from_slice(PATCH_MAGIC);
        e.u32(self.strings.len() as u32);
        for s in &self.strings {
            e.str(s);
        }
        e.u32(self.edges.len() as u32);
        for &(f, t, a, l) in &self.edges {
            e.u32(f);
            e.u32(t);
            e.u32(u32::from(a.0));
            e.u32(l);
        }
        for list in [&self.probes, &self.inputs] {
            e.u32(list.len() as u32);
            for &(n, p) in list {
                e.u32(n);
                e.u32(p);
            }
        }
        e.u32(self.memberships.len() as u32);
        for &(n, d, p) in &self.memberships {
            e.u32(n);
            e.u32(d);
            e.u32(p);
        }
        let key_order = |a: &&UncKey, b: &&UncKey| {
            (a.0, a.1.as_str(), a.2, a.3.map(|s| (s.start, s.end))).cmp(&(
                b.0,
                b.1.as_str(),
                b.2,
                b.3.map(|s| (s.start, s.end)),
            ))
        };
        let mut dropped: Vec<&UncKey> = self.dropped.iter().collect();
        dropped.sort_by(key_order);
        e.u32(dropped.len() as u32);
        for k in dropped {
            e.key(k);
        }
        let mut added: Vec<(&u32, &Vec<Added>)> = self.added.iter().collect();
        added.sort_by_key(|(n, _)| **n);
        e.u32(added.len() as u32);
        for (n, v) in added {
            e.u32(*n);
            e.u32(v.len() as u32);
            for a in v {
                e.str(a.kind.as_str());
                e.u32(a.line);
                e.str(&a.detail);
                e.str(a.effect.as_str());
                e.str(a.origin.as_str());
            }
        }
        let mut resolved: Vec<(&UncKey, &String)> = self.resolved.iter().collect();
        resolved.sort_by(|a, b| key_order(&a.0, &b.0));
        e.u32(resolved.len() as u32);
        for (k, d) in resolved {
            e.key(k);
            e.str(d);
        }
        match &self.resolved_entries {
            Some(set) => {
                e.u32(1);
                e.u32(set.universe());
                e.u32(set.len() as u32);
                for n in set.iter() {
                    e.u32(n.0);
                }
            }
            None => e.u32(0),
        }
        let mut tags = self.tags.clone();
        tags.sort_unstable();
        tags.dedup();
        e.u32(tags.len() as u32);
        for (n, t) in tags {
            e.u32(n);
            e.u32(t);
        }
        // Cut edges, as a trailing section that is only there when the patch cuts
        // something: a patch without cuts has the bytes it always had.
        if !self.cuts.is_empty() {
            let mut cuts = self.cuts.clone();
            cuts.sort_unstable();
            cuts.dedup();
            e.u32(cuts.len() as u32);
            for (f, t) in cuts {
                e.u32(f);
                e.u32(t);
            }
        }
        e.0
    }

    /// The patch [`Patch::to_bytes`] wrote; `None` for anything else.
    pub fn from_bytes(bytes: &[u8]) -> Option<Patch> {
        let mut d = Dec(bytes.strip_prefix(PATCH_MAGIC)?);
        let mut p = Patch::new();
        for _ in 0..d.u32()? {
            let s = d.str()?;
            p.intern(s);
        }
        for _ in 0..d.u32()? {
            let (f, t, a, l) = (d.u32()?, d.u32()?, d.u32()?, d.u32()?);
            p.edges.push((f, t, EdgeAttr(u16::try_from(a).ok()?), l));
        }
        for which in 0..2 {
            for _ in 0..d.u32()? {
                let pair = (d.u32()?, d.u32()?);
                if which == 0 {
                    p.probes.push(pair);
                } else {
                    p.inputs.push(pair);
                }
            }
        }
        for _ in 0..d.u32()? {
            p.memberships.push((d.u32()?, d.u32()?, d.u32()?));
        }
        for _ in 0..d.u32()? {
            p.dropped.insert(d.key()?);
        }
        for _ in 0..d.u32()? {
            let n = d.u32()?;
            let mut v = Vec::new();
            for _ in 0..d.u32()? {
                v.push(Added {
                    kind: UncertaintyKind::parse(d.str()?)?,
                    line: d.u32()?,
                    detail: d.str()?.to_owned(),
                    effect: Effect::parse(d.str()?)?,
                    origin: Origin::parse(d.str()?)?,
                });
            }
            p.added.insert(n, v);
        }
        for _ in 0..d.u32()? {
            let k = d.key()?;
            p.resolved.insert(k, d.str()?.to_owned());
        }
        if d.u32()? == 1 {
            let mut set = NodeSet::new(d.u32()?);
            for _ in 0..d.u32()? {
                set.insert(NodeId(d.u32()?));
            }
            p.resolved_entries = Some(set);
        }
        for _ in 0..d.u32()? {
            p.tags.push((d.u32()?, d.u32()?));
        }
        if !d.0.is_empty() {
            for _ in 0..d.u32()? {
                p.cuts.push((d.u32()?, d.u32()?));
            }
        }
        // Every string index must exist.
        let n = p.strings.len() as u32;
        let ok = p.edges.iter().all(|e| e.3 < n)
            && p.probes
                .iter()
                .chain(&p.inputs)
                .chain(&p.tags)
                .all(|x| x.1 < n)
            && p.memberships.iter().all(|m| m.1 < n && m.2 < n);
        (ok && d.0.is_empty()).then_some(p)
    }

    /// Whether the patch changes nothing.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
            && self.probes.is_empty()
            && self.inputs.is_empty()
            && self.memberships.is_empty()
            && self.tags.is_empty()
            && self.dropped.is_empty()
            && self.added.is_empty()
            && self.resolved.is_empty()
            && self.cuts.is_empty()
    }

    /// Interns a label, path, directory or pattern for the `*_id` methods
    /// (callers that add the same string for many nodes intern it once).
    pub fn intern(&mut self, s: &str) -> u32 {
        match self.string_ix.get(s) {
            Some(&i) => i,
            None => {
                let i = self.strings.len() as u32;
                self.strings.push(s.to_owned());
                self.string_ix.insert(s.to_owned(), i);
                i
            }
        }
    }

    /// Adds the edge `from → to` (`from` depends on `to`). `label` is shown
    /// where a specifier would be (`cwd services/a: ./src/routes
    /// (helper.ts:57)`).
    pub fn edge(&mut self, from: NodeId, to: NodeId, attr: EdgeAttr, label: &str) {
        let l = self.intern(label);
        self.edge_id(from, to, attr, l);
    }

    /// [`Patch::edge`] with an interned label.
    pub fn edge_id(&mut self, from: NodeId, to: NodeId, attr: EdgeAttr, label: u32) {
        if from != to {
            self.edges.push((from.0, to.0, attr, label));
        }
    }

    /// The edges the patch adds: `(from, to, attr)`.
    pub fn edges(&self) -> impl Iterator<Item = (NodeId, NodeId, EdgeAttr)> + '_ {
        self.edges
            .iter()
            .map(|(f, t, a, _)| (NodeId(*f), NodeId(*t), *a))
    }

    /// The edges the patch adds with their labels: `(from, to, attr, label)`.
    pub fn labeled_edges(&self) -> impl Iterator<Item = (NodeId, NodeId, EdgeAttr, &str)> + '_ {
        self.edges.iter().map(|(f, t, a, l)| {
            let label = self.strings.get(*l as usize).map_or("", String::as_str);
            (NodeId(*f), NodeId(*t), *a, label)
        })
    }

    /// The edges the patch adds with the ids of their labels
    /// ([`Patch::string`]): `(from, to, attr, label id)`. For a digest of
    /// the edges that does not look at each label once per edge.
    pub fn edge_ids(&self) -> impl Iterator<Item = (NodeId, NodeId, EdgeAttr, u32)> + '_ {
        self.edges
            .iter()
            .map(|(f, t, a, l)| (NodeId(*f), NodeId(*t), *a, *l))
    }

    /// The interned string `id` (a label of [`Patch::edge_ids`]); empty for an
    /// unknown id.
    pub fn string(&self, id: u32) -> &str {
        self.strings.get(id as usize).map_or("", String::as_str)
    }

    /// Number of interned strings: the ids of [`Patch::edge_ids`] are below it.
    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    /// Adds a probe: a missing path whose creation (or a file created below it)
    /// affects `n`.
    pub fn probe(&mut self, n: NodeId, path: &str) {
        let p = self.intern(path);
        self.probes.push((n.0, p));
    }

    /// [`Patch::probe`] with an interned path.
    pub fn probe_id(&mut self, n: NodeId, path: u32) {
        self.probes.push((n.0, path));
    }

    /// Adds a config input: a path whose change, creation or deletion affects
    /// `n`, whether or not it is a node.
    pub fn input(&mut self, n: NodeId, path: &str) {
        let p = self.intern(path);
        self.inputs.push((n.0, p));
    }

    /// [`Patch::input`] with an interned path.
    pub fn input_id(&mut self, n: NodeId, path: u32) {
        self.inputs.push((n.0, path));
    }

    /// Adds a directory membership: a file added to or deleted from `dir` that
    /// matches `pattern` affects `n`.
    pub fn membership(&mut self, n: NodeId, dir: &str, pattern: &str) {
        let (d, p) = (self.intern(dir), self.intern(pattern));
        self.memberships.push((n.0, d, p));
    }

    /// [`Patch::membership`] with an interned directory and pattern.
    pub fn membership_id(&mut self, n: NodeId, dir: u32, pattern: u32) {
        self.memberships.push((n.0, dir, pattern));
    }

    /// Drops the uncertainty record of `n` with this kind, line and span: the
    /// patch represents everything the reference can load.
    pub fn drop_uncertainty(
        &mut self,
        n: NodeId,
        kind: UncertaintyKind,
        line: u32,
        span: Option<Span>,
    ) {
        self.dropped.insert((n.0, kind, line, span));
    }

    /// Removes every base edge `from → to`. The caller adds the edges that
    /// replace it (on the files that really cause the load).
    pub fn cut(&mut self, from: NodeId, to: NodeId) {
        self.cuts.push((from.0, to.0));
    }

    /// Number of cut node pairs (before deduplication).
    pub fn cut_count(&self) -> usize {
        self.cuts.len()
    }

    /// Number of added uncertainty records.
    pub fn added_count(&self) -> usize {
        self.added.values().map(Vec::len).sum()
    }

    /// Adds an open uncertainty record to `n`, raised by a reference with
    /// `effect`.
    pub fn add_uncertainty(
        &mut self,
        n: NodeId,
        kind: UncertaintyKind,
        line: u32,
        detail: &str,
        effect: Effect,
    ) {
        self.add_uncertainty_from(n, kind, line, detail, effect, Origin::Unknown);
    }

    /// [`Patch::add_uncertainty`] with the origin of the unknown value.
    pub fn add_uncertainty_from(
        &mut self,
        n: NodeId,
        kind: UncertaintyKind,
        line: u32,
        detail: &str,
        effect: Effect,
        origin: Origin,
    ) {
        let v = self.added.entry(n.0).or_default();
        let a = Added {
            kind,
            line,
            detail: detail.to_owned(),
            effect,
            origin,
        };
        if !v.contains(&a) {
            v.push(a);
        }
    }

    /// Marks the uncertainty record of `n` with this kind, line and span as
    /// resolved for the entries of [`Patch::set_resolved_entries`].
    /// `detail` replaces its detail text (say how it was resolved, so
    /// reports and exports show it).
    pub fn resolve_for_entries(
        &mut self,
        n: NodeId,
        kind: UncertaintyKind,
        line: u32,
        span: Option<Span>,
        detail: &str,
    ) {
        self.resolved
            .insert((n.0, kind, line, span), detail.to_owned());
    }

    /// The entries for which entry-resolved records don't count.
    pub fn set_resolved_entries(&mut self, entries: NodeSet) {
        self.resolved_entries = Some(entries);
    }

    /// The entries for which entry-resolved records don't count.
    pub fn resolved_entries(&self) -> Option<&NodeSet> {
        self.resolved_entries.as_ref()
    }

    /// Number of extra edges (before deduplication).
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Number of entry-resolved records.
    pub fn resolved_count(&self) -> usize {
        self.resolved.len()
    }

    /// Number of dropped records.
    pub fn dropped_count(&self) -> usize {
        self.dropped.len()
    }

    /// Tags `n` with `tag` ([`GraphView::tagged`]). A layer that tags at all
    /// says that it looked at every node: add a tag even when nothing was
    /// found (on any node, under a name of its own), so that the view has
    /// an index and an absent tag means "no node".
    pub fn tag(&mut self, n: NodeId, tag: &str) {
        let t = self.intern(tag);
        self.tags.push((n.0, t));
    }

    /// Merges `other` into `self`. Node ids must come from the same graph. The
    /// entry sets of entry-resolved records are intersected when both
    /// patches have one (a record only stops counting for entries every
    /// patch resolved it for).
    pub fn merge(&mut self, other: Patch) {
        let map: Vec<u32> = other.strings.iter().map(|x| self.intern(x)).collect();
        for (f, t, a, l) in other.edges {
            self.edges.push((f, t, a, map[l as usize]));
        }
        for (n, p) in other.probes {
            self.probes.push((n, map[p as usize]));
        }
        for (n, p) in other.inputs {
            self.inputs.push((n, map[p as usize]));
        }
        for (n, d, p) in other.memberships {
            self.memberships.push((n, map[d as usize], map[p as usize]));
        }
        for (n, t) in other.tags {
            self.tags.push((n, map[t as usize]));
        }
        self.dropped.extend(other.dropped);
        for (n, v) in other.added {
            for a in v {
                self.add_uncertainty_from(NodeId(n), a.kind, a.line, &a.detail, a.effect, a.origin);
            }
        }
        self.resolved.extend(other.resolved);
        self.cuts.extend(other.cuts);
        self.resolved_entries = match (self.resolved_entries.take(), other.resolved_entries) {
            (Some(mut a), Some(b)) => {
                a.intersect_with(&b);
                Some(a)
            }
            (a, b) => a.or(b),
        };
    }
}

/// CSR lists per node over a table of strings.
#[derive(Default)]
struct PerNode<T> {
    off: Vec<u32>,
    items: Vec<T>,
}

impl<T: Clone> PerNode<T> {
    fn build(n: u32, mut pairs: Vec<(u32, T)>) -> Self
    where
        T: Ord,
    {
        pairs.sort();
        pairs.dedup();
        let mut off = vec![0u32; n as usize + 1];
        for (node, _) in &pairs {
            off[*node as usize + 1] += 1;
        }
        for i in 0..n as usize {
            off[i + 1] += off[i];
        }
        PerNode {
            off,
            items: pairs.into_iter().map(|(_, t)| t).collect(),
        }
    }

    fn get(&self, n: NodeId) -> &[T] {
        if self.off.is_empty() {
            return &[];
        }
        &self.items[self.off[n.index()] as usize..self.off[n.index() + 1] as usize]
    }
}

/// Node lists per string id (CSR), for the lookups by path.
#[derive(Default)]
struct ByString<T> {
    off: Vec<u32>,
    items: Vec<T>,
}

impl<T: Copy + Ord> ByString<T> {
    fn build(strings: usize, mut pairs: Vec<(u32, T)>) -> Self {
        pairs.sort_unstable();
        pairs.dedup();
        let mut off = vec![0u32; strings + 1];
        for (s, _) in &pairs {
            off[*s as usize + 1] += 1;
        }
        for i in 0..strings {
            off[i + 1] += off[i];
        }
        ByString {
            off,
            items: pairs.into_iter().map(|(_, t)| t).collect(),
        }
    }

    fn get(&self, s: u32) -> &[T] {
        &self.items[self.off[s as usize] as usize..self.off[s as usize + 1] as usize]
    }
}

/// A base graph with a [`Patch`] applied (see the module documentation).
pub struct Patched<'g, G: GraphView> {
    base: &'g G,
    strings: Vec<String>,
    string_ix: HashMap<String, u32>,
    /// Extra edges per node: `(to, attr, label)`.
    edges: PerNode<(u32, u16, u32)>,
    probes: PerNode<u32>,
    inputs: PerNode<u32>,
    memberships: PerNode<(u32, u32)>,
    /// Nodes per probed path, input path and membership directory (with the
    /// pattern).
    probe_ix: ByString<u32>,
    /// The ids of the probed paths in the order of the paths, for the lookup of
    /// what lies below a path ([`GraphView::probe_lookup_below`]).
    probe_sorted: std::sync::OnceLock<Vec<u32>>,
    input_ix: ByString<u32>,
    member_ix: ByString<(u32, u32)>,
    /// Nodes per tag.
    tag_ix: ByString<u32>,
    dropped: HashSet<UncKey>,
    added: HashMap<u32, Vec<Added>>,
    resolved: HashMap<UncKey, String>,
    resolved_entries: Option<NodeSet>,
    /// Nodes whose uncertainty records the patch touches.
    touched: NodeSet,
    /// The base adjacency plus the extra edges, minus the cut ones (`None`
    /// without extra or cut edges).
    fwd: Option<Adjacency>,
    /// Cut node pairs.
    cut: HashSet<(u32, u32)>,
    /// The nodes that have a cut edge.
    cut_from: NodeSet,
}

impl<'g, G: GraphView> Patched<'g, G> {
    /// `base` with `patch` applied.
    pub fn new(base: &'g G, patch: Patch) -> Self {
        let n = base.node_count();
        let mut touched = NodeSet::new(n);
        for k in patch.dropped.iter().chain(patch.resolved.keys()) {
            touched.insert(NodeId(k.0));
        }
        for k in patch.added.keys() {
            touched.insert(NodeId(*k));
        }
        let ns = patch.strings.len();
        let probe_ix = ByString::build(ns, patch.probes.iter().map(|&(x, p)| (p, x)).collect());
        let input_ix = ByString::build(ns, patch.inputs.iter().map(|&(x, p)| (p, x)).collect());
        // Per directory grouped by pattern, like the store's lookup (the engine matches
        // each run of one pattern once).
        let member_ix = ByString::build(
            ns,
            patch
                .memberships
                .iter()
                .map(|&(x, d, p)| (d, (p, x)))
                .collect(),
        );
        let tag_ix = ByString::build(ns, patch.tags.iter().map(|&(x, t)| (t, x)).collect());
        let edges = PerNode::build(
            n,
            patch
                .edges
                .iter()
                .map(|&(f, t, a, l)| (f, (t, a.0, l)))
                .collect(),
        );
        let cut: HashSet<(u32, u32)> = patch.cuts.iter().copied().collect();
        // A cut concerns the edges of a few nodes: only their rows are looked at edge
        // by edge.
        let mut cut_from = NodeSet::new(n);
        for &(from, _) in &cut {
            if from < n {
                cut_from.insert(NodeId(from));
            }
        }
        let fwd = (!edges.items.is_empty() || !cut.is_empty()).then(|| {
            let owned;
            let b = match base.forward_adjacency() {
                Some(a) => a,
                None => {
                    owned = Adjacency::from_view(base);
                    &owned
                }
            };
            let (boff, bto, battr) = b.parts();
            let mut off = Vec::with_capacity(n as usize + 1);
            let mut to = Vec::with_capacity(bto.len() + edges.items.len());
            let mut attr = Vec::with_capacity(bto.len() + edges.items.len());
            off.push(0u32);
            for i in 0..n as usize {
                let (lo, hi) = (boff[i] as usize, boff[i + 1] as usize);
                if !cut_from.contains(NodeId(i as u32)) {
                    to.extend_from_slice(&bto[lo..hi]);
                    attr.extend_from_slice(&battr[lo..hi]);
                } else {
                    for k in lo..hi {
                        if !cut.contains(&(i as u32, bto[k])) {
                            to.push(bto[k]);
                            attr.push(battr[k]);
                        }
                    }
                }
                for &(t, a, _) in edges.get(NodeId(i as u32)) {
                    to.push(t);
                    attr.push(EdgeAttr(a));
                }
                off.push(to.len() as u32);
            }
            Adjacency::from_parts(off, to, attr)
        });
        Patched {
            base,
            edges,
            probes: PerNode::build(n, patch.probes),
            inputs: PerNode::build(n, patch.inputs),
            memberships: PerNode::build(
                n,
                patch
                    .memberships
                    .into_iter()
                    .map(|(node, d, p)| (node, (d, p)))
                    .collect(),
            ),
            probe_ix,
            probe_sorted: std::sync::OnceLock::new(),
            input_ix,
            member_ix,
            tag_ix,
            strings: patch.strings,
            string_ix: patch.string_ix,
            dropped: patch.dropped,
            added: patch.added,
            resolved: patch.resolved,
            resolved_entries: patch.resolved_entries,
            touched,
            fwd,
            cut,
            cut_from,
        }
    }

    /// The base graph.
    pub fn base(&self) -> &'g G {
        self.base
    }

    /// The extra edges of `n`: `(target, label)`.
    pub fn extra_edges(&self, n: NodeId) -> impl Iterator<Item = (NodeId, &str)> + '_ {
        self.edges
            .get(n)
            .iter()
            .map(|&(t, _, l)| (NodeId(t), self.strings[l as usize].as_str()))
    }
}

impl<G: GraphView> GraphView for Patched<'_, G> {
    fn node_count(&self) -> u32 {
        self.base.node_count()
    }

    fn path(&self, n: NodeId) -> &str {
        self.base.path(n)
    }

    fn package(&self, n: NodeId) -> PackageId {
        self.base.package(n)
    }

    fn lang(&self, n: NodeId) -> Option<&str> {
        self.base.lang(n)
    }

    fn is_external(&self, n: NodeId) -> bool {
        self.base.is_external(n)
    }

    fn package_count(&self) -> u32 {
        self.base.package_count()
    }

    fn package_info(&self, p: PackageId) -> PackageInfo<'_> {
        self.base.package_info(p)
    }

    fn out_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_ {
        let cuts = self.cut_from.contains(n);
        self.base
            .out_edges(n)
            .filter(move |e| !cuts || e.to.is_none_or(|t| !self.cut.contains(&(n.0, t.0))))
            .chain(self.edges.get(n).iter().map(|&(t, a, l)| Edge {
                to: Some(NodeId(t)),
                attr: EdgeAttr(a),
                line: 0,
                span: None,
                specifier: Some(self.strings[l as usize].as_str()),
            }))
    }

    fn uncertainties(&self, n: NodeId) -> impl Iterator<Item = Uncertainty<'_>> + '_ {
        let touched = self.touched.contains(n);
        let added: &[Added] = if touched {
            self.added.get(&n.0).map_or(&[], Vec::as_slice)
        } else {
            &[]
        };
        self.base
            .uncertainties(n)
            .filter_map(move |u| {
                if !touched {
                    return Some(u);
                }
                let k = key_of(n, &u);
                if self.dropped.contains(&k) {
                    return None;
                }
                Some(match self.resolved.get(&k) {
                    Some(d) => Uncertainty {
                        detail: d.as_str(),
                        ..u
                    },
                    None => u,
                })
            })
            .chain(added.iter().map(|a| Uncertainty {
                kind: a.kind,
                line: a.line,
                span: None,
                detail: a.detail.as_str(),
                scope: None,
                effect: a.effect,
                origin: a.origin,
            }))
    }

    fn probes(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        self.base.probes(n).chain(
            self.probes
                .get(n)
                .iter()
                .map(|&p| self.strings[p as usize].as_str()),
        )
    }

    fn memberships(&self, n: NodeId) -> impl Iterator<Item = Membership<'_>> + '_ {
        self.base
            .memberships(n)
            .chain(self.memberships.get(n).iter().map(|&(d, p)| Membership {
                dir: self.strings[d as usize].as_str(),
                pattern: self.strings[p as usize].as_str(),
            }))
    }

    fn inputs(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        self.base.inputs(n).chain(
            self.inputs
                .get(n)
                .iter()
                .map(|&p| self.strings[p as usize].as_str()),
        )
    }

    fn node_by_path(&self, path: &str) -> Option<NodeId> {
        self.base.node_by_path(path)
    }

    fn glob(&self, glob: &crate::glob::Glob, out: &mut NodeSet) {
        self.base.glob(glob, out);
    }

    fn has_lookups(&self) -> bool {
        self.base.has_lookups()
    }

    fn probe_lookup(&self, path: &str, f: &mut dyn FnMut(NodeId, &str)) {
        self.base.probe_lookup(path, f);
        if let Some(&id) = self.string_ix.get(path) {
            let p = self.strings[id as usize].as_str();
            for &n in self.probe_ix.get(id) {
                f(NodeId(n), p);
            }
        }
    }

    fn probe_lookup_below(&self, path: &str, f: &mut dyn FnMut(NodeId, &str)) {
        self.base.probe_lookup_below(path, f);
        if self.probe_ix.items.is_empty() {
            return;
        }
        // The patch's probed paths in order, made on the first such lookup.
        let sorted = self.probe_sorted.get_or_init(|| {
            let mut ids: Vec<u32> = (0..self.strings.len() as u32)
                .filter(|&i| !self.probe_ix.get(i).is_empty())
                .collect();
            ids.sort_unstable_by(|a, b| self.strings[*a as usize].cmp(&self.strings[*b as usize]));
            ids
        });
        let prefix = format!("{path}/");
        let lo = sorted.partition_point(|&i| self.strings[i as usize].as_str() < prefix.as_str());
        for &i in &sorted[lo..] {
            let p = self.strings[i as usize].as_str();
            if !p.starts_with(&prefix) {
                break;
            }
            for &n in self.probe_ix.get(i) {
                f(NodeId(n), p);
            }
        }
    }

    fn membership_lookup(&self, dir: &str, f: &mut dyn FnMut(NodeId, &str, &str)) {
        self.base.membership_lookup(dir, f);
        if let Some(&id) = self.string_ix.get(dir) {
            let d = self.strings[id as usize].as_str();
            for &(pat, n) in self.member_ix.get(id) {
                f(NodeId(n), d, self.strings[pat as usize].as_str());
            }
        }
    }

    fn tagged(&self, tag: &str) -> Option<&[u32]> {
        if self.tag_ix.items.is_empty() {
            return self.base.tagged(tag);
        }
        Some(match self.string_ix.get(tag) {
            Some(&id) => self.tag_ix.get(id),
            None => &[],
        })
    }

    fn input_lookup(&self, path: &str, f: &mut dyn FnMut(NodeId)) {
        self.base.input_lookup(path, f);
        if let Some(&id) = self.string_ix.get(path) {
            for &n in self.input_ix.get(id) {
                f(NodeId(n));
            }
        }
    }

    fn has_uncertainty(&self, n: NodeId) -> bool {
        if self.touched.contains(n) {
            self.uncertainties(n).next().is_some()
        } else {
            self.base.has_uncertainty(n)
        }
    }

    fn forward_adjacency(&self) -> Option<&Adjacency> {
        match &self.fwd {
            Some(a) => Some(a),
            None => self.base.forward_adjacency(),
        }
    }

    fn reverse_adjacency(&self) -> Option<&Adjacency> {
        match &self.fwd {
            Some(_) => None,
            None => self.base.reverse_adjacency(),
        }
    }

    fn resolved_entries(&self) -> Option<&NodeSet> {
        match (&self.resolved_entries, self.base.resolved_entries()) {
            // A base that is itself patched keeps its own records; this layer's set applies
            // to this layer's records (`is_entry_resolved`).
            (Some(s), _) => Some(s),
            (None, b) => b,
        }
    }

    fn is_entry_resolved(&self, n: NodeId, u: &Uncertainty<'_>) -> bool {
        if self.resolved_entries.is_none() {
            return self.base.is_entry_resolved(n, u);
        }
        self.touched.contains(n) && self.resolved.contains_key(&key_of(n, u))
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;

    #[test]
    fn bytes_round_trip() {
        let mut p = Patch::new();
        p.edge(NodeId(1), NodeId(2), EdgeAttr(7), "env X: a.ts:3");
        p.probe(NodeId(1), "fixtures/a.json");
        p.input(NodeId(1), "test/setup.ts");
        p.membership(NodeId(3), "", "**/seed/a.json");
        p.tag(NodeId(2), "mock:marked");
        p.tag(NodeId(1), "mock:marked");
        let span = Some(Span { start: 4, end: 9 });
        p.drop_uncertainty(NodeId(1), UncertaintyKind::EnvDependent, 3, span);
        p.drop_uncertainty(NodeId(5), UncertaintyKind::ComputedSpecifier, 8, None);
        p.add_uncertainty_from(
            NodeId(5),
            UncertaintyKind::EnvDependent,
            8,
            "x; X is assigned at b.ts:1",
            Effect::Read,
            Origin::Env,
        );
        p.resolve_for_entries(
            NodeId(6),
            UncertaintyKind::CwdDependent,
            2,
            span,
            "per test",
        );
        let mut set = NodeSet::new(10);
        set.insert(NodeId(4));
        p.set_resolved_entries(set);
        let bytes = p.to_bytes();
        let q = Patch::from_bytes(&bytes).expect("decodes");
        assert_eq!(q.to_bytes(), bytes, "deterministic and complete");
        assert_eq!(q.edge_count(), 1);
        assert_eq!(q.dropped_count(), 2);
        assert_eq!(q.resolved_count(), 1);
        assert!(q.resolved_entries().is_some_and(|s| s.contains(NodeId(4))));
        // Anything else is rejected.
        assert!(Patch::from_bytes(&bytes[..bytes.len() - 1]).is_none());
        assert!(Patch::from_bytes(b"DTPATCH0").is_none());
        assert!(Patch::from_bytes(&Patch::new().to_bytes()).is_some_and(|p| p.is_empty()));
    }

    #[test]
    fn cut_edges_round_trip_as_a_trailing_section() {
        let mut p = Patch::new();
        p.edge(NodeId(1), NodeId(2), EdgeAttr(7), "bound: a.ts:3");
        let plain = p.to_bytes();
        p.cut(NodeId(4), NodeId(2));
        p.cut(NodeId(3), NodeId(2));
        p.cut(NodeId(4), NodeId(2));
        let bytes = p.to_bytes();
        // A patch without cuts keeps the bytes it had before cuts existed.
        assert!(bytes.starts_with(&plain) && bytes.len() > plain.len());
        let q = Patch::from_bytes(&bytes).expect("decodes");
        assert_eq!(q.to_bytes(), bytes, "deterministic and complete");
        assert_eq!(q.cut_count(), 2);
        assert!(!q.is_empty());
        assert!(Patch::from_bytes(&bytes[..bytes.len() - 4]).is_none());
        assert_eq!(Patch::from_bytes(&plain).expect("decodes").cut_count(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AffectedOptions, ChangeKind, ChangeSet, Engine, MemGraph, Phase, RefFlags, RefKind, Verdict,
    };

    const ATTR: EdgeAttr = EdgeAttr::new(RefKind::Require, Phase::Runtime, RefFlags::NONE);

    /// ```text
    /// svc/a/x.test.ts ─┐                     svc/a/src/routes.ts
    /// svc/b/y.test.ts ─┼→ helper.ts (cwd_dependent@3: process.cwd() + '/src/routes')
    /// tools/run.ts ────┘                     svc/b/src/routes.ts
    /// ```
    fn graph() -> MemGraph {
        let mut b = MemGraph::builder();
        let ta = b.file("svc/a/x.test.ts");
        let tb = b.file("svc/b/y.test.ts");
        let run = b.file("tools/run.ts");
        let helper = b.file("helper.ts");
        b.file("svc/a/src/routes.ts");
        b.file("svc/b/src/routes.ts");
        b.file("unrelated.ts");
        for t in [ta, tb, run] {
            b.edge(t, helper, RefKind::Import);
        }
        b.uncertainty(
            helper,
            UncertaintyKind::CwdDependent,
            3,
            "process.cwd() + '/src/routes'",
        );
        b.build().unwrap()
    }

    fn patch(g: &MemGraph) -> Patch {
        let id = |p: &str| g.node_by_path(p).unwrap();
        let mut p = Patch::new();
        p.edge(
            id("svc/a/x.test.ts"),
            id("svc/a/src/routes.ts"),
            ATTR,
            "cwd svc/a: ./src/routes",
        );
        p.edge(
            id("svc/b/y.test.ts"),
            id("svc/b/src/routes.ts"),
            ATTR,
            "cwd svc/b: ./src/routes",
        );
        p.probe(id("svc/a/x.test.ts"), "svc/a/src/routes.js");
        p.input(id("svc/b/y.test.ts"), "svc/b/data.json");
        p.membership(id("svc/b/y.test.ts"), "svc/b/fixtures", "*.json");
        p.resolve_for_entries(
            id("helper.ts"),
            UncertaintyKind::CwdDependent,
            3,
            None,
            "process.cwd() + '/src/routes'; resolved per test",
        );
        let mut tests = NodeSet::new(g.node_count());
        tests.insert(id("svc/a/x.test.ts"));
        tests.insert(id("svc/b/y.test.ts"));
        p.set_resolved_entries(tests);
        p
    }

    fn run(
        g: &Patched<'_, MemGraph>,
        kind: ChangeKind,
        path: &str,
        entries: &[&str],
    ) -> Vec<(String, Verdict, bool)> {
        let e = Engine::new(g);
        let mut cs = ChangeSet::new();
        cs.push(kind, path);
        let mut set = NodeSet::new(g.node_count());
        for p in entries {
            set.insert(g.node_by_path(p).unwrap());
        }
        e.affected(&cs, &set, &AffectedOptions::default())
            .entries
            .iter()
            .map(|r| (g.path(r.entry).to_owned(), r.verdict, r.uncertain.is_some()))
            .collect()
    }

    const ALL: &[&str] = &["svc/a/x.test.ts", "svc/b/y.test.ts", "tools/run.ts"];

    #[test]
    fn entry_resolved_records_only_count_for_other_entries() {
        let g = graph();
        // Without the patch every entry that reaches the helper is uncertain.
        let plain = Patched::new(&g, Patch::new());
        let r = run(&plain, ChangeKind::Modified, "unrelated.ts", ALL);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|x| x.2));

        let pg = Patched::new(&g, patch(&g));
        // An unrelated change: only the entry outside the resolved set stays uncertain.
        let r = run(&pg, ChangeKind::Modified, "unrelated.ts", ALL);
        assert_eq!(r, [("tools/run.ts".to_owned(), Verdict::Affected, true)]);
        // A service's routes select that service's test through the patch's edge.
        let r = run(&pg, ChangeKind::Modified, "svc/a/src/routes.ts", ALL);
        assert_eq!(
            r,
            [
                ("svc/a/x.test.ts".to_owned(), Verdict::Affected, false),
                ("tools/run.ts".to_owned(), Verdict::Affected, true),
            ]
        );
    }

    #[test]
    fn probes_inputs_and_memberships_seed_the_entry() {
        let g = graph();
        let pg = Patched::new(&g, patch(&g));
        let tests = &ALL[..2];
        let r = run(&pg, ChangeKind::Added, "svc/a/src/routes.js", tests);
        assert_eq!(
            r,
            [("svc/a/x.test.ts".to_owned(), Verdict::Affected, false)]
        );
        let r = run(&pg, ChangeKind::Modified, "svc/b/data.json", tests);
        assert_eq!(
            r,
            [("svc/b/y.test.ts".to_owned(), Verdict::Affected, false)]
        );
        let r = run(&pg, ChangeKind::Added, "svc/b/fixtures/n.json", tests);
        assert_eq!(
            r,
            [("svc/b/y.test.ts".to_owned(), Verdict::Affected, false)]
        );
        assert!(run(&pg, ChangeKind::Added, "svc/b/fixtures/n.txt", tests).is_empty());
    }

    #[test]
    fn dropped_and_added_records() {
        let g = graph();
        let id = |p: &str| g.node_by_path(p).unwrap();
        let mut p = Patch::new();
        p.drop_uncertainty(id("helper.ts"), UncertaintyKind::CwdDependent, 3, None);
        p.add_uncertainty(
            id("tools/run.ts"),
            UncertaintyKind::ComputedSpecifier,
            7,
            "argument of loader wrapper `load`",
            Effect::Load,
        );
        let pg = Patched::new(&g, p);
        assert!(!pg.has_uncertainty(id("helper.ts")));
        assert!(pg.has_uncertainty(id("tools/run.ts")));
        let r = run(&pg, ChangeKind::Modified, "unrelated.ts", ALL);
        assert_eq!(r, [("tools/run.ts".to_owned(), Verdict::Affected, true)]);
        // The view's edges and adjacency agree.
        let pg = Patched::new(&g, patch(&g));
        let n = id("svc/a/x.test.ts");
        let outs: Vec<_> = pg.out_edges(n).filter_map(|e| e.to).collect();
        let (adj, _) = pg.forward_adjacency().unwrap().neighbors(n);
        assert_eq!(outs.iter().map(|x| x.0).collect::<Vec<_>>(), adj);
        assert_eq!(
            pg.extra_edges(n).map(|(_, l)| l).collect::<Vec<_>>(),
            ["cwd svc/a: ./src/routes"]
        );
        let u: Vec<_> = pg.uncertainties(id("helper.ts")).collect();
        assert_eq!(u.len(), 1);
        assert!(u[0].detail.ends_with("resolved per test"));
    }

    #[test]
    fn merge_intersects_resolved_entries() {
        let g = graph();
        let mut a = patch(&g);
        let mut b = Patch::new();
        let mut only_a = NodeSet::new(g.node_count());
        only_a.insert(g.node_by_path("svc/a/x.test.ts").unwrap());
        b.set_resolved_entries(only_a);
        a.merge(b);
        assert_eq!(a.resolved_entries().unwrap().len(), 1);
        assert!(!a.is_empty());
        assert!(Patch::new().is_empty());
    }
}
