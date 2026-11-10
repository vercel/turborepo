//! The Go resolver: imports to package files, the file's own package scope,
//! embeds, cgo includes, manifests and runtime file references.
//!
//! | Reference | Targets |
//! |---|---|
//! | `import "p"` | one [`Target::Glob`] `<pkg dir>/*.go` whose matches are the package's non-test Go files that build for the target; an extra [`RefKind::Package`] glob for its cgo/assembly/object files; the providing module's `go.mod` and the context's `go.sum`/`go.work`/`vendor/modules.txt` as manifests. Standard library: [`Target::Builtin`]. Third-party with `--externals stop|list`: [`Target::External`] `module@version`. A package of a required module that is not in the module cache: [`Target::External`] `module@version` for every module that may provide it, the manifests, and an `unresolved_specifier` record (its files can't be traced); the graph store turns that into an edge to the module's opaque package ([`Resolver::external_identity`]) |
//! | the file itself ([`Resolver::file_edges`]) | its package mates as a [`RefKind::Package`] glob (for a `_test.go` file, also every `_test.go` file of the directory), the package's other files, and the manifests |
//! | `//go:embed p` | a [`Target::Glob`] with the embedded files ([`crate::embed`]) |
//! | cgo `#include "x.h"` | the header next to the file; `<x.h>` is a builtin |
//! | runtime file APIs | the shared asset rules ([`ASSET_RULES`]) |
//!
//! With [`BuildContext::all_targets`] (`--all-targets`,
//! [`turborepo_trace_core::TraceOptions::all_targets`]), files that only build
//! for other targets are extra edges flagged
//! [`turborepo_trace_core::RefFlags::CONDITIONAL`]; their spec names the
//! constraint. `go.mod`, `go.sum`, `go.work` and `vendor/modules.txt` are
//! [`turborepo_trace_core::Phase::Build`] manifests.
//!
//! Observations (graph mode): every lookup goes through the dir cache, so
//! missing `go.mod`, vendor and module cache paths are probes automatically.
//! Package directories are recorded as `(dir, "*")` memberships, embed patterns
//! as `(dir, pattern)` plus `(matched dir, "**")`, and the contents of files
//! excluded by a build constraint as reads. Memoized steps replay
//! their observations ([`turborepo_trace_core::Memo`]).
//!
//! The module cache and `GOROOT` are environment roots
//! ([`Resolver::environment_roots`]), and files below the module cache or a
//! `vendor/` directory have a module identity ([`Resolver::package_identity`],
//! [`crate::identity`]).

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::{
    Base, DirCache, EnvRoot, Externals, ExtraEdge, FileId, Memo, PackageIdentity, Phase, ProjectCx,
    RawRef, RefFlags, RefKind, Resolution, ResolveCx, Resolver, Specifier, Target, Uncertainty,
    UncertaintyKind,
    fs::normalize,
    os::OsStrExt,
    record,
    resolve_path::{
        MissingAsset,
        assets::{computed_specifier, explain_missing, resolve_alternatives, unsupported_base},
    },
};

use crate::{
    ASSET_RULES, GoOptions, GoRoot,
    constraint::BuildContext,
    embed,
    extract::{CGO_SYSTEM_BASE, IMPORT_BASE},
    modules::{self, Ctx, PkgLoc, VendorMode, has_go_files, is_bundled_std, is_std_like},
    package::{FileClass, PackageFiles, package_files},
};

/// Resolver state for one tracer.
pub struct GoResolver {
    bcx: BuildContext,
    modcache: PathBuf,
    goroot: Option<PathBuf>,
    vendor: VendorMode,
    workspace: bool,
    base: PathBuf,
    externals: Externals,
    module_roots: Memo<PathBuf, Option<PathBuf>>,
    go_works: Memo<PathBuf, Option<PathBuf>>,
    ctxs: Memo<PathBuf, Arc<Ctx>>,
    primary: Memo<(), Option<Arc<Ctx>>>,
    packages: Memo<PathBuf, Arc<PackageFiles>>,
    imports: Memo<(PathBuf, String), Arc<PkgLoc>>,
    goroot_std: Memo<String, bool>,
    /// `go.sum` hashes of the trace base's module or workspace, for identities.
    sums: std::sync::OnceLock<(turborepo_trace_lockfile::gosum::GoSum, Vec<PathBuf>)>,
}

impl GoResolver {
    /// Creates the resolver from the tracer configuration and the language
    /// options.
    pub fn new(project: &ProjectCx<'_>, opts: &GoOptions) -> Self {
        let platform = project.options.platform();
        let host = turborepo_trace_core::Platform::host();
        let env_cgo = std::env::var("CGO_ENABLED").ok();
        let cgo = opts.cgo.unwrap_or_else(|| match env_cgo.as_deref() {
            Some("0") => false,
            Some("1") => true,
            // Go enables cgo for native builds and disables it when cross-compiling.
            _ => platform.go_os() == host.go_os() && platform.go_arch() == host.go_arch(),
        });
        let mut tags = opts.tags.clone();
        let mut vendor = opts.vendor;
        if let Ok(flags) = std::env::var("GOFLAGS") {
            for f in flags.split_ascii_whitespace() {
                if let Some(t) = f.strip_prefix("-tags=") {
                    tags.extend(t.split(',').filter(|t| !t.is_empty()).map(String::from));
                }
                if opts.vendor == VendorMode::Auto {
                    match f {
                        "-mod=vendor" => vendor = VendorMode::On,
                        "-mod=mod" | "-mod=readonly" => vendor = VendorMode::Off,
                        _ => {}
                    }
                }
            }
        }
        let bcx = BuildContext {
            platform,
            tags,
            cgo,
            go_minor: opts.go_minor.unwrap_or(crate::constraint::LATEST_GO_MINOR),
            all_targets: project.options.all_targets,
        };
        let goroot = match &opts.goroot {
            GoRoot::Auto => modules::default_goroot(),
            GoRoot::Path(p) => Some(normalize(p)),
            GoRoot::None => None,
        };
        let workspace = opts
            .workspace
            .unwrap_or_else(|| std::env::var("GOWORK").map_or(true, |v| v != "off"));
        GoResolver {
            bcx,
            modcache: normalize(
                &opts
                    .modcache
                    .clone()
                    .unwrap_or_else(modules::default_modcache),
            ),
            goroot,
            vendor,
            workspace,
            base: project.options.base.clone(),
            externals: project.options.externals,
            module_roots: Memo::new(),
            go_works: Memo::new(),
            ctxs: Memo::new(),
            primary: Memo::new(),
            packages: Memo::new(),
            imports: Memo::new(),
            goroot_std: Memo::new(),
            sums: std::sync::OnceLock::new(),
        }
    }

    /// The module cache in use.
    pub fn modcache(&self) -> &Path {
        &self.modcache
    }

    /// The build context in use.
    pub fn build_context(&self) -> &BuildContext {
        &self.bcx
    }

    fn in_modcache(&self, p: &Path) -> bool {
        p.starts_with(&self.modcache)
    }

    /// The nearest directory at or above `dir` with a `go.mod`.
    fn module_root(&self, fs: &DirCache, dir: &Path) -> Option<PathBuf> {
        self.module_roots.get(fs, &dir.to_path_buf(), || {
            if fs.is_file(&dir.join("go.mod")) {
                return Some(dir.to_path_buf());
            }
            if dir == self.modcache {
                return None;
            }
            dir.parent()
                .filter(|p| *p != dir)
                .and_then(|p| self.module_root(fs, p))
        })
    }

    /// The nearest `go.work` at or above `dir`.
    fn go_work(&self, fs: &DirCache, dir: &Path) -> Option<PathBuf> {
        if !self.workspace {
            return None;
        }
        self.go_works.get(fs, &dir.to_path_buf(), || {
            let w = dir.join("go.work");
            if fs.is_file(&w) {
                return Some(w);
            }
            dir.parent()
                .filter(|p| *p != dir)
                .and_then(|p| self.go_work(fs, p))
        })
    }

    fn ctx(&self, fs: &DirCache, key: &Path) -> Arc<Ctx> {
        self.ctxs.get(fs, &key.to_path_buf(), || {
            Arc::new(if key.file_name().is_some_and(|n| n == "go.work") {
                modules::work_ctx(fs, key, self.vendor, &self.modcache)
            } else {
                let root = key.parent().unwrap_or(Path::new("/"));
                modules::module_ctx(fs, root, self.vendor, &self.modcache)
            })
        })
    }

    /// The context of the trace base: its `go.work`, else its module.
    fn primary(&self, fs: &DirCache) -> Option<Arc<Ctx>> {
        self.primary.get(fs, &(), || {
            if let Some(w) = self.go_work(fs, &self.base) {
                return Some(self.ctx(fs, &w));
            }
            self.module_root(fs, &self.base)
                .map(|r| self.ctx(fs, &r.join("go.mod")))
        })
    }

    /// The resolution context of a file in `dir`.
    fn ctx_for(&self, fs: &DirCache, dir: &Path) -> Option<Arc<Ctx>> {
        if self.in_modcache(dir) {
            if let Some(p) = self.primary(fs) {
                return Some(p);
            }
            let (root, _, _) = modules::modcache_module(&self.modcache, dir)?;
            return Some(self.ctx(fs, &root.join("go.mod")));
        }
        let root = self.module_root(fs, dir)?;
        if let Some(w) = self.go_work(fs, &root)
            && modules::work_uses(fs, &w).contains(&root)
        {
            return Some(self.ctx(fs, &w));
        }
        if let Some(p) = self.primary(fs)
            && (p.is_main_root(&root)
                || p.local_replace_dirs().any(|d| d == root)
                || p.vendor.as_ref().is_some_and(|v| dir.starts_with(&v.dir)))
        {
            return Some(p);
        }
        Some(self.ctx(fs, &root.join("go.mod")))
    }

    /// The `go.mod` of the module containing a file in `dir`.
    fn own_go_mod(&self, fs: &DirCache, dir: &Path) -> Option<PathBuf> {
        if self.in_modcache(dir) {
            let m = modules::modcache_module(&self.modcache, dir)?
                .0
                .join("go.mod");
            return fs.is_file(&m).then_some(m);
        }
        self.module_root(fs, dir).map(|r| r.join("go.mod"))
    }

    fn is_goroot_std(&self, fs: &DirCache, import: &str) -> bool {
        let Some(g) = &self.goroot else { return false };
        self.goroot_std.get(fs, &import.to_owned(), || {
            record::pause(|| has_go_files(fs, &g.join("src").join(import)))
        })
    }

    /// Where `import`, written in a file in `from_dir`, is.
    pub fn locate(&self, fs: &DirCache, from_dir: &Path, import: &str) -> Arc<PkgLoc> {
        if import == "C" || is_bundled_std(import) {
            return Arc::new(PkgLoc::Std);
        }
        if import.starts_with("./") || import.starts_with("../") || import == "." {
            return Arc::new(PkgLoc::Dir {
                dir: normalize(&from_dir.join(import)),
                module: None,
                go_mod: None,
            });
        }
        if is_std_like(import) && self.is_goroot_std(fs, import) {
            return Arc::new(PkgLoc::Std);
        }
        let Some(ctx) = self.ctx_for(fs, from_dir) else {
            return Arc::new(PkgLoc::Missing(if is_std_like(import) {
                format!("package {import} is not in std")
            } else {
                format!(
                    "cannot find package {import}: no go.mod above {}",
                    from_dir.display()
                )
            }));
        };
        self.imports
            .get(fs, &(ctx.key.clone(), import.to_owned()), || {
                let loc = ctx.locate(fs, import);
                Arc::new(match loc {
                    PkgLoc::Missing(_) if is_std_like(import) && ctx.main_for(import).is_none() => {
                        PkgLoc::Missing(format!("package {import} is not in std"))
                    }
                    other => other,
                })
            })
    }

    /// The package files of `dir`, with its `(dir, "*")` membership.
    pub fn package(&self, fs: &DirCache, dir: &Path) -> Arc<PackageFiles> {
        self.packages.get(fs, &dir.to_path_buf(), || {
            let pf = package_files(fs, dir, &self.bcx);
            if fs.records() {
                record::listing(dir, "*");
                for p in &pf.read_excluded {
                    record::read(p);
                }
            }
            Arc::new(pf)
        })
    }

    fn resolve_import(&self, r: &RawRef<'_>, import: &str, cx: &ResolveCx<'_>) -> Resolution {
        let loc = self.locate(cx.fs, cx.from_dir, import);
        let mut manifests: Vec<PathBuf> = Vec::new();
        let ctx = self.ctx_for(cx.fs, cx.from_dir);
        let (dir, module, go_mod) = match &*loc {
            PkgLoc::Std => return Resolution::target(Target::Builtin(import.into())),
            PkgLoc::Missing(why) => {
                return Resolution::unresolved(Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    r.span,
                    why.clone(),
                ));
            }
            PkgLoc::NotDownloaded(modules) => {
                return self.not_downloaded(r, import, modules, ctx.as_deref(), cx);
            }
            PkgLoc::Dir {
                dir,
                module,
                go_mod,
            } => (dir, module, go_mod),
        };
        manifests.extend(go_mod.iter().cloned());
        if let Some(c) = &ctx {
            manifests.extend(c.manifests.iter().cloned());
        }
        manifests.extend(self.own_go_mod(cx.fs, cx.from_dir));
        let mut res = if let (Some((m, v)), false) = (module, self.externals == Externals::Follow) {
            Resolution::target(Target::External(format!("{m}@{v}").into()))
        } else {
            self.package_resolution(r, dir, false, None, cx)
        };
        manifests.sort();
        manifests.dedup();
        for m in manifests {
            if cx.fs.is_file(&m) {
                res.extra.push(manifest_edge(&m, cx));
            }
        }
        res
    }

    /// An import whose providing module is known from the build list but is not
    /// in the module cache: one [`Target::External`] `module@version` per
    /// candidate module, the manifests of a resolved import, and an
    /// [`UncertaintyKind::UnresolvedSpecifier`] record (the package's files
    /// are not traced). The graph store links such a target to the module's
    /// opaque package through [`Resolver::external_identity`] and drops the
    /// record: what the import means is decided by `go.mod` and `go.sum`,
    /// which are inputs of the file.
    fn not_downloaded(
        &self,
        r: &RawRef<'_>,
        import: &str,
        modules: &[(String, String, PathBuf)],
        ctx: Option<&Ctx>,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let names: Vec<String> = modules.iter().map(|(m, v, _)| format!("{m}@{v}")).collect();
        let mut res = Resolution {
            targets: names
                .iter()
                .map(|n| Target::External(n.as_str().into()))
                .collect(),
            ..Resolution::default()
        };
        res.uncertainty = Some(Uncertainty::new(
            UncertaintyKind::UnresolvedSpecifier,
            r.span,
            format!(
                "package {import}: module {} is required but not in the module cache",
                names.join(" or ")
            ),
        ));
        let mut manifests: Vec<PathBuf> = Vec::new();
        if let Some(c) = ctx {
            manifests.extend(c.manifests.iter().cloned());
        }
        manifests.extend(self.own_go_mod(cx.fs, cx.from_dir));
        manifests.sort();
        manifests.dedup();
        for m in manifests {
            if cx.fs.is_file(&m) {
                res.extra.push(manifest_edge(&m, cx));
            }
        }
        res
    }

    /// The targets for package `dir`: its Go files (plus tests with `tests`),
    /// other files and conditional files. `exclude` is left out (the
    /// referencing file itself).
    fn package_resolution(
        &self,
        r: &RawRef<'_>,
        dir: &Path,
        tests: bool,
        exclude: Option<&Path>,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let pf = self.package(cx.fs, dir);
        let keep = |p: &&PathBuf| Some(p.as_path()) != exclude;
        let mut go: Vec<&PathBuf> = pf.go.iter().filter(keep).collect();
        if tests {
            go.extend(pf.tests.iter().filter(keep));
            go.sort();
        }
        let mut res = if !go.is_empty() {
            Resolution::target(glob(cx, format!("{}/*.go", dir.display()), &go))
        } else if exclude.is_some() {
            // A file alone in its package: no mates.
            Resolution::default()
        } else {
            let why = if pf.any_go {
                format!(
                    "build constraints exclude all Go files in {} for {}/{}",
                    dir.display(),
                    self.bcx.platform.go_os(),
                    self.bcx.platform.go_arch()
                )
            } else {
                format!("no Go files in {}", dir.display())
            };
            let kind = if pf.any_go {
                UncertaintyKind::ConditionalResolution
            } else {
                UncertaintyKind::UnresolvedSpecifier
            };
            Resolution::unresolved(Uncertainty::new(kind, r.span, why))
        };
        let other: Vec<&PathBuf> = pf.other.iter().filter(keep).collect();
        if !other.is_empty() {
            res.extra.push(ExtraEdge::new(
                RefKind::Package,
                glob(cx, format!("{}/*.[chsS]*", dir.display()), &other),
                "package (cgo, assembly and object files)",
            ));
        }
        for c in &pf.conditional {
            if (c.class == FileClass::Test && !tests) || Some(c.path.as_path()) == exclude {
                continue;
            }
            let kind = match c.class {
                FileClass::Go | FileClass::Test => r.kind,
                FileClass::Other => RefKind::Package,
            };
            res.extra.push(
                ExtraEdge::new(
                    kind,
                    Target::File(cx.intern(&c.path)),
                    format!(
                        "{} (other targets: {})",
                        c.path.file_name().unwrap_or_default().to_string_lossy(),
                        c.why
                    ),
                )
                .with_flags(RefFlags::CONDITIONAL),
            );
        }
        res
    }

    fn resolve_embed(&self, r: &RawRef<'_>, spec: &str, cx: &ResolveCx<'_>) -> Resolution {
        let dir = cx.from_dir;
        let p = match embed::parse(spec) {
            Ok(p) => p,
            Err(e) => {
                return Resolution::unresolved(Uncertainty::new(
                    UncertaintyKind::UnsupportedSyntax,
                    r.span,
                    format!("//go:embed {spec}: {e}"),
                ));
            }
        };
        let result = record::pause_listings(|| embed::expand(cx.fs, dir, &p, cx.options.max_glob));
        if cx.fs.records() {
            record::listing(dir, &embed::membership_glob(p.glob));
        }
        match result {
            Ok(exp) => {
                if cx.fs.records() {
                    for d in &exp.dirs {
                        record::listing(d, "**");
                    }
                }
                let pattern = format!("{}/{spec}", dir.display());
                let truncated = exp.truncated;
                let mut res = Resolution::target(Target::Glob {
                    pattern: pattern.clone().into(),
                    matches: exp.files.iter().map(|f| cx.intern(f)).collect(),
                    truncated,
                });
                if truncated {
                    res.uncertainty = Some(Uncertainty::new(
                        UncertaintyKind::PatternTooBroad,
                        r.span,
                        format!(
                            "{pattern}: more than {} files; truncated",
                            cx.options.max_glob
                        ),
                    ));
                }
                res
            }
            Err(e) => Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::UnresolvedSpecifier,
                r.span,
                format!("//go:embed {spec}: {e}"),
            )),
        }
    }

    fn resolve_include(&self, r: &RawRef<'_>, spec: &str, cx: &ResolveCx<'_>) -> Resolution {
        let path = normalize(&cx.from_dir.join(spec));
        if cx.fs.is_file(&path) {
            return Resolution::file(cx.intern(&path));
        }
        Resolution::unresolved(Uncertainty::new(
            UncertaintyKind::UnresolvedSpecifier,
            r.span,
            format!(
                "#include \"{spec}\" not found next to the file (cgo CFLAGS include paths are not \
                 modelled)"
            ),
        ))
    }

    fn resolve_spec(&self, r: &RawRef<'_>, spec: &Specifier<'_>, cx: &ResolveCx<'_>) -> Resolution {
        match (r.kind, spec) {
            (RefKind::Import, Specifier::Literal(s)) if r.base == IMPORT_BASE => {
                self.resolve_import(r, s, cx)
            }
            (RefKind::Import, Specifier::Literal(s)) if r.base == CGO_SYSTEM_BASE => {
                Resolution::target(Target::Builtin(format!("<{s}>").into()))
            }
            (RefKind::Import, Specifier::Literal(s)) => self.resolve_include(r, s, cx),
            (RefKind::Embed, Specifier::Literal(s)) => self.resolve_embed(r, s, cx),
            // A path value anchored at the source directory that reached no tracked API (the
            // shared anchored-asset policy).
            (RefKind::Asset, Specifier::Literal(_) | Specifier::Pattern(_))
                if r.base == Base::FileDir =>
            {
                ASSET_RULES.resolve_anchored_asset(r, spec, cx.from_dir, MissingAsset::Report, cx)
            }
            (_, Specifier::Literal(s)) if r.base != IMPORT_BASE => {
                ASSET_RULES.resolve_path(r, s, cx)
            }
            (_, Specifier::Pattern(parts)) => ASSET_RULES.resolve_pattern(r, parts, cx),
            (_, Specifier::Alternatives(alts)) => {
                resolve_alternatives(r, spec, alts, |a| self.resolve_spec(r, a, cx))
            }
            (_, Specifier::Unknown) => computed_specifier(r, spec),
            _ => unsupported_base(r),
        }
    }
}

fn glob(cx: &ResolveCx<'_>, pattern: String, files: &[&PathBuf]) -> Target {
    Target::Glob {
        pattern: pattern.into(),
        matches: files.iter().map(|p| cx.intern(p)).collect(),
        truncated: false,
    }
}

impl Resolver for GoResolver {
    fn resolve(&self, r: &RawRef<'_>, _from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let res = self.resolve_spec(r, &r.spec, cx);
        if r.kind == RefKind::SharedLib {
            // `plugin.Open`: keep the extractor's cwd-relative record for a missing plugin.
            return explain_missing(r, res);
        }
        res
    }

    fn file_edges(&self, _file: FileId, cx: &ResolveCx<'_>) -> Vec<ExtraEdge> {
        let path = cx.from_path;
        let name = path.file_name().map(|n| n.as_bytes()).unwrap_or_default();
        if !name.ends_with(b".go") {
            return Vec::new();
        }
        let is_test = name.ends_with(b"_test.go");
        let dir = cx.from_dir;
        // The package scope: everything compiled together with this file.
        let r = RawRef::new(
            RefKind::Package,
            Specifier::literal("package"),
            turborepo_trace_core::Span::DUMMY,
        )
        .with_base(Base::FileDir);
        let res = self.package_resolution(&r, dir, is_test, Some(path), cx);
        let mut out = Vec::new();
        for t in res.targets {
            if !matches!(t, Target::Unresolved) {
                out.push(ExtraEdge::new(RefKind::Package, t, "package"));
            }
        }
        out.extend(res.extra);
        // Manifests: the module's go.mod and the context's go.sum/go.work/modules.txt.
        let mut manifests: Vec<PathBuf> = Vec::new();
        manifests.extend(self.own_go_mod(cx.fs, dir));
        if let Some(c) = self.ctx_for(cx.fs, dir) {
            manifests.extend(c.manifests.iter().cloned());
        }
        manifests.sort();
        manifests.dedup();
        for m in manifests {
            if cx.fs.is_file(&m) {
                out.push(manifest_edge(&m, cx));
            }
        }
        out
    }

    fn environment_roots(&self) -> Vec<EnvRoot> {
        let mut out = vec![EnvRoot::new("go:modcache", self.modcache.clone())];
        if let Some(g) = &self.goroot {
            out.push(EnvRoot::new("go:goroot", g.clone()));
        }
        out
    }

    fn package_identity(&self, path: &Path, fs: &DirCache) -> Option<PackageIdentity> {
        let b = path.as_os_str().as_bytes();
        if !self.in_modcache(path) && memchr::memmem::find(b, b"/vendor/").is_none() {
            return None;
        }
        let (sums, pins) = self.sums.get_or_init(|| {
            let root = self
                .go_work(fs, &self.base)
                .and_then(|w| w.parent().map(Path::to_path_buf))
                .or_else(|| self.module_root(fs, &self.base))
                .unwrap_or_else(|| self.base.clone());
            let sums = crate::identity::load_sums(fs, &root);
            let pins = ["go.sum", "go.work.sum"]
                .iter()
                .map(|f| root.join(f))
                .filter(|p| fs.is_file(p))
                .collect();
            (sums, pins)
        });
        let id = crate::identity::module_identity_in(fs, path, Some(&self.modcache), Some(sums))?;
        let mut pins = pins.clone();
        if !id.root.starts_with(&self.modcache) {
            // vendor/<module>: vendor/modules.txt pins the version.
            let mut d = id.root.as_path();
            while let Some(p) = d.parent() {
                if p.file_name().is_some_and(|n| n == "vendor") {
                    pins.push(p.join("modules.txt"));
                    break;
                }
                d = p;
            }
        }
        Some(PackageIdentity {
            ecosystem: "go",
            name: id.module,
            version: id.version,
            integrity: id.sum,
            root: id.root,
            pins,
        })
    }

    fn external_identity(&self, name: &str, fs: &DirCache) -> Option<PackageIdentity> {
        // `module@version` of a module that is not in the module cache: the identity
        // its files would have there.
        let (module, version) = name.rsplit_once('@')?;
        if module.is_empty() || !version.starts_with('v') {
            return None;
        }
        let root = self.modcache.join(format!(
            "{}@{}",
            modules::escape(module),
            modules::escape(version)
        ));
        self.package_identity(&root.join("go.mod"), fs)
    }

    fn clear_cache(&self) {
        self.module_roots.clear();
        self.go_works.clear();
        self.ctxs.clear();
        self.primary.clear();
        self.packages.clear();
        self.imports.clear();
        self.goroot_std.clear();
    }
}

/// A `go.mod`/`go.sum`/`go.work`/`vendor/modules.txt` manifest edge (build
/// phase).
fn manifest_edge(m: &Path, cx: &ResolveCx<'_>) -> ExtraEdge {
    let spec = m
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ExtraEdge::new(RefKind::Manifest, Target::File(cx.intern(m)), spec).with_phase(Phase::Build)
}
