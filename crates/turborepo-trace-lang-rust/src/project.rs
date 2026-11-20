//! The cargo project model behind the resolver and the `cargo test` adapter:
//! packages and their targets, workspaces, `Cargo.lock`, where dependency
//! crates live, and an approximation of cargo's feature resolution. No `cargo
//! metadata`: everything is read from `Cargo.toml`, `Cargo.lock` and
//! `.cargo/config.toml` through the shared [`DirCache`].
//!
//! Dependency crates are located in this order:
//!
//! 1. path dependencies (`path = "…"`, also through
//!    `[workspace.dependencies]`);
//! 2. git dependencies in `$CARGO_HOME/git/checkouts/<repo>-<hash>/<short
//!    rev>/`;
//! 3. `vendor/`-style directory sources configured through `[source]`
//!    replacement in `.cargo/config.toml` (for crates.io and for git sources);
//! 4. `$CARGO_HOME/registry/src/<index>/<name>-<version>/`.
//!
//! Features: starting from every workspace member with its `default` feature,
//! enabled features and optional dependencies are propagated to a fixpoint
//! (`dep:x`, `x/y`, `x?/y`, implicit optional-dependency features,
//! `default-features = false`). Platform-specific dependencies are filtered by
//! the target ([`crate::cfg`]); unknown predicates keep them. Simplifications:
//! features of build and dev dependencies are unified with normal ones (as
//! cargo's resolver 1), and dev-dependencies of members are always active.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use turborepo_trace_core::{DirCache, Memo, Platform, fs::normalize, record};
use turborepo_trace_lockfile::cargo::{CargoLock, CargoPackage, parse_raw};

use crate::{
    BOUNDARY,
    cfg::{CfgEnv, Tri, target_key_matches},
    manifest::{CrateTarget, Dep, DepKind, Manifest, TargetKind, member_glob_matches},
};

/// A package (a directory with a `Cargo.toml` that has `[package]`).
#[derive(Debug)]
pub struct Package {
    /// The package directory.
    pub dir: PathBuf,
    /// Its manifest, with workspace inheritance applied.
    pub manifest: Arc<Manifest>,
    /// Its targets.
    pub targets: Vec<CrateTarget>,
    /// Whether it lives in the registry, a git checkout or a vendor directory.
    pub third_party: bool,
}

impl Package {
    /// `Cargo.toml`.
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join("Cargo.toml")
    }

    /// The library target.
    pub fn lib(&self) -> Option<&CrateTarget> {
        self.targets.iter().find(|t| t.kind == TargetKind::Lib)
    }

    /// The build script target.
    pub fn build_script(&self) -> Option<&CrateTarget> {
        self.targets.iter().find(|t| t.kind == TargetKind::Build)
    }

    /// The target whose crate root is `file`.
    pub fn target_of(&self, file: &Path) -> Option<&CrateTarget> {
        self.targets.iter().find(|t| t.root == file)
    }
}

/// A third-party crate's identity, from `Cargo.lock` (an opaque
/// package).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CrateIdentity {
    /// Package name.
    pub name: String,
    /// Version.
    pub version: String,
    /// `source` (registry or git URL with commit).
    pub source: Option<String>,
    /// `checksum` (registry crates).
    pub checksum: Option<String>,
}

/// A dependency of one crate target, as resolved.
#[derive(Clone, Debug)]
pub struct ResolvedDep {
    /// The dependency key.
    pub key: String,
    /// Its kind.
    pub kind: DepKind,
    /// The package, or why it could not be located.
    pub target: Result<Arc<Package>, String>,
    /// Identity from `Cargo.lock`, for third-party crates.
    pub identity: Option<CrateIdentity>,
}

/// Source replacement from `.cargo/config.toml`.
#[derive(Clone, Debug, Default)]
pub struct CargoConfig {
    /// Config files found (inputs).
    pub files: Vec<PathBuf>,
    /// Directory replacing crates.io (`[source.crates-io] replace-with = …` →
    /// `directory = …`).
    pub crates_io_dir: Option<PathBuf>,
    /// Directories replacing git sources, by repository URL.
    pub git_dirs: Vec<(String, PathBuf)>,
}

/// A workspace: root, members, lockfile and config.
#[derive(Debug)]
pub struct Workspace {
    /// The workspace root directory.
    pub root: PathBuf,
    /// Member package directories.
    pub members: Vec<PathBuf>,
    /// `Cargo.lock`, if present.
    pub lock_path: Option<PathBuf>,
    /// The parsed lockfile.
    pub lock: Option<CargoLock>,
    /// Source replacement.
    pub config: CargoConfig,
    graph: OnceLock<Graph>,
}

/// Feature-resolved crate graph of one workspace.
#[derive(Debug, Default)]
struct Graph {
    nodes: Vec<Node>,
    by_dir: HashMap<PathBuf, usize>,
}

#[derive(Debug)]
struct Node {
    pkg: Arc<Package>,
    lock_idx: Option<usize>,
    member: bool,
    /// Per manifest dependency: the resolved node, an error, or `None` if
    /// filtered out by the target platform.
    deps: Vec<Option<Result<usize, String>>>,
    /// Per manifest dependency that could not be located: its `Cargo.lock`
    /// entry (a crate that is locked but not downloaded).
    missing: Vec<Option<usize>>,
    active: Vec<bool>,
    features: BTreeSet<String>,
    reached: bool,
}

/// Which features of the workspace members are enabled (cargo's `--features`,
/// `--all-features`, `--no-default-features`), applied to every member.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FeatureSelection {
    /// Extra features: `name` (a feature or optional dependency of a member) or
    /// `pkg/feature` (a feature of member `pkg`, or of dependency `pkg` of
    /// every member).
    pub features: Vec<String>,
    /// Every feature and optional dependency of every member.
    pub all_features: bool,
    /// Do not enable members' `default` features.
    pub no_default_features: bool,
}

/// The cargo project model. Cheap to share; all lookups are memoized.
#[derive(Debug)]
pub struct CargoProject {
    fs: Arc<DirCache>,
    platform: Platform,
    cargo_home: PathBuf,
    selection: FeatureSelection,
    all_targets: bool,
    manifests: Memo<PathBuf, Option<Arc<Manifest>>>,
    packages: Memo<PathBuf, Option<Arc<Package>>>,
    workspaces: Mutex<HashMap<PathBuf, Arc<Workspace>>>,
}

/// `$CARGO_HOME`, else `$HOME/.cargo`.
pub fn default_cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")))
        .unwrap_or_else(|| PathBuf::from("/nonexistent/.cargo"))
}

impl CargoProject {
    /// A project model reading through `fs`, targeting `platform`.
    pub fn new(fs: Arc<DirCache>, platform: Platform, cargo_home: PathBuf) -> Self {
        CargoProject {
            fs,
            platform,
            cargo_home: normalize(&cargo_home),
            selection: FeatureSelection::default(),
            all_targets: false,
            manifests: Memo::new(),
            packages: Memo::new(),
            workspaces: Mutex::default(),
        }
    }

    /// The same model with an explicit feature selection.
    pub fn with_features(mut self, selection: FeatureSelection) -> Self {
        self.selection = selection;
        self
    }

    /// The same model keeping dependencies for every target (`--all-targets`).
    pub fn with_all_targets(mut self, yes: bool) -> Self {
        self.all_targets = yes;
        self
    }

    /// The feature selection.
    pub fn feature_selection(&self) -> &FeatureSelection {
        &self.selection
    }

    /// The fs cache.
    pub fn fs(&self) -> &DirCache {
        &self.fs
    }

    /// `$CARGO_HOME`.
    pub fn cargo_home(&self) -> &Path {
        &self.cargo_home
    }

    /// The target platform.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Drops all memoized state.
    pub fn clear(&self) {
        self.manifests.clear();
        self.packages.clear();
        self.workspaces
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn memo<T: Clone>(&self, map: &Memo<PathBuf, T>, key: &Path, compute: impl FnOnce() -> T) -> T {
        map.get(&self.fs, &key.to_path_buf(), compute)
    }

    /// The raw manifest in `dir` (no inheritance), if `dir/Cargo.toml` exists
    /// and parses.
    pub fn raw_manifest(&self, dir: &Path) -> Option<Arc<Manifest>> {
        // A manifest above the repository is the machine's: the searches for a package
        // and for a workspace root go up from a crate, and what the checkout happens to
        // live in (another checkout's workspace, a home directory) is no part of the
        // tree.
        if self.fs.is_above_base(dir) {
            return None;
        }
        let path = dir.join("Cargo.toml");
        if !self.fs.is_file(&path) {
            return None;
        }
        self.memo(&self.manifests, &path, || {
            let bytes = self.fs.read_cached(&path).ok()?;
            let text = std::str::from_utf8(&bytes).ok()?;
            Manifest::parse(dir, text).ok().map(Arc::new)
        })
    }

    /// The package whose directory is exactly `dir`.
    pub fn package_at(&self, dir: &Path) -> Option<Arc<Package>> {
        let key = dir.join("Cargo.toml#pkg");
        self.memo(&self.packages, &key, || {
            let raw = self.raw_manifest(dir)?;
            if !raw.is_package() {
                return None;
            }
            let third_party =
                BOUNDARY.contains_from(dir, self.fs.local_offset(dir)) || self.is_vendored(dir);
            let mut m = (*raw).clone();
            if !m.inherited.is_empty()
                && !third_party
                && let Some(root) = self.workspace_root_of(dir, &raw)
                && let Some(ws) = self.raw_manifest(&root)
            {
                m.inherit(&ws);
            }
            let targets = m.targets(&self.fs);
            Some(Arc::new(Package {
                dir: dir.to_path_buf(),
                manifest: Arc::new(m),
                targets,
                third_party,
            }))
        })
    }

    fn is_vendored(&self, dir: &Path) -> bool {
        // A `vendor` directory of the repository: one the checkout itself lives in is
        // not.
        dir.parent().is_some_and(|p| {
            p.file_name().is_some_and(|n| n == "vendor") && !self.fs.is_above_base(p)
        })
    }

    /// The package containing `dir`: the nearest ancestor (or `dir` itself)
    /// with a package manifest. Stops at package boundaries: inside the
    /// registry it never looks above the crate directory.
    pub fn package_of(&self, dir: &Path) -> Option<Arc<Package>> {
        let inside = dir.join("x");
        let stop = BOUNDARY
            .package_root_and_name_from(&inside, self.fs.local_offset(&inside))
            .map(|(root, _)| root);
        self.memo(&self.packages, dir, || {
            let mut d = Some(dir);
            while let Some(cur) = d {
                if let Some(p) = self.package_at(cur) {
                    return Some(p);
                }
                if stop.as_deref() == Some(cur) {
                    return None;
                }
                // A virtual workspace manifest ends the search.
                if self
                    .raw_manifest(cur)
                    .is_some_and(|m| m.workspace.is_some())
                {
                    return None;
                }
                d = cur.parent();
            }
            None
        })
    }

    /// Whether `file` is the crate root of some target of its package.
    pub fn is_crate_root(&self, file: &Path) -> bool {
        file.parent()
            .and_then(|d| self.package_of(d))
            .is_some_and(|p| p.target_of(file).is_some())
    }

    fn workspace_root_of(&self, dir: &Path, m: &Manifest) -> Option<PathBuf> {
        if let Some(rel) = &m.workspace_path {
            return Some(normalize(&dir.join(rel)));
        }
        if m.workspace.is_some() {
            return Some(dir.to_path_buf());
        }
        if BOUNDARY.contains_from(dir, self.fs.local_offset(dir)) {
            return None;
        }
        let mut d = dir.parent();
        while let Some(cur) = d {
            if let Some(wm) = self.raw_manifest(cur)
                && let Some(w) = &wm.workspace
            {
                let rel = dir
                    .strip_prefix(cur)
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let excluded = w.exclude.iter().any(|e| {
                    rel == e.trim_end_matches('/')
                        || rel.starts_with(&format!("{}/", e.trim_end_matches('/')))
                });
                let member = w.members.iter().any(|g| member_glob_matches(g, &rel));
                return (member && !excluded).then(|| cur.to_path_buf());
            }
            d = cur.parent();
        }
        None
    }

    /// The workspace `pkg` belongs to (its own directory when standalone).
    /// Third-party crates belong to the workspace that depends on them; for
    /// those, prefer [`CargoProject::identity`].
    pub fn workspace_of(&self, pkg: &Package) -> Arc<Workspace> {
        let root = self
            .workspace_root_of(&pkg.dir, &pkg.manifest)
            .unwrap_or_else(|| pkg.dir.clone());
        self.workspace(&root)
    }

    /// The workspace rooted at `root` (loaded once).
    pub fn workspace(&self, root: &Path) -> Arc<Workspace> {
        if let Some(w) = self
            .workspaces
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(root)
        {
            return Arc::clone(w);
        }
        let ws = record::pause(|| self.load_workspace(root));
        let ws = Arc::new(ws);
        Arc::clone(
            self.workspaces
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(root.to_path_buf())
                .or_insert(ws),
        )
    }

    fn load_workspace(&self, root: &Path) -> Workspace {
        let mut members = Vec::new();
        if let Some(m) = self.raw_manifest(root) {
            if m.is_package() {
                members.push(root.to_path_buf());
            }
            if let Some(w) = &m.workspace {
                for pat in &w.members {
                    for dir in self.glob_dirs(root, pat) {
                        let rel = dir
                            .strip_prefix(root)
                            .map(|r| r.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let excluded = w.exclude.iter().any(|e| rel == e.trim_end_matches('/'));
                        if !excluded
                            && self.raw_manifest(&dir).is_some_and(|m| m.is_package())
                            && !members.contains(&dir)
                        {
                            members.push(dir);
                        }
                    }
                }
            }
        }
        members.sort();
        let lock_path = Some(root.join("Cargo.lock")).filter(|p| self.fs.is_file(p));
        let lock = lock_path.as_ref().and_then(|p| {
            let bytes = self.fs.read_cached(p).ok()?;
            parse_raw(std::str::from_utf8(&bytes).ok()?).ok()
        });
        Workspace {
            root: root.to_path_buf(),
            members,
            lock_path,
            lock,
            config: self.load_config(root),
            graph: OnceLock::new(),
        }
    }

    /// Directories matching a `members` glob (segment-wise `*`/`?`).
    fn glob_dirs(&self, root: &Path, pat: &str) -> Vec<PathBuf> {
        let mut cur = vec![root.to_path_buf()];
        for seg in pat.split('/').filter(|s| !s.is_empty() && *s != ".") {
            let mut next = Vec::new();
            for d in &cur {
                if seg.contains(['*', '?', '[']) {
                    if let Some(l) = self.fs.list(d) {
                        for e in l.iter() {
                            let name = String::from_utf8_lossy(e.name_bytes());
                            let p = d.join(e.name());
                            if member_glob_matches(seg, &name) && self.fs.is_dir(&p) {
                                next.push(p);
                            }
                        }
                    }
                } else {
                    let p = normalize(&d.join(seg));
                    if self.fs.is_dir(&p) {
                        next.push(p);
                    }
                }
            }
            cur = next;
        }
        cur
    }

    fn load_config(&self, root: &Path) -> CargoConfig {
        let mut cfg = CargoConfig::default();
        let mut files = Vec::new();
        let mut d = Some(root);
        while let Some(cur) = d {
            // Up to the repository root: a `.cargo/config.toml` above it is the machine's
            // (cargo's own home directory is asked below).
            if self.fs.is_above_base(cur) {
                break;
            }
            for name in [".cargo/config.toml", ".cargo/config"] {
                let p = cur.join(name);
                if self.fs.is_file(&p) {
                    files.push(p);
                    break;
                }
            }
            d = cur.parent();
        }
        let home = self.cargo_home.join("config.toml");
        if self.fs.is_file(&home) && !files.contains(&home) {
            files.push(home);
        }
        // Nearer files take precedence: merge `source` tables from the farthest first.
        let mut sources: BTreeMap<String, (toml::Table, PathBuf)> = BTreeMap::new();
        for f in files.iter().rev() {
            let Ok(bytes) = self.fs.read_cached(f) else {
                continue;
            };
            let Ok(t) = toml::from_str::<toml::Table>(&String::from_utf8_lossy(&bytes)) else {
                continue;
            };
            // Paths are relative to the directory containing `.cargo/`.
            let base = f
                .parent()
                .and_then(Path::parent)
                .unwrap_or(Path::new("/"))
                .to_path_buf();
            if let Some(src) = t.get("source").and_then(toml::Value::as_table) {
                for (k, v) in src {
                    if let Some(v) = v.as_table() {
                        sources.insert(k.clone(), (v.clone(), base.clone()));
                    }
                }
            }
        }
        let dir_of = |start: &str| -> Option<PathBuf> {
            let mut name = start.to_owned();
            for _ in 0..8 {
                let (t, base) = sources.get(&name)?;
                if let Some(d) = t.get("directory").and_then(toml::Value::as_str) {
                    return Some(normalize(&base.join(d)));
                }
                name = t.get("replace-with")?.as_str()?.to_owned();
            }
            None
        };
        cfg.crates_io_dir = sources
            .get("crates-io")
            .and_then(|(t, _)| t.get("replace-with"))
            .and_then(|_| dir_of("crates-io"));
        for (name, (t, _)) in &sources {
            if let Some(url) = t.get("git").and_then(toml::Value::as_str)
                && t.get("replace-with").is_some()
                && let Some(d) = dir_of(name)
            {
                cfg.git_dirs
                    .push((url.trim_end_matches(".git").to_owned(), d));
            }
        }
        cfg.files = files;
        cfg
    }

    // ---- locating crates ----

    fn locate(&self, ws: &Workspace, p: &CargoPackage) -> Result<PathBuf, String> {
        let src = p.source.as_deref().unwrap_or("");
        let missing = |where_: &str| {
            format!(
                "crate {} {} not found in {where_}; run `cargo fetch` (or `cargo vendor`)",
                p.name, p.version
            )
        };
        if let Some(git) = src.strip_prefix("git+") {
            let (url, rev) = git.split_once('#').unwrap_or((git, ""));
            let url_base = url
                .split('?')
                .next()
                .unwrap_or(url)
                .trim_end_matches(".git");
            if let Some((_, dir)) = ws.config.git_dirs.iter().find(|(u, _)| u == url_base)
                && let Some(d) = self.vendored(dir, p)
            {
                return Ok(d);
            }
            return self
                .git_checkout(url_base, rev, &p.name)
                .ok_or_else(|| missing("$CARGO_HOME/git/checkouts"));
        }
        if let Some(dir) = &ws.config.crates_io_dir {
            return self
                .vendored(dir, p)
                .ok_or_else(|| missing(&dir.display().to_string()));
        }
        self.registry_dir(&p.name, &p.version)
            .ok_or_else(|| missing("$CARGO_HOME/registry/src"))
    }

    fn vendored(&self, dir: &Path, p: &CargoPackage) -> Option<PathBuf> {
        let versioned = dir.join(format!("{}-{}", p.name, p.version));
        if self.fs.is_file(&versioned.join("Cargo.toml")) {
            return Some(versioned);
        }
        let plain = dir.join(&p.name);
        let m = self.raw_manifest(&plain)?;
        (m.version.as_deref().is_none_or(|v| v == p.version)).then_some(plain)
    }

    /// `$CARGO_HOME/registry/src/<index>/<name>-<version>`, trying every index
    /// directory.
    pub fn registry_dir(&self, name: &str, version: &str) -> Option<PathBuf> {
        let src = self.cargo_home.join("registry/src");
        let listing = record::pause_listings(|| self.fs.list(&src))?;
        let leaf = format!("{name}-{version}");
        listing.iter().find_map(|e| {
            let d = src.join(e.name()).join(&leaf);
            self.fs.is_file(&d.join("Cargo.toml")).then_some(d)
        })
    }

    fn git_checkout(&self, url: &str, rev: &str, name: &str) -> Option<PathBuf> {
        let repo = url.rsplit('/').next().unwrap_or(url);
        let co = self.cargo_home.join("git/checkouts");
        let listing = record::pause_listings(|| self.fs.list(&co))?;
        let short = &rev[..rev.len().min(7)];
        for e in listing.iter() {
            let n = String::from_utf8_lossy(e.name_bytes());
            let Some(rest) = n.strip_prefix(repo).and_then(|r| r.strip_prefix('-')) else {
                continue;
            };
            if rest.contains('-') {
                continue;
            }
            let root = co.join(e.name()).join(short);
            if !self.fs.is_dir(&root) {
                continue;
            }
            if let Some(d) = self.find_package_in(&root, name, 3) {
                return Some(d);
            }
        }
        None
    }

    /// A package named `name` at or below `root` (workspace members first, then
    /// a shallow walk).
    fn find_package_in(&self, root: &Path, name: &str, depth: usize) -> Option<PathBuf> {
        let m = self.raw_manifest(root);
        if m.as_ref().and_then(|m| m.name.as_deref()) == Some(name) {
            return Some(root.to_path_buf());
        }
        if let Some(w) = m.as_ref().and_then(|m| m.workspace.as_ref()) {
            for pat in &w.members {
                for d in self.glob_dirs(root, pat) {
                    if self
                        .raw_manifest(&d)
                        .and_then(|m| m.name.clone())
                        .as_deref()
                        == Some(name)
                    {
                        return Some(d);
                    }
                }
            }
        }
        if depth == 0 {
            return None;
        }
        let listing = record::pause_listings(|| self.fs.list(root))?;
        listing.iter().find_map(|e| {
            let n = e.name_bytes();
            if n.starts_with(b".") || n == b"target" {
                return None;
            }
            let d = root.join(e.name());
            if self.fs.is_dir(&d) {
                self.find_package_in(&d, name, depth - 1)
            } else {
                None
            }
        })
    }

    // ---- graph ----

    fn graph<'w>(&self, ws: &'w Workspace) -> &'w Graph {
        ws.graph
            .get_or_init(|| record::pause(|| self.build_graph(ws)))
    }

    fn build_graph(&self, ws: &Workspace) -> Graph {
        let mut g = Graph::default();
        let lock = ws.lock.as_ref();
        let env = CfgEnv::new(self.platform).with_all_targets(self.all_targets);
        // Nodes, discovered breadth-first.
        let mut queue: Vec<usize> = Vec::new();
        for m in &ws.members {
            if let Some(pkg) = self.package_at(m) {
                let idx = add_node(&mut g, pkg, lock, true);
                queue.push(idx);
            }
        }
        let mut qi = 0;
        while qi < queue.len() {
            let n = queue[qi];
            qi += 1;
            let pkg = Arc::clone(&g.nodes[n].pkg);
            let member = g.nodes[n].member;
            let lock_idx = g.nodes[n].lock_idx;
            let mut deps = Vec::with_capacity(pkg.manifest.deps.len());
            let mut missing = Vec::with_capacity(pkg.manifest.deps.len());
            for d in &pkg.manifest.deps {
                if (d.kind == DepKind::Dev && !member)
                    || d.target
                        .as_deref()
                        .is_some_and(|t| target_key_matches(t, &env) == Tri::False)
                {
                    deps.push(None);
                    missing.push(None);
                    continue;
                }
                let r = self
                    .resolve_dep(ws, lock, lock_idx, d)
                    .and_then(|(dir, li)| {
                        if let Some(&i) = g.by_dir.get(&dir) {
                            return Ok(i);
                        }
                        let pkg = self
                            .package_at(&dir)
                            .ok_or_else(|| format!("no package manifest in {}", dir.display()))?;
                        let i = add_node(&mut g, pkg, lock, false);
                        if g.nodes[i].lock_idx.is_none() {
                            g.nodes[i].lock_idx = li;
                        }
                        queue.push(i);
                        Ok(i)
                    });
                // Locked but not downloaded: the lock entry still says which crate it is.
                missing.push(match (&r, lock) {
                    (Err(_), Some(l)) if d.path.is_none() => lock_idx
                        .and_then(|i| find_dep_entry(l, i, &d.package))
                        .filter(|&j| l.packages[j].source.is_some()),
                    _ => None,
                });
                deps.push(Some(r));
            }
            g.nodes[n].active = vec![false; deps.len()];
            g.nodes[n].deps = deps;
            g.nodes[n].missing = missing;
        }
        resolve_features(&mut g, &self.selection);
        g
    }

    /// Locates dependency `d` of the package with lock entry `lock_idx`.
    fn resolve_dep(
        &self,
        ws: &Workspace,
        lock: Option<&CargoLock>,
        lock_idx: Option<usize>,
        d: &Dep,
    ) -> Result<(PathBuf, Option<usize>), String> {
        if let Some(p) = &d.path {
            let dir = normalize(&d.path_base.join(p));
            let li = lock.and_then(|l| {
                lock_idx
                    .and_then(|i| find_dep_entry(l, i, &d.package))
                    .filter(|&j| l.packages[j].source.is_none())
            });
            return Ok((dir, li));
        }
        let lock = lock.ok_or_else(|| {
            format!(
                "dependency {} cannot be located without a Cargo.lock",
                d.package
            )
        })?;
        let li = lock_idx
            .and_then(|i| find_dep_entry(lock, i, &d.package))
            .ok_or_else(|| format!("dependency {} is not in Cargo.lock", d.package))?;
        let p = &lock.packages[li];
        if p.source.is_none() {
            return Err(format!("path dependency {} has no path", d.package));
        }
        self.locate(ws, p).map(|dir| (dir, Some(li)))
    }

    /// The node of the package in `dir`, searching every loaded workspace graph
    /// (and the workspace of `dir` itself).
    fn node_of(&self, dir: &Path) -> Option<(Arc<Workspace>, usize)> {
        if let Some(pkg) = self.package_at(dir)
            && !pkg.third_party
        {
            let ws = self.workspace_of(&pkg);
            if let Some(&i) = self.graph(&ws).by_dir.get(dir) {
                return Some((ws, i));
            }
        }
        let all: Vec<Arc<Workspace>> = self
            .workspaces
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for ws in all {
            if let Some(&i) = self.graph(&ws).by_dir.get(dir) {
                return Some((ws, i));
            }
        }
        None
    }

    /// The dependencies linked into target kind `kind` of the package in
    /// `pkg_dir`, after feature resolution, sorted by key.
    pub fn crate_deps(&self, pkg_dir: &Path, kind: TargetKind) -> Vec<ResolvedDep> {
        let Some((ws, n)) = self.node_of(pkg_dir) else {
            return Vec::new();
        };
        let g = self.graph(&ws);
        let node = &g.nodes[n];
        let mut out = Vec::new();
        for (i, d) in node.pkg.manifest.deps.iter().enumerate() {
            let wanted = match kind {
                TargetKind::Build => d.kind == DepKind::Build,
                k if k.uses_dev_deps() => matches!(d.kind, DepKind::Normal | DepKind::Dev),
                _ => d.kind == DepKind::Normal,
            };
            if !wanted || !node.active.get(i).copied().unwrap_or(false) {
                continue;
            }
            let Some(Some(r)) = node.deps.get(i) else {
                continue;
            };
            let target = r.clone().map(|t| Arc::clone(&g.nodes[t].pkg));
            let identity = match r {
                Ok(t) => {
                    let tn = &g.nodes[*t];
                    if tn.pkg.third_party {
                        ws.lock
                            .as_ref()
                            .zip(tn.lock_idx)
                            .and_then(|(l, i)| identity_of(l, i))
                    } else {
                        None
                    }
                }
                // Not downloaded: the lock entry says which crate it is; without a lock
                // entry a registry or git dependency is whatever its requirement admits.
                Err(_) => ws
                    .lock
                    .as_ref()
                    .zip(node.missing.get(i).copied().flatten())
                    .and_then(|(l, j)| identity_of(l, j))
                    .or_else(|| {
                        d.path.is_none().then(|| CrateIdentity {
                            name: d.package.clone(),
                            version: "*".to_owned(),
                            source: None,
                            checksum: None,
                        })
                    }),
            };
            out.push(ResolvedDep {
                key: d.key.clone(),
                kind: d.kind,
                target,
                identity,
            });
        }
        out.sort_by(|a, b| a.key.cmp(&b.key).then(a.kind.cmp(&b.kind)));
        out
    }

    /// A crate that a loaded workspace locks as `name` `version` from a
    /// registry or a git repository: its identity, the `Cargo.lock` that
    /// pins it and the directory its sources would have in `$CARGO_HOME`.
    /// For crates that are locked but not downloaded.
    pub fn locked_crate(
        &self,
        name: &str,
        version: &str,
    ) -> Option<(CrateIdentity, PathBuf, PathBuf)> {
        let all: Vec<Arc<Workspace>> = self
            .workspaces
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        let mut found: Vec<(CrateIdentity, PathBuf, PathBuf)> = Vec::new();
        for ws in all {
            let (Some(lock), Some(path)) = (ws.lock.as_ref(), ws.lock_path.as_ref()) else {
                continue;
            };
            let Some(j) = lock
                .packages
                .iter()
                .position(|p| p.name == name && p.version == version && p.source.is_some())
            else {
                continue;
            };
            let Some(id) = identity_of(lock, j) else {
                continue;
            };
            let git = id.source.as_deref().is_some_and(|s| s.starts_with("git+"));
            let root = if git {
                self.cargo_home.join("git/checkouts").join(name)
            } else {
                self.cargo_home
                    .join("registry/src/index.crates.io")
                    .join(format!("{name}-{version}"))
            };
            found.push((id, path.clone(), root));
        }
        // Deterministic when several workspaces lock the crate.
        found.sort_by(|a, b| a.1.cmp(&b.1));
        found.into_iter().next()
    }

    /// Enabled features of the package in `pkg_dir`, if it is part of a loaded
    /// graph.
    pub fn features(&self, pkg_dir: &Path) -> Option<BTreeSet<String>> {
        let (ws, n) = self.node_of(pkg_dir)?;
        Some(self.graph(&ws).nodes[n].features.clone())
    }

    /// The identity of the third-party crate containing `path` (registry, git
    /// checkout or vendor), from the lockfile of a loaded workspace that
    /// depends on it.
    pub fn identity(&self, path: &Path) -> Option<CrateIdentity> {
        let dir = if self.fs.is_dir(path) {
            path
        } else {
            path.parent()?
        };
        let pkg = self.package_of(dir)?;
        if !pkg.third_party {
            return None;
        }
        let (ws, n) = self.node_of(&pkg.dir)?;
        let lock = ws.lock.as_ref()?;
        identity_of(lock, self.graph(&ws).nodes[n].lock_idx?)
    }

    /// [`CargoProject::identity`] plus the crate's package directory (the
    /// containing package, also inside a git checkout that holds a whole
    /// workspace) and the `Cargo.lock` that pins it.
    pub fn identity_with_root(&self, path: &Path) -> Option<(CrateIdentity, PathBuf, PathBuf)> {
        let dir = if self.fs.is_dir(path) {
            path
        } else {
            path.parent()?
        };
        let pkg = self.package_of(dir)?;
        if !pkg.third_party {
            return None;
        }
        let (ws, n) = self.node_of(&pkg.dir)?;
        let lock = ws.lock.as_ref()?;
        let id = identity_of(lock, self.graph(&ws).nodes[n].lock_idx?)?;
        Some((id, pkg.dir.clone(), ws.lock_path.clone()?))
    }

    /// Every third-party crate in the workspace graph of `pkg_dir`, with its
    /// location.
    pub fn third_party(&self, pkg_dir: &Path) -> Vec<(CrateIdentity, PathBuf)> {
        let Some((ws, _)) = self.node_of(pkg_dir) else {
            return Vec::new();
        };
        let Some(lock) = ws.lock.as_ref() else {
            return Vec::new();
        };
        let mut out: Vec<_> = self
            .graph(&ws)
            .nodes
            .iter()
            .filter(|n| n.pkg.third_party && n.reached)
            .filter_map(|n| Some((identity_of(lock, n.lock_idx?)?, n.pkg.dir.clone())))
            .collect();
        out.sort();
        out
    }
}

fn identity_of(lock: &CargoLock, i: usize) -> Option<CrateIdentity> {
    let p = lock.packages.get(i)?;
    Some(CrateIdentity {
        name: p.name.clone(),
        version: p.version.clone(),
        source: p.source.clone(),
        checksum: p.checksum.clone(),
    })
}

fn add_node(g: &mut Graph, pkg: Arc<Package>, lock: Option<&CargoLock>, member: bool) -> usize {
    let lock_idx = lock.and_then(|l| {
        let name = pkg.manifest.name.as_deref()?;
        let ver = pkg.manifest.version.as_deref();
        l.packages
            .iter()
            .position(|p| {
                p.name == name && p.source.is_none() && ver.is_none_or(|v| v == p.version)
            })
            .or_else(|| {
                pkg.third_party
                    .then(|| {
                        l.packages
                            .iter()
                            .position(|p| p.name == name && ver.is_none_or(|v| v == p.version))
                    })
                    .flatten()
            })
    });
    let i = g.nodes.len();
    g.by_dir.insert(pkg.dir.clone(), i);
    g.nodes.push(Node {
        pkg,
        lock_idx,
        member,
        deps: Vec::new(),
        missing: Vec::new(),
        active: Vec::new(),
        features: BTreeSet::new(),
        reached: false,
    });
    i
}

/// The lock package a `dependencies` entry of package `i` names for package
/// `name`.
fn find_dep_entry(lock: &CargoLock, i: usize, name: &str) -> Option<usize> {
    let p = lock.packages.get(i)?;
    p.dependencies.iter().find_map(|e| {
        let (head, source) = match e.split_once(" (") {
            Some((h, s)) => (h, Some(s.trim_end_matches(')'))),
            None => (e.as_str(), None),
        };
        let mut it = head.split_whitespace();
        if it.next()? != name {
            return None;
        }
        let version = it.next();
        lock.packages.iter().position(|q| {
            q.name == name
                && version.is_none_or(|v| q.version == v)
                && source.is_none_or(|s| q.source.as_deref() == Some(s))
        })
    })
}

enum Work {
    Reach(usize),
    Feature(usize, String),
    Dep(usize, usize),
}

/// Propagates features and optional dependencies to a fixpoint (see the module
/// docs).
fn resolve_features(g: &mut Graph, sel: &FeatureSelection) {
    let mut work: Vec<Work> = Vec::new();
    // `x/y` and `x?/y` requests: (node, dep key, feature, weak).
    let mut dep_feats: Vec<(usize, String, String, bool)> = Vec::new();
    let members: Vec<usize> = (0..g.nodes.len()).filter(|&i| g.nodes[i].member).collect();
    let member_named = |g: &Graph, name: &str| {
        members
            .iter()
            .copied()
            .find(|&i| g.nodes[i].pkg.manifest.name.as_deref() == Some(name))
    };
    for &i in &members {
        work.push(Work::Reach(i));
        if !sel.no_default_features {
            work.push(Work::Feature(i, "default".into()));
        }
        let m = Arc::clone(&g.nodes[i].pkg.manifest);
        if sel.all_features {
            work.extend(m.features.keys().map(|f| Work::Feature(i, f.clone())));
            work.extend(
                m.deps
                    .iter()
                    .filter(|d| d.optional)
                    .map(|d| Work::Feature(i, d.key.clone())),
            );
        }
        for f in &sel.features {
            match f.split_once('/') {
                Some((x, y)) => {
                    if let Some(t) = member_named(g, x) {
                        work.push(Work::Feature(t, y.to_owned()));
                    } else {
                        for (di, d) in m.deps.iter().enumerate() {
                            if d.key == x {
                                work.push(Work::Dep(i, di));
                            }
                        }
                        dep_feats.push((i, x.to_owned(), y.to_owned(), false));
                    }
                }
                None if m.features.contains_key(f) || m.deps.iter().any(|d| d.key == *f) => {
                    work.push(Work::Feature(i, f.clone()))
                }
                None => {}
            }
        }
    }
    let mut guard = 0usize;
    loop {
        while let Some(w) = work.pop() {
            guard += 1;
            if guard > 5_000_000 {
                return;
            }
            match w {
                Work::Reach(n) => {
                    if g.nodes[n].reached {
                        continue;
                    }
                    g.nodes[n].reached = true;
                    for (di, d) in g.nodes[n].pkg.manifest.deps.iter().enumerate() {
                        if !d.optional {
                            work.push(Work::Dep(n, di));
                        }
                    }
                }
                Work::Dep(n, di) => {
                    let node = &mut g.nodes[n];
                    let t = match node.deps.get(di).cloned() {
                        Some(Some(Ok(t))) => t,
                        // A dependency that cannot be located (locked but not downloaded)
                        // is still a dependency: it stays in the list with its reason.
                        Some(Some(Err(_))) => {
                            node.active[di] = true;
                            continue;
                        }
                        _ => continue,
                    };
                    if node.active[di] {
                        continue;
                    }
                    node.active[di] = true;
                    let d = &node.pkg.manifest.deps[di];
                    work.push(Work::Reach(t));
                    for f in &d.features {
                        work.push(Work::Feature(t, f.clone()));
                    }
                    if d.default_features {
                        work.push(Work::Feature(t, "default".into()));
                    }
                }
                Work::Feature(n, f) => {
                    if !g.nodes[n].features.insert(f.clone()) {
                        continue;
                    }
                    let m = Arc::clone(&g.nodes[n].pkg.manifest);
                    let dep_idx = |key: &str| -> Vec<usize> {
                        m.deps
                            .iter()
                            .enumerate()
                            .filter(|(_, d)| d.key == key)
                            .map(|(i, _)| i)
                            .collect()
                    };
                    match m.features.get(&f) {
                        Some(items) => {
                            for item in items {
                                if let Some(dep) = item.strip_prefix("dep:") {
                                    for di in dep_idx(dep) {
                                        work.push(Work::Dep(n, di));
                                    }
                                } else if let Some((x, y)) = item.split_once('/') {
                                    let (x, weak) = match x.strip_suffix('?') {
                                        Some(x) => (x, true),
                                        None => (x, false),
                                    };
                                    if !weak {
                                        for di in dep_idx(x) {
                                            work.push(Work::Dep(n, di));
                                        }
                                        if m.features.contains_key(x) {
                                            work.push(Work::Feature(n, x.to_owned()));
                                        }
                                    }
                                    dep_feats.push((n, x.to_owned(), y.to_owned(), weak));
                                } else if m.features.contains_key(item) {
                                    work.push(Work::Feature(n, item.clone()));
                                } else {
                                    for di in dep_idx(item) {
                                        work.push(Work::Dep(n, di));
                                    }
                                }
                            }
                        }
                        None => {
                            // Implicit feature of an optional dependency.
                            for di in dep_idx(&f) {
                                if m.deps[di].optional {
                                    work.push(Work::Dep(n, di));
                                }
                            }
                        }
                    }
                }
            }
        }
        // Dependency features whose dependency is (now) active.
        for (n, key, feat, _) in &dep_feats {
            let node = &g.nodes[*n];
            for (di, d) in node.pkg.manifest.deps.iter().enumerate() {
                if d.key == *key
                    && node.active.get(di) == Some(&true)
                    && let Some(Some(Ok(t))) = node.deps.get(di)
                    && !g.nodes[*t].features.contains(feat)
                {
                    work.push(Work::Feature(*t, feat.clone()));
                }
            }
        }
        if work.is_empty() {
            break;
        }
    }
}
