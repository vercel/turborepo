//! Go language plugin of the tracer.
//!
//! # Module system
//!
//! A Go import names a package: a directory. Resolution follows `cmd/go` in
//! module mode without running it ([`modules`]): the standard library (bundled
//! list or `$GOROOT`), the main module or `go.work` workspace modules,
//! `vendor/` (with `vendor/modules.txt`), then the module cache
//! (`$GOMODCACHE/<escaped module>@<version>`), honoring `replace` (local
//! directories and module versions) and `exclude`. Versions are the `require`
//! versions of the main modules, which are the build list for `go` ≥ 1.17; a
//! minimal version selection over the module graph covers imports no listed
//! module provides.
//!
//! An import edge fans out to the package's non-test `.go` files that build for
//! the target ([`package`]): `//go:build` / `// +build` constraints
//! ([`constraint`]) and `_GOOS_GOARCH` file names for
//! [`turborepo_trace_core::Platform`] (the trace's `--target`, default host),
//! `-tags`, cgo and the Go release. The package's cgo, assembly and `.syso`
//! files come along, and `go.mod`, `go.sum`, `go.work` and `vendor/modules.txt`
//! are manifests. Every Go file also depends on its package mates
//! ([`turborepo_trace_core::Resolver::file_edges`]), a `_test.go` file on every
//! test file of its directory.
//!
//! # References ([`extract`])
//!
//! Imports (with aliases, `_` and `.`), `import "C"` with the preamble's
//! `#include` lines, `//go:embed` patterns ([`embed`];
//! [`turborepo_trace_core::RefKind::Embed`], build phase), and the runtime file
//! APIs in [`APIS`] (`os.ReadFile`, `os.Open`, `os.ReadDir`,
//! `template.ParseFiles`/`ParseGlob`, `http.Dir`, ...), which are
//! [`turborepo_trace_core::UncertaintyKind::CwdDependent`] unless absolute.
//! Files with a build constraint produce
//! [`turborepo_trace_core::RefFlags::CONDITIONAL`] references.
//!
//! A read whose path the evaluator can't follow (a parameter, a struct field, a
//! helper's result) is a `computed_specifier` record. Change queries bound it
//! by the path-like strings of the code loaded with it; [`names`] collects the
//! strings of a Go file for that, [`temp`] finds the names that hold a path
//! below a temporary directory, and [`anchors`] follows where the file's own
//! location (`runtime.Caller`) goes.
//!
//! # Third-party packages
//!
//! The package boundary is [`BOUNDARY`] (`pkg/mod` and `vendor`);
//! [`identity::module_identity`] maps a file there to `module@version` plus its
//! `go.sum` hash. A required module that is not in the module cache still has
//! an identity: its imports are edges to the module
//! version ([`modules::PkgLoc::NotDownloaded`]).

// Every public item is documented (the workspace lint, kept per crate here).
#![warn(missing_docs)]

pub mod anchors;
pub mod constraint;
pub mod embed;
pub mod extract;
pub mod header;
pub mod identity;
pub mod lex;
pub mod modules;
pub mod names;
pub mod package;
mod resolve;
pub mod stdlib;
pub mod temp;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub use constraint::BuildContext;
pub use extract::APIS;
pub use identity::{ModuleIdentity, module_identity, module_identity_in};
pub use modules::VendorMode;
pub use resolve::GoResolver;
use turborepo_trace_core::{
    ExtractCx, LangId, Language, PackageBoundary, ProjectCx, RefSink, Resolver,
    resolve_path::AssetRules,
};

/// `TURBO_TRACE_DEBUG_ANCHORS=0` turns following a file's location off (for A/B
/// measurements, as for JS; graphs built either way are not interchangeable): a
/// file that uses `runtime.Caller` then names everything below its location.
fn anchors_enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var_os("ANCHORS").is_none_or(|v| v != "0")
    });
    *ENABLED
}

/// Where third-party Go packages live: the module cache and `vendor/`.
pub const BOUNDARY: PackageBoundary = PackageBoundary::GO;

/// Asset rules for runtime file references: the Go package boundary, every file
/// kept.
pub const ASSET_RULES: AssetRules = AssetRules::new(BOUNDARY);

/// How `GOROOT` is found (only consulted for standard library packages missing
/// from the bundled list).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GoRoot {
    /// `$GOROOT`, else common install locations.
    #[default]
    Auto,
    /// This directory.
    Path(PathBuf),
    /// None: the bundled list only.
    None,
}

/// Go-specific options (the `go` command's flags and environment). Including
/// files for every target is the shared
/// [`turborepo_trace_core::TraceOptions::all_targets`] (`--all-targets`). Unset
/// fields fall back to the environment (`CGO_ENABLED`, `GOFLAGS`
/// `-tags=`/`-mod=`, `GOWORK`, `GOMODCACHE`, `GOPATH`, `GOROOT`) and then to
/// Go's defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoOptions {
    /// Build tags (`-tags`).
    pub tags: Vec<String>,
    /// `CGO_ENABLED`. Default: `$CGO_ENABLED`, else on for native targets and
    /// off when cross-compiling.
    pub cgo: Option<bool>,
    /// The Go release for `go1.N` tags. Default:
    /// [`constraint::LATEST_GO_MINOR`].
    pub go_minor: Option<u32>,
    /// The module cache. Default: `$GOMODCACHE`, `$GOPATH/pkg/mod`,
    /// `~/go/pkg/mod`.
    pub modcache: Option<PathBuf>,
    /// `GOROOT` lookup.
    pub goroot: GoRoot,
    /// Vendoring (`-mod`).
    pub vendor: VendorMode,
    /// Use `go.work` files. Default: unless `GOWORK=off`.
    pub workspace: Option<bool>,
}

/// The Go language.
#[derive(Clone, Debug, Default)]
pub struct GoLanguage {
    opts: Arc<GoOptions>,
}

impl GoLanguage {
    /// Go with the given options.
    pub fn new(opts: GoOptions) -> Self {
        GoLanguage {
            opts: Arc::new(opts),
        }
    }

    /// The options.
    pub fn options(&self) -> &GoOptions {
        &self.opts
    }
}

impl Language for GoLanguage {
    fn id(&self) -> LangId {
        LangId("go")
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["go"]
    }

    fn sniff(&self, _path: &Path, _head: &[u8]) -> bool {
        false
    }

    fn extract<'s>(&self, src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        extract::extract(src, Some(cx.path), out);
        // The file's path names, for reads of other files whose path cannot be
        // evaluated (`turborepo_trace_core::names`).
        let test_file = cx
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_test.go"));
        let names = names::names_with(src, test_file, anchors_enabled());
        if !names.is_empty() {
            out.fact(names.encode());
        }
    }

    fn config_key(&self) -> String {
        format!("{:?}", self.opts)
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        Box::new(GoResolver::new(project, &self.opts))
    }
}
