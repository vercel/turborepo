//! The compiled-in plugin model: [`Language`] and [`Resolver`] plus
//! the contexts passed to them.

use std::{
    borrow::Cow,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use smallvec::SmallVec;

use crate::{
    Base, FileId, RawRef, RefKind, Target, TraceOptions, Uncertainty, fs::DirCache,
    intern::PathInterner,
};

/// A stable language identifier such as `"js"` or `"python"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LangId(pub &'static str);

impl fmt::Display for LangId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A compiled-in language plugin.
///
/// Extraction must be a pure function of `(src, cx)`: it must not touch the
/// filesystem, so it can run in parallel, be fuzzed and be cached. It must
/// never panic on arbitrary bytes.
pub trait Language: Send + Sync + 'static {
    /// Stable identifier.
    fn id(&self) -> LangId;

    /// File extensions handled, without the leading dot. Multi-part extensions
    /// such as `"d.ts"` take precedence over shorter ones.
    fn extensions(&self) -> &'static [&'static str];

    /// Detects files without a known extension from their first bytes, such as
    /// a shebang. Only consulted for extensionless files that are followed
    /// as code.
    fn sniff(&self, _path: &Path, _head: &[u8]) -> bool {
        false
    }

    /// Whether a file with one of this language's extensions is not written in
    /// it after all, judged by its first bytes: a `.sh` file whose `#!`
    /// line names another interpreter is run by that interpreter. Such a
    /// file is detected like one without a known extension
    /// ([`Language::sniff`]).
    fn disowns(&self, _path: &Path, _head: &[u8]) -> bool {
        false
    }

    /// Finds references in `src` and pushes them to `out`.
    fn extract<'s>(&self, src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>);

    /// Creates the resolver for one tracer. It runs on all worker threads and
    /// shares the core fs cache through [`ResolveCx`].
    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver>;

    /// A stable description of the language's own options that affect results
    /// (build tags, cargo features, extra search roots), for cache keys
    /// such as the graph store's options digest. Default: empty (no
    /// options).
    fn config_key(&self) -> String {
        String::new()
    }
}

/// Resolves references produced by one language.
pub trait Resolver: Send + Sync {
    /// Resolves `r`, found in `from`, to one or more targets plus the manifests
    /// consulted.
    ///
    /// Every call must yield at least one target, or an uncertainty explaining
    /// why not.
    fn resolve(&self, r: &RawRef<'_>, from: FileId, cx: &ResolveCx<'_>) -> Resolution;

    /// Edges that belong to a parsed file as a whole rather than to one of its
    /// references, such as the file's own nearest `package.json` (needed
    /// even by an entry without references, as in nft) or files a package
    /// special case attaches to a known file of that package. Called once
    /// per parsed file, after its references are resolved.
    /// [`RefKind::Manifest`] edges are deduplicated against the manifests of
    /// the references.
    ///
    /// `cx.from_path` is the file itself. Default: none.
    fn file_edges(&self, _file: FileId, _cx: &ResolveCx<'_>) -> Vec<ExtraEdge> {
        Vec::new()
    }

    /// Drops resolver-internal caches (parsed manifests, resolved specifiers).
    /// Called by [`crate::Tracer::clear_cache`] together with the shared fs
    /// cache. Default: nothing.
    fn clear_cache(&self) {}

    /// Directories outside the trace base that this language legitimately reads
    /// from: a Python environment's site directories, the Go module cache
    /// and `GOROOT`, the cargo registry and git checkouts. Files below them
    /// are reported per root instead of with
    /// one [`crate::UncertaintyKind::OutsideBase`] record per file (see
    /// [`crate::Trace::environment_roots`]). Called once, when the tracer is
    /// built. Default: none.
    fn environment_roots(&self) -> Vec<EnvRoot> {
        Vec::new()
    }

    /// The third-party package that `path` (absolute, real) belongs to, if this
    /// language knows it: a Python distribution, a Go module version, a
    /// crate from the registry or a git checkout. Used by the graph store
    /// to key opaque packages per ecosystem. Must be cheap for paths the
    /// language does not own. Default: `None`.
    fn package_identity(&self, _path: &Path, _fs: &DirCache) -> Option<PackageIdentity> {
        None
    }

    /// The third-party package a [`Target::External`] name of this language
    /// stands for, when the package's files are not installed: the identity
    /// its files would have (a Go `module@version` that `go.mod` requires
    /// but the module cache lacks). The graph store links such a target to
    /// the package's opaque node, so the graph is the same with and without
    /// the language's package cache. Default: `None` (the name is display
    /// only).
    fn external_identity(&self, _name: &str, _fs: &DirCache) -> Option<PackageIdentity> {
        None
    }
}

/// A named directory outside the trace base that a language reads from by
/// design (see [`Resolver::environment_roots`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EnvRoot {
    /// Stable name, `<lang>:<what>`: `python:site`, `go:modcache`, `go:goroot`,
    /// `cargo:registry`, `cargo:git`.
    pub name: Box<str>,
    /// Absolute, normalized directory.
    pub path: std::path::PathBuf,
}

impl EnvRoot {
    /// A root named `name` at `path`.
    pub fn new(name: impl Into<Box<str>>, path: impl Into<std::path::PathBuf>) -> Self {
        EnvRoot {
            name: name.into(),
            path: path.into(),
        }
    }
}

/// The identity of a third-party package, from the language's own metadata (see
/// [`Resolver::package_identity`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageIdentity {
    /// Ecosystem: `npm`, `pypi`, `go`, `crates`.
    pub ecosystem: &'static str,
    /// Package name in that ecosystem (canonical distribution name, module
    /// path, crate name).
    pub name: String,
    /// Version.
    pub version: String,
    /// Content hash or source pin, if known (`go.sum` `h1:`, `Cargo.lock`
    /// checksum or git source, a digest of the dist-info `RECORD`).
    pub integrity: Option<String>,
    /// The package's root directory (absolute, real).
    pub root: std::path::PathBuf,
    /// Files that pin this version and whose change can make the language pick
    /// another version without the package's own files changing
    /// (`Cargo.lock`). The graph store treats them as config inputs of
    /// every shard that links the package.
    pub pins: Vec<std::path::PathBuf>,
}

/// Read-only context for [`Language::extract`].
#[derive(Copy, Clone, Debug)]
pub struct ExtractCx<'a> {
    /// The file being extracted.
    pub file: FileId,
    /// Its absolute path.
    pub path: &'a Path,
    /// Trace options.
    pub options: &'a TraceOptions,
}

/// Collects the output of [`Language::extract`].
#[derive(Debug, Default)]
pub struct RefSink<'s> {
    refs: Vec<RawRef<'s>>,
    diagnostics: Vec<Uncertainty>,
    facts: Vec<String>,
}

impl<'s> RefSink<'s> {
    /// Creates an empty sink.
    pub fn new() -> Self {
        RefSink {
            refs: Vec::new(),
            diagnostics: Vec::new(),
            facts: Vec::new(),
        }
    }

    /// Records a fact about the file that other files' analyses need (see
    /// [`crate::env`]): a tagged string, a pure function of the file's text.
    /// Facts are cached with the extraction and ignored by the tracer.
    pub fn fact(&mut self, fact: String) {
        self.facts.push(fact);
    }

    /// Takes the facts recorded so far.
    pub fn take_facts(&mut self) -> Vec<String> {
        std::mem::take(&mut self.facts)
    }

    /// Records a reference.
    #[inline]
    pub fn push(&mut self, r: RawRef<'s>) {
        self.refs.push(r);
    }

    /// Records a file-level uncertainty not tied to a reference, such as lexer
    /// recovery ([`crate::UncertaintyKind::ParseIncomplete`]).
    pub fn diagnostic(&mut self, u: Uncertainty) {
        self.diagnostics.push(u);
    }

    /// References recorded so far.
    pub fn refs(&self) -> &[RawRef<'s>] {
        &self.refs
    }

    /// Diagnostics recorded so far.
    pub fn diagnostics(&self) -> &[Uncertainty] {
        &self.diagnostics
    }

    /// Splits into references and diagnostics.
    pub fn into_parts(self) -> (Vec<RawRef<'s>>, Vec<Uncertainty>) {
        (self.refs, self.diagnostics)
    }
}

/// Context for [`Language::resolver`]: tracer-wide configuration.
#[derive(Copy, Clone, Debug)]
pub struct ProjectCx<'a> {
    /// Trace options (`base` is the project root).
    pub options: &'a TraceOptions,
    /// The shared fs cache, for reading project configuration up front. It is
    /// reference counted so a resolver can keep a handle, for example to
    /// plug it into a third-party resolver's filesystem abstraction
    /// (`oxc_resolver::FileSystem` in `lang-js`).
    pub fs: &'a Arc<DirCache>,
}

/// How the name of a source starts that is no file of the tree: text the tool
/// wrote itself to ask a resolver something (`__turbotrace_framework__.js` with
/// a `require` of a test runner's request, `__turbotrace_task__.js`,
/// `__turbotrace_link__.js`), analyzed as if it were a file in a directory.
pub const SYNTHETIC_PREFIX: &str = "__turbotrace_";

/// Context for [`Resolver::resolve`].
#[derive(Copy, Clone, Debug)]
pub struct ResolveCx<'a> {
    /// Absolute path of the referencing file.
    pub from_path: &'a Path,
    /// Its directory.
    pub from_dir: &'a Path,
    /// Trace options.
    pub options: &'a TraceOptions,
    /// The shared fs cache.
    pub fs: &'a DirCache,
    /// The shared path interner.
    pub interner: &'a PathInterner,
}

impl ResolveCx<'_> {
    /// Interns an absolute, normalized path.
    pub fn intern(&self, path: &Path) -> FileId {
        self.interner.intern(path)
    }

    /// Whether `path` (absolute, normalized) is a *host path*: the trace is
    /// confined to its base ([`TraceOptions::confined`],
    /// [`DirCache::is_host_path`]), the referencing file lies inside the
    /// base, and `path` lies outside it and outside the languages'
    /// environment roots.
    ///
    /// A host path depends on no file of the tree, so a resolver must not ask
    /// the file system about it: what exists in `/tmp`, `/proc` or a home
    /// directory is a property of the machine, not of the repository. A
    /// file outside the base (third-party code in a package store or a
    /// language's module cache) may still name its own neighbors.
    pub fn is_host_path(&self, path: &Path) -> bool {
        self.fs.is_host_path(path) && self.from_path.starts_with(&self.options.base)
    }

    /// Whether a path that the source of the referencing file writes as `spec`,
    /// relative to `dir`, is a host path by the way it is written, wherever
    /// it lands: the trace is confined, the referencing file lies inside
    /// the base, and `spec` is an absolute path or climbs above the base on
    /// its way (`../../other/x` from a file one level deep).
    ///
    /// Where such a path lands depends on the place of the checkout: an
    /// absolute path is inside the repository only on a machine that keeps
    /// the checkout there, and a climb above the base comes back in only
    /// when the directory is called what the path says (`../repo/data.json`
    /// from the root of a checkout called `repo`). A graph is a function of
    /// the tree, so the path is never looked up where it lands
    /// ([`ResolveCx::is_host_path`] is the test for the path itself). What it
    /// may name in the repository, at whatever place the code runs, is
    /// [`ResolveCx::seen_from_elsewhere`].
    ///
    /// Only for what a file of the tree writes. A request the tool makes itself
    /// and hands to a resolver as the source of a file that does not exist
    /// ([`SYNTHETIC_PREFIX`]: a test runner's mapper target after
    /// `<rootDir>` is substituted, a task's entry) is an absolute path
    /// because the tool made it one: it is judged by where it lands.
    pub fn names_host_path(&self, dir: &Path, spec: &str) -> bool {
        if !self.fs.confined() || !self.from_path.starts_with(&self.options.base) {
            return false;
        }
        if self.from_path.file_name().is_some_and(|n| {
            n.as_encoded_bytes()
                .starts_with(SYNTHETIC_PREFIX.as_bytes())
        }) {
            return false;
        }
        if spec.starts_with('/') {
            return true;
        }
        let Ok(rel) = dir.strip_prefix(&self.options.base) else {
            return false;
        };
        let mut depth = rel.components().count();
        for c in spec.split('/') {
            match c {
                "" | "." => {}
                ".." if depth == 0 => return true,
                ".." => depth -= 1,
                _ => depth += 1,
            }
        }
        false
    }

    /// The files and directories of the repository that a path may name which
    /// the source of the referencing file writes as `spec`, relative to
    /// `dir`, when it is a host path by the way it is written
    /// ([`ResolveCx::names_host_path`]): the repository seen from
    /// another place ([`crate::fs::seen_from_elsewhere`]), as absolute paths
    /// inside the base with the condition under which the written path
    /// names them. Empty for every other path, and for one that names no
    /// place a checkout is kept at.
    ///
    /// The same list wherever the checkout is: it is made from what the source
    /// writes.
    pub fn seen_from_elsewhere(
        &self,
        dir: &Path,
        spec: &str,
    ) -> Vec<(PathBuf, crate::fs::Elsewhere)> {
        if !self.names_host_path(dir, spec) {
            return Vec::new();
        }
        let depth = dir
            .strip_prefix(&self.options.base)
            .ok()
            .map(|rel| rel.components().count());
        crate::fs::seen_from_elsewhere(depth, spec)
            .into_iter()
            .map(|e| (self.options.base.join(&e.rel), e))
            .collect()
    }

    /// The directory `base` refers to, or `None` for [`Base::Language`]
    /// anchors, which the language resolver must interpret itself.
    pub fn base_dir(&self, base: &Base) -> Option<Cow<'_, Path>> {
        match base {
            Base::FileDir => Some(Cow::Borrowed(self.from_dir)),
            Base::Cwd => Some(Cow::Borrowed(&self.options.cwd)),
            Base::ProjectRoot => Some(Cow::Borrowed(&self.options.base)),
            Base::Dir(id) => self
                .interner
                .try_path(*id)
                .map(|p| Cow::Owned(p.to_path_buf())),
            Base::Language(_) => None,
        }
    }
}

/// The result of resolving one reference.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    /// Targets. Usually one; several for alternatives.
    pub targets: SmallVec<[Target; 1]>,
    /// Manifests consulted, emitted as [`crate::RefKind::Manifest`] edges.
    pub manifests: SmallVec<[FileId; 2]>,
    /// Why the result is uncertain, if it is. Overrides an uncertainty already
    /// on the reference.
    pub uncertainty: Option<Uncertainty>,
    /// Further edges of their own kinds produced by the same reference, such as
    /// the native addon and shared libraries a `require('bindings')` call
    /// loads. They share the reference's span; their flags add to the
    /// reference's, and their phase defaults to the reference's (see
    /// [`ExtraEdge`]).
    pub extra: Vec<ExtraEdge>,
    /// The reference has no file edge, by design: for example Python's
    /// `from pkg import name` where `name` is an attribute, not a submodule. No
    /// edge (not even an unresolved one) is emitted for the targets, and no
    /// uncertainty is reported; manifests, extra edges and recorded
    /// observations are kept. Set by [`Resolution::none`].
    pub none: bool,
}

/// An edge produced next to a reference's own targets (see
/// [`Resolution::extra`] and [`Resolver::file_edges`]), with its own kind and
/// therefore its own inclusion reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraEdge {
    /// Kind of the edge, which determines whether the target is followed and
    /// its reason.
    pub kind: RefKind,
    /// Where it points.
    pub target: Target,
    /// Display form for reports, such as `special-case bindings:
    /// build/Release/*.node`.
    pub spec: Box<str>,
    /// Why this edge is uncertain, if it is.
    pub uncertainty: Option<Uncertainty>,
    /// Qualifiers. For an edge of a reference they are added to the reference's
    /// flags; for a file-level edge they are the edge's flags. For example
    /// Go files that only build for other targets (`--all-targets`) are
    /// [`crate::RefFlags::CONDITIONAL`].
    pub flags: crate::RefFlags,
    /// Phase, overriding the default: the reference's phase for an edge of a
    /// reference, [`RefKind::default_phase`] for a file-level edge. For
    /// example `go.mod` and `Cargo.lock` are [`crate::Phase::Build`]
    /// inputs. Edges whose phase the trace's phase filter excludes are
    /// dropped.
    pub phase: Option<crate::Phase>,
}

impl ExtraEdge {
    /// An edge without uncertainty.
    pub fn new(kind: RefKind, target: Target, spec: impl Into<Box<str>>) -> Self {
        ExtraEdge {
            kind,
            target,
            spec: spec.into(),
            uncertainty: None,
            flags: crate::RefFlags::empty(),
            phase: None,
        }
    }

    /// Attaches an uncertainty record.
    pub fn with_uncertainty(mut self, u: Uncertainty) -> Self {
        self.uncertainty = Some(u);
        self
    }

    /// Adds flags.
    pub fn with_flags(mut self, flags: crate::RefFlags) -> Self {
        self.flags |= flags;
        self
    }

    /// Sets the phase.
    pub fn with_phase(mut self, phase: crate::Phase) -> Self {
        self.phase = Some(phase);
        self
    }
}

impl Resolution {
    /// A single target.
    pub fn target(t: Target) -> Self {
        Resolution {
            targets: smallvec::smallvec![t],
            ..Default::default()
        }
    }

    /// A single file.
    pub fn file(id: FileId) -> Self {
        Self::target(Target::File(id))
    }

    /// Resolution failed for the reason in `u`.
    pub fn unresolved(u: Uncertainty) -> Self {
        Resolution {
            targets: smallvec::smallvec![Target::Unresolved],
            manifests: SmallVec::new(),
            uncertainty: Some(u),
            extra: Vec::new(),
            none: false,
        }
    }

    /// The reference has no file edge, by design (see the `none` field): it is
    /// neither resolved nor uncertain. Whatever the resolution observed
    /// (probes of the submodule candidates) is still recorded in graph
    /// mode.
    pub fn none() -> Self {
        Resolution {
            none: true,
            ..Default::default()
        }
    }

    /// Adds an uncertainty.
    pub fn with_uncertainty(mut self, u: Uncertainty) -> Self {
        self.uncertainty = Some(u);
        self
    }

    /// Adds an extra edge.
    pub fn with_extra(mut self, e: ExtraEdge) -> Self {
        self.extra.push(e);
        self
    }

    /// Adds a consulted manifest.
    pub fn with_manifest(mut self, id: FileId) -> Self {
        self.manifests.push(id);
        self
    }
}

/// The set of languages known to a tracer, with extension lookup.
#[derive(Clone, Default)]
pub struct LanguageSet {
    langs: Vec<Arc<dyn Language>>,
}

impl fmt::Debug for LanguageSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.langs.iter().map(|l| l.id()))
            .finish()
    }
}

impl LanguageSet {
    /// Creates a set from languages; earlier languages win on extension
    /// conflicts.
    pub fn new(langs: Vec<Arc<dyn Language>>) -> Self {
        LanguageSet { langs }
    }

    /// All languages.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Language>> {
        self.langs.iter()
    }

    /// Number of languages.
    pub fn len(&self) -> usize {
        self.langs.len()
    }

    /// Whether there are no languages.
    pub fn is_empty(&self) -> bool {
        self.langs.is_empty()
    }

    /// The language at `index`.
    pub fn get(&self, index: usize) -> Option<&Arc<dyn Language>> {
        self.langs.get(index)
    }

    /// Finds the language for `path` by its longest matching extension (so
    /// `a.d.ts` prefers `"d.ts"` over `"ts"`). Returns the language index.
    pub fn by_extension(&self, path: &Path) -> Option<usize> {
        use crate::os::OsStrExt;
        let name = path.file_name()?.as_bytes();
        // Candidate suffixes after each '.', longest first. A leading dot (dotfile) is
        // not an extension separator.
        for (i, _) in name.iter().enumerate().skip(1).filter(|(_, b)| **b == b'.') {
            let ext = &name[i + 1..];
            if ext.is_empty() {
                continue;
            }
            for (li, l) in self.langs.iter().enumerate() {
                if l.extensions().iter().any(|e| e.as_bytes() == ext) {
                    return Some(li);
                }
            }
        }
        None
    }

    /// Asks each language to sniff `head`. Returns the first match.
    pub fn sniff(&self, path: &Path, head: &[u8]) -> Option<usize> {
        self.langs.iter().position(|l| l.sniff(path, head))
    }

    /// The language of the file at `path` whose first bytes are `head`: the one
    /// its extension names, unless that language disowns the file
    /// ([`Language::disowns`]) or no language has the extension; then the
    /// first one that sniffs it.
    pub fn detect(&self, path: &Path, head: &[u8]) -> Option<usize> {
        match self.by_extension(path) {
            Some(i) if !self.langs[i].disowns(path, head) => Some(i),
            _ => self.sniff(path, head),
        }
    }
}
