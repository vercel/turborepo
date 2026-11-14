//! Output formats: `nft`, `json`, `ndjson`, `list` and
//! `summary`.
//!
//! All formats are deterministic: the same trace produces byte-identical output
//! regardless of thread count. Paths inside the trace base are written relative
//! to it; paths outside it are written absolute in `json`/`ndjson`, and are
//! omitted from `nft` and `list` and reported in
//! [`OutputReport::outside_base`]. Non-UTF-8 paths are written lossily
//! (U+FFFD).
//!
//! # Environment roots
//!
//! Files outside the base below a language's environment root
//! ([`crate::Resolver::environment_roots`]: Python site directories, the Go
//! module cache and `GOROOT`, the cargo registry and git checkouts) are
//! expected: a deployment provides them from its own environment (a venv, the
//! module cache), and builds read them from there. They are therefore not
//! `outside_base` records:
//!
//! - `nft` and `list` omit them, like every path outside the base, and report
//!   one [`OutputReport::environment_roots`] line per root with its file count
//!   (the CLI prints it to stderr) instead of listing each file;
//! - `json` writes them with their absolute path, `"outside_base": false`
//!   (omitted) and `"root": "<name>"`, and lists the roots used in a top-level
//!   `"roots"` object (`name → absolute path`, omitted when empty); `ndjson`
//!   does the same per line;
//! - `summary` prints one line per root under "files by reason".
//!
//! No [`crate::UncertaintyKind::OutsideBase`] record is produced for them.
//!
//! # Large traces
//!
//! `ndjson` streams one JSON object per line and never holds the whole document
//! in memory. Entries reference files by id (the index in file order), so a
//! closure costs a few bytes per file instead of its paths. `json` can do the
//! same with [`EntryFiles::Ids`]; by default it keeps schema version 1 (paths
//! per entry).

use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{FileId, Target, Trace, Uncertainty, lexkit::LineIndex};

/// Schema version of the `json` format.
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// An output format.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// `.nft.json`: `{"version": 1, "files": [...]}`, optionally with
    /// `symlinks`.
    Nft,
    /// The versioned JSON schema with reasons, edges, uncertainties and
    /// stats.
    Json,
    /// One path per line, then one `external:<name>` line per untraversed
    /// package (`--externals stop|list`).
    List,
    /// Human-readable report: counts, special-case hits and uncertainties
    /// grouped by kind, package and file, with source snippets.
    Summary,
    /// Streaming JSON lines (see [`write_ndjson`]).
    Ndjson,
}

/// How `json` output lists the files of each entry.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum EntryFiles {
    /// Paths, as in schema version 1 (default).
    #[default]
    Paths,
    /// Indexes into the top-level `files` array (schema version 2), so
    /// per-entry closures share the file table instead of repeating paths.
    Ids,
}

/// Which files to write.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    /// The union of all entry closures.
    #[default]
    All,
    /// The closure of the entry at this index.
    Entry(usize),
}

/// Options shared by the writers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputOptions {
    /// Which files to write (`nft` and `list`).
    pub selection: Selection,
    /// Add the Turbopack `symlinks` extension to `nft` output when symlinks are
    /// present.
    pub nft_symlinks: bool,
    /// How `json` lists entry files.
    pub entry_files: EntryFiles,
}

impl Default for OutputOptions {
    fn default() -> Self {
        OutputOptions {
            selection: Selection::All,
            nft_symlinks: true,
            entry_files: EntryFiles::Paths,
        }
    }
}

/// Information about a write that callers may want to surface as warnings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputReport {
    /// Included files outside the base that were omitted (`nft`, `list`),
    /// excluding files below environment roots.
    pub outside_base: Vec<PathBuf>,
    /// Environment roots whose files were omitted (`nft`, `list`): name,
    /// directory and number of omitted files.
    pub environment_roots: Vec<(String, PathBuf, usize)>,
}

impl OutputReport {
    /// Classifies an omitted path outside the base.
    fn omit(&mut self, trace: &Trace, p: &Path) {
        let root = trace.file_id(p).and_then(|id| trace.environment_root(id));
        match root {
            Some(r) => match self
                .environment_roots
                .iter_mut()
                .find(|(n, _, _)| **n == *r.name)
            {
                Some(e) => e.2 += 1,
                None => self
                    .environment_roots
                    .push((r.name.to_string(), r.path.clone(), 1)),
            },
            None => self.outside_base.push(p.to_path_buf()),
        }
    }
}

/// Writes `trace` in `format`.
pub fn write(
    trace: &Trace,
    format: Format,
    opts: &OutputOptions,
    w: &mut dyn Write,
) -> io::Result<OutputReport> {
    match format {
        Format::Nft => write_nft(trace, opts, w),
        Format::Json => write_json_with(trace, opts, w).map(|()| OutputReport::default()),
        Format::List => write_list(trace, opts, w),
        Format::Summary => write_summary(trace, w).map(|()| OutputReport::default()),
        Format::Ndjson => write_ndjson(trace, w).map(|()| OutputReport::default()),
    }
}

fn selected<'t>(trace: &'t Trace, sel: Selection) -> Box<dyn Iterator<Item = &'t Path> + 't> {
    match sel {
        Selection::All => Box::new(trace.files()),
        Selection::Entry(i) => Box::new(trace.entry_files(i)),
    }
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Path for display: relative to the base when inside it, absolute otherwise.
fn display_path(trace: &Trace, p: &Path) -> String {
    match trace.relative(p) {
        Some(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Some(rel) => lossy(rel),
        None => lossy(p),
    }
}

#[derive(Serialize)]
struct NftOut {
    version: u32,
    files: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    symlinks: BTreeMap<String, String>,
    // TODO: `fileHashes`, a parallel array to `files`, when hashing is enabled.
}

/// Writes `.nft.json` (`{"version":1,"files":[...]}`), plus `symlinks` (file
/// index to target path) when enabled and present. Entries are included in
/// `files`, as in nft's `fileList`.
pub fn write_nft(
    trace: &Trace,
    opts: &OutputOptions,
    w: &mut dyn Write,
) -> io::Result<OutputReport> {
    let mut report = OutputReport::default();
    let mut files = Vec::new();
    let mut position: HashMap<&Path, usize> = HashMap::new();
    for p in selected(trace, opts.selection) {
        match trace.relative(p) {
            Some(rel) => {
                position.insert(p, files.len());
                files.push(lossy(rel));
            }
            None => report.omit(trace, p),
        }
    }
    let mut symlinks = BTreeMap::new();
    if opts.nft_symlinks {
        for (link, target) in trace.symlinks() {
            if let (Some(&i), Some(t)) = (position.get(link), trace.relative(target)) {
                symlinks.insert(i.to_string(), lossy(t));
            }
        }
    }
    let out = NftOut {
        version: 1,
        files,
        symlinks,
    };
    serde_json::to_writer(&mut *w, &out)?;
    writeln!(w)?;
    Ok(report)
}

/// Writes one path per line, relative to the base. Untraversed third-party
/// packages (`--externals stop|list`) follow as `external:<name>` lines, so the
/// list stays complete about what the entries need; there are none with the
/// default `--externals follow`.
pub fn write_list(
    trace: &Trace,
    opts: &OutputOptions,
    w: &mut dyn Write,
) -> io::Result<OutputReport> {
    use crate::os::OsStrExt;
    let mut report = OutputReport::default();
    for p in selected(trace, opts.selection) {
        match trace.relative(p) {
            Some(rel) => {
                w.write_all(rel.as_os_str().as_bytes())?;
                w.write_all(b"\n")?;
            }
            None => report.omit(trace, p),
        }
    }
    for name in trace.externals() {
        writeln!(w, "external:{name}")?;
    }
    Ok(report)
}

#[derive(Serialize)]
struct JsonOut<'t> {
    version: u32,
    base: String,
    entries: JsonEntries<'t>,
    files: Vec<JsonFile>,
    edges: Vec<JsonEdge>,
    uncertainties: Vec<JsonUncertainty>,
    symlinks: BTreeMap<String, String>,
    /// Untraversed third-party packages (`--externals stop|list`); omitted when
    /// empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    externals: Vec<String>,
    /// Environment roots used (name to absolute path); omitted when empty.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    roots: BTreeMap<String, String>,
    stats: JsonStats,
}

/// The `entries` array, serialized lazily so closures are never materialized as
/// strings.
struct JsonEntries<'t> {
    trace: &'t Trace,
    ids: bool,
}

impl Serialize for JsonEntries<'_> {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = ser.serialize_seq(Some(self.trace.entries().len()))?;
        for (i, e) in self.trace.entries().enumerate() {
            seq.serialize_element(&JsonEntry {
                trace: self.trace,
                index: i,
                path: display_path(self.trace, e),
                ids: self.ids,
            })?;
        }
        seq.end()
    }
}

struct JsonEntry<'t> {
    trace: &'t Trace,
    index: usize,
    path: String,
    ids: bool,
}

impl Serialize for JsonEntry<'_> {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = ser.serialize_struct("JsonEntry", 2)?;
        st.serialize_field("path", &self.path)?;
        if self.ids {
            st.serialize_field("files", &EntryIds(self.trace, self.index))?;
        } else {
            st.serialize_field("files", &EntryPaths(self.trace, self.index))?;
        }
        st.end()
    }
}

struct EntryPaths<'t>(&'t Trace, usize);

impl Serialize for EntryPaths<'_> {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_seq(self.0.entry_files(self.1).map(|p| display_path(self.0, p)))
    }
}

struct EntryIds<'t>(&'t Trace, usize);

impl Serialize for EntryIds<'_> {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_seq(self.0.entry_file_indices(self.1))
    }
}

#[derive(Serialize)]
struct JsonFile {
    path: String,
    reasons: Vec<&'static str>,
    parents: Vec<String>,
    parsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<&'static str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    outside_base: bool,
    /// The environment root of a file outside the base below one.
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
}

#[derive(Serialize)]
struct JsonEdge {
    from: String,
    kind: &'static str,
    phase: &'static str,
    flags: Vec<&'static str>,
    spec: String,
    span: [u32; 2],
    line: u32,
    target: JsonTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    uncertainty: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsonTarget {
    File {
        path: String,
    },
    Dir {
        path: String,
    },
    Glob {
        pattern: String,
        matches: Vec<String>,
        truncated: bool,
    },
    Builtin {
        name: String,
    },
    External {
        name: String,
    },
    Unresolved,
}

#[derive(Serialize)]
struct JsonUncertainty {
    file: String,
    kind: &'static str,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    spec: Option<String>,
    span: [u32; 2],
    line: u32,
    col: u32,
}

#[derive(Serialize)]
struct JsonStats {
    entries: usize,
    files: usize,
    parsed_files: usize,
    edges: usize,
    uncertainties: usize,
}

/// Lazily built line indexes, for converting spans to line/column in reports.
#[derive(Default)]
struct Lines {
    cache: HashMap<FileId, Option<LineIndex>>,
}

impl Lines {
    fn line_col(&mut self, trace: &Trace, file: FileId, offset: u32) -> (u32, u32) {
        let idx = self.cache.entry(file).or_insert_with(|| {
            let path = trace.path(file)?;
            std::fs::read(path).ok().map(|src| LineIndex::new(&src))
        });
        match idx {
            Some(i) => {
                let lc = i.line_col(offset);
                (lc.line, lc.col)
            }
            None => (0, 0),
        }
    }
}

/// Writes the versioned JSON schema (see [`JSON_SCHEMA_VERSION`]),
/// listing entry files by path. See [`write_json_with`].
pub fn write_json(trace: &Trace, w: &mut dyn Write) -> io::Result<()> {
    write_json_with(trace, &OutputOptions::default(), w)
}

fn json_file(trace: &Trace, id: FileId, parents: &mut dyn FnMut(FileId) -> String) -> JsonFile {
    let reason = trace.reason(id).unwrap_or_default();
    let path = trace.path(id).unwrap_or(Path::new(""));
    let root = trace.environment_root(id);
    JsonFile {
        path: display_path(trace, path),
        reasons: reason.kinds.names().collect(),
        parents: reason.parents.iter().map(|&p| parents(p)).collect(),
        parsed: trace.is_parsed(id),
        lang: trace.language(id).map(|l| l.0),
        outside_base: trace.relative(path).is_none() && root.is_none(),
        root: root.map(|r| r.name.to_string()),
    }
}

fn json_target(trace: &Trace, t: &Target) -> JsonTarget {
    let pid = |id: FileId| {
        trace
            .path(id)
            .map(|p| display_path(trace, p))
            .unwrap_or_default()
    };
    match t {
        Target::File(id) => JsonTarget::File { path: pid(*id) },
        Target::Dir(id) => JsonTarget::Dir { path: pid(*id) },
        Target::Glob {
            pattern,
            matches,
            truncated,
        } => JsonTarget::Glob {
            pattern: display_path(trace, Path::new(&**pattern)),
            matches: matches.iter().map(|&m| pid(m)).collect(),
            truncated: *truncated,
        },
        Target::Builtin(name) => JsonTarget::Builtin {
            name: name.to_string(),
        },
        Target::External(name) => JsonTarget::External {
            name: name.to_string(),
        },
        Target::Unresolved => JsonTarget::Unresolved,
    }
}

fn json_edge(trace: &Trace, e: &crate::Edge, lines: &mut Lines) -> JsonEdge {
    JsonEdge {
        from: trace
            .path(e.from)
            .map(|p| display_path(trace, p))
            .unwrap_or_default(),
        kind: e.kind.as_str(),
        phase: e.phase.as_str(),
        flags: e.flags.names().collect(),
        spec: e.spec.to_string(),
        span: [e.span.start, e.span.end],
        line: lines.line_col(trace, e.from, e.span.start).0,
        target: json_target(trace, &e.to),
        uncertainty: e.uncertainty.as_ref().map(|u| u.kind.as_str()),
    }
}

fn json_uncertainty(
    trace: &Trace,
    file: FileId,
    u: &Uncertainty,
    edge: Option<&crate::Edge>,
    lines: &mut Lines,
) -> JsonUncertainty {
    let (line, col) = lines.line_col(trace, file, u.span.start);
    JsonUncertainty {
        file: trace
            .path(file)
            .map(|p| display_path(trace, p))
            .unwrap_or_default(),
        kind: u.kind.as_str(),
        detail: u.detail.to_string(),
        spec: edge.map(|e| e.spec.to_string()),
        span: [u.span.start, u.span.end],
        line,
        col,
    }
}

fn json_roots(trace: &Trace) -> BTreeMap<String, String> {
    trace
        .environment_roots()
        .iter()
        .map(|(r, _)| (r.name.to_string(), lossy(&r.path)))
        .collect()
}

fn json_stats(trace: &Trace) -> JsonStats {
    let s = trace.stats();
    JsonStats {
        entries: s.entries,
        files: s.files,
        parsed_files: s.parsed_files,
        edges: s.edges,
        uncertainties: s.uncertainties,
    }
}

/// Writes the versioned JSON schema.
///
/// With [`EntryFiles::Paths`] (default) this is schema version 1
/// ([`JSON_SCHEMA_VERSION`]): each entry lists the paths of its closure. With
/// [`EntryFiles::Ids`] it is version 2: each entry lists indexes into the
/// top-level `files` array, which keeps the document small when many entries
/// share most of their files. Entries are serialized lazily either way.
///
/// Line and column numbers are computed by re-reading source files; they are 0
/// when a file cannot be read.
pub fn write_json_with(trace: &Trace, opts: &OutputOptions, w: &mut dyn Write) -> io::Result<()> {
    let dp = |p: &Path| display_path(trace, p);
    let mut pid = |id: FileId| trace.path(id).map(dp).unwrap_or_default();
    let mut lines = Lines::default();
    let ids = opts.entry_files == EntryFiles::Ids;
    let files = trace
        .file_ids()
        .map(|id| json_file(trace, id, &mut pid))
        .collect();
    let edges = trace
        .edges()
        .iter()
        .map(|e| json_edge(trace, e, &mut lines))
        .collect();
    let uncertainties = trace
        .uncertainties()
        .into_iter()
        .map(|(file, u, edge)| json_uncertainty(trace, file, u, edge, &mut lines))
        .collect();
    let symlinks = trace.symlinks().map(|(l, t)| (dp(l), dp(t))).collect();
    let out = JsonOut {
        version: if ids { 2 } else { JSON_SCHEMA_VERSION },
        base: lossy(trace.base()),
        entries: JsonEntries { trace, ids },
        files,
        edges,
        uncertainties,
        symlinks,
        externals: trace.externals().into_iter().map(String::from).collect(),
        roots: json_roots(trace),
        stats: json_stats(trace),
    };
    let mut buf = io::BufWriter::with_capacity(1 << 16, w);
    serde_json::to_writer_pretty(&mut buf, &out)?;
    writeln!(buf)?;
    buf.flush()
}

/// Schema version of the `ndjson` format.
pub const NDJSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum NdLine<'a> {
    Header {
        version: u32,
        base: String,
        #[serde(skip_serializing_if = "BTreeMap::is_empty")]
        roots: BTreeMap<String, String>,
    },
    File {
        id: usize,
        #[serde(flatten)]
        file: JsonFile,
    },
    Edge {
        #[serde(flatten)]
        edge: JsonEdge,
    },
    Uncertainty {
        #[serde(flatten)]
        uncertainty: JsonUncertainty,
    },
    Symlink {
        link: String,
        target: String,
    },
    External {
        name: &'a str,
    },
    Entry {
        path: String,
        /// Indexes of the `file` lines (their `id`).
        files: EntryIds<'a>,
    },
    Stats {
        #[serde(flatten)]
        stats: JsonStats,
    },
}

/// Writes the trace as JSON lines, streaming (schema
/// [`NDJSON_SCHEMA_VERSION`]).
///
/// One object per line, each with a `type`: a `header` (version, base,
/// environment roots), then every `file` (with `id`, its index in path order,
/// and the same fields as in `json`), every `edge`, every `uncertainty`, every
/// `symlink`, every `external`, then one `entry` per entry whose `files` are
/// file ids, and finally `stats`. Field meanings match `json`. Nothing is
/// buffered beyond one line, so the output size is not bounded by memory.
pub fn write_ndjson(trace: &Trace, w: &mut dyn Write) -> io::Result<()> {
    let mut w = io::BufWriter::with_capacity(1 << 16, w);
    let line = |w: &mut io::BufWriter<&mut dyn Write>, l: &NdLine<'_>| -> io::Result<()> {
        serde_json::to_writer(&mut *w, l)?;
        w.write_all(b"\n")
    };
    line(
        &mut w,
        &NdLine::Header {
            version: NDJSON_SCHEMA_VERSION,
            base: lossy(trace.base()),
            roots: json_roots(trace),
        },
    )?;
    let dp = |p: &Path| display_path(trace, p);
    let mut pid = |id: FileId| trace.path(id).map(dp).unwrap_or_default();
    for (i, id) in trace.file_ids().enumerate() {
        let file = json_file(trace, id, &mut pid);
        line(&mut w, &NdLine::File { id: i, file })?;
    }
    let mut lines = Lines::default();
    for e in trace.edges() {
        let edge = json_edge(trace, e, &mut lines);
        line(&mut w, &NdLine::Edge { edge })?;
    }
    for (file, u, edge) in trace.uncertainties() {
        let uncertainty = json_uncertainty(trace, file, u, edge, &mut lines);
        line(&mut w, &NdLine::Uncertainty { uncertainty })?;
    }
    for (l, t) in trace.symlinks() {
        line(
            &mut w,
            &NdLine::Symlink {
                link: dp(l),
                target: dp(t),
            },
        )?;
    }
    for name in trace.externals() {
        line(&mut w, &NdLine::External { name })?;
    }
    for (i, e) in trace.entries().enumerate() {
        line(
            &mut w,
            &NdLine::Entry {
                path: dp(e),
                files: EntryIds(trace, i),
            },
        )?;
    }
    line(
        &mut w,
        &NdLine::Stats {
            stats: json_stats(trace),
        },
    )?;
    w.flush()
}

/// Spec prefix that marks edges added by package special cases; `summary` lists
/// them.
pub const SPECIAL_CASE_PREFIX: &str = "special-case ";

/// How the specifier of an edge starts that stands for one reading of a path
/// which leaves the repository by the way it is written (an absolute path, a
/// climb above the root): the repository seen from another place
/// ([`crate::fs::seen_from_elsewhere`]). The rest is the reading in one
/// sentence ([`crate::fs::Elsewhere::sentence`]), which `--explain` prints.
pub const ELSEWHERE_PREFIX: &str = "elsewhere ";

/// How the detail of the record ends that a read below a temporary directory
/// keeps (a path the code writes out as `/tmp/…`: [`crate::fs::TEMP_DIRS`]).
/// What is there is made at run time, under a name of that run: the path names
/// no file of the repository, also when a build step copies one there. That is
/// an assumption, so the read keeps a `generated_file` record: ignored by
/// default, open under `--strict` and with `TURBO_TRACE_DEBUG_TEMP_READS=0`.
/// The query crate finds the record by this text
/// (`turborepo_trace_graph::output::TEMP_READ_NOTE`, the same).
pub const TEMP_READ_NOTE: &str = "is below a temporary directory: what is there is made at run \
                                  time, not a file of the repository";

/// How the detail of the record ends that a read keeps whose path has a
/// computed directory directly below an absolute directory the repository may
/// be mounted at (`` `/app/${dir}/x.json` ``, [`crate::fs::mounted_at`]). Read
/// as the repository mounted there it would be `*/x.json` from the root, a glob
/// over the whole repository: it is not read that way (the reading "mounted at
/// `/`", `app/*/x.json`, is). That is an assumption, so the read keeps a
/// `generated_file` record: ignored by default, open under `--strict` and with
/// `TURBO_TRACE_DEBUG_MOUNT_READS=0`. The query crate finds the record by
/// this text (`turborepo_trace_graph::output::MOUNT_READ_NOTE`, the same).
pub const MOUNT_READ_NOTE: &str = "is below a directory that is computed directly below a place \
                                   the repository may be mounted at: not read as a path of the \
                                   repository";

/// The package a path belongs to, from its last `node_modules/<name>` (or
/// `node_modules/@scope/<name>`) segment; `None` for project files.
pub fn package_of(path: &Path) -> Option<String> {
    use crate::os::OsStrExt;
    let b = path.as_os_str().as_bytes();
    let i = memchr_rfind(b, b"/node_modules/")? + b"/node_modules/".len();
    let rest = &b[i..];
    let mut parts = rest.split(|&c| c == b'/');
    let first = parts.next()?;
    let name = if first.starts_with(b"@") {
        let second = parts.next()?;
        [first, b"/", second].concat()
    } else {
        first.to_vec()
    };
    Some(String::from_utf8_lossy(&name).into_owned())
}

fn memchr_rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Sources and line indexes for snippets.
#[derive(Default)]
struct Sources {
    cache: HashMap<FileId, Option<(Vec<u8>, LineIndex)>>,
}

impl Sources {
    /// `(line, col, line text)` for `offset` in `file`.
    fn locate(&mut self, trace: &Trace, file: FileId, offset: u32) -> Option<(u32, u32, String)> {
        let entry = self.cache.entry(file).or_insert_with(|| {
            let src = std::fs::read(trace.path(file)?).ok()?;
            let idx = LineIndex::new(&src);
            Some((src, idx))
        });
        let (src, idx) = entry.as_ref()?;
        let lc = idx.line_col(offset);
        let start = idx.line_start(lc.line)? as usize;
        let end = idx
            .line_start(lc.line + 1)
            .map(|e| e as usize)
            .unwrap_or(src.len())
            .min(src.len());
        let text = String::from_utf8_lossy(&src[start.min(end)..end]);
        let text = text.trim_end_matches(['\n', '\r']).trim();
        const MAX: usize = 160;
        let text = if text.chars().count() > MAX {
            let cut: String = text.chars().take(MAX).collect();
            format!("{cut}…")
        } else {
            text.to_string()
        };
        Some((lc.line, lc.col, text))
    }
}

/// Writes the human-readable `summary` report: counts, files by reason,
/// special-case hits, quiet records (fs probes of missing files), externals,
/// and every uncertainty grouped by kind, then package (from `node_modules`
/// paths; `(project)` otherwise), then file, with `line:col`, detail and the
/// source line.
pub fn write_summary(trace: &Trace, w: &mut dyn Write) -> io::Result<()> {
    use crate::{ReasonKinds, Target};
    let dp = |p: &Path| display_path(trace, p);
    let pid = |id: FileId| trace.path(id).map(dp).unwrap_or_default();
    let mut src = Sources::default();
    let s = trace.stats();

    writeln!(w, "turbo trace summary")?;
    writeln!(w, "base: {}", trace.base().display())?;
    writeln!(
        w,
        "entries: {}, files: {} (parsed {}), edges: {}, uncertainties: {}",
        s.entries, s.files, s.parsed_files, s.edges, s.uncertainties
    )?;

    // Files by reason (a file may have several).
    let reasons = [
        ReasonKinds::INITIAL,
        ReasonKinds::DEPENDENCY,
        ReasonKinds::ASSET,
        ReasonKinds::SHAREDLIB,
        ReasonKinds::MANIFEST,
        ReasonKinds::RESOLVE,
    ];
    let mut counts = [0usize; 6];
    let mut outside = 0usize;
    for id in trace.file_ids() {
        let r = trace.reason(id).unwrap_or_default();
        for (i, k) in reasons.iter().enumerate() {
            if r.kinds.contains(*k) {
                counts[i] += 1;
            }
        }
        if trace.path(id).is_some_and(|p| trace.relative(p).is_none())
            && trace.environment_root(id).is_none()
        {
            outside += 1;
        }
    }
    writeln!(w, "\nfiles by reason:")?;
    for (k, n) in reasons.iter().zip(counts) {
        if n > 0 {
            let name = k.names().next().unwrap_or_default();
            writeln!(w, "  {name:<11} {n}")?;
        }
    }
    if outside > 0 {
        writeln!(
            w,
            "  ({outside} outside the base; omitted from nft/list output)"
        )?;
    }
    for (r, n) in trace.environment_roots() {
        writeln!(
            w,
            "  ({n} in environment root {} at {}; omitted from nft/list output)",
            r.name,
            r.path.display()
        )?;
    }

    // Special-case hits: spec -> (files, first referencing file).
    let mut special: BTreeMap<&str, (std::collections::BTreeSet<FileId>, String)> = BTreeMap::new();
    let mut probes: Vec<(String, u32, u32, &str)> = Vec::new();
    for e in trace.edges() {
        if let Some(rest) = e.spec.strip_prefix(SPECIAL_CASE_PREFIX) {
            let entry = special
                .entry(rest)
                .or_insert_with(|| (Default::default(), pid(e.from)));
            match &e.to {
                Target::File(id) | Target::Dir(id) => {
                    entry.0.insert(*id);
                }
                Target::Glob { matches, .. } => entry.0.extend(matches.iter().copied()),
                _ => {}
            }
        } else if matches!(e.to, Target::Unresolved) && e.uncertainty.is_none() {
            let (line, col) = src
                .locate(trace, e.from, e.span.start)
                .map(|(l, c, _)| (l, c))
                .unwrap_or((0, 0));
            probes.push((pid(e.from), line, col, &e.spec));
        }
    }
    if !special.is_empty() {
        writeln!(w, "\nspecial cases ({}):", special.len())?;
        for (spec, (files, from)) in &special {
            writeln!(w, "  {spec}  [{} file(s), from {from}]", files.len())?;
        }
    }
    if !probes.is_empty() {
        writeln!(
            w,
            "\nreferences to missing or skipped paths, not uncertain ({}):",
            probes.len()
        )?;
        for (f, l, c, spec) in &probes {
            writeln!(w, "  {f}:{l}:{c}  {spec}")?;
        }
    }
    let externals = trace.externals();
    if !externals.is_empty() {
        writeln!(w, "\nexternals ({}):", externals.len())?;
        for e in externals {
            writeln!(w, "  {e}")?;
        }
    }

    // Uncertainties: kind -> package -> file -> records.
    type Rec = (u32, u32, String, String, Option<String>);
    let mut grouped: BTreeMap<&'static str, BTreeMap<String, BTreeMap<String, Vec<Rec>>>> =
        BTreeMap::new();
    let all = trace.uncertainties();
    for (file, u, edge) in &all {
        let path = trace.path(*file).unwrap_or(Path::new(""));
        let pkg = package_of(path).unwrap_or_else(|| "(project)".to_string());
        let loc = if u.span.end > u.span.start {
            src.locate(trace, *file, u.span.start)
        } else {
            None
        };
        let (line, col, snippet) = match loc {
            Some((l, c, t)) => (l, c, Some(t)),
            None => (0, 0, None),
        };
        // Flags such as `optional` (inside `try`) tell whether a failure matters.
        let spec = edge
            .map(|e| {
                let flags: Vec<&str> = e
                    .flags
                    .names()
                    .filter(|f| *f == "optional" || *f == "conditional")
                    .collect();
                if flags.is_empty() {
                    e.spec.to_string()
                } else {
                    format!("{} [{}]", e.spec, flags.join(", "))
                }
            })
            .unwrap_or_default();
        grouped
            .entry(u.kind.as_str())
            .or_default()
            .entry(pkg)
            .or_default()
            .entry(dp(path))
            .or_default()
            .push((line, col, spec, u.detail.to_string(), snippet));
    }
    if grouped.is_empty() {
        writeln!(w, "\nno uncertainties")?;
        return Ok(());
    }
    writeln!(w, "\nuncertainties ({}):", all.len())?;
    for (kind, pkgs) in &grouped {
        let n: usize = pkgs.values().flat_map(|f| f.values()).map(Vec::len).sum();
        writeln!(w, "\n{kind} ({n})")?;
        for (pkg, files) in pkgs {
            let n: usize = files.values().map(Vec::len).sum();
            writeln!(w, "  {pkg} ({n})")?;
            for (file, recs) in files {
                writeln!(w, "    {file}")?;
                for (line, col, spec, detail, snippet) in recs {
                    // Skip the prefix when the detail already starts with the specifier.
                    let spec = if spec.is_empty() || detail.starts_with(&format!("{spec}: ")) {
                        String::new()
                    } else {
                        format!("{spec}: ")
                    };
                    if *line > 0 {
                        writeln!(w, "      {line}:{col}  {spec}{detail}")?;
                    } else {
                        writeln!(w, "      {spec}{detail}")?;
                    }
                    if let Some(t) = snippet.as_deref().filter(|t| !t.is_empty()) {
                        writeln!(w, "        | {t}")?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_from_paths() {
        assert_eq!(package_of(Path::new("/a/src/x.js")), None);
        assert_eq!(
            package_of(Path::new("/a/node_modules/react/index.js")).as_deref(),
            Some("react")
        );
        assert_eq!(
            package_of(Path::new("/a/node_modules/@img/sharp-linux-x64/lib/x.node")).as_deref(),
            Some("@img/sharp-linux-x64")
        );
        assert_eq!(
            package_of(Path::new(
                "/a/node_modules/.pnpm/foo@1.0.0/node_modules/foo/index.js"
            ))
            .as_deref(),
            Some("foo")
        );
    }
}
