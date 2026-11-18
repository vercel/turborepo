//! The Rust resolver: module files, embeds, build-script inputs and runtime
//! reads, plus the per-file edges cargo implies (the package's `Cargo.toml`,
//! `Cargo.lock`, the build script, the library linked into binaries and tests,
//! and dependency crates).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use turborepo_trace_core::{
    Base, DirCache, EnvRoot, Externals, ExtraEdge, FileId, PackageIdentity, Phase, RawRef, RefKind,
    Resolution, ResolveCx, Resolver, Specifier, Target, Uncertainty, UncertaintyKind,
    fs::{FileKind, normalize},
    resolve_path::{
        MissingAsset,
        assets::{computed_specifier, explain_missing, resolve_alternatives},
    },
};

use crate::{
    ASSET_RULES,
    extract::{CRATE_ROOT, MOD, MOD_PATH, OUT_DIR, TEMPLATES},
    manifest::TargetKind,
    project::{CargoProject, Package},
};

/// Resolver for Rust references (see the module docs).
#[derive(Debug)]
pub struct RustResolver {
    project: Arc<CargoProject>,
    externals: Externals,
    /// Files reached through `#[path]`: they own their directory like `mod.rs`
    /// files.
    path_owned: Mutex<HashSet<PathBuf>>,
    /// Crates reported as not downloaded (`name@version`), with the file that
    /// pins each.
    not_downloaded: Mutex<HashMap<String, Vec<PathBuf>>>,
}

impl RustResolver {
    /// A resolver over `project`.
    pub fn new(project: Arc<CargoProject>, externals: Externals) -> Self {
        RustResolver {
            project,
            externals,
            path_owned: Mutex::default(),
            not_downloaded: Mutex::default(),
        }
    }

    /// The project model.
    pub fn project(&self) -> &Arc<CargoProject> {
        &self.project
    }

    fn is_mod_rs(&self, file: &Path) -> bool {
        file.file_name().is_some_and(|n| n == "mod.rs")
            || self
                .path_owned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(file)
            || self.project.is_crate_root(file)
    }

    fn crate_dir(&self, cx: &ResolveCx<'_>) -> Option<PathBuf> {
        self.project.package_of(cx.from_dir).map(|p| p.dir.clone())
    }

    fn resolve_mod(
        &self,
        r: &RawRef<'_>,
        spec: &str,
        by_path: bool,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        let (inline, leaf) = if by_path {
            spec.split_once("::").unwrap_or(("", spec))
        } else {
            spec.rsplit_once("::").unwrap_or(("", spec))
        };
        let file = cx.from_path;
        let dir = cx.from_dir;
        let stem = file
            .file_stem()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        let owned = self.is_mod_rs(file);
        // Directory the declaration is relative to.
        let base_for = |mod_rs: bool| -> PathBuf {
            if by_path && inline.is_empty() {
                return dir.to_path_buf();
            }
            let mut b = dir.to_path_buf();
            if !mod_rs {
                b.push(&stem);
            }
            if !inline.is_empty() {
                b.push(inline);
            }
            b
        };
        let attempt = |mod_rs: bool| -> Option<PathBuf> {
            let base = base_for(mod_rs);
            if by_path {
                let p = normalize(&base.join(leaf));
                return cx.fs.is_file(&p).then_some(p);
            }
            let a = base.join(format!("{leaf}.rs"));
            if cx.fs.is_file(&a) {
                return Some(a);
            }
            let b = base.join(leaf).join("mod.rs");
            cx.fs.is_file(&b).then_some(b)
        };
        // A file reached through `#[path]` from a file this resolver has not seen
        // (graph mode) owns its directory too: fall back to the mod-rs rule.
        let found = attempt(owned).or_else(|| (!owned).then(|| attempt(true)).flatten());
        match found {
            Some(p) => {
                if by_path {
                    self.path_owned
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(p.clone());
                }
                Resolution::file(cx.intern(&p))
            }
            None => {
                let base = base_for(owned);
                let detail = if by_path {
                    format!("#[path] module {} not found", base.join(leaf).display())
                } else {
                    format!(
                        "module `{leaf}` not found: neither {} nor {}",
                        base.join(format!("{leaf}.rs")).display(),
                        base.join(leaf).join("mod.rs").display()
                    )
                };
                Resolution::unresolved(Uncertainty::new(
                    UncertaintyKind::UnresolvedSpecifier,
                    r.span,
                    detail,
                ))
            }
        }
    }

    /// An embed, build input or runtime read relative to `base` (a language
    /// anchor such as the crate root), through the shared asset rules with
    /// an explicit base directory.
    fn resolve_file(&self, r: &RawRef<'_>, base: &Path, cx: &ResolveCx<'_>) -> Resolution {
        self.resolve_spec(r, &r.spec, base, cx)
    }

    fn resolve_spec(
        &self,
        r: &RawRef<'_>,
        spec: &Specifier<'_>,
        base: &Path,
        cx: &ResolveCx<'_>,
    ) -> Resolution {
        if r.kind == RefKind::Asset {
            // An anchored path value bound to a name (the shared anchored-asset policy).
            return ASSET_RULES.resolve_anchored_asset(r, spec, base, MissingAsset::Report, cx);
        }
        match spec {
            Specifier::Literal(s) => {
                let path = normalize(&base.join(s.as_ref()));
                let dir_kinds = matches!(r.kind, RefKind::Embed | RefKind::FsReadDir)
                    || r.phase == turborepo_trace_core::Phase::Build;
                match cx.fs.stat(&path).map(|s| s.kind) {
                    Some(FileKind::File) => Resolution::file(cx.intern(&path)),
                    Some(FileKind::Dir) if dir_kinds => {
                        Resolution::target(Target::Dir(cx.intern(&path)))
                    }
                    _ if matches!(r.kind, RefKind::Embed | RefKind::Mod)
                        || r.phase == turborepo_trace_core::Phase::Build =>
                    {
                        Resolution::unresolved(Uncertainty::new(
                            UncertaintyKind::UnresolvedSpecifier,
                            r.span,
                            format!("{} does not exist", path.display()),
                        ))
                    }
                    _ => ASSET_RULES.resolve_path_in(r, s, base, cx),
                }
            }
            Specifier::Pattern(parts) => {
                let mut res = ASSET_RULES.resolve_pattern_in(r, parts, base, cx);
                if res.uncertainty.is_none()
                    && let Some(u) = &r.uncertainty
                {
                    res.uncertainty = Some(u.clone());
                }
                res
            }
            Specifier::Alternatives(alts) => {
                resolve_alternatives(r, spec, alts, |a| self.resolve_spec(r, a, base, cx))
            }
            Specifier::Unknown => computed_specifier(r, spec),
        }
    }

    fn templates(&self, r: &RawRef<'_>, cx: &ResolveCx<'_>) -> Resolution {
        let Some(root) = self.crate_dir(cx) else {
            return no_crate(r);
        };
        let mut dirs = Vec::new();
        for cfg in ["askama.toml", "rinja.toml"] {
            let p = root.join(cfg);
            if cx.fs.is_file(&p)
                && let Ok(b) = cx.fs.read_cached(&p)
                && let Ok(t) = toml::from_str::<toml::Table>(&String::from_utf8_lossy(&b))
                && let Some(ds) = t
                    .get("general")
                    .and_then(|g| g.get("dirs"))
                    .and_then(toml::Value::as_array)
            {
                dirs.extend(
                    ds.iter()
                        .filter_map(toml::Value::as_str)
                        .map(|d| normalize(&root.join(d))),
                );
            }
        }
        if dirs.is_empty() {
            dirs.push(root.join("templates"));
        }
        let spec = r.spec.as_literal().unwrap_or_default();
        let dir = dirs
            .iter()
            .find(|d| cx.fs.is_file(&d.join(spec)))
            .unwrap_or(&dirs[0])
            .clone();
        let mut res = self.resolve_file(r, &dir, cx);
        // Templates include and extend each other by name: the whole directory is an
        // input.
        if cx.fs.is_dir(&dir) {
            res.extra.push(ExtraEdge::new(
                RefKind::Embed,
                Target::Dir(cx.intern(&dir)),
                "askama templates directory (include/extends)",
            ));
        }
        res
    }

    fn dep_edge(&self, key: &str, pkg: &Package, cx: &ResolveCx<'_>) -> Option<ExtraEdge> {
        let lib = pkg.lib()?;
        if pkg.third_party && self.externals != Externals::Follow {
            return Some(ExtraEdge::new(
                RefKind::Package,
                Target::External(pkg.manifest.package_name().into()),
                format!("crate {key}"),
            ));
        }
        let ver = pkg.manifest.version.as_deref().unwrap_or("");
        Some(ExtraEdge::new(
            RefKind::Package,
            Target::File(cx.intern(&lib.root)),
            format!("crate {key} {ver}").trim_end().to_owned(),
        ))
    }
}

impl RustResolver {
    /// Resolution by base anchor (see [`Resolver::resolve`]).
    fn resolve_base(&self, r: &RawRef<'_>, _from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        match &r.base {
            Base::Language(MOD) | Base::Language(MOD_PATH) => match r.spec.as_literal() {
                Some(s) => self.resolve_mod(r, s, r.base == Base::Language(MOD_PATH), cx),
                None => computed_specifier(r, &r.spec),
            },
            Base::Language(CRATE_ROOT) => match self.crate_dir(cx) {
                Some(root) => self.resolve_file(r, &root, cx),
                None => no_crate(r),
            },
            Base::Language(TEMPLATES) => self.templates(r, cx),
            Base::Language(OUT_DIR) => {
                Resolution::unresolved(r.uncertainty.clone().unwrap_or_else(|| {
                    Uncertainty::new(UncertaintyKind::GeneratedFile, r.span, r.spec.to_string())
                }))
            }
            Base::Language(_) => computed_specifier(r, &r.spec),
            base => match cx.base_dir(base) {
                Some(dir) => {
                    let dir = dir.into_owned();
                    let mut res = self.resolve_spec(r, &r.spec, &dir, cx);
                    if res.uncertainty.is_none()
                        && let Some(u) = &r.uncertainty
                        && !matches!(r.spec, Specifier::Literal(_))
                    {
                        res.uncertainty = Some(u.clone());
                    }
                    res
                }
                None => computed_specifier(r, &r.spec),
            },
        }
    }
}

fn no_crate(r: &RawRef<'_>) -> Resolution {
    Resolution::unresolved(Uncertainty::new(
        UncertaintyKind::UnresolvedSpecifier,
        r.span,
        format!("{}: no Cargo.toml above the file", r.spec),
    ))
}

impl Resolver for RustResolver {
    fn resolve(&self, r: &RawRef<'_>, from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        let res = self.resolve_base(r, from, cx);
        if r.kind == RefKind::SharedLib {
            // `Library::new`: keep the extractor's cwd/loader-path record for a missing
            // library.
            return explain_missing(r, res);
        }
        res
    }

    fn file_edges(&self, file: FileId, cx: &ResolveCx<'_>) -> Vec<ExtraEdge> {
        let _ = file;
        let path = cx.from_path;
        let Some(pkg) = self.project.package_of(cx.from_dir) else {
            return Vec::new();
        };
        let mut out = vec![
            ExtraEdge::new(
                RefKind::Manifest,
                Target::File(cx.intern(&pkg.manifest_path())),
                "Cargo.toml",
            )
            .with_phase(Phase::Build),
        ];
        if !pkg.third_party {
            let ws = self.project.workspace_of(&pkg);
            if ws.root != pkg.dir {
                let m = ws.root.join("Cargo.toml");
                if cx.fs.is_file(&m) {
                    out.push(
                        ExtraEdge::new(
                            RefKind::Manifest,
                            Target::File(cx.intern(&m)),
                            "workspace Cargo.toml",
                        )
                        .with_phase(Phase::Build),
                    );
                }
            }
            if let Some(l) = &ws.lock_path {
                out.push(
                    ExtraEdge::new(RefKind::Manifest, Target::File(cx.intern(l)), "Cargo.lock")
                        .with_phase(Phase::Build),
                );
            }
            for f in &ws.config.files {
                // Ancestor configs above the trace base still drive source replacement,
                // but only configs inside it are reported as inputs.
                if f.starts_with(&cx.options.base) && !f.starts_with(self.project.cargo_home()) {
                    out.push(
                        ExtraEdge::new(
                            RefKind::Manifest,
                            Target::File(cx.intern(f)),
                            ".cargo/config.toml",
                        )
                        .with_phase(Phase::Build),
                    );
                }
            }
        }
        let Some(t) = pkg.target_of(path) else {
            return out;
        };
        if t.kind != TargetKind::Build
            && let Some(b) = pkg.build_script()
        {
            out.push(ExtraEdge::new(
                RefKind::Package,
                Target::File(cx.intern(&b.root)),
                "build script",
            ));
        }
        if matches!(
            t.kind,
            TargetKind::Bin | TargetKind::Test | TargetKind::Bench | TargetKind::Example
        ) && let Some(lib) = pkg.lib()
        {
            out.push(ExtraEdge::new(
                RefKind::Package,
                Target::File(cx.intern(&lib.root)),
                format!("lib {}", lib.name),
            ));
        }
        for d in self.project.crate_deps(&pkg.dir, t.kind) {
            match &d.target {
                Ok(p) => out.extend(self.dep_edge(&d.key, p, cx)),
                // A crate that `Cargo.lock` pins but that is not downloaded: the graph store
                // links the target to the crate's package (`external_identity`) and drops
                // the record. What the dependency is is decided by `Cargo.toml` and
                // `Cargo.lock`, which are inputs of the file.
                Err(why) => out.push(
                    ExtraEdge::new(
                        RefKind::Package,
                        match &d.identity {
                            Some(id) => {
                                let name = format!("{}@{}", id.name, id.version);
                                // What pins it: the lockfile, else the manifest that
                                // declares the requirement.
                                let ws = self.project.workspace_of(&pkg);
                                let pin = match (&ws.lock_path, id.version.as_str()) {
                                    (Some(l), v) if v != "*" => l.clone(),
                                    _ => pkg.manifest_path(),
                                };
                                self.not_downloaded
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .entry(name.clone())
                                    .or_insert_with(|| vec![pin]);
                                Target::External(name.into())
                            }
                            None => Target::Unresolved,
                        },
                        format!("crate {}", d.key),
                    )
                    .with_uncertainty(Uncertainty::new(
                        UncertaintyKind::UnresolvedSpecifier,
                        turborepo_trace_core::Span::DUMMY,
                        why.clone(),
                    )),
                ),
            }
        }
        out
    }

    fn clear_cache(&self) {
        self.not_downloaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.project.clear();
        self.path_owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// A crate that is locked or required but not downloaded (see
    /// `file_edges`): the identity its sources would have in `$CARGO_HOME`,
    /// pinned by `Cargo.lock` or, without a lock entry, by the manifest
    /// that requires it.
    fn external_identity(&self, name: &str, _fs: &DirCache) -> Option<PackageIdentity> {
        let pins = self
            .not_downloaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name)
            .cloned()?;
        let (krate, version) = name.rsplit_once('@')?;
        let locked = self.project.locked_crate(krate, version);
        let root = match &locked {
            Some((_, _, root)) => root.clone(),
            None => self
                .project
                .cargo_home()
                .join("registry/src/index.crates.io")
                .join(format!("{krate}-{version}")),
        };
        Some(PackageIdentity {
            ecosystem: "crates",
            name: krate.to_owned(),
            version: version.to_owned(),
            integrity: locked.and_then(|(id, _, _)| id.checksum.or(id.source)),
            root,
            pins,
        })
    }

    fn environment_roots(&self) -> Vec<EnvRoot> {
        let home = self.project.cargo_home();
        vec![
            EnvRoot::new("cargo:registry", home.join("registry")),
            EnvRoot::new("cargo:git", home.join("git")),
        ]
    }

    fn package_identity(&self, path: &Path, fs: &DirCache) -> Option<PackageIdentity> {
        // In the part of the path that is the repository's: a checkout below a
        // directory called `vendor` is no vendored crate.
        let vendored = fs
            .local(path)
            .components()
            .any(|c| c.as_os_str() == std::ffi::OsStr::new("vendor"));
        if !crate::BOUNDARY.contains_from(path, fs.local_offset(path)) && !vendored {
            return None;
        }
        if let Some((id, root, lock)) = self.project.identity_with_root(path) {
            return Some(PackageIdentity {
                ecosystem: "crates",
                name: id.name,
                version: id.version,
                integrity: id.checksum.or(id.source),
                root,
                pins: vec![lock],
            });
        }
        // No loaded workspace graph links this crate yet (for example when a cached
        // result is validated before anything was traced): the crate's own
        // manifest names it, and the caller fills the integrity from
        // `Cargo.lock`.
        let dir = if fs.is_dir(path) {
            path
        } else {
            path.parent()?
        };
        let pkg = self.project.package_of(dir)?;
        if !pkg.third_party {
            return None;
        }
        Some(PackageIdentity {
            ecosystem: "crates",
            name: pkg.manifest.name.clone()?,
            version: pkg.manifest.version.clone()?,
            integrity: None,
            root: pkg.dir.clone(),
            pins: Vec::new(),
        })
    }
}
