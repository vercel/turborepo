//! Graph mode: analyze files one by one, without traversal.
//!
//! [`Tracer::analyze`] extracts and resolves each given file independently and
//! returns its edges grouped per reference, each group with the filesystem
//! observations its resolution made ([`crate::record`]). Nothing is followed: a
//! whole-repo build passes every source file as an input, and the store decides
//! which targets are first-party files, leaves, or third-party packages.
//!
//! Extraction can be skipped for files whose contents did not change: pass the
//! previous [`CachedExtraction`] (keyed by content hash by the caller). Line
//! numbers are part of the extraction, so a cached file yields exactly the same
//! result as a fresh one.
//!
//! The tracer must be built with [`crate::TracerBuilder::record`] for
//! observations to be complete; without it the groups are still returned, with
//! empty observations.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use rayon::prelude::*;

use crate::{
    Edge, FileId, RawRef, Tracer, Uncertainty, UncertaintyKind,
    lang::{ExtractCx, LangId, RefSink},
    lexkit::LineIndex,
    sched::RefGroup,
    timing::Span,
    trace::Shared,
};

/// One file to analyze.
#[derive(Clone, Debug)]
pub struct AnalyzeInput {
    /// Absolute path (normalized; symlinked components are kept as given).
    pub path: PathBuf,
    /// A previous extraction of the same contents. When set, the file is not
    /// read.
    pub cached: Option<CachedExtraction>,
}

/// The pure, cacheable part of a file's analysis: its language, references and
/// diagnostics, with the 1-based line of each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedExtraction {
    /// Language id.
    pub lang: LangId,
    /// References in extraction order.
    pub refs: Vec<RawRef<'static>>,
    /// 1-based line of each reference (parallel to `refs`).
    pub ref_lines: Vec<u32>,
    /// File-level diagnostics.
    pub diagnostics: Vec<Uncertainty>,
    /// 1-based line of each diagnostic (parallel to `diagnostics`).
    pub diagnostic_lines: Vec<u32>,
    /// Facts for other files' analyses ([`crate::RefSink::fact`]).
    pub facts: Vec<String>,
}

/// One edge of a [`FileAnalysis`] with the concrete files it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnalyzedEdge {
    /// The edge. File targets are real paths (symlinks below the base
    /// resolved).
    pub edge: Edge,
    /// Concrete files reached: the file itself, glob matches, or files below a
    /// directory.
    pub succ: Vec<FileId>,
    /// Symlinks crossed to reach `succ`.
    pub links: Vec<FileId>,
}

/// The analysis of one file.
#[derive(Clone, Debug)]
pub struct FileAnalysis {
    /// The input path.
    pub path: PathBuf,
    /// The extraction (fresh or the cached one passed in), or `None` if no
    /// language handles the file or it could not be read.
    pub extraction: Option<CachedExtraction>,
    /// xxh3-128 of the contents, when the file was read (not for cached
    /// extractions).
    pub hash: Option<u128>,
    /// All edges, in reference order, then the file-level edges.
    pub edges: Vec<AnalyzedEdge>,
    /// Edge ranges and observations per reference, plus one file-level group
    /// last.
    pub groups: Vec<RefGroup>,
    /// A read error, if the file could not be read.
    pub read_error: Option<Uncertainty>,
    /// The first [`FILE_HEAD_BYTES`] bytes of the contents, when the file was
    /// read (with [`FileAnalysis::hash`]), so callers can check headers
    /// (generated-code markers) without opening the file again.
    pub head: Option<Box<[u8]>>,
}

/// How many leading bytes of a freshly read file [`FileAnalysis::head`] keeps.
pub const FILE_HEAD_BYTES: usize = 1024;

impl Tracer {
    /// Analyzes every input independently and in parallel (see the module
    /// docs). The output is in input order and does not depend on the
    /// thread count.
    pub fn analyze(&self, inputs: Vec<AnalyzeInput>) -> Vec<FileAnalysis> {
        let shared = &self.shared;
        shared.pool.install(|| {
            inputs
                .into_par_iter()
                .map(|i| shared.analyze_one(i))
                .collect()
        })
    }

    /// Analyzes `src` as the contents of a file at `path` (which need not
    /// exist), on the calling thread: extraction with the language for
    /// `path`'s extension, then resolution as for [`Tracer::analyze`]. Used
    /// to resolve synthetic references as if written in a file at that
    /// path, for example a test framework's setup-file request (`require(".
    /// /setup")`) resolved from its config directory.
    pub fn analyze_source(&self, path: &Path, src: &[u8]) -> FileAnalysis {
        let Some(cached) = self.shared.extract_owned(path, src) else {
            return FileAnalysis {
                path: path.to_path_buf(),
                extraction: None,
                hash: None,
                edges: Vec::new(),
                groups: Vec::new(),
                read_error: None,
                head: None,
            };
        };
        let mut a = self.shared.analyze_one(AnalyzeInput {
            path: path.to_path_buf(),
            cached: Some(cached),
        });
        a.hash = Some(xxhash_rust::xxh3::xxh3_128(src));
        a.head = Some(src[..src.len().min(FILE_HEAD_BYTES)].into());
        a
    }

    /// The path of an interned id (from [`AnalyzedEdge`] targets).
    pub fn path_of(&self, id: FileId) -> Arc<Path> {
        self.shared.interner.path(id)
    }
}

impl Shared {
    fn analyze_one(&self, input: AnalyzeInput) -> FileAnalysis {
        let path = input.path;
        let mut out = FileAnalysis {
            path: path.clone(),
            extraction: None,
            hash: None,
            edges: Vec::new(),
            groups: Vec::new(),
            read_error: None,
            head: None,
        };
        let extraction = match input.cached {
            Some(c) => Some(c),
            None => {
                static READ: Span = Span::new("trace: read file");
                static EXTRACT: Span = Span::new("trace: extract (lex, recognize, value flow)");
                let t = READ.enter();
                let src = std::fs::read(&path);
                drop(t);
                let src = match src {
                    Ok(s) => s,
                    Err(e) => {
                        out.read_error = Some(Uncertainty::new(
                            UncertaintyKind::ParseIncomplete,
                            crate::Span::DUMMY,
                            format!("cannot read file: {e}"),
                        ));
                        return out;
                    }
                };
                out.hash = Some(xxhash_rust::xxh3::xxh3_128(&src));
                out.head = Some(src[..src.len().min(FILE_HEAD_BYTES)].into());
                let _t = EXTRACT.enter();
                self.extract_owned(&path, &src)
            }
        };
        let Some(ex) = extraction else {
            return out;
        };
        let Some(li) = self.langs.iter().position(|l| l.id() == ex.lang) else {
            out.extraction = Some(ex);
            return out;
        };
        let file = self.interner.intern(&path);
        static RESOLVE: Span = Span::new("trace: resolve references");
        let t = RESOLVE.enter();
        let resolved = self.resolve_refs(file, &path, li, &ex.refs, true);
        drop(t);
        out.edges = resolved
            .edges
            .into_iter()
            .zip(resolved.succ)
            .zip(resolved.links)
            .map(|((edge, succ), links)| AnalyzedEdge { edge, succ, links })
            .collect();
        out.groups = resolved.groups;
        out.extraction = Some(ex);
        out
    }

    /// Extracts `src` with the language for `path` (extension, then sniffing)
    /// into owned references with line numbers.
    fn extract_owned(&self, path: &Path, src: &[u8]) -> Option<CachedExtraction> {
        let li = self.langs.detect(path, &src[..src.len().min(512)])?;
        let lang = self.langs.get(li)?;
        let file = self.interner.intern(path);
        let cx = ExtractCx {
            file,
            path,
            options: &self.options,
        };
        let mut sink = RefSink::new();
        lang.extract(src, &cx, &mut sink);
        let facts = sink.take_facts();
        let (refs, diagnostics) = sink.into_parts();
        let idx = LineIndex::new(src);
        let line = |off: u32| idx.line_col(off).line;
        let ref_lines = refs.iter().map(|r| line(r.span.start)).collect();
        let diagnostic_lines = diagnostics.iter().map(|d| line(d.span.start)).collect();
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
        Some(CachedExtraction {
            lang: lang.id(),
            refs,
            ref_lines,
            diagnostics,
            diagnostic_lines,
            facts,
        })
    }
}
