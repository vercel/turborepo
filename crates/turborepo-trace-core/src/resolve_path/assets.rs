//! Resolution of runtime file references (fs reads, directory listings, assets)
//! and of partially known module paths, following nft's rules. Shared by every
//! language; each one configures [`AssetRules`] (JS: `node_modules` boundary
//! and nft's excluded files).
//!
//! What [`AssetRules`] does with a non-module reference (`FsRead`, `FsReadDir`,
//! `Asset`), relative to its [`turborepo_trace_core::Base`]:
//!
//! - [`AssetRules::resolve_path`] (literal): an existing file is a leaf; an
//!   existing directory expands to every file below it
//!   ([`AssetRules::resolve_dir`]); a missing path is normal for probes
//!   (`existsSync`, `os.path.exists`) and kept as a quiet
//!   [`Target::Unresolved`] edge without uncertainty, unless it only matches
//!   with different casing ([`UncertaintyKind::CaseMismatch`]).
//! - [`AssetRules::resolve_pattern`] (`'../data/' + x + '.json'`): glob against
//!   the dir cache, never from a leading wildcard, capped by `--max-glob`
//!   ([`UncertaintyKind::PatternTooBroad`] when truncated).
//! - [`AssetRules::resolve_module_pattern`]: a module reference with a pattern
//!   (webpack context modules, nft wildcard requires), with extension variants.
//! - [`resolve_alternatives`]: each branch resolved and merged.
//!
//! nft's rules, parameterized:
//!
//! - Globs never descend into [`AssetRules::skip_dirs`] below their static root
//!   (nft adds `ignore: <root>/**/node_modules/**`). The static root itself may
//!   be inside one.
//! - Files rejected by [`AssetRules::keep`] are never matched (nft's
//!   `excludeAssetExtensions`/`excludeAssetFiles` for JS).
//! - A directory used as an asset is not expanded as a whole if it is the
//!   referencing file's own directory (unless `FsReadDir`), the runtime cwd, a
//!   package marker directory such as `node_modules` itself, an ancestor of the
//!   referencing file's directory, or `/` (nft's `validWildcard`;
//!   [`AssetRules::invalid_dir`]).
//! - A file inside a package tree never emits assets outside its boundary
//!   ([`PackageBoundary::is_outside`]).
//!
//! # Host paths
//!
//! In a confined trace ([`turborepo_trace_core::TraceOptions::confined`]: graph
//! mode) a path outside the base is a *host path*
//! ([`ResolveCx::is_host_path`]): `/tmp/out.json`, `/proc/<pid>/stat`,
//! `/etc/passwd`, `../sibling-checkout/x`. What is there depends on the
//! machine, never on a file of the repository, so the file system is not asked
//! about it ([`host_reference`]):
//!
//! | Reference | Result |
//! |---|---|
//! | file read, directory listing, glob, asset path | an *external reference*: an unresolved edge that keeps the specifier. No existence check, no expansion, no match count, no node for the host file, no uncertainty of the resolver's own |
//! | module load or execution that the source writes as an absolute path below a temporary directory ([`is_temp_path`]) | an open `computed_specifier` record with origin data: the module was generated at run time from files the site does not name |
//! | module load or execution elsewhere | an `unresolved_specifier` record: the same result as on a machine where the path is missing |
//!
//! A path that is only possibly outside (an unknown base) is not a host path
//! and keeps its record.
//!
//! What a host path is must not depend on where the checkout is, so it is
//! decided on what the source writes ([`host_reference`]): a relative path that
//! leaves the repository is "elsewhere" also below `/tmp`, and a path written
//! as absolute, or one that climbs above the base, is a host path also when it
//! lands inside the checkout of this machine ([`ResolveCx::names_host_path`]).
//!
//! Such a path is not looked up where it lands, and it may still be this
//! repository's: code that runs in a container at `/app` names its own files by
//! that place. So next to the external reference it gets what it names when the
//! repository is seen from another place ([`AssetRules::readings`],
//! [`with_readings`]): every file that its trailing components name from the
//! root (for a climb, the one path that follows the directories
//! above the root). A reading that exists is an edge with the reading as its
//! specifier ([`turborepo_trace_core::output::ELSEWHERE_PREFIX`]); one that
//! does not leaves the probes of its lookup. The same at every place, and an
//! over-approximation rather than an assumption.
//!
//! The directories above the base are host paths too (`readdir('/tmp')` is the
//! same reference whether the checkout lives in `/work/repo` or in
//! `/tmp/ci/repo`), with one exception, for patterns ([`host_pattern_dir`]): a
//! pattern that climbs above the base by relative steps, or starts at `/`
//! itself, can lead back into the repository (`path.join(root, '..', name)`,
//! `'/' + file`). It is not an external reference, and it is not expanded
//! either, since what lies next to the checkout is the machine's: it keeps
//! an open `pattern_too_broad` record ([`above_base_pattern`]).

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

pub use crate::fs::is_temp_path;
use crate::{
    Base, ExtraEdge, FileId, Origin, PackageBoundary, Part, RawRef, RefFlags, RefKind, Resolution,
    ResolveCx, Span, Specifier, Target, Uncertainty, UncertaintyKind, UncertaintyScope,
    eval::{ExpandOptions, Expansion, PatPart, expand_with},
    fs::{FileKind, normalize},
    output::{ELSEWHERE_PREFIX, SPECIAL_CASE_PREFIX},
    resolve_path::{PathResolver, PathResolverConfig},
};

/// A file filter that keeps everything.
pub fn keep_all(_: &Path) -> bool {
    true
}

/// A language's asset configuration (see the module docs).
#[derive(Copy, Clone, Debug)]
pub struct AssetRules {
    /// Package layout: containment, package roots and marker directories.
    pub boundary: PackageBoundary,
    /// Directory names globs never enter below their static root (JS:
    /// `node_modules`).
    pub skip_dirs: &'static [&'static str],
    /// Files assets may match (JS: everything except C sources, build files,
    /// READMEs and changelogs). [`keep_all`] keeps everything.
    pub keep: fn(&Path) -> bool,
}

/// What [`AssetRules::resolve_anchored_asset`] does with a path that does not
/// exist.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MissingAsset {
    /// A quiet probe without uncertainty (nft's behavior; JS).
    Quiet,
    /// An [`UncertaintyKind::UnresolvedSpecifier`] record.
    Report,
}

/// The result of a glob, ready to become a [`Target::Glob`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobOut {
    /// Display pattern (absolute).
    pub pattern: String,
    /// Matches, sorted.
    pub matches: Vec<PathBuf>,
    /// More matches existed than the limit.
    pub truncated: bool,
}

impl GlobOut {
    /// Interns the matches into a [`Target::Glob`].
    pub fn into_target(self, cx: &ResolveCx<'_>) -> Target {
        Target::Glob {
            pattern: self.pattern.into(),
            matches: self.matches.iter().map(|p| cx.intern(p)).collect(),
            truncated: self.truncated,
        }
    }
}

impl AssetRules {
    /// Rules for `boundary` that skip no directories and keep every file.
    pub const fn new(boundary: PackageBoundary) -> Self {
        AssetRules {
            boundary,
            skip_dirs: &[],
            keep: keep_all,
        }
    }

    /// Expands `parts` relative to `base` with [`AssetRules::skip_dirs`] and
    /// the file filter `keep` (often [`AssetRules::keep`]), capped by
    /// `--max-glob`. `None` if the pattern was refused (leading wildcard or
    /// anchored at `/`).
    pub fn glob(
        &self,
        parts: &[PatPart],
        base: &Path,
        cx: &ResolveCx<'_>,
        keep: fn(&Path) -> bool,
    ) -> Option<GlobOut> {
        let opts = ExpandOptions {
            limit: cx.options.max_glob,
            skip_dirs: self.skip_dirs,
            keep_file: Some(keep),
        };
        match expand_with(parts, base, cx.fs, &opts) {
            Expansion::Matches {
                pattern,
                matches,
                truncated,
            } => Some(GlobOut {
                pattern,
                matches,
                truncated,
            }),
            Expansion::Refused(_) => None,
        }
    }

    /// Every file below `dir` matching `keep`, as `<dir>/**`.
    pub fn tree_glob(&self, dir: &Path, cx: &ResolveCx<'_>, keep: fn(&Path) -> bool) -> GlobOut {
        let mut prefix = dir.as_os_str().to_string_lossy().into_owned();
        if !prefix.ends_with('/') {
            prefix.push('/');
        }
        let parts = [
            PatPart::Lit(prefix),
            PatPart::Wild {
                reason: "**".into(),
            },
        ];
        let mut g = self
            .glob(&parts, Path::new("/"), cx, keep)
            .unwrap_or_default();
        g.pattern = format!("{}/**", dir.display());
        g
    }

    /// Every asset below `dir` ([`AssetRules::keep`]), as `<dir>/**`.
    pub fn dir_glob(&self, dir: &Path, cx: &ResolveCx<'_>) -> GlobOut {
        self.tree_glob(dir, cx, self.keep)
    }

    /// Why a directory may not be expanded as an asset (nft's `validWildcard`),
    /// or `None` if it may. `allow_own_dir` admits the referencing file's
    /// own directory (nft special-cases `fs.readdirSync(__dirname)`), also
    /// when it is the runtime cwd.
    pub fn invalid_dir(
        &self,
        dir: &Path,
        cx: &ResolveCx<'_>,
        allow_own_dir: bool,
    ) -> Option<Cow<'static, str>> {
        if dir == cx.from_dir && !allow_own_dir {
            return Some("the referencing file's own directory".into());
        }
        // `fs.readdirSync(__dirname)` lists the own directory even when it is also the
        // cwd.
        if dir == cx.options.cwd && !(allow_own_dir && dir == cx.from_dir) {
            return Some("the runtime working directory".into());
        }
        if let Some(m) = self.boundary.marker_of_dir(dir) {
            return Some(format!("a {m} directory").into());
        }
        if cx.from_dir != dir && cx.from_dir.starts_with(dir) {
            return Some("an ancestor of the referencing file's directory".into());
        }
        if dir.as_os_str() == "/" {
            return Some("the filesystem root".into());
        }
        None
    }

    /// Whether `target` is outside the package boundary of the referencing file
    /// ([`PackageBoundary::is_outside_from`]). A marker directory above the
    /// checkout is no
    /// boundary ([`turborepo_trace_core::DirCache::local_offset`]).
    fn is_outside(&self, cx: &ResolveCx<'_>, target: &Path) -> bool {
        self.boundary
            .is_outside_from(cx.from_path, cx.fs.local_offset(cx.from_path), target)
    }

    /// Non-module references: a file or directory relative to the reference's
    /// base.
    pub fn resolve_path(&self, r: &RawRef<'_>, spec: &str, cx: &ResolveCx<'_>) -> Resolution {
        let Some(base) = cx.base_dir(&r.base) else {
            return unsupported_base(r);
        };
        self.resolve_path_at(r, spec, &base, located(r), cx)
    }

    /// [`AssetRules::resolve_path`] relative to an explicit directory instead
    /// of the reference's [`turborepo_trace_core::Base`]: for language
    /// anchors such as a crate root or a package directory, which the
    /// language resolves itself. Nothing is interned for the base.
    pub fn resolve_path_in(
        &self,
        r: &RawRef<'_>,
        spec: &str,
        base: &Path,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        self.resolve_path_at(r, spec, base, true, cx)
    }

    /// [`AssetRules::resolve_path_in`]; `located`: `base` is a directory the
    /// language knows (see [`AssetRules::readings`]).
    fn resolve_path_at(
        &self,
        r: &RawRef<'_>,
        spec: &str,
        base: &Path,
        located: bool,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let path = normalize(&base.join(spec));
        // Outside the repository, or inside it at this place only (an absolute path, a
        // climb above the base that comes back in through the checkout's own name): not
        // looked up where it lands, and read as the repository seen from another place.
        if cx.is_host_path(&path) || cx.names_host_path(base, spec) {
            let host = host_reference(r, &path, spec, cx);
            let mut readings = self.readings(base, located, spec, spec, false, cx);
            if r.kind == RefKind::FsReadDir {
                readings.extend(self.root_reading(spec, spec, cx));
            }
            return with_readings(host, r, &readings, |reading| {
                self.resolve_path_in(r, &reading.rel, &cx.options.base, cx)
            });
        }
        let probe = is_probe_kind(r.kind);
        if probe && self.is_outside(cx, &path) {
            return Resolution::target(Target::Unresolved);
        }
        match cx.fs.stat(&path).map(|s| s.kind) {
            Some(FileKind::File) => Resolution::file(cx.intern(&path)),
            Some(FileKind::Dir) if probe => self.resolve_dir(r, &path, cx),
            _ => {
                let resolver = PathResolver::new(PathResolverConfig {
                    allow_dirs: r.kind == RefKind::FsReadDir,
                    detect_case_mismatch: true,
                    ..Default::default()
                });
                let res = resolver.resolve_to(spec, base, cx, r.span);
                let case_mismatch = res
                    .uncertainty
                    .as_ref()
                    .is_some_and(|u| u.kind == UncertaintyKind::CaseMismatch);
                if probe && !case_mismatch {
                    // A probe of a missing path (`existsSync`) is normal: keep a quiet record.
                    return Resolution::target(Target::Unresolved);
                }
                res
            }
        }
    }

    /// An existing directory used by a non-module reference: every file below
    /// it.
    pub fn resolve_dir(&self, r: &RawRef<'_>, dir: &Path, cx: &ResolveCx<'_>) -> Resolution {
        if cx.is_host_path(dir) {
            return host_reference(r, dir, "", cx);
        }
        if let Some(why) = self.invalid_dir(dir, cx, r.kind == RefKind::FsReadDir) {
            // Reading the cwd or an ancestor as a whole is not modelled; a plain fs probe
            // of such a directory (`existsSync(process.cwd())`) is harmless.
            if r.kind == RefKind::FsRead {
                return Resolution::target(Target::Unresolved);
            }
            return Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::PatternTooBroad,
                r.span,
                format!("directory {} is {why}; not expanded", display_rel(dir, cx)),
            ));
        }
        let g = self.dir_glob(dir, cx);
        let truncated = g.truncated;
        let pattern = g.pattern.clone();
        let mut res = Resolution::target(g.into_target(cx));
        if truncated {
            // Covered: a non-module reference reads contents below `dir` only, so the
            // directory bounds every file it can read (the matches beyond the cap
            // included).
            let u = Uncertainty::new(
                UncertaintyKind::PatternTooBroad,
                r.span,
                format!(
                    "{pattern}: more than {} files; truncated",
                    cx.options.max_glob
                ),
            );
            res.uncertainty = Some(if names_only(r) && is_probe_kind(r.kind) {
                names_only_truncated(r, u.detail.to_string())
            } else if is_probe_kind(r.kind) {
                u.with_scope(UncertaintyScope::below(dir))
            } else {
                u
            });
        }
        res
    }

    /// A non-module reference (fs read, readdir, asset) with a partially known
    /// path.
    pub fn resolve_pattern(
        &self,
        r: &RawRef<'_>,
        parts: &[Part<'_>],
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let Some(base) = cx.base_dir(&r.base) else {
            return unsupported_base(r);
        };
        self.resolve_pattern_at(r, parts, &base, located(r), cx)
    }

    /// [`AssetRules::resolve_pattern`] relative to an explicit directory (see
    /// [`AssetRules::resolve_path_in`]).
    pub fn resolve_pattern_in(
        &self,
        r: &RawRef<'_>,
        parts: &[Part<'_>],
        base: &Path,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        self.resolve_pattern_at(r, parts, base, true, cx)
    }

    /// [`AssetRules::resolve_pattern_in`]; `located`: `base` is a directory the
    /// language knows (see [`AssetRules::readings`]).
    fn resolve_pattern_at(
        &self,
        r: &RawRef<'_>,
        parts: &[Part<'_>],
        base: &Path,
        located: bool,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let written = parts;
        let parts: Vec<PatPart> = parts.iter().map(PatPart::from).collect();
        let too_broad = |why: String| {
            Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::PatternTooBroad,
                r.span,
                format!("{}: {why}; not expanded", r.spec),
            ))
        };
        let Some(PatPart::Lit(first)) = parts.first() else {
            return too_broad("starts with a wildcard".into());
        };
        // The static directory: the literal prefix up to its last `/`.
        let static_dir = match first.rfind('/') {
            Some(i) => normalize(&base.join(&first[..=i])),
            None => base.to_path_buf(),
        };
        if host_pattern_dir(&static_dir, base, first, cx) {
            let mut host = host_reference(r, &static_dir, first, cx);
            // The static directory as the repository seen from another place: the pattern
            // below each directory it may name.
            let prefix = static_prefix(first);
            let shown = format!("{first}*");
            let mut readings = self.readings(base, located, prefix, &shown, true, cx);
            // A directory computed directly below a place the repository may be mounted at
            // (`/app/${dir}/x.json`): mounted there, the path would be `*/x.json` from the
            // root, a glob over the whole repository, and is not read that way. An
            // assumption, so the read keeps a record that `--strict` counts
            // ([`crate::output::MOUNT_READ_NOTE`]).
            if self.computed_below_mount(r, first, parts.len(), parts.get(1), cx)
                && host.uncertainty.is_none()
            {
                host.uncertainty = Some(mount_record(r));
            }
            // Mounted at the static directory itself: for a name that is computed as a
            // whole (`'/app/' + name`). A pattern with more to it (`'/srv/x/' + v +
            // '/node_modules/pkg/index.js'`) would be a glob over the whole repository.
            if first.ends_with('/') && parts.len() == 2 && matches!(parts[1], PatPart::Wild { .. })
            {
                readings.extend(self.root_reading(prefix, &shown, cx));
            }
            return with_readings(host, r, &readings, |reading| {
                let mut parts = written.to_vec();
                parts[0] = Part::Lit(reading.below(&first[prefix.len()..]).into());
                self.resolve_pattern_in(r, &parts, &cx.options.base, cx)
            });
        }
        if cx.is_host_path(&static_dir) {
            return above_base_pattern(r, base, first, cx);
        }
        if self.is_outside(cx, &static_dir) {
            return Resolution::target(Target::Unresolved);
        }
        // nft's `validWildcard`: `<dir>/*` for the own dir, cwd, a marker dir or an
        // ancestor.
        let rest_is_wild = match first.rfind('/') {
            Some(i) => i + 1 == first.len(),
            None => first.is_empty(),
        } && parts.len() == 2
            && matches!(parts[1], PatPart::Wild { .. });
        if rest_is_wild
            && let Some(why) = self.invalid_dir(&static_dir, cx, r.kind == RefKind::FsReadDir)
        {
            return too_broad(format!("{} is {why}", display_rel(&static_dir, cx)));
        }
        match cx.fs.stat(&static_dir).map(|s| s.kind) {
            Some(FileKind::Dir) => {}
            // A missing directory (or a file in its place) matches nothing; for probes this is
            // normal. The extractor's uncertainty, if any, is kept.
            _ => return Resolution::target(Target::Unresolved),
        }
        let Some(g) = self.glob(&parts, base, cx, self.keep) else {
            return too_broad("starts with a wildcard or is anchored at /".into());
        };
        if g.matches.is_empty() && !g.truncated {
            return Resolution::target(Target::Unresolved);
        }
        let u = glob_uncertainty(r, &g, cx.options.max_glob);
        Resolution::target(g.into_target(cx)).with_uncertainty(u)
    }

    /// A module reference with a partially known specifier, already anchored by
    /// the language (`parts` relative to `base`; a package prefix replaced
    /// by the package directory): a glob over files, like webpack context
    /// modules and nft's wildcard requires.
    ///
    /// Without a trailing wildcard each of `extensions` (with the dot) is also
    /// tried (nft appends `?(.js|.json|.node)`). Matches are deduplicated,
    /// the referencing file itself is dropped, and the total is capped by
    /// `--max-glob`. No match is [`UncertaintyKind::UnresolvedSpecifier`].
    /// Manifests are left to the caller.
    ///
    /// `None` when the pattern is refused (leading wildcard or anchored at
    /// `/`), so callers can tell a refusal, which consulted nothing, from
    /// an expansion; report it with [`refused_module_pattern`].
    pub fn resolve_module_pattern(
        &self,
        r: &RawRef<'_>,
        parts: &[PatPart],
        base: &Path,
        extensions: &[&str],
        keep: fn(&Path) -> bool,
        cx: &ResolveCx<'_>,
    ) -> Option<Resolution> {
        let mut variants = vec![parts.to_vec()];
        if matches!(parts.last(), Some(PatPart::Lit(_))) {
            for e in extensions {
                let mut v = parts.to_vec();
                v.push(PatPart::Lit((*e).to_string()));
                variants.push(v);
            }
        }
        let mut all: Option<GlobOut> = None;
        for v in &variants {
            let g = self.glob(v, base, cx, keep)?;
            match &mut all {
                None => all = Some(g),
                Some(acc) => {
                    acc.truncated |= g.truncated;
                    acc.matches.extend(g.matches);
                }
            }
        }
        let mut g = all.unwrap_or_default();
        g.matches.sort_unstable_by(|a, b| {
            use crate::os::OsStrExt;
            a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes())
        });
        g.matches.dedup();
        g.matches.retain(|m| m.as_path() != cx.from_path);
        if g.matches.len() > cx.options.max_glob {
            g.matches.truncate(cx.options.max_glob);
            g.truncated = true;
        }
        if variants.len() > 1 {
            g.pattern.push_str("?(ext)");
        }
        Some(if g.matches.is_empty() && !g.truncated {
            let what = format!(
                "no files match {} from {}",
                g.pattern,
                cx.from_path.display()
            );
            Resolution::unresolved(match &r.uncertainty {
                // What the pattern names depends on more than the files below this base
                // (the working directory of whoever runs the file, the environment): that
                // nothing matches from here says nothing about where it runs, so the
                // extractor's record stays, with its kind.
                Some(u) if u.kind != UncertaintyKind::PartialSpecifier => {
                    Uncertainty::new(u.kind, u.span, format!("{}; {what}", u.detail))
                        .with_origin(u.origin)
                }
                _ => Uncertainty::new(UncertaintyKind::UnresolvedSpecifier, r.span, what),
            })
        } else {
            let u = glob_uncertainty(r, &g, cx.options.max_glob);
            Resolution::target(g.into_target(cx)).with_uncertainty(u)
        })
    }

    /// The anchored-path asset policy shared by every language: an `Asset`
    /// reference whose path value was built from a language anchor
    /// (`__dirname`, `__file__`, `CARGO_MANIFEST_DIR`, `runtime.Caller`)
    /// and that never reached a tracked API.
    ///
    /// - An existing file is a leaf with no uncertainty (the anchored literal
    ///   is nft's documented behavior).
    /// - An existing directory expands with nft's `validWildcard` rules
    ///   ([`AssetRules::resolve_dir`]).
    /// - A pattern globs from its static prefix with a `PartialSpecifier`
    ///   record ([`AssetRules::resolve_pattern_in`]).
    /// - A missing path is a quiet probe with [`MissingAsset::Quiet`] (JS,
    ///   matching nft), or an [`UncertaintyKind::UnresolvedSpecifier`] record
    ///   with [`MissingAsset::Report`] (the file may be created at runtime, or
    ///   be missing from the deployment).
    ///
    /// `base` is the anchor directory the language resolved.
    pub fn resolve_anchored_asset(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        base: &Path,
        missing: MissingAsset,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let mut res = match spec {
            Specifier::Literal(s) => self.resolve_path_in(r, s, base, cx),
            Specifier::Pattern(parts) => self.resolve_pattern_in(r, parts, base, cx),
            Specifier::Alternatives(alts) => {
                return resolve_alternatives(r, spec, alts, |a| {
                    self.resolve_anchored_asset(r, a, base, missing, cx)
                });
            }
            Specifier::Unknown => return computed_specifier(r, spec),
        };
        let unresolved = res.targets.iter().all(|t| matches!(t, Target::Unresolved));
        if missing == MissingAsset::Report && unresolved && res.uncertainty.is_none() {
            let shown = match spec {
                Specifier::Literal(s) => display_rel(&normalize(&base.join(&**s)), cx),
                _ => format!("{}/{spec}", display_rel(base, cx)),
            };
            res.uncertainty = Some(r.uncertainty.clone().unwrap_or_else(|| {
                Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    r.span,
                    format!(
                        "anchored path {shown} does not exist: created at runtime, or missing \
                         from the tree"
                    ),
                )
            }));
        }
        res
    }

    /// What a path that leaves the repository by the way it is written
    /// (`spelled`, relative to `base`: an absolute path, a climb above the
    /// root) may name inside it: the repository seen from another place
    /// ([`ResolveCx::seen_from_elsewhere`]), each reading with the label of
    /// its edges. `shown` is the reference as a reader knows it, and
    /// `below` says that the readings are directories whose files the reference
    /// names.
    ///
    /// None for a file of an installed package ([`AssetRules::boundary`]): it
    /// was written without knowing this repository, so an absolute path in
    /// it is no place this repository is mounted at (and a file inside a
    /// package tree never emits assets outside its boundary anyway).
    ///
    /// `located`: `base` is a directory the language knows (the file's own, a
    /// script's working directory). When it is only the directory a process
    /// is assumed to run in ([`Base::Cwd`], resolved against the repository
    /// root until a test's own working directory is known), a relative path
    /// has no reading: how many levels it climbs above the root is counted
    /// from a directory that is a guess (`../../packages/x/dist/a.svg`,
    /// written for a working directory two levels down, would name
    /// `dist/a.svg`). An absolute path is read the same from every
    /// directory.
    pub fn readings(
        &self,
        base: &Path,
        located: bool,
        spelled: &str,
        shown: &str,
        below: bool,
        cx: &ResolveCx<'_>,
    ) -> Vec<Reading> {
        if !cx.fs.confined()
            || (!located && !spelled.starts_with('/'))
            || self
                .boundary
                .contains_from(cx.from_path, cx.fs.local_offset(cx.from_path))
        {
            return Vec::new();
        }
        cx.seen_from_elsewhere(base, spelled)
            .into_iter()
            .map(|(_, e)| {
                let what = if below {
                    format!("the files below {}", e.rel)
                } else {
                    e.rel.clone()
                };
                Reading {
                    label: format!("{ELSEWHERE_PREFIX}{}", e.sentence(shown, &what)),
                    rel: e.rel,
                }
            })
            .collect()
    }

    /// The reading of `spelled`, an absolute path the code writes for a
    /// directory, in which the path itself is the place the repository is
    /// mounted at ([`turborepo_trace_core::fs::mounted_at`]): the
    /// repository root, [`Reading::rel`] empty. For the static directory of
    /// a pattern that is a directory and a name computed as a
    /// whole (`'/app/' + name`, `path.join('/app', name)`) and for a directory
    /// that is listed (`readdirSync('/app')`): a read of the root with a
    /// computed name, which gets what such a read gets inside the
    /// repository (the record of a directory that is not expanded, which
    /// the names of the code bound in JS/TS and Go). Not for a pattern with
    /// more to it: its reading at the deepest directory it spells would be a
    /// glob over the whole repository (on one of the measured repositories
    /// it tied a file to a root task's outputs and through them to every
    /// file of the root package). `shown`: the path or pattern in the
    /// sentence. `None` under the conditions of [`AssetRules::readings`].
    pub fn root_reading(&self, spelled: &str, shown: &str, cx: &ResolveCx<'_>) -> Option<Reading> {
        if !cx.fs.confined()
            || self
                .boundary
                .contains_from(cx.from_path, cx.fs.local_offset(cx.from_path))
        {
            return None;
        }
        let e = crate::fs::mounted_at(spelled)?;
        Some(Reading {
            label: format!(
                "{ELSEWHERE_PREFIX}{}",
                e.sentence(shown, "the files of the repository")
            ),
            rel: e.rel,
        })
    }

    /// Whether the pattern of the read `r` (its first literal `first`, `len`
    /// parts, `second` the part after the first) has a computed directory
    /// directly below an absolute directory the repository may be mounted
    /// at (`` `/app/${dir}/x.json` ``): the reading "mounted there"
    /// ([`AssetRules::root_reading`]) is not taken for it, which is an
    /// assumption with a record of its own ([`mount_record`]).
    fn computed_below_mount(
        &self,
        r: &RawRef<'_>,
        first: &str,
        len: usize,
        second: Option<&PatPart>,
        cx: &ResolveCx<'_>,
    ) -> bool {
        is_probe_kind(r.kind)
            && first.ends_with('/')
            && len > 2
            && matches!(second, Some(PatPart::Wild { .. }))
            && self.root_reading(static_prefix(first), first, cx).is_some()
    }

    /// The result for a literal or partially known path relative to `base` that
    /// is a host path (for a pattern: whose static directory, the literal
    /// prefix up to its last `/`, is one): [`host_reference`], with what
    /// the path may name when the repository is seen from another place
    /// ([`AssetRules::readings`]). [`above_base_pattern`] for a
    /// pattern that climbs above the base. `None` otherwise, and for specifiers
    /// without a literal prefix.
    ///
    /// `inside` resolves one reading the way the language resolves a path
    /// inside the repository: it is handed the reference with the reading
    /// as its specifier, relative to the repository root
    /// ([`Base::ProjectRoot`]). `located`: `base` is a directory the
    /// language knows (see [`AssetRules::readings`]).
    pub fn host_reference_at(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        base: &Path,
        located: bool,
        cx: &ResolveCx<'_>,
        mut inside: impl FnMut(&RawRef<'_>) -> Resolution,
    ) -> Option<Resolution> {
        if !cx.fs.confined() {
            return None;
        }
        let at_root = |spec: Specifier<'static>| RawRef {
            base: Base::ProjectRoot,
            spec,
            ..r.clone()
        };
        match spec {
            Specifier::Literal(s) => {
                let p = normalize(&base.join(&**s));
                // Outside, or inside at this place only: written as an absolute path or as a
                // climb above the base.
                if !cx.is_host_path(&p) && !cx.names_host_path(base, s) {
                    return None;
                }
                let host = host_reference(r, &p, s, cx);
                let mut readings = self.readings(base, located, s, s, false, cx);
                if r.kind == RefKind::FsReadDir {
                    readings.extend(self.root_reading(s, s, cx));
                }
                Some(with_readings(host, r, &readings, |reading| {
                    inside(&at_root(Specifier::Literal(
                        format!("./{}", reading.rel).into(),
                    )))
                }))
            }
            Specifier::Pattern(parts) => match parts.first() {
                Some(Part::Lit(first)) => {
                    let dir = normalize(&base.join(&first[..=first.rfind('/')?]));
                    if !host_pattern_dir(&dir, base, first, cx) {
                        return cx
                            .is_host_path(&dir)
                            .then(|| above_base_pattern(r, base, first, cx));
                    }
                    let mut host = host_reference(r, &dir, first, cx);
                    let prefix = static_prefix(first);
                    let shown = format!("{first}*");
                    let mut readings = self.readings(base, located, prefix, &shown, true, cx);
                    // (A directory computed directly below a place the repository may be
                    // mounted at: see [`AssetRules::computed_below_mount`].)
                    if is_probe_kind(r.kind)
                        && first.ends_with('/')
                        && parts.len() > 2
                        && matches!(parts[1], Part::Wild { .. })
                        && host.uncertainty.is_none()
                        && self.root_reading(prefix, &shown, cx).is_some()
                    {
                        host.uncertainty = Some(mount_record(r));
                    }
                    // For a read, not a module load: a load by an absolute path keeps
                    // the record of a module that may be the machine's.
                    if is_probe_kind(r.kind)
                        && first.ends_with('/')
                        && parts.len() == 2
                        && matches!(parts[1], Part::Wild { .. })
                    {
                        readings.extend(self.root_reading(prefix, &shown, cx));
                    }
                    Some(with_readings(host, r, &readings, |reading| {
                        let mut parts: Vec<Part<'static>> =
                            parts.iter().cloned().map(Part::into_owned).collect();
                        parts[0] = Part::Lit(
                            format!("./{}", reading.below(&first[prefix.len()..])).into(),
                        );
                        inside(&at_root(Specifier::Pattern(parts)))
                    }))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// An asset-directory edge for package special cases: every asset below
    /// `dir`, with a [`UncertaintyKind::PatternTooBroad`] record when
    /// truncated. `label` is prefixed with [`SPECIAL_CASE_PREFIX`].
    pub fn dir_edge(
        &self,
        dir: &Path,
        cx: &ResolveCx<'_>,
        label: impl std::fmt::Display,
    ) -> ExtraEdge {
        let g = self.dir_glob(dir, cx);
        let truncated = g.truncated;
        let mut e = ExtraEdge::new(RefKind::Asset, g.into_target(cx), special_spec(label));
        if truncated {
            e = e.with_uncertainty(Uncertainty::new(
                UncertaintyKind::PatternTooBroad,
                Span::DUMMY,
                format!(
                    "{}: more than {} files",
                    display_rel(dir, cx),
                    cx.options.max_glob
                ),
            ));
        }
        e
    }
}

/// The record of a read with a computed directory directly below a place the
/// repository may be mounted at
/// ([`turborepo_trace_core::output::MOUNT_READ_NOTE`]): ignored by default,
/// open under `--strict`.
fn mount_record(r: &RawRef<'_>) -> Uncertainty {
    Uncertainty::new(
        UncertaintyKind::GeneratedFile,
        r.span,
        format!("{} {}", r.spec, crate::output::MOUNT_READ_NOTE),
    )
    .with_origin(Origin::Data)
}

/// The result for a reference whose path, or whose pattern's static directory,
/// is the host path `path` ([`ResolveCx::is_host_path`]; see the module docs).
/// Nothing is looked up.
///
/// A read, listing, glob or asset path is an external reference: an unresolved
/// edge with the reference's specifier and no uncertainty of its own (the
/// extractor's, if any, stays); below a temporary directory that the source
/// writes out it keeps a `generated_file`
/// record ([`turborepo_trace_core::output::TEMP_READ_NOTE`]: ignored by
/// default). A module load keeps a record, because code runs that the graph
/// does not have: open when the module comes from a temporary directory
/// (generated at run time, from files the site does not name), and otherwise
/// the record of a module that is not installed.
///
/// `spelled` is the path as the source writes it (for a pattern its first
/// literal; empty when only the path is known). What
/// kind of host path it is must not depend on where the checkout is, so it is
/// decided on what the code says: a temporary directory only for an absolute
/// spelling ([`turborepo_trace_core::fs::TEMP_DIRS`]), and `.git` on the
/// absolute spelling or, for a path reached by relative steps, on the part
/// below the directory where it leaves the way to the repository
/// ([`turborepo_trace_core::DirCache::local`]). A checkout below `/tmp` or
/// below a directory called `.git` changes neither.
pub fn host_reference(
    r: &RawRef<'_>,
    path: &Path,
    spelled: &str,
    cx: &ResolveCx<'_>,
) -> Resolution {
    let absolute = spelled.starts_with('/');
    if is_probe_kind(r.kind) {
        // A read below a temporary directory that the code writes out: what is there
        // was made at run time. An assumption (a build step may copy a file of
        // the repository there), so it keeps a record that `--strict` counts.
        if absolute && is_temp_path(path) {
            return Resolution::unresolved(
                Uncertainty::new(
                    UncertaintyKind::GeneratedFile,
                    r.span,
                    format!("{} {}", r.spec, crate::output::TEMP_READ_NOTE),
                )
                .with_origin(Origin::Data),
            );
        }
        return Resolution::target(Target::Unresolved);
    }
    let named = if absolute { path } else { cx.fs.local(path) };
    let metadata =
        cx.fs.is_repository_metadata(path) || named.components().any(|c| c.as_os_str() == ".git");
    Resolution::unresolved(if metadata {
        Uncertainty::new(
            UncertaintyKind::UnresolvedSpecifier,
            r.span,
            format!(
                "{} is inside .git: the repository's metadata, not part of its tree",
                r.spec
            ),
        )
    } else if absolute && is_temp_path(path) {
        Uncertainty::new(
            UncertaintyKind::ComputedSpecifier,
            r.span,
            format!(
                "{} is loaded from a temporary directory: a module generated at run time",
                r.spec
            ),
        )
        .with_origin(Origin::Data)
    } else {
        // What the text says is how the path is written, not where it lands: at the one
        // place where an absolute path points into the checkout it is no path "outside
        // the repository" (a shard shows it from the root on there).
        let what = if absolute {
            "is an absolute path"
        } else if spelled.is_empty() {
            "is outside the repository"
        } else {
            "leaves the repository"
        };
        Uncertainty::new(
            UncertaintyKind::UnresolvedSpecifier,
            r.span,
            format!("{} {what}: not looked up on the build machine", r.spec),
        )
    })
}

/// The literal prefix of a pattern's first literal up to its last `/`: what
/// names the pattern's static directory.
fn static_prefix(first: &str) -> &str {
    first.rfind('/').map_or("", |i| &first[..=i])
}

/// Whether the static directory `dir` of a pattern relative to `base` whose
/// first literal is `first` is a host directory. One of:
///
/// - a host path ([`ResolveCx::is_host_path`]) that is not above the base (a
///   sibling of the checkout, `/usr/lib`, the repository's `.git`);
/// - a directory above the base that the source spells as an absolute path
///   below `/` (`'/tmp/' + name`: the checkout merely happens to live below
///   `/tmp` on this machine);
/// - a directory inside the base that the source reaches by an absolute path or
///   by a climb above the base ([`ResolveCx::names_host_path`]): it is inside
///   only at this place.
///
/// A relative climb that ends above the base (`'../../' + name`) and `'/' +
/// name` can lead back into the repository: no host directory
/// ([`above_base_pattern`]).
pub fn host_pattern_dir(dir: &Path, base: &Path, first: &str, cx: &ResolveCx<'_>) -> bool {
    if !cx.is_host_path(dir) {
        return cx.names_host_path(base, static_prefix(first));
    }
    !cx.fs.is_above_base(dir) || (first.starts_with('/') && dir != Path::new("/"))
}

/// The result for a pattern relative to `base` whose static directory lies
/// above the base without being a host directory ([`host_pattern_dir`]): it
/// climbs there by relative steps (`'../../' + name`) or starts at `/`. It can
/// lead back into the repository, so it stays an open record; it is not
/// expanded, because what lies next to the checkout (a sibling checkout, the
/// rest of a home directory) depends on the machine.
///
/// The text says how far the source climbs, counted on what it writes: the
/// directory the climb ends at on this machine is `/` sooner below a short path
/// than below a long one.
pub fn above_base_pattern(
    r: &RawRef<'_>,
    base: &Path,
    first: &str,
    cx: &ResolveCx<'_>,
) -> Resolution {
    let whence = if first.starts_with('/') {
        "starts at the filesystem root".to_owned()
    } else {
        // Levels above the base at the end of the literal prefix.
        let depth = base
            .strip_prefix(&cx.options.base)
            .map_or(0, |rel| rel.components().count() as isize);
        let level = static_prefix(first)
            .split('/')
            .fold(depth, |level, c| match c {
                "" | "." => level,
                ".." => level - 1,
                _ => level + 1,
            });
        format!(
            "climbs above the repository root (to {})",
            vec![".."; level.unsigned_abs().max(1)].join("/")
        )
    };
    Resolution::unresolved(Uncertainty::new(
        UncertaintyKind::PatternTooBroad,
        r.span,
        format!("{}: {whence}; not expanded", r.spec),
    ))
}

/// Whether the directory a reference is relative to is one the language knows:
/// not the working directory a process is assumed to run in
/// ([`AssetRules::readings`]).
pub fn located(r: &RawRef<'_>) -> bool {
    r.base != Base::Cwd
}

/// One reading of a path that leaves the repository by the way it is written
/// ([`AssetRules::readings`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The file or directory it may name, as a path from the repository root.
    /// Empty: the root itself ([`AssetRules::root_reading`]).
    pub rel: String,
    /// The specifier of the edges to what is found there: [`ELSEWHERE_PREFIX`]
    /// and the reading in one sentence.
    pub label: String,
}

impl Reading {
    /// `rest` (what a pattern's first literal says after its static directory)
    /// below the directory this reading names, as a path from the
    /// repository root.
    pub fn below(&self, rest: &str) -> String {
        if self.rel.is_empty() {
            format!("./{rest}")
        } else {
            format!("{}/{rest}", self.rel)
        }
    }
}

/// `host`, the result for a path that leaves the repository by the way it is
/// written ([`host_reference`]), with what its `readings` name inside the
/// repository; `inside` resolves one reading like a path inside the repository.
///
/// What a reading finds becomes edges of the reference's kind next to the
/// reference's own unresolved one, labelled with the reading
/// ([`Reading::label`]); a record of its own (a truncated directory, a glob)
/// stays on them. A reading that finds nothing leaves the probes of its lookup,
/// like every path inside the repository that does not exist: a file
/// appearing there changes what the reference names. Its "not found" is no
/// record (the reference keeps the record of a host path, if it is a module
/// load), but a record that says more is kept (a directory that is not
/// expanded, a path that exists with another casing).
///
/// An over-approximation, not an assumption: the same under `--strict`.
pub fn with_readings(
    mut host: Resolution,
    r: &RawRef<'_>,
    readings: &[Reading],
    mut inside: impl FnMut(&Reading) -> Resolution,
) -> Resolution {
    for reading in readings {
        let res = inside(reading);
        if res.none {
            continue;
        }
        let mut record = res.uncertainty;
        let mut found = false;
        for t in res.targets {
            if matches!(t, Target::Unresolved) {
                continue;
            }
            let mut e = ExtraEdge::new(r.kind, t, reading.label.as_str());
            e.uncertainty = record.take();
            host.extra.push(e);
            found = true;
        }
        if found {
            host.manifests.extend(res.manifests);
            host.extra.extend(res.extra);
        } else if let Some(u) = record
            && u.kind != UncertaintyKind::UnresolvedSpecifier
        {
            host.uncertainty = merge_branch_records(host.uncertainty.take(), Some(u));
        }
    }
    dedup_ids(&mut host.manifests);
    host
}

/// The result for a module pattern [`AssetRules::resolve_module_pattern`]
/// refused.
pub fn refused_module_pattern(r: &RawRef<'_>) -> Resolution {
    Resolution::unresolved(Uncertainty::new(
        UncertaintyKind::PatternTooBroad,
        r.span,
        format!(
            "pattern {} starts with a wildcard or is anchored at /; not expanded",
            r.spec
        ),
    ))
}

/// Resolves each alternative with `each` and merges the results: targets
/// deduplicated (an unresolved branch is dropped when another resolved),
/// manifests and extra edges combined. Unless every branch was a quiet fs
/// probe, the result carries the branch record that says
/// the most ([`merge_branch_records`]: open before covered before fully
/// expanded), else the reference's own, else
/// [`UncertaintyKind::ConditionalResolution`] naming `spec`.
pub fn resolve_alternatives(
    r: &RawRef<'_>,
    spec: &Specifier<'_>,
    alts: &[Specifier<'_>],
    each: impl FnMut(&Specifier<'_>) -> Resolution,
) -> Resolution {
    alternatives(r, spec, alts, each, false)
}

/// [`resolve_alternatives`] for a language whose alternatives are the values an
/// expression can have (a table of module names indexed by a key, a value
/// checked against a list): when every alternative resolved and none left a
/// record, each one is a target and nothing about the reference is unknown, so
/// it gets no record of its own. (One that did not resolve keeps
/// [`UncertaintyKind::ConditionalResolution`]: what it names is not covered.)
pub fn resolve_value_alternatives(
    r: &RawRef<'_>,
    spec: &Specifier<'_>,
    alts: &[Specifier<'_>],
    each: impl FnMut(&Specifier<'_>) -> Resolution,
) -> Resolution {
    alternatives(r, spec, alts, each, true)
}

fn alternatives(
    r: &RawRef<'_>,
    spec: &Specifier<'_>,
    alts: &[Specifier<'_>],
    mut each: impl FnMut(&Specifier<'_>) -> Resolution,
    values: bool,
) -> Resolution {
    let mut out = Resolution::default();
    let mut quiet = true;
    let mut all_resolved = true;
    for a in alts {
        let res = each(a);
        all_resolved &= res.targets.iter().any(|t| !matches!(t, Target::Unresolved));
        out.targets.extend(
            res.targets
                .into_iter()
                .filter(|t| !matches!(t, Target::Unresolved)),
        );
        out.manifests.extend(res.manifests);
        out.extra.extend(res.extra);
        if res.uncertainty.is_some() {
            quiet = false;
        }
        out.uncertainty = merge_branch_records(out.uncertainty.take(), res.uncertainty);
    }
    dedup_ids(&mut out.manifests);
    let mut seen = std::collections::HashSet::new();
    out.targets.retain(|t| seen.insert(t.clone()));
    if out.targets.is_empty() {
        out.targets.push(Target::Unresolved);
    }
    // Fs probes of missing branches stay quiet; module alternatives are conditional
    // by nature.
    if !(quiet && is_probe_kind(r.kind)) && out.uncertainty.is_none() {
        out.uncertainty = match &r.uncertainty {
            Some(u) => Some(u.clone()),
            // The values of an expression, each resolved: nothing is left unknown.
            None if values && quiet && all_resolved => None,
            None => Some(Uncertainty::new(
                UncertaintyKind::ConditionalResolution,
                r.span,
                format!("one of: {spec}"),
            )),
        };
    }
    out
}

/// How much a branch's record says about what the reference can reach: an open
/// record (2: nothing bounds the branch), a covered one (1: a scope bounds it),
/// or one a directory membership stands for (0:
/// [`UncertaintyKind::PartialSpecifier`], a glob that was expanded completely).
fn record_rank(u: &Uncertainty) -> u8 {
    match (u.is_covered(), u.kind) {
        (true, _) => 1,
        (false, UncertaintyKind::PartialSpecifier) => 0,
        (false, _) => 2,
    }
}

/// The record of a reference after one more branch. A reference carries one
/// record, so the one that says the most wins ([`record_rank`]; the earlier
/// branch on a tie): an open branch makes the reference open, and a branch
/// truncated at `--max-glob` keeps its scope next to a branch that was expanded
/// completely. Without this, the files beyond the cap of a later branch would
/// be neither edges nor inside a scope. Two covered branches with
/// different scopes get one scope that contains both ([`merge_scopes`]).
fn merge_branch_records(
    acc: Option<Uncertainty>,
    next: Option<Uncertainty>,
) -> Option<Uncertainty> {
    let (a, b) = match (acc, next) {
        (Some(a), Some(b)) => (a, b),
        (a, b) => return a.or(b),
    };
    match record_rank(&a).cmp(&record_rank(&b)) {
        std::cmp::Ordering::Less => Some(b),
        std::cmp::Ordering::Greater => Some(a),
        std::cmp::Ordering::Equal => match (&a.scope, &b.scope) {
            (Some(x), Some(y)) if x != y => {
                let scope = merge_scopes(x, y);
                let detail = format!("{} | {}", a.detail, b.detail);
                Some(Uncertainty::new(a.kind, a.span, detail).with_scope(scope))
            }
            _ => Some(a),
        },
    }
}

/// One scope that contains `a` and `b`: their common ancestor directory, with
/// each pattern prefixed by its directory's path below it
/// (`{a/<pattern>,b/<pattern>}`).
fn merge_scopes(a: &UncertaintyScope, b: &UncertaintyScope) -> UncertaintyScope {
    let mut dir = PathBuf::new();
    for (x, y) in a.dir.components().zip(b.dir.components()) {
        if x != y {
            break;
        }
        dir.push(x);
    }
    let below = |s: &UncertaintyScope| -> String {
        let rel = s.dir.strip_prefix(&dir).unwrap_or(&s.dir);
        let rel = rel.to_string_lossy();
        // A directory name with glob syntax can't be written as a literal prefix: cover
        // everything below the common ancestor instead.
        if rel.contains(['*', '?', '[', ']', '{', '}', ',']) {
            "**".to_owned()
        } else if rel.is_empty() {
            s.pattern.to_string()
        } else {
            format!("{rel}/{}", s.pattern)
        }
    };
    let (pa, pb) = (below(a), below(b));
    let pattern = if pa == "**" || pb == "**" {
        "**".to_owned()
    } else if pa == pb {
        pa
    } else {
        format!("{{{pa},{pb}}}")
    };
    UncertaintyScope {
        dir,
        pattern: pattern.into(),
    }
}

/// For a reference the extractor already explained (a loader argument that is
/// cwd- or environment-dependent): when nothing resolved, its record replaces
/// the resolver's generic one, with "not found" appended. Other results are
/// returned unchanged.
pub fn explain_missing(r: &RawRef<'_>, mut res: Resolution) -> Resolution {
    if let Some(u) = &r.uncertainty
        && !matches!(r.spec, Specifier::Unknown)
        && res.targets.iter().all(|t| matches!(t, Target::Unresolved))
    {
        res.uncertainty = Some(Uncertainty::new(
            u.kind,
            u.span,
            format!("{}; not found", u.detail),
        ));
    }
    res
}

/// The unresolved result for an unknown specifier `spec` of `r` (the whole
/// specifier or an alternative): the reference's own uncertainty, else
/// [`UncertaintyKind::ComputedSpecifier`].
pub fn computed_specifier(r: &RawRef<'_>, spec: &Specifier<'_>) -> Resolution {
    Resolution::unresolved(r.uncertainty.clone().unwrap_or_else(|| {
        Uncertainty::new(
            UncertaintyKind::ComputedSpecifier,
            r.span,
            format!("computed specifier {spec}"),
        )
    }))
}

/// The unresolved result for a [`turborepo_trace_core::Base::Language`] anchor
/// the resolver does not handle.
pub fn unsupported_base(r: &RawRef<'_>) -> Resolution {
    Resolution::unresolved(Uncertainty::new(
        UncertaintyKind::UnsupportedSyntax,
        r.span,
        format!("unsupported base {:?}", r.base),
    ))
}

/// Kinds for which a missing path is normal (fs probes such as `existsSync`).
pub fn is_probe_kind(kind: RefKind) -> bool {
    matches!(kind, RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset)
}

/// Whether `r` observes names only ([`RefFlags::NAMES`]): which paths exist,
/// never what the files contain. Truncating its expansion at `--max-glob` loses
/// nothing a change query needs: the edges past the limit would only carry
/// edits of file contents, which it cannot see, and the directory memberships
/// recorded for the expansion cover every path that can appear or disappear.
pub fn names_only(r: &RawRef<'_>) -> bool {
    r.flags.contains(RefFlags::NAMES)
}

/// The record of a names-only reference whose expansion was truncated: what a
/// complete expansion gets (the extractor's own record, else
/// `PartialSpecifier`), with the truncation noted. A directory membership
/// stands for it.
fn names_only_truncated(r: &RawRef<'_>, what: String) -> Uncertainty {
    let what =
        format!("{what}; names only, so the directory membership covers additions and deletions");
    match &r.uncertainty {
        Some(u) => Uncertainty::new(u.kind, u.span, format!("{}; {what}", u.detail)),
        None => Uncertainty::new(UncertaintyKind::PartialSpecifier, r.span, what),
    }
}

/// The uncertainty for a successful glob: the extractor's own record (it knows
/// why the specifier is partial) with the glob result appended, else
/// `PartialSpecifier`; `PatternTooBroad` when truncated.
///
/// A truncated glob of a non-module reference ([`is_probe_kind`]) is covered.
/// Its scope is the pattern itself below its static directory ([`glob_scope`]):
/// the files the reference can read are the files the pattern matches, the ones
/// beyond the limit included. A names-only reference ([`names_only`]) needs no
/// scope at all. A truncated module glob stays open: the modules beyond the cap
/// would be executed, and their own imports have no edges.
pub fn glob_uncertainty(r: &RawRef<'_>, g: &GlobOut, max: usize) -> Uncertainty {
    if g.truncated {
        let what = format!("{}: more than {max} matches; truncated", g.pattern);
        if names_only(r) && is_probe_kind(r.kind) {
            return names_only_truncated(r, what);
        }
        let u = Uncertainty::new(UncertaintyKind::PatternTooBroad, r.span, what);
        return match glob_scope(&g.pattern) {
            Some(scope) if is_probe_kind(r.kind) => u.with_scope(scope),
            _ => u,
        };
    }
    let what = format!("{}: {} match(es)", g.pattern, g.matches.len());
    match &r.uncertainty {
        Some(u) => Uncertainty::new(u.kind, u.span, format!("{}; {what}", u.detail)),
        None => Uncertainty::new(UncertaintyKind::PartialSpecifier, r.span, what),
    }
}

/// The scope of an absolute display pattern whose `*`s may cross `/`
/// (`/a/b/*.json`): its static directory (the part before the first wildcard,
/// up to its last `/`: `/a/b`) and the store-glob forms of the rest
/// ([`membership_patterns`]: `{*.json,*/**/*.json}`). `None` for a relative
/// pattern or one that starts with a wildcard at the root.
pub fn glob_scope(pattern: &str) -> Option<UncertaintyScope> {
    let wild = pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len());
    let cut = pattern[..wild].rfind('/')?;
    let dir = &pattern[..cut];
    if !pattern.starts_with('/') || dir.is_empty() {
        return None;
    }
    // A comma would split the alternatives: any one character stands for it.
    let forms: Vec<String> = membership_patterns(&pattern[cut + 1..])
        .into_iter()
        .map(|p| p.replace(',', "?"))
        .collect();
    let pattern = match forms.as_slice() {
        [one] => one.clone(),
        // `*` in one segment or across several: everything below the directory.
        [single, _] if single == "*" => "**".to_owned(),
        _ => format!("{{{}}}", forms.join(",")),
    };
    Some(UncertaintyScope {
        dir: PathBuf::from(dir),
        pattern: pattern.into(),
    })
}

/// The store-glob forms of a pattern `rest` (relative to its static directory)
/// whose `*`s may cross `/`:
///
/// - `rest` itself, which covers every match where no wildcard crosses `/`;
/// - `P*/**/*S`, with `P` the text before the first `*` (never containing `/`)
///   and `S` the text after the last one: every path that starts with `P`, ends
///   with `S` and has a `/` in between, which covers every match where some
///   wildcard crosses `/` (literal text between wildcards falls inside them, an
///   over-approximation).
///
/// Glob metacharacters in the literal text become `?` (any one character but
/// `/`), which over-approximates without relying on escaping rules.
pub fn membership_patterns(rest: &str) -> Vec<String> {
    let escape = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                '?' | '[' | ']' | '{' | '}' => '?',
                c => c,
            })
            .collect()
    };
    let (Some(first), Some(last)) = (rest.find('*'), rest.rfind('*')) else {
        return vec![escape(rest)];
    };
    let single = rest.split('*').map(escape).collect::<Vec<_>>().join("*");
    let crossing = format!(
        "{}*/**/*{}",
        escape(&rest[..first]),
        escape(&rest[last + 1..])
    );
    vec![single, crossing]
}

/// `path` for messages: relative to the trace base when inside it (`.` for the
/// base itself) or above it (`../..`), otherwise absolute. Keeps uncertainty
/// details independent of where the project lives.
pub fn display_rel(path: &Path, cx: &ResolveCx<'_>) -> String {
    let base = cx.options.base.as_path();
    match path.strip_prefix(base) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_owned(),
        Ok(rel) => rel.display().to_string(),
        // An ancestor of the base: `..` per level.
        Err(_) if base.starts_with(path) => {
            let ups = base.components().count() - path.components().count();
            vec![".."; ups].join("/")
        }
        Err(_) => path.display().to_string(),
    }
}

/// Splits a path pattern with `*` wildcards into glob parts.
pub fn pattern_parts(pat: &str) -> Vec<PatPart> {
    let mut out = Vec::new();
    for (i, lit) in pat.split('*').enumerate() {
        if i > 0 {
            out.push(PatPart::Wild { reason: "*".into() });
        }
        if !lit.is_empty() {
            out.push(PatPart::Lit(lit.to_string()));
        }
    }
    out
}

/// An [`ExtraEdge`] spec for a package special case: `label` with
/// [`SPECIAL_CASE_PREFIX`], so `--format summary` lists the hit.
pub fn special_spec(label: impl std::fmt::Display) -> Box<str> {
    format!("{SPECIAL_CASE_PREFIX}{label}").into()
}

/// Removes duplicate file ids, keeping first occurrences.
pub fn dedup_ids(ids: &mut smallvec::SmallVec<[FileId; 2]>) {
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert_eq!(
            pattern_parts("/a/*/b-*.node"),
            [
                PatPart::Lit("/a/".into()),
                PatPart::Wild { reason: "*".into() },
                PatPart::Lit("/b-".into()),
                PatPart::Wild { reason: "*".into() },
                PatPart::Lit(".node".into()),
            ]
        );
        assert!(is_probe_kind(RefKind::FsReadDir));
        assert!(!is_probe_kind(RefKind::Require));
    }

    #[test]
    fn a_truncated_glob_is_scoped_by_its_pattern() {
        let s = |d: &str, p: &str| {
            Some(UncertaintyScope {
                dir: d.into(),
                pattern: p.into(),
            })
        };
        // One wildcard for the whole rest: everything below the static directory.
        assert_eq!(glob_scope("/r/data/*"), s("/r/data", "**"));
        // Literal text narrows it: the same forms as the glob's memberships.
        assert_eq!(
            glob_scope("/r/*/package.json"),
            s("/r", "{*/package.json,*/**/*/package.json}")
        );
        assert_eq!(
            glob_scope("/r/icons/*.svg"),
            s("/r/icons", "{*.svg,*/**/*.svg}")
        );
        assert_eq!(
            glob_scope("/r/svc-*/src/routes.ts"),
            s("/r", "{svc-*/src/routes.ts,svc-*/**/*/src/routes.ts}")
        );
        // Glob syntax and commas in literal text match any one character.
        assert_eq!(
            glob_scope("/r/app/[id]/a,b/*.ts"),
            s("/r/app", "{?id?/a?b/*.ts,?id?/a?b/*/**/*.ts}")
        );
        assert_eq!(glob_scope("rel/*"), None);
        assert_eq!(glob_scope("/*"), None);
        assert_eq!(
            membership_patterns("a*/x/*b"),
            ["a*/x/*b", "a*/**/*b"],
            "shared with the memberships the JS resolver records"
        );
    }

    #[test]
    fn names_only_truncation_is_a_membership_record() {
        let mut r = RawRef::new(RefKind::FsRead, Specifier::Unknown, Span::DUMMY);
        let g = GlobOut {
            pattern: "/r/*/package.json".into(),
            matches: Vec::new(),
            truncated: true,
        };
        // A content read: covered by the pattern.
        let u = glob_uncertainty(&r, &g, 2);
        assert_eq!(u.kind, UncertaintyKind::PatternTooBroad);
        assert_eq!(
            u.scope.as_ref().map(|s| &*s.pattern),
            Some("{*/package.json,*/**/*/package.json}")
        );
        // An existence check or a listing: no scope, the kind of a complete expansion.
        r.flags = RefFlags::NAMES;
        let u = glob_uncertainty(&r, &g, 2);
        assert_eq!(u.kind, UncertaintyKind::PartialSpecifier);
        assert!(u.scope.is_none());
        assert!(u.detail.contains("truncated; names only"), "{}", u.detail);
        // The extractor's own record keeps its kind (a cwd-relative path stays one).
        r.uncertainty = Some(rec(UncertaintyKind::CwdDependent, "dir"));
        let u = glob_uncertainty(&r, &g, 2);
        assert_eq!(u.kind, UncertaintyKind::CwdDependent);
        // A module load is never names-only.
        r.kind = RefKind::Require;
        r.uncertainty = None;
        let u = glob_uncertainty(&r, &g, 2);
        assert_eq!(u.kind, UncertaintyKind::PatternTooBroad);
        assert!(u.scope.is_none());
    }

    fn rec(kind: UncertaintyKind, detail: &str) -> Uncertainty {
        Uncertainty::new(kind, Span::DUMMY, detail)
    }

    fn scoped(dir: &str, pattern: &str) -> Uncertainty {
        rec(UncertaintyKind::PatternTooBroad, dir).with_scope(UncertaintyScope {
            dir: dir.into(),
            pattern: pattern.into(),
        })
    }

    #[test]
    fn branch_records_keep_the_one_that_says_most() {
        let partial = || rec(UncertaintyKind::PartialSpecifier, "2 matches");
        let open = || rec(UncertaintyKind::PatternTooBroad, "not expanded");
        let merge = |a: Option<Uncertainty>, b: Option<Uncertainty>| merge_branch_records(a, b);
        // A truncated branch after a complete one keeps its scope, in either order.
        let m = merge(Some(partial()), Some(scoped("/r/data", "**"))).unwrap();
        assert_eq!(m.scope, scoped("/r/data", "**").scope);
        let m = merge(Some(scoped("/r/data", "**")), Some(partial())).unwrap();
        assert!(m.is_covered());
        // An open branch makes the reference open.
        let m = merge(Some(scoped("/r/data", "**")), Some(open())).unwrap();
        assert!(!m.is_covered());
        assert_eq!(m.kind, UncertaintyKind::PatternTooBroad);
        let m = merge(Some(partial()), Some(open())).unwrap();
        assert_eq!(&*m.detail, "not expanded");
        // Ties keep the earlier branch; a missing record changes nothing.
        let m = merge(
            Some(partial()),
            Some(rec(UncertaintyKind::PartialSpecifier, "x")),
        )
        .unwrap();
        assert_eq!(&*m.detail, "2 matches");
        assert_eq!(merge(None, Some(open())).unwrap().detail, open().detail);
        assert!(merge(Some(open()), None).is_some());
        assert!(merge(None, None).is_none());
    }

    #[test]
    fn two_scopes_merge_into_one_that_contains_both() {
        let s = |d: &str, p: &str| UncertaintyScope {
            dir: d.into(),
            pattern: p.into(),
        };
        assert_eq!(
            merge_scopes(&s("/r/a", "**"), &s("/r/b/c", "*.json")),
            s("/r", "{a/**,b/c/*.json}")
        );
        assert_eq!(
            merge_scopes(&s("/r/a", "*.md"), &s("/r/a", "*.txt")),
            s("/r/a", "{*.md,*.txt}")
        );
        // One scope is the common ancestor itself.
        assert_eq!(
            merge_scopes(&s("/r", "*/x"), &s("/r/a", "**")),
            s("/r", "{*/x,a/**}")
        );
        assert_eq!(merge_scopes(&s("/r", "**"), &s("/r/a", "*")), s("/r", "**"));
        // A directory name with glob syntax is not written as a prefix.
        assert_eq!(
            merge_scopes(&s("/r/[id]", "*"), &s("/r/b", "*")),
            s("/r", "**")
        );
        let m = merge_branch_records(Some(scoped("/r/a", "**")), Some(scoped("/r/b", "**")));
        assert_eq!(m.unwrap().scope, Some(s("/r", "{a/**,b/**}")));
    }

    #[test]
    fn temp_dirs_are_a_fixed_list() {
        for p in [
            "/tmp",
            "/tmp/out/bundle.js",
            "/var/tmp/x",
            "/var/folders/ab/T/x",
            "/private/tmp/x",
            "/private/var/folders/ab/T/x",
            "/dev/shm/x",
        ] {
            assert!(is_temp_path(Path::new(p)), "{p}");
        }
        for p in ["/tmpx/a", "/usr/tmp-data", "/var/run/x", "/repo/tmp/x", "/"] {
            assert!(!is_temp_path(Path::new(p)), "{p}");
        }
    }

    /// A resolve context for a file in `/work/repo` (nothing is read: the paths
    /// do not exist).
    fn with_cx<R>(confined: bool, from: &str, f: impl FnOnce(&ResolveCx<'_>) -> R) -> R {
        with_cx_at("/work/repo", confined, from, f)
    }

    /// [`with_cx`] for a checkout at `base`.
    fn with_cx_at<R>(
        base: &str,
        confined: bool,
        from: &str,
        f: impl FnOnce(&ResolveCx<'_>) -> R,
    ) -> R {
        let options = crate::TraceOptions {
            base: base.into(),
            cwd: base.into(),
            confined,
            ..Default::default()
        };
        let fs = crate::DirCache::new();
        if confined {
            fs.confine(&options.base, [PathBuf::from("/opt/modcache")]);
        }
        let interner = crate::PathInterner::new();
        let from = Path::new(from);
        f(&ResolveCx {
            from_path: from,
            from_dir: from.parent().unwrap(),
            options: &options,
            fs: &fs,
            interner: &interner,
        })
    }

    fn raw(kind: RefKind, spec: &'static str) -> RawRef<'static> {
        RawRef::new(kind, Specifier::Literal(spec.into()), Span::new(1, 2))
    }

    /// [`AssetRules::host_reference_at`] where no reading finds anything.
    fn host_reference_at(
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        base: &Path,
        cx: &ResolveCx<'_>,
    ) -> Option<Resolution> {
        let rules = AssetRules::new(PackageBoundary::NODE_MODULES);
        rules.host_reference_at(r, spec, base, true, cx, |_| {
            Resolution::target(Target::Unresolved)
        })
    }

    #[test]
    fn host_paths_by_position() {
        with_cx(true, "/work/repo/src/a.js", |cx| {
            for p in [
                "/tmp/x",
                "/proc",
                "/etc/passwd",
                "/work/other/x",
                "/work/repo2/x",
                "/~/.npmrc",
            ] {
                assert!(cx.is_host_path(Path::new(p)), "{p}");
            }
            // The directories the checkout happens to live below are the machine's too.
            for p in ["/work", "/"] {
                assert!(cx.is_host_path(Path::new(p)), "{p}");
            }
            // Inside the base, and a declared environment root.
            for p in ["/work/repo", "/work/repo/data", "/opt/modcache/m@v1/f"] {
                assert!(!cx.is_host_path(Path::new(p)), "{p}");
            }
            // A pattern above the base: host when spelled as an absolute path, not when it
            // climbs there or starts at `/`.
            let base = Path::new("/work/repo");
            let host = |dir: &str, first: &str| host_pattern_dir(Path::new(dir), base, first, cx);
            assert!(host("/work", "/work/"));
            assert!(host("/tmp", "/tmp/x-"));
            assert!(host("/other", "../../other/"));
            assert!(!host("/work", "../"));
            assert!(!host("/", "/"));
            assert!(!host("/work/repo/data", "data/"));
            // Inside the base at this place only: an absolute spelling, and a climb above
            // the base that comes back in through the checkout's own name.
            assert!(host("/work/repo/data", "/work/repo/data/"));
            assert!(host("/work/repo/data", "../repo/data/"));
            assert!(!host("/work/repo/data", "src/../data/"));
            assert_eq!(display_rel(Path::new("/work/repo/data"), cx), "data");
            assert_eq!(display_rel(Path::new("/work/repo"), cx), ".");
            assert_eq!(display_rel(Path::new("/work"), cx), "..");
            assert_eq!(display_rel(Path::new("/"), cx), "../..");
            assert_eq!(display_rel(Path::new("/tmp/x"), cx), "/tmp/x");
        });
        // A file outside the base (third-party code in a store) may name its neighbors.
        with_cx(true, "/store/pkg/index.js", |cx| {
            assert!(!cx.is_host_path(Path::new("/store/pkg/data.json")));
        });
        // `trace` is not confined: nothing is a host path.
        with_cx(false, "/work/repo/src/a.js", |cx| {
            assert!(!cx.is_host_path(Path::new("/tmp/x")));
        });
    }

    #[test]
    fn host_references_by_kind() {
        let rules = AssetRules::new(PackageBoundary::NODE_MODULES);
        with_cx(true, "/work/repo/src/a.js", |cx| {
            // Reads, listings and assets: an external reference, nothing else. Below a
            // temporary directory that the code writes out: with the record of a path that
            // is taken to name no file of the repository.
            for kind in [RefKind::FsRead, RefKind::FsReadDir, RefKind::Asset] {
                let res = rules.resolve_path(&raw(kind, "/tmp/out"), "/tmp/out", cx);
                assert_eq!(&res.targets[..], [Target::Unresolved], "{kind}");
                let u = res.uncertainty.expect("the record of a temporary path");
                assert_eq!(
                    (u.kind, u.origin, &*u.detail),
                    (
                        UncertaintyKind::GeneratedFile,
                        Origin::Data,
                        "/tmp/out is below a temporary directory: what is there is made at run \
                         time, not a file of the repository"
                    ),
                    "{kind}"
                );
                for spec in ["../../other/x", "../../../tmp/x"] {
                    let res = rules.resolve_path(&raw(kind, spec), spec, cx);
                    assert_eq!(res, Resolution::target(Target::Unresolved), "{kind} {spec}");
                }
            }
            // A glob whose static directory is a host path.
            let parts = [
                Part::Lit("/proc/".into()),
                Part::Wild {
                    reason: "pid".into(),
                },
                Part::Lit("/stat".into()),
            ];
            let r = RawRef::new(
                RefKind::FsRead,
                Specifier::Pattern(parts.to_vec()),
                Span::new(1, 2),
            );
            let res = rules.resolve_pattern(&r, &parts, cx);
            assert_eq!(res, Resolution::target(Target::Unresolved));
            // A module: a record, by where it comes from.
            let r = raw(RefKind::Require, "/usr/lib/addon.node");
            let u = host_reference(
                &r,
                Path::new("/usr/lib/addon.node"),
                "/usr/lib/addon.node",
                cx,
            )
            .uncertainty
            .unwrap();
            assert_eq!(u.kind, UncertaintyKind::UnresolvedSpecifier);
            assert_eq!(
                &*u.detail,
                "/usr/lib/addon.node is an absolute path: not looked up on the build machine"
            );
            let r = raw(RefKind::DynamicImport, "/tmp/out/bundle.mjs");
            let u = host_reference(
                &r,
                Path::new("/tmp/out/bundle.mjs"),
                "/tmp/out/bundle.mjs",
                cx,
            )
            .uncertainty
            .unwrap();
            assert_eq!(
                (u.kind, u.origin),
                (UncertaintyKind::ComputedSpecifier, Origin::Data)
            );
            assert!(u.scope.is_none());
            // `host_reference_at`: literals and the static directory of patterns; nothing
            // for a path inside the base or a pattern without a directory.
            let base = Path::new("/work/repo/src");
            let lit = |s: &'static str| Specifier::Literal(s.into());
            let r = raw(RefKind::Require, "x");
            assert!(host_reference_at(&r, &lit("/opt/tool/x.js"), base, cx).is_some());
            assert!(host_reference_at(&r, &lit("../../../etc/x.js"), base, cx).is_some());
            assert!(host_reference_at(&r, &lit("./x.js"), base, cx).is_none());
            assert!(host_reference_at(&r, &lit("../../.."), base, cx).is_some());
            assert!(host_reference_at(&r, &lit(".."), base, cx).is_none());
            let pat = Specifier::Pattern(vec![
                Part::Lit("/tmp/gen-".into()),
                Part::Wild {
                    reason: "id".into(),
                },
            ]);
            assert!(host_reference_at(&r, &pat, base, cx).is_some());
            // A pattern that climbs above the base is neither expanded nor external.
            let pat = Specifier::Pattern(vec![
                Part::Lit("../../".into()),
                Part::Wild {
                    reason: "name".into(),
                },
            ]);
            let u = host_reference_at(&r, &pat, base, cx)
                .unwrap()
                .uncertainty
                .unwrap();
            assert_eq!(u.kind, UncertaintyKind::PatternTooBroad);
            assert_eq!(
                &*u.detail,
                "x: climbs above the repository root (to ..); not expanded"
            );
            let pat = Specifier::Pattern(vec![
                Part::Lit("gen-".into()),
                Part::Wild {
                    reason: "id".into(),
                },
            ]);
            assert!(host_reference_at(&r, &pat, base, cx).is_none());
        });
    }

    /// What a module load that leaves the repository is, at a checkout at
    /// `base`: the kind of its record and the text after the specifier, or
    /// `None` when it stays inside.
    fn load_at(base: &str, spec: &'static str) -> Option<(UncertaintyKind, String)> {
        let from = format!("{base}/src/a.js");
        with_cx_at(base, true, &from, |cx| {
            let r = raw(RefKind::Require, spec);
            let res = host_reference_at(&r, &r.spec, cx.from_dir, cx)?;
            let u = res.uncertainty.unwrap();
            let text = u.detail.strip_prefix(spec).unwrap_or(&u.detail).to_owned();
            Some((u.kind, text))
        })
    }

    #[test]
    fn the_place_of_the_checkout_is_no_part_of_a_host_reference() {
        const ABSOLUTE: &str = " is an absolute path: not looked up on the build machine";
        const LEAVES: &str = " leaves the repository: not looked up on the build machine";
        const TEMP: &str = " is loaded from a temporary directory: a module generated at run time";
        const GIT: &str = " is inside .git: the repository's metadata, not part of its tree";
        let places = [
            "/work/repo",
            "/tmp/ci/repo",
            "/private/var/folders/ab/T/repo",
            "/home/me/.git/modules/repo",
            "/ci/node_modules/app/repo",
            "/repo",
        ];
        for base in places {
            let load = |spec| load_at(base, spec);
            let unresolved =
                |text: &str| Some((UncertaintyKind::UnresolvedSpecifier, text.to_owned()));
            // A relative path that leaves the repository is outside it, whatever the
            // directories above are: not a temporary file below `/tmp`, not metadata below
            // a directory called `.git`.
            assert_eq!(load("../../sibling/x.js"), unresolved(LEAVES), "{base}");
            assert_eq!(
                load("../../../../../../far.js"),
                unresolved(LEAVES),
                "{base}"
            );
            // Also when the names above lead back in: the checkout is called `repo` here.
            assert_eq!(load("../../repo/src/b.js"), unresolved(LEAVES), "{base}");
            // A temporary directory is one the source writes out.
            let temp = Some((UncertaintyKind::ComputedSpecifier, TEMP.to_owned()));
            assert_eq!(load("/tmp/out/bundle.js"), temp, "{base}");
            assert_eq!(load("/tmp/ci/repo/src/b.js"), temp, "{base}");
            assert_eq!(load("/usr/lib/addon.node"), unresolved(ABSOLUTE), "{base}");
            // An absolute path into the checkout of one machine is not looked up there
            // either, and the text does not call it a path outside the repository.
            assert_eq!(load("/work/repo/src/b.js"), unresolved(ABSOLUTE), "{base}");
            // The repository's own metadata, and metadata the source names.
            assert_eq!(load("../.git/hooks/x.js"), unresolved(GIT), "{base}");
            assert_eq!(
                load("../../other/.git/hooks/x.js"),
                unresolved(GIT),
                "{base}"
            );
            assert_eq!(load("/srv/.git/hooks/x.js"), unresolved(GIT), "{base}");
            // Inside.
            assert_eq!(load("./b.js"), None, "{base}");
            assert_eq!(load("../lib/b.js"), None, "{base}");
        }
        // A pattern: the text counts the climb on what the source writes.
        for base in places {
            let from = format!("{base}/src/a.js");
            with_cx_at(base, true, &from, |cx| {
                let r = raw(RefKind::FsRead, "x");
                let pat = |first: &'static str| {
                    Specifier::Pattern(vec![
                        Part::Lit(first.into()),
                        Part::Wild {
                            reason: "name".into(),
                        },
                    ])
                };
                let detail = |first: &'static str| {
                    host_reference_at(&r, &pat(first), cx.from_dir, cx)
                        .and_then(|res| res.uncertainty)
                        .map(|u| u.detail.to_string())
                };
                assert_eq!(
                    detail("../../../../"),
                    Some("x: climbs above the repository root (to ../../..); not expanded".into()),
                    "{base}"
                );
                assert_eq!(
                    detail("/"),
                    Some("x: starts at the filesystem root; not expanded".into()),
                    "{base}"
                );
                // A read below a host directory is an external reference: no record.
                assert_eq!(detail("../../sibling/"), None, "{base}");
                assert!(
                    host_reference_at(&r, &pat("../../sibling/"), cx.from_dir, cx).is_some(),
                    "{base}"
                );
                assert!(
                    host_reference_at(&r, &pat("../../repo/data/"), cx.from_dir, cx).is_some(),
                    "{base}"
                );
                assert!(host_reference_at(&r, &pat("data/"), cx.from_dir, cx).is_none());
            });
        }
    }
}
