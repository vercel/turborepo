//! [`MemGraph`]: an in-memory [`GraphView`] with a builder, used by tests and JSON import.

use std::collections::HashMap;

use crate::{
    glob::Glob,
    set::NodeSet,
    view::{
        Adjacency, Edge, GraphView, Membership, NodeId, PackageId, PackageInfo, PackageVersion,
        Span, Uncertainty,
    },
    vocab::{EdgeAttr, Effect, Origin, Phase, RefFlags, RefKind, UncertaintyKind},
};

const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
struct Pkg {
    name: Box<str>,
    version: Option<Box<str>>,
    root: Box<str>,
}

/// Per-edge details kept out of the hot adjacency arrays.
#[derive(Copy, Clone, Debug)]
struct EdgeDetail {
    line: u32,
    span_start: u32,
    span_end: u32,
    spec: u32,
}

impl EdgeDetail {
    fn span(&self) -> Option<Span> {
        (self.span_start != NONE).then_some(Span {
            start: self.span_start,
            end: self.span_end,
        })
    }
}

#[derive(Clone, Debug)]
struct UncRec {
    kind: UncertaintyKind,
    line: u32,
    span: Option<Span>,
    detail: Box<str>,
    /// Covered: scope directory and pattern.
    scope: Option<(Box<str>, Box<str>)>,
    effect: Effect,
    origin: Origin,
}

/// A grouped list: items of node `i` are `items[off[i]..off[i + 1]]`.
#[derive(Clone, Debug, Default)]
struct Grouped<T> {
    off: Vec<u32>,
    items: Vec<T>,
}

impl<T> Grouped<T> {
    /// Groups `pairs` by key, keeping insertion order within a key. Pairs added
    /// in key order (the common case) are not moved.
    fn build(n: u32, mut pairs: Vec<(u32, T)>) -> Self {
        if !pairs.is_sorted_by_key(|p| p.0) {
            pairs.sort_by_key(|p| p.0);
        }
        let mut off = vec![0u32; n as usize + 1];
        for (k, _) in &pairs {
            off[*k as usize + 1] += 1;
        }
        for i in 0..n as usize {
            off[i + 1] += off[i];
        }
        Grouped {
            off,
            items: pairs.into_iter().map(|(_, v)| v).collect(),
        }
    }

    fn get(&self, n: NodeId) -> &[T] {
        &self.items[self.off[n.index()] as usize..self.off[n.index() + 1] as usize]
    }
}

/// An immutable in-memory graph. Build one with [`MemGraph::builder`] or load
/// it with [`MemGraph::from_json_slice`](crate::json).
#[derive(Clone, Debug)]
pub struct MemGraph {
    pub(crate) root: Option<String>,
    path_buf: String,
    path_off: Vec<u32>,
    node_pkg: Vec<u32>,
    node_lang: Vec<u16>,
    langs: Vec<Box<str>>,
    external: NodeSet,
    by_path: Vec<u32>,
    packages: Vec<Pkg>,
    fwd: Adjacency,
    fwd_detail: Vec<EdgeDetail>,
    unresolved: Grouped<(EdgeAttr, EdgeDetail)>,
    specs: Vec<Box<str>>,
    uncs: Grouped<UncRec>,
    strs: String,
    probes: Grouped<StrRef>,
    members: Grouped<(StrRef, StrRef)>,
}

/// A `start..end` range into a string arena.
type StrRef = (u32, u32);

fn arena_push(buf: &mut String, s: &str) -> StrRef {
    let start = buf.len();
    buf.push_str(s);
    assert!(buf.len() <= u32::MAX as usize, "string arena over 4 GiB");
    (start as u32, buf.len() as u32)
}

#[inline]
fn arena_get(buf: &str, r: StrRef) -> &str {
    &buf[r.0 as usize..r.1 as usize]
}

/// Errors from [`GraphBuilder::build`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// Two nodes share a path.
    #[error("duplicate file path `{0}`")]
    DuplicatePath(String),
    /// A reference to a node or package id that does not exist.
    #[error("{0}")]
    BadReference(String),
}

/// An edge to add with [`GraphBuilder::add_edge`].
#[derive(Clone, Debug)]
pub struct EdgeSpec {
    /// Target, or `None` for unresolved.
    pub to: Option<NodeId>,
    /// Kind.
    pub kind: RefKind,
    /// Phase.
    pub phase: Phase,
    /// Flags.
    pub flags: RefFlags,
    /// 1-based line, 0 if unknown.
    pub line: u32,
    /// Byte span.
    pub span: Option<Span>,
    /// Specifier as written.
    pub specifier: Option<String>,
}

impl EdgeSpec {
    /// An edge of `kind` to `to` with the kind's default phase and no flags.
    pub fn new(to: Option<NodeId>, kind: RefKind) -> Self {
        EdgeSpec {
            to,
            kind,
            phase: kind.default_phase(),
            flags: RefFlags::NONE,
            line: 0,
            span: None,
            specifier: None,
        }
    }

    /// Sets flags.
    pub fn flags(mut self, flags: RefFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Sets the phase.
    pub fn phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    /// Sets the line.
    pub fn line(mut self, line: u32) -> Self {
        self.line = line;
        self
    }

    /// Sets the specifier.
    pub fn specifier(mut self, s: impl Into<String>) -> Self {
        self.specifier = Some(s.into());
        self
    }
}

/// Builder for [`MemGraph`]. Storage is columnar so that building a 2M-edge
/// graph moves little memory.
#[derive(Debug)]
pub struct GraphBuilder {
    root: Option<String>,
    packages: Vec<Pkg>,
    default_pkg: Option<u32>,
    path_buf: String,
    path_off: Vec<u32>,
    node_pkg: Vec<u32>,
    node_lang: Vec<u16>,
    langs: Vec<Box<str>>,
    lang_ids: HashMap<Box<str>, u16>,
    external: Vec<u32>,
    e_from: Vec<u32>,
    e_to: Vec<u32>,
    e_attr: Vec<EdgeAttr>,
    e_detail: Vec<EdgeDetail>,
    specs: Vec<Box<str>>,
    spec_ids: HashMap<Box<str>, u32>,
    uncs: Vec<(u32, UncRec)>,
    strs: String,
    probes: Vec<(u32, StrRef)>,
    members: Vec<(u32, (StrRef, StrRef))>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        GraphBuilder {
            root: None,
            packages: Vec::new(),
            default_pkg: None,
            path_buf: String::new(),
            path_off: vec![0],
            node_pkg: Vec::new(),
            node_lang: Vec::new(),
            langs: Vec::new(),
            lang_ids: HashMap::new(),
            external: Vec::new(),
            e_from: Vec::new(),
            e_to: Vec::new(),
            e_attr: Vec::new(),
            e_detail: Vec::new(),
            specs: Vec::new(),
            spec_ids: HashMap::new(),
            uncs: Vec::new(),
            strs: String::new(),
            probes: Vec::new(),
            members: Vec::new(),
        }
    }
}

/// Strips a leading `./` and trailing `/`.
pub(crate) fn normalize(path: &str) -> &str {
    let p = path.strip_prefix("./").unwrap_or(path);
    let p = if p == "." { "" } else { p };
    p.strip_suffix('/').unwrap_or(p)
}

impl GraphBuilder {
    /// Records the absolute root the relative paths are based on
    /// (informational).
    pub fn root(&mut self, root: impl Into<String>) -> &mut Self {
        self.root = Some(root.into());
        self
    }

    /// Adds a package. `version: None` means a workspace package.
    pub fn package(&mut self, name: &str, version: Option<&str>, root: &str) -> PackageId {
        self.packages.push(Pkg {
            name: name.into(),
            version: version.map(Into::into),
            root: normalize(root).into(),
        });
        PackageId(self.packages.len() as u32 - 1)
    }

    /// Adds a file in the default root package (`.`, workspace), language
    /// guessed from the extension.
    pub fn file(&mut self, path: &str) -> NodeId {
        let pkg = match self.default_pkg {
            Some(p) => PackageId(p),
            None => {
                let p = self.package(".", None, "");
                self.default_pkg = Some(p.0);
                p
            }
        };
        let lang = guess_lang(path);
        self.add_file(path, pkg, lang, false)
    }

    /// Adds a file.
    pub fn add_file(
        &mut self,
        path: &str,
        package: PackageId,
        lang: Option<&str>,
        external: bool,
    ) -> NodeId {
        let id = self.node_pkg.len() as u32;
        self.path_buf.push_str(normalize(path));
        assert!(self.path_buf.len() <= u32::MAX as usize, "paths over 4 GiB");
        self.path_off.push(self.path_buf.len() as u32);
        self.node_pkg.push(package.0);
        let l = match lang {
            None => u16::MAX,
            Some(l) => match self.lang_ids.get(l) {
                Some(&i) => i,
                None => {
                    let i = self.langs.len() as u16;
                    self.langs.push(l.into());
                    self.lang_ids.insert(l.into(), i);
                    i
                }
            },
        };
        self.node_lang.push(l);
        if external {
            self.external.push(id);
        }
        NodeId(id)
    }

    /// Number of files added so far.
    pub fn node_count(&self) -> u32 {
        self.node_pkg.len() as u32
    }

    /// Reserves room for `nodes` more files and `edges` more edges.
    pub fn reserve(&mut self, nodes: usize, edges: usize) {
        self.node_pkg.reserve(nodes);
        self.node_lang.reserve(nodes);
        self.path_off.reserve(nodes);
        self.e_from.reserve(edges);
        self.e_to.reserve(edges);
        self.e_attr.reserve(edges);
        self.e_detail.reserve(edges);
    }

    /// Adds a resolved edge with the kind's default phase and no flags.
    pub fn edge(&mut self, from: NodeId, to: NodeId, kind: RefKind) -> &mut Self {
        self.add_edge(from, EdgeSpec::new(Some(to), kind))
    }

    /// Adds an edge.
    pub fn add_edge(&mut self, from: NodeId, edge: EdgeSpec) -> &mut Self {
        self.add_edge_raw(
            from,
            edge.to,
            EdgeAttr::new(edge.kind, edge.phase, edge.flags),
            edge.line,
            edge.span,
            edge.specifier.as_deref(),
        )
    }

    /// Adds an edge from packed attributes, without allocating unless the
    /// specifier is new.
    pub fn add_edge_raw(
        &mut self,
        from: NodeId,
        to: Option<NodeId>,
        attr: EdgeAttr,
        line: u32,
        span: Option<Span>,
        specifier: Option<&str>,
    ) -> &mut Self {
        let spec = match specifier {
            None => NONE,
            Some(s) => match self.spec_ids.get(s) {
                Some(&id) => id,
                None => {
                    let id = self.specs.len() as u32;
                    self.specs.push(s.into());
                    self.spec_ids.insert(s.into(), id);
                    id
                }
            },
        };
        self.e_from.push(from.0);
        self.e_to.push(to.map_or(NONE, |t| t.0));
        self.e_attr.push(attr);
        self.e_detail.push(EdgeDetail {
            line,
            span_start: span.map_or(NONE, |s| s.start),
            span_end: span.map_or(NONE, |s| s.end),
            spec,
        });
        self
    }

    /// Records an uncertainty on `n`.
    pub fn uncertainty(
        &mut self,
        n: NodeId,
        kind: UncertaintyKind,
        line: u32,
        detail: &str,
    ) -> &mut Self {
        self.uncs.push((
            n.0,
            UncRec {
                kind,
                line,
                span: None,
                detail: detail.into(),
                scope: None,
                effect: Effect::None,
                origin: Origin::Unknown,
            },
        ));
        self
    }

    /// Sets what the most recently recorded uncertainty's reference does with
    /// its target and where its unknown value comes from (see
    /// [`Uncertainty::effect`] and [`Uncertainty::origin`]).
    pub fn uncertainty_class(&mut self, effect: Effect, origin: Origin) -> &mut Self {
        if let Some((_, u)) = self.uncs.last_mut() {
            u.effect = effect;
            u.origin = origin;
        }
        self
    }

    /// Records a covered uncertainty on `n`: every file the reference can read
    /// lies below `dir` (root-relative) and matches `pattern` (see
    /// [`Uncertainty::scope`]).
    pub fn covered_uncertainty(
        &mut self,
        n: NodeId,
        kind: UncertaintyKind,
        line: u32,
        detail: &str,
        dir: &str,
        pattern: &str,
    ) -> &mut Self {
        self.uncs.push((
            n.0,
            UncRec {
                kind,
                line,
                span: None,
                detail: detail.into(),
                scope: Some((normalize(dir).into(), pattern.into())),
                effect: Effect::None,
                origin: Origin::Unknown,
            },
        ));
        self
    }

    /// Records a probed missing path on `n`.
    pub fn probe(&mut self, n: NodeId, path: &str) -> &mut Self {
        let r = arena_push(&mut self.strs, normalize(path));
        self.probes.push((n.0, r));
        self
    }

    /// Records a directory membership on `n`.
    pub fn membership(&mut self, n: NodeId, dir: &str, pattern: &str) -> &mut Self {
        let d = arena_push(&mut self.strs, normalize(dir));
        let p = arena_push(&mut self.strs, pattern);
        self.members.push((n.0, (d, p)));
        self
    }

    /// Freezes the graph.
    pub fn build(self) -> Result<MemGraph, BuildError> {
        let n = self.node_pkg.len() as u32;
        let np = self.packages.len() as u32;
        let path = |i: u32| {
            &self.path_buf
                [self.path_off[i as usize] as usize..self.path_off[i as usize + 1] as usize]
        };
        let check = |id: u32, what: &str| {
            if id >= n {
                Err(BuildError::BadReference(format!(
                    "{what} refers to file {id}, but there are {n} files"
                )))
            } else {
                Ok(())
            }
        };
        for (i, &p) in self.node_pkg.iter().enumerate() {
            if p >= np {
                return Err(BuildError::BadReference(format!(
                    "file `{}` refers to package {p}, but there are {np} packages",
                    path(i as u32)
                )));
            }
        }
        for (&f, &t) in self.e_from.iter().zip(&self.e_to) {
            check(f, "edge source")?;
            if t != NONE {
                check(t, "edge target")?;
            }
        }
        for (f, _) in &self.uncs {
            check(*f, "uncertainty")?;
        }
        for (f, _) in &self.probes {
            check(*f, "probe")?;
        }
        for (f, _) in &self.members {
            check(*f, "membership")?;
        }

        let mut by_path: Vec<u32> = (0..n).collect();
        by_path.sort_unstable_by(|&a, &b| path(a).cmp(path(b)));
        for w in by_path.windows(2) {
            if path(w[0]) == path(w[1]) {
                return Err(BuildError::DuplicatePath(path(w[0]).to_owned()));
            }
        }

        // Resolved edges: counting sort by source into CSR (stable).
        let mut offsets = vec![0u32; n as usize + 1];
        let mut unresolved = Vec::new();
        for (i, (&f, &t)) in self.e_from.iter().zip(&self.e_to).enumerate() {
            if t == NONE {
                unresolved.push((f, (self.e_attr[i], self.e_detail[i])));
            } else {
                offsets[f as usize + 1] += 1;
            }
        }
        for i in 0..n as usize {
            offsets[i + 1] += offsets[i];
        }
        let m = offsets[n as usize] as usize;
        let mut targets = vec![0u32; m];
        let mut attrs = vec![EdgeAttr(0); m];
        let mut details = vec![
            EdgeDetail {
                line: 0,
                span_start: NONE,
                span_end: NONE,
                spec: NONE,
            };
            m
        ];
        let mut cursor = offsets.clone();
        for (i, (&f, &t)) in self.e_from.iter().zip(&self.e_to).enumerate() {
            if t != NONE {
                let c = cursor[f as usize] as usize;
                targets[c] = t;
                attrs[c] = self.e_attr[i];
                details[c] = self.e_detail[i];
                cursor[f as usize] += 1;
            }
        }
        drop(cursor);

        Ok(MemGraph {
            root: self.root,
            external: NodeSet::from_iter_in(n, self.external.into_iter().map(NodeId)),
            by_path,
            path_buf: self.path_buf,
            path_off: self.path_off,
            node_pkg: self.node_pkg,
            node_lang: self.node_lang,
            langs: self.langs,
            packages: self.packages,
            fwd: Adjacency::from_parts(offsets, targets, attrs),
            fwd_detail: details,
            unresolved: Grouped::build(n, unresolved),
            specs: self.specs,
            uncs: Grouped::build(n, self.uncs),
            strs: self.strs,
            probes: Grouped::build(n, self.probes),
            members: Grouped::build(n, self.members),
        })
    }
}

/// Guesses a language id from a file extension (used by
/// [`GraphBuilder::file`]).
pub fn guess_lang(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1;
    Some(match ext {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => "js",
        "py" => "py",
        "go" => "go",
        "rs" => "rust",
        _ => return None,
    })
}

impl MemGraph {
    /// A new builder.
    pub fn builder() -> GraphBuilder {
        GraphBuilder::default()
    }

    /// The absolute root, if recorded.
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    /// Number of resolved plus unresolved edges.
    pub fn edge_count(&self) -> usize {
        self.fwd.edge_count() + self.unresolved.items.len()
    }
}

impl GraphView for MemGraph {
    fn node_count(&self) -> u32 {
        self.node_pkg.len() as u32
    }

    fn path(&self, n: NodeId) -> &str {
        &self.path_buf[self.path_off[n.index()] as usize..self.path_off[n.index() + 1] as usize]
    }

    fn package(&self, n: NodeId) -> PackageId {
        PackageId(self.node_pkg[n.index()])
    }

    fn lang(&self, n: NodeId) -> Option<&str> {
        match self.node_lang[n.index()] {
            u16::MAX => None,
            l => Some(&self.langs[l as usize]),
        }
    }

    fn is_external(&self, n: NodeId) -> bool {
        self.external.contains(n)
    }

    fn package_count(&self) -> u32 {
        self.packages.len() as u32
    }

    fn package_info(&self, p: PackageId) -> PackageInfo<'_> {
        let pkg = &self.packages[p.0 as usize];
        PackageInfo {
            name: &pkg.name,
            version: match &pkg.version {
                None => PackageVersion::Workspace,
                Some(v) => PackageVersion::Version(v),
            },
            root: &pkg.root,
        }
    }

    fn out_edges(&self, n: NodeId) -> impl Iterator<Item = Edge<'_>> + '_ {
        let spec = |d: &EdgeDetail| (d.spec != NONE).then(|| &*self.specs[d.spec as usize]);
        let range = self.fwd.range(n);
        let (targets, attrs) = self.fwd.neighbors(n);
        let resolved =
            targets
                .iter()
                .zip(attrs)
                .zip(&self.fwd_detail[range])
                .map(move |((&t, &attr), d)| Edge {
                    to: Some(NodeId(t)),
                    attr,
                    line: d.line,
                    span: d.span(),
                    specifier: spec(d),
                });
        let unresolved = self.unresolved.get(n).iter().map(move |(attr, d)| Edge {
            to: None,
            attr: *attr,
            line: d.line,
            span: d.span(),
            specifier: spec(d),
        });
        resolved.chain(unresolved)
    }

    fn uncertainties(&self, n: NodeId) -> impl Iterator<Item = Uncertainty<'_>> + '_ {
        self.uncs.get(n).iter().map(|u| Uncertainty {
            kind: u.kind,
            line: u.line,
            span: u.span,
            detail: &u.detail,
            scope: u
                .scope
                .as_ref()
                .map(|(dir, pattern)| Membership { dir, pattern }),
            effect: u.effect,
            origin: u.origin,
        })
    }

    fn probes(&self, n: NodeId) -> impl Iterator<Item = &str> + '_ {
        self.probes.get(n).iter().map(|&r| arena_get(&self.strs, r))
    }

    fn memberships(&self, n: NodeId) -> impl Iterator<Item = Membership<'_>> + '_ {
        self.members.get(n).iter().map(|&(d, p)| Membership {
            dir: arena_get(&self.strs, d),
            pattern: arena_get(&self.strs, p),
        })
    }

    fn node_by_path(&self, path: &str) -> Option<NodeId> {
        let path = normalize(path);
        self.by_path
            .binary_search_by(|&i| self.path(NodeId(i)).cmp(path))
            .ok()
            .map(|i| NodeId(self.by_path[i]))
    }

    fn glob(&self, glob: &Glob, out: &mut NodeSet) {
        for prefix in glob.literal_prefixes() {
            let lo = self
                .by_path
                .partition_point(|&i| self.path(NodeId(i)).as_bytes() < prefix);
            for &i in &self.by_path[lo..] {
                let p = self.path(NodeId(i));
                if !p.as_bytes().starts_with(prefix) {
                    break;
                }
                if glob.matches(p) {
                    out.insert(NodeId(i));
                }
            }
        }
    }

    fn has_uncertainty(&self, n: NodeId) -> bool {
        !self.uncs.get(n).is_empty()
    }

    fn forward_adjacency(&self) -> Option<&Adjacency> {
        Some(&self.fwd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_read() {
        let mut b = MemGraph::builder();
        let a = b.file("./src/a.ts");
        let c = b.file("src/c.ts");
        b.add_edge(a, EdgeSpec::new(None, RefKind::Require).specifier("x"));
        b.edge(a, c, RefKind::Import);
        b.probe(a, "src/x.ts")
            .membership(a, "src/locales/", "*.json");
        b.uncertainty(a, UncertaintyKind::ComputedSpecifier, 3, "require(x)");
        let g = b.build().unwrap();
        assert_eq!(g.path(a), "src/a.ts");
        assert_eq!(g.lang(a), Some("js"));
        assert_eq!(g.node_by_path("./src/c.ts"), Some(c));
        let edges: Vec<_> = g.out_edges(a).collect();
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].to, Some(c));
        assert_eq!(edges[1].specifier, Some("x"));
        assert_eq!(g.probes(a).collect::<Vec<_>>(), ["src/x.ts"]);
        assert_eq!(g.memberships(a).next().unwrap().dir, "src/locales");
        assert!(g.has_uncertainty(a) && !g.has_uncertainty(c));
        let mut s = NodeSet::new(2);
        g.glob(&Glob::new("src/*.ts").unwrap(), &mut s);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn duplicate_path() {
        let mut b = MemGraph::builder();
        b.file("a.js");
        b.file("./a.js");
        assert_eq!(
            b.build().unwrap_err(),
            BuildError::DuplicatePath("a.js".into())
        );
    }
}
