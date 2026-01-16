//! Future flags for enabling experimental or upcoming features
//!
//! This module contains the `FutureFlags` structure which allows users to
//! opt-in to experimental features before they become the default behavior.
//!
//! ## Usage
//!
//! Future flags can be configured in the root `turbo.json`:
//!
//! ```json
//! {
//!   "futureFlags": {
//!   }
//! }
//! ```
//!
//! Note: Future flags are only allowed in the root `turbo.json` and will cause
//! an error if specified in workspace packages.

use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use struct_iterable::Iterable;
use ts_rs::TS;

/// Opt into breaking changes prior to major releases, experimental features,
/// and beta features.
///
/// Note: Currently all previous future flags (turboExtendsKeyword,
/// nonRootExtends) have been graduated and are now enabled by default.
#[derive(
    Serialize, Default, Debug, Copy, Clone, Iterable, Deserializable, PartialEq, Eq, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
#[schemars(rename_all = "camelCase")]
#[deserializable()]
pub struct FutureFlags {
    /// When using `outputLogs: "errors-only"`, show task hashes when tasks
    /// complete successfully. This provides visibility into which tasks are
    /// running without showing full output logs.
    #[serde(default)]
    pub errors_only_show_hash: bool,
}

impl TS for FutureFlags {
    type WithoutGenerics = Self;

    fn name() -> String {
        "FutureFlags".to_string()
    }

    fn inline() -> String {
        "{ errorsOnlyShowHash?: boolean }".to_string()
    }

    fn inline_flattened() -> String {
        "{ errorsOnlyShowHash?: boolean }".to_string()
    }

    fn decl() -> String {
        "type FutureFlags = { errorsOnlyShowHash?: boolean };".to_string()
    }

    fn decl_concrete() -> String {
        "type FutureFlags = { errorsOnlyShowHash?: boolean };".to_string()
    }

    fn dependencies() -> Vec<ts_rs::Dependency> {
        vec![]
    }
}

impl FutureFlags {
    /// Create a new FutureFlags
    pub fn new() -> Self {
        Self::default()
    }
}
