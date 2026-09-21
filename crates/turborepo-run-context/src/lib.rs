//! Shared repository context for Turborepo runs.

use std::sync::Arc;

use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_microfrontends_config::UnifiedTurboJsonLoader;
use turborepo_repository::package_graph::PackageGraph;
use turborepo_scm::SCM;
use turborepo_turbo_json::TurboJson;
use turborepo_ui::ColorConfig;

/// Repository data shared throughout a Turborepo run.
#[derive(Clone)]
pub struct RepoContext {
    pub repo_root: AbsoluteSystemPathBuf,
    pub color_config: ColorConfig,
    pub version: &'static str,
    pub scm: SCM,
    pub pkg_dep_graph: Arc<PackageGraph>,
    pub turbo_json_loader: UnifiedTurboJsonLoader,
    pub root_turbo_json: TurboJson,
}

/// Why remote caching was disabled by local configuration.
/// Determined during options resolution without a network call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RemoteCacheDisabledReason {
    /// User never opted in: no token, no team.
    NotLinked,
    /// `TURBO_TOKEN` is set but no team is configured.
    /// The token gets effectively ignored because `is_linked` returns false.
    TokenWithoutTeam,
    /// `remoteCache.enabled: false` in turbo.json.
    InConfig,
    /// CLI flags (for example, `--cache=local:rw`, or `--no-cache` with
    /// `--force`) disabled both remote reads and writes.
    ByFlags,
    /// The cache config (via `TURBO_CACHE` or `--cache`) explicitly requests
    /// remote caching, but no credentials are configured.
    RequestedWithoutCredentials,
}

/// Why remote caching is temporarily unavailable.
#[derive(Debug, Clone, Copy)]
pub enum RemoteCacheUnavailableReason {
    CouldNotConnect,
    UsageLimitExceeded,
    SpendingPaused,
    DisabledForTeam,
    AuthenticationFailed,
    UnexpectedServerError,
}

/// Resolved remote cache status for the run prelude display.
#[derive(Debug, Clone, Copy)]
pub enum RemoteCacheStatus {
    Disabled(RemoteCacheDisabledReason),
    Enabled,
    Unavailable(RemoteCacheUnavailableReason),
}
