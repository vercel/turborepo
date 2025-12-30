//! Raw turbo.json structures for parsing
//!
//! These structures represent the raw parsed form of turbo.json before
//! any processing or validation has been applied.

use std::collections::BTreeMap;

use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;
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

/// Pipeline is a map of task names to their raw definitions
#[derive(Serialize, Default, Debug, PartialEq, Clone)]
#[serde(transparent)]
pub struct Pipeline(pub BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>>);

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

// Iterable is required to enumerate allowed keys
#[derive(Clone, Debug, Default, Iterable, Serialize, Deserializable)]
#[serde(rename_all = "camelCase")]
pub struct RawRemoteCacheOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_slug: Option<Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preflight: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Spanned<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

// Unified structure that represents either root or package turbo.json
#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
pub struct RawTurboJson {
    #[serde(skip)]
    pub span: Spanned<()>,
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<UnescapedString>,
    #[serde(skip_serializing)]
    pub experimental_spaces: Option<SpacesJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Spanned<Vec<UnescapedString>>>,
    // Global root filesystem dependencies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_dependencies: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
    // Tasks is a map of task entries which define the task graph
    // and cache behavior on a per task or per package-task basis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Pipeline>,
    #[serde(skip_serializing)]
    pub pipeline: Option<Spanned<Pipeline>>,
    // Configuration options when interfacing with the remote cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_cache: Option<RawRemoteCacheOptions>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "ui")]
    pub ui: Option<Spanned<UIMode>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "dangerouslyDisablePackageManagerCheck"
    )]
    pub allow_no_package_manager: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_mode: Option<Spanned<EnvMode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<Spanned<UnescapedString>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_update_notifier: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Spanned<Vec<Spanned<String>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundaries: Option<Spanned<BoundariesConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<Spanned<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub future_flags: Option<Spanned<FutureFlags>>,
    #[deserializable(rename = "//")]
    #[serde(skip)]
    pub _comment: Option<String>,
}

#[derive(Serialize, Default, Debug, PartialEq, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
#[deserializable(unknown_fields = "deny")]
pub struct RawTaskDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Spanned<Vec<Spanned<UnescapedString>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistent: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interruptible: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_logs: Option<Spanned<OutputLogsMode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<Spanned<bool>>,
    // TODO: Remove this once we have the ability to load task definitions directly
    // instead of deriving them from a TurboJson
    #[serde(skip)]
    pub env_mode: Option<Spanned<EnvMode>>,
    // This can currently only be set internally and isn't a part of turbo.json
    #[serde(skip_serializing_if = "Option::is_none")]
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
