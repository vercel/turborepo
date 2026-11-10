//! Per-route fingerprints of a Next.js build output.
//!
//! A route's fingerprint covers every output file the route can load: its
//! server entry and traced chunks, its client chunks, its stylesheets, its
//! prerendered output and the files its trace lists outside the dist dir. Two
//! builds give a route the same fingerprint iff that output is the same after
//! removing what is not content:
//!
//! | Noise | Normalization |
//! |---|---|
//! | the build id (also regex-escaped in data-route regexes) | replaced by a placeholder |
//! | server action ids (salted per build) | replaced by `file#export` from the server reference manifest; ids it does not list (42 hex digits) by one placeholder |
//! | names of other output files inside a file (content hashes) | replaced by a placeholder; a file's hash covers its own content, dependencies count through closure membership |
//! | the absolute paths of the app directory and of the output file tracing root (the repository root of a monorepo) | replaced by placeholders |
//! | declared volatile values (a commit SHA inlined through `env`) | replaced by a placeholder |
//! | debug ids (a UUID per chunk that tools such as Sentry inject, named by a `//# debugId=` comment) | replaced by a placeholder |
//! | source map references (`//# sourceMappingURL=`: maps are named by a hash of their own) | removed |
//! | identifier names the minifier picked per chunk | Turbopack chunks are split into modules and every bound local is renamed by order of appearance ([`super::modules`], feature `oxc`) |
//! | module ids (derived from what identifies a module: text with salted action ids for the proxy of a server action, an absolute path for an alias, a `node_modules` path that pnpm changes when another package is bumped) | where an id is referenced, a hash of the target's text stands, refined [`ROUNDS`] time(s) with the modules the target references; a module's entry does not contain its number |
//! | the order of an entry's server actions (sorted by salted id) | export lists are compared in sorted order |
//!
//! The last one matters because the minifier names locals per chunk: a
//! different inlined string in one module renames identifiers in the other
//! modules of the chunk, so hashing chunk bytes (even with the value replaced)
//! reports every route that loads the chunk.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::OnceLock,
};

use aho_corasick::{AhoCorasick, MatchKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use xxhash_rust::xxh3::xxh3_128;

use super::{
    css::{self, CssRules},
    invokes::Invocation,
    manifest::{
        BuildRoute, BuildRouteKind, BuildRouter, BuildRule, Bundler, ExternalFile, NextBuild,
        RouteRegex,
    },
};

/// The format version of [`Fingerprint`].
pub const FINGERPRINT_VERSION: u32 = 4;

/// Options for [`fingerprint`].
#[derive(Clone, Debug, Default)]
pub struct FingerprintOptions {
    /// Volatile values as `(name, value)`: each occurrence of `value` in an
    /// output file is replaced by a placeholder. Values shorter than 6
    /// bytes are ignored.
    pub volatile: Vec<(String, String)>,
    /// Hash whole chunk files instead of modules (also the behavior without the
    /// `oxc` feature, and for webpack builds).
    pub whole_chunks: bool,
    /// Keep the per-file hashes of every component (larger output, for
    /// `--explain`).
    pub detail: bool,
}

/// The hash of one component of a route.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    /// The hash.
    pub hash: String,
    /// The number of files.
    pub files: usize,
    /// Per-file own-content hashes ([`FingerprintOptions::detail`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<BTreeMap<String, String>>,
}

/// One route's fingerprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteFingerprint {
    /// The manifest key (`/about/page`).
    pub key: String,
    /// Router.
    pub router: BuildRouter,
    /// Page or handler.
    pub kind: BuildRouteKind,
    /// Server entry and the chunks and manifests it loads.
    pub server: Digest,
    /// Client JavaScript (and other non-CSS static files).
    pub client: Digest,
    /// Stylesheets.
    pub css: Digest,
    /// The id of the stylesheets' rule set in [`Fingerprint::css`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub css_rules: Option<String>,
    /// Prerendered output.
    pub prerender: Digest,
    /// Traced files outside the dist dir (paths and, where the trace has them,
    /// hashes).
    pub external: String,
    /// The hash over the route's own output (the components above).
    pub own: String,
    /// The routes, actions and external targets this route invokes at runtime
    /// ([`super::invokes::apply`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invokes: Vec<Invocation>,
    /// `own` combined with the own fingerprints of the routes it invokes,
    /// transitively. Equal to `own` until [`super::invokes::apply`] ran or
    /// when it invokes nothing.
    pub effective: String,
}

/// The fingerprint of a build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// [`FINGERPRINT_VERSION`].
    pub version: u32,
    /// The build id (informational: it is not part of any hash).
    pub build_id: String,
    /// The app directory relative to the repository root (the nearest ancestor
    /// directory with `.git`), `.` for the root itself. Fingerprints of the
    /// same app are compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// The bundler.
    pub bundler: Bundler,
    /// Whether chunks were hashed per module with canonical identifiers.
    pub per_module: bool,
    /// The names of the volatile values that were replaced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volatile: Vec<String>,
    /// Routes by pathname (`/blog/[slug]`).
    pub routes: BTreeMap<String, RouteFingerprint>,
    /// Inputs of every route: `middleware`, `instrumentation`,
    /// `middleware_matchers`, `routing` (redirects, rewrites, headers),
    /// `route_table`, `config`, `images`.
    pub global: BTreeMap<String, String>,
    /// Rule sets of the stylesheets, by id ([`RouteFingerprint::css_rules`]).
    #[serde(default)]
    pub css: BTreeMap<String, CssRules>,
    /// The evaluated routing rules (for rule-level diffs against requested
    /// paths).
    #[serde(default)]
    pub rules: Vec<BuildRule>,
    /// Static then dynamic route regexes (for route-table diffs).
    #[serde(default)]
    pub route_regexes: Vec<RouteRegex>,
    /// `basePath` (empty when unset): requests are matched against rules with
    /// it and against routes without it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_path: String,
    /// `caseSensitiveRoutes`: redirects and rewrites match case-sensitively.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub case_sensitive: bool,
    /// `i18n` of the routes manifest (`locales`, `defaultLocale`), if the app
    /// has it.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub i18n: Value,
    /// The middleware matchers (`regexp`, `originalSource`, conditions); `None`
    /// without middleware.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middleware_matchers: Option<Value>,
    /// Files under `public/` (relative to it) with their content hashes: they
    /// are served from the project directory, not from the build output.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub public: BTreeMap<String, String>,
    /// Things that made the fingerprint less precise or less complete.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub(crate) fn hash_hex(bytes: &[u8]) -> String {
    format!("{:032x}", xxh3_128(bytes))
}

/// Hash of a set of lines (sorted, deduplicated).
pub(crate) fn hash_set<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let set: BTreeSet<&str> = items.into_iter().collect();
    let mut buf = String::new();
    for s in set {
        buf.push_str(s);
        buf.push('\n');
    }
    hash_hex(buf.as_bytes())
}

/// Files that index every route: never part of a route, never followed.
// Invariant: a literal pattern, compiled by the unit tests.
#[allow(clippy::expect_used)]
fn is_global_index(rel: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"^(?:static/[^/]+/_(?:buildManifest|ssgManifest|clientMiddlewareManifest)\.js",
            r"|server/(?:app-paths-manifest|pages-manifest|server-reference-manifest|middleware-manifest",
            r"|functions-config-manifest|next-font-manifest|prefetch-hints|preview-props",
            r"|middleware-react-loadable-manifest|react-loadable-manifest|app-build-manifest)\.(?:json|js)",
            r"|server/(?:middleware-build-manifest|interception-route-rewrite-manifest)\.js)$"
        ))
        .expect("regex")
    })
    .is_match(rel)
}

/// The hash token in the name of an output file, if it has one.
// Invariant: a literal pattern, compiled by the unit tests.
#[allow(clippy::expect_used)]
fn name_token(rel: &str) -> Option<&str> {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        [
            // Turbopack static chunk, and its runtime chunk.
            r"^(?:turbopack-)?([a-z0-9_~-]{12,14})\.(?:js|css)$",
            // Turbopack server chunk.
            r"[-_.]([a-z0-9_~-]{13})\._\.js$",
            r"_([a-z0-9_~-]{13})\.js$",
            // webpack.
            r"(?:^|[-.])([0-9a-f]{16,20})\.(?:js|css)$",
            // static/media.
            r"\.([0-9a-f]{8})\.[a-z0-9]+$",
        ]
        .iter()
        .map(|r| Regex::new(r).expect("regex"))
        .collect()
    });
    // Only where bundlers put hashed names: a page file named like a hash
    // (`server/pages/ subscriptions.js`) must not turn an ordinary word into a
    // placeholder.
    if !["static/", "server/chunks/", "server/edge/"]
        .iter()
        .any(|p| rel.starts_with(p))
    {
        return None;
    }
    let base = rel.rsplit('/').next()?;
    res.iter()
        .find_map(|re| re.captures(base))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        // A hash has a digit; `static/chunks/main-app-util.js` does not.
        .filter(|t| t.bytes().any(|b| b.is_ascii_digit()))
}

/// The stack of a thread that parses chunks (reserved, not committed).
const SPLIT_STACK: usize = 512 << 20;

/// How many times the name of a module is refined with the names of the modules
/// it references. With 0 rounds a name is the module's own text; two modules
/// with the same text that reference different modules (two forwarding modules)
/// would share it, and a change of which one an importer uses would go unseen.
/// Every round also makes a module's entry depend on modules one reference
/// further away, so a change reaches more importers (and the routes that share
/// a chunk with them): one round separates forwarding modules and keeps that
/// reach at two references. (On 20 commit pairs of a public app, 0, 1 and 4
/// rounds flagged the same routes.)
pub const ROUNDS: usize = 1;

/// The debug id a file ends with (`//# debugId=<uuid>`), if any.
fn debug_id(build: &NextBuild, rel: &str) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(build.dist.join(rel)).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(400))).ok()?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).ok()?;
    let key = b"debugId=";
    let at = tail.windows(key.len()).rposition(|w| w == key)? + key.len();
    let id: Vec<u8> = tail[at..]
        .iter()
        .copied()
        .take_while(|b| b.is_ascii_hexdigit() || *b == b'-')
        .collect();
    (id.len() >= 32).then_some(id)
}

/// Replaces everything in an output file that is not its own content.
pub(crate) struct Normalizer {
    ac: Option<AhoCorasick>,
    replacements: Vec<Vec<u8>>,
}

impl Normalizer {
    // Invariant: literal patterns; building fails only past size limits far above
    // these.
    #[allow(clippy::expect_used)]
    pub(crate) fn new(build: &NextBuild, volatile: &[(String, String)]) -> Normalizer {
        let mut pats: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut add = |p: Vec<u8>, r: &str, pats: &mut Vec<(Vec<u8>, Vec<u8>)>| {
            if !p.is_empty() && seen.insert(p.clone()) {
                pats.push((p, r.as_bytes().to_vec()));
            }
        };
        // Declared volatile values first: on equal matches the earlier pattern wins.
        for (name, value) in volatile {
            if value.len() >= 6 {
                add(
                    value.clone().into_bytes(),
                    &format!("<ENV:{name}>"),
                    &mut pats,
                );
            }
        }
        // The build id: raw, regex-escaped, and regex-escaped inside JSON.
        let id = &build.build_id;
        if id.len() >= 6 {
            for v in [id.clone(), id.replace('-', "\\-"), id.replace('-', "\\\\-")] {
                add(v.into_bytes(), "<BUILD_ID>", &mut pats);
            }
        }
        let root = build.project.to_string_lossy();
        if root.len() > 1 {
            add(root.as_bytes().to_vec(), "<ROOT>", &mut pats);
        }
        // The paths the build itself recorded: the app directory where it ran (the
        // output may have been moved since) and the tracing root above it,
        // which traced paths of a monorepo start with.
        if let Some(v) = build
            .read("required-server-files.json")
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            if let Some(dir) = v.get("appDir").and_then(Value::as_str)
                && dir.len() > 1
            {
                add(dir.as_bytes().to_vec(), "<ROOT>", &mut pats);
            }
            for key in ["outputFileTracingRoot", "turbopack"] {
                let dir = match v.get("config").and_then(|c| c.get(key)) {
                    Some(Value::String(s)) => Some(s.as_str()),
                    Some(o) => o.get("root").and_then(Value::as_str),
                    None => None,
                };
                if let Some(dir) = dir
                    && dir.len() > 1
                {
                    add(dir.as_bytes().to_vec(), "<REPO>", &mut pats);
                }
            }
        }
        // Debug ids: one per file that has one, and the same id inside the file.
        for f in &build.files {
            if f.ends_with(".js")
                && let Some(id) = debug_id(build, f)
            {
                add(id, "<DEBUG_ID>", &mut pats);
            }
        }
        for a in &build.actions {
            add(
                a.id.clone().into_bytes(),
                &format!("<ACTION {}>", a.stable_id()),
                &mut pats,
            );
        }
        for f in &build.files {
            if let Some(t) = name_token(f) {
                add(t.as_bytes().to_vec(), "<F>", &mut pats);
            }
        }
        let ac = (!pats.is_empty()).then(|| {
            AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build(pats.iter().map(|(p, _)| p))
                .expect("patterns")
        });
        Normalizer {
            ac,
            replacements: pats.into_iter().map(|(_, r)| r).collect(),
        }
    }

    // Invariant: a literal pattern, compiled by the unit tests.
    #[allow(clippy::expect_used)]
    pub(crate) fn apply(&self, bytes: &[u8]) -> Vec<u8> {
        // A source map reference names a file that is not output of a route (maps are
        // not read), with a hash of its own.
        static MAPS: OnceLock<regex::bytes::Regex> = OnceLock::new();
        let maps = MAPS.get_or_init(|| {
            regex::bytes::Regex::new(r"(?m)^(?://[#@]|/\*#) ?sourceMappingURL=[^\n]*$")
                .expect("regex")
        });
        let bytes = maps.replace_all(bytes, &b""[..]);
        let bytes = match &self.ac {
            Some(ac) => ac.replace_all_bytes(&bytes, &self.replacements),
            None => bytes.into_owned(),
        };
        // A chunk that a manifest names but the output does not hold (a route's
        // loadable manifest lists such names): a hashed name like the others,
        // with no file to take the name from.
        static NAMES: OnceLock<regex::bytes::Regex> = OnceLock::new();
        let names = NAMES.get_or_init(|| {
            regex::bytes::Regex::new(r"(?-u)static/chunks/([a-z0-9_~-]{12,14})\.(js|css)\b")
                .expect("regex")
        });
        let bytes = match names.replace_all(&bytes, |c: &regex::bytes::Captures<'_>| {
            // A hash has a digit, as in `name_token`.
            if c[1].iter().any(u8::is_ascii_digit) {
                let mut v = b"static/chunks/<F>.".to_vec();
                v.extend_from_slice(&c[2]);
                v
            } else {
                c[0].to_vec()
            }
        }) {
            std::borrow::Cow::Borrowed(_) => bytes,
            std::borrow::Cow::Owned(v) => v,
        };
        // Action ids the server reference manifest does not list (a function that is
        // registered but not exported): 42 hex digits, salted like the others.
        static ACTIONS: OnceLock<regex::bytes::Regex> = OnceLock::new();
        let actions = ACTIONS
            .get_or_init(|| regex::bytes::Regex::new(r"(?-u)\b[0-9a-f]{42}\b").expect("regex"));
        match actions.replace_all(&bytes, &b"<ACTION>"[..]) {
            std::borrow::Cow::Borrowed(_) => bytes,
            std::borrow::Cow::Owned(v) => v,
        }
    }
}

struct Ctx<'a> {
    build: &'a NextBuild,
    opts: &'a FingerprintOptions,
    norm: Normalizer,
    /// Per file: its own-content hash and, for split chunks, its module
    /// entries.
    hashes: HashMap<String, (String, Option<Vec<String>>)>,
    /// The module entries of every chunk that could be split
    /// ([`Ctx::split_chunks`]).
    entries: HashMap<String, Vec<String>>,
    refs: HashMap<String, Vec<String>>,
    notes: BTreeSet<String>,
}

fn is_text(rel: &str) -> bool {
    [
        ".js", ".json", ".css", ".html", ".rsc", ".meta", ".txt", ".body",
    ]
    .iter()
    .any(|e| rel.ends_with(e))
}

/// Whether `rel` is where Turbopack writes chunks.
fn is_chunk_path(rel: &str) -> bool {
    rel.ends_with(".js") && (rel.starts_with("static/chunks/") || rel.starts_with("server/chunks/"))
}

impl Ctx<'_> {
    /// Splits every chunk into modules and gives each module an entry: its id
    /// and a hash of its canonical text (see [`super::modules`]).
    // Invariant: spawning fails only when the OS has no threads left; spawning fails only when the
    // OS has no threads left, and a join fails only when the worker panicked, which this re-raises.
    #[allow(clippy::expect_used)]
    fn split_chunks(&mut self) {
        use super::modules::{self, Chunk};
        let build = self.build;
        let norm = &self.norm;
        let files: Vec<&String> = build.files.iter().filter(|f| is_chunk_path(f)).collect();
        // Minified bundles nest deeply (a chain of a thousand `+` is one expression):
        // the parser and the visitors recurse, so they run on threads with a
        // large stack.
        let next = std::sync::atomic::AtomicUsize::new(0);
        let workers = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .min(16);
        let mut split: Vec<(String, Option<Chunk>)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..workers)
                .map(|_| {
                    std::thread::Builder::new()
                        .stack_size(SPLIT_STACK)
                        .spawn_scoped(s, || {
                            let mut out = Vec::new();
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some(rel) = files.get(i) else {
                                    break;
                                };
                                let Some(raw) = build.read(rel) else {
                                    continue;
                                };
                                let bytes = norm.apply(&raw);
                                let Ok(text) = std::str::from_utf8(&bytes) else {
                                    continue;
                                };
                                if modules::is_turbopack_chunk(text) {
                                    out.push(((*rel).clone(), modules::split(text)));
                                }
                            }
                            out
                        })
                        .expect("spawn a thread")
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().expect("chunk splitting thread"))
                .collect()
        });
        split.sort_by(|a, b| a.0.cmp(&b.0));
        let mut chunks: Vec<(String, Chunk)> = Vec::new();
        for (rel, c) in split {
            match c {
                Some(c) => chunks.push((rel, c)),
                None => {
                    self.notes.insert(format!(
                        "{rel}: chunk not split into modules (hashed whole)"
                    ));
                }
            }
        }
        // A module id is not content: it is derived from what identifies the module (a
        // path, or for a generated module text that holds values of this build), and it
        // differs between builds of one source (see `super::modules`). Where an id is
        // referenced, a name made from the target's text stands instead: the hash of
        // its text with every id as one placeholder, refined [`ROUNDS`] times
        // with the names of the modules it references. (An id defined in
        // several chunks, as a module shared by two bundles is, stands for all
        // its definitions.)
        let mut defs: HashMap<&str, Vec<&str>> = HashMap::new();
        for (_, c) in &chunks {
            for m in &c.modules {
                defs.entry(m.id.as_str()).or_default().push(m.text.as_str());
            }
        }
        let describe = |name: &dyn Fn(&str) -> Option<String>| -> HashMap<&str, String> {
            defs.iter()
                .map(|(id, texts)| {
                    let mut hs: Vec<String> = texts
                        .iter()
                        .map(|t| hash_hex(resolve_with(t, name).as_bytes()))
                        .collect();
                    hs.sort_unstable();
                    hs.dedup();
                    (*id, hash_hex(hs.join(" ").as_bytes()))
                })
                .collect()
        };
        let mut names: HashMap<&str, String> =
            describe(&|id| defs.contains_key(id).then(|| "module".to_string()));
        for _ in 0..ROUNDS {
            let next = describe(&|id| names.get(id).cloned());
            names = next;
        }
        // Diagnostic: TURBO_TRACE_DEBUG_FP_DUMP=<dir> writes every chunk's entries with
        // the text each one hashes, to compare two builds module by module.
        let dump = turborepo_trace_core::debug::var_os("FP_DUMP").map(std::path::PathBuf::from);
        for (rel, c) in &chunks {
            let mut entries: Vec<String> = Vec::new();
            let mut lines = String::new();
            let texts = c
                .modules
                .iter()
                .map(|m| ("m", m.id.as_str(), m.text.as_str()))
                .chain(c.other.iter().map(|t| ("x", "", t.as_str())));
            for (kind, id, text) in texts {
                let resolved = resolve_with(text, &|id| names.get(id).cloned());
                let entry = format!("{kind}:{}", hash_hex(resolved.as_bytes()));
                if dump.is_some() {
                    lines.push_str(&format!(
                        "{entry}\t{id}\t{}\n",
                        resolved.replace('\n', "\\n")
                    ));
                }
                entries.push(entry);
            }
            if let Some(dir) = &dump {
                let _ = std::fs::create_dir_all(dir);
                let _ = std::fs::write(dir.join(rel.replace('/', "__")), lines);
            }
            self.entries.insert(rel.clone(), entries);
        }
    }

    /// References to other output files in the text of `rel`.
    // Invariant: a literal pattern, compiled by the unit tests.
    #[allow(clippy::expect_used)]
    fn refs(&mut self, rel: &str) -> Vec<String> {
        if let Some(r) = self.refs.get(rel) {
            return r.clone();
        }
        static RE: OnceLock<regex::bytes::Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            regex::bytes::Regex::new(
                r"(?:static|server)/[A-Za-z0-9_\-.~@()\[\]$!%/]+?\.(?:js|css|json|woff2?|ttf|otf|png|jpe?g|gif|webp|avif|svg|ico|wasm|txt)",
            )
            .expect("regex")
        });
        let mut out: BTreeSet<String> = BTreeSet::new();
        if is_text(rel)
            && let Some(bytes) = self.build.read(rel)
        {
            for m in re.find_iter(&bytes) {
                let Ok(s) = std::str::from_utf8(m.as_bytes()) else {
                    continue;
                };
                if self.build.files.contains(s) {
                    out.insert(s.to_string());
                } else {
                    let d = super::manifest::percent_decode(s);
                    if self.build.files.contains(&d) {
                        out.insert(d);
                    }
                }
            }
        }
        let v: Vec<String> = out.into_iter().collect();
        self.refs.insert(rel.to_string(), v.clone());
        v
    }

    /// The files reachable from `seeds` through references, without the global
    /// indexes.
    fn closure(&mut self, seeds: impl IntoIterator<Item = String>) -> BTreeSet<String> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = seeds.into_iter().collect();
        while let Some(f) = stack.pop() {
            if seen.contains(&f) || !self.build.files.contains(&f) || is_global_index(&f) {
                continue;
            }
            for r in self.refs(&f) {
                if !seen.contains(&r) {
                    stack.push(r);
                }
            }
            seen.insert(f);
        }
        seen
    }

    fn file(&mut self, rel: &str) -> &(String, Option<Vec<String>>) {
        if !self.hashes.contains_key(rel) {
            let raw = self.build.read(rel).unwrap_or_default();
            let bytes = if is_text(rel) {
                self.norm.apply(&raw)
            } else {
                raw
            };
            let own = hash_hex(&bytes);
            let entries = self.entries.get(rel).cloned();
            self.hashes.insert(rel.to_string(), (own, entries));
        }
        &self.hashes[rel]
    }

    fn digest(&mut self, files: &BTreeSet<String>) -> Digest {
        let mut parts: BTreeSet<String> = BTreeSet::new();
        let mut detail = BTreeMap::new();
        for f in files {
            let (own, entries) = self.file(f).clone();
            match entries {
                Some(e) => parts.extend(e),
                None => {
                    parts.insert(own.clone());
                }
            }
            if self.opts.detail {
                detail.insert(f.clone(), own);
            }
        }
        Digest {
            hash: hash_set(parts.iter().map(String::as_str)),
            files: files.len(),
            detail: self.opts.detail.then_some(detail),
        }
    }

    fn external(&mut self, files: &[ExternalFile]) -> String {
        let mut lines: Vec<String> = Vec::new();
        for f in files {
            let hash = match &f.hash {
                Some(h) => h.clone(),
                // No hash in the trace (webpack): first-party files are read, library files
                // count by path (their versions change through the lockfile).
                None if !f.path.contains("node_modules/") => {
                    match crate::fsx::read(self.build.project.join(&f.path)) {
                        Ok(b) => hash_hex(&b),
                        Err(_) => "missing".to_string(),
                    }
                }
                None => String::new(),
            };
            lines.push(format!("{}:{hash}", f.path));
        }
        hash_set(lines.iter().map(String::as_str))
    }

    fn json_hash(&self, v: &Value) -> String {
        let s = serde_json::to_string(v).unwrap_or_default();
        hash_hex(&self.norm.apply(s.as_bytes()))
    }

    fn route(
        &mut self,
        r: &BuildRoute,
        css_sets: &mut BTreeMap<String, CssRules>,
    ) -> RouteFingerprint {
        let mut server_seeds: Vec<String> = vec![r.entry.clone()];
        server_seeds.extend(
            r.trace
                .inside
                .iter()
                .filter(|f| f.starts_with("server/"))
                .cloned(),
        );
        server_seeds.extend(r.client_manifest.iter().cloned());
        server_seeds.extend(r.edge_files.iter().cloned());
        let sclo = self.closure(server_seeds);
        let server: BTreeSet<String> = sclo
            .iter()
            .filter(|f| f.starts_with("server/"))
            .cloned()
            .collect();
        let mut client_seeds: Vec<String> = r.client_files.clone();
        if r.kind == BuildRouteKind::Page {
            // Static files a server chunk names (asset URLs, stylesheets).
            client_seeds.extend(sclo.iter().filter(|f| f.starts_with("static/")).cloned());
        }
        let cclo = self.closure(client_seeds);
        let (css, client): (BTreeSet<String>, BTreeSet<String>) =
            cclo.into_iter().partition(|f| f.ends_with(".css"));
        let prerender: BTreeSet<String> = self.build.prerender_files(r).into_iter().collect();

        let server_d = self.digest(&server);
        let client_d = self.digest(&client);
        let css_d = self.digest(&css);
        let prerender_d = self.digest(&prerender);
        let external = self.external(&r.trace.external);
        let css_rules = (!css.is_empty()).then(|| {
            let id = css_d.hash.clone();
            if !css_sets.contains_key(&id) {
                let mut rules = CssRules::default();
                for f in &css {
                    if let Some(b) = self.build.read(f) {
                        css::parse_into(&String::from_utf8_lossy(&self.norm.apply(&b)), &mut rules);
                    }
                }
                css_sets.insert(id.clone(), rules);
            }
            id
        });
        let own = hash_hex(
            format!(
                "{}\n{}\n{}\n{}\n{external}",
                server_d.hash, client_d.hash, css_d.hash, prerender_d.hash
            )
            .as_bytes(),
        );
        RouteFingerprint {
            key: r.key.clone(),
            router: r.router,
            kind: r.kind,
            server: server_d,
            client: client_d,
            css: css_d,
            css_rules,
            prerender: prerender_d,
            external,
            effective: own.clone(),
            own,
            invokes: Vec::new(),
        }
    }
}

/// [`super::modules::resolve`] with owned replacements.
fn resolve_with(text: &str, name: &dyn Fn(&str) -> Option<String>) -> String {
    // The replacements are short-lived strings: collect them, then borrow.
    let mut ids = super::modules::integers(text);
    if ids.is_empty() {
        return text.to_string();
    }
    ids.sort_unstable();
    ids.dedup();
    let found: HashMap<&str, String> = ids
        .into_iter()
        .filter_map(|id| name(id).map(|h| (id, h)))
        .collect();
    super::modules::resolve(text, |id| found.get(id).map(String::as_str))
}

/// Config keys that hold paths, ids or derived data of the build, not behavior.
const CONFIG_SKIP: &[&str] = &[
    "configFile",
    "configFileName",
    "outputFileTracingRoot",
    "distDirRoot",
    "repoRoot",
    "buildId",
    "deploymentId",
    "cacheHandlers",
    "clientRouterFilters",
];

fn strip_keys(v: &Value, skip: &[&str]) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| !skip.contains(&k.as_str()))
                .map(|(k, x)| (k.clone(), strip_keys(x, skip)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| strip_keys(x, skip)).collect()),
        other => other.clone(),
    }
}

/// The project directory relative to the nearest ancestor that has `.git`.
fn app_dir(build: &NextBuild) -> Option<String> {
    let mut dir = build.project.as_path();
    loop {
        if crate::fsx::exists(&dir.join(".git")) {
            let rel = build.project.strip_prefix(dir).ok()?;
            let rel = rel.to_string_lossy().replace('\\', "/");
            return Some(if rel.is_empty() { ".".to_string() } else { rel });
        }
        dir = dir.parent()?;
    }
}

/// The files under `<project>/public` with their content hashes.
fn public_files(build: &NextBuild) -> BTreeMap<String, String> {
    let dir = build.project.join("public");
    let mut out = BTreeMap::new();
    crate::fsx::walk_files(&dir, &mut |p| {
        let Ok(rel) = p.strip_prefix(&dir) else {
            return;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if let Ok(bytes) = crate::fsx::read(p) {
            out.insert(rel, hash_hex(&bytes));
        }
    });
    out
}

/// Fingerprints `build`.
pub fn fingerprint(build: &NextBuild, opts: &FingerprintOptions) -> Fingerprint {
    let per_module =
        !opts.whole_chunks && build.bundler == Bundler::Turbopack && super::modules::AVAILABLE;
    let mut cx = Ctx {
        build,
        opts,
        norm: Normalizer::new(build, &opts.volatile),
        hashes: HashMap::new(),
        entries: HashMap::new(),
        refs: HashMap::new(),
        notes: build.warnings.iter().cloned().collect(),
    };
    if per_module {
        cx.split_chunks();
    }
    if !per_module && !opts.whole_chunks {
        cx.notes.insert(
            match build.bundler {
                Bundler::Webpack => {
                    "webpack build: chunks are hashed whole (module ids and identifier names are \
                     not normalized)"
                }
                Bundler::Turbopack => "built without the `oxc` feature: chunks are hashed whole",
            }
            .to_string(),
        );
    }
    let mut css_sets = BTreeMap::new();
    let mut routes = BTreeMap::new();
    for r in &build.routes {
        let fp = cx.route(r, &mut css_sets);
        routes.insert(r.pathname.clone(), fp);
    }

    let mut global = BTreeMap::new();
    for g in &build.globals {
        let clo = cx.closure(g.files.iter().cloned());
        let d = cx.digest(&clo);
        let ext = cx.external(&g.external);
        global.insert(
            g.name.clone(),
            hash_hex(format!("{}\n{ext}", d.hash).as_bytes()),
        );
        if g.name == "middleware" {
            global.insert("middleware_matchers".to_string(), cx.json_hash(&g.matchers));
        }
    }
    let rules: Vec<&Value> = build.routing.rules.iter().map(|r| &r.raw).collect();
    global.insert(
        "routing".to_string(),
        cx.json_hash(&serde_json::json!({
            "basePath": build.routing.base_path,
            "rules": rules,
            "switches": build.routing.switches,
        })),
    );
    let route_regexes: Vec<RouteRegex> = build
        .routing
        .static_routes
        .iter()
        .chain(&build.routing.dynamic_routes)
        .cloned()
        .collect();
    global.insert(
        "route_table".to_string(),
        cx.json_hash(&serde_json::to_value(&route_regexes).unwrap_or(Value::Null)),
    );
    global.insert(
        "config".to_string(),
        cx.json_hash(&strip_keys(&build.config, CONFIG_SKIP)),
    );
    global.insert("images".to_string(), cx.json_hash(&build.images));

    Fingerprint {
        version: FINGERPRINT_VERSION,
        build_id: build.build_id.clone(),
        app: app_dir(build),
        bundler: build.bundler,
        per_module,
        volatile: opts
            .volatile
            .iter()
            .filter(|(_, v)| v.len() >= 6)
            .map(|(n, _)| n.clone())
            .collect(),
        routes,
        global,
        css: css_sets,
        rules: build.routing.rules.clone(),
        route_regexes,
        base_path: build.routing.base_path.clone(),
        case_sensitive: build
            .routing
            .switches
            .get("caseSensitive")
            .and_then(Value::as_bool)
            == Some(true),
        i18n: build
            .routing
            .switches
            .get("i18n")
            .cloned()
            .unwrap_or(Value::Null),
        middleware_matchers: build
            .globals
            .iter()
            .find(|g| g.name == "middleware")
            .map(|g| g.matchers.clone()),
        public: public_files(build),
        notes: cx.notes.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_tokens() {
        assert_eq!(
            name_token("static/chunks/3vl0xogptndf0.js"),
            Some("3vl0xogptndf0")
        );
        assert_eq!(
            name_token("static/chunks/turbopack-2u1d16houkiyo.js"),
            Some("2u1d16houkiyo")
        );
        assert_eq!(
            name_token("server/chunks/ssr/[root-of-the-server]__0chjj_py90_7q._.js"),
            Some("0chjj_py90_7q")
        );
        assert_eq!(
            name_token(
                "server/chunks/ssr/_next-internal_server_app_about_page_actions_0xvgj076vz0b3.js"
            ),
            Some("0xvgj076vz0b3")
        );
        assert_eq!(
            name_token("static/chunks/app/layout-3d4b3b14417be46b.js"),
            Some("3d4b3b14417be46b")
        );
        assert_eq!(
            name_token("static/media/font.1a2b3c4d.woff2"),
            Some("1a2b3c4d")
        );
        assert_eq!(name_token("server/app/about/page.js"), None);
        assert_eq!(name_token("routes-manifest.json"), None);
    }

    #[test]
    fn source_maps_and_unlisted_action_ids_are_not_content() {
        let n = Normalizer {
            ac: None,
            replacements: Vec::new(),
        };
        let a = n.apply(
            b"a();\n//# sourceMappingURL=abc.js.map\nb(\"40a452eb01d137bf9096b9ae851bd2be2e6340dc9f\")",
        );
        let b = n.apply(
            b"a();\n//# sourceMappingURL=xyz.js.map\nb(\"40ee79c5bb97fa7810c0ce515af47a662acc9d4004\")",
        );
        assert_eq!(a, b);
        assert!(!String::from_utf8_lossy(&a).contains("sourceMappingURL"));
        // 40 hex digits (a commit id) and 64 (a SHA-256) are content.
        for other in [
            "3fb02e6eb114885be677f1e60ee17c8da0e4c3ad",
            "130de9dbc1d647f48a41d0f7067622d1591d0de92e9efb5a3681a813c72a2b85",
        ] {
            let c = n.apply(format!("x(\"{other}\")").as_bytes());
            assert!(String::from_utf8_lossy(&c).contains(other));
        }
    }

    #[test]
    fn a_chunk_name_without_a_file_is_not_content() {
        // A loadable manifest can name a chunk that is not among the output files: no
        // pattern comes from a file then, and the name is a hash all the same.
        let n = Normalizer {
            ac: None,
            replacements: Vec::new(),
        };
        let a = n.apply(br#"{"500001":{"id":500001,"files":["static/chunks/0ab1cdefgh2ij.js"]}}"#);
        let b = n.apply(br#"{"500001":{"id":500001,"files":["static/chunks/9zy8xwvuts7rq.js"]}}"#);
        assert_eq!(a, b);
        // Another kind of file, another module and a name that is a word are content.
        for other in [
            r#"{"500001":{"id":500001,"files":["static/chunks/0ab1cdefgh2ij.css"]}}"#,
            r#"{"500002":{"id":500002,"files":["static/chunks/0ab1cdefgh2ij.js"]}}"#,
            r#"{"500001":{"id":500001,"files":["static/chunks/main-app-util.js"]}}"#,
        ] {
            assert_ne!(a, n.apply(other.as_bytes()), "{other}");
        }
    }

    #[test]
    fn global_indexes() {
        assert!(is_global_index("static/abc-DEF/_buildManifest.js"));
        assert!(is_global_index("server/server-reference-manifest.json"));
        assert!(!is_global_index(
            "server/app/about/page/build-manifest.json"
        ));
        assert!(!is_global_index(
            "server/app/about/page_client-reference-manifest.js"
        ));
    }

    #[test]
    fn set_hash_ignores_order_and_duplicates() {
        assert_eq!(hash_set(["b", "a", "a"]), hash_set(["a", "b"]));
        assert_ne!(hash_set(["a"]), hash_set(["a", "b"]));
    }
}
