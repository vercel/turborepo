//! Parallel traversal.
//!
//! One rayon task covers one visit: dedupe, then (for files followed as code)
//! detect the language, read the file into a per-thread reused buffer, extract,
//! resolve and spawn tasks for the successors. Rayon's scope is a work-stealing
//! pool, so idle threads steal pending visits from busy ones.
//!
//! Per-file results ([`FileNode`]) are cached in the tracer, so later `trace()`
//! calls that reach the same files do not read, extract or resolve them again.

use std::{cell::RefCell, collections::HashSet, io::Read, path::Path, sync::Arc};

use dashmap::DashMap;

use crate::{
    Edge, ExtractCx, FileId, RawRef, RefFlags, RefKind, RefSink, ResolveCx, Target, Uncertainty,
    UncertaintyKind, os::OsStrExt, trace::Shared,
};

/// Visit state bits.
pub(crate) const INCLUDED: u8 = 1;
pub(crate) const PARSED: u8 = 2;

/// Read buffers above this capacity are not kept for reuse, so one huge file
/// does not pin memory on a worker thread for the rest of the process.
const MAX_RETAINED_BUF: usize = 16 << 20;

/// Bytes passed to [`crate::Language::sniff`].
const SNIFF_LEN: usize = 512;

thread_local! {
    static READ_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// The real path of a file plus the symlinks crossed to reach it, from
/// [`Shared::realize`].
#[derive(Debug)]
pub(crate) struct Realized {
    pub real: FileId,
    pub links: Vec<FileId>,
}

/// The result of [`Shared::resolve_refs`]: edges with their successors
/// (parallel vectors) and, when recording, the edge range and observations of
/// each reference.
#[derive(Debug, Default)]
pub(crate) struct Resolved {
    pub edges: Vec<Edge>,
    pub succ: Vec<Vec<FileId>>,
    pub links: Vec<Vec<FileId>>,
    pub groups: Vec<RefGroup>,
}

/// The edges one reference produced and what its resolution observed (graph
/// mode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefGroup {
    /// Index of the reference in the file's extraction order, or `None` for the
    /// file-level edges ([`crate::Resolver::file_edges`]).
    pub ref_index: Option<u32>,
    /// The reference's edges: a range into [`crate::FileAnalysis::edges`].
    pub edges: std::ops::Range<usize>,
    /// Filesystem observations of the resolution.
    pub observations: crate::record::Observations,
}

/// Cached extraction plus resolution result for one file.
#[derive(Debug, Default)]
pub(crate) struct FileNode {
    /// Index of the detected language, if any. `None` means a leaf.
    pub lang: Option<usize>,
    /// Outgoing edges, in extraction order.
    pub edges: Vec<Edge>,
    /// Concrete files reached through each edge (parallel to `edges`): the file
    /// itself, glob matches, or files below a directory. Always real paths
    /// (symlinks resolved).
    pub succ: Vec<Vec<FileId>>,
    /// Symlinks crossed to reach `succ` (parallel to `edges`). They are
    /// included in the output as unparsed files, as nft does.
    pub links: Vec<Vec<FileId>>,
    /// File-level uncertainties (lexer recovery, read errors).
    pub diagnostics: Vec<Uncertainty>,
}

impl Shared {
    /// Returns the cached node for `file`, computing it on first use.
    pub(crate) fn node(&self, file: FileId) -> Arc<FileNode> {
        if let Some(n) = self.nodes.get(&file) {
            return Arc::clone(&n);
        }
        let node = Arc::new(self.compute_node(file));
        // Two concurrent `trace()` calls may compute the same node; keep the first.
        Arc::clone(&self.nodes.entry(file).or_insert(node))
    }

    fn compute_node(&self, file: FileId) -> FileNode {
        let path = self.interner.path(file);
        let by_ext = self.langs.by_extension(&path);
        if by_ext.is_none() && (self.langs.is_empty() || has_extension(&path)) {
            return FileNode::default();
        }

        // Take the buffer out of the thread-local instead of borrowing it, so a
        // resolver that itself uses rayon (and might run another visit on this
        // thread) cannot cause a re-entrant borrow.
        let mut buf = READ_BUF.with(|b| std::mem::take(&mut *b.borrow_mut()));
        buf.clear();
        let node = match read_into(&path, &mut buf) {
            Err(e) => FileNode {
                lang: by_ext,
                diagnostics: vec![Uncertainty::new(
                    UncertaintyKind::ParseIncomplete,
                    crate::Span::DUMMY,
                    format!("cannot read file: {e}"),
                )],
                ..Default::default()
            },
            Ok(()) => {
                let lang = self.langs.detect(&path, &buf[..buf.len().min(SNIFF_LEN)]);
                match lang {
                    Some(li) => self.extract_and_resolve(file, &path, li, &buf),
                    None => FileNode::default(),
                }
            }
        };
        if buf.capacity() <= MAX_RETAINED_BUF {
            READ_BUF.with(|b| *b.borrow_mut() = buf);
        }
        node
    }

    fn extract_and_resolve(&self, file: FileId, path: &Path, li: usize, src: &[u8]) -> FileNode {
        let Some(lang) = self.langs.get(li) else {
            return FileNode::default();
        };
        let cx = ExtractCx {
            file,
            path,
            options: &self.options,
        };
        let mut sink = RefSink::new();
        lang.extract(src, &cx, &mut sink);
        let (refs, diagnostics) = sink.into_parts();
        let resolved = self.resolve_refs(file, path, li, &refs, false);
        FileNode {
            lang: Some(li),
            edges: resolved.edges,
            succ: resolved.succ,
            links: resolved.links,
            diagnostics,
        }
    }

    /// Resolves the references of one file (in extraction order) and computes
    /// the successors of every edge. With `record`, each reference, and the
    /// file-level edges, is resolved inside its own [`crate::record`] frame
    /// and reported as a [`RefGroup`].
    pub(crate) fn resolve_refs(
        &self,
        file: FileId,
        path: &Path,
        li: usize,
        refs: &[RawRef<'_>],
        record: bool,
    ) -> Resolved {
        let mut out = Resolved::default();
        let Some(resolver) = self.resolvers.get(li) else {
            return out;
        };
        let opts = &self.options;
        let from_dir = path.parent().unwrap_or(Path::new("/"));
        let rcx = ResolveCx {
            from_path: path,
            from_dir,
            options: opts,
            fs: &self.fs,
            interner: &self.interner,
        };

        let edges = &mut out.edges;
        edges.reserve(refs.len());
        let mut seen_manifests = HashSet::new();
        for (index, r) in refs.iter().enumerate() {
            if !opts.phase.allows(r.phase)
                || (r.flags.contains(RefFlags::TYPE_ONLY) && !opts.include_types)
            {
                continue;
            }
            let frame = record.then(crate::record::start);
            let first = edges.len();
            let res = resolver.resolve(r, file, &rcx);
            let uncertainty = res.uncertainty.or_else(|| r.uncertainty.clone());
            let spec: Box<str> = r.spec.to_string().into();
            let mut targets = res.targets.into_iter().peekable();
            if targets.peek().is_none() && !res.none {
                let u = uncertainty.clone().unwrap_or_else(|| {
                    Uncertainty::new(
                        UncertaintyKind::UnresolvedSpecifier,
                        r.span,
                        format!("cannot resolve {spec}"),
                    )
                });
                edges.push(Edge {
                    from: file,
                    to: Target::Unresolved,
                    kind: r.kind,
                    phase: r.phase,
                    flags: r.flags,
                    span: r.span,
                    spec: spec.clone(),
                    uncertainty: Some(u),
                });
            }
            for to in targets.filter(|_| !res.none) {
                edges.push(Edge {
                    from: file,
                    to,
                    kind: r.kind,
                    phase: r.phase,
                    flags: r.flags,
                    span: r.span,
                    spec: spec.clone(),
                    uncertainty: uncertainty.clone(),
                });
            }
            for m in res.manifests {
                if seen_manifests.insert(m) {
                    edges.push(Edge {
                        from: file,
                        to: Target::File(m),
                        kind: RefKind::Manifest,
                        phase: r.phase,
                        flags: RefFlags::empty(),
                        span: r.span,
                        spec: spec.clone(),
                        uncertainty: None,
                    });
                }
            }
            for mut x in res.extra {
                if let Some(u) = &mut x.uncertainty
                    && u.span == crate::Span::DUMMY
                {
                    u.span = r.span;
                }
                let phase = x.phase.unwrap_or(r.phase);
                if !opts.phase.allows(phase) {
                    continue;
                }
                if x.kind == RefKind::Manifest
                    && let Target::File(m) = x.target
                    && !seen_manifests.insert(m)
                {
                    continue;
                }
                edges.push(Edge {
                    from: file,
                    to: x.target,
                    kind: x.kind,
                    phase,
                    flags: r.flags | x.flags,
                    span: r.span,
                    spec: x.spec,
                    uncertainty: x.uncertainty,
                });
            }
            for e in &mut edges[first..] {
                let (succ, links) = self.successors(e);
                out.succ.push(succ);
                out.links.push(links);
            }
            if let Some(frame) = frame {
                let mut observations = frame.finish();
                observations.dedup();
                out.groups.push(RefGroup {
                    ref_index: Some(index as u32),
                    edges: first..edges.len(),
                    observations,
                });
            }
        }

        // Edges of the file as a whole (its own package scope, special cases).
        let frame = record.then(crate::record::start);
        let first = edges.len();
        for x in resolver.file_edges(file, &rcx) {
            let phase = x.phase.unwrap_or_else(|| x.kind.default_phase());
            if !opts.phase.allows(phase) {
                continue;
            }
            if x.kind == RefKind::Manifest
                && let Target::File(m) = x.target
                && !seen_manifests.insert(m)
            {
                continue;
            }
            edges.push(Edge {
                from: file,
                to: x.target,
                kind: x.kind,
                phase,
                flags: x.flags,
                span: crate::Span::DUMMY,
                spec: x.spec,
                uncertainty: x.uncertainty,
            });
        }
        for e in &mut edges[first..] {
            let (succ, links) = self.successors(e);
            out.succ.push(succ);
            out.links.push(links);
        }
        if let Some(frame) = frame {
            let mut observations = frame.finish();
            observations.dedup();
            out.groups.push(RefGroup {
                ref_index: None,
                edges: first..edges.len(),
                observations,
            });
        }
        out
    }

    /// Concrete files reached through `edge`, plus the symlinks crossed to
    /// reach them.
    ///
    /// Rewrites the edge target to real paths, so graph nodes are real files
    /// (pnpm layouts resolve to `node_modules/.pnpm/...`). Expands
    /// directory targets, marking the edge
    /// [`UncertaintyKind::PatternTooBroad`] if the expansion hits the glob
    /// limit. [`crate::fs::is_pseudo_fs`] for a path outside the base. The
    /// files of the tree are files wherever the tree is: a checkout on a
    /// RAM disk (`/dev/shm/ci/repo`) is below `/dev`, and every edge into
    /// it would have no successor.
    fn pseudo_fs(&self, path: &Path) -> bool {
        !path.starts_with(&self.options.base) && crate::fs::is_pseudo_fs(path)
    }

    fn successors(&self, edge: &mut Edge) -> (Vec<FileId>, Vec<FileId>) {
        // Kernel pseudo-filesystems are never part of a deployment or a build input; do
        // not even resolve their links (`/proc/self`).
        if let Target::File(id) | Target::Dir(id) = &edge.to
            && self.pseudo_fs(&self.interner.path(*id))
        {
            return (Vec::new(), Vec::new());
        }
        let mut links = Vec::new();
        let succ = match &mut edge.to {
            Target::File(id) => {
                *id = self.realize_into(*id, &mut links);
                vec![*id]
            }
            Target::Glob { matches, .. } => {
                matches.retain(|m| !self.pseudo_fs(&self.interner.path(*m)));
                for m in matches.iter_mut() {
                    *m = self.realize_into(*m, &mut links);
                }
                // A link and its target may both match.
                let mut seen = HashSet::new();
                matches.retain(|m| seen.insert(*m));
                matches.clone()
            }
            Target::Dir(id) => {
                *id = self.realize_into(*id, &mut links);
                let dir = self.interner.path(*id);
                // `.git` is not part of the tree: nothing below it is a dependency, and
                // how many files it holds says nothing about the repository.
                let (files, truncated) = if self.fs.is_repository_metadata(&dir) {
                    (Vec::new(), false)
                } else {
                    self.fs.walk_files(&dir, self.options.max_glob)
                };
                if truncated && edge.uncertainty.is_none() {
                    let u = Uncertainty::new(
                        UncertaintyKind::PatternTooBroad,
                        edge.span,
                        format!(
                            "directory {} has more than {} files; truncated",
                            dir.display(),
                            self.options.max_glob
                        ),
                    );
                    // A data read (not a module load) of the directory reads contents below
                    // it only: covered (see `Uncertainty::scope`).
                    let data = matches!(
                        edge.kind,
                        crate::RefKind::Asset | crate::RefKind::FsRead | crate::RefKind::FsReadDir
                    );
                    edge.uncertainty = Some(if data && edge.flags.contains(RefFlags::NAMES) {
                        // Names only: the `**` membership the walk recorded covers every
                        // path that can appear or disappear, and contents are not observed.
                        Uncertainty::new(
                            UncertaintyKind::PartialSpecifier,
                            edge.span,
                            format!(
                                "{}; names only, covered by the directory membership",
                                u.detail
                            ),
                        )
                    } else if data {
                        u.with_scope(crate::UncertaintyScope::below(dir.to_path_buf()))
                    } else {
                        u
                    });
                }
                let mut seen = HashSet::new();
                files
                    .iter()
                    .map(|p| self.realize_into(self.interner.intern(p), &mut links))
                    .filter(|id| seen.insert(*id))
                    .collect()
            }
            Target::Builtin(_) | Target::External(_) | Target::Unresolved => Vec::new(),
        };
        links.sort_unstable();
        links.dedup();
        (succ, links)
    }

    /// [`Shared::realize`], appending the links crossed to `links`.
    fn realize_into(&self, file: FileId, links: &mut Vec<FileId>) -> FileId {
        let r = self.realize(file);
        links.extend_from_slice(&r.links);
        r.real
    }

    /// The real path of `file` and the symlinks crossed to get there
    /// (`readlink` only for entries `d_type` reports as links). Records
    /// each link's target in [`Shared::symlinks`]. Cached per file.
    pub(crate) fn realize(&self, file: FileId) -> Arc<Realized> {
        if let Some(r) = self.real.get(&file) {
            return Arc::clone(&r);
        }
        let path = self.interner.path(file);
        // Memoized per file, so recording here would depend on which file got here
        // first.
        let rp = if self.fs.records() {
            crate::record::pause(|| self.link_resolver.resolve(&self.fs, &path))
        } else {
            self.link_resolver.resolve(&self.fs, &path)
        };
        let real = if *rp.path == *path {
            file
        } else {
            self.interner.intern(&rp.path)
        };
        let mut links = Vec::with_capacity(rp.links.len());
        for (link, target) in &rp.links {
            let l = self.interner.intern(link);
            let t = self.interner.intern(target);
            self.symlinks.insert(l, t);
            if !links.contains(&l) {
                links.push(l);
            }
        }
        let r = Arc::new(Realized { real, links });
        Arc::clone(&self.real.entry(file).or_insert(r))
    }

    /// Whether `file` matches an ignore glob.
    fn is_ignored(&self, file: FileId) -> bool {
        if self.ignore.is_empty() {
            return false;
        }
        let path = self.interner.path(file);
        let rel = path.strip_prefix(&self.options.base).unwrap_or(&path);
        let bytes = rel.as_os_str().as_bytes();
        self.ignore
            .matches(bytes.strip_prefix(b"/").unwrap_or(bytes))
    }
}

fn has_extension(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.as_bytes())
        .is_some_and(|n| n.len() > 1 && n[1..].contains(&b'.'))
}

fn read_into(path: &Path, buf: &mut Vec<u8>) -> std::io::Result<()> {
    let mut f = std::fs::File::open(path)?;
    if let Ok(meta) = f.metadata() {
        buf.reserve(usize::try_from(meta.len()).unwrap_or(0));
    }
    f.read_to_end(buf)?;
    Ok(())
}

/// One traversal over the shared tracer state.
pub(crate) struct Run<'t> {
    shared: &'t Shared,
    /// Visit state per file: [`INCLUDED`] and [`PARSED`] bits.
    pub state: DashMap<FileId, u8>,
}

impl<'t> Run<'t> {
    pub fn new(shared: &'t Shared) -> Self {
        Run {
            shared,
            state: DashMap::new(),
        }
    }

    /// Traverses from `entries` (real paths) until no new files are found.
    /// `entry_links` are symlinks crossed to reach the entries; they are
    /// included unparsed.
    pub fn run(&self, entries: &[FileId], entry_links: &[FileId]) {
        self.shared.pool.scope(|s| {
            for &e in entries {
                s.spawn(move |s| self.visit(s, e, true, true));
            }
            for &l in entry_links {
                s.spawn(move |s| self.visit(s, l, false, true));
            }
        });
    }

    /// Whether a visit with `parse` would change anything for `file`.
    fn is_new(&self, file: FileId, parse: bool) -> bool {
        let want = if parse { INCLUDED | PARSED } else { INCLUDED };
        self.state.get(&file).is_none_or(|s| *s & want != want)
    }

    fn visit<'s>(&'s self, scope: &rayon::Scope<'s>, file: FileId, parse: bool, is_entry: bool) {
        if !is_entry && self.shared.is_ignored(file) {
            return;
        }
        let need_parse = {
            let mut st = self.state.entry(file).or_insert(0);
            let old = *st;
            *st |= INCLUDED | if parse { PARSED } else { 0 };
            parse && old & PARSED == 0
        };
        if need_parse {
            let node = self.shared.node(file);
            for ((edge, succ), links) in node.edges.iter().zip(&node.succ).zip(&node.links) {
                let follow = edge.kind.follows();
                for &to in succ {
                    if self.is_new(to, follow) {
                        scope.spawn(move |s| self.visit(s, to, follow, false));
                    }
                }
                for &l in links {
                    if self.is_new(l, false) {
                        scope.spawn(move |s| self.visit(s, l, false, false));
                    }
                }
            }
        }
    }
}
