//! Third-party package identity (opaque packages).
//!
//! Two sources, tried in order:
//!
//! 1. **Language providers** ([`IdentityProvider`], implemented for [`Tracer`]
//!    through [`turborepo_trace_core::Resolver::package_identity`]): a Python
//!    distribution (dist-info name, version, a digest of its `RECORD`), a Go
//!    module version below the module cache or a `vendor/` directory (`go.sum`
//!    hash), a crate from the cargo registry, a git checkout or `vendor/`
//!    (`Cargo.lock` checksum or source; the containing package, not the
//!    checkout root). Their names are namespaced by ecosystem (`pypi:requests`,
//!    `go:golang.org/x/text`, `crates:serde`), so opaque nodes of different
//!    ecosystems never collide. A missing integrity is taken from that
//!    ecosystem's lockfile ([`Locks`]).
//! 2. **`node_modules`**: a file reached inside a package directory
//!    (`…/node_modules/<name>/…`, pnpm's `…/.pnpm/<id>/node_modules/<name>/…`)
//!    belongs to the package rooted at the last marker. Its identity is
//!    `name@version` from the installed `package.json`, plus the integrity from
//!    the repository lockfile when it lists that version (pnpm, npm, yarn and
//!    bun lockfiles through `turborepo_trace_lockfile`). npm names are not
//!    namespaced.
//!
//! Lockfiles are loaded per ecosystem with the matching parser
//! ([`load_lockfiles`]): the JS lockfile at the root, every `Cargo.lock`, every
//! `go.mod` with its `go.sum` (`turborepo_trace_lockfile::gosum::parse`),
//! `uv.lock`, `poetry.lock`, `Pipfile.lock` and `requirements*.txt`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use turborepo_trace_core::{DirCache, PackageBoundary, PackageIdentity, Tracer};
use turborepo_trace_lockfile::Lockfile;

/// Answers "which third-party package is this file in" for languages that know
/// (see the module docs). Implemented for [`Tracer`], which asks its languages'
/// resolvers.
pub trait IdentityProvider: Send + Sync {
    /// The package `path` (absolute, real) belongs to, if it is third-party.
    fn identity(&self, path: &Path, fs: &DirCache) -> Option<PackageIdentity>;

    /// The package an external target name stands for when its files are not
    /// installed ([`turborepo_trace_core::Resolver::external_identity`]).
    /// Default: unknown.
    fn external(&self, _name: &str, _fs: &DirCache) -> Option<PackageIdentity> {
        None
    }
}

impl IdentityProvider for Tracer {
    fn identity(&self, path: &Path, _fs: &DirCache) -> Option<PackageIdentity> {
        self.package_identity(path)
    }

    fn external(&self, name: &str, _fs: &DirCache) -> Option<PackageIdentity> {
        self.external_identity(name)
    }
}

/// The store name of a package: npm names as is, others as
/// `<ecosystem>:<name>`.
pub fn store_name(ecosystem: &str, name: &str) -> String {
    if ecosystem == "npm" {
        name.to_owned()
    } else {
        format!("{ecosystem}:{name}")
    }
}

/// A language-provided identity, resolved for the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LangIdentity {
    /// The identity (namespaced name).
    pub id: Identity,
    /// The package root (absolute).
    pub root: PathBuf,
    /// Files pinning the version (config inputs of linking shards).
    pub pins: Vec<PathBuf>,
}

/// Lockfiles of the non-JS ecosystems, by ecosystem.
#[derive(Debug, Default)]
pub struct Locks {
    /// `(ecosystem, lockfile)`.
    pub locks: Vec<(&'static str, Lockfile)>,
}

impl Locks {
    /// The integrity `ecosystem`'s lockfiles record for `name@version`.
    pub fn integrity(&self, ecosystem: &str, name: &str, version: &str) -> Option<String> {
        let canon = |n: &str| n.to_ascii_lowercase().replace(['_', '.'], "-");
        let mut cands: Vec<(&str, &str)> = self
            .locks
            .iter()
            .filter(|(e, _)| *e == ecosystem)
            .flat_map(|(_, l)| l.packages())
            .filter(|p| {
                p.version == version
                    && (p.name == name || (ecosystem == "pypi" && canon(&p.name) == canon(name)))
            })
            .filter_map(|p| {
                // Cargo git dependencies have no checksum: their pinned source is the
                // identity, as `CargoProject::identity` reports it.
                let i = p.integrity.as_deref().or(if ecosystem == "crates" {
                    p.resolution.as_deref()
                } else {
                    None
                })?;
                Some((p.id.as_str(), i))
            })
            .collect();
        cands.sort_unstable();
        cands.first().map(|(_, i)| (*i).to_owned())
    }
}

/// A third-party package identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identity {
    /// Name (from `package.json`, else from the path).
    pub name: String,
    /// Version (from `package.json`, else `"unknown"`).
    pub version: String,
    /// Integrity from the lockfile.
    pub integrity: Option<String>,
}

/// A dependency resolved from the lockfile alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockTarget {
    /// A locked third-party package.
    Package(Identity),
    /// A workspace package, by root (repo-relative).
    Workspace(String),
}

/// The lockfile names looked for at the repository root, in order.
pub const LOCKFILES: &[&str] = &[
    "pnpm-lock.yaml",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "bun.lock",
];

/// Parses the JS lockfile on first use.
type LockLoader = Box<dyn Fn() -> Option<Lockfile> + Send + Sync>;

/// Resolves identities, memoized per package root.
pub struct Identities {
    boundaries: Vec<PackageBoundary>,
    /// The repository root ([`Identities::with_repo`]).
    repo: Option<PathBuf>,
    /// The JS lockfile, parsed when first needed (a warm refresh that reuses
    /// every shard never parses it).
    lock: std::sync::OnceLock<Option<Lockfile>>,
    lock_loader: Option<LockLoader>,
    locks: Locks,
    provider: Option<Arc<dyn IdentityProvider>>,
    memo: Mutex<HashMap<PathBuf, Option<Identity>>>,
}

impl std::fmt::Debug for Identities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identities")
            .field("boundaries", &self.boundaries)
            .field("lock", &self.lock.get().map(|l| l.is_some()))
            .field("locks", &self.locks.locks.len())
            .field("provider", &self.provider.is_some())
            .finish()
    }
}

impl Identities {
    /// Identities for the given package layouts and JS lockfile.
    pub fn new(boundaries: Vec<PackageBoundary>, lock: Option<Lockfile>) -> Self {
        Identities {
            boundaries,
            repo: None,
            lock: std::sync::OnceLock::from(lock),
            lock_loader: None,
            locks: Locks::default(),
            provider: None,
            memo: Mutex::new(HashMap::new()),
        }
    }

    /// Identities whose JS lockfile is parsed by `loader` on first use.
    pub fn with_lazy_lock(
        boundaries: Vec<PackageBoundary>,
        loader: impl Fn() -> Option<Lockfile> + Send + Sync + 'static,
    ) -> Self {
        Identities {
            boundaries,
            repo: None,
            lock: std::sync::OnceLock::new(),
            lock_loader: Some(Box::new(loader)),
            locks: Locks::default(),
            provider: None,
            memo: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> Option<&Lockfile> {
        self.lock
            .get_or_init(|| self.lock_loader.as_ref().and_then(|f| f()))
            .as_ref()
    }

    /// Adds a language identity provider (see the module docs).
    pub fn with_provider(mut self, p: Arc<dyn IdentityProvider>) -> Self {
        self.provider = Some(p);
        self
    }

    /// Adds the lockfiles of other ecosystems.
    pub fn with_locks(mut self, locks: Locks) -> Self {
        self.locks = locks;
        self
    }

    /// The language-provided identity of `path`, with a missing integrity
    /// filled in from the ecosystem's lockfiles.
    pub fn language(&self, path: &Path, fs: &DirCache) -> Option<LangIdentity> {
        let p = self.provider.as_ref()?.identity(path, fs)?;
        Some(self.lang_identity(p))
    }

    /// The identity of a language package that is named by an external target
    /// but not installed (a Go module that `go.mod` requires and the module
    /// cache lacks), as [`Identities::language`] would report it for one of
    /// its files.
    pub fn external(&self, name: &str, fs: &DirCache) -> Option<LangIdentity> {
        let p = self.provider.as_ref()?.external(name, fs)?;
        Some(self.lang_identity(p))
    }

    fn lang_identity(&self, p: PackageIdentity) -> LangIdentity {
        let integrity = p
            .integrity
            .clone()
            .or_else(|| self.locks.integrity(p.ecosystem, &p.name, &p.version));
        LangIdentity {
            id: Identity {
                name: store_name(p.ecosystem, &p.name),
                version: p.version,
                integrity,
            },
            root: p.root,
            pins: p.pins,
        }
    }

    /// Names the repository root (absolute, real). The directories it lives
    /// below are no part of the tree, whatever they are called: a marker
    /// segment counts only in the part of a path below the last directory
    /// it shares with the root
    /// ([`turborepo_trace_core::DirCache::local_offset`]). Without it, a
    /// checkout below a directory called `node_modules` or `vendor` is an
    /// installed package, and every file of it third party.
    pub fn with_repo(mut self, repo: &Path) -> Self {
        self.repo = Some(repo.to_path_buf());
        self
    }

    /// Where the part of `path` starts that marker segments are looked for in.
    fn local_offset(&self, path: &Path) -> usize {
        self.repo
            .as_deref()
            .map_or(0, |r| turborepo_trace_core::fs::shared_prefix(r, path))
    }

    /// Whether `path` lies inside an installed package directory (any marker
    /// segment).
    pub fn is_third_party(&self, path: &Path) -> bool {
        let from = self.local_offset(path);
        self.boundaries.iter().any(|b| b.contains_from(path, from))
    }

    /// The package root of a path inside a package, and the name derived from
    /// the path.
    pub fn package_root(&self, path: &Path) -> Option<(PathBuf, String)> {
        let from = self.local_offset(path);
        self.boundaries
            .iter()
            .filter_map(|b| b.package_root_and_name_from(path, from))
            .max_by_key(|(r, _)| r.as_os_str().len())
    }

    /// The identity of the package rooted at `root` (a real path), reading its
    /// manifest through `fs`.
    pub fn of_root(&self, fs: &DirCache, root: &Path, path_name: &str) -> Identity {
        if let Some(Some(id)) = self
            .memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(root)
        {
            return id.clone();
        }
        let manifest = fs
            .read_cached(&root.join("package.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        let field = |k: &str| {
            manifest
                .as_ref()
                .and_then(|m| m.get(k)?.as_str().map(str::to_owned))
        };
        let name = field("name").unwrap_or_else(|| path_name.to_owned());
        let version = field("version").unwrap_or_else(|| "unknown".to_owned());
        let integrity = self.integrity(&name, &version);
        let id = Identity {
            name,
            version,
            integrity,
        };
        self.memo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(root.to_path_buf(), Some(id.clone()));
        id
    }

    /// The lockfile integrity of `name@version`: the first entry with an
    /// integrity, by lockfile id, so peer-dependency variants agree.
    pub fn integrity(&self, name: &str, version: &str) -> Option<String> {
        let lock = self.lock()?;
        let mut cands: Vec<_> = lock
            .packages_named(name)
            .filter(|p| p.version == version)
            .filter_map(|p| Some((p.id.as_str(), p.integrity.as_deref()?)))
            .collect();
        cands.sort_unstable();
        cands.first().map(|(_, i)| (*i).to_owned())
    }

    /// What dependency `name` of the workspace package at `importer`
    /// (repo-relative, `""` for the root) resolves to according to the
    /// lockfile, falling back to the root importer (Node's walk-up reaches
    /// the root `node_modules`). Used when nothing is installed: the
    /// package node is then keyed by the lockfile entry alone.
    pub fn lock_resolve(&self, importer: &str, name: &str) -> Option<LockTarget> {
        let lock = self.lock()?;
        let imp = if importer.is_empty() { "." } else { importer };
        let r = lock
            .resolve(imp, name)
            .or_else(|| lock.resolve(".", name))?;
        match r {
            turborepo_trace_lockfile::Resolved::Package(p) => Some(LockTarget::Package(Identity {
                name: p.name.clone(),
                version: p.version.clone(),
                integrity: p.integrity.clone(),
            })),
            turborepo_trace_lockfile::Resolved::Importer(i) => {
                Some(LockTarget::Workspace(if i.path == "." {
                    String::new()
                } else {
                    i.path.clone()
                }))
            }
            turborepo_trace_lockfile::Resolved::Unresolved(_) => None,
        }
    }

    /// Forgets memoized identities (after the filesystem changed).
    pub fn clear(&self) {
        self.memo.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Loads the lockfiles of every ecosystem: the JS lockfile at `root`
/// ([`load_lockfile`]) and, among the walked repo-relative `files`, every
/// `Cargo.lock`, `go.mod` + `go.sum`, `uv.lock`, `poetry.lock`, `Pipfile.lock`
/// and `requirements*.txt` (each with its own parser). Returns the JS lockfile,
/// the others, and `(repo-relative path, xxh3-128 hex)` for every lockfile
/// present, sorted.
pub fn load_lockfiles(
    root: &Path,
    files: &[String],
) -> (Option<Lockfile>, Locks, Vec<(String, String)>) {
    load_lockfiles_inner(root, files, true)
}

fn load_lockfiles_inner(
    root: &Path,
    files: &[String],
    parse_js: bool,
) -> (Option<Lockfile>, Locks, Vec<(String, String)>) {
    use turborepo_trace_lockfile::{LockfileKind, gosum, parse};
    let (lock, mut hashes) = if parse_js {
        load_lockfile(root)
    } else {
        (None, lockfile_hashes(root))
    };
    let mut locks = Locks::default();
    let hash = |b: &[u8]| format!("{:032x}", xxhash_rust::xxh3::xxh3_128(b));
    for f in files {
        let name = f.rsplit('/').next().unwrap_or(f);
        let (eco, kind) = match name {
            "Cargo.lock" => ("crates", LockfileKind::Cargo),
            "go.sum" => ("go", LockfileKind::GoSum),
            _ => continue,
        };
        let Ok(bytes) = std::fs::read(root.join(f)) else {
            continue;
        };
        hashes.push((f.clone(), hash(&bytes)));
        let text = String::from_utf8_lossy(&bytes);
        let parsed = if kind == LockfileKind::GoSum {
            // `go.sum` hashes are keyed through the module's `go.mod` requirements.
            let go_mod = f.strip_suffix("go.sum").map(|d| format!("{d}go.mod"));
            match go_mod.and_then(|m| std::fs::read_to_string(root.join(m)).ok()) {
                Some(m) => gosum::parse(&m, Some(&text)),
                None => parse(kind, &text),
            }
        } else {
            parse(kind, &text)
        };
        if let Ok(l) = parsed {
            locks.locks.push((eco, l));
        }
    }
    hashes.sort();
    hashes.dedup();
    (lock, locks, hashes)
}

/// [`load_lockfiles`] without parsing the JS lockfile: returns the root and a
/// loader for it (the first of [`LOCKFILES`] that parses, as [`load_lockfile`])
/// instead.
pub fn load_lockfiles_deferred(
    root: &Path,
    files: &[String],
) -> (
    impl Fn() -> Option<Lockfile> + Send + Sync + 'static,
    Locks,
    Vec<(String, String)>,
) {
    let (_, locks, hashes) = load_lockfiles_inner(root, files, false);
    let root = root.to_path_buf();
    (move || load_lockfile(&root).0, locks, hashes)
}

/// `(file name, xxh3-128 hex)` of every JS lockfile present at `root`.
fn lockfile_hashes(root: &Path) -> Vec<(String, String)> {
    LOCKFILES
        .iter()
        .filter_map(|name| {
            let bytes = std::fs::read(root.join(name)).ok()?;
            Some((
                name.to_string(),
                format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&bytes)),
            ))
        })
        .collect()
}

/// Finds and parses the repository lockfile. Returns the parsed lockfile (if
/// any parses) and `(file name, xxh3-128 hex)` for every lockfile present.
pub fn load_lockfile(root: &Path) -> (Option<Lockfile>, Vec<(String, String)>) {
    let mut hashes = Vec::new();
    let mut lock = None;
    for name in LOCKFILES {
        let p = root.join(name);
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        hashes.push((
            name.to_string(),
            format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&bytes)),
        ));
        if lock.is_none()
            && let Ok(text) = String::from_utf8(bytes)
            && let Ok(l) = turborepo_trace_lockfile::parse_named(name, &text)
        {
            lock = Some(l);
        }
    }
    (lock, hashes)
}
