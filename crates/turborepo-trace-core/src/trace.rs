//! The library entry point: [`Tracer`], built with
//! [`TracerBuilder`], produces a [`Trace`].

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;
use fixedbitset::FixedBitSet;

use crate::{
    Edge, Externals, FileDiagnostic, FileId, FileReason, PhaseFilter, RawRef, ReasonKinds, Target,
    TraceOptions, Uncertainty,
    fs::{DirCache, DirCacheStats, LinkResolver, normalize},
    glob::GlobSet,
    intern::{PathInterner, path_bytes},
    lang::{
        EnvRoot, ExtractCx, LangId, Language, LanguageSet, PackageIdentity, ProjectCx, RefSink,
        Resolver,
    },
    sched::{FileNode, INCLUDED, PARSED, Realized, Run},
};

/// Errors from building a tracer or running a trace.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// An entry does not exist.
    #[error("entry not found: {0}")]
    EntryNotFound(PathBuf),
    /// An entry exists but is not a regular file.
    #[error("entry is not a file: {0}")]
    EntryNotFile(PathBuf),
    /// A requested option is not implemented yet.
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),
    /// The thread pool could not be created.
    #[error("cannot create thread pool: {0}")]
    ThreadPool(String),
    /// An I/O error, such as failing to read the current directory.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// Configures a [`Tracer`]. Obtain one with [`Tracer::builder`].
#[derive(Default)]
pub struct TracerBuilder {
    options: TraceOptions,
    langs: Vec<Arc<dyn Language>>,
    record: bool,
    hidden: Option<PathBuf>,
    hidden_name: Option<String>,
}

impl std::fmt::Debug for TracerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracerBuilder")
            .field("options", &self.options)
            .field(
                "langs",
                &self.langs.iter().map(|l| l.id()).collect::<Vec<_>>(),
            )
            .field("record", &self.record)
            .finish()
    }
}

impl TracerBuilder {
    /// Replaces all options.
    pub fn options(mut self, options: TraceOptions) -> Self {
        self.options = options;
        self
    }

    /// Output base directory (default: the current directory).
    pub fn base(mut self, base: impl Into<PathBuf>) -> Self {
        self.options.base = base.into();
        self
    }

    /// Runtime working directory for cwd-relative reads (default: the current
    /// directory).
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.options.cwd = cwd.into();
        self
    }

    /// Which phases to include (default: all).
    pub fn phase(mut self, phase: PhaseFilter) -> Self {
        self.options.phase = phase;
        self
    }

    /// Include type-only references (default: false).
    pub fn include_types(mut self, yes: bool) -> Self {
        self.options.include_types = yes;
        self
    }

    /// Cap on glob and directory expansion (default: 1000).
    pub fn max_glob(mut self, n: usize) -> Self {
        self.options.max_glob = n;
        self
    }

    /// Adds an ignore glob.
    pub fn ignore(mut self, glob: impl Into<String>) -> Self {
        self.options.ignore.push(glob.into());
        self
    }

    /// Sets resolution conditions.
    pub fn conditions<S: Into<String>>(mut self, c: impl IntoIterator<Item = S>) -> Self {
        self.options.conditions = c.into_iter().map(Into::into).collect();
        self
    }

    /// Resolve every condition a package defines, in addition to the configured
    /// ones (default: false).
    pub fn all_conditions(mut self, yes: bool) -> Self {
        self.options.all_conditions = yes;
        self
    }

    /// Sets the build target (see [`crate::Platform::parse`] for accepted
    /// forms).
    pub fn target(mut self, t: impl Into<String>) -> Self {
        self.options.target = Some(t.into());
        self
    }

    /// Sets the target platform explicitly, overriding
    /// [`TracerBuilder::target`].
    pub fn platform(mut self, p: crate::Platform) -> Self {
        self.options.platform = Some(p);
        self
    }

    /// Include files for every target as conditional edges (default: false).
    /// See [`TraceOptions::all_targets`].
    pub fn all_targets(mut self, yes: bool) -> Self {
        self.options.all_targets = yes;
        self
    }

    /// Sets the dependency policy. [`Externals::Stop`] and [`Externals::List`]
    /// are honored by languages whose resolvers classify third-party
    /// packages (JS); others follow.
    pub fn externals(mut self, e: Externals) -> Self {
        self.options.externals = e;
        self
    }

    /// Worker threads; 0 means one per core (default).
    pub fn threads(mut self, n: usize) -> Self {
        self.options.threads = n;
        self
    }

    /// Emit file hashes. Not yet implemented; `build` fails when enabled.
    pub fn hashes(mut self, yes: bool) -> Self {
        self.options.hashes = yes;
        self
    }

    /// Paths outside the base are host paths and are never looked up (default:
    /// false). See [`TraceOptions::confined`].
    pub fn confined(mut self, yes: bool) -> Self {
        self.options.confined = yes;
        self
    }

    /// A directory that is not part of the traced tree: the shared fs cache
    /// hides it ([`DirCache::hide`]). Graph mode passes the graph directory
    /// when it is inside the repository.
    pub fn hide(mut self, dir: Option<&Path>) -> Self {
        self.hidden = dir.map(normalize);
        self
    }

    /// Directories of this name, in the base and below it, are not part of the
    /// traced tree either ([`DirCache::hide_named`]). Graph mode passes
    /// `.turbotrace`, the default name of a graph directory.
    pub fn hide_named(mut self, name: &str) -> Self {
        self.hidden_name = Some(name.to_owned());
        self
    }

    /// Graph mode: the shared fs cache reports observations to
    /// [`crate::record`], so [`Tracer::analyze`] can return probes,
    /// memberships and config reads per reference (default: false). Costs a
    /// little on every fs query; leave it off for `trace`.
    pub fn record(mut self, yes: bool) -> Self {
        self.record = yes;
        self
    }

    /// Registers a language. Earlier languages win extension conflicts.
    pub fn language(self, lang: impl Language) -> Self {
        self.language_arc(Arc::new(lang))
    }

    /// Registers a shared language.
    pub fn language_arc(mut self, lang: Arc<dyn Language>) -> Self {
        self.langs.push(lang);
        self
    }

    /// Validates options, creates resolvers and the thread pool.
    pub fn build(self) -> Result<Tracer, TraceError> {
        let mut options = self.options;
        if options.hashes {
            // TODO: fileHashes. Hash parsed files inline, leaves on a
            // low-priority pool, cache by (dev, inode, size, mtime_ns).
            return Err(TraceError::NotImplemented("file hashes (--hashes)"));
        }
        let cwd = std::env::current_dir().map_err(|source| TraceError::Io {
            context: "cannot read current directory".into(),
            source,
        })?;
        let absolutize = |p: &Path| {
            if p.as_os_str().is_empty() {
                cwd.clone()
            } else {
                normalize(&cwd.join(p))
            }
        };
        options.base = absolutize(&options.base);
        options.cwd = absolutize(&options.cwd);

        let fs = Arc::new(if self.record {
            DirCache::with_recording()
        } else {
            DirCache::new()
        });
        if let Some(h) = &self.hidden {
            fs.hide(h);
        }
        if let Some(n) = &self.hidden_name {
            fs.hide_named(&options.base, n);
        }
        let langs = LanguageSet::new(self.langs);
        let project = ProjectCx {
            options: &options,
            fs: &fs,
        };
        let resolvers: Vec<Box<dyn Resolver>> =
            langs.iter().map(|l| l.resolver(&project)).collect();
        let mut env_roots: Vec<EnvRoot> = resolvers
            .iter()
            .flat_map(|r| r.environment_roots())
            .map(|mut r| {
                r.path = normalize(&r.path);
                r
            })
            .filter(|r| !r.path.as_os_str().is_empty() && r.path != Path::new("/"))
            .collect();
        // Deepest first, so the most specific root names a file.
        env_roots.sort_by(|a, b| {
            b.path
                .as_os_str()
                .len()
                .cmp(&a.path.as_os_str().len())
                .then_with(|| a.cmp(b))
        });
        env_roots.dedup_by(|a, b| a.path == b.path);
        if options.confined {
            fs.confine(&options.base, env_roots.iter().map(|r| r.path.clone()));
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(options.threads)
            .thread_name(|i| format!("turbo-trace-{i}"))
            .build()
            .map_err(|e| TraceError::ThreadPool(e.to_string()))?;
        let ignore = GlobSet::new(&options.ignore);
        let link_resolver = LinkResolver::new(&options.base);
        Ok(Tracer {
            shared: Shared {
                options,
                langs,
                resolvers,
                env_roots,
                link_resolver,
                fs,
                interner: PathInterner::new(),
                nodes: DashMap::new(),
                real: DashMap::new(),
                symlinks: DashMap::new(),
                ignore,
                pool,
            },
        })
    }
}

/// State shared by all traversals of one tracer.
pub(crate) struct Shared {
    pub options: TraceOptions,
    pub langs: LanguageSet,
    pub resolvers: Vec<Box<dyn Resolver>>,
    /// Environment roots of all resolvers, deepest first.
    pub env_roots: Vec<EnvRoot>,
    pub fs: Arc<DirCache>,
    /// Resolves symlinked path components below the base.
    pub link_resolver: LinkResolver,
    pub interner: PathInterner,
    pub nodes: DashMap<FileId, Arc<FileNode>>,
    /// Memo for [`Shared::realize`].
    pub real: DashMap<FileId, Arc<Realized>>,
    /// Every symlink encountered: link path to its resolved target (a file or
    /// directory).
    pub symlinks: DashMap<FileId, FileId>,
    pub ignore: GlobSet,
    pub pool: rayon::ThreadPool,
}

/// A reusable tracer. Caches (directory listings, interned paths, per-file
/// extraction and resolution results) persist across [`Tracer::trace`] calls,
/// like nft's reusable `cache`.
///
/// The caches assume the filesystem does not change while the tracer is in use;
/// call [`Tracer::clear_cache`] after changes.
pub struct Tracer {
    pub(crate) shared: Shared,
}

impl std::fmt::Debug for Tracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracer")
            .field("options", &self.shared.options)
            .field("langs", &self.shared.langs)
            .finish_non_exhaustive()
    }
}

/// Raw references and diagnostics for one file, from [`Tracer::extract`].
#[derive(Debug)]
pub struct Extraction {
    /// The file's absolute path.
    pub path: PathBuf,
    /// The detected language, or `None` if no language handles the file.
    pub lang: Option<LangId>,
    /// References, in source order as reported by the language.
    pub refs: Vec<RawRef<'static>>,
    /// File-level diagnostics.
    pub diagnostics: Vec<Uncertainty>,
    /// Facts for other files' analyses ([`crate::RefSink::fact`]).
    pub facts: Vec<String>,
}

impl Tracer {
    /// Starts configuring a tracer.
    pub fn builder() -> TracerBuilder {
        TracerBuilder::default()
    }

    /// The effective options (with absolute `base` and `cwd`).
    pub fn options(&self) -> &TraceOptions {
        &self.shared.options
    }

    /// The registered languages.
    pub fn languages(&self) -> &LanguageSet {
        &self.shared.langs
    }

    /// The shared directory cache.
    pub fn fs(&self) -> &DirCache {
        &self.shared.fs
    }

    /// The shared directory cache as a shared handle, for components that route
    /// their own file access through it (test-framework adapters in graph
    /// builds).
    pub fn fs_arc(&self) -> &Arc<DirCache> {
        &self.shared.fs
    }

    /// Interns an absolute, normalized path (for example a [`crate::Base::Dir`]
    /// anchor of a cached extraction).
    pub fn intern(&self, path: &Path) -> FileId {
        self.shared.interner.intern(path)
    }

    /// Drops all caches.
    pub fn clear_cache(&self) {
        self.shared.fs.clear();
        self.shared.link_resolver.clear();
        self.shared.nodes.clear();
        self.shared.real.clear();
        self.shared.symlinks.clear();
        for r in &self.shared.resolvers {
            r.clear_cache();
        }
    }

    /// Resolves an entry argument to an absolute, normalized path relative to
    /// the current directory and checks that it is a file.
    fn entry_path(&self, entry: &Path) -> Result<PathBuf, TraceError> {
        let cwd = std::env::current_dir().map_err(|source| TraceError::Io {
            context: "cannot read current directory".into(),
            source,
        })?;
        let path = normalize(&cwd.join(entry));
        match self.shared.fs.stat(&path) {
            None => Err(TraceError::EntryNotFound(path)),
            Some(s) if s.kind != crate::fs::FileKind::File => Err(TraceError::EntryNotFile(path)),
            Some(_) => Ok(path),
        }
    }

    /// Traces from `entries` (relative paths are resolved against the current
    /// directory).
    ///
    /// Builds one shared graph for all entries and computes each entry's
    /// closure. Output order is deterministic and independent of the thread
    /// count.
    ///
    /// Graph nodes are real paths: symlinked components below the base are
    /// resolved, and each symlink crossed is included as a file of its own
    /// (see [`Trace::symlinks`]). An entry given through a symlink is
    /// traced at its real path.
    pub fn trace<P: AsRef<Path>>(&self, entries: &[P]) -> Result<Trace, TraceError> {
        let mut entry_ids = Vec::with_capacity(entries.len());
        let mut entry_links = Vec::with_capacity(entries.len());
        for e in entries {
            let p = self.entry_path(e.as_ref())?;
            let r = self.shared.realize(self.shared.interner.intern(&p));
            if !entry_ids.contains(&r.real) {
                entry_ids.push(r.real);
                entry_links.push(r.links.clone());
            }
        }
        let all_links: Vec<FileId> = entry_links.iter().flatten().copied().collect();
        let run = Run::new(&self.shared);
        run.run(&entry_ids, &all_links);
        Ok(Trace::build(&self.shared, &run, &entry_ids, &entry_links))
    }

    /// Extracts raw references from one file without resolving them.
    pub fn extract(&self, file: &Path) -> Result<Extraction, TraceError> {
        let path = self.entry_path(file)?;
        let src = std::fs::read(&path).map_err(|source| TraceError::Io {
            context: format!("cannot read {}", path.display()),
            source,
        })?;
        let langs = &self.shared.langs;
        let lang = langs
            .detect(&path, &src[..src.len().min(512)])
            .and_then(|i| langs.get(i));
        let Some(lang) = lang else {
            return Ok(Extraction {
                path,
                lang: None,
                refs: Vec::new(),
                diagnostics: Vec::new(),
                facts: Vec::new(),
            });
        };
        let id = self.shared.interner.intern(&path);
        let cx = ExtractCx {
            file: id,
            path: &path,
            options: &self.shared.options,
        };
        let mut sink = RefSink::new();
        lang.extract(&src, &cx, &mut sink);
        let facts = sink.take_facts();
        let (refs, diagnostics) = sink.into_parts();
        let refs = refs
            .into_iter()
            .map(|r| RawRef {
                spec: r.spec.into_owned(),
                kind: r.kind,
                base: r.base,
                span: r.span,
                flags: r.flags,
                phase: r.phase,
                uncertainty: r.uncertainty,
            })
            .collect();
        Ok(Extraction {
            path,
            lang: Some(lang.id()),
            refs,
            diagnostics,
            facts,
        })
    }
}

/// Deterministic counts for one trace.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceStats {
    /// Distinct entries.
    pub entries: usize,
    /// Files in the union of all closures.
    pub files: usize,
    /// Files that were extracted by a language.
    pub parsed_files: usize,
    /// Edges.
    pub edges: usize,
    /// Distinct uncertainty records (edge and file level), as in
    /// [`Trace::uncertainties`].
    pub uncertainties: usize,
}

/// One step of a [`Trace::why`] chain.
#[derive(Copy, Clone, Debug)]
pub struct WhyStep<'t> {
    /// The including file.
    pub from: &'t Path,
    /// The included file.
    pub to: &'t Path,
    /// How.
    pub via: WhyVia<'t>,
}

/// How one file pulls in the next in a [`Trace::why`] chain.
#[derive(Copy, Clone, Debug)]
pub enum WhyVia<'t> {
    /// Through a reference.
    Edge(&'t Edge),
    /// `to` is the symlink target of `from`.
    Symlink,
}

#[derive(Clone, Debug)]
struct Node {
    id: FileId,
    path: Arc<Path>,
    parsed: bool,
    lang: Option<LangId>,
    /// Index into [`Trace::env_roots`] for a file outside the base below an
    /// environment root.
    env_root: Option<u32>,
}

/// Adjacency entry: target node and the edge index (`None` for symlink links).
type Adj = (u32, Option<u32>);

/// The result of [`Tracer::trace`]: a shared graph over all entries with
/// per-entry closures.
///
/// All iteration orders are deterministic: files sort by path bytes, edges by
/// source file path, span, kind and target.
#[derive(Debug)]
pub struct Trace {
    base: PathBuf,
    nodes: Vec<Node>,
    index: HashMap<FileId, u32>,
    /// Paths for every FileId mentioned anywhere (nodes plus ignored targets).
    paths: HashMap<FileId, Arc<Path>>,
    edges: Vec<Edge>,
    diagnostics: Vec<FileDiagnostic>,
    adj: Vec<Vec<Adj>>,
    reasons: Vec<(ReasonKinds, Vec<u32>)>,
    /// `(link node, resolved target)`, sorted by link. The target may be a
    /// directory.
    symlinks: Vec<(u32, Arc<Path>)>,
    entries: Vec<u32>,
    closures: Vec<FixedBitSet>,
    /// Environment roots with at least one file, sorted by name then path, with
    /// their file counts.
    env_roots: Vec<(EnvRoot, usize)>,
}

impl Trace {
    fn build(
        shared: &Shared,
        run: &Run<'_>,
        entry_ids: &[FileId],
        entry_links: &[Vec<FileId>],
    ) -> Trace {
        let interner = &shared.interner;
        let mut nodes: Vec<Node> = run
            .state
            .iter()
            .filter(|kv| *kv.value() & INCLUDED != 0)
            .map(|kv| Node {
                id: *kv.key(),
                path: interner.path(*kv.key()),
                parsed: *kv.value() & PARSED != 0,
                lang: None,
                env_root: None,
            })
            .collect();
        nodes.sort_unstable_by(|a, b| path_bytes(&a.path).cmp(path_bytes(&b.path)));
        let index: HashMap<FileId, u32> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i as u32))
            .collect();
        let mut paths: HashMap<FileId, Arc<Path>> =
            nodes.iter().map(|n| (n.id, Arc::clone(&n.path))).collect();

        // Gather edges with their successors and a deterministic sort key.
        let mut gathered: Vec<(Edge, Vec<FileId>, Vec<FileId>)> = Vec::new();
        let mut diagnostics = Vec::new();
        for n in nodes.iter_mut().filter(|n| n.parsed) {
            let Some(fnode) = shared.nodes.get(&n.id).map(|r| Arc::clone(&r)) else {
                continue;
            };
            n.lang = fnode
                .lang
                .and_then(|li| shared.langs.get(li))
                .map(|l| l.id());
            // Reached by a following kind but handled by no language (`.c`, `.s`): a leaf.
            if fnode.lang.is_none() {
                n.parsed = false;
            }
            for u in &fnode.diagnostics {
                diagnostics.push(FileDiagnostic {
                    file: n.id,
                    uncertainty: u.clone(),
                });
            }
            for ((e, succ), links) in fnode.edges.iter().zip(&fnode.succ).zip(&fnode.links) {
                let mut mention = |id: FileId| {
                    paths.entry(id).or_insert_with(|| interner.path(id));
                };
                match &e.to {
                    Target::File(id) | Target::Dir(id) => mention(*id),
                    Target::Glob { matches, .. } => matches.iter().copied().for_each(&mut mention),
                    _ => {}
                }
                succ.iter().copied().for_each(&mut mention);
                links.iter().copied().for_each(&mut mention);
                gathered.push((e.clone(), succ.clone(), links.clone()));
            }
        }
        let key_path = |id: &FileId| path_bytes(&paths[id]).to_vec();
        gathered.sort_by_cached_key(|(e, _, _)| {
            (
                index.get(&e.from).copied(),
                e.span,
                e.kind,
                target_key(&e.to, &key_path),
                e.spec.clone(),
            )
        });
        // Normalize glob match order by path as well.
        for (e, succ, _) in &mut gathered {
            if let Target::Glob { matches, .. } = &mut e.to {
                matches.sort_by_cached_key(&key_path);
            }
            succ.sort_by_cached_key(&key_path);
        }
        diagnostics.sort_by(|a, b| {
            (index.get(&a.file), a.uncertainty.span, a.uncertainty.kind).cmp(&(
                index.get(&b.file),
                b.uncertainty.span,
                b.uncertainty.kind,
            ))
        });

        // Adjacency and reasons.
        let mut adj: Vec<Vec<Adj>> = vec![Vec::new(); nodes.len()];
        let mut reasons: Vec<(ReasonKinds, Vec<u32>)> =
            vec![(ReasonKinds::empty(), Vec::new()); nodes.len()];
        let link_target = |id: &FileId| shared.symlinks.get(id).map(|t| *t);
        let mut edges = Vec::with_capacity(gathered.len());
        // First edge (in sorted order) reaching each node, for outside-base reports.
        let mut first_ref: Vec<Option<(u32, crate::Span, Box<str>)>> = vec![None; nodes.len()];
        for (ei, (e, succ, links)) in gathered.into_iter().enumerate() {
            if let Some(&from) = index.get(&e.from) {
                for to in succ.iter().chain(&links) {
                    if let Some(&ti) = index.get(to)
                        && first_ref[ti as usize].is_none()
                        && ti != from
                    {
                        first_ref[ti as usize] = Some((from, e.span, e.spec.clone()));
                    }
                }
                // Symlinks crossed: included with reason `resolve`, as nft does.
                let mut via_link = Vec::new();
                for l in &links {
                    if let Some(&li) = index.get(l) {
                        adj[from as usize].push((li, Some(ei as u32)));
                        reasons[li as usize].0 |= ReasonKinds::RESOLVE;
                        reasons[li as usize].1.push(from);
                        via_link.extend(link_target(l));
                    }
                }
                for to in succ {
                    if let Some(&ti) = index.get(&to) {
                        // A file reached through a file symlink hangs off the link, so
                        // `why` shows the link; otherwise it hangs off the referencing file.
                        if !via_link.contains(&to) {
                            adj[from as usize].push((ti, Some(ei as u32)));
                        }
                        reasons[ti as usize].0 |= e.kind.reason();
                        reasons[ti as usize].1.push(from);
                    }
                }
            }
            edges.push(e);
        }
        let mut symlinks = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            if let Some(t) = link_target(&n.id) {
                let tpath = paths.entry(t).or_insert_with(|| interner.path(t)).clone();
                symlinks.push((i as u32, tpath));
                if let Some(&ti) = index.get(&t) {
                    adj[i].push((ti, None));
                    reasons[ti as usize].0 |= ReasonKinds::RESOLVE;
                    reasons[ti as usize].1.push(i as u32);
                }
            }
        }
        // Links crossed to reach an entry.
        let entry_link_nodes: Vec<Vec<u32>> = entry_links
            .iter()
            .map(|ls| ls.iter().filter_map(|l| index.get(l).copied()).collect())
            .collect();
        for &l in entry_link_nodes.iter().flatten() {
            reasons[l as usize].0 |= ReasonKinds::RESOLVE;
        }
        for a in &mut adj {
            a.sort_unstable();
            a.dedup_by_key(|(to, _)| *to);
        }
        let entries: Vec<u32> = entry_ids
            .iter()
            .filter_map(|id| index.get(id).copied())
            .collect();
        for &e in &entries {
            reasons[e as usize].0 |= ReasonKinds::INITIAL;
        }
        for r in &mut reasons {
            r.1.sort_unstable();
            r.1.dedup();
        }

        // Files outside the base are dropped from `nft`/`list` output: report each
        // once, on the file that first references it (or on itself for entries
        // and symlink targets reached without an edge).
        let base = &shared.options.base;
        let mut outside = false;
        let mut root_hits: Vec<usize> = vec![0; shared.env_roots.len()];
        // Below a language's environment root (site-packages, module cache, cargo
        // registry): expected, reported once per root instead of per file.
        for n in nodes.iter_mut().filter(|n| !n.path.starts_with(base)) {
            if let Some(ri) = shared
                .env_roots
                .iter()
                .position(|r| n.path.starts_with(&r.path))
            {
                root_hits[ri] += 1;
                n.env_root = Some(ri as u32);
            }
        }
        for (i, n) in nodes.iter().enumerate() {
            if n.path.starts_with(base) || n.env_root.is_some() {
                continue;
            }
            outside = true;
            let (file, span, via) = match &first_ref[i] {
                Some((from, span, spec)) => (nodes[*from as usize].id, *span, Some(spec)),
                None => match reasons[i].1.first() {
                    Some(&p) => (nodes[p as usize].id, crate::Span::DUMMY, None),
                    None => (n.id, crate::Span::DUMMY, None),
                },
            };
            let via = via.map(|s| format!(" (via '{s}')")).unwrap_or_default();
            diagnostics.push(FileDiagnostic {
                file,
                uncertainty: Uncertainty::new(
                    crate::UncertaintyKind::OutsideBase,
                    span,
                    format!(
                        "{} is outside the base {}{via}",
                        n.path.display(),
                        base.display()
                    ),
                ),
            });
        }
        if outside {
            diagnostics.sort_by(|a, b| {
                (
                    index.get(&a.file),
                    a.uncertainty.span,
                    a.uncertainty.kind,
                    &a.uncertainty.detail,
                )
                    .cmp(&(
                        index.get(&b.file),
                        b.uncertainty.span,
                        b.uncertainty.kind,
                        &b.uncertainty.detail,
                    ))
            });
        }

        // Compact the roots to those with files, in a stable display order.
        let mut used: Vec<(usize, &EnvRoot)> = shared
            .env_roots
            .iter()
            .enumerate()
            .filter(|(i, _)| root_hits[*i] > 0)
            .collect();
        used.sort_by(|a, b| a.1.cmp(b.1));
        let mut remap = vec![u32::MAX; shared.env_roots.len()];
        for (new, (old, _)) in used.iter().enumerate() {
            remap[*old] = new as u32;
        }
        for n in &mut nodes {
            if let Some(r) = &mut n.env_root {
                *r = remap[*r as usize];
            }
        }
        let env_roots: Vec<(EnvRoot, usize)> = used
            .iter()
            .map(|(i, r)| ((*r).clone(), root_hits[*i]))
            .collect();

        let closures = entry_ids
            .iter()
            .zip(&entry_link_nodes)
            .filter_map(|(id, links)| Some((index.get(id).copied()?, links)))
            .map(|(e, links)| {
                let mut seen = FixedBitSet::with_capacity(nodes.len());
                let mut queue = VecDeque::from([e]);
                seen.insert(e as usize);
                for &l in links {
                    if !seen.put(l as usize) {
                        queue.push_back(l);
                    }
                }
                while let Some(n) = queue.pop_front() {
                    for &(to, _) in &adj[n as usize] {
                        if !seen.put(to as usize) {
                            queue.push_back(to);
                        }
                    }
                }
                seen
            })
            .collect();

        Trace {
            base: shared.options.base.clone(),
            nodes,
            index,
            paths,
            edges,
            diagnostics,
            adj,
            reasons,
            symlinks,
            entries,
            closures,
            env_roots,
        }
    }

    /// Environment roots ([`crate::Resolver::environment_roots`]) that files of
    /// this trace lie in, with the number of such files, sorted by name.
    /// Files outside the base below an environment root get no per-file
    /// [`crate::UncertaintyKind::OutsideBase`] record; output formats
    /// report them per root instead (see [`crate::output`]).
    pub fn environment_roots(&self) -> &[(EnvRoot, usize)] {
        &self.env_roots
    }

    /// The environment root an included file outside the base lies in, if any.
    pub fn environment_root(&self, id: FileId) -> Option<&EnvRoot> {
        let &i = self.index.get(&id)?;
        let r = self.nodes[i as usize].env_root?;
        self.env_roots.get(r as usize).map(|(r, _)| r)
    }

    /// The base directory output paths are relative to.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Entry paths, in the order given (duplicates removed).
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &Path> {
        self.entries.iter().map(|&i| &*self.nodes[i as usize].path)
    }

    /// All files in the union of the entry closures, sorted by path.
    pub fn files(&self) -> impl ExactSizeIterator<Item = &Path> {
        self.nodes.iter().map(|n| &*n.path)
    }

    /// The ids of all files, sorted by path.
    pub fn file_ids(&self) -> impl ExactSizeIterator<Item = FileId> + '_ {
        self.nodes.iter().map(|n| n.id)
    }

    /// Files in the closure of the entry at `entry_index`, sorted by path.
    /// Empty if the index is out of range.
    pub fn entry_files(&self, entry_index: usize) -> impl Iterator<Item = &Path> {
        self.closures
            .get(entry_index)
            .into_iter()
            .flat_map(|c| c.ones())
            .map(|i| &*self.nodes[i].path)
    }

    /// Indexes into [`Trace::files`] (file ids in output order) of the closure
    /// of the entry at `entry_index`, ascending. Empty if the index is out
    /// of range.
    pub fn entry_file_indices(&self, entry_index: usize) -> impl Iterator<Item = usize> + '_ {
        self.closures
            .get(entry_index)
            .into_iter()
            .flat_map(|c| c.ones())
    }

    /// The index of `entry` among [`Trace::entries`].
    pub fn entry_index(&self, entry: &Path) -> Option<usize> {
        self.entries
            .iter()
            .position(|&i| &*self.nodes[i as usize].path == entry)
    }

    /// The path of any file id mentioned in this trace.
    pub fn path(&self, id: FileId) -> Option<&Path> {
        self.paths.get(&id).map(|p| &**p)
    }

    /// The id of an included file, by absolute path.
    pub fn file_id(&self, path: &Path) -> Option<FileId> {
        self.node_index(path).map(|i| self.nodes[i as usize].id)
    }

    fn node_index(&self, path: &Path) -> Option<u32> {
        self.nodes
            .binary_search_by(|n| path_bytes(&n.path).cmp(path_bytes(path)))
            .ok()
            .map(|i| i as u32)
    }

    /// Whether the file was extracted by a language (as opposed to a leaf).
    pub fn is_parsed(&self, id: FileId) -> bool {
        self.index
            .get(&id)
            .is_some_and(|&i| self.nodes[i as usize].parsed)
    }

    /// The language that extracted `id`, if any.
    pub fn language(&self, id: FileId) -> Option<LangId> {
        self.index
            .get(&id)
            .and_then(|&i| self.nodes[i as usize].lang)
    }

    /// All edges, sorted.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// File-level diagnostics, sorted.
    pub fn diagnostics(&self) -> &[FileDiagnostic] {
        &self.diagnostics
    }

    /// Every distinct uncertainty record (edges and file diagnostics) with its
    /// file, sorted by file path then span. A record shared by several
    /// edges of one reference (one per target) is listed once, with the
    /// first such edge.
    pub fn uncertainties(&self) -> Vec<(FileId, &Uncertainty, Option<&Edge>)> {
        let mut out: Vec<_> = self
            .edges
            .iter()
            .filter_map(|e| e.uncertainty.as_ref().map(|u| (e.from, u, Some(e))))
            .chain(
                self.diagnostics
                    .iter()
                    .map(|d| (d.file, &d.uncertainty, None)),
            )
            .collect();
        out.sort_by(|a, b| {
            (self.index.get(&a.0), a.1.span, a.1.kind).cmp(&(
                self.index.get(&b.0),
                b.1.span,
                b.1.kind,
            ))
        });
        // A reference with several targets (alternatives) yields one edge per target,
        // all carrying the same record: report it once.
        let mut seen = std::collections::HashSet::new();
        out.retain(|(f, u, _)| seen.insert((*f, *u)));
        out
    }

    /// Why `id` is included: reason kinds and parents (sorted by path).
    pub fn reason(&self, id: FileId) -> Option<FileReason> {
        let &i = self.index.get(&id)?;
        let (kinds, parents) = &self.reasons[i as usize];
        Some(FileReason {
            kinds: *kinds,
            parents: parents.iter().map(|&p| self.nodes[p as usize].id).collect(),
        })
    }

    /// Symlinks among the included files, as `(link, resolved target)`, sorted
    /// by link.
    ///
    /// Every symlink crossed while reaching a file is included: file links and
    /// directory links such as pnpm's `node_modules/foo ->
    /// .pnpm/foo@1.0.0/node_modules/foo`. The target is the link's real
    /// path; for directory links it is not itself a file of the trace.
    pub fn symlinks(&self) -> impl Iterator<Item = (&Path, &Path)> {
        self.symlinks
            .iter()
            .map(|(l, t)| (&*self.nodes[*l as usize].path, &**t))
    }

    /// Third-party packages that were not traversed (`--externals stop|list`),
    /// sorted and without duplicates.
    pub fn externals(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .edges
            .iter()
            .filter_map(|e| match &e.to {
                Target::External(name) => Some(&**name),
                _ => None,
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// A shortest chain of inclusions from `entry` to `file` (both absolute
    /// paths), or `None` if `file` is not in `entry`'s closure. An empty
    /// chain means `file == entry`.
    pub fn why(&self, entry: &Path, file: &Path) -> Option<Vec<WhyStep<'_>>> {
        let start = self.node_index(entry)?;
        if !self.entries.contains(&start) {
            return None;
        }
        let goal = self.node_index(file)?;
        let mut pred: Vec<Option<(u32, Option<u32>)>> = vec![None; self.nodes.len()];
        let mut seen = FixedBitSet::with_capacity(self.nodes.len());
        seen.insert(start as usize);
        let mut queue = VecDeque::from([start]);
        while let Some(n) = queue.pop_front() {
            if n == goal {
                break;
            }
            for &(to, via) in &self.adj[n as usize] {
                if !seen.put(to as usize) {
                    pred[to as usize] = Some((n, via));
                    queue.push_back(to);
                }
            }
        }
        if !seen.contains(goal as usize) {
            return None;
        }
        let mut steps = Vec::new();
        let mut cur = goal;
        while cur != start {
            let (from, via) = pred[cur as usize]?;
            steps.push(WhyStep {
                from: &self.nodes[from as usize].path,
                to: &self.nodes[cur as usize].path,
                via: match via {
                    Some(ei) => WhyVia::Edge(&self.edges[ei as usize]),
                    None => WhyVia::Symlink,
                },
            });
            cur = from;
        }
        steps.reverse();
        Some(steps)
    }

    /// Deterministic counts.
    pub fn stats(&self) -> TraceStats {
        TraceStats {
            entries: self.entries.len(),
            files: self.nodes.len(),
            parsed_files: self.nodes.iter().filter(|n| n.parsed).count(),
            edges: self.edges.len(),
            uncertainties: self.uncertainties().len(),
        }
    }

    /// `path` relative to the base, or `None` if it lies outside the base.
    pub fn relative<'p>(&self, path: &'p Path) -> Option<&'p Path> {
        path.strip_prefix(&self.base).ok()
    }
}

impl Tracer {
    /// The third-party package `path` belongs to according to the registered
    /// languages ([`crate::Resolver::package_identity`]), first match in
    /// registration order.
    pub fn package_identity(&self, path: &Path) -> Option<PackageIdentity> {
        self.shared
            .resolvers
            .iter()
            .find_map(|r| r.package_identity(path, &self.shared.fs))
    }

    /// The third-party package an external target `name` stands for
    /// ([`crate::Resolver::external_identity`]), first match in registration
    /// order.
    pub fn external_identity(&self, name: &str) -> Option<PackageIdentity> {
        self.shared
            .resolvers
            .iter()
            .find_map(|r| r.external_identity(name, &self.shared.fs))
    }

    /// Every language's environment roots, deepest first.
    pub fn environment_roots(&self) -> &[EnvRoot] {
        &self.shared.env_roots
    }

    /// Directory cache counters (not deterministic across warm and cold runs).
    pub fn fs_stats(&self) -> DirCacheStats {
        self.shared.fs.stats()
    }
}

/// Sort key for a target: variant rank, then path bytes or name.
fn target_key(t: &Target, key_path: &impl Fn(&FileId) -> Vec<u8>) -> (u8, Vec<u8>) {
    match t {
        Target::File(id) => (0, key_path(id)),
        Target::Dir(id) => (1, key_path(id)),
        Target::Glob { pattern, .. } => (2, pattern.as_bytes().to_vec()),
        Target::Builtin(name) => (3, name.as_bytes().to_vec()),
        Target::External(name) => (4, name.as_bytes().to_vec()),
        Target::Unresolved => (5, Vec::new()),
    }
}
