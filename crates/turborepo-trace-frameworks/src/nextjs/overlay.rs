//! A [`GraphView`] that adds virtual nodes and edges on top of a base graph (a
//! [`turborepo_trace_graph::MemGraph`] or the store's composed graph), so
//! `affected` runs unchanged over route nodes.
//!
//! Virtual nodes are either pseudo nodes (`next:route:/x`, `next:global`) or
//! file paths missing from the base graph (CSS, JSON, a route file added by the
//! change set). A path node matches changes like any file node.

use std::collections::HashMap;

use turborepo_trace_graph::{
    Edge, EdgeAttr, GraphView, Membership, NodeId, PackageId, PackageInfo, Phase, RefFlags,
    RefKind, Uncertainty, UncertaintyKind,
};

const ATTR: EdgeAttr = EdgeAttr::new(RefKind::Import, Phase::Runtime, RefFlags::NONE);

#[derive(Default)]
struct Extra {
    path: String,
    memberships: Vec<(String, String)>,
    probes: Vec<String>,
    uncertainties: Vec<(UncertaintyKind, String)>,
}

/// The overlay.
pub struct Overlay<'g, G: GraphView> {
    base: &'g G,
    extra: Vec<Extra>,
    out: HashMap<u32, Vec<u32>>,
    by_path: HashMap<String, u32>,
}

enum Either<A, B> {
    A(A),
    B(B),
}

impl<T, A: Iterator<Item = T>, B: Iterator<Item = T>> Iterator for Either<A, B> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        match self {
            Either::A(a) => a.next(),
            Either::B(b) => b.next(),
        }
    }
}

impl<'g, G: GraphView> Overlay<'g, G> {
    /// An empty overlay over `base`.
    pub fn new(base: &'g G) -> Self {
        Overlay {
            base,
            extra: Vec::new(),
            out: HashMap::new(),
            by_path: HashMap::new(),
        }
    }

    /// The base graph.
    pub fn base(&self) -> &'g G {
        self.base
    }

    /// The node for `path`: the base node, an existing virtual node, or a new
    /// virtual node.
    pub fn node(&mut self, path: &str) -> NodeId {
        if let Some(n) = self.base.node_by_path(path) {
            return n;
        }
        if let Some(&n) = self.by_path.get(path) {
            return NodeId(n);
        }
        let id = self.base.node_count() + self.extra.len() as u32;
        self.extra.push(Extra {
            path: path.to_string(),
            ..Extra::default()
        });
        self.by_path.insert(path.to_string(), id);
        NodeId(id)
    }

    /// Adds an edge `from → to` (`from` depends on `to`).
    pub fn edge(&mut self, from: NodeId, to: NodeId) {
        if from == to {
            return;
        }
        let v = self.out.entry(from.0).or_default();
        if !v.contains(&to.0) {
            v.push(to.0);
        }
    }

    fn extra_mut(&mut self, n: NodeId) -> Option<&mut Extra> {
        let i = n.0.checked_sub(self.base.node_count())?;
        self.extra.get_mut(i as usize)
    }

    /// Adds a membership to a virtual node.
    pub fn membership(&mut self, n: NodeId, dir: &str, pattern: &str) {
        if let Some(e) = self.extra_mut(n) {
            e.memberships.push((dir.to_string(), pattern.to_string()));
        }
    }

    /// Adds a probe to a virtual node.
    pub fn probe(&mut self, n: NodeId, path: &str) {
        if let Some(e) = self.extra_mut(n) {
            e.probes.push(path.to_string());
        }
    }

    /// Adds an uncertainty to a virtual node.
    pub fn uncertainty(&mut self, n: NodeId, kind: UncertaintyKind, detail: &str) {
        if let Some(e) = self.extra_mut(n) {
            e.uncertainties.push((kind, detail.to_string()));
        }
    }

    /// Whether `n` is virtual.
    pub fn is_virtual(&self, n: NodeId) -> bool {
        n.0 >= self.base.node_count()
    }

    fn extra(&self, n: NodeId) -> Option<&Extra> {
        let i = n.0.checked_sub(self.base.node_count())?;
        self.extra.get(i as usize)
    }
}

impl<G: GraphView> GraphView for Overlay<'_, G> {
    fn node_count(&self) -> u32 {
        self.base.node_count() + self.extra.len() as u32
    }

    fn path(&self, n: NodeId) -> &str {
        match self.extra(n) {
            Some(e) => &e.path,
            None => self.base.path(n),
        }
    }

    fn package(&self, n: NodeId) -> PackageId {
        if self.is_virtual(n) {
            PackageId(0)
        } else {
            self.base.package(n)
        }
    }

    fn lang(&self, n: NodeId) -> Option<&str> {
        if self.is_virtual(n) {
            None
        } else {
            self.base.lang(n)
        }
    }

    fn is_external(&self, n: NodeId) -> bool {
        !self.is_virtual(n) && self.base.is_external(n)
    }

    fn package_count(&self) -> u32 {
        self.base.package_count().max(1)
    }

    fn package_info(&self, p: PackageId) -> PackageInfo<'_> {
        if self.base.package_count() == 0 {
            return PackageInfo {
                name: "",
                version: turborepo_trace_graph::PackageVersion::Workspace,
                root: "",
            };
        }
        self.base.package_info(p)
    }

    fn out_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_ {
        let extra = self.out.get(&n.0).map(Vec::as_slice).unwrap_or(&[]);
        let virt = extra.iter().map(|&t| Edge {
            to: Some(NodeId(t)),
            attr: ATTR,
            line: 0,
            span: None,
            specifier: None,
        });
        if self.is_virtual(n) {
            Either::A(virt)
        } else {
            Either::B(self.base.out_edges(n).chain(virt))
        }
    }

    fn uncertainties(&self, n: NodeId) -> impl Iterator<Item = Uncertainty<'_>> + '_ {
        match self.extra(n) {
            Some(e) => Either::A(e.uncertainties.iter().map(|(k, d)| Uncertainty {
                kind: *k,
                line: 0,
                span: None,
                detail: d.as_str(),
                scope: None,
                effect: turborepo_trace_graph::Effect::None,
                origin: turborepo_trace_graph::Origin::Unknown,
            })),
            None => Either::B(self.base.uncertainties(n)),
        }
    }

    fn probes(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        match self.extra(n) {
            Some(e) => Either::A(e.probes.iter().map(String::as_str)),
            None => Either::B(self.base.probes(n)),
        }
    }

    fn memberships(&self, n: NodeId) -> impl Iterator<Item = Membership<'_>> + '_ {
        match self.extra(n) {
            Some(e) => Either::A(e.memberships.iter().map(|(d, p)| Membership {
                dir: d.as_str(),
                pattern: p.as_str(),
            })),
            None => Either::B(self.base.memberships(n)),
        }
    }

    fn node_by_path(&self, path: &str) -> Option<NodeId> {
        self.base
            .node_by_path(path)
            .or_else(|| self.by_path.get(path).map(|&n| NodeId(n)))
    }

    fn has_uncertainty(&self, n: NodeId) -> bool {
        match self.extra(n) {
            Some(e) => !e.uncertainties.is_empty(),
            None => self.base.has_uncertainty(n),
        }
    }
}
