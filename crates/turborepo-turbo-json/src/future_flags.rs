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
//!     "affectedUsingTaskInputs": true
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
    /// Enable experimental OpenTelemetry exporter support.
    ///
    /// When enabled, Turborepo will honor the `experimentalObservability`
    /// configuration block (if present) to send run summaries to an
    /// observability backend.
    #[serde(default)]
    pub experimental_observability: bool,
    /// Enforce a minimum length of 32 bytes for
    /// `TURBO_REMOTE_CACHE_SIGNATURE_KEY` when `remoteCache.signature` is
    /// enabled. Short keys weaken the HMAC-SHA256 signature, making
    /// brute-force tag collision feasible.
    #[serde(default)]
    pub longer_signature_key: bool,
    /// Use task-level `inputs` globs to determine which tasks are affected by
    /// changed files when running with `--affected`. When enabled, only tasks
    /// whose declared inputs match the changed files are selected, rather than
    /// selecting all tasks in changed packages.
    #[serde(default)]
    pub affected_using_task_inputs: bool,
    /// Use task-level `inputs` globs to determine which tasks to re-run when
    /// files change in `turbo watch`. When enabled, only tasks whose declared
    /// inputs match the changed files are re-executed, rather than re-running
    /// all tasks in changed packages.
    #[serde(default)]
    pub watch_using_task_inputs: bool,
    /// Include files matching `globalDependencies` globs in the `turbo prune`
    /// output. Without this flag, `globalDependencies` entries are preserved in
    /// the pruned `turbo.json` but the actual files are not copied.
    #[serde(default)]
    pub prune_includes_global_files: bool,
    /// Resolve `--filter` at the task level instead of the package level.
    /// Git-range filters (e.g. `--filter=[main]`) will match against task
    /// `inputs` globs, and the `...` dependency/dependent syntax will
    /// traverse the task graph in addition to the package graph.
    #[serde(default)]
    pub filter_using_tasks: bool,
    /// Move global configuration keys (like `globalDependencies`, `ui`,
    /// `envMode`, etc.) under a top-level `global` key for clarity.
    ///
    /// When enabled, keys are renamed: `globalDependencies` becomes
    /// `global.inputs`, `globalEnv` becomes `global.env`, and
    /// `globalPassThroughEnv` becomes `global.passThroughEnv`.
    #[serde(default)]
    pub global_configuration: bool,
    /// Enable incremental task caching. When enabled, Turborepo persists
    /// tool-managed incremental build artifacts (e.g. `.tsbuildinfo`) across
    /// runs via the remote cache, restoring them before execution on cache
    /// misses to speed up rebuilds.
    #[serde(default)]
    #[schemars(skip)]
    pub incremental_tasks: bool,
}

// Manual TS impl because #[derive(TS)] conflicts with the Iterable and
// Deserializable derives. Each new field must be added to inline(),
// inline_flattened(), decl(), and decl_concrete() below.
impl TS for FutureFlags {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    fn name() -> String {
        "FutureFlags".to_string()
    }

    fn inline() -> String {
        "{ errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, longerSignatureKey?: \
         boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: boolean, \
         pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, globalConfiguration?: \
         boolean }"
            .to_string()
    }

    fn inline_flattened() -> String {
        "{ errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, longerSignatureKey?: \
         boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: boolean, \
         pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, globalConfiguration?: \
         boolean }"
            .to_string()
    }

    fn decl() -> String {
        "type FutureFlags = { errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, \
         longerSignatureKey?: boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: \
         boolean, pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, \
         globalConfiguration?: boolean };"
            .to_string()
    }

    fn decl_concrete() -> String {
        "type FutureFlags = { errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, \
         longerSignatureKey?: boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: \
         boolean, pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, \
         globalConfiguration?: boolean };"
            .to_string()
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
