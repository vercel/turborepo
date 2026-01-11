//! Raw turbo.json structures for parsing
//!
//! These structures represent the raw parsed form of turbo.json before
//! any processing or validation has been applied.

use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use schemars::JsonSchema;
use serde::Serialize;
use struct_iterable::Iterable;
use ts_rs::TS;
use turbopath::AbsoluteSystemPath;
use turborepo_boundaries::BoundariesConfig;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::ROOT_PKG_NAME;
use turborepo_task_id::TaskName;
use turborepo_types::{EnvMode, OutputLogsMode, UIMode};
use turborepo_unescape::UnescapedString;

use crate::{error::Error, future_flags::FutureFlags};

// Forward declarations for types that will be moved later
// For now we define minimal versions here

/// Spaces configuration (experimental)
#[derive(Serialize, Debug, Default, PartialEq, Clone, Deserializable)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<UnescapedString>,
}

/// An object representing the task dependency graph of your project.
///
/// turbo interprets these conventions to schedule, execute, and cache the
/// outputs of tasks in your project.
///
/// Documentation: https://turborepo.com/docs/reference/configuration#tasks
#[derive(Serialize, Default, Debug, PartialEq, Clone, JsonSchema)]
#[serde(transparent)]
pub struct Pipeline(pub BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>>);

/// Custom TS implementation for Pipeline to generate Record<string, Pipeline>
/// where "Pipeline" here refers to RawTaskDefinition (the task config).
/// This is needed because Pipeline is a transparent wrapper around BTreeMap,
/// and we need to produce a TypeScript indexed object type.
impl TS for Pipeline {
    type WithoutGenerics = Self;

    fn name() -> String {
        // Don't emit a named type - inline it instead
        String::new()
    }

    fn inline() -> String {
        // Inline as an indexed object type: { [script: string]: Pipeline }
        // Note: "Pipeline" in TS refers to RawTaskDefinition due to the rename
        "{ [script: string]: Pipeline }".to_string()
    }

    fn inline_flattened() -> String {
        Self::inline()
    }

    fn decl() -> String {
        // No separate declaration needed - it's inlined
        String::new()
    }

    fn decl_concrete() -> String {
        String::new()
    }

    fn dependencies() -> Vec<ts_rs::Dependency> {
        // Depends on RawTaskDefinition (exported as "Pipeline")
        vec![]
    }
}

impl Pipeline {
    pub fn insert(&mut self, key: TaskName<'static>, value: Spanned<RawTaskDefinition>) {
        self.0.insert(key, value);
    }
}

impl std::ops::Deref for Pipeline {
    type Target = BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Pipeline {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl IntoIterator for Pipeline {
    type Item = (TaskName<'static>, Spanned<RawTaskDefinition>);
    type IntoIter =
        <BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Trait to check if a task definition has any configuration beyond just the
/// `extends` field. This is used to determine if a task definition with
/// `extends: false` should actually skip inheritance or if it's just an
/// empty marker.
pub trait HasConfigBeyondExtends {
    fn has_config_beyond_extends(&self) -> bool;
}

/// Configuration options that control how turbo interfaces with the remote
/// cache.
///
/// Documentation: https://turborepo.com/docs/core-concepts/remote-caching
#[derive(Clone, Debug, Default, Iterable, Serialize, Deserializable, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "RemoteCache")]
#[ts(export, rename = "RemoteCache")]
pub struct RawRemoteCacheOptions {
    /// Set endpoint for API calls to the remote cache.
    ///
    /// Documentation: https://turborepo.com/docs/core-concepts/remote-caching#self-hosting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<Spanned<String>>,

    /// Set endpoint for requesting tokens during `turbo login`.
    ///
    /// Documentation: https://turborepo.com/docs/core-concepts/remote-caching#self-hosting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<Spanned<String>>,

    /// The slug of the Remote Cache team.
    ///
    /// Value will be passed as `slug` in the querystring for all Remote
    /// Cache HTTP calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_slug: Option<Spanned<String>>,

    /// The ID of the Remote Cache team.
    ///
    /// Value will be passed as `teamId` in the querystring for all Remote
    /// Cache HTTP calls. Must start with `team_` or it will not be used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<Spanned<String>>,

    /// Indicates if signature verification is enabled for requests to the
    /// remote cache.
    ///
    /// When `true`, Turborepo will sign every uploaded artifact using the
    /// value of the environment variable `TURBO_REMOTE_CACHE_SIGNATURE_KEY`.
    /// Turborepo will reject any downloaded artifacts that have an invalid
    /// signature or are missing a signature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Spanned<bool>>,

    /// When enabled, any HTTP request will be preceded by an OPTIONS request
    /// to determine if the request is supported by the endpoint.
    ///
    /// Documentation: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS#preflighted_requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preflight: Option<Spanned<bool>>,

    /// Sets a timeout for remote cache operations.
    ///
    /// Value is given in seconds and only whole values are accepted.
    /// If `0` is passed, then there is no timeout for any cache operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub timeout: Option<Spanned<u64>>,

    /// Indicates if the remote cache is enabled.
    ///
    /// When `false`, Turborepo will disable all remote cache operations,
    /// even if the repo has a valid token. If `true`, remote caching is
    /// enabled, but still requires the user to login and link their repo
    /// to a remote cache.
    ///
    /// Documentation: https://turborepo.com/docs/core-concepts/remote-caching
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Spanned<bool>>,

    /// Sets a timeout for remote cache uploads.
    ///
    /// Value is given in seconds and only whole values are accepted.
    /// If `0` is passed, then there is no timeout for any remote cache uploads.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub upload_timeout: Option<Spanned<u64>>,
}

// Root turbo.json
#[derive(Default, Debug, Clone, Iterable, Deserializable)]
pub struct RawRootTurboJson {
    pub span: Spanned<()>,

    #[deserializable(rename = "$schema")]
    pub schema: Option<UnescapedString>,
    pub experimental_spaces: Option<SpacesJson>,

    // Global root filesystem dependencies
    pub global_dependencies: Option<Vec<Spanned<UnescapedString>>>,
    pub global_env: Option<Vec<Spanned<UnescapedString>>>,
    pub global_pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
    // Tasks is a map of task entries which define the task graph
    // and cache behavior on a per task or per package-task basis.
    pub tasks: Option<Pipeline>,
    pub pipeline: Option<Spanned<Pipeline>>,
    // Configuration options when interfacing with the remote cache
    pub remote_cache: Option<RawRemoteCacheOptions>,
    pub ui: Option<Spanned<UIMode>>,
    #[deserializable(rename = "dangerouslyDisablePackageManagerCheck")]
    pub allow_no_package_manager: Option<Spanned<bool>>,
    pub daemon: Option<Spanned<bool>>,
    pub env_mode: Option<Spanned<EnvMode>>,
    pub no_update_notifier: Option<Spanned<bool>>,
    pub cache_dir: Option<Spanned<UnescapedString>>,
    pub concurrency: Option<Spanned<String>>,
    pub tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub boundaries: Option<Spanned<BoundariesConfig>>,

    pub future_flags: Option<Spanned<FutureFlags>>,
    #[deserializable(rename = "//")]
    pub _comment: Option<String>,
}

// Package turbo.json
#[derive(Default, Debug, Clone, Iterable, Deserializable)]
pub struct RawPackageTurboJson {
    pub span: Spanned<()>,
    #[deserializable(rename = "$schema")]
    pub schema: Option<UnescapedString>,
    pub extends: Option<Spanned<Vec<UnescapedString>>>,
    pub tasks: Option<Pipeline>,
    pub pipeline: Option<Spanned<Pipeline>>,
    pub tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub boundaries: Option<Spanned<BoundariesConfig>>,
    #[deserializable(rename = "//")]
    pub _comment: Option<String>,
}

/// Configuration schema for turbo.json.
///
/// An object representing the task dependency graph of your project.
/// turbo interprets these conventions to schedule, execute, and cache
/// the outputs of tasks in your project.
///
/// Documentation: https://turborepo.com/docs/reference/configuration
#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RawTurboJson {
    // Internal field - excluded from schema
    #[serde(skip)]
    #[schemars(skip)]
    #[ts(skip)]
    pub span: Spanned<()>,

    /// JSON Schema URL for validation.
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    #[ts(rename = "$schema")]
    pub schema: Option<UnescapedString>,

    // Internal field - excluded from schema
    #[serde(skip_serializing)]
    #[schemars(skip)]
    #[ts(skip)]
    pub experimental_spaces: Option<SpacesJson>,

    /// This key is only available in Workspace Configs and cannot be used in
    /// your root turbo.json.
    ///
    /// Tells turbo to extend your root `turbo.json` and overrides with the
    /// keys provided in your Workspace Configs. Currently, only the `["//"]`
    /// value is allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Spanned<Vec<UnescapedString>>>,

    /// A list of globs to include in the set of implicit global hash
    /// dependencies.
    ///
    /// The contents of these files will be included in the global hashing
    /// algorithm and affect the hashes of all tasks.
    ///
    /// This is useful for busting the cache based on:
    /// - `.env` files (not in Git)
    /// - Any root level file that impacts package tasks that are not
    ///   represented in the traditional dependency graph (e.g. a root
    ///   `tsconfig.json`, `jest.config.ts`, `.eslintrc`, etc.)
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#globaldependencies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_dependencies: Option<Vec<Spanned<UnescapedString>>>,

    /// A list of environment variables for implicit global hash dependencies.
    ///
    /// The variables included in this list will affect all task hashes.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#globalenv
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_env: Option<Vec<Spanned<UnescapedString>>>,

    /// An allowlist of environment variables that should be made to all tasks,
    /// but should not contribute to the task's cache key, e.g.
    /// `AWS_SECRET_KEY`.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#globalpassthroughenv
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_pass_through_env: Option<Vec<Spanned<UnescapedString>>>,

    /// An object representing the task dependency graph of your project.
    ///
    /// turbo interprets these conventions to schedule, execute, and cache the
    /// outputs of tasks in your project.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Pipeline>,

    // Deprecated field - excluded from schema
    #[serde(skip_serializing)]
    #[schemars(skip)]
    #[ts(skip)]
    pub pipeline: Option<Spanned<Pipeline>>,

    /// Configuration options when interfacing with the remote cache.
    ///
    /// Documentation: https://turborepo.com/docs/core-concepts/remote-caching
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_cache: Option<RawRemoteCacheOptions>,

    /// Enable use of the UI for `turbo`.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#ui
    #[serde(skip_serializing_if = "Option::is_none", rename = "ui")]
    pub ui: Option<Spanned<UIMode>>,

    /// Disable check for `packageManager` in root `package.json`.
    ///
    /// This is highly discouraged as it leaves `turbo` dependent on system
    /// configuration to infer the correct package manager. Some turbo features
    /// are disabled if this is set to true.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "dangerouslyDisablePackageManagerCheck"
    )]
    #[ts(rename = "dangerouslyDisablePackageManagerCheck")]
    pub allow_no_package_manager: Option<Spanned<bool>>,

    /// Turborepo runs a background process to pre-calculate some expensive
    /// operations. This standalone process (daemon) is a performance
    /// optimization, and not required for proper functioning of `turbo`.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#daemon
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon: Option<Spanned<bool>>,

    /// Turborepo's Environment Modes allow you to control which environment
    /// variables are available to a task at runtime.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#envmode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_mode: Option<Spanned<EnvMode>>,

    /// Specify the filesystem cache directory.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#cachedir
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<Spanned<UnescapedString>>,

    /// When set to `true`, disables the update notification that appears when
    /// a new version of `turbo` is available.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#noupdatenotifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_update_notifier: Option<Spanned<bool>>,

    /// Used to tag a package for boundaries rules.
    ///
    /// Boundaries rules can restrict which packages a tag group can import
    /// or be imported by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Spanned<Vec<Spanned<String>>>>,

    /// Configuration for `turbo boundaries`.
    ///
    /// Allows users to restrict a package's dependencies and dependents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundaries: Option<Spanned<BoundariesConfig>>,

    /// Set/limit the maximum concurrency for task execution.
    ///
    /// Must be an integer greater than or equal to `1` or a percentage value
    /// like `50%`. Use `1` to force serial execution (one task at a time).
    /// Use `100%` to use all available logical processors.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#concurrency
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<Spanned<String>>,

    /// Opt into breaking changes prior to major releases, experimental
    /// features, and beta features.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub future_flags: Option<Spanned<FutureFlags>>,

    // Internal field - excluded from schema
    #[deserializable(rename = "//")]
    #[serde(skip)]
    #[schemars(skip)]
    #[ts(skip)]
    pub _comment: Option<String>,
}

/// Configuration for a pipeline task.
///
/// The name of a task that can be executed by turbo. If turbo finds a
/// workspace package with a `package.json` scripts object with a matching
/// key, it will apply the pipeline task configuration to that npm script
/// during execution.
#[derive(Serialize, Default, Debug, PartialEq, Clone, Iterable, Deserializable, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[schemars(rename = "Pipeline")]
#[ts(export, rename = "Pipeline")]
#[deserializable(unknown_fields = "deny")]
pub struct RawTaskDefinition {
    // Internal field - excluded from schema
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    #[ts(skip)]
    pub extends: Option<Spanned<bool>>,

    /// Whether or not to cache the outputs of the task.
    ///
    /// Setting cache to false is useful for long-running "watch" or
    /// development mode tasks.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<Spanned<bool>>,

    /// The list of tasks that this task depends on.
    ///
    /// Prefixing an item in `dependsOn` with a `^` prefix tells turbo that
    /// this task depends on the package's topological dependencies completing
    /// the task first. Items without a `^` prefix express the relationships
    /// between tasks within the same package.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#dependson
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Spanned<Vec<Spanned<UnescapedString>>>>,

    /// A list of environment variables that this task depends on.
    ///
    /// Note: If you are migrating from a turbo version 1.5 or below, you may
    /// be used to prefixing your variables with a `$`. You no longer need to
    /// use the `$` prefix.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#env
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<Spanned<UnescapedString>>>,

    /// The set of glob patterns to consider as inputs to this task.
    ///
    /// Changes to files covered by these globs will cause a cache miss and
    /// the task will be rerun. If a file has been changed that is **not**
    /// included in the set of globs, it will not cause a cache miss.
    /// If omitted or empty, all files in the package are considered as inputs.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#inputs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<Spanned<UnescapedString>>>,

    /// An allowlist of environment variables that should be made available
    /// in this task's environment, but should not contribute to the task's
    /// cache key, e.g. `AWS_SECRET_KEY`.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#passthroughenv
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass_through_env: Option<Vec<Spanned<UnescapedString>>>,

    /// Indicates whether the task exits or not.
    ///
    /// Setting `persistent` to `true` tells turbo that this is a long-running
    /// task and will ensure that other tasks cannot depend on it.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#persistent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistent: Option<Spanned<bool>>,

    /// Label a persistent task as interruptible to allow it to be restarted
    /// by `turbo watch`.
    ///
    /// `turbo watch` watches for changes to your packages and automatically
    /// restarts tasks that are affected. However, if a task is persistent,
    /// it will not be restarted by default. To enable restarting persistent
    /// tasks, set `interruptible` to `true`.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#interruptible
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interruptible: Option<Spanned<bool>>,

    /// The set of glob patterns indicating a task's cacheable filesystem
    /// outputs.
    ///
    /// Turborepo captures task logs for all tasks. This enables us to cache
    /// tasks whose runs produce no artifacts other than logs (such as linters).
    /// Logs are always treated as a cacheable artifact and never need to be
    /// specified.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#outputs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Spanned<UnescapedString>>>,

    /// Output mode for the task.
    ///
    /// Documentation: https://turborepo.com/docs/reference/run#--output-logs-option
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_logs: Option<Spanned<OutputLogsMode>>,

    /// Mark a task as interactive allowing it to receive input from stdin.
    ///
    /// Interactive tasks must be marked with `"cache": false` as the input
    /// they receive from stdin can change the outcome of the task.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#interactive
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<Spanned<bool>>,

    // Internal field - excluded from schema
    #[serde(skip)]
    #[schemars(skip)]
    #[ts(skip)]
    pub env_mode: Option<Spanned<EnvMode>>,

    /// A list of tasks that will run alongside this task.
    ///
    /// Tasks in this list will not be run until completion before this task
    /// starts execution.
    ///
    /// Documentation: https://turborepo.com/docs/reference/configuration#with
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "with")]
    pub with: Option<Vec<Spanned<UnescapedString>>>,
}

impl HasConfigBeyondExtends for RawTaskDefinition {
    fn has_config_beyond_extends(&self) -> bool {
        self.cache.is_some()
            || self.depends_on.is_some()
            || self.env.is_some()
            || self.inputs.is_some()
            || self.pass_through_env.is_some()
            || self.persistent.is_some()
            || self.interruptible.is_some()
            || self.outputs.is_some()
            || self.output_logs.is_some()
            || self.interactive.is_some()
            || self.with.is_some()
    }
}

impl From<RawRootTurboJson> for RawTurboJson {
    fn from(root: RawRootTurboJson) -> Self {
        RawTurboJson {
            span: root.span,
            schema: root.schema,
            experimental_spaces: root.experimental_spaces,
            global_dependencies: root.global_dependencies,
            global_env: root.global_env,
            global_pass_through_env: root.global_pass_through_env,
            tasks: root.tasks,
            pipeline: root.pipeline,
            remote_cache: root.remote_cache,
            ui: root.ui,
            allow_no_package_manager: root.allow_no_package_manager,
            daemon: root.daemon,
            env_mode: root.env_mode,
            cache_dir: root.cache_dir,
            no_update_notifier: root.no_update_notifier,
            tags: root.tags,
            boundaries: root.boundaries,
            concurrency: root.concurrency,
            future_flags: root.future_flags,
            _comment: root._comment,
            extends: None, // Root configs never have extends
        }
    }
}

impl From<RawPackageTurboJson> for RawTurboJson {
    fn from(pkg: RawPackageTurboJson) -> Self {
        RawTurboJson {
            span: pkg.span,
            schema: pkg.schema,
            extends: pkg.extends,
            tasks: pkg.tasks,
            pipeline: pkg.pipeline,
            boundaries: pkg.boundaries,
            tags: pkg.tags,
            _comment: pkg._comment,
            ..Default::default()
        }
    }
}

impl RawTurboJson {
    pub fn read(
        repo_root: &AbsoluteSystemPath,
        path: &AbsoluteSystemPath,
        is_root: bool,
    ) -> Result<Option<RawTurboJson>, Error> {
        let Some(contents) = path.read_existing_to_string()? else {
            return Ok(None);
        };

        // Anchoring the path can fail if the path resides outside of the repository
        // Just display absolute path in that case.
        let root_relative_path = repo_root.anchor(path).map_or_else(
            |_| path.as_str().to_owned(),
            |relative| relative.to_string(),
        );

        Ok(Some(if is_root {
            RawTurboJson::from(RawRootTurboJson::parse(&contents, &root_relative_path)?)
        } else {
            RawTurboJson::from(RawPackageTurboJson::parse(&contents, &root_relative_path)?)
        }))
    }

    /// Produces a new turbo.json without any tasks that reference non-existent
    /// workspaces
    pub fn prune_tasks<S: AsRef<str>>(&self, workspaces: &[S]) -> Self {
        let mut this = self.clone();
        if let Some(pipeline) = &mut this.tasks {
            pipeline.0.retain(|task_name, _| {
                task_name.in_workspace(ROOT_PKG_NAME)
                    || workspaces
                        .iter()
                        .any(|workspace| task_name.in_workspace(workspace.as_ref()))
            })
        }

        this
    }

    // NOTE: This method depends on turborepo-lib types (TaskAccessTraceFile)
    // and has been commented out for now. It will be re-enabled when the
    // dependency structure is resolved.
    //
    // pub fn from_task_access_trace(trace: &HashMap<String, TaskAccessTraceFile>)
    // -> Option<Self> {     if trace.is_empty() {
    //         return None;
    //     }
    //
    //     let mut pipeline = Pipeline::default();
    //
    //     for (task_name, trace_file) in trace {
    //         let spanned_outputs: Vec<Spanned<UnescapedString>> = trace_file
    //             .outputs
    //             .iter()
    //             .map(|output| Spanned::new(output.clone()))
    //             .collect();
    //         let task_definition = RawTaskDefinition {
    //             outputs: Some(spanned_outputs),
    //             env: Some(
    //                 trace_file
    //                     .accessed
    //                     .env_var_keys
    //                     .iter()
    //                     .map(|unescaped_string|
    // Spanned::new(unescaped_string.clone()))                     .collect(),
    //             ),
    //             ..Default::default()
    //         };
    //
    //         let name = TaskName::from(task_name.as_str());
    //         let root_task = name.into_root_task();
    //         pipeline.insert(root_task, Spanned::new(task_definition.clone()));
    //     }
    //
    //     Some(RawTurboJson {
    //         tasks: Some(pipeline),
    //         ..RawTurboJson::default()
    //     })
    // }
}
