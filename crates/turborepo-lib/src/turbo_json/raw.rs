use std::collections::HashMap;

use biome_deserialize_macros::Deserializable;
use serde::Serialize;
use struct_iterable::Iterable;
use turbopath::AbsoluteSystemPath;
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::ROOT_PKG_NAME;
use turborepo_task_id::TaskName;
use turborepo_unescape::UnescapedString;

use super::{FutureFlags, Pipeline, SpacesJson, UIMode};
use crate::{
    boundaries::BoundariesConfig,
    cli::{EnvMode, OutputLogsMode},
    config::Error,
    run::task_access::TaskAccessTraceFile,
};

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
    pub(crate) span: Spanned<()>,

    #[deserializable(rename = "$schema")]
    pub(crate) schema: Option<UnescapedString>,
    pub(crate) experimental_spaces: Option<SpacesJson>,

    // Global root filesystem dependencies
    pub(crate) global_dependencies: Option<Vec<Spanned<UnescapedString>>>,
    pub(crate) global_env: Option<Vec<Spanned<UnescapedString>>>,
    pub(crate) global_pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
    // Tasks is a map of task entries which define the task graph
    // and cache behavior on a per task or per package-task basis.
    pub(crate) tasks: Option<Pipeline>,
    pub(crate) pipeline: Option<Spanned<Pipeline>>,
    // Configuration options when interfacing with the remote cache
    pub(crate) remote_cache: Option<RawRemoteCacheOptions>,
    pub(crate) ui: Option<Spanned<UIMode>>,
    #[deserializable(rename = "dangerouslyDisablePackageManagerCheck")]
    pub(crate) allow_no_package_manager: Option<Spanned<bool>>,
    pub(crate) daemon: Option<Spanned<bool>>,
    pub(crate) env_mode: Option<Spanned<EnvMode>>,
    pub(crate) no_update_notifier: Option<Spanned<bool>>,
    pub(crate) cache_dir: Option<Spanned<UnescapedString>>,
    pub(crate) concurrency: Option<Spanned<String>>,
    pub(crate) tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub(crate) boundaries: Option<Spanned<BoundariesConfig>>,

    pub(crate) future_flags: Option<Spanned<FutureFlags>>,
    #[deserializable(rename = "//")]
    pub(crate) _comment: Option<String>,
}

// Package turbo.json
#[derive(Default, Debug, Clone, Iterable, Deserializable)]
pub struct RawPackageTurboJson {
    pub(crate) span: Spanned<()>,
    #[deserializable(rename = "$schema")]
    pub(crate) schema: Option<UnescapedString>,
    pub(crate) extends: Option<Spanned<Vec<UnescapedString>>>,
    pub(crate) tasks: Option<Pipeline>,
    pub(crate) pipeline: Option<Spanned<Pipeline>>,
    pub(crate) tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub(crate) boundaries: Option<Spanned<BoundariesConfig>>,
    #[deserializable(rename = "//")]
    pub(crate) _comment: Option<String>,
}

// Unified structure that represents either root or package turbo.json
#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
pub struct RawTurboJson {
    #[serde(skip)]
    pub(crate) span: Spanned<()>,
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub(crate) schema: Option<UnescapedString>,
    #[serde(skip_serializing)]
    pub experimental_spaces: Option<SpacesJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extends: Option<Spanned<Vec<UnescapedString>>>,
    // Global root filesystem dependencies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) global_dependencies: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) global_env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) global_pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
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
    pub(crate) _comment: Option<String>,
}

#[derive(Serialize, Default, Debug, PartialEq, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
#[deserializable(unknown_fields = "deny")]
pub struct RawTaskDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) depends_on: Option<Spanned<Vec<Spanned<UnescapedString>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) inputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) persistent: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interruptible: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) outputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_logs: Option<Spanned<OutputLogsMode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interactive: Option<Spanned<bool>>,
    // TODO: Remove this once we have the ability to load task definitions directly
    // instead of deriving them from a TurboJson
    #[serde(skip)]
    pub(crate) env_mode: Option<Spanned<EnvMode>>,
    // This can currently only be set internally and isn't a part of turbo.json
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) with: Option<Vec<Spanned<UnescapedString>>>,
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
    pub(super) fn read(
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

    pub fn from_task_access_trace(trace: &HashMap<String, TaskAccessTraceFile>) -> Option<Self> {
        if trace.is_empty() {
            return None;
        }

        let mut pipeline = Pipeline::default();

        for (task_name, trace_file) in trace {
            let spanned_outputs: Vec<Spanned<UnescapedString>> = trace_file
                .outputs
                .iter()
                .map(|output| Spanned::new(output.clone()))
                .collect();
            let task_definition = RawTaskDefinition {
                outputs: Some(spanned_outputs),
                env: Some(
                    trace_file
                        .accessed
                        .env_var_keys
                        .iter()
                        .map(|unescaped_string| Spanned::new(unescaped_string.clone()))
                        .collect(),
                ),
                ..Default::default()
            };

            let name = TaskName::from(task_name.as_str());
            let root_task = name.into_root_task();
            pipeline.insert(root_task, Spanned::new(task_definition.clone()));
        }

        Some(RawTurboJson {
            tasks: Some(pipeline),
            ..RawTurboJson::default()
        })
    }
}
