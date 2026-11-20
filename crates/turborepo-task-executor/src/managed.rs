//! The run-owned managed-tool contract shared with task dispatch.

use turborepo_env::EnvironmentVariableMap;

/// Revalidate at actual lookup/probe and process-spawn boundaries. Returned
/// overrides are applied after strict/loose filtering, never to global process
/// environment. A cached map is not fresh installation authority.
pub trait ManagedEnvironment: Send + Sync {
    fn revalidate(&self) -> Result<EnvironmentVariableMap, String>;

    /// Portable selected execution identity, not local paths or timestamps.
    fn cache_identity(&self) -> &str;
}
