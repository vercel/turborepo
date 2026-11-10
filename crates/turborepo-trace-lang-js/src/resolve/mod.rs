//! JS / TS resolution.
//!
//! Module references (`import`, `export ... from`, `import()`, `require()`) are
//! resolved with [`oxc_resolver`] running on the tracer's shared fs cache
//! ([`CachedFs`]):
//!
//! - Conditions: [`JsResolveOptions::conditions`], else `--conditions`, else
//!   `node, import, require, default`. ESM references (`Import`,
//!   `DynamicImport`, `ReExport`) drop `require` from the set and CommonJS ones
//!   (`Require`, workers, forks) drop `import`.
//! - Extensions `.js .mjs .cjs .ts .tsx .mts .cts .jsx .json .node`, `index`
//!   files, `main` fields ([`JsResolveOptions::main_fields`]), `exports`,
//!   `imports` (`#internal`) and package self-reference.
//! - TS files get TypeScript's preferences: `.ts` before `.js` and the `.js ->
//!   .ts` extension alias. JS files keep Node's preferences but fall back from
//!   a missing `./x.js` to `./x.ts`.
//! - `tsconfig.json` / `jsconfig.json` `paths` and `baseUrl` apply to JS and TS
//!   files alike, as in Next.js and other bundlers: the config is the one whose
//!   `files`/`include` covers the file (with `allowJs` for JS), through
//!   `references`, else the nearest config in the file's package
//!   ([`JsResolveOptions::tsconfig_fallback`]). See [`tsconfig`]. A project
//!   reference or `extends` that cannot be loaded is skipped and reported once
//!   per config as [`UncertaintyKind::ParseIncomplete`] (the config could not
//!   be read).
//! - Node builtins (`fs`, `node:fs`, `node:test`) become [`Target::Builtin`] as
//!   `node:<name>`.
//! - `module-sync` (Node >= 22 matches it for `import` and `require`, older
//!   versions do not): when the mapping manifest (the package root's for a bare
//!   specifier, the referencing file's scope for `#imports` and
//!   self-references) has the condition, the specifier is resolved once more
//!   with it toggled and both results are kept, as nft does.
//! - With the `browser` condition, the `browser` field object of the target's
//!   package remaps files (`"./lib/node.js": "./lib/browser.js"`); the remapped
//!   file is added next to the original, as nft does.
//! - A path the extractor anchored at a package's resolved file
//!   ([`crate::extract::evaluator::RESOLVED_BASE`],
//!   `path.join(require.resolve('pkg'), '../x.wasm')`, `import(new
//!   URL('./x.js', import.meta.resolve('pkg')))`, `require.resolve('dep', {
//!   paths: [...] })`) resolves the package first, from the file or from the
//!   place the reference names; a read of a directory there is not expanded
//!   (reported as too broad), a module load is the file or a glob below it
//!   (`located`).
//! - An asset path value from the extractor's deep pass that names a directory
//!   expands to every file below it, with a `PartialSpecifier` record (the
//!   whole directory is a guess).
//! - Unless [`JsResolveOptions::exports_only`] is set, a bare specifier into a
//!   package with `exports` also resolves the legacy `main`/file path and emits
//!   both, as nft does (older runtimes and bundlers ignore `exports`).
//! - Failures become [`Target::Unresolved`] with
//!   [`UncertaintyKind::UnresolvedSpecifier`]; relative specifiers that only
//!   match with different casing report [`UncertaintyKind::CaseMismatch`].
//! - `--externals stop|list`: a bare specifier that lands in a third-party
//!   package becomes [`Target::External`] with the package name and is not
//!   traversed. Third party means the package's real path is inside a
//!   `node_modules` directory (pnpm's `node_modules/.pnpm/...`, a hoisted copy)
//!   or outside the workspace (the nearest ancestor with `pnpm-workspace.yaml`
//!   or a `package.json` declaring `workspaces`, else `--base`). Workspace
//!   packages linked into `node_modules` (pnpm `workspace:`, npm, yarn and bun
//!   workspaces) are first party and traversed, as in the store
//!   (`turborepo_trace_store::normalize`).
//!
//! Symlinks are not resolved here (`oxc_resolver`'s `symlinks: false`): core
//! turns every target into its real path and records the links crossed, so pnpm
//! layouts work and there is one canonicalization for all languages.
//!
//! Manifests (nft behavior): every resolution emits the nearest `package.json`
//! of the referencing file (Node reads it for `type`), of the resolved file,
//! and, for bare specifiers, the package root's `package.json` (read for
//! `exports`/`main`). They are computed from the dir cache rather than from
//! oxc's read log, because oxc caches parsed manifests and only reports a read
//! the first time, which would make output depend on thread scheduling.
//!
//! Type-only references never reach the resolver unless `--include-types` is
//! set (core filters them). A target that is a `.d.ts` file is excluded by
//! default and reported as unresolved, since nothing exists at runtime.
//!
//! Non-module references resolve as paths relative to their
//! [`turborepo_trace_core::Base`] (`FileDir`, `Cwd` = `--cwd`, `ProjectRoot` =
//! `--base`, absolute literals), following nft. The logic is shared by all
//! languages ([`turborepo_trace_core::resolve_path::assets`]); JS configures it
//! with [`assets::JS_ASSET_RULES`]:
//!
//! - `FsRead`, `FsReadDir`, `Asset`: an existing file is a leaf with reason
//!   `asset`; an existing directory expands to every file below it (`<dir>/**`,
//!   nested `node_modules` skipped, capped by `--max-glob`) unless it is the
//!   file's own directory (allowed for `FsReadDir`, like nft's
//!   `readdirSync(__dirname)`), the cwd, a `node_modules` directory or an
//!   ancestor. A missing path is normal for probes (`existsSync`): it is kept
//!   as a quiet [`Target::Unresolved`] edge without uncertainty. Files inside
//!   `node_modules` never emit assets outside their `node_modules` tree (nft's
//!   package boundary), also quietly.
//! - Patterns (`'../data/' + x + '.json'`) expand with
//!   [`turborepo_trace_core::eval`] globs against the dir cache: never from a
//!   leading wildcard, capped by `--max-glob`
//!   ([`UncertaintyKind::PatternTooBroad`] when truncated), nested
//!   `node_modules` skipped. `<own dir>/*`, `<cwd>/*` and the like are not
//!   expanded (nft's `validWildcard`) and reported as too broad.
//! - Module kinds with patterns (`require('./locales/' + x)`, like webpack
//!   context modules and nft wildcard requires) glob the same way, also trying
//!   each resolver extension when the pattern does not end in a wildcard; every
//!   match is followed (parsed if code). A bare package prefix (`require('pkg/'
//!   + x)`) globs inside that package. No match is
//!   [`UncertaintyKind::UnresolvedSpecifier`].
//! - `Worker` and `ChildProcess` resolve like `require` from their base (Node
//!   runs them through the CommonJS loader), so extensions and `index` files
//!   are probed.
//! - Alternatives resolve each branch and include every result.
//! - An uncertainty from the extractor (`CwdDependent`, `EnvDependent`, ...) is
//!   kept; a successful glob adds its match count to its detail.
//!
//! Package special cases and native addons: see [`crate::special`]. Every
//! `.node` target also pulls in the shared libraries shipped in its package
//! (reason `sharedlib`, through the shared [`SharedLibs`]).
//!
//! Graph mode ([`turborepo_trace_core::record`]): while a recording frame is
//! open, oxc runs through `resolve_with_context`, whose per-resolution
//! `missing_dependencies` log is complete even when oxc's own caches are warm;
//! it replaces the dir cache's view of the same calls (paused). Paths in that
//! log that exist (a directory probed as a file) are dropped, and so
//! are its `package.json` entries, which oxc only reports on a cold cache.
//! Instead the resolver records `package.json` lookups deterministically: the
//! nearest-manifest chain of the referencing file and of each target, a probed
//! directory's own `package.json`, and the manifest of the package a bare
//! specifier lands in. JS and TS files outside `node_modules` also record their
//! config discovery chain (`tsconfig.json`, else `jsconfig.json`, in every
//! ancestor), `extends` files and project references: what the load of a config
//! read is what the loader asked of the file system, each load in a loader
//! of its own ([`tsconfig`], "What a load is computed from"). Pattern
//! globs record their precise `(static directory, pattern)` memberships instead
//! of every directory the glob listed.
//!
//! With `--all-conditions`
//! ([`turborepo_trace_core::TraceOptions::all_conditions`]), a bare
//! specifier into a package with `exports` additionally resolves under each
//! condition the package defines (alone and in pairs, for nested condition
//! objects) and includes every result.

pub mod assets;
mod fs;
pub mod governance;
mod located;
#[cfg(test)]
mod located_tests;
mod overlay;
mod tsconfig;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use assets::JS_ASSET_RULES;
use dashmap::DashMap;
pub use fs::CachedFs;
use oxc_resolver::{
    ResolveContext, ResolveError, ResolveOptions, ResolverGeneric, TsConfig, TsconfigDiscovery,
};
use tsconfig::Tsconfigs;
use turborepo_trace_core::{
    Base, DirCache, Externals, ExtraEdge, FileId, Part, ProjectCx, RawRef, RefFlags, RefKind,
    Resolution, ResolveCx, Resolver, Specifier, Target, Uncertainty, UncertaintyKind,
    eval::PatPart,
    fs::normalize,
    os::OsStrExt,
    record,
    resolve_path::{
        PathResolver, PathResolverConfig, SharedLibs, SpecKind,
        assets::{
            computed_specifier, dedup_ids as dedup, membership_patterns, refused_module_pattern,
            resolve_value_alternatives, unsupported_base,
        },
        spec_kind,
    },
};

use crate::{
    extract::{MODULE_STRING_BASE, evaluator::RESOLVED_BASE},
    special::{Platform, Special},
};

/// Default `exports` conditions.
pub const DEFAULT_CONDITIONS: &[&str] = &["node", "import", "require", "default"];

/// Extensions probed for JS files, in Node's preference order.
const JS_EXTENSIONS: &[&str] = &[
    ".js", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts", ".jsx", ".json", ".node",
];

/// Extensions probed for TS files: TypeScript prefers sources over emitted JS.
const TS_EXTENSIONS: &[&str] = &[
    ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json", ".node",
];

/// Options for [`JsResolver`] beyond the tracer-wide
/// [`turborepo_trace_core::TraceOptions`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsResolveOptions {
    /// `exports`/`imports` conditions. `None` uses `--conditions` if given,
    /// else [`DEFAULT_CONDITIONS`].
    pub conditions: Option<Vec<String>>,
    /// `package.json` fields naming the entry point, in order. Default
    /// `["main"]`, as Node.
    pub main_fields: Vec<String>,
    /// Only use `exports` for packages that define it (nft's `exportsOnly`).
    /// Default `false`: the legacy `main`/file resolution is emitted as
    /// well.
    pub exports_only: bool,
    /// Discover `tsconfig.json` / `jsconfig.json` per file (JS and TS) for
    /// `paths`/`baseUrl`. Default `true`.
    pub tsconfig: bool,
    /// When no config covers a file through `files`/`include` (and `allowJs`
    /// for JS files), use the nearest config at or below the file's package
    /// root anyway, as Next.js does for every module of an app. Default
    /// `true`. With `false`, only TypeScript's ownership rules apply (like
    /// vite-tsconfig-paths without `loose`).
    pub tsconfig_fallback: bool,
    /// Platform for native binaries and package special cases. `None` uses the
    /// tracer's
    /// platform ([`turborepo_trace_core::TraceOptions::platform()`]: `--target`
    /// if it parses, else the host).
    pub platform: Option<Platform>,
}

impl Default for JsResolveOptions {
    fn default() -> Self {
        JsResolveOptions {
            conditions: None,
            main_fields: vec!["main".into()],
            exports_only: false,
            tsconfig: true,
            tsconfig_fallback: true,
            platform: None,
        }
    }
}

/// TypeScript's `.js -> .ts` mapping. TS files prefer the TS source; JS files
/// prefer the JS file (Node semantics) but fall back to a TS source rather than
/// failing (allowJs projects, bundlers), following the over-approximation
/// principle.
fn extension_alias(ts: bool) -> Vec<(String, Vec<String>)> {
    let pairs: [(&str, &[&str]); 4] = if ts {
        [
            (".js", &[".ts", ".tsx", ".js", ".jsx"]),
            (".jsx", &[".tsx", ".jsx"]),
            (".mjs", &[".mts", ".mjs"]),
            (".cjs", &[".cts", ".cjs"]),
        ]
    } else {
        [
            (".js", &[".js", ".ts", ".tsx", ".jsx"]),
            (".jsx", &[".jsx", ".tsx"]),
            (".mjs", &[".mjs", ".mts"]),
            (".cjs", &[".cjs", ".cts"]),
        ]
    };
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
        .collect()
}

/// Module system of a reference.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum System {
    Esm,
    Cjs,
}

type Oxc = ResolverGeneric<CachedFs>;

/// The JS / TS resolver. Shares one oxc cache across all variants.
pub struct JsResolver {
    /// Indexed by `[system][is_ts]`.
    oxc: [[Oxc; 2]; 2],
    /// `exports` ignored, for the legacy resolution (`None` with
    /// `exports_only`).
    legacy: Option<Oxc>,
    /// Case-mismatch diagnosis for failed relative specifiers.
    case_probe: PathResolver,
    externals: Externals,
    include_types: bool,
    /// Nearest `package.json` per directory.
    nearest_pkg: DashMap<Box<[u8]>, Option<Arc<Path>>>,
    /// Package special cases and native addons.
    special: Special,
    /// Shared libraries next to native addons.
    sharedlibs: SharedLibs,
    /// `--all-conditions`.
    all_conditions: bool,
    /// Per-file `tsconfig.json` / `jsconfig.json` discovery (`None` without
    /// `tsconfig`).
    tsconfigs: Option<Tsconfigs>,
    /// Configs whose load failures were already reported.
    reported_configs: DashMap<PathBuf, ()>,
    /// Workspace root per directory holding a `node_modules` (`--externals
    /// stop|list`).
    workspace_roots: DashMap<PathBuf, PathBuf>,
    /// Resolvers for `--all-conditions` and `module-sync`, by `(conditions,
    /// system is CJS, is TS)`.
    cond_variants: DashMap<(Vec<String>, bool, bool), Oxc>,
    /// `browser` is one of the conditions: apply `package.json` `browser`
    /// remappings.
    browser: bool,
}

impl std::fmt::Debug for JsResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsResolver")
            .field("options", self.oxc[0][0].options())
            .finish_non_exhaustive()
    }
}

impl JsResolver {
    /// Creates a resolver for one tracer.
    pub fn new(project: &ProjectCx<'_>, opts: &JsResolveOptions) -> Self {
        let trace = project.options;
        let platform = opts.platform.unwrap_or_else(|| trace.platform());
        let base_conditions: Vec<String> = match &opts.conditions {
            Some(c) => c.clone(),
            None if !trace.conditions.is_empty() => trace.conditions.clone(),
            None => DEFAULT_CONDITIONS.iter().map(|s| s.to_string()).collect(),
        };
        let browser = base_conditions.iter().any(|c| c == "browser");
        let without = |drop: &str| -> Vec<String> {
            base_conditions
                .iter()
                .filter(|c| *c != drop)
                .cloned()
                .collect()
        };
        let exts = |list: &[&str], dts: bool| -> Vec<String> {
            let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
            if dts {
                v.push(".d.ts".into());
            }
            v
        };
        let base = ResolveOptions {
            condition_names: without("import"),
            extensions: exts(JS_EXTENSIONS, false),
            main_fields: opts.main_fields.clone(),
            main_files: vec!["index".into()],
            builtin_modules: true,
            symlinks: false,
            node_path: false,
            tsconfig: None,
            ..ResolveOptions::default()
        };
        let root =
            Oxc::new_with_file_system(CachedFs::new_shared(Arc::clone(project.fs)), base.clone());
        let variant = |system: System, ts: bool| -> Oxc {
            let mut o = base.clone();
            o.condition_names = match system {
                System::Esm => without("require"),
                System::Cjs => without("import"),
            };
            o.extension_alias = extension_alias(ts);
            if ts {
                o.extensions = exts(TS_EXTENSIONS, trace.include_types);
            }
            // `paths` are only applied when a tsconfig option is set; the config itself is
            // discovered by `Tsconfigs` and passed per resolution (`resolve_with_context`).
            // `Auto` makes oxc use the passed config as is, and `resolve()` ignores it.
            if opts.tsconfig {
                o.tsconfig = Some(TsconfigDiscovery::Auto);
            }
            root.clone_with_options(o)
        };
        // Configs are loaded by loaders of their own, on the same file system
        // ([`Tsconfigs::load`]).
        let tsconfigs = opts.tsconfig.then(|| {
            Tsconfigs::new(
                CachedFs::new_shared(Arc::clone(project.fs)),
                opts.tsconfig_fallback,
            )
        });
        let legacy = (!opts.exports_only).then(|| {
            let mut o = base.clone();
            o.exports_fields = Vec::new();
            root.clone_with_options(o)
        });
        JsResolver {
            oxc: [
                [variant(System::Esm, false), variant(System::Esm, true)],
                [variant(System::Cjs, false), variant(System::Cjs, true)],
            ],
            legacy,
            case_probe: PathResolver::new(PathResolverConfig {
                extensions: JS_EXTENSIONS
                    .iter()
                    .map(|e| e.trim_start_matches('.').to_string())
                    .collect(),
                index_files: JS_EXTENSIONS.iter().map(|e| format!("index{e}")).collect(),
                detect_case_mismatch: true,
                ..Default::default()
            }),
            externals: trace.externals,
            include_types: trace.include_types,
            nearest_pkg: DashMap::new(),
            special: Special::new(platform),
            sharedlibs: SharedLibs::new(&platform, JS_ASSET_RULES),
            all_conditions: trace.all_conditions,
            tsconfigs,
            reported_configs: DashMap::new(),
            workspace_roots: DashMap::new(),
            cond_variants: DashMap::new(),
            browser,
        }
    }

    /// The platform native binaries are selected for.
    pub fn platform(&self) -> &Platform {
        self.special.platform()
    }

    fn oxc(&self, system: System, from: &Path) -> &Oxc {
        let ts = is_ts_file(from);
        &self.oxc[(system == System::Cjs) as usize][ts as usize]
    }

    /// The nearest `package.json` at or above `dir`, answered from the dir
    /// cache and memoized per directory.
    ///
    /// While recording, the lookup chain is reported on every call (the memo
    /// would otherwise make it depend on which file asked first): each
    /// `package.json` missing on the way up as a probe, and the one found
    /// as a read.
    fn nearest_package_json(&self, fs: &DirCache, dir: &Path) -> Option<Arc<Path>> {
        if !fs.records() || !record::is_recording() {
            return self.nearest_package_json_memo(fs, dir);
        }
        let found = record::pause(|| self.nearest_package_json_memo(fs, dir));
        let stop = found.as_deref().and_then(Path::parent);
        let mut cur = Some(dir);
        while let Some(d) = cur {
            if Some(d) == stop {
                break;
            }
            record::missing(&d.join("package.json"));
            cur = d.parent();
        }
        // Whoever reads the manifest for resolution gets its view
        // ([`turborepo_trace_core::manifest`], `DirCache::read_manifest`).
        if let Some(m) = &found {
            record::view_read(m);
        }
        found
    }

    fn nearest_package_json_memo(&self, fs: &DirCache, dir: &Path) -> Option<Arc<Path>> {
        let key = dir.as_os_str().as_bytes();
        if let Some(r) = self.nearest_pkg.get(key) {
            return r.clone();
        }
        let cand = dir.join("package.json");
        let found = if fs.is_file(&cand) {
            Some(Arc::from(cand))
        } else {
            dir.parent()
                .and_then(|p| self.nearest_package_json_memo(fs, p))
        };
        self.nearest_pkg.insert(key.into(), found.clone());
        found
    }

    /// Records the config discovery of a file ([`tsconfig`]): discovery walks
    /// up from the file's directory to the first config that covers it, and
    /// the fallback depends on the configs below the package root, so every
    /// ancestor's `tsconfig.json` (else `jsconfig.json`) is recorded, as a
    /// probe when missing and as a read (with its `extends` chain and
    /// project references) when present. Records what the choice of the
    /// file's governing tsconfig depends on (P13): exactly the levels
    /// discovery consulted ([`Tsconfigs::walk`]), not every ancestor.
    ///
    /// - the governing config and its `extends` chain: content inputs (they
    ///   decide `paths`/`baseUrl`); when a project reference governs, also the
    ///   config listing it (dropping the reference changes the choice). "The
    ///   chain" is everything the load of the config read and every place it
    ///   looked at ([`tsconfig::DirConfig::observed`]): the configs, the
    ///   manifests a package request went through, the candidates that were not
    ///   there;
    /// - the config of every level the walk passed (it did not cover the file,
    ///   but an edit could make it): content inputs;
    /// - missing config candidates at the consulted levels: probes;
    /// - project references that were checked and did not cover the file:
    ///   probes of the places their load looked at and found empty (a file that
    ///   appears there is noticed), and no input: editing a referenced
    ///   package's tsconfig selects nothing through the files below the
    ///   referencing config;
    /// - levels above the governing one: nothing (discovery never looked there,
    ///   and [`JsResolver::config_errors`] reports nothing from there).
    ///
    /// What discovery loaded at a consulted level without the file's result
    /// reading it is a **key observation** ([`record::key`]): the file's
    /// records hold as long as it stays as it is, and no query selects
    /// through it. Whether a referenced project loads is reported on every
    /// file that consults the level, and which files it includes decides
    /// whether it governs them, so the project's config and its `extends` chain
    /// are key observations of a file it does not govern (a project that
    /// comes to be broken, or to include the file, is seen by a refresh);
    /// so are the references after the governing one, and the `extends`
    /// chain of a config whose reference governs.
    ///
    /// All of it is the same for every file that consults a directory with the
    /// same governing reference: it is observed once
    /// ([`level_observations`]), the inputs are replayed and the key
    /// observations are one shared set.
    fn record_tsconfig_chain(
        &self,
        tsconfigs: &Tsconfigs,
        walk: &tsconfig::Walk,
        cx: &ResolveCx<'_>,
    ) {
        let (gov_level, gov_ref) = match walk.governing {
            tsconfig::Governing::Level(l) => (Some(l), None),
            tsconfig::Governing::Reference(l, r) => (Some(l), Some(r)),
            tsconfig::Governing::Fallback(_) | tsconfig::Governing::None => (None, None),
        };
        for (i, (d, dc)) in walk.levels.iter().enumerate() {
            if gov_level.is_some_and(|g| i > g) {
                break;
            }
            // The reference of this level that governs the file, if one does.
            let governing = gov_ref.filter(|_| gov_level == Some(i));
            let level = tsconfigs.level_record(d, governing, || {
                let (inputs, key) = level_observations(d, dc.as_deref(), governing, cx);
                tsconfig::LevelRecord {
                    inputs,
                    key: Arc::new(key),
                }
            });
            record::replay(&level.inputs);
            record::key(&level.key);
        }
    }

    /// The config governing the referencing file when the resolver does not
    /// apply its `paths`, `baseUrl` or `rootDirs` to it although the runner
    /// does: the file is a file of the repository ([`in_node_modules`] is
    /// false) in a checkout below a directory called `node_modules`
    /// (`/ci/node_modules/app`). oxc takes every file below such a
    /// directory, by its absolute path, for a file of an installed package, and
    /// resolves its imports without the config's aliases. `None` when the
    /// config has none of them.
    fn aliases_not_applied(&self, cx: &ResolveCx<'_>) -> Option<Arc<TsConfig>> {
        if !cx
            .from_dir
            .components()
            .any(|c| c.as_os_str() == "node_modules")
        {
            return None;
        }
        self.aliasing(cx)
    }

    /// The config governing the referencing file when it gives `paths`,
    /// `baseUrl` or `rootDirs`, whether the resolver applies them or not
    /// ([`record::aliased`]): then where the checkout lies decides the
    /// file's records ([`JsResolver::aliases_not_applied`]). `None` for a
    /// file of an installed package (no config governs it) and for a config
    /// without them.
    fn aliasing(&self, cx: &ResolveCx<'_>) -> Option<Arc<TsConfig>> {
        if in_node_modules(cx.fs, cx.from_path) {
            return None;
        }
        self.governing(cx).filter(|ts| gives_aliases(ts))
    }

    /// The config governing the referencing file, if any.
    fn governing(&self, cx: &ResolveCx<'_>) -> Option<Arc<TsConfig>> {
        let t = self.tsconfigs.as_ref()?;
        record::pause(|| {
            let pkg = self.nearest_package_json_memo(cx.fs, cx.from_dir);
            t.governing(cx.fs, cx.from_path, pkg.as_deref().and_then(Path::parent))
        })
    }

    /// Load failures of the configs a file's discovery consults (the levels of
    /// `walk`: up to the governing config, every level when none covers the
    /// file), as one uncertainty edge per config. A config above the
    /// governing one is not consulted, so its failures say nothing about
    /// the file, and nothing of it is in the file's record. Outside graph
    /// mode each config is reported once per trace, on the first file that
    /// consults it; in graph mode on every such file, so that per-file
    /// results do not depend on scheduling.
    fn config_errors(&self, walk: &tsconfig::Walk, cx: &ResolveCx<'_>, out: &mut Vec<ExtraEdge>) {
        let every_file = cx.fs.records() && record::is_recording();
        for (_, dc) in &walk.levels {
            let Some(dc) = dc else {
                continue;
            };
            if dc.errors.is_empty()
                || (!every_file && self.reported_configs.insert(dc.path.clone(), ()).is_some())
            {
                continue;
            }
            out.push(
                ExtraEdge::new(
                    RefKind::Manifest,
                    Target::Unresolved,
                    dc.path.to_string_lossy().into_owned(),
                )
                .with_uncertainty(Uncertainty::new(
                    // A config that names another one that is not there (`extends`, a
                    // project reference) is a literal reference that does not resolve:
                    // the file's recorded chain has the config as a read and a probe for
                    // every place it looked, so only that file appearing or an edit of
                    // the config changes how the file's imports resolve. Any other
                    // failure (a config that does not parse, a cycle) says nothing about
                    // what would fix it.
                    if dc.only_missing && tsconfig_probed() {
                        UncertaintyKind::UnresolvedSpecifier
                    } else {
                        UncertaintyKind::ParseIncomplete
                    },
                    turborepo_trace_core::Span::DUMMY,
                    // The resolver's message names where it looked: for a config that
                    // extends one above the repository that is a directory of this
                    // machine.
                    turborepo_trace_core::fs::from_base(
                        &cx.options.base,
                        &format!("tsconfig {}: {}", dc.path.display(), dc.errors.join("; ")),
                    ),
                )),
            );
        }
    }

    /// Whether a package path is third party for `--externals stop|list`: its
    /// real path is inside a `node_modules` directory or outside the
    /// workspace. A workspace package linked into `node_modules` is first
    /// party.
    fn is_third_party(&self, path: &Path, cx: &ResolveCx<'_>) -> bool {
        if !in_node_modules(cx.fs, path) {
            return false;
        }
        record::pause(|| {
            let real = cx.fs.canonicalize(path).path;
            if in_node_modules(cx.fs, &real) {
                return true;
            }
            // The directory holding the `node_modules` the link was found in (the first
            // one of the path that is no part of where the checkout lives).
            let bytes = path.as_os_str().as_bytes();
            let from = cx.fs.local_offset(path);
            let Some(i) = memchr::memmem::find(&bytes[from..], b"/node_modules/") else {
                return true;
            };
            let holder = Path::new(std::ffi::OsStr::from_bytes(&bytes[..(from + i).max(1)]));
            !real.starts_with(self.workspace_root(holder, cx))
        })
    }

    /// The workspace root for packages installed under `dir`: the nearest
    /// ancestor (or `dir`) with `pnpm-workspace.yaml` or a `package.json`
    /// declaring `workspaces`, else the trace base.
    ///
    /// The result is memoized per directory, so the lookups are never recorded:
    /// they would land on whichever file asked first, which depends on
    /// thread scheduling. A caller whose result depends on the root records
    /// it for itself ([`JsResolver::record_workspace_root`]).
    fn workspace_root(&self, dir: &Path, cx: &ResolveCx<'_>) -> PathBuf {
        if let Some(r) = self.workspace_roots.get(dir) {
            return r.clone();
        }
        let fs = cx.fs;
        let root = record::pause(|| {
            dir.ancestors()
                .find(|d| {
                    fs.is_file(&d.join("pnpm-workspace.yaml"))
                        || fs.read_manifest(&d.join("package.json")).is_ok_and(|b| {
                            serde_json::from_slice::<serde_json::Value>(&b)
                                .is_ok_and(|v| v.get("workspaces").is_some())
                        })
                })
                .map_or_else(|| cx.options.base.clone(), Path::to_path_buf)
        });
        self.workspace_roots.insert(dir.to_path_buf(), root.clone());
        root
    }

    /// Records what makes `root` the workspace root of `dir`
    /// ([`JsResolver::workspace_root`]) for the reference being resolved: a
    /// `pnpm-workspace.yaml` in a directory between them would move the
    /// root, so each is a probe. The same probes for every file of a
    /// directory, whoever computed the root first. (A `workspaces` field added
    /// to a `package.json` in between is not seen: recording those files'
    /// contents would make every such file depend on every manifest above
    /// it.)
    fn record_workspace_root(dir: &Path, root: &Path) {
        for d in dir.ancestors().take_while(|d| *d != root) {
            record::missing(&d.join("pnpm-workspace.yaml"));
        }
    }

    fn resolve_spec(&self, r: &RawRef<'_>, spec: &Specifier<'_>, cx: &ResolveCx<'_>) -> Resolution {
        if r.base == Base::Language(RESOLVED_BASE) {
            return self.resolve_next_to_module(r, spec, cx);
        }
        if r.base == Base::Language(MODULE_STRING_BASE) {
            return self.resolve_module_string(spec, cx);
        }
        if let Some(mut res) = self.host_module(r, spec, cx) {
            // As for every module reference: the referencing file's own package scope.
            if let Some(m) = record::pause(|| self.nearest_package_json(cx.fs, cx.from_dir)) {
                res.manifests.insert(0, cx.intern(&m));
            }
            return res;
        }
        match spec {
            Specifier::Literal(s) => match module_system(r) {
                Some(system) => self.resolve_module(r, s, system, cx),
                None => JS_ASSET_RULES.resolve_path(r, s, cx),
            },
            Specifier::Pattern(parts) if cx.fs.records() && record::is_recording() => {
                let module = module_system(r).is_some();
                let frame = record::start();
                let res = match module {
                    true => self.resolve_module_pattern(r, parts, cx),
                    false => JS_ASSET_RULES.resolve_pattern(r, parts, cx),
                };
                let mut obs = frame.finish();
                refine_glob_memberships(&res, &mut obs);
                record::extend(obs);
                res
            }
            Specifier::Pattern(parts) => match module_system(r) {
                Some(_) => self.resolve_module_pattern(r, parts, cx),
                None => JS_ASSET_RULES.resolve_pattern(r, parts, cx),
            },
            Specifier::Alternatives(alts) => {
                resolve_value_alternatives(r, spec, alts, |a| self.resolve_spec(r, a, cx))
            }
            Specifier::Unknown => computed_specifier(r, spec),
        }
    }

    /// A module string (the extractor's [`MODULE_STRING_BASE`] anchor): a
    /// string literal in a registry position that may name a module some
    /// loader imports later (a route table's handler paths). It becomes an
    /// edge only when it names an existing first-party source file, as a
    /// path relative to the workspace root, to the file's package root or, for
    /// `./` and `../`, to the file's directory, with the usual extension and
    /// `index` probing. Anything else yields nothing: no edge and no
    /// uncertainty.
    ///
    /// Only in graph mode (change-impact queries): deployment traces keep nft's
    /// file set. Probes are recorded when the directory the string points
    /// into exists (a handler that a table names before its file is added),
    /// and not otherwise, so the many path-like strings that name nothing
    /// (`'application/json'`) leave no trace.
    fn resolve_module_string(&self, spec: &Specifier<'_>, cx: &ResolveCx<'_>) -> Resolution {
        let Specifier::Literal(s) = spec else {
            return Resolution::none();
        };
        if !cx.fs.records() || !record::is_recording() || in_node_modules(cx.fs, cx.from_path) {
            return Resolution::none();
        }
        let relative = s.starts_with("./") || s.starts_with("../");
        let mut dirs: Vec<PathBuf> = Vec::new();
        if relative {
            dirs.push(cx.from_dir.to_path_buf());
        } else {
            dirs.push(self.workspace_root(cx.from_dir, cx));
            if let Some(d) = self
                .nearest_package_json(cx.fs, cx.from_dir)
                .as_deref()
                .and_then(Path::parent)
                && !dirs.iter().any(|x| x == d)
            {
                dirs.push(d.to_path_buf());
            }
        }
        let request = if relative {
            s.to_string()
        } else {
            format!("./{s}")
        };
        let oxc = self.oxc(System::Cjs, cx.from_path);
        for (i, dir) in dirs.iter().enumerate() {
            let target = normalize(&dir.join(s.as_ref()));
            let parent = record::pause(|| target.parent().is_some_and(|p| cx.fs.is_dir(p)));
            if !parent {
                continue;
            }
            if !relative && i == 0 {
                Self::record_workspace_root(cx.from_dir, dir);
            }
            if let Ok(found) = oxc_resolve(oxc, At::Dir(dir), &request, cx) {
                let path = normalize(found.path());
                if is_module_source(&path) && !in_node_modules(cx.fs, &path) && path != cx.from_path
                {
                    return Resolution::file(cx.intern(&path));
                }
                // The string names an existing file that is no module (`input:
                // './openapi.yaml'` in a tool's config): no edge. The result still depends
                // on the file being there: once it is gone, the lookup records probes for
                // the path and its module extensions. That dependency is recorded as the
                // file's membership of its directory, so deleting the file re-traces this
                // one (a refreshed graph equals a fresh build) and editing it does not.
                if path != cx.from_path
                    && let (Some(parent), Some(name)) = (path.parent(), path.file_name())
                {
                    record::listing(parent, &literal_glob(&name.to_string_lossy()));
                }
            }
        }
        Resolution::none()
    }

    /// A module reference with a partially known specifier: a glob over files,
    /// like webpack context modules and nft's wildcard requires.
    fn resolve_module_pattern(
        &self,
        r: &RawRef<'_>,
        parts: &[Part<'_>],
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let mut parts: Vec<PatPart> = parts.iter().map(PatPart::from).collect();
        let Some(base) = cx.base_dir(&r.base) else {
            return unsupported_base(r);
        };
        // Anchors: relative or absolute paths at the base; a bare prefix where the
        // governing tsconfig's `paths` / `baseUrl` put it, and inside the
        // package directory. Each is the pattern's first literal (`None`: as
        // written) and the directory it starts in.
        let mut anchors: Vec<(Option<String>, PathBuf)> = vec![(None, base.into_owned())];
        if let Some(PatPart::Lit(first)) = parts.first() {
            match spec_kind(first) {
                SpecKind::Relative | SpecKind::Absolute => {}
                // `'node:' + name` names builtins only, a `data:` URL carries its source and
                // an `http(s):` URL is not a file: nothing to expand, nothing open.
                _ if ["node:", "data:", "http:", "https:"]
                    .iter()
                    .any(|s| first.starts_with(s)) =>
                {
                    return Resolution::unresolved(Uncertainty::new(
                        UncertaintyKind::PartialSpecifier,
                        r.span,
                        format!("pattern {} names no file", r.spec),
                    ));
                }
                _ if first.starts_with('#') || first.contains(':') => {
                    return Resolution::unresolved(not_expanded(
                        r,
                        format!(
                            "pattern {} with an `imports` or URL prefix is not expanded",
                            r.spec
                        ),
                    ));
                }
                _ => {
                    let name = package_name(first).to_string();
                    let complete = first.len() > name.len() && first.as_bytes()[name.len()] == b'/';
                    if !complete {
                        // The package name itself is partial (`'@img/sharp-' + platform`). nft
                        // does not expand non-relative wildcard requires; neither do we, since
                        // the glob would span all of node_modules.
                        let what = format!(
                            "pattern {}: package name not known statically; not expanded",
                            r.spec
                        );
                        return Resolution::unresolved(not_expanded(r, what));
                    }
                    anchors.clear();
                    // The prefix as an alias of the governing tsconfig (`@/guides/` under
                    // `"@/*": ["./src/*"]`): the pattern continues below each mapped
                    // directory. The module resolver tries those first and the package
                    // after them, so both are expanded.
                    match self.governing(cx).map(|ts| paths_prefix(&ts, first)) {
                        Some(PathsPrefix::Ambiguous(key)) => {
                            return Resolution::unresolved(not_expanded(
                                r,
                                format!(
                                    "pattern {}: the tsconfig `paths` key {key} may or may not \
                                     apply; not expanded",
                                    r.spec
                                ),
                            ));
                        }
                        Some(PathsPrefix::Dirs(dirs)) => {
                            for d in dirs {
                                let p = Path::new(&d);
                                let dir = if d.ends_with('/') {
                                    p.to_path_buf()
                                } else {
                                    p.parent().unwrap_or(p).to_path_buf()
                                };
                                anchors.push((Some(d), dir));
                            }
                        }
                        Some(PathsPrefix::None) | None => {}
                    }
                    if let Some(dir) = assets::find_package_dir(cx, cx.from_dir, &name) {
                        let first = format!("{}{}", dir.display(), &first[name.len()..]);
                        anchors.push((Some(first), dir));
                    }
                    if anchors.is_empty() {
                        return Resolution::unresolved(Uncertainty::new(
                            UncertaintyKind::UnresolvedSpecifier,
                            r.span,
                            format!(
                                "pattern {}: package prefix not found from {}",
                                r.spec,
                                cx.from_path.display()
                            ),
                        ));
                    }
                }
            }
        }
        // Without a trailing wildcard, also try each extension (nft appends
        // `?(.js|.json|.node)`).
        let exts = if is_ts_file(cx.from_path) {
            TS_EXTENSIONS
        } else {
            JS_EXTENSIONS
        };
        let keep: fn(&Path) -> bool = if self.include_types {
            assets::keep_asset
        } else {
            |p| assets::keep_asset(p) && !is_dts(p)
        };
        let mut out: Option<Resolution> = None;
        for (first, base) in anchors {
            if let Some(first) = first {
                parts[0] = PatPart::Lit(first);
            }
            let Some(res) = JS_ASSET_RULES.resolve_module_pattern(r, &parts, &base, exts, keep, cx)
            else {
                return refused_module_pattern(r);
            };
            out = Some(match out {
                None => res,
                Some(acc) => merge_expansions(acc, res),
            });
        }
        let Some(mut res) = out else {
            return refused_module_pattern(r);
        };
        if let Some(m) = record::pause(|| self.nearest_package_json(cx.fs, cx.from_dir)) {
            res.manifests.push(cx.intern(&m));
        }
        res
    }

    fn resolve_module(
        &self,
        r: &RawRef<'_>,
        spec: &str,
        system: System,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let fs = cx.fs;
        let oxc = self.oxc(system, cx.from_path);
        // `new Worker('w.js')` / `fork('child.js')` name paths, not packages.
        let dotted;
        let spec = if matches!(r.kind, RefKind::Worker | RefKind::ChildProcess)
            && spec_kind(spec) == SpecKind::Bare
            && cx
                .base_dir(&r.base)
                .is_some_and(|b| fs.exists(&normalize(&b.join(spec))))
        {
            dotted = format!("./{spec}");
            dotted.as_str()
        } else {
            spec
        };
        if is_package_spec(spec) {
            record::package(package_name(spec));
        }
        let result = match &r.base {
            // Per-file resolution: applies the governing tsconfig's `paths`/`baseUrl`.
            Base::FileDir => {
                // An absolute path is not a module name: `paths` and `baseUrl` do not apply
                // to it (oxc would try `<baseUrl>/<absolute path>` and its extensions, probes
                // that hold the checkout's location).
                let governing = match spec_kind(spec) {
                    SpecKind::Absolute => None,
                    _ => self.governing(cx),
                };
                let tsconfig = governing.as_deref();
                // Where the checkout lies decides this resolution when the config gives
                // aliases ([`record::aliased`]): the fact is on the reference's own record,
                // so whatever takes the record (a runner's request resolved for a virtual
                // file, a link resolved into a package) takes the fact with it.
                if tsconfig.is_some_and(gives_aliases) {
                    record::aliased();
                }
                oxc_resolve(oxc, At::File(cx.from_dir, tsconfig), spec, cx)
            }
            other => match cx.base_dir(other) {
                Some(dir) => oxc_resolve(oxc, At::Dir(&dir), spec, cx),
                None => return unsupported_base(r),
            },
        };
        let mut res = match result {
            Ok(found) => {
                let path = normalize(found.path());
                match self.target_for(r, spec, &path, cx) {
                    Ok(res) => res,
                    Err(u) => Resolution::unresolved(u),
                }
            }
            Err(ResolveError::Builtin { resolved, .. }) => {
                Resolution::target(Target::Builtin(resolved.into()))
            }
            // A package that is installed (or linked) and does not answer (an `exports` map
            // without the subpath, no entry file, no manifest) decides by its manifest: a
            // config input of the reference although nothing resolved, or a probe where
            // there is none. The walk records it (`record_node_modules_walk`).
            Err(e) => {
                let mut res = self.unresolved(r, spec, &e, cx);
                // Below a `node_modules` directory the config's `paths`, `baseUrl` and
                // `rootDirs` were not applied: what the resolution probed does not bound
                // the import (the file an alias names was not looked at).
                if matches!(r.base, Base::FileDir)
                    && spec_kind(spec) != SpecKind::Absolute
                    && let Some(ts) = self.aliases_not_applied(cx)
                    && let Some(u) = &mut res.uncertainty
                    && u.kind == UncertaintyKind::UnresolvedSpecifier
                {
                    u.kind = UncertaintyKind::ConditionalResolution;
                    u.detail =
                        format!("{}; {}", u.detail, aliases_not_applied_note(&ts, cx)).into();
                }
                res
            }
        };
        // Legacy `main`/file resolution next to `exports` (nft without `exportsOnly`).
        if let Some(legacy) = &self.legacy
            && is_package_spec(spec)
            && res.targets.iter().any(|t| matches!(t, Target::File(_)))
            && let Some(pkg_json) = package_root_json(fs, spec, &res, cx)
            && fs
                .read_manifest(&pkg_json)
                .is_ok_and(|b| memchr::memmem::find(&b, b"\"exports\"").is_some())
            && let Ok(found) = oxc_resolve(legacy, At::Dir(cx.from_dir), spec, cx)
        {
            let path = normalize(found.path());
            let id = cx.intern(&path);
            if !res.targets.contains(&Target::File(id)) && !(is_dts(&path) && !self.include_types) {
                res.targets.push(Target::File(id));
                if let Some(m) = path.parent().and_then(|d| self.nearest_package_json(fs, d)) {
                    res.manifests.push(cx.intern(&m));
                }
            }
            // `pkg/sub` resolved through the directory `pkg/sub/` and its own
            // `package.json` `main` (`@swc/helpers/_/
            // _interop_require_default`): Node reads that manifest
            // too, even when the file is the one `exports` names.
            let sub = spec[package_name(spec).len()..].trim_start_matches('/');
            if !sub.is_empty()
                && let Some(root) = pkg_json.parent()
            {
                let sub_path = normalize(&root.join(sub));
                // Not found as a file probe of the subpath itself (`pkg/sub.js`).
                let via_dir = path.starts_with(&sub_path)
                    || !path
                        .as_os_str()
                        .as_bytes()
                        .starts_with(sub_path.as_os_str().as_bytes());
                let dir_pj = sub_path.join("package.json");
                if via_dir && fs.is_file(&dir_pj) {
                    res.manifests.push(cx.intern(&dir_pj));
                }
            }
        }
        if self.all_conditions && is_package_spec(spec) {
            self.other_conditions(spec, system, cx, &mut res);
        }
        if (is_package_spec(spec) || spec.starts_with('#'))
            && res.targets.iter().any(|t| matches!(t, Target::File(_)))
            && self.mentions_module_sync(spec, &res, cx)
        {
            self.module_sync_variant(spec, system, cx, &mut res);
        }
        if self.browser {
            self.browser_remap(cx, &mut res);
        }
        // The referencing file's own package scope (`type`, `imports`, self-reference).
        // Its lookup chain is recorded once per file, by `file_edges`.
        if let Some(m) = record::pause(|| self.nearest_package_json(fs, cx.from_dir)) {
            res.manifests.insert(0, cx.intern(&m));
        }
        dedup(&mut res.manifests);
        res
    }

    /// `--all-conditions`: resolves `spec` under every condition its package's
    /// `exports` defines (alone and in pairs, so nested condition objects
    /// are reached) and adds each new result.
    fn other_conditions(
        &self,
        spec: &str,
        system: System,
        cx: &ResolveCx<'_>,
        res: &mut Resolution,
    ) {
        let Some(dir) = assets::find_package_dir(cx, cx.from_dir, package_name(spec)) else {
            return;
        };
        let Ok(bytes) = cx.fs.read_manifest(&dir.join("package.json")) else {
            return;
        };
        let Ok(pj) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return;
        };
        let mut conds = Vec::new();
        if let Some(e) = pj.get("exports") {
            collect_conditions(e, &mut conds);
        }
        conds.sort();
        conds.dedup();
        if conds.is_empty() {
            return;
        }
        let mut sets: Vec<Vec<String>> = conds.iter().map(|c| vec![c.clone()]).collect();
        if conds.len() <= 12 {
            for (i, a) in conds.iter().enumerate() {
                for b in &conds[i + 1..] {
                    sets.push(vec![a.clone(), b.clone()]);
                }
            }
        }
        let ts = is_ts_file(cx.from_path);
        let cjs = system == System::Cjs;
        for set in sets {
            let key = (set.clone(), cjs, ts);
            if !self.cond_variants.contains_key(&key) {
                let mut o = self.oxc(system, cx.from_path).options().clone();
                o.condition_names = set;
                let v = self.oxc[0][0].clone_with_options(o);
                self.cond_variants.entry(key.clone()).or_insert(v);
            }
            let Some(oxc) = self.cond_variants.get(&key) else {
                continue;
            };
            let Ok(found) = oxc_resolve(&oxc, At::Dir(cx.from_dir), spec, cx) else {
                continue;
            };
            let path = normalize(found.path());
            if is_dts(&path) && !self.include_types {
                continue;
            }
            let t = Target::File(cx.intern(&path));
            if !res.targets.contains(&t) {
                res.targets.retain(|t| !matches!(t, Target::Unresolved));
                res.targets.push(t);
                if let Some(m) = path
                    .parent()
                    .and_then(|d| self.nearest_package_json(cx.fs, d))
                {
                    res.manifests.push(cx.intern(&m));
                }
            }
        }
    }

    /// Whether the manifest that maps `spec` (the package root's for a bare
    /// specifier, the referencing file's scope for `#imports` and
    /// self-references) has a `module-sync` condition.
    fn mentions_module_sync(&self, spec: &str, res: &Resolution, cx: &ResolveCx<'_>) -> bool {
        record::pause(|| {
            let mut cands: Vec<PathBuf> = Vec::new();
            if is_package_spec(spec)
                && let Some(p) = package_root_json(cx.fs, spec, res, cx)
            {
                cands.push(p);
            }
            if let Some(p) = self.nearest_package_json_memo(cx.fs, cx.from_dir) {
                cands.push(p.to_path_buf());
            }
            cands.iter().any(|p| {
                cx.fs
                    .read_manifest(p)
                    .is_ok_and(|b| memchr::memmem::find(&b, b"\"module-sync\"").is_some())
            })
        })
    }

    /// Node >= 22 matches the `module-sync` condition for both `import` and
    /// `require`; older versions do not. Like nft, resolve once more with
    /// the condition toggled and keep both results.
    fn module_sync_variant(
        &self,
        spec: &str,
        system: System,
        cx: &ResolveCx<'_>,
        res: &mut Resolution,
    ) {
        let mut o = self.oxc(system, cx.from_path).options().clone();
        if o.condition_names.iter().any(|c| c == "module-sync") {
            o.condition_names.retain(|c| c != "module-sync");
        } else {
            o.condition_names.push("module-sync".into());
        }
        let key = (
            o.condition_names.clone(),
            system == System::Cjs,
            is_ts_file(cx.from_path),
        );
        if !self.cond_variants.contains_key(&key) {
            let v = self.oxc[0][0].clone_with_options(o);
            self.cond_variants.entry(key.clone()).or_insert(v);
        }
        let Some(oxc) = self.cond_variants.get(&key) else {
            return;
        };
        let Ok(found) = oxc_resolve(&oxc, At::Dir(cx.from_dir), spec, cx) else {
            return;
        };
        self.add_file_target(&normalize(found.path()), cx, res);
    }

    /// Adds `path` (and its package scope) to `res` unless present or a
    /// declaration file.
    fn add_file_target(&self, path: &Path, cx: &ResolveCx<'_>, res: &mut Resolution) {
        if is_dts(path) && !self.include_types {
            return;
        }
        let t = Target::File(cx.intern(path));
        if res.targets.contains(&t) {
            return;
        }
        res.targets.retain(|t| !matches!(t, Target::Unresolved));
        res.targets.push(t);
        if let Some(m) = path
            .parent()
            .and_then(|d| self.nearest_package_json(cx.fs, d))
        {
            res.manifests.push(cx.intern(&m));
        }
    }

    /// With the `browser` condition, a package's `browser` field object remaps
    /// its files (`"./lib/node.js": "./lib/browser.js"`). nft follows the
    /// remapping and keeps the original file too; so do we.
    fn browser_remap(&self, cx: &ResolveCx<'_>, res: &mut Resolution) {
        let files: Vec<PathBuf> = res
            .targets
            .iter()
            .filter_map(|t| match t {
                Target::File(id) => cx.interner.try_path(*id).map(|p| p.to_path_buf()),
                _ => None,
            })
            .collect();
        for file in files {
            let mut dir = file.parent();
            for _ in 0..8 {
                let Some(d) = dir else { break };
                if d.file_name() == Some(std::ffi::OsStr::new("node_modules")) {
                    break;
                }
                let pj = d.join("package.json");
                let browser = record::pause(|| {
                    let bytes = cx.fs.read_manifest(&pj).ok()?;
                    memchr::memmem::find(&bytes, b"\"browser\"")?;
                    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
                    v.get("browser").filter(|b| b.is_object()).cloned()
                });
                if let Some(map) = browser {
                    let Ok(rel) = file.strip_prefix(d) else { break };
                    let rel = rel.to_string_lossy();
                    let keys = [format!("./{rel}"), rel.to_string()];
                    let mapped = keys
                        .iter()
                        .find_map(|k| map.get(k.as_str()).and_then(|v| v.as_str()));
                    if let Some(to) = mapped {
                        let target = normalize(&d.join(to));
                        if cx.fs.is_file(&target) {
                            self.add_file_target(&target, cx, res);
                        }
                    }
                    break;
                }
                if d.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n == "node_modules")
                {
                    break;
                }
                dir = d.parent();
            }
        }
    }

    /// The resolution for a successfully resolved module path.
    fn target_for(
        &self,
        r: &RawRef<'_>,
        spec: &str,
        path: &Path,
        cx: &ResolveCx<'_>,
    ) -> Result<Resolution, Uncertainty> {
        if is_dts(path) && !self.include_types {
            return Err(Uncertainty::new(
                UncertaintyKind::UnresolvedSpecifier,
                r.span,
                format!(
                    "'{spec}' resolves only to the declaration file {} (excluded; use \
                     --include-types)",
                    path.display()
                ),
            ));
        }
        if self.externals != Externals::Follow
            && is_package_spec(spec)
            && self.is_third_party(path, cx)
        {
            return Ok(Resolution::target(Target::External(
                package_name(spec).into(),
            )));
        }
        let id = cx.intern(path);
        let mut res = Resolution::file(id);
        // Package root manifest (`exports`, `main`), then the target's own scope.
        if is_package_spec(spec)
            && let Some(m) = package_root_json(cx.fs, spec, &res, cx)
        {
            record::view_read(&m);
            res.manifests.push(cx.intern(&m));
        }
        if let Some(m) = path
            .parent()
            .and_then(|d| self.nearest_package_json(cx.fs, d))
        {
            res.manifests.push(cx.intern(&m));
        }
        Ok(res)
    }

    fn unresolved(
        &self,
        r: &RawRef<'_>,
        spec: &str,
        e: &ResolveError,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        // A relative path that exists only with different casing: works on macOS,
        // breaks on Linux. Report it with its target.
        if matches!(spec_kind(spec), SpecKind::Relative | SpecKind::Absolute)
            && let Some(base) = cx.base_dir(&r.base)
        {
            let probe = self.case_probe.resolve_to(spec, &base, cx, r.span);
            if probe
                .uncertainty
                .as_ref()
                .is_some_and(|u| u.kind == UncertaintyKind::CaseMismatch)
                && probe.targets.iter().any(|t| matches!(t, Target::File(_)))
            {
                return probe;
            }
            // `./types` with only `types.d.ts` on disk: nothing exists at runtime.
            let dts = normalize(&base.join(format!("{spec}.d.ts")));
            if !self.include_types && cx.fs.is_file(&dts) {
                return Resolution::unresolved(Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    r.span,
                    format!(
                        "'{spec}' resolves only to the declaration file {} (excluded; use \
                         --include-types)",
                        dts.display()
                    ),
                ));
            }
        }
        Resolution::unresolved(Uncertainty::new(
            UncertaintyKind::UnresolvedSpecifier,
            r.span,
            format!(
                "cannot resolve '{spec}' from {}: {e}",
                cx.from_path.display()
            ),
        ))
    }
}

impl JsResolver {
    /// A module load whose path lies outside the repository
    /// (`require('/usr/lib/addon.node')`, `import('../../other-checkout/x.js')`
    /// from a root file, a worker script in `/tmp`): the record of a host
    /// path, without asking the file system about it (in a confined trace
    /// only, see [`ResolveCx::is_host_path`]), and edges to what the path
    /// names when the repository is seen from another place
    /// ([`turborepo_trace_core::resolve_path::assets::AssetRules::host_reference_at`]): each reading is
    /// resolved like a relative specifier from the repository root. Only for
    /// specifiers written as paths: a package name, an alias or an
    /// `imports` key is resolved as usual, wherever its package lives.
    fn host_module(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        cx: &ResolveCx<'_>,
    ) -> Option<Resolution> {
        if !cx.fs.confined() {
            return None;
        }
        module_system(r)?;
        let first = match spec {
            Specifier::Literal(s) => &**s,
            Specifier::Pattern(parts) => match parts.first()? {
                Part::Lit(l) => &**l,
                _ => return None,
            },
            _ => return None,
        };
        if !matches!(spec_kind(first), SpecKind::Relative | SpecKind::Absolute) {
            return None;
        }
        let base = cx.base_dir(&r.base)?;
        let located = turborepo_trace_core::resolve_path::assets::located(r);
        JS_ASSET_RULES.host_reference_at(r, spec, &base, located, cx, |at_root| {
            self.resolve_spec(at_root, &at_root.spec, cx)
        })
    }
}

impl Resolver for JsResolver {
    fn resolve(&self, r: &RawRef<'_>, _from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let mut res = self.resolve_spec(r, &r.spec, cx);
        // An optional load (inside a `try` whose `catch` swallows the error): a
        // failure is tolerated by the code, so it is no uncertainty.
        // The probes recorded while resolving stay (adding the module later
        // changes the result), and so does an explicit unresolved edge, which
        // the store can still resolve from the lockfile.
        if r.flags.contains(RefFlags::OPTIONAL)
            && matches!(
                r.kind,
                RefKind::Require | RefKind::Import | RefKind::DynamicImport | RefKind::ReExport
            )
            && matches!(r.spec, Specifier::Literal(_))
            && !res.none
            && res.targets.iter().all(|t| matches!(t, Target::Unresolved))
            && res
                .uncertainty
                .as_ref()
                .is_some_and(|u| u.kind == UncertaintyKind::UnresolvedSpecifier)
        {
            res.uncertainty = None;
            if res.targets.is_empty() {
                res.targets.push(Target::Unresolved);
            }
        }
        // A path value emitted as an asset (the extractor's deep pass) that names a
        // directory: including every file below it is a guess.
        if r.kind == RefKind::Asset
            && res.uncertainty.is_none()
            && let Specifier::Literal(s) = &r.spec
            && res.targets.iter().any(|t| matches!(t, Target::Glob { .. }))
        {
            res.uncertainty = Some(Uncertainty::new(
                UncertaintyKind::PartialSpecifier,
                r.span,
                format!(
                    "directory {s} is referenced as a path value, not through a tracked fs call; \
                     every file below it is included"
                ),
            ));
        }
        // Loader packages (`bindings`, `node-gyp-build`, ...): find the addon they
        // load.
        if module_system(r).is_some() && !r.flags.contains(RefFlags::TYPE_ONLY) {
            let mut loaders = Vec::new();
            literal_package_specs(&r.spec, &mut loaders);
            loaders.sort_unstable();
            loaders.dedup();
            for l in loaders {
                res.extra.extend(self.special.loader_edges(l, cx));
            }
        }
        let targets = res.targets.to_vec();
        self.sharedlibs
            .add_for_targets(&targets, &mut res.extra, is_node_file, cx);
        res
    }

    fn file_edges(&self, _file: FileId, cx: &ResolveCx<'_>) -> Vec<ExtraEdge> {
        let mut out = Vec::new();
        let recording = cx.fs.records() && record::is_recording();
        if let Some(t) = &self.tsconfigs
            && is_js_or_ts_file(cx.from_path)
        {
            // Discovery for the file, once: what it consulted is recorded, and reported
            // where it failed to load.
            let walk = record::pause(|| {
                let pkg = self.nearest_package_json_memo(cx.fs, cx.from_dir);
                t.walk(cx.fs, cx.from_path, pkg.as_deref().and_then(Path::parent))
            });
            if recording {
                self.record_tsconfig_chain(t, &walk, cx);
                // Whether the place of the checkout decides the file's records: a fact of
                // the file's record, at every place ([`record::aliased`]).
                if walk.config.as_deref().is_some_and(gives_aliases) {
                    record::aliased();
                }
            }
            self.config_errors(&walk, cx, &mut out);
            // Every import of the file may resolve elsewhere than the resolver says (an
            // alias that names a file of the repository where an installed package
            // answered, or nothing): one open record for the file.
            if let Some(ts) = self.aliases_not_applied(cx) {
                out.push(
                    ExtraEdge::new(
                        RefKind::Manifest,
                        Target::Unresolved,
                        ts.path().to_string_lossy().into_owned(),
                    )
                    .with_uncertainty(Uncertainty::new(
                        UncertaintyKind::ConditionalResolution,
                        turborepo_trace_core::Span::DUMMY,
                        format!(
                            "what the imports of this file resolve to is not known: {}",
                            aliases_not_applied_note(&ts, cx)
                        ),
                    )),
                );
            }
        }
        // (The same fact for a file of the language without a JS or TS extension, which
        // a config governs as the nearest one in its package: a script with a
        // `node` shebang. No graph build of the tests traces one as a file of a
        // package, so no case shows this line; it keeps the fact complete for
        // one that does.)
        if recording
            && self.tsconfigs.is_some()
            && !is_js_or_ts_file(cx.from_path)
            && self.aliasing(cx).is_some()
        {
            record::aliased();
        }
        // The file's own package scope, needed even without references (nft emits the
        // `package.json` boundary of every module).
        if let Some(m) = self.nearest_package_json(cx.fs, cx.from_dir) {
            out.push(ExtraEdge::new(
                RefKind::Manifest,
                Target::File(cx.intern(&m)),
                "package.json scope",
            ));
        }
        out.extend(self.special.file_edges(cx));
        self.sharedlibs
            .add_for_targets(&[], &mut out, is_node_file, cx);
        out
    }

    fn clear_cache(&self) {
        // All variants share one oxc cache.
        self.oxc[0][0].clear_cache();
        self.nearest_pkg.clear();
        self.sharedlibs.clear_cache();
        if let Some(t) = &self.tsconfigs {
            t.clear();
        }
        self.reported_configs.clear();
        self.workspace_roots.clear();
    }
}

/// Where an oxc resolution starts.
#[derive(Copy, Clone)]
enum At<'a> {
    /// From a file's directory, with the tsconfig governing the file.
    File(&'a Path, Option<&'a TsConfig>),
    /// From a directory, without tsconfig.
    Dir(&'a Path),
}

/// Runs oxc. While recording, uses `resolve_with_context` and records its
/// missing paths (see the module docs); otherwise the plain, equivalent API.
fn oxc_resolve(
    oxc: &Oxc,
    at: At<'_>,
    spec: &str,
    cx: &ResolveCx<'_>,
) -> Result<oxc_resolver::Resolution, ResolveError> {
    if !cx.fs.records() || !record::is_recording() {
        return match at {
            // Only `resolve_with_context` takes an explicit tsconfig.
            At::File(d, Some(t)) => {
                oxc.resolve_with_context(d, spec, Some(t), &mut ResolveContext::default())
            }
            // `resolve` uses only a manually configured tsconfig, which these resolvers
            // never have.
            At::File(d, None) | At::Dir(d) => oxc.resolve(d, spec),
        };
    }
    let mut ctx = ResolveContext::default();
    let result = record::pause(|| match at {
        At::File(d, tsconfig) => oxc.resolve_with_context(d, spec, tsconfig, &mut ctx),
        At::Dir(d) => oxc.resolve_with_context(d, spec, None, &mut ctx),
    });
    // `package.json` and `node_modules` lookups are memoized inside oxc and only
    // logged on the first call; both are recorded deterministically here
    // instead.
    let mut missing: Vec<PathBuf> = ctx
        .missing_dependencies
        .into_iter()
        .filter(|p| {
            !matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("package.json" | "node_modules")
            )
        })
        .map(|p| normalize(&p))
        .collect();
    missing.sort_unstable();
    let fs = cx.fs;
    let base = match at {
        At::File(d, _) | At::Dir(d) => d,
    };
    // The path a relative or absolute request names, as written.
    let asked = matches!(spec_kind(spec), SpecKind::Relative | SpecKind::Absolute)
        .then(|| normalize(&base.join(spec)));
    for m in &missing {
        // A directory probed as a file, or a file probed as a directory, exists: adding
        // files below it is covered by the probes made inside it.
        let Some(there) = record::pause(|| fs.stat(m)) else {
            record::missing(m);
            continue;
        };
        // Not so for a directory with the name of a file the lookup tried next to the
        // request (`x.ts/` for `./x`): it asked "is this a file?", got a no and went on
        // to the next name, and nothing was asked inside. That no is an answer
        // like any other, and a file that takes the directory's place is
        // another: the name is recorded as not found, and what is there is part
        // of the record's state. (The request as written is another matter: a
        // directory of that name is entered, when no file answers first.)
        let tried_as_a_file = there.kind == turborepo_trace_core::fs::FileKind::Dir
            && asked
                .as_ref()
                .is_some_and(|a| m != a && m.parent() == a.parent());
        // The same answer anywhere else the lookup tried a name with an extension: an
        // index file of a directory it entered (`x/index.ts/`), a file next to the
        // target of a mapping or of a package's entry (`src/x.ts/` for `@/x`). A
        // directory is one of those when the lookup asked nothing inside it (it was not
        // entered) and it is the name of a candidate: `index.<ext>`, or a path the
        // lookup also tried without that extension.
        let tried_as_a_file = tried_as_a_file
            || (there.kind == turborepo_trace_core::fs::FileKind::Dir
                && asked.as_ref() != Some(m)
                && candidate_name(m, &missing)
                && !missing.iter().any(|p| p != m && p.starts_with(m))
                && !result
                    .as_ref()
                    .is_ok_and(|found| found.path().starts_with(m)));
        if tried_as_a_file {
            record::missing(m);
        }
    }
    if is_package_spec(spec) {
        let walked = match &result {
            Ok(found) => Walked::Found(found.path()),
            // A builtin is no package of any level.
            Err(ResolveError::Builtin { .. }) => Walked::Elsewhere,
            Err(_) => Walked::Failed,
        };
        record_node_modules_walk(fs, base, package_name(spec), walked);
    }
    // A request that names a directory: its own `package.json` was read or looked
    // for (oxc reports this only on a cold cache). Resolved through its index
    // or `main`, and also when the directory did not answer (a manifest that
    // names no entry, no index file): what the manifest says decides that, and
    // one that comes to name an entry makes the request resolve. So the
    // manifest (its resolution view) is an input of whoever asked
    // for the directory, whatever the answer was.
    if matches!(spec_kind(spec), SpecKind::Relative | SpecKind::Absolute) {
        let target = normalize(&base.join(spec));
        let through_directory = match &result {
            Ok(found) => found.path() != target,
            Err(_) => unanswered_directory(),
        };
        if through_directory && record::pause(|| fs.is_dir(&target)) {
            let pj = target.join("package.json");
            if record::pause(|| fs.is_file(&pj)) {
                record::view_read(&pj);
            } else {
                record::missing(&pj);
            }
        }
    }
    result
}

/// Whether the path `m` reads as a file name that a lookup which also asked for
/// `tried` made up: an index file (`index.<ext>`), or one of `tried` with an
/// extension added.
fn candidate_name(m: &Path, tried: &[PathBuf]) -> bool {
    let Some(name) = m.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // (An extension may have a dot of its own: `.d.ts`.)
    name.match_indices('.')
        .filter(|(at, _)| *at > 0 && *at + 1 < name.len())
        .map(|(at, _)| &name[..at])
        .any(|stem| stem == "index" || tried.binary_search(&m.with_file_name(stem)).is_ok())
}

/// Whether the manifest of a directory that a relative request names and that
/// does not answer is recorded (`TURBO_TRACE_DEBUG_UNANSWERED_DIRECTORY=0`
/// switches it off, for a comparison of selections: without it a refresh can
/// keep records that a fresh build would not make).
fn unanswered_directory() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        turborepo_trace_core::debug::var_os("UNANSWERED_DIRECTORY").is_none_or(|v| v != "0")
    })
}

/// How the search for a package ended ([`record_node_modules_walk`]).
#[derive(Clone, Copy)]
enum Walked<'a> {
    /// At this file (spelled through the package directory it was found in).
    Found(&'a Path),
    /// Nowhere: every level was asked.
    Failed,
    /// With an answer that no package directory gave (a builtin).
    Elsewhere,
}

/// Records Node's `node_modules` walk-up for package `name` from `start`: at
/// each ancestor (not itself a `node_modules` directory), a missing
/// `node_modules` directory or a missing `node_modules/<name>` directory, up to
/// the level whose package directory answered.
///
/// A package directory that is there and does not answer (no manifest, or one
/// that names no entry, and no index file) does not end the walk, in Node and
/// in oxc alike: the next level is asked. What such a level says depends on its
/// manifest, so the manifest is recorded: its resolution view when there is
/// one, a probe when there is none (the files the lookup tried inside the
/// directory are probes already). Without that record the state of a file's
/// records would depend on the history of the tree: a manifest that was
/// there at the last trace would be an input, one that came later would not.
///
/// What was found in none of the package directories on the way (a `paths`
/// mapping of a tsconfig answered before any level was asked) ends the record
/// at the first package directory, which no walk reached.
fn record_node_modules_walk(fs: &DirCache, start: &Path, name: &str, walked: Walked<'_>) {
    let marker = std::ffi::OsStr::new("node_modules");
    // Whether a package directory above the current level holds what was found:
    // asked once, at the first package directory that does not hold it.
    let mut further: Option<bool> = None;
    let mut cur = Some(start);
    while let Some(d) = cur {
        cur = d.parent();
        if d.file_name() == Some(marker) {
            continue;
        }
        let nm = d.join("node_modules");
        if !record::pause(|| fs.is_dir(&nm)) {
            record::missing(&nm);
            continue;
        }
        let pkg = nm.join(name);
        if !record::pause(|| fs.is_dir(&pkg)) {
            record::missing(&pkg);
            continue;
        }
        match walked {
            Walked::Elsewhere => return,
            Walked::Found(found) => {
                if found.starts_with(&pkg) {
                    return;
                }
                // (The file is there, so a directory that holds it is one.)
                let above = *further.get_or_insert_with(|| {
                    cur.into_iter().flat_map(Path::ancestors).any(|d| {
                        found
                            .strip_prefix(d)
                            .ok()
                            .and_then(|rest| rest.strip_prefix(marker).ok())
                            .is_some_and(|rest| rest.starts_with(name))
                    })
                });
                if !above {
                    return;
                }
            }
            Walked::Failed => {}
        }
        // A package directory above the repository is the machine's: a graph does not
        // depend on what it holds (like a probe there, which the store drops).
        if fs.is_above_base(d) {
            continue;
        }
        // The walk went through this package directory.
        let manifest = pkg.join("package.json");
        if record::pause(|| fs.is_file(&manifest)) {
            record::view_read(&manifest);
        } else {
            record::missing(&manifest);
        }
    }
}

/// `TURBO_TRACE_DEBUG_TSCONFIG_PROBED=0`: a tsconfig that names a missing
/// config stays a `parse_incomplete` record on the files it governs, as before
/// (for A/B measurements; graphs built either way are not interchangeable).
fn tsconfig_probed() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var_os("TSCONFIG_PROBED").is_none_or(|v| v != "0")
    });
    *ON
}

/// What the files that consult the directory `d` in their config discovery
/// record of it, when the reference `governing` of its config governs them
/// (`None`: none does): their inputs, and their key observations (see
/// `JsResolver::record_tsconfig_chain`).
///
/// What a config's load depends on is what the load asked of the file system
/// ([`tsconfig::DirConfig::observed`], [`tsconfig`] module docs), not a reading
/// of the config's `extends` by a second set of rules.
fn level_observations(
    d: &Path,
    dc: Option<&tsconfig::DirConfig>,
    governing: Option<usize>,
    cx: &ResolveCx<'_>,
) -> (record::Observations, record::Observations) {
    let config = tsconfig::CONFIG_NAMES
        .iter()
        .map(|name| d.join(name))
        .find(|cand| record::pause(|| cx.fs.is_file(cand)));
    let refs = dc.map(|dc| dc.ref_paths.as_slice()).unwrap_or(&[]);
    let loaded = |r: &Path| dc.is_none_or(|dc| dc.loaded(r));
    // What the load of the level's config asked of the file system.
    let own = || {
        if let Some(dc) = dc {
            record::extend_from(&dc.observed);
        }
    };
    // The same for the load of the reference `k`: the config it names with its
    // `extends` chain, or the places it was looked for when it is missing.
    let reference = |k: usize| {
        if let Some(o) = dc.and_then(|dc| dc.ref_observed.get(k)) {
            record::extend_from(o);
        }
    };
    // Of a reference that was checked and does not govern the file: the places its
    // load looked at and found nothing (and the directories it found there
    // without reading anything in them), no contents (P13).
    let probes = |k: usize| {
        let Some(o) = dc.and_then(|dc| dc.ref_observed.get(k)) else {
            return;
        };
        for p in &o.missing {
            record::missing(p);
        }
        for (dir, name) in &o.listings {
            record::listing(dir, name);
        }
    };
    let ((), inputs) = record::capture(|| {
        for name in tsconfig::CONFIG_NAMES {
            let cand = d.join(name);
            if config.as_ref() != Some(&cand) {
                record::missing(&cand);
                continue;
            }
            match governing {
                // The listing config: its `references` decide; its own `extends` chain
                // does not apply to the file.
                Some(_) => record::read(&cand),
                None => own(),
            }
            for (k, r) in refs.iter().enumerate() {
                match governing {
                    Some(g) if k == g => reference(k),
                    // Not consulted after the governing one.
                    Some(g) if k > g => break,
                    // A broken reference is skipped (and reported on every file that
                    // consults it): fixing it can change the choice, so it stays a
                    // content input; such files are uncertain anyway.
                    _ if !loaded(r) => reference(k),
                    _ => probes(k),
                }
            }
            break;
        }
    });
    let ((), key) = record::capture(|| {
        if config.is_none() {
            return;
        }
        // Whether the listing config loads is reported on the file.
        if governing.is_some() {
            own();
        }
        for (k, r) in refs.iter().enumerate() {
            match governing {
                Some(g) if k == g => {}
                // Loaded with the others, and reported when it does not load.
                Some(g) if k > g => reference(k),
                _ if !loaded(r) => {}
                // It loads and does not include the file: no input, and both can change.
                _ => reference(k),
            }
        }
    });
    (
        tsconfig::recorded(cx.fs, inputs),
        tsconfig::recorded(cx.fs, key),
    )
}

/// The record of a module pattern that is not expanded: nothing stands for the
/// files it can name, so the record is open.
/// [`UncertaintyKind::PartialSpecifier`] means "a glob that was expanded; its
/// directory memberships cover the rest" and `affected` does not count it. The
/// extractor gives that kind to every pattern, so here it becomes
/// [`UncertaintyKind::PatternTooBroad`]; any other record of the extractor (the
/// working directory, the environment) keeps its kind.
fn not_expanded(r: &RawRef<'_>, what: String) -> Uncertainty {
    match &r.uncertainty {
        Some(u) if u.kind != UncertaintyKind::PartialSpecifier => {
            Uncertainty::new(u.kind, u.span, format!("{}; {what}", u.detail))
        }
        Some(u) if !u.detail.is_empty() => Uncertainty::new(
            UncertaintyKind::PatternTooBroad,
            u.span,
            format!("{}; {what}", u.detail),
        ),
        _ => Uncertainty::new(UncertaintyKind::PatternTooBroad, r.span, what),
    }
}

/// Where the literal start of a bare module pattern lands under a tsconfig.
enum PathsPrefix {
    /// Neither `paths` nor `baseUrl` maps it.
    None,
    /// The mapped starts, as absolute path text (not necessarily a whole
    /// segment).
    Dirs(Vec<String>),
    /// A `paths` key may apply or not, depending on the unknown rest.
    Ambiguous(String),
}

/// Maps the literal start of a bare module pattern (`@/guides/` of
/// `@/guides/*.mdx`) through the tsconfig's `paths` and `baseUrl`. The unknown
/// rest stands in for the key's `*`, so a key that only matches for some values
/// of it (`@/guides/special/*`, an exact `@/guides/index`, a key with text
/// after its `*`) makes the mapping ambiguous.
fn paths_prefix(ts: &TsConfig, first: &str) -> PathsPrefix {
    if let Some(paths) = &ts.compiler_options.paths {
        for key in paths.keys() {
            let stem = key.split('*').next().unwrap_or(key);
            if stem.len() > first.len() && stem.starts_with(first) {
                return PathsPrefix::Ambiguous(key.clone());
            }
        }
    }
    const HOLE: &str = "__turbotrace_hole__";
    let mut dirs = Vec::new();
    for p in ts.resolve_path_alias_or_base_url(&format!("{first}{HOLE}")) {
        match p.to_str().and_then(|s| s.strip_suffix(HOLE)) {
            Some(d) => dirs.push(d.to_owned()),
            None => return PathsPrefix::Ambiguous(format!("matching {first}*")),
        }
    }
    if dirs.is_empty() {
        PathsPrefix::None
    } else {
        PathsPrefix::Dirs(dirs)
    }
}

/// Joins the expansions of one pattern from two anchors: an anchor where
/// nothing matched adds no unresolved target next to one that matched, and the
/// record is that of an expansion that found something, if any did.
fn merge_expansions(mut a: Resolution, b: Resolution) -> Resolution {
    let found = |r: &Resolution| r.targets.iter().any(|t| !matches!(t, Target::Unresolved));
    if found(&b) && !found(&a) {
        a.targets.clear();
        a.uncertainty = b.uncertainty.clone();
    }
    let keep_unresolved = !found(&a) && !found(&b);
    for t in b.targets {
        if (keep_unresolved || !matches!(t, Target::Unresolved)) && !a.targets.contains(&t) {
            a.targets.push(t);
        }
    }
    a.manifests.extend(b.manifests);
    a.extra.extend(b.extra);
    a
}

/// Refines the listings a pattern resolution recorded (graph mode).
///
/// The glob expansion lists its static directory and every directory it enters,
/// which the dir cache records as `(dir, "**")`: sound, and present even when
/// nothing matched. When the resolution produced a [`Target::Glob`], its
/// pattern is known, so the listings at or below the pattern's static directory
/// are replaced by the pattern's memberships ([`membership_patterns`]).
///
/// Glob patterns are absolute with `*` for each wildcard; a wildcard may cross
/// `/` ([`turborepo_trace_core::eval`]). Module patterns tried with each
/// extension are displayed with a `?(ext)` suffix; each membership then also
/// gets a `.*` variant.
fn refine_glob_memberships(res: &Resolution, obs: &mut record::Observations) {
    let globs = res
        .targets
        .iter()
        .chain(res.extra.iter().map(|x| &x.target))
        .filter_map(|t| match t {
            Target::Glob { pattern, .. } => Some(&**pattern),
            _ => None,
        });
    for pattern in globs {
        let (pattern, ext) = match pattern.strip_suffix("?(ext)") {
            Some(p) => (p, true),
            None => (pattern, false),
        };
        let Some(star) = pattern.find('*') else {
            continue;
        };
        let Some(slash) = pattern[..star].rfind('/') else {
            continue;
        };
        let dir = Path::new(if slash == 0 { "/" } else { &pattern[..slash] });
        let rest = &pattern[slash + 1..];
        obs.listings
            .retain(|(d, p)| !(d.starts_with(dir) && p == "**"));
        for p in membership_patterns(rest) {
            // Extension variants append a literal `.ext`, which never contains `/`.
            if ext {
                obs.listings.push((dir.to_path_buf(), format!("{p}.*")));
            }
            obs.listings.push((dir.to_path_buf(), p));
        }
    }
}

/// Native addons, whose packages' shared libraries are emitted (nft: `.node`
/// only).
fn is_node_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "node")
}

/// Package names of the literal bare specifiers in `spec` (including
/// alternatives).
fn literal_package_specs<'a>(spec: &'a Specifier<'_>, out: &mut Vec<&'a str>) {
    match spec {
        Specifier::Literal(s) if is_package_spec(s) => out.push(package_name(s)),
        Specifier::Alternatives(alts) => {
            for a in alts {
                literal_package_specs(a, out);
            }
        }
        _ => {}
    }
}

/// Condition names used anywhere in an `exports` value (object keys not
/// starting with `.`).
fn collect_conditions(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            for (k, v) in m {
                if !k.starts_with('.') {
                    out.push(k.clone());
                }
                collect_conditions(v, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|v| collect_conditions(v, out)),
        _ => {}
    }
}

/// The module system a reference resolves under, or `None` for plain path
/// references.
fn module_system(r: &RawRef<'_>) -> Option<System> {
    match r.kind {
        RefKind::Import | RefKind::DynamicImport | RefKind::ReExport => Some(System::Esm),
        RefKind::Require => Some(System::Cjs),
        // `new Worker(path)` / `fork(path)` run the script through the CommonJS loader, so
        // they resolve like `require` paths from their base (file dir or cwd).
        RefKind::Worker | RefKind::ChildProcess => Some(System::Cjs),
        _ => None,
    }
}

/// A membership pattern that matches exactly the entry `name` of a directory,
/// whatever characters the name is made of ([`record::literal_pattern`]).
fn literal_glob(name: &str) -> String {
    record::literal_pattern(name)
}

/// Whether `path` is a JS or TS source file (not a declaration file, not data).
fn is_module_source(path: &Path) -> bool {
    !is_dts(path)
        && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e,
                "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx"
            )
        })
}

fn is_ts_file(path: &Path) -> bool {
    matches!(
        path.extension().map(|e| e.as_bytes()),
        Some(b"ts" | b"tsx" | b"mts" | b"cts")
    )
}

/// Whether `path` is a JS or TS module the resolver gets references from.
fn is_js_or_ts_file(path: &Path) -> bool {
    matches!(
        path.extension().map(|e| e.as_bytes()),
        Some(b"js" | b"jsx" | b"mjs" | b"cjs" | b"ts" | b"tsx" | b"mts" | b"cts")
    )
}

/// Whether `path` is a TypeScript declaration file.
fn is_dts(path: &Path) -> bool {
    let n = path.file_name().map(|n| n.as_bytes()).unwrap_or_default();
    [&b".d.ts"[..], b".d.mts", b".d.cts"]
        .iter()
        .any(|s| n.ends_with(s))
        || (n.ends_with(b".ts") && memchr::memmem::find(n, b".d.").is_some())
}

/// Whether a specifier names a package (bare, not relative, absolute, `#import`
/// or URL).
fn is_package_spec(spec: &str) -> bool {
    spec_kind(spec) == SpecKind::Bare
        && !spec.starts_with('#')
        && !spec.starts_with("node:")
        && !spec.contains(':')
}

/// The package name of a bare specifier: `a` for `a/b/c`, `@s/a` for `@s/a/b`.
pub fn package_name(spec: &str) -> &str {
    let mut it = spec.match_indices('/');
    let end = if spec.starts_with('@') {
        it.nth(1)
    } else {
        it.next()
    };
    match end {
        Some((i, _)) => &spec[..i],
        None => spec,
    }
}

/// Whether `ts` gives `paths`, `baseUrl` or `rootDirs`.
fn gives_aliases(ts: &TsConfig) -> bool {
    let o = &ts.compiler_options;
    o.paths.as_ref().is_some_and(|p| !p.is_empty())
        || o.base_url.is_some()
        || o.root_dirs.as_ref().is_some_and(|d| !d.is_empty())
}

/// Why the imports of a file that [`JsResolver::aliases_not_applied`] names are
/// not bounded by what their resolution probed (one sentence, for `--explain`).
fn aliases_not_applied_note(ts: &TsConfig, cx: &ResolveCx<'_>) -> String {
    turborepo_trace_core::fs::from_base(
        &cx.options.base,
        &format!(
            "the paths, baseUrl and rootDirs of {} are not applied to it, because the checkout \
             lies below a directory called node_modules and the resolver takes every file there \
             for a file of an installed package (keep the checkout outside node_modules)",
            ts.path().display()
        ),
    )
}

/// Whether `path` goes through a `node_modules` directory, in the part the
/// place of the checkout has no say in ([`DirCache::local`]): the files of a
/// checkout below `/ci/node_modules/app` are not installed packages.
fn in_node_modules(fs: &DirCache, path: &Path) -> bool {
    fs.local(path)
        .components()
        .any(|c| c.as_os_str() == "node_modules")
}

/// The `package.json` at the root of the package a bare `spec` resolved into:
/// the last `node_modules/<name>` directory on the resolved path (pnpm and npm
/// layouts alike, since paths are not canonicalized here).
fn package_root_json(
    fs: &DirCache,
    spec: &str,
    res: &Resolution,
    cx: &ResolveCx<'_>,
) -> Option<PathBuf> {
    let Some(Target::File(id)) = res.targets.first() else {
        return None;
    };
    let path = cx.interner.try_path(*id)?;
    let needle = format!("/node_modules/{}/", package_name(spec));
    let bytes = path.as_os_str().as_bytes();
    let i = memchr::memmem::rfind(bytes, needle.as_bytes())?;
    let root = Path::new(std::ffi::OsStr::from_bytes(&bytes[..i + needle.len()]));
    let pj = root.join("package.json");
    fs.is_file(&pj).then_some(pj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        assert_eq!(package_name("react"), "react");
        assert_eq!(package_name("react-dom/client"), "react-dom");
        assert_eq!(package_name("@scope/pkg"), "@scope/pkg");
        assert_eq!(package_name("@scope/pkg/sub/path"), "@scope/pkg");
        assert!(is_package_spec("react"));
        assert!(!is_package_spec("./a"));
        assert!(!is_package_spec("#internal"));
        assert!(!is_package_spec("node:fs"));
        assert!(!is_package_spec("/abs"));
    }

    #[test]
    fn glob_membership_patterns() {
        assert_eq!(membership_patterns("*.json"), ["*.json", "*/**/*.json"]);
        assert_eq!(membership_patterns("a*/x/*b"), ["a*/x/*b", "a*/**/*b"]);
        assert_eq!(membership_patterns("x{1}*"), ["x?1?*", "x?1?*/**/*"]);
        assert_eq!(membership_patterns("plain"), ["plain"]);
    }

    #[test]
    fn declaration_files() {
        assert!(is_dts(Path::new("/a/index.d.ts")));
        assert!(is_dts(Path::new("/a/index.d.mts")));
        assert!(!is_dts(Path::new("/a/index.ts")));
        assert!(!is_dts(Path::new("/a/d.ts")));
        assert!(is_ts_file(Path::new("/a/x.mts")));
        assert!(!is_ts_file(Path::new("/a/x.mjs")));
    }
}
