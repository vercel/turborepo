//! A graph as one file: `graph pack` writes it, `graph unpack` installs it into
//! an empty graph directory, and the next build refreshes it like any stored
//! graph.
//!
//! Nobody should pay for a cold build: CI packs the graph of a commit, and a
//! fresh checkout of that commit or of a nearby one unpacks it and re-traces
//! only the packages whose inputs differ. Which artifact is unpacked never
//! matters for correctness: a build recomputes every shard's key from the
//! working tree and reuses a shard only under an equal key
//! ([`mod@crate::build`], "Keys"), exactly as it does for a graph it built
//! itself.
//!
//! # What travels
//!
//! A graph directory holds four kinds of files:
//!
//! - **Primary**: the manifest `graph.json`, the shards (`store/pack`, its log,
//!   loose `store/<key>` files) and the walked-file list
//!   `store/files-<digest>`. They are a function of the tree, the options
//!   digest ([`crate::build::options_digest`], which covers the binary's
//!   [`crate::build::build_id`], the shard format and the target platform) and
//!   nothing else: not the checkout's location, not the machine. The artifact
//!   always holds these, with the pack rewritten to the manifest's shards in
//!   manifest order, so two builds of one tree give the same artifact bytes.
//! - **Derived from the manifest** (`cache/composed`, `cache/tables`):
//!   recomputed on the first open; optional in the artifact
//!   ([`PackOptions::caches`]), where they save the first query's composition
//!   when the working tree is the artifact's tree. The layers derived from the
//!   composed graph (`cache/derived-*`) are keyed the same way and stay behind:
//!   the first query computes them.
//! - **Checked against the graph they are used with** ([`CHECKED`]:
//!   `cache/xfile-trace`, the recorded run of cross-file binding). Such a file
//!   names files by their path in the repository and contents by their hash,
//!   never a stamp or a location, and whoever reads it compares what it
//!   recorded with the graph at hand before using any of it: on a graph where
//!   one recorded answer differs it is ignored. So it is valid wherever the
//!   artifact is, and for the tree after an edit as much as for the packed one.
//!   It travels when the directory has one (a query has run since the build),
//!   and saves the first query after a restore what it was recorded for.
//! - **Keyed by this machine's file stamps** (`cache/hashes`,
//!   `cache/manifests`, `cache/walk`, `cache/obs`, `cache/names-data`: device,
//!   inode, size and mtime). They never travel: on another checkout a stamp
//!   could name other bytes.
//!
//! `cache/<name>` is where a section's file lives in the graph directory's
//! cache directory, [`crate::cache_dir`]: by default `<graph dir>/cache`, and a
//! per-machine directory when the host sets one ([`crate::set_machine_cache`]),
//! as `turbo trace` does so that nothing keyed by stamps is in a directory
//! that travels with the repository.
//!
//! Everything else in `cache/` stays behind (`cache/xfile-facts` is keyed by
//! content and would be valid, but is large; it is extracted again when it is
//! needed).
//!
//! # File format
//!
//! | Bytes | Content |
//! |---|---|
//! | 0..8 | magic `DTGRAPH1` |
//! | 8..12 | header length `n`, little endian |
//! | 12..12+n | the header: JSON ([`Header`]) |
//! | then 8 | xxh3-64 of the header bytes |
//! | then | the sections' chunks in header order, each one zstd frame |
//!
//! Chunks are independent frames of at most [`PackOptions::chunk`] bytes of
//! content, so packing and unpacking use every core. Each carries two checksums
//! in the header: the xxh3-64 of its frame, the bytes as they are in the file,
//! and the xxh3-64 of its content.
//!
//! Every byte of the file is under a checksum: the header under its own, every
//! byte after it under the frame checksum of its chunk (the chunk table must
//! add up to the file's length). The content checksum alone does not do that: a
//! frame can be damaged and still decompress to the same content (zstd leaves a
//! bit of the frame header unused, and a match may point at another copy of the
//! same bytes). [`unpack`] checks every frame of every section first, also of
//! sections it then does not install.
//!
//! # Compatibility
//!
//! [`unpack`] fails closed: an artifact of another schema, shard format, build
//! identity or options digest, a truncated file or a checksum mismatch installs
//! nothing, and the next build starts from an empty store. No single damaged
//! byte gets past that. The manifest is written last, so an interrupted unpack
//! leaves no graph either.
//!
//! An artifact is trusted like the store itself: a shard's key names its
//! inputs, not its bytes, so restore artifacts only from where you would take a
//! build cache.
//!
//! # Another checkout location
//!
//! A shard is meant to say nothing about where its checkout lives: keys do not,
//! and almost all bytes do not. Some still do (a CI workflow's `${{
//! github.workspace }}` is stored expanded in a task's command lines, which the
//! environment registry reads). [`pack`] finds the shards whose bytes contain
//! the root's path and lists them in the header ([`Located`]) with a digest of
//! that path; [`unpack`] at another root leaves those shards out of the
//! manifest, so the refresh traces their packages again. A leak therefore costs
//! a re-trace, never a graph that differs from the one built in place.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    manifest::{Manifest, ManifestOptions},
    shard,
};

/// Magic bytes of an artifact file.
pub const MAGIC: &[u8; 8] = b"DTGRAPH1";
/// Header schema name.
pub const SCHEMA: &str = "turbotrace-graph-artifact";
/// Header schema version. 2: a chunk carries the checksum of its frame
/// ([`Chunk::zsum`]).
pub const VERSION: u32 = 2;
/// Default content bytes per chunk.
pub const DEFAULT_CHUNK: usize = 4 << 20;
/// Default zstd level.
pub const DEFAULT_LEVEL: i32 = 3;

const MANIFEST: &str = "graph.json";
const FILES: &str = "files";
const PACK: &str = "pack";
const COMPOSED: &str = "cache/composed";
const TABLES: &str = "cache/tables";
/// Files of the graph directory that their reader checks against the graph it
/// uses them with (the module docs, "What travels"): packed when present,
/// installed as they are.
///
/// `cache/xfile-trace` is written by the command line's cross-file binding: the
/// questions its last run asked of the graph (files by repository path, facts
/// by content hash) with digests of the answers, and the run's result, behind a
/// header that names the binary. The result is used only on a graph that gives
/// every recorded answer again.
pub const CHECKED: &[&str] = &["cache/xfile-trace"];
/// Largest header [`read_header`] accepts.
const MAX_HEADER: usize = 64 << 20;

/// The header of an artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Always [`SCHEMA`].
    pub schema: String,
    /// [`VERSION`].
    pub version: u32,
    /// The version of the crate that packed it.
    pub turbotrace: String,
    /// [`crate::build::build_id`] of the binary that built the graph.
    pub build_id: String,
    /// Shard format version.
    pub format: u32,
    /// The manifest's options digest.
    pub options_digest: String,
    /// The manifest's build options (what the digest was computed for).
    pub options: ManifestOptions,
    /// What the graph was built at (a commit or tree id). Informational: a
    /// build validates shards against the working tree, never against this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Shards in the pack.
    pub shards: usize,
    /// Small text files that belong to the graph directory but not to the store
    /// (the CLI's recorded language flags), by file name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
    /// The shards that name the directory the graph was built in, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub located: Option<Located>,
    /// The sections, in file order.
    pub sections: Vec<Section>,
}

/// Shards whose bytes contain the path of the repository root they were built
/// at: reusable only in a checkout at the same path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Located {
    /// xxh3-128 of the root's path, hex.
    pub root: String,
    /// Keys of the shards.
    pub shards: Vec<String>,
}

/// Where the file of a section lives for the graph directory `out`: a
/// `cache/<name>` section in the directory's [`crate::cache_dir`], every other
/// one in `out`.
fn file_of(out: &Path, name: &str) -> PathBuf {
    match name.strip_prefix("cache/") {
        Some(rest) => crate::cache_dir(out).join(rest),
        None => out.join(name),
    }
}

fn root_digest(root: &str) -> String {
    format!("{:032x}", xxhash_rust::xxh3::xxh3_128(root.as_bytes()))
}

/// One file of the graph directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// `graph.json`, `files`, `pack`, `cache/composed` or `cache/tables`.
    pub name: String,
    /// Content bytes.
    pub raw: u64,
    /// The chunks, in order.
    pub chunks: Vec<Chunk>,
}

/// One zstd frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// Content bytes.
    pub raw: u64,
    /// Compressed bytes.
    pub zst: u64,
    /// xxh3-64 of the content, hex.
    pub sum: String,
    /// xxh3-64 of the frame (the `zst` bytes in the file), hex.
    pub zsum: String,
}

impl Header {
    /// Content bytes of every section.
    pub fn raw_bytes(&self) -> u64 {
        self.sections.iter().map(|s| s.raw).sum()
    }

    /// Whether the derived caches are included.
    pub fn has_caches(&self) -> bool {
        self.sections.iter().any(|s| s.name == COMPOSED)
    }
}

/// Why an artifact was not written or not installed.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// I/O on a path.
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    /// Nothing to pack.
    #[error("no graph in {0}")]
    NotBuilt(PathBuf),
    /// The graph to pack is incomplete.
    #[error("{0}")]
    Store(String),
    /// Not an artifact file, or one this binary does not read.
    #[error("{0}: not a graph artifact of schema version {VERSION}")]
    NotAnArtifact(PathBuf),
    /// Built by another binary, for another shard format, or with options this
    /// host computes another digest for.
    #[error("incompatible graph artifact: {0}")]
    Incompatible(String),
    /// Truncated, or a checksum does not match.
    #[error("corrupt graph artifact: {0}")]
    Corrupt(String),
    /// The graph directory already holds a graph.
    #[error("{0} already holds a graph")]
    Exists(PathBuf),
}

impl ArtifactError {
    /// Whether the artifact itself was unusable (as opposed to an I/O failure
    /// while installing it): the caller goes on with a normal build.
    pub fn is_rejection(&self) -> bool {
        matches!(
            self,
            ArtifactError::NotAnArtifact(_)
                | ArtifactError::Incompatible(_)
                | ArtifactError::Corrupt(_)
                | ArtifactError::Exists(_)
        )
    }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ArtifactError + '_ {
    move |e| ArtifactError::Io(path.to_path_buf(), e)
}

/// Options of [`pack`].
#[derive(Clone, Debug)]
pub struct PackOptions {
    /// What the graph was built at ([`Header::label`]).
    pub label: Option<String>,
    /// Include the composed graph and its tables (computed first when missing
    /// or stale).
    pub caches: bool,
    /// zstd level.
    pub level: i32,
    /// Content bytes per chunk.
    pub chunk: usize,
    /// Small text files of the graph directory to carry along
    /// ([`Header::extra`]).
    pub extra: Vec<String>,
}

impl Default for PackOptions {
    fn default() -> Self {
        PackOptions {
            label: None,
            caches: false,
            level: DEFAULT_LEVEL,
            chunk: DEFAULT_CHUNK,
            extra: Vec::new(),
        }
    }
}

/// What [`pack`] wrote.
#[derive(Clone, Debug)]
pub struct PackReport {
    /// The header written.
    pub header: Header,
    /// Bytes of the artifact file.
    pub bytes: u64,
    /// Milliseconds: reading the store, compressing, writing.
    pub timings_ms: [u64; 3],
}

fn sum_hex(b: &[u8]) -> String {
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(b))
}

/// Packs the graph in `out` (a graph directory) into the file `dest`.
// Invariant: our own structs (string keys, derived impls) serialize to a Vec.
#[allow(clippy::expect_used)]
pub fn pack(out: &Path, dest: &Path, opts: &PackOptions) -> Result<PackReport, ArtifactError> {
    let t0 = Instant::now();
    crate::wait_for_cache_writes();
    let mut manifest = Manifest::read(out).ok_or_else(|| ArtifactError::NotBuilt(out.into()))?;
    let store = out.join("store");
    if opts.caches {
        // Composes (or reads back) the graph of this manifest and its tables, which
        // writes both cache files for exactly this manifest.
        let g = crate::Graph::from_manifest(out, manifest.clone())
            .map_err(|e| ArtifactError::Store(e.to_string()))?;
        g.warm();
        drop(g);
        crate::wait_for_cache_writes();
    }
    let packed = shard::Pack::open(&store);
    let shards: Vec<shard::LoadedShard> = manifest
        .shards
        .par_iter()
        .map(|e| shard::load_from(&store, packed.as_ref(), &e.key))
        .collect::<Result<_, _>>()
        .map_err(|e| ArtifactError::Store(e.to_string()))?;
    let list: Vec<(&str, &shard::LoadedShard)> = manifest
        .shards
        .iter()
        .map(|e| e.key.as_str())
        .zip(&shards)
        .collect();
    let pack_bytes = shard::Pack::bytes(&list);
    // Shards that name the directory of this checkout (see the module docs).
    let located = (manifest.root.len() > 1)
        .then(|| {
            let find = memchr::memmem::Finder::new(manifest.root.as_bytes());
            let mut keys: Vec<String> = list
                .par_iter()
                .filter(|(_, s)| find.find(s.archive()).is_some())
                .map(|(k, _)| (*k).to_owned())
                .collect();
            keys.sort_unstable();
            keys.dedup();
            keys
        })
        .filter(|keys| !keys.is_empty())
        .map(|shards| Located {
            root: root_digest(&manifest.root),
            shards,
        });
    drop(list);
    drop(shards);
    drop(packed);
    let files_bytes = match &manifest.files {
        Some(name) => crate::manifest::read_files_raw(&store, name).ok_or_else(|| {
            ArtifactError::Store(format!(
                "{}: missing or corrupt",
                store.join(name).display()
            ))
        })?,
        None => Vec::new(),
    };
    // The root is where this checkout happens to be: not part of the artifact.
    manifest.root = String::new();
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    manifest_bytes.push(b'\n');
    let mut contents: Vec<(&str, Vec<u8>)> = vec![(MANIFEST, manifest_bytes)];
    if manifest.files.is_some() {
        contents.push((FILES, files_bytes));
    }
    contents.push((PACK, pack_bytes));
    if opts.caches {
        for name in [COMPOSED, TABLES] {
            let p = file_of(out, name);
            contents.push((name, std::fs::read(&p).map_err(io(&p))?));
        }
    }
    // What is checked against the graph it is used with: when the directory has it.
    for name in CHECKED {
        if let Ok(bytes) = std::fs::read(file_of(out, name))
            && !bytes.is_empty()
        {
            contents.push((*name, bytes));
        }
    }
    let mut extra = BTreeMap::new();
    for name in &opts.extra {
        if let Ok(text) = std::fs::read_to_string(out.join(name)) {
            extra.insert(name.clone(), text);
        }
    }
    let t1 = Instant::now();

    // Compress every chunk of every section in parallel.
    let chunk = opts.chunk.max(64 << 10);
    let jobs: Vec<(usize, &[u8])> = contents
        .iter()
        .enumerate()
        .flat_map(|(si, (_, b))| b.chunks(chunk).map(move |c| (si, c)))
        .collect();
    let frames: Vec<(usize, Chunk, Vec<u8>)> = jobs
        .into_par_iter()
        .map(|(si, c)| {
            let z = zstd::bulk::compress(c, opts.level)
                .map_err(|e| ArtifactError::Store(format!("zstd: {e}")))?;
            Ok((
                si,
                Chunk {
                    raw: c.len() as u64,
                    zst: z.len() as u64,
                    sum: sum_hex(c),
                    zsum: sum_hex(&z),
                },
                z,
            ))
        })
        .collect::<Result<_, ArtifactError>>()?;
    let mut sections: Vec<Section> = contents
        .iter()
        .map(|(name, b)| Section {
            name: (*name).to_owned(),
            raw: b.len() as u64,
            chunks: Vec::new(),
        })
        .collect();
    for (si, c, _) in &frames {
        sections[*si].chunks.push(c.clone());
    }
    let header = Header {
        schema: SCHEMA.to_owned(),
        version: VERSION,
        turbotrace: env!("CARGO_PKG_VERSION").to_owned(),
        build_id: crate::build::build_id().to_owned(),
        format: shard::FORMAT_VERSION,
        options_digest: manifest.options_digest.clone(),
        options: manifest.options.clone(),
        label: opts.label.clone(),
        shards: manifest.shards.len(),
        extra,
        located,
        sections,
    };
    let head = serde_json::to_vec(&header).expect("header serializes");
    let t2 = Instant::now();

    let tmp = dest.with_extension(format!("tmp.{}", std::process::id()));
    if let Some(d) = dest.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(io(d))?;
    }
    let write = || -> std::io::Result<u64> {
        use std::io::Write;
        let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
        f.write_all(MAGIC)?;
        f.write_all(&(head.len() as u32).to_le_bytes())?;
        f.write_all(&head)?;
        f.write_all(&xxhash_rust::xxh3::xxh3_64(&head).to_le_bytes())?;
        let mut n = (MAGIC.len() + 4 + head.len() + 8) as u64;
        for (_, _, z) in &frames {
            f.write_all(z)?;
            n += z.len() as u64;
        }
        f.flush()?;
        Ok(n)
    };
    let bytes = match write().and_then(|n| std::fs::rename(&tmp, dest).map(|()| n)) {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(ArtifactError::Io(dest.to_path_buf(), e));
        }
    };
    Ok(PackReport {
        header,
        bytes,
        timings_ms: [
            (t1 - t0).as_millis() as u64,
            (t2 - t1).as_millis() as u64,
            t2.elapsed().as_millis() as u64,
        ],
    })
}

/// The header of the artifact in `bytes` and the offset of its first chunk.
// Invariant: a fixed-width range of the slice converts to an array of that width.
#[allow(clippy::expect_used)]
fn parse_header(bytes: &[u8], src: &Path) -> Result<(Header, usize), ArtifactError> {
    let not = || ArtifactError::NotAnArtifact(src.to_path_buf());
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err(not());
    }
    let n = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes")) as usize;
    if n > MAX_HEADER {
        return Err(not());
    }
    let (head, rest) = bytes[12..]
        .split_at_checked(n)
        .ok_or_else(|| ArtifactError::Corrupt("the header is cut off".into()))?;
    let sum = rest
        .first_chunk::<8>()
        .ok_or_else(|| ArtifactError::Corrupt("the header is cut off".into()))?;
    if xxhash_rust::xxh3::xxh3_64(head).to_le_bytes() != *sum {
        return Err(ArtifactError::Corrupt("header checksum mismatch".into()));
    }
    let header: Header = serde_json::from_slice(head).map_err(|_| not())?;
    if header.schema != SCHEMA || header.version != VERSION {
        return Err(not());
    }
    Ok((header, 12 + n + 8))
}

/// Reads the header of the artifact `src` (its checksum is checked, its chunks
/// are not).
// Invariant: a fixed-width range of the slice converts to an array of that width.
#[allow(clippy::expect_used)]
pub fn read_header(src: &Path) -> Result<Header, ArtifactError> {
    use std::io::Read;
    let mut f = std::fs::File::open(src).map_err(io(src))?;
    let mut fixed = [0u8; 12];
    f.read_exact(&mut fixed)
        .map_err(|_| ArtifactError::NotAnArtifact(src.to_path_buf()))?;
    let n = u32::from_le_bytes(fixed[8..12].try_into().expect("4 bytes")) as usize;
    if &fixed[..8] != MAGIC || n > MAX_HEADER {
        return Err(ArtifactError::NotAnArtifact(src.to_path_buf()));
    }
    let mut bytes = fixed.to_vec();
    bytes.resize(12 + n + 8, 0);
    f.read_exact(&mut bytes[12..])
        .map_err(|_| ArtifactError::Corrupt("the header is cut off".into()))?;
    Ok(parse_header(&bytes, src)?.0)
}

/// What [`unpack`] installed.
#[derive(Clone, Debug)]
pub struct UnpackReport {
    /// The artifact's header.
    pub header: Header,
    /// Shards left out of the manifest because they name another checkout's
    /// directory ([`Located`]): their packages are traced again.
    pub relocated: usize,
    /// Bytes of the artifact file.
    pub bytes: u64,
    /// Bytes written to the graph directory.
    pub written: u64,
    /// Milliseconds: opening and checking the header, the pack, everything
    /// else.
    pub timings_ms: [u64; 3],
}

/// Checks that this binary can reuse what `h` describes, apart from the options
/// digest (which needs the languages: the caller of [`unpack`] supplies it).
fn check_binary(h: &Header) -> Result<(), ArtifactError> {
    if h.format != shard::FORMAT_VERSION {
        return Err(ArtifactError::Incompatible(format!(
            "shard format {} (this binary reads {})",
            h.format,
            shard::FORMAT_VERSION
        )));
    }
    if h.build_id != crate::build::build_id() || h.turbotrace != env!("CARGO_PKG_VERSION") {
        return Err(ArtifactError::Incompatible(format!(
            "built by version {} (build {}), this is {} (build {})",
            h.turbotrace,
            h.build_id,
            env!("CARGO_PKG_VERSION"),
            crate::build::build_id()
        )));
    }
    Ok(())
}

/// One chunk to decompress: section, offset of its content in the section, the
/// frame, and its header record.
struct Job<'a> {
    section: usize,
    at: u64,
    frame: &'a [u8],
    chunk: &'a Chunk,
}

fn inflate(j: &Job<'_>, name: &str) -> Result<Vec<u8>, ArtifactError> {
    let bad = |what: &str| ArtifactError::Corrupt(format!("{name}: chunk at {} {what}", j.at));
    let raw = zstd::bulk::decompress(j.frame, j.chunk.raw as usize)
        .map_err(|_| bad("does not decompress"))?;
    if raw.len() as u64 != j.chunk.raw || sum_hex(&raw) != j.chunk.sum {
        return Err(bad("has another checksum"));
    }
    Ok(raw)
}

/// Whether `name` is a walked-file list's store name (`files-<32 hex digits>`).
/// Installs the artifact `src` into the graph directory `out` for the
/// repository at `root`.
///
/// `expect` is given the header and returns the options digest this binary
/// computes on this host for the header's options
/// ([`crate::build::options_digest_of`]), or why it cannot: an artifact whose
/// digest differs is rejected, since no shard of it could be reused.
/// `extra` names the [`Header::extra`] files the caller wants written to `out`.
///
/// Without `force`, a directory that already holds a graph this binary reads is
/// left alone ([`ArtifactError::Exists`]): it is usually closer to the working
/// tree than an artifact. With `force` the artifact replaces it (its pack and
/// log; its machine-keyed caches stay, they are valid for the files they stamp,
/// and so does what is keyed by the manifest or checked against the graph it is
/// used with, [`CHECKED`]).
///
/// On any error nothing usable is left behind half-written: every file is
/// written under a temporary name and renamed, the manifest last.
pub fn unpack(
    src: &Path,
    out: &Path,
    root: &Path,
    expect: &dyn Fn(&Header) -> Result<String, String>,
    extra: &[&str],
    force: bool,
) -> Result<UnpackReport, ArtifactError> {
    let t0 = Instant::now();
    let file = std::fs::File::open(src).map_err(io(src))?;
    let len = file.metadata().map_err(io(src))?.len();
    if len == 0 {
        return Err(ArtifactError::NotAnArtifact(src.to_path_buf()));
    }
    // SAFETY: read-only; an artifact is written under a temporary name and renamed.
    let map = unsafe { memmap2::Mmap::map(&file) }.map_err(io(src))?;
    let (header, data) = parse_header(&map, src)?;
    check_binary(&header)?;
    match expect(&header) {
        Ok(d) if d == header.options_digest => {}
        Ok(d) => {
            return Err(ArtifactError::Incompatible(format!(
                "options digest {} (this host and binary compute {d} for the same options: \
                 another target platform or language configuration)",
                header.options_digest
            )));
        }
        Err(why) => return Err(ArtifactError::Incompatible(why)),
    }
    if !force && Manifest::read(out).is_some() {
        return Err(ArtifactError::Exists(out.to_path_buf()));
    }

    // The chunk table against the file.
    let mut jobs: Vec<Job<'_>> = Vec::new();
    let mut pos = data as u64;
    for (si, s) in header.sections.iter().enumerate() {
        let mut at = 0u64;
        for c in &s.chunks {
            let end = pos
                .checked_add(c.zst)
                .filter(|&e| e <= len)
                .ok_or_else(|| ArtifactError::Corrupt("the file is cut off".into()))?;
            jobs.push(Job {
                section: si,
                at,
                frame: &map[pos as usize..end as usize],
                chunk: c,
            });
            at = at.saturating_add(c.raw);
            pos = end;
        }
        if at != s.raw {
            return Err(ArtifactError::Corrupt(format!("{}: chunk sizes", s.name)));
        }
    }
    if pos != len {
        return Err(ArtifactError::Corrupt("bytes after the last chunk".into()));
    }
    // Every frame as it is in the file, before any of them is used: with the
    // header's own checksum and the lengths above, no byte of the file is left
    // unchecked. (The content checksums below do not see a damaged frame that
    // decompresses to the same content, nor the frames of a section that is not
    // installed here.)
    jobs.par_iter().try_for_each(|j| {
        if sum_hex(j.frame) == j.chunk.zsum {
            return Ok(());
        }
        Err(ArtifactError::Corrupt(format!(
            "{}: the chunk at {} has another checksum in the file",
            header.sections[j.section].name, j.at
        )))
    })?;
    let section = |name: &str| header.sections.iter().position(|s| s.name == name);
    let small = |name: &str| -> Result<Option<Vec<u8>>, ArtifactError> {
        let Some(si) = section(name) else {
            return Ok(None);
        };
        let parts: Vec<Vec<u8>> = jobs
            .par_iter()
            .filter(|j| j.section == si)
            .map(|j| inflate(j, name))
            .collect::<Result<_, _>>()?;
        Ok(Some(parts.concat()))
    };

    // Every chunk is checked before a file of the graph directory is replaced: the
    // small sections in memory, the pack under a temporary name.
    let manifest_bytes =
        small(MANIFEST)?.ok_or_else(|| ArtifactError::Corrupt("no manifest".into()))?;
    let mut manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| ArtifactError::Corrupt("the manifest does not parse".into()))?;
    if manifest.schema != crate::manifest::SCHEMA
        || manifest.version != crate::manifest::VERSION
        || manifest.format != header.format
        || manifest.options_digest != header.options_digest
        || manifest.shards.len() != header.shards
        || !manifest.names_are_canonical()
    {
        return Err(ArtifactError::Corrupt(
            "the manifest does not match the header".into(),
        ));
    }
    let files = small(FILES)?;
    match (&manifest.files, &files) {
        (None, None) => {}
        (Some(name), Some(b))
            if crate::manifest::is_files_name(name)
                && *name == format!("files-{:032x}", xxhash_rust::xxh3::xxh3_128(b)) => {}
        _ => {
            return Err(ArtifactError::Corrupt(
                "the walked-file list does not match the manifest".into(),
            ));
        }
    }
    // At another root, the shards that name the root they were built at are not
    // this tree's ([`Located`]): the installed pack forgets them and the
    // manifest leaves them out, so the refresh traces their packages again and
    // writes their shards.
    let root = std::fs::canonicalize(root)
        .map(|p| turborepo_trace_core::fs::normalize(&p))
        .unwrap_or_else(|_| root.to_path_buf());
    let moved = header
        .located
        .as_ref()
        .filter(|l| l.root != root_digest(&root.to_string_lossy()));
    // Each cache is checked against the manifest's key when it is opened. Composed
    // from shards that are traced again here, the caches stay behind.
    let caches: Vec<(&str, Option<Vec<u8>>)> = [COMPOSED, TABLES]
        .into_iter()
        .map(|name| match moved {
            None => small(name).map(|b| (name, b)),
            Some(_) => Ok((name, None)),
        })
        .collect::<Result<_, _>>()?;
    // Valid for any graph, so also here, where some shards are traced again.
    let checked: Vec<(&str, Vec<u8>)> = CHECKED
        .iter()
        .filter_map(|name| small(name).map(|b| Some((*name, b?))).transpose())
        .collect::<Result<_, _>>()?;
    let pack_si = section(PACK).ok_or_else(|| ArtifactError::Corrupt("no pack".into()))?;
    let t1 = Instant::now();

    let store = out.join("store");
    std::fs::create_dir_all(&store).map_err(io(&store))?;
    let suffix = format!("tmp.{}", std::process::id());
    let mut written = 0u64;

    // The pack: every chunk decompressed and written at its offset, in parallel.
    let pack_path = store.join(shard::PACK_FILE);
    let pack_tmp = pack_path.with_extension(&suffix);
    let install_pack = || -> Result<(), ArtifactError> {
        use turborepo_trace_core::os::FileExt;
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&pack_tmp)
            .map_err(io(&pack_tmp))?;
        let raw_len = header.sections[pack_si].raw;
        f.set_len(raw_len).map_err(io(&pack_tmp))?;
        let pack_jobs: Vec<&Job<'_>> = jobs.iter().filter(|j| j.section == pack_si).collect();
        if raw_len > 0 && turborepo_trace_core::debug::var_os("UNPACK_MMAP").is_some() {
            // Measurement switch: decompress into a shared mapping of the file instead of
            // into a buffer that is then written.
            // SAFETY: the file was just created under a temporary name by this process.
            let mut map = unsafe { memmap2::MmapMut::map_mut(&f) }.map_err(io(&pack_tmp))?;
            let mut rest: &mut [u8] = &mut map;
            let mut slices = Vec::with_capacity(pack_jobs.len());
            for j in &pack_jobs {
                let (head, tail) = rest.split_at_mut(j.chunk.raw as usize);
                slices.push(head);
                rest = tail;
            }
            pack_jobs.par_iter().zip(slices).try_for_each(|(j, dst)| {
                let bad = |what: &str| {
                    ArtifactError::Corrupt(format!("{PACK}: chunk at {} {what}", j.at))
                };
                let n = zstd::bulk::decompress_to_buffer(j.frame, dst)
                    .map_err(|_| bad("does not decompress"))?;
                if n as u64 != j.chunk.raw || sum_hex(dst) != j.chunk.sum {
                    return Err(bad("has another checksum"));
                }
                Ok(())
            })?;
        } else {
            pack_jobs.par_iter().try_for_each(|j| {
                let raw = inflate(j, PACK)?;
                f.write_all_at(&raw, j.at).map_err(io(&pack_tmp))
            })?;
        }
        drop(f);
        // A pack of exactly the manifest's shards.
        let p = shard::Pack::read(&pack_tmp)
            .ok_or_else(|| ArtifactError::Corrupt("the pack does not parse".into()))?;
        if let Some(e) = manifest.shards.iter().find(|e| !p.contains(&e.key)) {
            return Err(ArtifactError::Corrupt(format!(
                "the pack lacks shard {}",
                e.key
            )));
        }
        drop(p);
        if let Some(l) = moved {
            shard::Pack::forget(&pack_tmp, &l.shards).map_err(io(&pack_tmp))?;
        }
        Ok(())
    };
    if let Err(e) = install_pack() {
        let _ = std::fs::remove_file(&pack_tmp);
        return Err(e);
    }
    // A graph that is being replaced stops being one before its shards go.
    let _ = std::fs::remove_file(out.join(MANIFEST));
    std::fs::rename(&pack_tmp, &pack_path).map_err(io(&pack_path))?;
    // Shards of whatever was here before are referenced by nothing now.
    let _ = std::fs::remove_file(store.join(shard::LOG_FILE));
    written += header.sections[pack_si].raw;
    let t2 = Instant::now();

    let put = |path: &Path, bytes: &[u8]| -> Result<u64, ArtifactError> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(io(d))?;
        }
        let tmp = path.with_extension(&suffix);
        std::fs::write(&tmp, bytes)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                ArtifactError::Io(path.to_path_buf(), e)
            })?;
        Ok(bytes.len() as u64)
    };
    if let (Some(name), Some(b)) = (&manifest.files, &files) {
        written += put(&store.join(name), b)?;
    }
    for (name, bytes) in &caches {
        match bytes {
            Some(b) => written += put(&file_of(out, name), b)?,
            None => {
                let _ = std::fs::remove_file(file_of(out, name));
            }
        }
    }
    // (One that a replaced graph left behind stays: it is checked like the
    // artifact's.)
    for (name, bytes) in &checked {
        written += put(&file_of(out, name), bytes)?;
    }
    for name in extra {
        if let Some(text) = header.extra.get(*name) {
            written += put(&out.join(name), text.as_bytes())?;
        }
    }
    manifest.root = root.to_string_lossy().into_owned();
    let mut relocated = 0;
    if let Some(l) = moved {
        let before = manifest.shards.len();
        manifest
            .shards
            .retain(|e| l.shards.binary_search(&e.key).is_err());
        relocated = before - manifest.shards.len();
    }
    manifest
        .write(out)
        .map_err(|e| ArtifactError::Store(e.to_string()))?;
    Ok(UnpackReport {
        header,
        relocated,
        bytes: len,
        written,
        timings_ms: [
            (t1 - t0).as_millis() as u64,
            (t2 - t1).as_millis() as u64,
            t2.elapsed().as_millis() as u64,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::is_files_name;

    #[test]
    fn walked_file_list_names() {
        assert!(is_files_name("files-0123456789abcdef0123456789abcdef"));
        for bad in [
            "files-0123456789abcdef0123456789abcde",
            "files-0123456789abcdef0123456789abcdef0",
            "files-../../../../somewhere/else-0123456",
            "../files-0123456789abcdef0123456789abcdef",
            "files-0123456789abcdef0123456789abcdeg",
            "files-0123456789ABCDEF0123456789abcdef",
            "/tmp/files-0123456789abcdef0123456789abcdef",
            "graph.json",
            "",
        ] {
            assert!(!is_files_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_header_round_trips_and_other_schemas_are_rejected() {
        let h = Header {
            schema: SCHEMA.into(),
            version: VERSION,
            turbotrace: "0.0.0".into(),
            build_id: "b".into(),
            format: 1,
            options_digest: "d".into(),
            options: ManifestOptions::default(),
            label: None,
            shards: 0,
            extra: BTreeMap::new(),
            located: None,
            sections: vec![Section {
                name: PACK.into(),
                raw: 3,
                chunks: vec![Chunk {
                    raw: 3,
                    zst: 12,
                    sum: sum_hex(b"abc"),
                    zsum: sum_hex(b"the frame"),
                }],
            }],
        };
        let file = |h: &Header| {
            let head = serde_json::to_vec(h).unwrap();
            let mut b = MAGIC.to_vec();
            b.extend_from_slice(&(head.len() as u32).to_le_bytes());
            b.extend_from_slice(&head);
            b.extend_from_slice(&xxhash_rust::xxh3::xxh3_64(&head).to_le_bytes());
            b
        };
        let src = Path::new("x");
        let bytes = file(&h);
        let (back, data) = parse_header(&bytes, src).unwrap();
        assert_eq!(back, h);
        assert_eq!(data, bytes.len());
        assert_eq!(back.raw_bytes(), 3);
        assert!(!back.has_caches());
        let other = Header {
            schema: "something-else".into(),
            ..h.clone()
        };
        assert!(matches!(
            parse_header(&file(&other), src),
            Err(ArtifactError::NotAnArtifact(_))
        ));
        // A header of this schema written by another binary parses; the binary check
        // rejects it.
        assert!(matches!(
            check_binary(&back),
            Err(ArtifactError::Incompatible(_))
        ));
    }
}
