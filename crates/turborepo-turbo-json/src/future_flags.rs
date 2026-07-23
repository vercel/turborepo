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
    /// Select requested task entrypoints according to whether the task resolves
    /// a command in the repository. When any package can run a requested task,
    /// packages without a command are not used as entrypoints. Tasks with no
    /// command anywhere remain available for graph-only orchestration, and
    /// missing tasks reached as dependencies remain in the Task Graph.
    #[serde(default)]
    pub strict_task_entrypoint_selection: bool,
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
    /// Treat the crates of a Cargo workspace as Turborepo packages.
    ///
    /// When enabled, Rust crates are discovered via `cargo metadata` and
    /// participate in the package graph: they resolve in `--filter`
    /// expressions, propagate `--affected`, and appear in `turbo query`.
    /// Filtered builds execute each selected crate. Unfiltered builds prefer
    /// entrypoints, falling back to libraries when no entrypoints exist.
    /// Entrypoints also expose `run` and `dev`. The `test`, `check`,
    /// `clippy`/`lint`, `bench`, and `doc`/`docs` tasks are selectable per
    /// crate with `--filter`. An unfiltered run executes one workspace-wide
    /// Cargo verification command; filtered runs use the selected crates,
    /// or the workspace command when the workspace package is selected
    /// directly.
    ///
    /// All crates implicitly register `build` and the verification tasks;
    /// entrypoints with one binary also register `run` and `dev`. The workspace
    /// package registers the verification tasks. Normal task definitions
    /// configure or override these defaults, and package configuration can
    /// exclude them with `extends: false`.
    ///
    /// Task caching uses Cargo-derived inputs and caches entrypoint build
    /// deliverables. Library builds default to uncached. This feature is
    /// experimental.
    #[serde(default)]
    pub experimental_cargo_workspaces: bool,
    /// Serve the Remote Cache as an sccache storage backend for Cargo crate
    /// tasks. When enabled (together with `experimentalCargoWorkspaces` and
    /// a linked Remote Cache), `turbo` starts a local proxy and routes
    /// rustc invocations through `sccache`, caching individual compilation
    /// units in the Remote Cache.
    ///
    /// Only engages in CI: cold environments are where a compile cache
    /// pays off, while local development is better served by cargo's own
    /// incremental compilation (which sccache would disable). Nothing needs
    /// to be installed: `turbo` embeds sccache and acts as the compiler
    /// wrapper itself.
    #[serde(default)]
    #[schemars(skip)]
    pub experimental_cargo_sccache: bool,
    /// Allow task definitions to declare the command they run via the
    /// `command` field, replacing the toolchain's own resolution
    /// (package.json scripts, Cargo verb tables). Using `command` without
    /// this flag is a hard error — silently ignoring it would change what
    /// executes.
    #[serde(default)]
    #[schemars(skip)]
    pub experimental_task_command: bool,
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
         pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, \
         strictTaskEntrypointSelection?: boolean, globalConfiguration?: boolean, \
         experimentalCargoWorkspaces?: boolean, experimentalTaskCommand?: boolean }"
            .to_string()
    }

    fn inline_flattened() -> String {
        "{ errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, longerSignatureKey?: \
         boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: boolean, \
         pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, \
         strictTaskEntrypointSelection?: boolean, globalConfiguration?: boolean, \
         experimentalCargoWorkspaces?: boolean, experimentalTaskCommand?: boolean }"
            .to_string()
    }

    fn decl() -> String {
        "type FutureFlags = { errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, \
         longerSignatureKey?: boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: \
         boolean, pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, \
         strictTaskEntrypointSelection?: boolean, globalConfiguration?: boolean, \
         experimentalCargoWorkspaces?: boolean, experimentalTaskCommand?: boolean };"
            .to_string()
    }

    fn decl_concrete() -> String {
        "type FutureFlags = { errorsOnlyShowHash?: boolean, experimentalObservability?: boolean, \
         longerSignatureKey?: boolean, affectedUsingTaskInputs?: boolean, watchUsingTaskInputs?: \
         boolean, pruneIncludesGlobalFiles?: boolean, filterUsingTasks?: boolean, \
         strictTaskEntrypointSelection?: boolean, globalConfiguration?: boolean, \
         experimentalCargoWorkspaces?: boolean, experimentalTaskCommand?: boolean };"
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
