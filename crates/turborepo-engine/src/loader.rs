//! Trait abstraction for TurboJson loading.
//!
//! This trait allows the engine to be decoupled from the concrete
//! TurboJsonLoader implementation in turborepo-lib.

use turborepo_repository::package_graph::PackageName;
use turborepo_turbo_json::TurboJson;

use crate::BuilderError;

/// Trait for loading TurboJson configurations.
///
/// This abstraction allows EngineBuilder to work with different
/// loader implementations (real filesystem, in-memory for tests, etc.)
pub trait TurboJsonLoader {
    /// Load the TurboJson for a given package.
    ///
    /// Returns the TurboJson configuration or an error if loading fails.
    /// The error should indicate if no turbo.json exists (via
    /// is_no_turbo_json).
    #[allow(clippy::result_large_err)]
    fn load(&self, package: &PackageName) -> Result<&TurboJson, BuilderError>;
}
