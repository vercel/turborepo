//! References next to a resolved module (the extractor's [`RESOLVED_BASE`]
//! anchor): the specifier is the text of a [`Located`] module followed by a
//! relative path (`require.resolve('pkg')/../x.wasm`,
//! `import.meta.resolve('pkg')/../inner.js`, `require.resolve('dep', from
//! require.resolve('pkg/package.json'))`).
//!
//! The module is found first, as Node finds it:
//!
//! - `require.resolve(m)` with the CommonJS conditions,
//!   `import.meta.resolve(m)` with the ESM ones, from the directory of the
//!   referencing file;
//! - with a start (`require.resolve(m, { paths: [p] })`, a `require` made by
//!   `createRequire(p)`) from that place instead: a directory relative to the
//!   file's, or a path next to another located module (found the same way,
//!   recursively). A start that is a file stands for its directory, as its
//!   `node_modules` lookup does in Node. With `{ paths }` Node still tries the
//!   calling module's own package first (self-reference through `exports`), so
//!   a request for the file's own package adds that target.
//!
//! Then the relative part is joined onto the file found:
//!
//! - a read (`readFileSync`, an asset) is that path: a file is a leaf, a
//!   directory is reported and not expanded, a missing path is a quiet probe;
//! - a `require` of it searches extensions, `index` files and a directory's
//!   `package.json` as for any path; `import()` loads exactly that file. A
//!   target in a third-party package is the package ([`Target::External`])
//!   unless externals are followed, like a bare import of a file of that
//!   package;
//! - a pattern (a module load with a computed rest) is a glob below the located
//!   file.
//!
//! # A module that is not installed
//!
//! When the module cannot be found on disk (a checkout without `node_modules`),
//! the file a path next to it names is not known either. The reference then
//! gets an unresolved edge with the bare module specifier, which the graph
//! store links through the lockfile entry of the importing package, as it does
//! for a plain import. That edge is everything when the load is exactly the
//! located module. When a path is joined onto it, or another module is searched
//! from it, there is a second unresolved edge for the load itself:
//!
//! - The module is a package of the registry: **the load is taken to stay
//!   inside the package tree its lockfile entry stands for** (the package and
//!   what it installs), as everything a registry package loads by itself is.
//!   The record is an `unresolved_specifier`: not counted by default, open
//!   under `--strict`, and it says so in `--explain`.
//!   `TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0` turns the assumption off (the load
//!   is then open, as below). What it gives up is pinned by runtime cases
//!   (`tests/flow_oracle/notinstalled.mjs`, run by the flow oracle): a module
//!   the package's own search finds outside its tree (a workspace package
//!   through pnpm's hidden hoist), and a path with enough parent steps to climb
//!   out of the store.
//! - The module is a package of the repository (a workspace package the
//!   lockfile links and nothing installed): its lockfile entry leads to its
//!   entry file, not to the file next to it, so the load is open (a computed
//!   specifier).
//! - The search started in another package of the repository (a start that
//!   exists but does not find the module): the lockfile entry of *that*
//!   package's importer decides, which no edge of this file's importer links.
//!   The reference then also depends on the lockfiles at the workspace root as
//!   a whole (a read of each; one that is missing is a probe) and on the
//!   `package.json` of that place, with the same two cases.
//! - The search started in an installed third-party package and found nothing:
//!   Node finds nothing either (an `unresolved_specifier`; what the search
//!   probed is recorded).
//! - The package of the module is there (installed, or linked from the
//!   repository) and does not have the request (an `exports` map without the
//!   subpath, no file at the path): Node finds nothing either, and the
//!   package's manifest is an input of the reference, as for a plain import.
//!
//! Which of the first two it is, is the lockfile's answer for the importer: the
//! answer that links a plain import of the module. This resolver has no
//! lockfile, and what it decides must be checked again by every refresh, so it
//! does not look for the name among the manifests of the repository (nothing
//! would re-trace a file when a package takes the name). It emits the edge of
//! the load in the form of [`turborepo_trace_core::located`] with the record of
//! a registry package, and the graph store, which keeps every lockfile link
//! in the shard's key, makes the record open when the lockfile links the
//! importer to a package of the repository. A name that no lockfile entry links
//! is not resolvable until something is installed, which the `node_modules`
//! probes and the missing link record.

use std::path::{Path, PathBuf};

use oxc_resolver::ResolveError;
use turborepo_trace_core::{
    Externals, ExtraEdge, Part, RawRef, RefKind, Resolution, ResolveCx, Specifier, Target,
    Uncertainty, UncertaintyKind,
    eval::PatPart,
    fs::{FileKind, normalize},
    record,
    resolve_path::assets::{
        computed_specifier, refused_module_pattern, resolve_value_alternatives,
    },
};

use super::{
    At, JS_EXTENSIONS, JsResolver, System, TS_EXTENSIONS,
    assets::{self, JS_ASSET_RULES},
    is_dts, is_package_spec, is_ts_file, not_expanded, oxc_resolve, package_name,
};
use crate::extract::evaluator::{Located, Start};

/// A module that was not found.
struct Missing {
    /// The request that failed.
    module: String,
    /// The directory the search started in.
    from: PathBuf,
    /// The module of the reference itself failed (not one a start is built
    /// from).
    outer: bool,
    /// Where the search started.
    place: Place,
    /// `import.meta.resolve`.
    esm: bool,
    /// The request names a builtin module (`fs`): the same from every place,
    /// and no file.
    builtin: Option<String>,
    /// The package of the request is there (installed, or linked from the
    /// repository) and its manifest refused the request: an `exports` map
    /// without the subpath, no file at the path.
    present: bool,
}

/// Whether a load next to a registry package that is not installed is taken to
/// stay in that package's tree (see the module docs).
/// `TURBO_TRACE_DEBUG_LOCATED_PACKAGE=0` turns it off, for A/B measurements
/// (graphs built either way are not interchangeable).
fn stays_in_package() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var_os("LOCATED_PACKAGE").is_none_or(|v| v != "0")
    });
    *ON
}

/// Where a search that found nothing started.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Place {
    /// In the package of the referencing file: the lockfile entry of that
    /// package's importer says what the request resolves to.
    Own,
    /// In another package of the repository: the lockfile entry of *its*
    /// importer does.
    Repository,
    /// In an installed third-party package.
    Installed,
}

/// Lockfiles of the package managers, at a workspace root.
const LOCKFILES: &[&str] = &[
    "pnpm-lock.yaml",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
];

/// Whether a reference of this kind loads the module it names through the
/// module system.
fn is_load(kind: RefKind) -> bool {
    matches!(
        kind,
        RefKind::Require | RefKind::Import | RefKind::DynamicImport | RefKind::ReExport
    )
}

/// The package a path inside a `node_modules` directory belongs to: the name
/// after the last `node_modules`.
fn package_of(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let rest = &s[s.rfind("/node_modules/")? + "/node_modules/".len()..];
    let name = package_name(rest);
    (!name.is_empty() && !name.starts_with('.')).then(|| name.to_owned())
}

impl JsResolver {
    /// Resolves a reference with the
    /// [`RESOLVED_BASE`](crate::extract::evaluator::RESOLVED_BASE)
    /// anchor (see the module docs).
    pub(super) fn resolve_next_to_module(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        match spec {
            Specifier::Literal(s) => {
                let Some((located, rel)) = Located::parse(s) else {
                    return computed_specifier(r, spec);
                };
                if !(rel.is_empty() || rel.starts_with('/')) {
                    return computed_specifier(r, spec);
                }
                let mut res = self.next_to(&located, rel, r, cx);
                self.add_self_reference(&located, r, cx, &mut res, |own| {
                    self.next_to(own, rel, r, cx)
                });
                res
            }
            Specifier::Alternatives(alts) => self.alternatives_next_to(r, spec, alts, cx),
            Specifier::Pattern(parts) => {
                let located = match parts.first() {
                    Some(Part::Lit(first)) => Located::parse(first),
                    _ => None,
                };
                let Some((located, rest)) = located.filter(|(_, rest)| rest.starts_with('/'))
                else {
                    return computed_specifier(r, spec);
                };
                if !is_load(r.kind) {
                    return Resolution::unresolved(not_expanded(
                        r,
                        format!(
                            "pattern {} next to a resolved module is not expanded",
                            r.spec
                        ),
                    ));
                }
                let mut res = self.pattern_next_to(&located, rest, parts, r, cx);
                self.add_self_reference(&located, r, cx, &mut res, |own| {
                    self.pattern_next_to(own, rest, parts, r, cx)
                });
                res
            }
            Specifier::Unknown => computed_specifier(r, spec),
        }
    }

    /// The values an expression can have, each next to a module. A branch whose
    /// module is not installed has its edges and records of its own
    /// ([`JsResolver::not_located`]) and leaves nothing for the reference:
    /// with only such branches the reference has no edge, and next to
    /// branches that resolved it has no record.
    fn alternatives_next_to(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        alts: &[Specifier<'_>],
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let (mut absent, mut recorded) = (0usize, false);
        let mut out = resolve_value_alternatives(r, spec, alts, |a| {
            let res = self.resolve_next_to_module(r, a, cx);
            absent += usize::from(res.none);
            recorded |= res.uncertainty.is_some();
            res
        });
        // The module of several branches gets one edge.
        let mut seen: Vec<(RefKind, Box<str>)> = Vec::new();
        out.extra.retain(|e| {
            let key = (e.kind, e.spec.clone());
            let new = !seen.contains(&key);
            if new {
                seen.push(key);
            }
            new
        });
        if absent == 0 {
            return out;
        }
        if !recorded {
            out.uncertainty = None;
        }
        if absent == alts.len() {
            out.targets.clear();
            out.none = true;
        } else {
            out.targets.retain(|t| !matches!(t, Target::Unresolved));
            if out.targets.is_empty() {
                out.targets.push(Target::Unresolved);
            }
        }
        out
    }

    /// With a start, Node still tries the package of the calling module first
    /// when the request names it and it has `exports`
    /// (`require.resolve('self', { paths })`): adds what the request gives
    /// from the file (`resolve` gets the module without its start).
    fn add_self_reference(
        &self,
        located: &Located,
        r: &RawRef<'_>,
        cx: &ResolveCx<'_>,
        res: &mut Resolution,
        resolve: impl FnOnce(&Located) -> Resolution,
    ) {
        if located.start().is_none() || !is_load(r.kind) {
            return;
        }
        let Some(pj) = record::pause(|| self.nearest_package_json(cx.fs, cx.from_dir)) else {
            return;
        };
        // The file's own manifest decides: a config input of the reference.
        res.manifests.insert(0, cx.intern(&pj));
        let own = record::pause(|| cx.fs.read_manifest(&pj))
            .ok()
            .and_then(|b| {
                let v: serde_json::Value = serde_json::from_slice(&b).ok()?;
                (v.get("name")?.as_str()? == package_name(located.module())
                    && v.get("exports").is_some())
                .then_some(())
            });
        if own.is_none() {
            return;
        }
        let Some(own) = Located::new(located.module(), false) else {
            return;
        };
        let other = resolve(&own);
        for t in other.targets {
            if !matches!(t, Target::Unresolved) && !res.targets.contains(&t) {
                res.targets.retain(|t| !matches!(t, Target::Unresolved));
                res.targets.push(t);
            }
        }
        res.manifests.extend(other.manifests);
    }

    /// The file `located` is, searched the way Node does (see the module docs).
    fn locate(
        &self,
        located: &Located,
        cx: &ResolveCx<'_>,
        outer: bool,
    ) -> Result<PathBuf, Missing> {
        let start = match located.start() {
            None => None,
            Some(Start::Dir(d)) => Some(normalize(&cx.from_dir.join(&**d))),
            Some(Start::Next(inner, rel)) => {
                Some(normalize(&self.locate(inner, cx, false)?.join(&**rel)))
            }
        };
        // Node looks for `node_modules` next to the start and in every directory above
        // it: from a file, that is from its directory (the probe says what would change
        // it: a directory in the file's place, with a `node_modules` of its own).
        let dir = match start {
            None => cx.from_dir.to_path_buf(),
            Some(p) if record::pause(|| cx.fs.is_file(&p)) => {
                record::missing(&p.join("node_modules"));
                p.parent().unwrap_or(&p).to_path_buf()
            }
            Some(p) => p,
        };
        let system = if located.esm() {
            System::Esm
        } else {
            System::Cjs
        };
        let oxc = self.oxc(system, cx.from_path);
        match oxc_resolve(oxc, At::Dir(&dir), located.module(), cx) {
            Ok(found) => Ok(normalize(found.path())),
            Err(e) => {
                let builtin = match e {
                    ResolveError::Builtin { resolved, .. } => Some(resolved),
                    _ => None,
                };
                let place = if located.start().is_none() {
                    Place::Own
                } else {
                    // Which package the start belongs to is read from the manifests on the
                    // way up, and recorded: a `package.json` that appears between the start
                    // and the manifest found makes the place another package's.
                    if record::pause(|| self.is_third_party(&dir, cx)) {
                        Place::Installed
                    } else if self.nearest_package_json(cx.fs, &dir)
                        == self.nearest_package_json(cx.fs, cx.from_dir)
                    {
                        Place::Own
                    } else {
                        Place::Repository
                    }
                };
                // A package that is there decides by its manifest: a config input of the
                // reference although nothing resolved, as for a plain import of it
                // (editing the manifest resolves the reference again).
                let manifest = if is_package_spec(located.module()) {
                    record::pause(|| {
                        assets::find_package_dir(cx, &dir, package_name(located.module()))
                            .map(|d| d.join("package.json"))
                            .filter(|pj| cx.fs.is_file(pj))
                    })
                } else {
                    None
                };
                if let Some(pj) = &manifest {
                    record::view_read(pj);
                }
                Err(Missing {
                    module: located.module().to_owned(),
                    from: dir,
                    outer,
                    place,
                    esm: located.esm(),
                    builtin,
                    present: manifest.is_some(),
                })
            }
        }
    }

    /// The path `rel` joined onto the file `located` is.
    fn next_to(
        &self,
        located: &Located,
        rel: &str,
        r: &RawRef<'_>,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let file = match self.locate(located, cx, true) {
            Ok(f) => f,
            Err(m) => return self.not_located(m, rel.is_empty(), r, cx),
        };
        let path = normalize(&file.join(rel.trim_start_matches('/')));
        let mut res = if is_load(r.kind) {
            self.load_path(r, &path, cx)
        } else {
            match cx.fs.stat(&path).map(|st| st.kind) {
                Some(FileKind::File) => Resolution::file(cx.intern(&path)),
                Some(FileKind::Dir) => Resolution::unresolved(Uncertainty::new(
                    UncertaintyKind::PatternTooBroad,
                    r.span,
                    format!(
                        "directory {} next to {located} is not expanded",
                        path.display()
                    ),
                )),
                _ => Resolution::target(Target::Unresolved),
            }
        };
        // The manifest of the module's package, unless the trace stops at that package.
        if !res.targets.iter().any(|t| matches!(t, Target::External(_)))
            && let Some(m) = file
                .parent()
                .and_then(|d| self.nearest_package_json(cx.fs, d))
        {
            res.manifests.push(cx.intern(&m));
        }
        res
    }

    /// A module load of the absolute path `path`: `require` searches
    /// extensions, `index` files and a directory's `package.json`;
    /// `import()` loads exactly the file.
    fn load_path(&self, r: &RawRef<'_>, path: &Path, cx: &ResolveCx<'_>) -> Resolution {
        let found = if r.kind == RefKind::Require {
            let text = path.to_string_lossy();
            let dir = path.parent().unwrap_or(path);
            match oxc_resolve(self.oxc(System::Cjs, cx.from_path), At::Dir(dir), &text, cx) {
                Ok(f) => normalize(f.path()),
                Err(e) => return self.unresolved(r, &text, &e, cx),
            }
        } else {
            match cx.fs.stat(path).map(|st| st.kind) {
                Some(FileKind::File) => path.to_path_buf(),
                Some(FileKind::Dir) => {
                    return Resolution::unresolved(Uncertainty::new(
                        UncertaintyKind::UnresolvedSpecifier,
                        r.span,
                        format!("{} is a directory: import() loads files", path.display()),
                    ));
                }
                // A missing file: a probe.
                _ => {
                    return Resolution::unresolved(Uncertainty::new(
                        UncertaintyKind::UnresolvedSpecifier,
                        r.span,
                        format!("cannot resolve {}: no file {}", r.spec, path.display()),
                    ));
                }
            }
        };
        if is_dts(&found) && !self.include_types {
            return Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::UnresolvedSpecifier,
                r.span,
                format!(
                    "{} resolves only to the declaration file {} (excluded; use --include-types)",
                    r.spec,
                    found.display()
                ),
            ));
        }
        if let Some(t) = self.third_party(&found, cx) {
            return Resolution::target(t);
        }
        let mut res = Resolution::file(cx.intern(&found));
        if let Some(m) = found
            .parent()
            .and_then(|d| self.nearest_package_json(cx.fs, d))
        {
            res.manifests.push(cx.intern(&m));
        }
        res
    }

    /// For a registry package searched from `from`, a directory of another
    /// package of the repository, with nothing installed: what it resolves
    /// to is decided by the lockfile entry of that package's importer,
    /// which no edge of the referencing file's importer links. Adds the
    /// files that decide instead: the lockfiles at the workspace root (as
    /// reads; a missing one is a probe, so the first lockfile of a repository
    /// resolves the reference again) and the `package.json` of `from`.
    fn lockfile_edges(&self, from: &Path, cx: &ResolveCx<'_>, res: &mut Resolution) {
        let root = self.workspace_root(cx.from_dir, cx);
        for name in LOCKFILES {
            let path = root.join(name);
            if record::pause(|| cx.fs.is_file(&path)) {
                res.extra.push(ExtraEdge::new(
                    RefKind::FsRead,
                    Target::File(cx.intern(&path)),
                    *name,
                ));
            } else {
                record::missing(&path);
            }
        }
        // Recorded with the levels it passed: a `package.json` nearer to `from` is
        // another manifest to depend on.
        if let Some(m) = self.nearest_package_json(cx.fs, from) {
            res.manifests.push(cx.intern(&m));
        }
    }

    /// The package target of a path in a third-party package, unless externals
    /// are followed: what a bare import of a file of that package gives.
    fn third_party(&self, path: &Path, cx: &ResolveCx<'_>) -> Option<Target> {
        if self.externals == Externals::Follow || !self.is_third_party(path, cx) {
            return None;
        }
        Some(Target::External(package_of(path)?.into()))
    }

    /// A module load whose rest is a pattern (`parts`; `rest` is what follows
    /// the located module in the first literal, starting with `/`): a glob
    /// below the located file.
    fn pattern_next_to(
        &self,
        located: &Located,
        rest: &str,
        parts: &[Part<'_>],
        r: &RawRef<'_>,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let file = match self.locate(located, cx, true) {
            Ok(f) => f,
            Err(m) => return self.not_located(m, false, r, cx),
        };
        // The literal start as an absolute path: its directory part is steps from the
        // file.
        let (head, tail) = rest.rsplit_once('/').unwrap_or(("", rest));
        let dir = normalize(&file.join(head.trim_start_matches('/')));
        if let Some(t) = self.third_party(&file, cx) {
            return Resolution::target(t);
        }
        let mut pat: Vec<PatPart> = parts.iter().map(PatPart::from).collect();
        pat[0] = PatPart::Lit(format!("{}/{tail}", dir.display()));
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
        let Some(mut res) = JS_ASSET_RULES.resolve_module_pattern(r, &pat, &dir, exts, keep, cx)
        else {
            return refused_module_pattern(r);
        };
        if let Some(m) = file
            .parent()
            .and_then(|d| self.nearest_package_json(cx.fs, d))
        {
            res.manifests.push(cx.intern(&m));
        }
        res
    }

    /// The resolution when a module was not found (see "A module that is not
    /// installed" in the module docs). `exact`: the reference is exactly
    /// the located module (nothing is joined onto it).
    fn not_located(
        &self,
        m: Missing,
        exact: bool,
        r: &RawRef<'_>,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let cannot = || {
            Uncertainty::new(
                UncertaintyKind::UnresolvedSpecifier,
                r.span,
                format!("cannot resolve '{}' from {}", m.module, m.from.display()),
            )
        };
        if !is_load(r.kind) {
            // A read: the path is probed when the module appears.
            return Resolution::unresolved(cannot());
        }
        // A builtin module resolves to itself from every place.
        if let Some(b) = &m.builtin
            && exact
            && m.outer
        {
            return Resolution::target(Target::Builtin(b.as_str().into()));
        }
        let mut res = Resolution::none();
        // The whole load is the module, searched from the file's package: the lockfile
        // entry is the answer.
        let whole = exact && m.outer && m.place == Place::Own;
        if m.place == Place::Own {
            let kind = match (whole, m.esm) {
                (true, _) => r.kind,
                (false, true) => RefKind::DynamicImport,
                (false, false) => RefKind::Require,
            };
            res.extra.push(
                ExtraEdge::new(kind, Target::Unresolved, m.module.as_str())
                    .with_uncertainty(cannot()),
            );
        }
        if whole {
            return res;
        }
        // The load itself. Its text has a space: no lockfile entry is looked up for it.
        let package = package_name(&m.module);
        let not_installed = format!(
            "{}: '{}' is not installed (searched from {})",
            r.spec,
            m.module,
            m.from.display()
        );
        // The record of a registry package, on an edge that says whose lockfile entry
        // decides (`turborepo_trace_core::located`): the graph store makes it an open
        // record when that entry is a link to a package of the repository.
        let registry = stays_in_package();
        let plain = || format!("{} (not installed)", r.spec);
        let (u, spec) = match m.place {
            // Node finds nothing there either: what the search probed can change it.
            Place::Installed => (cannot(), plain()),
            // The package is there and does not have the request: its manifest and the
            // paths the search probed are what can change that.
            _ if m.present => (cannot(), plain()),
            Place::Own if registry => (
                Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    r.span,
                    format!(
                        "{not_installed}; the load is taken to stay in the package tree of the \
                         lockfile entry of '{package}'"
                    ),
                ),
                turborepo_trace_core::located::spec(&r.spec.to_string(), package, None),
            ),
            // No edge links the lockfile entry of another importer: the reference
            // depends on the lockfile as a whole and on that package's manifest.
            Place::Repository if registry => {
                self.lockfile_edges(&m.from, cx, &mut res);
                (
                    Uncertainty::new(
                        UncertaintyKind::UnresolvedSpecifier,
                        r.span,
                        format!(
                            "{not_installed}; the lockfile and the package.json of that place say \
                             what '{package}' is, and the load is taken to stay in its package \
                             tree"
                        ),
                    ),
                    turborepo_trace_core::located::spec(
                        &r.spec.to_string(),
                        package,
                        Some(&m.from),
                    ),
                )
            }
            _ => (
                Uncertainty::new(
                    UncertaintyKind::ComputedSpecifier,
                    r.span,
                    format!("{not_installed}, so the file is not known"),
                ),
                plain(),
            ),
        };
        res.extra
            .push(ExtraEdge::new(r.kind, Target::Unresolved, spec).with_uncertainty(u));
        res
    }
}
