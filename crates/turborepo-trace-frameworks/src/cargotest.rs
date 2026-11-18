//! The `cargo test` adapter.
//!
//! A project per package (`Cargo.toml` with `[package]`). What a test depends
//! on:
//!
//! | Test | Inputs |
//! |---|---|
//! | unit tests (a lib/bin crate root with `test = true`) | the crate's module tree (traced from the root), plus its dev-dependencies, which `cargo test` links into the test harness |
//! | integration tests (`tests/*.rs`, `tests/*/main.rs`, `[[test]]`) | the test file's module tree; the package library and normal + dev dependencies (attached by the Rust resolver to test crate roots) |
//! | every test | `Cargo.toml` (and the workspace root `Cargo.toml`), `Cargo.lock`, `.cargo/config.toml`, the build script and its `rerun-if-changed` inputs |
//!
//! Build-script inputs: each `cargo:rerun-if-changed=<path>` literal becomes a
//! config file (a file), a membership (a directory, `**`) or a probe (missing).
//! A build script that declares none is rerun by cargo when anything in the
//! package changes, so the package directory becomes a `**` membership.
//! Computed paths are reported as uncertainties and fall back to the same
//! membership.
//!
//! Discovery: `tests/*.rs`, `tests/*/main.rs` and every target root cargo
//! builds a test harness for (lib, bins, `[[test]]`, `[[bench]]`/`[[example]]`
//! with `test = true`).
//!
//! Dev-dependency identities (name, version, checksum from `Cargo.lock`) are
//! available from [`CargoTest::crate_inputs`]; the neutral [`TestInputs`] has
//! no field for package identities yet, so through the [`Framework`] trait
//! first-party dev-dependencies appear as entries and third-party ones as
//! entries inside the cargo package boundary (opaque nodes in the store).

use std::path::{Path, PathBuf};

use turborepo_trace_core::{Platform, Specifier, TraceOptions, UncertaintyKind};
use turborepo_trace_lang_rust::{
    CargoProject, CrateIdentity, Package, build_script_inputs, manifest::TargetKind,
};

use crate::{
    Framework, ModuleResolver, fsx,
    fsx::PathFsx,
    inputs::{
        FrameworkInputs, FrameworkUncertainty, GlobalInputs, InputRef, InputRole, MembershipSpec,
        PackageRef, Project, ResolutionOverrides, SnapshotPolicy, Target, TestDiscovery,
        TestInputs, TestPattern,
    },
};

/// The `cargo test` adapter.
#[derive(Copy, Clone, Debug, Default)]
pub struct CargoTest;

/// Everything the adapter knows about one package's tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrateInputs {
    /// The package directory.
    pub dir: PathBuf,
    /// Package name.
    pub name: String,
    /// Package version, if declared.
    pub version: Option<String>,
    /// `Cargo.toml`, workspace `Cargo.toml`, `Cargo.lock`,
    /// `.cargo/config.toml`.
    pub config_files: Vec<PathBuf>,
    /// The library crate root.
    pub lib: Option<PathBuf>,
    /// The build script.
    pub build_script: Option<PathBuf>,
    /// Existing files named by `rerun-if-changed`.
    pub build_files: Vec<PathBuf>,
    /// Directories named by `rerun-if-changed` (or the package directory when
    /// the build script declares nothing), as memberships.
    pub build_memberships: Vec<MembershipSpec>,
    /// Missing `rerun-if-changed` paths.
    pub build_probes: Vec<PathBuf>,
    /// Test crate roots: `(kind, root)`.
    pub tests: Vec<(TargetKind, PathBuf)>,
    /// `autotests = false`: only the `[[test]]` targets are integration tests;
    /// other files in `tests/` are modules of those.
    pub explicit_tests: bool,
    /// Dev-dependency crate roots (first-party and third-party).
    pub dev_dependency_roots: Vec<PathBuf>,
    /// Third-party dev-dependency identities from `Cargo.lock`.
    pub dev_dependencies: Vec<CrateIdentity>,
    /// Problems.
    pub uncertainties: Vec<FrameworkUncertainty>,
}

/// The cargo project model over `fs`, the cache of an [`fsx::direct`] section:
/// the model asks the cache itself.
fn project_model(fs: &std::sync::Arc<turborepo_trace_core::DirCache>) -> CargoProject {
    CargoProject::new(
        std::sync::Arc::clone(fs),
        Platform::host(),
        turborepo_trace_lang_rust::default_cargo_home(),
    )
}

fn is_package_manifest(path: &Path) -> bool {
    fsx::read_to_string(path)
        .ok()
        .and_then(|s| s.parse::<toml_lite::Table>().ok())
        .is_some_and(|t| t.has_package)
}

/// Minimal check for `[package]` without pulling a TOML parser into this crate.
mod toml_lite {
    pub struct Table {
        pub has_package: bool,
    }
    impl std::str::FromStr for Table {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
            Ok(Table {
                has_package: s
                    .lines()
                    .any(|l| matches!(l.trim(), "[package]" | "[project]")),
            })
        }
    }
}

impl CargoTest {
    /// The test inputs of the package in `dir` (see the module docs).
    pub fn crate_inputs(&self, dir: &Path) -> Option<CrateInputs> {
        fsx::direct(|fs| self.crate_inputs_with(&project_model(fs), dir))
    }

    /// [`CargoTest::crate_inputs`] with a caller-provided project model (shared
    /// fs cache).
    pub fn crate_inputs_with(&self, model: &CargoProject, dir: &Path) -> Option<CrateInputs> {
        let pkg = model.package_at(dir)?;
        let ws = model.workspace_of(&pkg);
        let mut out = CrateInputs {
            dir: dir.to_path_buf(),
            name: pkg.manifest.package_name(),
            version: pkg.manifest.version.clone(),
            ..Default::default()
        };
        out.config_files.push(pkg.manifest_path());
        let root_manifest = ws.root.join("Cargo.toml");
        if ws.root != pkg.dir && root_manifest.fx_is_file() {
            out.config_files.push(root_manifest);
        }
        out.config_files.extend(ws.lock_path.clone());
        out.config_files.extend(
            ws.config
                .files
                .iter()
                .filter(|f| !f.starts_with(model.cargo_home()))
                .cloned(),
        );
        out.lib = pkg.lib().map(|t| t.root.clone());
        out.explicit_tests = !pkg.manifest.autotests;
        out.tests = pkg
            .targets
            .iter()
            .filter(|t| t.tested && t.kind != TargetKind::Build)
            .map(|t| (t.kind, t.root.clone()))
            .collect();
        if let Some(b) = pkg.build_script() {
            out.build_script = Some(b.root.clone());
            self.build_inputs(&pkg, &b.root, &mut out);
        }
        for d in model.crate_deps(&pkg.dir, TargetKind::Test) {
            if d.kind != turborepo_trace_lang_rust::manifest::DepKind::Dev {
                continue;
            }
            match &d.target {
                Ok(p) => {
                    out.dev_dependency_roots
                        .extend(p.lib().map(|l| l.root.clone()));
                    out.dev_dependencies.extend(d.identity.clone());
                }
                // Locked or required but not downloaded: the lock entry (or the requirement)
                // identifies the crate, and the test's files link it by that identity.
                Err(_) if d.identity.is_some() => {
                    out.dev_dependencies.extend(d.identity.clone());
                }
                Err(why) => out.uncertainties.push(FrameworkUncertainty {
                    kind: UncertaintyKind::UnresolvedSpecifier,
                    file: pkg.manifest_path(),
                    line: fsx::line_of_field(
                        &fsx::read(pkg.manifest_path()).unwrap_or_default(),
                        &d.key,
                    ),
                    field: Some("dev-dependencies".into()),
                    detail: why.clone(),
                }),
            }
        }
        Some(out)
    }

    fn build_inputs(&self, pkg: &Package, script: &Path, out: &mut CrateInputs) {
        let whole = MembershipSpec {
            dir: pkg.dir.clone(),
            pattern: "**".into(),
            reason: "build script without rerun-if-changed reruns on any package change".into(),
        };
        let Ok(src) = fsx::read(script) else {
            out.build_memberships.push(whole);
            return;
        };
        let inputs = build_script_inputs(&src, script, &TraceOptions::default());
        if !inputs.declares_inputs {
            out.build_memberships.push(whole);
            return;
        }
        let mut computed = false;
        for r in &inputs.rerun_if_changed {
            match &r.spec {
                Specifier::Literal(s) => {
                    let p = fsx::join(&pkg.dir, s);
                    if p.fx_is_dir() {
                        out.build_memberships.push(MembershipSpec {
                            dir: p,
                            pattern: "**".into(),
                            reason: format!("cargo:rerun-if-changed={s}"),
                        });
                    } else if p.fx_is_file() {
                        out.build_files.push(p);
                    } else {
                        out.build_probes.push(p);
                    }
                }
                Specifier::Pattern(parts) => {
                    // Static prefix directory plus a glob for the rest.
                    let shown = r.spec.to_string();
                    let prefix = match parts.first() {
                        Some(turborepo_trace_core::Part::Lit(l)) => {
                            l.rsplit_once('/').map(|(d, _)| d)
                        }
                        _ => None,
                    };
                    match prefix {
                        Some(d) => {
                            let rest = shown.strip_prefix(d).unwrap_or(&shown);
                            out.build_memberships.push(MembershipSpec {
                                dir: fsx::join(&pkg.dir, d),
                                pattern: rest.trim_start_matches('/').to_owned(),
                                reason: format!("cargo:rerun-if-changed={shown}"),
                            });
                        }
                        None => computed = true,
                    }
                }
                _ => computed = true,
            }
            if computed {
                out.uncertainties.push(FrameworkUncertainty {
                    kind: UncertaintyKind::ComputedSpecifier,
                    file: script.to_path_buf(),
                    line: line_of(&src, r.span.start),
                    field: Some("rerun-if-changed".into()),
                    detail: format!("computed rerun-if-changed path {}", r.spec),
                });
            }
        }
        if computed {
            out.build_memberships.push(whole);
        }
    }
}

fn line_of(src: &[u8], offset: u32) -> u32 {
    let end = (offset as usize).min(src.len());
    1 + src[..end].iter().filter(|b| **b == b'\n').count() as u32
}

fn rel(dir: &Path, p: &Path) -> Option<String> {
    p.strip_prefix(dir)
        .ok()
        .map(|r| r.to_string_lossy().into_owned())
}

impl Framework for CargoTest {
    fn id(&self) -> &'static str {
        "cargo-test"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["Cargo.toml"]
    }

    fn is_config_file(&self, path: &Path) -> bool {
        path.file_name().is_some_and(|n| n == "Cargo.toml") && is_package_manifest(path)
    }

    fn discover(&self, root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fsx::walk_dirs(root, &mut |d| {
            if d.components()
                .any(|c| matches!(c.as_os_str().to_str(), Some("target" | "vendor")))
            {
                return;
            }
            let m = d.join("Cargo.toml");
            if m.fx_is_file() && is_package_manifest(&m) {
                out.push(m);
            }
        });
        out.sort();
        out
    }

    fn config_for(&self, test: &Path) -> Option<PathBuf> {
        let mut d = test.parent();
        while let Some(dir) = d {
            let m = dir.join("Cargo.toml");
            if m.fx_is_file() && is_package_manifest(&m) {
                return Some(m);
            }
            d = dir.parent();
        }
        None
    }

    fn load(&self, config: &Path) -> FrameworkInputs {
        let mut fi = FrameworkInputs {
            framework: self.id(),
            config_file: Some(config.to_path_buf()),
            ..Default::default()
        };
        let dir = config.parent().unwrap_or(Path::new("/")).to_path_buf();
        let Some(ci) = self.crate_inputs(&dir) else {
            fi.uncertainties.push(FrameworkUncertainty {
                kind: UncertaintyKind::ParseIncomplete,
                file: config.to_path_buf(),
                line: 0,
                field: None,
                detail: "not a readable package manifest".into(),
            });
            return fi;
        };
        let mut global = GlobalInputs {
            config_files: ci.config_files.clone(),
            ..Default::default()
        };
        global.config_files.extend(ci.build_files.iter().cloned());
        global.memberships = ci.build_memberships.clone();
        global.probes = ci.build_probes.clone();
        global.package = Some(PackageRef {
            ecosystem: "crates",
            name: ci.name.clone(),
            version: ci.version.clone(),
            root: dir.clone(),
        });
        if let Some(b) = &ci.build_script {
            global.entries.push(InputRef {
                role: InputRole::BuildScript,
                target: Target::Path(b.clone()),
            });
        }
        // A library appearing later is linked into every integration test.
        if ci.lib.is_none() {
            global.probes.push(dir.join("src/lib.rs"));
        }
        // Cargo's own discovery, unless the manifest turns it off: then a file in
        // `tests/` is a test only as a `[[test]]` target (the roots below), and
        // anything else there is a module of one.
        let mut include = if ci.explicit_tests {
            Vec::new()
        } else {
            vec![
                TestPattern::Glob("tests/*.rs".into()),
                TestPattern::Glob("tests/*/main.rs".into()),
            ]
        };
        for (_, root) in &ci.tests {
            if let Some(r) = rel(&dir, root) {
                let g = TestPattern::Glob(r);
                if !include.contains(&g) {
                    include.push(g);
                }
            }
        }
        fi.projects.push(Project {
            name: ci.name.clone(),
            config_file: Some(config.to_path_buf()),
            root_dir: dir.clone(),
            global,
            resolution: ResolutionOverrides::default(),
            discovery: TestDiscovery {
                roots: vec![dir.clone()],
                include,
                exclude: Vec::new(),
                glob_base: Some(dir.clone()),
                extensions: vec!["rs".into()],
                absolute: false,
                choice: None,
                spec: None,
            },
            mocks: Default::default(),
            snapshots: SnapshotPolicy::Default,
            uncertainties: ci.uncertainties,
        });
        fi
    }

    fn test_inputs(
        &self,
        project: &Project,
        test: &Path,
        _src: &[u8],
        _resolver: &dyn ModuleResolver,
    ) -> TestInputs {
        let mut out = TestInputs {
            test: test.to_path_buf(),
            package: project.global.package.clone(),
            ..Default::default()
        };
        // The package, its targets and (for a unit test) its dev-dependencies, from the
        // project model.
        let Some((pkg, dev)) = fsx::direct(|fs| {
            let model = project_model(fs);
            let pkg = model.package_at(&project.root_dir)?;
            let unit = matches!(
                pkg.target_of(test).map(|t| t.kind),
                Some(TargetKind::Lib | TargetKind::Bin)
            );
            let dev = if unit {
                model.crate_deps(&pkg.dir, TargetKind::Test)
            } else {
                Vec::new()
            };
            Some((pkg, dev))
        }) else {
            return out;
        };
        match pkg.target_of(test).map(|t| t.kind) {
            // Unit tests: the crate root is traced; dev-dependencies are linked too.
            Some(TargetKind::Lib | TargetKind::Bin) => {
                for d in dev {
                    if d.kind == turborepo_trace_lang_rust::manifest::DepKind::Dev
                        && let Ok(p) = &d.target
                        && let Some(l) = p.lib()
                    {
                        out.entries.push(InputRef {
                            role: InputRole::LinkedCrate,
                            target: Target::Path(l.root.clone()),
                        });
                    }
                }
            }
            // Integration tests: the library is linked (the Rust resolver attaches it, its
            // build script and dependencies to test crate roots); listed here too so the
            // adapter output is complete on its own.
            Some(_) => {
                if let Some(l) = pkg.lib() {
                    out.entries.push(InputRef {
                        role: InputRole::LinkedCrate,
                        target: Target::Path(l.root.clone()),
                    });
                }
            }
            None => {
                // `tests/foo.rs` not (yet) a target, e.g. with `autotests = false`.
                out.uncertainties.push(FrameworkUncertainty {
                    kind: UncertaintyKind::UnsupportedSyntax,
                    file: test.to_path_buf(),
                    line: 0,
                    field: None,
                    detail: "not a crate root of the package's test targets".into(),
                });
            }
        }
        out.entries.sort();
        out.entries.dedup();
        out
    }
}
