//! Which `tsconfig.json` governs a file, for `compilerOptions.paths` /
//! `baseUrl`.
//!
//! The tracer discovers configs itself instead of using `oxc_resolver`'s
//! [`oxc_resolver::TsconfigDiscovery::Auto`], for two reasons:
//!
//! - **JS files.** oxc's auto discovery only ran for TS files here. Bundlers
//!   apply `paths` to every module of a project: Next.js (webpack's
//!   `JsConfigPathsPlugin` and Turbopack) reads the app's `tsconfig.json`, else
//!   `jsconfig.json`, and applies it to `.js`/`.jsx`/ `.mjs` files too,
//!   whatever `allowJs` and `include` say; vite-tsconfig-paths and Vite's own
//!   `resolve.tsconfigPaths` use the config that owns the file.
//! - **Graceful degradation.** oxc loads every project reference eagerly, and
//!   one missing reference, or one reference whose `extends` cannot be resolved
//!   or parsed, fails every resolution from every file below that config. Here
//!   a broken reference is skipped and reported ([`DirConfig::errors`]); the
//!   rest keep working.
//!
//! The governing config of a file outside `node_modules` (none inside, like oxc
//! and TypeScript) is, walking up from the file's directory:
//!
//! 1. the first project reference (of the first config found at that level)
//!    whose `files`/`include`/`exclude` covers the file;
//! 2. else that config itself if it covers the file and is not solution-style
//!    (`references` plus explicitly empty `files` and `include`);
//! 3. else the walk continues upwards.
//!
//! At each level `tsconfig.json` is used if present, else `jsconfig.json`
//! (Next.js), whose `allowJs` defaults to `true` as in TypeScript. Coverage
//! follows TypeScript (and oxc): a `.js`-family file needs `allowJs`,
//! wildcard-terminated patterns only match TS (and, with `allowJs`, JS)
//! extensions, directory patterns get an implicit `/**/*`.
//!
//! When no config covers the file and
//! [`super::JsResolveOptions::tsconfig_fallback`] is set (the default), the
//! nearest config at or below the file's package root (nearest `package.json`)
//! is used anyway. That is the bundler behavior: Next.js applies the app's
//! config to every module it compiles, including `.jsx` files that an `include`
//! of `**/*.ts`, `**/*.js` does not name and JS files without `allowJs`.
//! Applying an alias the runtime might not apply only adds candidates, in line
//! with the tracer's over-approximation.
//!
//! Configs are loaded by `oxc_resolver` with references disabled (so `extends`
//! chains, `{configDir}` and `paths` compilation stay oxc's), and references
//! are loaded one by one.
//!
//! # What a load is computed from
//!
//! A file whose config discovery consults a config records what the config's
//! load read (`level_observations` in the parent module): a change of any of it
//! can change how the file's imports resolve. That record is not derived from
//! the config's text. It is what the loader asked of the file system while it
//! loaded ([`Tsconfigs::load`]): every file it read (the config, the configs
//! its `extends` chain names, the manifests a package request goes through) and
//! every path it looked for and did not find (a file that appears there is
//! another answer). The loader reads through the shared [`DirCache`],
//! which reports each of those to the open recording frame, and every load runs
//! in a loader of its own, so nothing is answered from what an earlier load
//! left in oxc's tables. Whatever rule decides which file an `extends` names (a
//! missing `.json`, a package's `exports` or `imports`, a directory, a link),
//! the file it names is in the record because it was read, and the places that
//! could name another are there because they were asked.
//!
//! A manifest is the one file the loader does not get whole. The resolution of
//! an `extends` entry that names a package reads the manifest of the nearest
//! package scope (a self-reference: its `name`, then its `exports`; a subpath
//! import: its `imports`) and the one of the package directory it finds
//! (`exports`, else `main`), and nothing else of either. The loader's file
//! system hands it exactly those fields ([`CachedFs::for_loader`],
//! [`turborepo_trace_core::manifest::loader_view`]) and records the read as one
//! of that view ([`turborepo_trace_core::record::loader_read`]). The nearest
//! manifest of a base config at the repository's root is the root manifest:
//! read whole, or through the resolution view, its `packageManager` would be an
//! input of every file below the config.
//!
//! [`recorded`] makes the record of it: a path behind a symbolic link of the
//! tree is recorded where the link leads, which is the path an edit names,
//! together with the link.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dashmap::DashMap;
use oxc_resolver::{
    ResolveError, ResolveOptions, TsConfig, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use turborepo_trace_core::{DirCache, Observations, fs::normalize, glob::Glob, record};

use super::{
    CachedFs, Oxc, in_node_modules,
    overlay::{Entry, Overlay},
};

/// Config file names looked for in each directory, in order.
pub(super) const CONFIG_NAMES: [&str; 2] = ["tsconfig.json", "jsconfig.json"];

/// A loaded config with its compiled coverage rules.
#[derive(Debug)]
pub(super) struct Config {
    pub(super) config: Arc<TsConfig>,
    /// The path it was found at: `<dir>/tsconfig.json`, or a reference as
    /// listed (normalized; a directory or a file).
    pub(super) path: PathBuf,
    owner: Owner,
}

/// What discovery chose for a file (see [`Tsconfigs::walk`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Governing {
    /// No config applies.
    None,
    /// The config of the level at this index covers the file and governs.
    Level(usize),
    /// Nothing covers the file; the config of the level at this index is the
    /// package fallback. Discovery walked every level.
    Fallback(usize),
    /// A project reference of the level at this index governs: its index in
    /// [`DirConfig::ref_paths`].
    Reference(usize, usize),
}

/// The levels discovery consulted for a file, innermost first, up to and
/// including the level where a covering config was found (every level when none
/// covers it).
#[derive(Debug)]
pub(super) struct Walk {
    pub(super) levels: Vec<(PathBuf, Option<Arc<DirConfig>>)>,
    pub(super) governing: Governing,
    /// The governing config ([`Tsconfigs::governing`]).
    pub(super) config: Option<Arc<TsConfig>>,
}

/// The config found in one directory, with its loadable project references.
#[derive(Debug)]
pub(super) struct DirConfig {
    /// `<dir>/tsconfig.json` or `<dir>/jsconfig.json`.
    pub(super) path: PathBuf,
    /// `None` when the config itself could not be loaded (its own `extends` is
    /// broken).
    pub(super) config: Option<Config>,
    /// References that loaded.
    refs: Vec<Config>,
    /// Paths of the references, loaded or not (for recording).
    pub(super) ref_paths: Vec<PathBuf>,
    /// Load failures: the config's own, then one per skipped reference.
    pub(super) errors: Vec<String>,
    /// Every failure is a config that is not there (the target of an `extends`,
    /// a project reference): the config says where it looked, and the
    /// recorded chain has a probe for each of those places.
    pub(super) only_missing: bool,
    /// What the load of the config asked of the file system
    /// ([`Tsconfigs::load`]).
    pub(super) observed: Arc<Observations>,
    /// The same for the load of each reference, in the order of
    /// [`DirConfig::ref_paths`].
    pub(super) ref_observed: Vec<Arc<Observations>>,
}

/// One load of a config: what the loader made of it, and what it asked of the
/// file system on the way ([`Tsconfigs::load`]).
pub(super) struct Load {
    pub(super) config: Result<Arc<TsConfig>, ResolveError>,
    pub(super) observed: Arc<Observations>,
}

/// What a file records for one directory its discovery consults. It is a
/// function of the directory's config and of which of its project references
/// governs the file (none, when the config itself does, or a level further up),
/// so it is observed once for all such files.
#[derive(Debug, Default)]
pub(super) struct LevelRecord {
    /// The file's inputs: replayed ([`turborepo_trace_core::record::replay`]).
    pub(super) inputs: Observations,
    /// What decides the file's records without being an input of it: one set,
    /// recorded by reference ([`turborepo_trace_core::record::key`]).
    pub(super) key: Arc<Observations>,
}

/// Per-tracer config discovery, memoized per directory and per file.
pub(super) struct Tsconfigs {
    /// The file system every load reads through: the shared cache and, at
    /// another revision, its overlay.
    fs: CachedFs,
    fallback: bool,
    /// By the path asked for (a config file, a project reference as listed).
    loads: DashMap<PathBuf, Arc<Load>>,
    dirs: DashMap<PathBuf, Option<Arc<DirConfig>>>,
    files: DashMap<PathBuf, Option<Arc<TsConfig>>>,
    /// By directory and governing reference.
    records: DashMap<(PathBuf, Option<usize>), Arc<LevelRecord>>,
    /// Another revision's files where they differ from the disk; the loader's
    /// file system has the same one ([`super::governance`]).
    overlay: Option<Arc<Overlay>>,
}

/// How the config of a directory differs between two revisions, for the files
/// that consult it ([`DirConfig::diff`]).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LevelDiff {
    /// The same config file, the same references, the same load failures and
    /// the same coverage of the config and of every reference.
    Same,
    /// Anything a file below may see: every such file is to be looked at.
    Whole,
    /// Only these references differ (added, removed, or with other coverage
    /// rules), and the ones both revisions list keep their order: a file
    /// that none of them covers at either revision is governed as before.
    Refs(Vec<PathBuf>),
}

impl std::fmt::Debug for Tsconfigs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tsconfigs")
            .field("fallback", &self.fallback)
            .finish_non_exhaustive()
    }
}

impl Tsconfigs {
    /// Discovery on `fs`: the shared cache, at the revision of its overlay if
    /// it has one.
    pub(super) fn new(fs: CachedFs, fallback: bool) -> Self {
        Tsconfigs {
            overlay: fs.overlay().cloned(),
            fs,
            fallback,
            loads: DashMap::new(),
            dirs: DashMap::new(),
            files: DashMap::new(),
            records: DashMap::new(),
        }
    }

    /// Loads the config that `path` names (a file, `<path>.json`, or the
    /// `tsconfig.json` of a directory) with its `extends` chain, references
    /// disabled, and returns it with what the load asked of the file
    /// system. Once per path.
    ///
    /// Each load runs in a loader of its own. oxc keeps what it has read (the
    /// configs it parsed, the kind of every path it asked about, the
    /// manifests it went through) and answers the next load from there:
    /// with a loader shared between loads, the file system would see the
    /// base config of a hundred projects being read once, by
    /// whichever project loaded first. Underneath, every read is the shared
    /// cache's, so a file is read from disk once whatever the number of
    /// loads that name it.
    ///
    /// The observations are those of the shared cache
    /// ([`DirCache::with_recording`]; none when the cache does not record),
    /// captured for this load alone whatever frame or pause the caller is
    /// in.
    pub(super) fn load(&self, path: &Path) -> Arc<Load> {
        if let Some(l) = self.loads.get(path) {
            return Arc::clone(&l);
        }
        let loader = Oxc::new_with_file_system(self.fs.for_loader(), loader_options());
        let (config, observed) = record::capture(|| loader.resolve_tsconfig(path));
        let l = Arc::new(Load {
            config,
            observed: Arc::new(kept_directories(self.fs.cache(), observed)),
        });
        self.loads.insert(path.to_path_buf(), Arc::clone(&l));
        l
    }

    pub(super) fn clear(&self) {
        self.loads.clear();
        self.dirs.clear();
        self.files.clear();
        self.records.clear();
    }

    /// The record of the directory `dir` for the files that the reference
    /// `governing` of its config governs (`None`: no reference of this
    /// level does), made by `make` the first time.
    pub(super) fn level_record(
        &self,
        dir: &Path,
        governing: Option<usize>,
        make: impl FnOnce() -> LevelRecord,
    ) -> Arc<LevelRecord> {
        let key = (dir.to_path_buf(), governing);
        if let Some(r) = self.records.get(&key) {
            return Arc::clone(&r);
        }
        let r = Arc::new(make());
        self.records.insert(key, Arc::clone(&r));
        r
    }

    /// The config governing `file`. `package_dir` is the directory of the
    /// file's nearest `package.json`, which bounds the fallback.
    pub(super) fn governing(
        &self,
        fs: &DirCache,
        file: &Path,
        package_dir: Option<&Path>,
    ) -> Option<Arc<TsConfig>> {
        if in_node_modules(fs, file) || !file.is_absolute() {
            return None;
        }
        if let Some(g) = self.files.get(file) {
            return g.clone();
        }
        let g = self.discover(fs, file, package_dir);
        self.files.insert(file.to_path_buf(), g.clone());
        g
    }

    fn discover(
        &self,
        fs: &DirCache,
        file: &Path,
        package_dir: Option<&Path>,
    ) -> Option<Arc<TsConfig>> {
        self.find(fs, file, package_dir, &mut |_, _| {}).0
    }

    /// The discovery walk for `file` with every level it consulted, for
    /// recording exactly what the choice depends on
    /// (`record_tsconfig_chain`). Same rules as [`Tsconfigs::governing`]
    /// (both run [`Tsconfigs::find`]).
    pub(super) fn walk(&self, fs: &DirCache, file: &Path, package_dir: Option<&Path>) -> Walk {
        if in_node_modules(fs, file) || !file.is_absolute() {
            return Walk {
                levels: Vec::new(),
                governing: Governing::None,
                config: None,
            };
        }
        let mut levels = Vec::new();
        let (config, governing) = self.find(fs, file, package_dir, &mut |d, dc| {
            levels.push((d.to_path_buf(), dc.cloned()));
        });
        Walk {
            levels,
            governing,
            config,
        }
    }

    /// Walks up from `file`, calling `visit` for every level consulted
    /// (innermost first), and returns the governing config with where it
    /// was found.
    fn find(
        &self,
        fs: &DirCache,
        file: &Path,
        package_dir: Option<&Path>,
        visit: &mut dyn FnMut(&Path, Option<&Arc<DirConfig>>),
    ) -> (Option<Arc<TsConfig>>, Governing) {
        let file_str = file.to_string_lossy();
        let mut fallback: Option<(Arc<TsConfig>, usize)> = None;
        let found = |c: &Config| Arc::clone(&c.config);
        let mut cur = file.parent();
        let mut level = 0usize;
        while let Some(d) = cur {
            cur = d.parent();
            let dc = self.dir_config(fs, d);
            visit(d, dc.as_ref());
            let at = level;
            level += 1;
            let Some(dc) = dc else {
                continue;
            };
            if let Some(r) = dc.refs.iter().find(|r| r.owner.covers(file, &file_str)) {
                let idx = dc.ref_paths.iter().position(|p| *p == r.path).unwrap_or(0);
                return (Some(found(r)), Governing::Reference(at, idx));
            }
            let Some(c) = &dc.config else {
                continue;
            };
            if !c.owner.solution && c.owner.covers(file, &file_str) {
                return (Some(found(c)), Governing::Level(at));
            }
            if fallback.is_none() && package_dir.is_none_or(|p| d.starts_with(p)) {
                fallback = Some((found(c), at));
            }
        }
        match fallback {
            Some((c, at)) if self.fallback => (Some(c), Governing::Fallback(at)),
            _ => (None, Governing::None),
        }
    }

    /// The config in directory `d`, loaded once.
    pub(super) fn dir_config(&self, fs: &DirCache, d: &Path) -> Option<Arc<DirConfig>> {
        if let Some(c) = self.dirs.get(d) {
            return c.clone();
        }
        let c = self.load_dir(fs, d).map(Arc::new);
        self.dirs.insert(d.to_path_buf(), c.clone());
        c
    }

    fn load_dir(&self, fs: &DirCache, d: &Path) -> Option<DirConfig> {
        let overlaid = |p: &Path| self.overlay.as_ref().and_then(|o| o.entry(fs, p));
        // Where the working tree has what the revision has at a path without an entry.
        let on_disk = |p: &Path| match &self.overlay {
            Some(o) => o.on_disk(fs, p).into_owned(),
            None => p.to_path_buf(),
        };
        let (path, js) = CONFIG_NAMES
            .iter()
            .map(|n| d.join(n))
            .zip([false, true])
            .find(|(p, _)| match overlaid(p) {
                Some(e) => e == Entry::File,
                None => fs.is_file(&on_disk(p)),
            })?;
        // oxc clears `references` when loading with references disabled: read them from
        // the file. A file that oxc could not parse has none here either.
        let text = match self.overlay.as_ref().and_then(|o| o.read(fs, &path)) {
            Some(r) => r,
            None => fs.read_cached(&on_disk(&path)),
        };
        let ref_paths: Vec<PathBuf> = text
            .ok()
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
            .and_then(|s| TsConfig::parse(true, &path, &path, s).ok())
            .map(|t| {
                t.references
                    .iter()
                    .map(|r| normalize(&d.join(&r.path)))
                    .filter(|p| *p != path && *p != d)
                    .collect()
            })
            .unwrap_or_default();
        let mut errors = Vec::new();
        let mut only_missing = true;
        let mut failed = |e: &oxc_resolver::ResolveError| {
            only_missing &= matches!(e, oxc_resolver::ResolveError::TsconfigNotFound(_));
        };
        let own = self.load(&path);
        let config = match &own.config {
            Ok(c) => Some(Config::new(
                Arc::clone(c),
                path.clone(),
                js,
                !ref_paths.is_empty(),
            )),
            Err(e) => {
                failed(e);
                errors.push(format!("not loaded, its `paths` are ignored: {e}"));
                None
            }
        };
        let mut refs = Vec::new();
        let mut ref_observed = Vec::with_capacity(ref_paths.len());
        for r in &ref_paths {
            let l = self.load(r);
            match &l.config {
                Ok(c) => refs.push(Config::new(Arc::clone(c), r.clone(), false, false)),
                Err(e) => {
                    failed(e);
                    errors.push(format!("skipped project reference {}: {e}", r.display()));
                }
            }
            ref_observed.push(Arc::clone(&l.observed));
        }
        Some(DirConfig {
            path,
            config,
            refs,
            ref_paths,
            errors,
            only_missing,
            observed: Arc::clone(&own.observed),
            ref_observed,
        })
    }
}

/// What a load observed, without the "is a directory" answers that something it
/// read lies in (the loader's file system records every one, as the directory's
/// membership of its parent under its literal name: `CachedFs::metadata`). A
/// file that was read inside a directory says that the directory is one, and
/// goes with it: the answer needs a record of its own only where the load
/// looked inside and read nothing (a directory at the name of an `extends`
/// target or of a project reference that holds no config).
///
/// "Lies in" by the path as it was asked and by where its links lead: a base
/// config of a workspace package is read through the package's `node_modules`
/// link, and the loader asks about every directory of the real path while it
/// follows the link. Without the second reading every file below such a config
/// recorded those directories.
fn kept_directories(fs: &DirCache, mut observed: Observations) -> Observations {
    if observed.listings.is_empty() {
        return observed;
    }
    let real = |p: &Path| fs.canonicalize(p).path;
    let read: Vec<PathBuf> = (observed.reads.iter())
        .chain(&observed.view_reads)
        .chain(&observed.loader_reads)
        .flat_map(|r| [real(r), r.clone()])
        .collect();
    let listings = std::mem::take(&mut observed.listings)
        .into_iter()
        .filter(|(parent, pattern)| {
            // (Each is the record of one name: `CachedFs::is_a_directory`.)
            let Some(name) = record::pattern_name(pattern) else {
                return true;
            };
            let dir = parent.join(name);
            let there = real(&dir);
            !read
                .iter()
                .any(|r| r.starts_with(&dir) || r.starts_with(&there))
        })
        .collect();
    observed.listings = listings;
    observed
}

/// The options of a loader of configs: references disabled ([`Tsconfigs`] loads
/// them one by one). Nothing else of a resolver's options reaches a config
/// load: an `extends` is resolved with oxc's own options for it.
fn loader_options() -> ResolveOptions {
    ResolveOptions {
        builtin_modules: true,
        symlinks: false,
        node_path: false,
        tsconfig: Some(TsconfigDiscovery::Manual(TsconfigOptions {
            config_file: PathBuf::from("/"),
            references: TsconfigReferences::Disabled,
        })),
        ..ResolveOptions::default()
    }
}

/// The record of what config discovery observed (`observed`: the loads of the
/// configs a file consults, [`Tsconfigs::load`], and the config names looked
/// for), as a file stores it:
///
/// - **The machine's paths are left out.** A lookup walks up to the file
///   system's root. A path that was looked for, or a manifest that was read, in
///   a directory above the repository or in a `node_modules` above it is the
///   machine's, and a graph does not depend on it (a probe there is dropped by
///   the store anyway). A config that an `extends` names by its path is read
///   wherever it lies.
/// - **Behind a symbolic link of the tree, the real path and the link.** A
///   config that was read through a link (the config itself is one, or a
///   directory on the way) is recorded where the link leads: that is the file
///   whose contents were read, and the path that an edit of it names. The link
///   is recorded with it, as a link crossed ([`record::link`]: where it leads
///   is part of the record's state) and as an input at its own path (a diff
///   that makes it lead elsewhere names the link, and nothing else). The same
///   for a path that was not found behind a link. Paths through a
///   `node_modules` directory are left as they are: the store records those
///   where the package directory leads, with the directory as a place seen.
/// - The bare package names of the `node_modules` paths, for the store's
///   walk-up records.
pub(super) fn recorded(fs: &DirCache, observed: Observations) -> Observations {
    let mut out = Observations {
        path_reads: observed.path_reads,
        packages: observed.packages,
        links: observed.links,
        key: observed.key,
        ..Observations::default()
    };
    let base = fs.base();
    let machines = |p: &Path| -> bool {
        let Some(base) = base else {
            return false;
        };
        if p.starts_with(base) {
            return false;
        }
        // The deepest directory above the repository that `p` lies in.
        let Some(above) = base.ancestors().skip(1).find(|a| p.starts_with(a)) else {
            return false;
        };
        let mut rest = p.strip_prefix(above).unwrap_or(p).components();
        let first = rest.next();
        rest.next().is_none() || first.is_some_and(|c| c.as_os_str() == "node_modules")
    };
    // `p` as it is recorded: where its links lead, with the links (not through
    // `node_modules`, and not through a link outside the repository).
    let real = |p: PathBuf, out: &mut Observations| -> PathBuf {
        if in_node_modules(fs, &p) {
            if let Some(name) = installed_name(&p) {
                out.packages.push(name);
            }
            return p;
        }
        let rp = fs.canonicalize(&p);
        if rp.links.is_empty()
            || rp
                .links
                .iter()
                .any(|(l, _)| base.is_some_and(|b| !l.starts_with(b)))
        {
            return p;
        }
        for (link, _) in rp.links {
            out.reads.push(link.clone());
            out.links.push(link);
        }
        rp.path
    };
    for p in observed.missing {
        if !machines(&p) {
            let p = real(p, &mut out);
            out.missing.push(p);
        }
    }
    for p in observed.reads {
        let p = real(p, &mut out);
        out.reads.push(p);
    }
    for p in observed.view_reads {
        if !machines(&p) {
            let p = real(p, &mut out);
            out.view_reads.push(p);
        }
    }
    for p in observed.loader_reads {
        if !machines(&p) {
            let p = real(p, &mut out);
            out.loader_reads.push(p);
        }
    }
    // The directories a load found where it asked (`kept_directories`): each as a
    // membership of its parent under its name, like a path that was not found.
    for (parent, pattern) in observed.listings {
        // (The pattern of one name, `record::literal_pattern`; anything else is kept as
        // it is.)
        let Some(name) = record::pattern_name(&pattern) else {
            out.listings.push((parent, pattern));
            continue;
        };
        let at = parent.join(&name);
        if machines(&at) {
            continue;
        }
        let at = real(at, &mut out);
        if let (Some(parent), Some(name)) = (at.parent(), at.file_name()) {
            out.listings.push((
                parent.to_path_buf(),
                record::literal_pattern(&name.to_string_lossy()),
            ));
        }
    }
    out.dedup();
    out
}

/// The bare package name that the path `p` names below its innermost
/// `node_modules` directory (`react`, `@scope/pkg`), if it names one.
fn installed_name(p: &Path) -> Option<String> {
    let mut after: Option<Vec<&str>> = None;
    for c in p.components() {
        let c = c.as_os_str().to_str()?;
        if c == "node_modules" {
            after = Some(Vec::new());
        } else if let Some(a) = &mut after
            && a.len() < 2
        {
            a.push(c);
        }
    }
    match after?.as_slice() {
        [scope, name, ..] if scope.starts_with('@') => Some(format!("{scope}/{name}")),
        [name, ..] if !name.starts_with('@') => Some((*name).to_owned()),
        _ => None,
    }
}

impl DirConfig {
    /// Whether the reference at `path` (an entry of [`DirConfig::ref_paths`])
    /// loaded.
    pub(super) fn loaded(&self, path: &Path) -> bool {
        self.refs.iter().any(|r| r.path == path)
    }

    /// Whether the reference at `path` loaded and covers `file`.
    pub(super) fn reference_covers(&self, path: &Path, file: &Path, file_str: &str) -> bool {
        self.refs
            .iter()
            .any(|r| r.path == path && r.owner.covers(file, file_str))
    }

    /// How this config differs from `other`, the config of the same directory
    /// at another revision, in what decides the discovery of the files that
    /// consult the directory ([`Tsconfigs::find`]) and the load failures
    /// reported on them.
    pub(super) fn diff(&self, other: &DirConfig) -> LevelDiff {
        if self.path != other.path || self.errors != other.errors {
            return LevelDiff::Whole;
        }
        match (&self.config, &other.config) {
            (Some(a), Some(b)) if a.owner.same(&b.owner) => {}
            (None, None) => {}
            _ => return LevelDiff::Whole,
        }
        // The first reference that covers a file governs it: the ones both list must be
        // in the same order.
        let common = |a: &[PathBuf], b: &[PathBuf]| -> Vec<PathBuf> {
            a.iter().filter(|r| b.contains(r)).cloned().collect()
        };
        let both = common(&self.ref_paths, &other.ref_paths);
        if both != common(&other.ref_paths, &self.ref_paths) {
            return LevelDiff::Whole;
        }
        let find = |dc: &'_ DirConfig, r: &Path| -> Option<usize> {
            dc.refs.iter().position(|c| c.path == r)
        };
        let mut differ: Vec<PathBuf> = Vec::new();
        for r in self.ref_paths.iter().chain(&other.ref_paths) {
            if differ.contains(r) {
                continue;
            }
            let same = both.contains(r)
                && match (find(self, r), find(other, r)) {
                    (Some(a), Some(b)) => self.refs[a].owner.same(&other.refs[b].owner),
                    // Loaded at neither (the failures are equal).
                    (None, None) => true,
                    _ => false,
                };
            if !same {
                differ.push(r.clone());
            }
        }
        if differ.is_empty() {
            LevelDiff::Same
        } else {
            LevelDiff::Refs(differ)
        }
    }
}

impl Config {
    /// `has_refs`: the file lists project references (oxc drops them when
    /// loading).
    fn new(config: Arc<TsConfig>, path: PathBuf, jsconfig: bool, has_refs: bool) -> Self {
        let owner = Owner::new(&config, jsconfig, has_refs);
        Config {
            config,
            path,
            owner,
        }
    }
}

/// TypeScript's `files` / `include` / `exclude` coverage, precompiled.
#[derive(Debug)]
struct Owner {
    dir: PathBuf,
    allow_js: bool,
    solution: bool,
    files: Option<Vec<PathBuf>>,
    include: Option<Vec<Pattern>>,
    exclude: Option<Vec<Pattern>>,
}

#[derive(Debug)]
struct Pattern {
    exact: String,
    glob: Glob,
    /// Wildcard-terminated (`**/*`, a bare directory): only matches compiled
    /// extensions.
    any_file: bool,
}

impl Pattern {
    fn new(p: &Path) -> Self {
        let exact = p.to_string_lossy().into_owned();
        let last = exact.rsplit('/').next().unwrap_or(&exact);
        let glob = if last.contains(['.', '*', '?']) {
            exact.clone()
        } else {
            format!("{}/**/*", exact.trim_end_matches('/'))
        };
        Pattern {
            any_file: glob.ends_with('*'),
            glob: Glob::new(&glob),
            exact,
        }
    }

    fn matches(&self, path: &str, ext_allowed: bool) -> bool {
        if self.exact == path {
            return true;
        }
        if self.any_file && !ext_allowed {
            return false;
        }
        self.glob.matches(path.as_bytes())
    }
}

impl Owner {
    /// Whether `other` covers the same files: the same rules, written the same
    /// way.
    fn same(&self, other: &Owner) -> bool {
        let pats = |v: &Option<Vec<Pattern>>| {
            v.as_ref()
                .map(|v| v.iter().map(|p| p.exact.clone()).collect::<Vec<_>>())
        };
        self.dir == other.dir
            && self.allow_js == other.allow_js
            && self.solution == other.solution
            && self.files == other.files
            && pats(&self.include) == pats(&other.include)
            && pats(&self.exclude) == pats(&other.exclude)
    }

    fn new(c: &TsConfig, jsconfig: bool, has_refs: bool) -> Self {
        let pats = |v: &Option<Vec<PathBuf>>| {
            v.as_ref()
                .map(|v| v.iter().map(|p| Pattern::new(p)).collect::<Vec<_>>())
        };
        Owner {
            dir: c.directory().to_path_buf(),
            allow_js: c.compiler_options.allow_js.unwrap_or(jsconfig),
            solution: has_refs
                && matches!(c.files.as_deref(), Some([]))
                && matches!(c.include.as_deref(), Some([])),
            files: c.files.clone(),
            include: pats(&c.include),
            exclude: pats(&c.exclude),
        }
    }

    fn covers(&self, path: &Path, path_str: &str) -> bool {
        if self
            .files
            .as_ref()
            .is_some_and(|f| f.iter().any(|f| f == path))
        {
            return true;
        }
        let (ts, js) = match path.extension().and_then(|e| e.to_str()) {
            Some("ts" | "tsx" | "mts" | "cts") => (true, false),
            Some("js" | "jsx" | "mjs" | "cjs") => (false, true),
            Some(_) => (false, false),
            None => return false,
        };
        if js && !self.allow_js {
            return false;
        }
        let ext_allowed = ts || js;
        let included = match &self.include {
            None => self.files.is_none() && path.starts_with(&self.dir) && ext_allowed,
            Some(p) => p.iter().any(|p| p.matches(path_str, ext_allowed)),
        };
        included
            && !self
                .exclude
                .as_ref()
                .is_some_and(|p| p.iter().any(|p| p.matches(path_str, ext_allowed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(json: &str, jsconfig: bool) -> Owner {
        let mut c = TsConfig::parse(
            true,
            Path::new("/p/tsconfig.json"),
            Path::new("/p/tsconfig.json"),
            json.to_string(),
        )
        .unwrap();
        // `build()` is internal to oxc; absolutize like it does.
        let abs = |v: &mut Option<Vec<PathBuf>>| {
            if let Some(v) = v {
                for p in v.iter_mut() {
                    *p = normalize(&Path::new("/p").join(&*p));
                }
            }
        };
        abs(&mut c.files);
        abs(&mut c.include);
        abs(&mut c.exclude);
        Owner::new(&c, jsconfig, false)
    }

    fn covers(o: &Owner, p: &str) -> bool {
        o.covers(Path::new(p), p)
    }

    #[test]
    fn coverage_follows_typescript() {
        let o = owner(
            r#"{"include":["**/*.ts","**/*.js","src"],"exclude":["gen"]}"#,
            false,
        );
        assert!(covers(&o, "/p/a/x.ts"));
        assert!(covers(&o, "/p/src/x.tsx"), "directory pattern");
        assert!(!covers(&o, "/p/a/x.js"), "js without allowJs");
        assert!(!covers(&o, "/p/gen/x.ts"), "excluded");
        assert!(!covers(&o, "/q/x.ts"), "outside");

        let o = owner(
            r#"{"compilerOptions":{"allowJs":true},"include":["**/*.ts","**/*.js"]}"#,
            false,
        );
        assert!(covers(&o, "/p/a/x.js"));
        assert!(!covers(&o, "/p/a/x.jsx"), "explicit extensions only");

        // No include: everything below the config with a compiled extension.
        let o = owner(r#"{}"#, false);
        assert!(covers(&o, "/p/a/b.mts"));
        assert!(!covers(&o, "/p/a/b.js"));
        assert!(!covers(&o, "/p/a/b.json"));
        // jsconfig.json implies allowJs.
        let o = owner(r#"{}"#, true);
        assert!(covers(&o, "/p/a/b.jsx"));
        // `files` only.
        let o = owner(r#"{"files":["a.ts"]}"#, false);
        assert!(covers(&o, "/p/a.ts"));
        assert!(!covers(&o, "/p/b.ts"));
    }
}
