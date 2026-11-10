//! Trace options shared by the tracer, languages and resolvers.

use std::path::{Path, PathBuf};

use crate::{PhaseFilter, platform::Platform};

/// Default cap on glob and directory expansion.
pub const DEFAULT_MAX_GLOB: usize = 1000;

/// What to do with third-party dependencies.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Externals {
    /// Trace into dependencies (deployment tracing). Default.
    #[default]
    Follow,
    /// Stop at dependency boundaries. Not yet implemented.
    Stop,
    /// Stop at boundaries and list the packages. Not yet implemented.
    List,
}

/// Options for one [`crate::Tracer`]. Languages and resolvers read them through
/// [`crate::ExtractCx`], [`crate::ProjectCx`] and [`crate::ResolveCx`].
///
/// `base` and `cwd` are made absolute and normalized by
/// [`crate::TracerBuilder::build`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceOptions {
    /// Output paths are relative to this directory. Files outside it are
    /// reported.
    pub base: PathBuf,
    /// The process working directory at runtime, for cwd-relative reads.
    pub cwd: PathBuf,
    /// Which phases to include.
    pub phase: PhaseFilter,
    /// Include `TYPE_ONLY` references.
    pub include_types: bool,
    /// Cap on glob and directory expansion.
    pub max_glob: usize,
    /// Ignore globs, matched against paths relative to `base` (see
    /// [`crate::glob`]).
    pub ignore: Vec<String>,
    /// Resolution conditions (JS `exports` conditions). Interpreted by
    /// languages.
    pub conditions: Vec<String>,
    /// Also resolve every other condition a package defines and include all
    /// results, for safety (optionally resolve every `exports` condition).
    /// Off by default.
    pub all_conditions: bool,
    /// Build target such as `linux/amd64`, `darwin/arm64`, `linux/x64/musl` or
    /// a Rust triple. Languages read the parsed form through
    /// [`TraceOptions::platform()`].
    pub target: Option<String>,
    /// Explicit target platform; overrides `target`. See
    /// [`TraceOptions::platform()`].
    pub platform: Option<Platform>,
    /// Include files for every target, not only [`TraceOptions::platform()`]:
    /// Go files excluded by build constraints or `_GOOS_GOARCH` names, Rust
    /// items behind target-dependent `cfg`. Such edges are
    /// [`crate::RefFlags::CONDITIONAL`]. Shared by the languages that
    /// filter by target (`--all-targets`).
    pub all_targets: bool,
    /// Dependency policy.
    pub externals: Externals,
    /// Worker threads; 0 means one per available core.
    pub threads: usize,
    /// Emit `fileHashes`. TODO: not yet implemented; enabling it fails at
    /// build time.
    pub hashes: bool,
    /// The result is a function of the tree below `base`: a path outside `base`
    /// named by a file inside it is a *host path*
    /// ([`crate::ResolveCx::is_host_path`]) and is never looked up on the
    /// machine. A file read, directory listing or glob of one becomes an
    /// external reference: an unresolved edge that keeps the specifier, with no
    /// existence check, no expansion and no match count. A module loaded
    /// from one keeps a record. Graph mode sets it (`base` is the
    /// repository root, and a file outside the repository can never be a
    /// changed file); `trace` leaves it off and reports files outside the
    /// base.
    pub confined: bool,
}

impl Default for TraceOptions {
    fn default() -> Self {
        TraceOptions {
            base: PathBuf::new(),
            cwd: PathBuf::new(),
            phase: PhaseFilter::All,
            include_types: false,
            max_glob: DEFAULT_MAX_GLOB,
            ignore: Vec::new(),
            conditions: Vec::new(),
            all_conditions: false,
            target: None,
            platform: None,
            all_targets: false,
            externals: Externals::Follow,
            threads: 0,
            hashes: false,
            confined: false,
        }
    }
}

impl TraceOptions {
    /// The part of `path` (absolute, normalized) that the place of the checkout
    /// has no say in, as [`crate::DirCache::local`] gives it for the cache
    /// of a trace with these options: below the last directory `path`
    /// shares with `base` when the trace is confined, else `path`.
    pub fn local<'p>(&self, path: &'p Path) -> &'p Path {
        use crate::os::OsStrExt;
        let off = if self.confined {
            crate::fs::shared_prefix(&crate::fs::normalize(&self.base), path)
        } else {
            0
        };
        if off == 0 {
            return path;
        }
        let rest = &path.as_os_str().as_bytes()[off..];
        Path::new(std::ffi::OsStr::from_bytes(
            rest.strip_prefix(b"/").unwrap_or(rest),
        ))
    }

    /// The platform every resolver targets: the `platform` field if set, else
    /// [`TraceOptions::target`] if it parses ([`Platform::parse`]), else the
    /// host. A `target` that does not parse falls back to the host.
    pub fn platform(&self) -> Platform {
        self.platform
            .or_else(|| self.target.as_deref().and_then(Platform::parse))
            .unwrap_or_else(Platform::host)
    }
}
