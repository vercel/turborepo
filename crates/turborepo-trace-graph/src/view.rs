//! The read-only graph interface the query engine runs on.

use crate::{
    glob::Glob,
    set::NodeSet,
    vocab::{EdgeAttr, Effect, Origin, Phase, RefFlags, RefKind, UncertaintyKind},
};

/// A dense file-node id, `0..node_count`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The id as an index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A dense package id, `0..package_count`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageId(pub u32);

/// A package's version: a workspace (first-party) package, or a resolved
/// third-party version.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PackageVersion<'a> {
    /// A workspace member (or the repo root). Displayed and serialized as
    /// `workspace`.
    Workspace,
    /// A third-party version from the lockfile or installed layout.
    Version(&'a str),
}

impl<'a> PackageVersion<'a> {
    /// `"workspace"` or the version string.
    pub fn as_str(&self) -> &'a str {
        match self {
            PackageVersion::Workspace => "workspace",
            PackageVersion::Version(v) => v,
        }
    }
}

/// One package.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PackageInfo<'a> {
    /// Package name (`@scope/name`, Go module path, crate name, ...).
    pub name: &'a str,
    /// Version, or workspace.
    pub version: PackageVersion<'a>,
    /// Package root, relative to the graph root (`""` for the repo root).
    pub root: &'a str,
}

/// A byte range in the referencing file.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// Start offset.
    pub start: u32,
    /// End offset (exclusive).
    pub end: u32,
}

/// One outgoing edge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Edge<'a> {
    /// Target node, or `None` if unresolved (or resolved to something that is
    /// not a node, such as a builtin).
    pub to: Option<NodeId>,
    /// Kind, phase and flags.
    pub attr: EdgeAttr,
    /// 1-based line of the reference; 0 if unknown.
    pub line: u32,
    /// Byte span of the reference, if known.
    pub span: Option<Span>,
    /// The specifier as written, if recorded.
    pub specifier: Option<&'a str>,
}

impl Edge<'_> {
    /// Reference kind.
    pub fn kind(&self) -> RefKind {
        self.attr.kind()
    }
    /// Phase.
    pub fn phase(&self) -> Phase {
        self.attr.phase()
    }
    /// Flags.
    pub fn flags(&self) -> RefFlags {
        self.attr.flags()
    }
}

/// One uncertainty recorded on a node.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Uncertainty<'a> {
    /// Category.
    pub kind: UncertaintyKind,
    /// 1-based line; 0 if unknown.
    pub line: u32,
    /// Byte span, if known.
    pub span: Option<Span>,
    /// Human-readable detail.
    pub detail: &'a str,
    /// The scope of a *covered* uncertainty: every file the reference can read
    /// lies below `scope.dir` and matches `scope.pattern`, and it reads
    /// only their contents. `None` is an *open* uncertainty. A covered
    /// uncertainty doesn't trigger the `--on-uncertain` policy; instead
    /// `affected` seeds its node on any added, modified or deleted path
    /// inside the scope.
    pub scope: Option<Membership<'a>>,
    /// What the reference that raised it does with its target ([`Effect::None`]
    /// for records not raised by a reference).
    pub effect: Effect,
    /// Where the unknown part of the value comes from.
    pub origin: Origin,
}

impl Uncertainty<'_> {
    /// Whether the uncertainty is covered by a scope.
    pub fn is_covered(&self) -> bool {
        self.scope.is_some()
    }
}

/// A directory membership: the node's resolution depends on the listing of
/// `dir` filtered by `pattern` (wildcard requires, globs, `readdir`,
/// `go:embed`, Go package dirs).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Membership<'a> {
    /// Directory relative to the graph root, without trailing slash (`""` =
    /// root).
    pub dir: &'a str,
    /// Glob relative to `dir` (see [`crate::glob`]); `**` patterns cover
    /// subdirectories.
    pub pattern: &'a str,
}

/// Read-only access to a composed whole-repo graph.
///
/// Paths are relative to the graph root, `/`-separated, without a leading `./`.
/// Implementations: [`crate::MemGraph`] (tests, JSON import) and, later, the
/// composed shard store. Methods returning iterators use return-position `impl
/// Trait`, so the engine is generic over the view (`Engine<G: GraphView>`) and
/// every call is monomorphized; the trait is not object-safe on purpose.
///
/// Required methods are cheap per-node accessors. The `*_adjacency` and
/// [`glob`] methods have defaults and exist so that a store that already holds
/// CSR arrays or a sorted path index can hand them over instead of letting the
/// engine rebuild them.
///
/// [`glob`]: GraphView::glob
pub trait GraphView {
    /// Number of file nodes; ids are `0..node_count`.
    fn node_count(&self) -> u32;
    /// Path of `n`, relative to the root.
    fn path(&self, n: NodeId) -> &str;
    /// Package that owns `n`.
    fn package(&self, n: NodeId) -> PackageId;
    /// Language id of `n` (`"js"`, `"py"`, ...), if the file is source code.
    fn lang(&self, n: NodeId) -> Option<&str>;
    /// Whether `n` belongs to a third-party package (an opaque package node or
    /// a followed external file).
    fn is_external(&self, n: NodeId) -> bool;

    /// Number of packages; ids are `0..package_count`.
    fn package_count(&self) -> u32;
    /// Package metadata.
    fn package_info(&self, p: PackageId) -> PackageInfo<'_>;

    /// Outgoing edges of `n`, resolved and unresolved.
    fn out_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_;
    /// Uncertainties recorded on `n`.
    fn uncertainties(&self, n: NodeId) -> impl Iterator<Item = Uncertainty<'_>> + '_;
    /// Missing paths `n`'s resolution probed (root-relative). Adding a file at
    /// a probed path, or below a probed directory, can change how `n`
    /// resolves.
    fn probes(&self, n: NodeId) -> impl Iterator<Item = &str> + '_;
    /// Directory memberships recorded on `n`.
    fn memberships(&self, n: NodeId) -> impl Iterator<Item = Membership<'_>> + '_;

    /// Files whose contents `n`'s resolution read (root-relative): manifests
    /// and configs such as `package.json` or the tsconfig `extends` chain,
    /// and the framework configs a test's inputs were derived from.
    /// Changing, adding or deleting one can change how `n`
    /// resolves. Default: none.
    fn inputs(&self, _n: NodeId) -> impl Iterator<Item = &str> + '_ {
        std::iter::empty()
    }

    /// The node with exactly this path.
    fn node_by_path(&self, path: &str) -> Option<NodeId>;

    /// Adds every node whose path matches `glob` to `out`. The default scans
    /// all nodes.
    fn glob(&self, glob: &Glob, out: &mut NodeSet) {
        for i in 0..self.node_count() {
            let n = NodeId(i);
            if glob.matches(self.path(n)) {
                out.insert(n);
            }
        }
    }

    /// Whether the view answers [`probe_lookup`](GraphView::probe_lookup),
    /// [`membership_lookup`](GraphView::membership_lookup) and
    /// [`input_lookup`](GraphView::input_lookup) itself (for example from
    /// inverted indexes it already holds). Otherwise the engine builds
    /// sorted indexes from [`probes`](GraphView::probes),
    /// [`memberships`](GraphView::memberships) and
    /// [`inputs`](GraphView::inputs) on first use.
    fn has_lookups(&self) -> bool {
        false
    }

    /// Calls `f(node, probe)` for every node with exactly `path` as a probe
    /// (only when [`has_lookups`](GraphView::has_lookups)).
    fn probe_lookup(&self, _path: &str, _f: &mut dyn FnMut(NodeId, &str)) {}

    /// Calls `f(node, probe)` for every probe of a node that lies below `path`
    /// (it starts with `path` and a `/`), only when
    /// [`has_lookups`](GraphView::has_lookups).
    fn probe_lookup_below(&self, _path: &str, _f: &mut dyn FnMut(NodeId, &str)) {}

    /// Calls `f(node, dir, pattern)` for every membership of a node on
    /// directory `dir` (only when [`has_lookups`](GraphView::has_lookups)).
    fn membership_lookup(&self, _dir: &str, _f: &mut dyn FnMut(NodeId, &str, &str)) {}

    /// Calls `f(node)` for every node with `path` as an input (only when
    /// [`has_lookups`](GraphView::has_lookups)).
    fn input_lookup(&self, _path: &str, _f: &mut dyn FnMut(NodeId)) {}

    /// The nodes that carry `tag`, when the view has an index of tags: a
    /// derived layer that looked at every node for some fact (marker edges,
    /// for example) leaves the nodes it found there
    /// ([`crate::Patch::tag`]), so a query asks instead of looking at every
    /// node again. `None`: the view has no such index (the caller looks
    /// itself); an empty list: no node carries the tag.
    fn tagged(&self, _tag: &str) -> Option<&[u32]> {
        None
    }

    /// Whether `n` has any uncertainty. Override if cheaper than
    /// `uncertainties(n)`.
    fn has_uncertainty(&self, n: NodeId) -> bool {
        self.uncertainties(n).next().is_some()
    }

    /// The entries for which *entry-resolved* uncertainties
    /// ([`is_entry_resolved`](GraphView::is_entry_resolved)) don't count.
    /// `None`: the view has no entry-resolved uncertainties. See
    /// [`crate::patch`].
    fn resolved_entries(&self) -> Option<&NodeSet> {
        None
    }

    /// Whether uncertainty `u` of `n` is resolved per entry: on each entry of
    /// [`resolved_entries`](GraphView::resolved_entries) the view holds the
    /// edges, probes and inputs that represent every target the reference
    /// can have when that entry runs it (a path relative to the test
    /// runner's working directory). `affected` then applies the
    /// `--on-uncertain` policy for `u` only to the other entries that reach
    /// `n`.
    fn is_entry_resolved(&self, _n: NodeId, _u: &Uncertainty<'_>) -> bool {
        false
    }

    /// Resolved outgoing edges in CSR form, if the view already holds them.
    /// Otherwise the engine builds them once from
    /// [`out_edges`](GraphView::out_edges).
    fn forward_adjacency(&self) -> Option<&Adjacency> {
        None
    }

    /// Resolved incoming edges in CSR form, if the view already holds them.
    /// Otherwise the engine builds them lazily by transposing the forward
    /// adjacency.
    fn reverse_adjacency(&self) -> Option<&Adjacency> {
        None
    }
}

/// Resolved edges in compressed sparse row form: the neighbors of node `i` are
/// `targets[offsets[i]..offsets[i + 1]]`, with packed attributes in the
/// parallel `attrs`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adjacency {
    offsets: Vec<u32>,
    targets: Vec<u32>,
    attrs: Vec<EdgeAttr>,
}

impl Adjacency {
    /// From raw CSR arrays. Panics if they are inconsistent.
    pub fn from_parts(offsets: Vec<u32>, targets: Vec<u32>, attrs: Vec<EdgeAttr>) -> Self {
        assert!(!offsets.is_empty(), "offsets needs node_count + 1 entries");
        assert_eq!(offsets[offsets.len() - 1] as usize, targets.len());
        assert_eq!(targets.len(), attrs.len());
        debug_assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
        let n = offsets.len() as u32 - 1;
        assert!(targets.iter().all(|&t| t < n), "edge target out of range");
        Adjacency {
            offsets,
            targets,
            attrs,
        }
    }

    /// From `(from, to, attr)` triples in any order (stable counting sort by
    /// `from`).
    pub fn from_triples(node_count: u32, triples: &[(u32, u32, EdgeAttr)]) -> Self {
        let mut offsets = vec![0u32; node_count as usize + 1];
        for &(f, _, _) in triples {
            offsets[f as usize + 1] += 1;
        }
        for i in 0..node_count as usize {
            offsets[i + 1] += offsets[i];
        }
        let mut cursor = offsets.clone();
        let mut targets = vec![0u32; triples.len()];
        let mut attrs = vec![EdgeAttr(0); triples.len()];
        for &(f, t, a) in triples {
            let c = &mut cursor[f as usize];
            targets[*c as usize] = t;
            attrs[*c as usize] = a;
            *c += 1;
        }
        Adjacency::from_parts(offsets, targets, attrs)
    }

    /// The resolved edges of a view, in `out_edges` order.
    pub fn from_view<G: GraphView + ?Sized>(g: &G) -> Self {
        let n = g.node_count();
        let mut offsets = Vec::with_capacity(n as usize + 1);
        let mut targets = Vec::new();
        let mut attrs = Vec::new();
        offsets.push(0);
        for i in 0..n {
            for e in g.out_edges(NodeId(i)) {
                if let Some(t) = e.to {
                    targets.push(t.0);
                    attrs.push(e.attr);
                }
            }
            offsets.push(targets.len() as u32);
        }
        Adjacency::from_parts(offsets, targets, attrs)
    }

    /// The transpose: for each node, its predecessors (in increasing source
    /// order).
    pub fn transpose(&self) -> Adjacency {
        let n = self.node_count() as usize;
        let mut offsets = vec![0u32; n + 1];
        for &t in &self.targets {
            offsets[t as usize + 1] += 1;
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        let mut cursor = offsets[..n].to_vec();
        let mut targets = vec![0u32; self.targets.len()];
        let mut attrs = vec![EdgeAttr(0); self.targets.len()];
        for from in 0..n {
            let (lo, hi) = (self.offsets[from] as usize, self.offsets[from + 1] as usize);
            for e in lo..hi {
                let t = self.targets[e] as usize;
                let c = cursor[t] as usize;
                targets[c] = from as u32;
                attrs[c] = self.attrs[e];
                cursor[t] += 1;
            }
        }
        Adjacency {
            offsets,
            targets,
            attrs,
        }
    }

    /// Number of nodes.
    pub fn node_count(&self) -> u32 {
        self.offsets.len() as u32 - 1
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.targets.len()
    }

    /// Neighbor ids and attributes of `n`.
    #[inline]
    pub fn neighbors(&self, n: NodeId) -> (&[u32], &[EdgeAttr]) {
        let (lo, hi) = (
            self.offsets[n.index()] as usize,
            self.offsets[n.index() + 1] as usize,
        );
        (&self.targets[lo..hi], &self.attrs[lo..hi])
    }

    /// Index range of `n`'s edges in the flat arrays.
    #[inline]
    pub fn range(&self, n: NodeId) -> std::ops::Range<usize> {
        self.offsets[n.index()] as usize..self.offsets[n.index() + 1] as usize
    }

    /// The raw CSR arrays (offsets, targets, attributes), as
    /// [`Adjacency::from_parts`] takes them.
    pub fn parts(&self) -> (&[u32], &[u32], &[EdgeAttr]) {
        (&self.offsets, &self.targets, &self.attrs)
    }
}
