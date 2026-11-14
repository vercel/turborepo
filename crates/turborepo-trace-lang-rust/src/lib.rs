//! Rust language plugin of the tracer.
//!
//! # Module system
//!
//! A crate is a tree of files rooted at a crate root (`src/lib.rs`,
//! `src/main.rs`, `src/bin/*.rs`, `tests/*.rs`, `build.rs`, ... as declared or
//! auto-discovered in `Cargo.toml`). `mod x;` adds `x.rs` or `x/mod.rs` next to
//! a mod-rs file (a crate root, a `mod.rs`, or a file loaded through
//! `#[path]`), and below `<stem>/` for any other file. `#[path = "…"]`
//! overrides that, relative to the file's directory, or to the inline module
//! directory inside `mod a { … }` blocks. Dependencies are whole crates, linked
//! through `Cargo.toml` and located through `Cargo.lock` ([`project`]).
//!
//! # References
//!
//! See [`extract`] for the recognized constructs and [`APIS`] for the tracked
//! API table. Everything a crate root needs from cargo is attached as
//! file-level edges by the resolver ([`resolve::RustResolver`]): `Cargo.toml`,
//! the workspace `Cargo.toml`, `Cargo.lock` and `.cargo/config.toml`
//! (manifests), the build script, the library a binary or test links,
//! and each dependency crate's library root ([`RefKind::Package`], build
//! phase). Third-party crates (registry, git checkouts, vendor) are opaque
//! packages identified by name, version and checksum from `Cargo.lock`
//! ([`project::CargoProject::identity`]).
//!
//! # Modules
//!
//! - [`lexer`]: exact tokenizer (nested comments, raw/byte/C strings, chars vs
//!   lifetimes).
//! - [`cfg`]: `cfg` predicates evaluated for the target platform
//!   (three-valued).
//! - [`extract`]: the recognizer and partial evaluator.
//! - [`manifest`]: `Cargo.toml` with target auto-discovery and workspace
//!   inheritance.
//! - [`project`]: workspaces, `Cargo.lock`, crate locations, feature
//!   resolution, identities.
//! - [`resolve`]: the [`Resolver`].

// Every public item is documented (the workspace lint, kept per crate here).
#![warn(missing_docs)]

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use turborepo_trace_core::{
    ExtractCx, LangId, Language, PackageBoundary, Phase, ProjectCx, RawRef, RefFlags, RefKind,
    RefSink, Resolver, TraceOptions,
    api::{ApiEntry, BaseRule, CallStyle},
    resolve_path::AssetRules,
};

pub mod cfg;
pub mod extract;
pub mod lexer;
pub mod manifest;
pub mod project;
pub mod resolve;

pub use project::{
    CargoProject, CrateIdentity, FeatureSelection, Package, ResolvedDep, default_cargo_home,
};
pub use resolve::RustResolver;

/// Tracked APIs in the shared format. Matching is by the qualifying
/// path segment (`fs::read`, `File::open`, imported names through `use`), the
/// macro name, or the attribute key; see [`extract`].
pub const APIS: &[ApiEntry] = &[
    // Compile-time embeds, relative to the invoking file.
    ApiEntry::new(&[], "include", RefKind::Embed)
        .style(CallStyle::Macro)
        .base(BaseRule::FileDir),
    ApiEntry::new(&[], "include_str", RefKind::Embed)
        .style(CallStyle::Macro)
        .base(BaseRule::FileDir),
    ApiEntry::new(&[], "include_bytes", RefKind::Embed)
        .style(CallStyle::Macro)
        .base(BaseRule::FileDir),
    // Crate-root relative embeds.
    ApiEntry::new(&["include_dir"], "include_dir", RefKind::Embed)
        .style(CallStyle::Macro)
        .base(BaseRule::PackageRoot),
    ApiEntry::new(&["sqlx"], "migrate", RefKind::Embed)
        .style(CallStyle::Macro)
        .base(BaseRule::PackageRoot),
    ApiEntry::new(&["rust_embed"], "folder", RefKind::Embed)
        .style(CallStyle::Attribute)
        .base(BaseRule::PackageRoot),
    ApiEntry::new(&["askama", "rinja"], "template.path", RefKind::Embed)
        .style(CallStyle::Attribute)
        .base(BaseRule::PackageRoot),
    // Test snapshots (`snapshots/*.snap` next to the file).
    ApiEntry::new(&["insta"], "assert_*_snapshot", RefKind::FsRead)
        .style(CallStyle::Macro)
        .base(BaseRule::FileDir),
    // Runtime reads (std, tokio, async-std, fs-err), cwd-relative unless anchored.
    ApiEntry::new(&["fs"], "read", RefKind::FsRead),
    ApiEntry::new(&["fs"], "read_to_string", RefKind::FsRead),
    ApiEntry::new(&["fs"], "read_dir", RefKind::FsReadDir),
    ApiEntry::new(&["fs"], "metadata", RefKind::FsRead).flags(RefFlags::OPTIONAL),
    ApiEntry::new(&["fs_err"], "read", RefKind::FsRead),
    ApiEntry::new(&["fs_err"], "read_to_string", RefKind::FsRead),
    ApiEntry::new(&["fs_err"], "read_dir", RefKind::FsReadDir),
    ApiEntry::new(&["File"], "open", RefKind::FsRead),
    ApiEntry::new(&["File"], "open_buffered", RefKind::FsRead),
    // Build scripts: C sources and include directories compiled by the `cc` crate.
    ApiEntry::new(&["cc::Build"], "file", RefKind::FsRead)
        .phase(Phase::Build)
        .base(BaseRule::PackageRoot),
    ApiEntry::new(&["cc::Build"], "include", RefKind::FsReadDir)
        .phase(Phase::Build)
        .base(BaseRule::PackageRoot),
];

/// Where third-party crates live: the cargo registry, git checkouts and
/// `vendor/`.
pub const BOUNDARY: PackageBoundary = PackageBoundary::CARGO;

/// Asset rules: the cargo boundary; globs never enter `target/` or `.git/`.
pub const ASSET_RULES: AssetRules = AssetRules {
    skip_dirs: &["target", ".git"],
    ..AssetRules::new(BOUNDARY)
};

/// Rust-specific options (cargo's flags). Including items and dependencies for
/// every target is the shared [`TraceOptions::all_targets`] (`--all-targets`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RustOptions {
    /// `$CARGO_HOME` (registry and git checkouts below it). Default:
    /// `$CARGO_HOME`, else `~/.cargo`.
    pub cargo_home: Option<PathBuf>,
    /// Feature selection (`--features`, `--all-features`,
    /// `--no-default-features`).
    pub features: FeatureSelection,
}

/// The Rust language.
///
/// Dependency crates are looked up below `$CARGO_HOME` (default `~/.cargo`);
/// [`RustLanguage::with_cargo_home`] overrides it.
#[derive(Clone, Debug, Default)]
pub struct RustLanguage {
    opts: RustOptions,
}

impl RustLanguage {
    /// The language with the default `$CARGO_HOME`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The language with an explicit cargo home (registry and git checkouts
    /// below it).
    pub fn with_cargo_home(dir: impl Into<PathBuf>) -> Self {
        RustLanguage {
            opts: RustOptions {
                cargo_home: Some(dir.into()),
                ..RustOptions::default()
            },
        }
    }

    /// The language with explicit options.
    pub fn with_options(opts: RustOptions) -> Self {
        RustLanguage { opts }
    }

    /// The options.
    pub fn options(&self) -> &RustOptions {
        &self.opts
    }
}

impl Language for RustLanguage {
    fn id(&self) -> LangId {
        LangId("rust")
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn sniff(&self, _path: &Path, head: &[u8]) -> bool {
        // `cargo -Zscript` files: `#!/usr/bin/env -S cargo +nightly -Zscript`.
        head.starts_with(b"#!")
            && memchr::memmem::find(&head[..head.len().min(128)], b"cargo").is_some()
    }

    fn extract<'s>(&self, src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        extract::extract(src, cx, out);
    }

    fn config_key(&self) -> String {
        format!("{:?}", self.opts)
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        let model = CargoProject::new(
            Arc::clone(project.fs),
            project.options.platform(),
            self.opts
                .cargo_home
                .clone()
                .unwrap_or_else(default_cargo_home),
        )
        .with_features(self.opts.features.clone())
        .with_all_targets(project.options.all_targets);
        Box::new(RustResolver::new(
            Arc::new(model),
            project.options.externals,
        ))
    }
}

/// What a build script declares through `cargo:rerun-if-changed`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildScriptInputs {
    /// The `rerun-if-changed` references (crate-root relative, build phase).
    pub rerun_if_changed: Vec<RawRef<'static>>,
    /// Whether the script prints any `rerun-if-changed`. Without one, cargo
    /// reruns it when any file in the package changes.
    pub declares_inputs: bool,
}

/// Extracts the `rerun-if-changed` declarations of a build script.
pub fn build_script_inputs(src: &[u8], path: &Path, options: &TraceOptions) -> BuildScriptInputs {
    let cx = ExtractCx {
        file: turborepo_trace_core::PathInterner::new().intern(path),
        path,
        options,
    };
    let mut sink = RefSink::new();
    extract::extract(src, &cx, &mut sink);
    let refs: Vec<RawRef<'static>> = sink
        .refs()
        .iter()
        .filter(|r| {
            r.kind == RefKind::FsRead
                && r.phase == Phase::Build
                && r.base == turborepo_trace_core::Base::Language(extract::CRATE_ROOT)
        })
        .map(|r| to_static(r.clone()))
        .collect();
    BuildScriptInputs {
        declares_inputs: !refs.is_empty()
            || memchr::memmem::find(src, b"rerun-if-changed").is_some(),
        rerun_if_changed: refs,
    }
}

fn to_static(r: RawRef<'_>) -> RawRef<'static> {
    RawRef {
        kind: r.kind,
        spec: r.spec.into_owned(),
        base: r.base,
        span: r.span,
        flags: r.flags,
        phase: r.phase,
        uncertainty: r.uncertainty,
    }
}
