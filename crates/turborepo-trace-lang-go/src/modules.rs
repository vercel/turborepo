//! Go module resolution without the `go` command: which module a file belongs
//! to, the resolution context (main module or workspace, requirements,
//! replacements, vendoring), and where an import path's package directory is.
//!
//! Order for an import path (as `cmd/go` in module mode):
//!
//! 1. the standard library: the bundled list ([`crate::stdlib`]) or
//!    `$GOROOT/src/<path>`;
//! 2. the main modules (the `go.mod` module, or every `use` module of a
//!    `go.work`), by longest module-path prefix;
//! 3. vendor mode (`vendor/modules.txt` present and `go` ≥ 1.14, or ≥ 1.22 for
//!    a workspace, unless disabled): `vendor/<path>`;
//! 4. module mode: the required module with the longest path prefix whose
//!    directory holds the package, after `replace` (local directory or another
//!    module version), in `$GOMODCACHE/<escaped path>@<escaped version>`.
//!
//! Versions come from the main modules' `require` lines (the highest across a
//! workspace). Since Go 1.17, `go mod tidy` lists every module that provides a
//! package in the build, at its selected version, so these are the build list.
//! Only when an import is not provided by a listed module (older `go.mod`
//! files), a minimal version selection over the module graph (`go.mod` files of
//! the module cache) runs once per context, without graph pruning.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use turborepo_trace_core::{
    DirCache,
    fs::{EntryKind, normalize},
};
use turborepo_trace_lockfile::gosum::{
    self, GoMod, GoReplace, GoSum, GoWork, ReplaceTarget, compare_versions,
};

use crate::stdlib::STD_PACKAGES;

/// How vendoring is decided (`-mod` flag).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum VendorMode {
    /// Vendor when `vendor/modules.txt` exists and the `go` version allows it
    /// (default).
    #[default]
    Auto,
    /// Always resolve third-party imports in `vendor/` (`-mod=vendor`).
    On,
    /// Never (`-mod=mod`).
    Off,
}

/// Escapes a module path or version for the module cache: each upper-case
/// letter becomes `!` plus its lower-case form.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Inverse of [`escape`]. `None` for malformed input (an upper-case letter or a
/// dangling `!`).
pub fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut bang = false;
    for c in s.chars() {
        if bang {
            if !c.is_ascii_lowercase() {
                return None;
            }
            out.push(c.to_ascii_uppercase());
            bang = false;
        } else if c == '!' {
            bang = true;
        } else if c.is_ascii_uppercase() {
            return None;
        } else {
            out.push(c);
        }
    }
    (!bang).then_some(out)
}

/// Whether `path` is in the standard library according to the bundled list.
pub fn is_bundled_std(path: &str) -> bool {
    STD_PACKAGES.binary_search(&path).is_ok()
}

/// Whether the first element of an import path has no dot: such paths are
/// reserved for the standard library (unless a main module claims them).
pub fn is_std_like(path: &str) -> bool {
    !path.split('/').next().unwrap_or(path).contains('.')
}

/// One module directory with its path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleDir {
    /// Module path.
    pub path: String,
    /// Directory.
    pub dir: PathBuf,
}

/// Where an import path's package was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PkgLoc {
    /// The standard library (or the cgo pseudo-package `C`).
    Std,
    /// A package directory.
    Dir {
        /// The directory.
        dir: PathBuf,
        /// The module providing it: path and version (`None` for main modules
        /// and local replacements, which are first-party).
        module: Option<(String, String)>,
        /// The providing module's `go.mod`, if it has one.
        go_mod: Option<PathBuf>,
    },
    /// Not on disk, but the build list names the modules that can provide it
    /// and they are not in the module cache (nothing was downloaded):
    /// `(module path, version, module directory in the cache)` after
    /// `replace`, longest module path first.
    NotDownloaded(Vec<(String, String, PathBuf)>),
    /// Not found; the message says why.
    Missing(String),
}

/// One `vendor/modules.txt` module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VendoredModule {
    /// Module path.
    pub path: String,
    /// Version (empty for a module replaced by a directory without version).
    pub version: String,
    /// Packages listed.
    pub packages: Vec<String>,
}

/// Parses `vendor/modules.txt`.
pub fn parse_modules_txt(src: &str) -> Vec<VendoredModule> {
    let mut out: Vec<VendoredModule> = Vec::new();
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            let mut f = rest.split_ascii_whitespace();
            let Some(path) = f.next() else { continue };
            let version = f.next().filter(|v| *v != "=>").unwrap_or_default();
            out.push(VendoredModule {
                path: path.to_owned(),
                version: version.to_owned(),
                packages: Vec::new(),
            });
        } else if line.starts_with('#') || line.trim().is_empty() {
            continue;
        } else if let Some(m) = out.last_mut() {
            m.packages.push(line.trim().to_owned());
        }
    }
    out
}

/// Vendoring state of a context.
#[derive(Clone, Debug)]
pub struct Vendor {
    /// The `vendor` directory.
    pub dir: PathBuf,
    /// Its `modules.txt`.
    pub modules_txt: PathBuf,
    /// Package import path → (module path, version).
    pub packages: HashMap<String, (String, String)>,
}

/// A resolution context: a main module or a workspace.
#[derive(Debug)]
pub struct Ctx {
    /// The `go.work` or `go.mod` that defines it.
    pub key: PathBuf,
    /// Main modules, longest path first.
    pub main: Vec<ModuleDir>,
    /// Required versions (highest across main modules).
    pub requires: HashMap<String, String>,
    /// Replacements with the directory local targets are relative to; `go.work`
    /// ones first.
    pub replaces: Vec<(GoReplace, PathBuf)>,
    /// Excluded versions.
    pub excludes: Vec<(String, String)>,
    /// Vendoring, if active.
    pub vendor: Option<Vendor>,
    /// Hashes from `go.sum` (and `go.work.sum`) files.
    pub sums: GoSum,
    /// Manifests every file of the context depends on: `go.work`,
    /// `go.work.sum`, the `go.sum` files and `vendor/modules.txt`. (Each
    /// file's own `go.mod` is added by the resolver.)
    pub manifests: Vec<PathBuf>,
    /// The module cache.
    pub modcache: PathBuf,
    /// The lazily computed MVS build list (fallback).
    mvs: OnceLock<HashMap<String, String>>,
}

impl Ctx {
    /// The main module whose path is the longest prefix of `import`.
    pub fn main_for(&self, import: &str) -> Option<&ModuleDir> {
        self.main.iter().find(|m| is_prefix(&m.path, import))
    }

    /// Whether `dir` is the root of a main module.
    pub fn is_main_root(&self, dir: &Path) -> bool {
        self.main.iter().any(|m| m.dir == dir)
    }

    /// Directories of local `replace` targets.
    pub fn local_replace_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.replaces.iter().filter_map(|(r, base)| match &r.new {
            ReplaceTarget::Local(d) => Some(normalize(&base.join(d))),
            ReplaceTarget::Module { .. } => None,
        })
    }

    fn replacement(&self, path: &str, version: &str) -> Option<(&ReplaceTarget, &Path)> {
        let find = |exact: bool| {
            self.replaces.iter().find(|(r, _)| {
                r.old == path
                    && if exact {
                        r.old_version.as_deref() == Some(version)
                    } else {
                        r.old_version.is_none()
                    }
            })
        };
        find(true)
            .or_else(|| find(false))
            .map(|(r, b)| (&r.new, b.as_path()))
    }

    /// The directory of `module@version` after replacement, with its `go.mod`
    /// path and whether it is a local (first-party) directory.
    pub fn module_dir(&self, module: &str, version: &str) -> (PathBuf, bool) {
        match self.replacement(module, version) {
            Some((ReplaceTarget::Local(d), base)) => (normalize(&base.join(d)), true),
            Some((ReplaceTarget::Module { path, version }, _)) => (
                self.modcache
                    .join(format!("{}@{}", escape(path), escape(version))),
                false,
            ),
            None => (
                self.modcache
                    .join(format!("{}@{}", escape(module), escape(version))),
                false,
            ),
        }
    }

    /// Finds the package directory of `import` (steps 2–4 of the module docs).
    pub fn locate(&self, fs: &DirCache, import: &str) -> PkgLoc {
        if let Some(m) = self.main_for(import) {
            let dir = join_import(&m.dir, &m.path, import);
            return if fs.is_dir(&dir) {
                PkgLoc::Dir {
                    dir,
                    module: None,
                    go_mod: Some(m.dir.join("go.mod")),
                }
            } else {
                // The package is the Go files of that directory, also while it is missing: a
                // generator can write them later (a gitignored `gen/` tree). The membership
                // makes an added file a change of the importer, and lets the graph store link
                // the importer to the task that produces files there.
                if fs.records() {
                    turborepo_trace_core::record::listing(&dir, "*.go");
                }
                PkgLoc::Missing(format!(
                    // No directory in the message: details must not depend on where the
                    // checkout lives (snapshots, shared reports).
                    "package {import} is not in main module {}",
                    m.path
                ))
            };
        }
        if let Some(v) = &self.vendor {
            let dir = v.dir.join(import);
            let module = v.packages.get(import).cloned().or_else(|| {
                v.packages
                    .values()
                    .filter(|(p, _)| is_prefix(p, import))
                    .max_by_key(|(p, _)| p.len())
                    .cloned()
            });
            return if has_go_files(fs, &dir) {
                PkgLoc::Dir {
                    dir,
                    module,
                    go_mod: Some(v.modules_txt.clone()),
                }
            } else {
                PkgLoc::Missing(format!(
                    "package {import} is not in {}",
                    v.modules_txt.display()
                ))
            };
        }
        if let Some(found) = self.locate_in(fs, import, &self.requires) {
            return found;
        }
        // Not provided by a listed module: try the full build list.
        let mvs = self.mvs.get_or_init(|| self.build_list(fs));
        if let Some(found) = self.locate_in(fs, import, mvs) {
            return found;
        }
        let absent = self.not_downloaded(fs, import, mvs);
        if !absent.is_empty() {
            return PkgLoc::NotDownloaded(absent);
        }
        // Without the module cache's path: a record must not name a directory of the
        // machine that built the graph.
        PkgLoc::Missing(format!(
            "no required module provides package {import} (or it is not in the module cache)"
        ))
    }

    fn locate_in(
        &self,
        fs: &DirCache,
        import: &str,
        versions: &HashMap<String, String>,
    ) -> Option<PkgLoc> {
        let mut cands: Vec<(&String, &String)> = versions
            .iter()
            .filter(|(m, _)| is_prefix(m, import))
            .collect();
        cands.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
        for (module, version) in cands {
            let (root, local) = self.module_dir(module, version);
            let dir = join_import(&root, module, import);
            if has_go_files(fs, &dir) {
                let go_mod = root.join("go.mod");
                let go_mod = fs.is_file(&go_mod).then_some(go_mod);
                return Some(PkgLoc::Dir {
                    dir,
                    module: (!local).then(|| (module.clone(), version.clone())),
                    go_mod,
                });
            }
        }
        None
    }

    /// The modules of `versions` that can provide `import` (their path is a
    /// prefix of it) and whose directory is missing from the module cache,
    /// longest path first. A module that is on disk (or replaced by a local
    /// directory) and lacks the package does not provide it; one that was
    /// never downloaded may. The directory lookups are recorded, so a
    /// module cache that appears later re-resolves the import.
    fn not_downloaded(
        &self,
        fs: &DirCache,
        import: &str,
        versions: &HashMap<String, String>,
    ) -> Vec<(String, String, PathBuf)> {
        let mut cands: Vec<(&String, &String)> = versions
            .iter()
            .filter(|(m, _)| is_prefix(m, import))
            .collect();
        cands.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));
        let mut out = Vec::new();
        for (module, version) in cands {
            let (root, local) = self.module_dir(module, version);
            if local || fs.is_dir(&root) {
                continue;
            }
            // The identity is the one the module's files would have: after `replace M => N
            // W`, the directory (and so the identity) is `N@W`.
            let (m, v) = match modcache_module(&self.modcache, &root.join("go.mod")) {
                Some((_, m, v)) => (m, v),
                None => (module.clone(), version.clone()),
            };
            out.push((m, v, root));
        }
        out
    }

    /// Minimal version selection over the module graph, starting from the
    /// requirements. Reads each module version's `go.mod` from the module
    /// cache download directory (or the module directory / a local
    /// replacement). Recording is paused: module cache contents are
    /// immutable and keyed by the versions in the recorded `go.mod` files.
    fn build_list(&self, fs: &DirCache) -> HashMap<String, String> {
        const MAX_MODULES: usize = 20_000;
        turborepo_trace_core::record::pause(|| {
            let mut sel: HashMap<String, String> = self.requires.clone();
            let mut queue: VecDeque<(String, String)> =
                sel.iter().map(|(m, v)| (m.clone(), v.clone())).collect();
            let mut visited = 0usize;
            while let Some((m, v)) = queue.pop_front() {
                visited += 1;
                if visited > MAX_MODULES {
                    break;
                }
                if sel.get(&m) != Some(&v) {
                    continue; // superseded by a higher version
                }
                let Some(gm) = self.read_module_go_mod(fs, &m, &v) else {
                    continue;
                };
                for r in &gm.requires {
                    if self
                        .excludes
                        .iter()
                        .any(|(p, ver)| *p == r.path && *ver == r.version)
                    {
                        continue;
                    }
                    let higher = sel
                        .get(&r.path)
                        .is_none_or(|cur| compare_versions(&r.version, cur).is_gt());
                    if higher {
                        sel.insert(r.path.clone(), r.version.clone());
                        queue.push_back((r.path.clone(), r.version.clone()));
                    }
                }
            }
            sel
        })
    }

    fn read_module_go_mod(&self, fs: &DirCache, module: &str, version: &str) -> Option<GoMod> {
        let (root, local) = self.module_dir(module, version);
        let mut candidates = Vec::new();
        if !local {
            let (m, v) = match self.replacement(module, version) {
                Some((ReplaceTarget::Module { path, version }, _)) => {
                    (path.as_str(), version.as_str())
                }
                _ => (module, version),
            };
            candidates.push(
                self.modcache
                    .join("cache/download")
                    .join(escape(m))
                    .join("@v")
                    .join(format!("{}.mod", escape(v))),
            );
        }
        candidates.push(root.join("go.mod"));
        candidates.into_iter().find_map(|p| {
            let bytes = fs.read_cached(&p).ok()?;
            gosum::parse_go_mod(std::str::from_utf8(&bytes).ok()?).ok()
        })
    }
}

/// The module directory containing `path` inside the module cache `modcache`:
/// the root (`<modcache>/<escaped path>@<escaped version>`) and the unescaped
/// module path and version.
pub fn modcache_module(modcache: &Path, path: &Path) -> Option<(PathBuf, String, String)> {
    let rel = path.strip_prefix(modcache).ok()?;
    let mut root = modcache.to_path_buf();
    let mut elems: Vec<String> = Vec::new();
    for c in rel.components() {
        let s = c.as_os_str().to_str()?;
        root.push(s);
        if let Some((last, version)) = s.split_once('@') {
            if elems.first().is_some_and(|e| e == "cache") {
                return None;
            }
            elems.push(last.to_owned());
            let module = unescape(&elems.join("/"))?;
            return Some((root, module, unescape(version)?));
        }
        elems.push(s.to_owned());
    }
    None
}

/// Whether module path `module` is a path prefix of `import`.
pub fn is_prefix(module: &str, import: &str) -> bool {
    import == module
        || (import.len() > module.len()
            && import.starts_with(module)
            && import.as_bytes()[module.len()] == b'/')
}

/// The directory of `import` inside module `module` rooted at `root`.
pub fn join_import(root: &Path, module: &str, import: &str) -> PathBuf {
    let rest = import[module.len().min(import.len())..].trim_start_matches('/');
    if rest.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rest)
    }
}

/// Whether `dir` exists and contains at least one `.go` file.
pub fn has_go_files(fs: &DirCache, dir: &Path) -> bool {
    if !fs.is_dir(dir) {
        return false;
    }
    let listing = turborepo_trace_core::record::pause_listings(|| fs.list(dir));
    let found = listing.is_some_and(|l| {
        l.iter().any(|e| {
            e.name_bytes().ends_with(b".go")
                && matches!(e.kind(), EntryKind::File | EntryKind::Symlink)
        })
    });
    if fs.records() {
        turborepo_trace_core::record::listing(dir, "*.go");
    }
    found
}

/// Reads and parses a `go.mod` through the dir cache.
pub fn read_go_mod(fs: &DirCache, path: &Path) -> Option<GoMod> {
    let bytes = fs.read_cached(path).ok()?;
    gosum::parse_go_mod(std::str::from_utf8(&bytes).ok()?).ok()
}

fn read_go_work(fs: &DirCache, path: &Path) -> Option<GoWork> {
    let bytes = fs.read_cached(path).ok()?;
    gosum::parse_go_work(std::str::from_utf8(&bytes).ok()?).ok()
}

fn read_sums(fs: &DirCache, paths: &[PathBuf], into: &mut GoSum) {
    for p in paths {
        if let Ok(b) = fs.read_cached(p)
            && let Ok(s) = std::str::from_utf8(&b)
            && let Ok(sum) = gosum::parse_go_sum(s)
        {
            into.entries.extend(sum.entries);
        }
    }
}

/// `go 1.N[.M]` → `N`.
fn go_minor(v: Option<&str>) -> u32 {
    v.and_then(|v| v.strip_prefix("1."))
        .and_then(|r| r.split('.').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn load_vendor(fs: &DirCache, root: &Path) -> Option<Vendor> {
    let modules_txt = root.join("vendor/modules.txt");
    let bytes = fs.read_cached(&modules_txt).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let mut packages = HashMap::new();
    for m in parse_modules_txt(text) {
        for p in m.packages {
            packages.insert(p, (m.path.clone(), m.version.clone()));
        }
    }
    Some(Vendor {
        dir: root.join("vendor"),
        modules_txt,
        packages,
    })
}

/// Adds the `go.mod` of every module that `replaces` (written in a file in
/// `base`) points at by directory. Such a module is part of the main module's
/// module graph: its requirements enter the build list, and when they disagree
/// with the main `go.mod` the `go` command refuses to load any package
/// ("updates to go.mod needed"). So its `go.mod` is an input of every file of
/// the context, not only of the files that import its packages.
fn local_replace_manifests(
    fs: &DirCache,
    replaces: &[GoReplace],
    base: &Path,
    manifests: &mut Vec<PathBuf>,
) {
    for r in replaces {
        if let ReplaceTarget::Local(d) = &r.new {
            let m = normalize(&base.join(d)).join("go.mod");
            if fs.is_file(&m) && !manifests.contains(&m) {
                manifests.push(m);
            }
        }
    }
}

/// Builds the context of a single main module rooted at `root`.
pub fn module_ctx(fs: &DirCache, root: &Path, vendor: VendorMode, modcache: &Path) -> Ctx {
    let go_mod_path = root.join("go.mod");
    let gm = read_go_mod(fs, &go_mod_path).unwrap_or_default();
    let mut requires = HashMap::new();
    for r in &gm.requires {
        requires.insert(r.path.clone(), r.version.clone());
    }
    let replaces = gm
        .replaces
        .iter()
        .map(|r| (r.clone(), root.to_path_buf()))
        .collect();
    let mut manifests = Vec::new();
    let go_sum = root.join("go.sum");
    let mut sums = GoSum::default();
    if fs.is_file(&go_sum) {
        read_sums(fs, std::slice::from_ref(&go_sum), &mut sums);
        manifests.push(go_sum);
    }
    let vendor = match vendor {
        VendorMode::Off => None,
        VendorMode::On => load_vendor(fs, root),
        VendorMode::Auto if go_minor(gm.go.as_deref()) >= 14 => load_vendor(fs, root),
        VendorMode::Auto => None,
    };
    if let Some(v) = &vendor {
        manifests.push(v.modules_txt.clone());
    }
    local_replace_manifests(fs, &gm.replaces, root, &mut manifests);
    Ctx {
        key: go_mod_path,
        main: vec![ModuleDir {
            path: gm.module.clone().unwrap_or_default(),
            dir: root.to_path_buf(),
        }],
        requires,
        replaces,
        excludes: gm.excludes.clone(),
        vendor,
        sums,
        manifests,
        modcache: modcache.to_path_buf(),
        mvs: OnceLock::new(),
    }
}

/// Builds the context of the workspace defined by `go_work`.
pub fn work_ctx(fs: &DirCache, go_work: &Path, vendor: VendorMode, modcache: &Path) -> Ctx {
    let root = go_work.parent().unwrap_or(Path::new("/")).to_path_buf();
    let w = read_go_work(fs, go_work).unwrap_or_default();
    let mut main = Vec::new();
    let mut requires: HashMap<String, String> = HashMap::new();
    let mut replaces: Vec<(GoReplace, PathBuf)> = w
        .replaces
        .iter()
        .map(|r| (r.clone(), root.clone()))
        .collect();
    let mut excludes = Vec::new();
    let mut sum_files = Vec::new();
    // The `go.mod` of every workspace module: the build list is the union of their
    // requirements, so each is an input of every file of the workspace.
    let mut mod_files = Vec::new();
    for u in &w.uses {
        let dir = normalize(&root.join(u));
        let gm = read_go_mod(fs, &dir.join("go.mod")).unwrap_or_default();
        if fs.is_file(&dir.join("go.mod")) {
            mod_files.push(dir.join("go.mod"));
        }
        local_replace_manifests(fs, &gm.replaces, &dir, &mut mod_files);
        for r in &gm.requires {
            let e = requires.entry(r.path.clone()).or_insert(r.version.clone());
            if compare_versions(&r.version, e).is_gt() {
                *e = r.version.clone();
            }
        }
        replaces.extend(gm.replaces.iter().map(|r| (r.clone(), dir.clone())));
        excludes.extend(gm.excludes.iter().cloned());
        let s = dir.join("go.sum");
        if fs.is_file(&s) {
            sum_files.push(s);
        }
        main.push(ModuleDir {
            path: gm.module.clone().unwrap_or_default(),
            dir,
        });
    }
    // Workspace modules are not resolved through requirements on each other.
    for m in &main {
        requires.remove(&m.path);
    }
    main.sort_by(|a, b| b.path.len().cmp(&a.path.len()).then(a.path.cmp(&b.path)));
    let mut manifests = vec![go_work.to_path_buf()];
    let work_sum = root.join("go.work.sum");
    if fs.is_file(&work_sum) {
        sum_files.push(work_sum.clone());
        manifests.push(work_sum);
    }
    let mut sums = GoSum::default();
    read_sums(fs, &sum_files, &mut sums);
    let vendor = match vendor {
        VendorMode::Off => None,
        VendorMode::On => load_vendor(fs, &root),
        VendorMode::Auto if go_minor(w.go.as_deref()) >= 22 => load_vendor(fs, &root),
        VendorMode::Auto => None,
    };
    if let Some(v) = &vendor {
        manifests.push(v.modules_txt.clone());
    }
    local_replace_manifests(fs, &w.replaces, &root, &mut mod_files);
    for m in mod_files {
        if !manifests.contains(&m) {
            manifests.push(m);
        }
    }
    Ctx {
        key: go_work.to_path_buf(),
        main,
        requires,
        replaces,
        excludes,
        vendor,
        sums,
        manifests,
        modcache: modcache.to_path_buf(),
        mvs: OnceLock::new(),
    }
}

/// The parsed `use` directories of a `go.work`, absolute.
pub fn work_uses(fs: &DirCache, go_work: &Path) -> Vec<PathBuf> {
    let root = go_work.parent().unwrap_or(Path::new("/"));
    read_go_work(fs, go_work)
        .map(|w| w.uses.iter().map(|u| normalize(&root.join(u))).collect())
        .unwrap_or_default()
}

/// The default module cache: `$GOMODCACHE`, else `$GOPATH[0]/pkg/mod`, else
/// `$HOME/go/pkg/mod`.
pub fn default_modcache() -> PathBuf {
    if let Some(m) = std::env::var_os("GOMODCACHE").filter(|v| !v.is_empty()) {
        return PathBuf::from(m);
    }
    if let Some(gp) = std::env::var_os("GOPATH").filter(|v| !v.is_empty())
        && let Some(first) = std::env::split_paths(&gp).next()
    {
        return first.join("pkg/mod");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
        .join("go/pkg/mod")
}

/// The default `GOROOT`: `$GOROOT`, else the first common install location that
/// holds `src/fmt`. `None` if none is found (the bundled standard library list
/// is used).
pub fn default_goroot() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(g) = std::env::var_os("GOROOT").filter(|v| !v.is_empty()) {
        cands.push(PathBuf::from(g));
    }
    cands.extend(
        [
            "/usr/local/go",
            "/usr/lib/go",
            "/opt/homebrew/opt/go/libexec",
            "/usr/local/opt/go/libexec",
        ]
        .map(PathBuf::from),
    );
    if let Some(h) = std::env::var_os("HOME") {
        cands.push(PathBuf::from(&h).join("go-sdk"));
        cands.push(PathBuf::from(&h).join("sdk/go"));
    }
    cands.into_iter().find(|g| g.join("src/fmt").is_dir())
}

/// Every module version listed in a context (requirements plus vendored
/// modules), sorted: what opaque third-party package identities are keyed by.
pub fn listed_modules(ctx: &Ctx) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = ctx
        .requires
        .iter()
        .map(|(m, v)| (m.clone(), v.clone()))
        .collect();
    if let Some(v) = &ctx.vendor {
        for (m, ver) in v.packages.values() {
            out.entry(m.clone()).or_insert_with(|| ver.clone());
        }
    }
    out
}

/// Shared handle type.
pub type CtxRef = Arc<Ctx>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(
            escape("github.com/Azure/azure-sdk"),
            "github.com/!azure/azure-sdk"
        );
        assert_eq!(
            unescape("github.com/!azure/azure-sdk").as_deref(),
            Some("github.com/Azure/azure-sdk")
        );
        assert_eq!(unescape("bad/!"), None);
        assert_eq!(unescape("Bad"), None);
        assert_eq!(escape("v1.0.0-RC1"), "v1.0.0-!r!c1");
    }

    #[test]
    fn modcache_paths() {
        let mc = Path::new("/h/go/pkg/mod");
        let (root, m, v) = modcache_module(
            mc,
            Path::new("/h/go/pkg/mod/github.com/!azure/x@v1.0.0-!r!c/a/b.go"),
        )
        .unwrap();
        assert_eq!(
            root,
            Path::new("/h/go/pkg/mod/github.com/!azure/x@v1.0.0-!r!c")
        );
        assert_eq!(
            (m.as_str(), v.as_str()),
            ("github.com/Azure/x", "v1.0.0-RC")
        );
        assert!(
            modcache_module(mc, Path::new("/h/go/pkg/mod/cache/download/x/@v/v1.mod")).is_none()
        );
        assert!(modcache_module(mc, Path::new("/src/a.go")).is_none());
    }

    #[test]
    fn prefixes() {
        assert!(is_prefix("github.com/a/b", "github.com/a/b"));
        assert!(is_prefix("github.com/a/b", "github.com/a/b/c"));
        assert!(!is_prefix("github.com/a/b", "github.com/a/bc"));
        assert_eq!(
            join_import(Path::new("/m"), "x.io/m", "x.io/m/a/b"),
            Path::new("/m/a/b")
        );
        assert_eq!(
            join_import(Path::new("/m"), "x.io/m", "x.io/m"),
            Path::new("/m")
        );
    }

    #[test]
    fn std_detection() {
        assert!(is_bundled_std("fmt"));
        assert!(is_bundled_std("net/http"));
        assert!(!is_bundled_std("github.com/a/b"));
        assert!(is_std_like("fmt"));
        assert!(!is_std_like("example.com/x"));
        let mut sorted = STD_PACKAGES.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STD_PACKAGES, "stdlib list must stay sorted");
    }

    #[test]
    fn modules_txt() {
        let m = parse_modules_txt(
            "# github.com/a/b v1.2.3\n## explicit; go 1.20\ngithub.com/a/b\ngithub.com/a/b/c\n# \
             k8s.io/api v0.0.0 => ./staging/src/k8s.io/api\n## explicit\nk8s.io/api/core/v1\n# \
             example.com/x => ../x\n",
        );
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].packages, ["github.com/a/b", "github.com/a/b/c"]);
        assert_eq!(m[1].version, "v0.0.0");
        assert_eq!(m[2].version, "");
    }

    #[test]
    fn go_minor_versions() {
        assert_eq!(go_minor(Some("1.22.0")), 22);
        assert_eq!(go_minor(Some("1.13")), 13);
        assert_eq!(go_minor(None), 0);
    }
}
