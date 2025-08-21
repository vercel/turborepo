use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Display,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use biome_deserialize_macros::Deserializable;
use camino::Utf8Path;
use clap::ValueEnum;
use miette::{NamedSource, SourceSpan};
use serde::{Deserialize, Serialize};
use struct_iterable::Iterable;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};
use turborepo_errors::Spanned;
use turborepo_repository::package_graph::ROOT_PKG_NAME;
use turborepo_task_id::{TaskId, TaskName};
use turborepo_unescape::UnescapedString;

use crate::{
    cli::{EnvMode, OutputLogsMode},
    config::{Error, InvalidEnvPrefixError},
    run::task_access::TaskAccessTraceFile,
    task_graph::{TaskDefinition, TaskInputs, TaskOutputs},
};

mod extend;
pub mod future_flags;
mod loader;
pub mod parser;
mod processed;

pub use future_flags::FutureFlags;
pub use loader::{TurboJsonLoader, TurboJsonReader};
pub use processed::ProcessedTaskDefinition;

use crate::{boundaries::BoundariesConfig, config::UnnecessaryPackageTaskSyntaxError};

const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone, Deserializable)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<UnescapedString>,
}

// A turbo.json config that is synthesized but not yet resolved.
// This means that we've done the work to synthesize the config from
// package.json, but we haven't yet resolved the workspace
// turbo.json files into a single definition. Therefore we keep the
// `RawTaskDefinition` type so we can determine which fields are actually
// set when we resolve the configuration.
//
// Note that the values here are limited to pipeline configuration.
// Configuration that needs to account for flags, env vars, etc. is
// handled via layered config.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TurboJson {
    text: Option<Arc<str>>,
    path: Option<Arc<str>>,
    pub(crate) tags: Option<Spanned<Vec<Spanned<String>>>>,
    pub(crate) boundaries: Option<Spanned<BoundariesConfig>>,
    pub(crate) extends: Spanned<Vec<String>>,
    pub(crate) global_deps: Vec<String>,
    pub(crate) global_env: Vec<String>,
    pub(crate) global_pass_through_env: Option<Vec<String>>,
    pub(crate) tasks: Pipeline,
    pub(crate) future_flags: FutureFlags,
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

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
// The raw deserialized turbo.json file.
pub struct RawTurboJson {
    #[serde(skip)]
    span: Spanned<()>,

    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<UnescapedString>,

    #[serde(skip_serializing)]
    pub experimental_spaces: Option<SpacesJson>,

    #[serde(skip_serializing_if = "Option::is_none")]
    extends: Option<Spanned<Vec<UnescapedString>>>,
    // Global root filesystem dependencies
    #[serde(skip_serializing_if = "Option::is_none")]
    global_dependencies: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    global_env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    global_pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
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
    _comment: Option<String>,
}

#[derive(Serialize, Default, Debug, PartialEq, Clone)]
#[serde(transparent)]
pub struct Pipeline(BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>>);

impl IntoIterator for Pipeline {
    type Item = (TaskName<'static>, Spanned<RawTaskDefinition>);
    type IntoIter =
        <BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Deref for Pipeline {
    type Target = BTreeMap<TaskName<'static>, Spanned<RawTaskDefinition>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Pipeline {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Deserializable, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "camelCase")]
pub enum UIMode {
    /// Use the terminal user interface
    Tui,
    /// Use the standard output stream
    Stream,
    /// Use the web user interface (experimental)
    Web,
}

impl Default for UIMode {
    fn default() -> Self {
        Self::Tui
    }
}

impl Display for UIMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UIMode::Tui => write!(f, "tui"),
            UIMode::Stream => write!(f, "stream"),
            UIMode::Web => write!(f, "web"),
        }
    }
}

impl UIMode {
    pub fn use_tui(&self) -> bool {
        matches!(self, Self::Tui)
    }

    /// Returns true if the UI mode has a sender,
    /// i.e. web or tui but not stream
    pub fn has_sender(&self) -> bool {
        matches!(self, Self::Tui | Self::Web)
    }
}

#[derive(Serialize, Default, Debug, PartialEq, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
#[deserializable(unknown_fields = "deny")]
pub struct RawTaskDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    cache: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depends_on: Option<Spanned<Vec<Spanned<UnescapedString>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pass_through_env: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    persistent: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interruptible: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_logs: Option<Spanned<OutputLogsMode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interactive: Option<Spanned<bool>>,
    // TODO: Remove this once we have the ability to load task definitions directly
    // instead of deriving them from a TurboJson
    #[serde(skip)]
    env_mode: Option<Spanned<EnvMode>>,
    // This can currently only be set internally and isn't a part of turbo.json
    #[serde(skip_serializing_if = "Option::is_none")]
    with: Option<Vec<Spanned<UnescapedString>>>,
}

impl TaskOutputs {
    /// Creates TaskOutputs from ProcessedOutputs with resolved paths
    fn from_processed(
        outputs: processed::ProcessedOutputs,
        turbo_root_path: &RelativeUnixPath,
    ) -> Result<Self, Error> {
        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();

        // Resolve all globs with the turbo_root path
        // Absolute path validation was already done during ProcessedGlob creation
        let resolved = outputs.resolve(turbo_root_path);

        for glob_str in resolved {
            if let Some(stripped_glob) = glob_str.strip_prefix('!') {
                exclusions.push(stripped_glob.to_string());
            } else {
                inclusions.push(glob_str);
            }
        }

        inclusions.sort();
        exclusions.sort();

        Ok(TaskOutputs {
            inclusions,
            exclusions,
        })
    }
}

impl TaskInputs {
    /// Creates TaskInputs from ProcessedInputs with resolved paths
    fn from_processed(
        inputs: processed::ProcessedInputs,
        turbo_root_path: &RelativeUnixPath,
    ) -> Result<Self, Error> {
        // Resolve all globs with the turbo_root path
        // Absolute path validation was already done during ProcessedGlob creation
        Ok(TaskInputs {
            globs: inputs.resolve(turbo_root_path),
            default: inputs.default,
        })
    }
}

impl TaskDefinition {
    /// Creates a TaskDefinition from a ProcessedTaskDefinition
    pub fn from_processed(
        processed: ProcessedTaskDefinition,
        path_to_repo_root: &RelativeUnixPath,
    ) -> Result<Self, Error> {
        // Convert outputs with turbo_root resolution
        let outputs = processed
            .outputs
            .map(|outputs| TaskOutputs::from_processed(outputs, path_to_repo_root))
            .transpose()?
            .unwrap_or_default();

        let cache = processed.cache.is_none_or(|c| c.into_inner());
        let interactive = processed
            .interactive
            .as_ref()
            .map(|value| value.value)
            .unwrap_or_default();

        if let Some(interactive) = &processed.interactive {
            let (span, text) = interactive.span_and_text("turbo.json");
            if cache && interactive.value {
                return Err(Error::InteractiveNoCacheable { span, text });
            }
        }

        let persistent = *processed.persistent.unwrap_or_default();
        let interruptible = processed.interruptible.unwrap_or_default();
        if *interruptible && !persistent {
            let (span, text) = interruptible.span_and_text("turbo.json");
            return Err(Error::InterruptibleButNotPersistent { span, text });
        }

        let mut topological_dependencies: Vec<Spanned<TaskName>> = Vec::new();
        let mut task_dependencies: Vec<Spanned<TaskName>> = Vec::new();
        if let Some(depends_on) = processed.depends_on {
            for dependency in depends_on.deps {
                let (dependency, depspan) = dependency.split();
                let dependency: String = dependency.into();
                if let Some(topo_dependency) =
                    dependency.strip_prefix(TOPOLOGICAL_PIPELINE_DELIMITER)
                {
                    topological_dependencies.push(depspan.to(topo_dependency.to_string().into()));
                } else {
                    task_dependencies.push(depspan.to(dependency.into()));
                }
            }
        }

        task_dependencies.sort_by(|a, b| a.value.cmp(&b.value));
        topological_dependencies.sort_by(|a, b| a.value.cmp(&b.value));

        let env = processed.env.map(|env| env.vars).unwrap_or_default();

        // Convert inputs with turbo_root resolution
        let inputs = processed
            .inputs
            .map(|inputs| TaskInputs::from_processed(inputs, path_to_repo_root))
            .transpose()?
            .unwrap_or_default();

        let pass_through_env = processed.pass_through_env.map(|env| env.vars);

        let with = processed.with.map(|with_tasks| with_tasks.tasks);

        Ok(TaskDefinition {
            outputs,
            cache,
            topological_dependencies,
            task_dependencies,
            env,
            inputs,
            pass_through_env,
            output_logs: *processed.output_logs.unwrap_or_default(),
            persistent,
            interruptible: *interruptible,
            interactive,
            env_mode: processed.env_mode.map(|mode| *mode.as_inner()),
            with,
        })
    }

    /// Helper method for tests that still use RawTaskDefinition
    #[cfg(test)]
    fn from_raw(
        raw_task: RawTaskDefinition,
        path_to_repo_root: &RelativeUnixPath,
    ) -> Result<Self, Error> {
        // Use default FutureFlags for backward compatibility
        let processed = ProcessedTaskDefinition::from_raw(raw_task, &FutureFlags::default())?;
        Self::from_processed(processed, path_to_repo_root)
    }
}

impl RawTurboJson {
    pub(crate) fn read(
        repo_root: &AbsoluteSystemPath,
        path: &AbsoluteSystemPath,
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
        let raw_turbo_json = RawTurboJson::parse(&contents, &root_relative_path)?;

        Ok(Some(raw_turbo_json))
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

impl TryFrom<RawTurboJson> for TurboJson {
    type Error = Error;

    fn try_from(raw_turbo: RawTurboJson) -> Result<Self, Error> {
        if let Some(pipeline) = raw_turbo.pipeline {
            let (span, text) = pipeline.span_and_text("turbo.json");
            return Err(Error::PipelineField { span, text });
        }

        // `futureFlags` key is only allowed in root turbo.json
        let is_workspace_config = raw_turbo.extends.is_some();
        if is_workspace_config {
            if let Some(future_flags) = raw_turbo.future_flags {
                let (span, text) = future_flags.span_and_text("turbo.json");
                return Err(Error::FutureFlagsInPackage { span, text });
            }
        }
        let mut global_env = HashSet::new();
        let mut global_file_dependencies = HashSet::new();

        if let Some(global_env_from_turbo) = raw_turbo.global_env {
            gather_env_vars(global_env_from_turbo, "globalEnv", &mut global_env)?;
        }

        for global_dep in raw_turbo.global_dependencies.into_iter().flatten() {
            if global_dep.strip_prefix(ENV_PIPELINE_DELIMITER).is_some() {
                let (span, text) = global_dep.span_and_text("turbo.json");
                return Err(Error::InvalidDependsOnValue {
                    field: "globalDependencies",
                    span,
                    text,
                });
            } else if Utf8Path::new(&global_dep.value).is_absolute() {
                let (span, text) = global_dep.span_and_text("turbo.json");
                return Err(Error::AbsolutePathInConfig {
                    field: "globalDependencies",
                    span,
                    text,
                });
            } else {
                global_file_dependencies.insert(global_dep.into_inner().into());
            }
        }

        let tasks = raw_turbo.tasks.clone().unwrap_or_default();

        Ok(TurboJson {
            text: raw_turbo.span.text,
            path: raw_turbo.span.path,
            tags: raw_turbo.tags,
            global_env: {
                let mut global_env: Vec<_> = global_env.into_iter().collect();
                global_env.sort();
                global_env
            },
            global_pass_through_env: raw_turbo
                .global_pass_through_env
                .map(|env| -> Result<Vec<String>, Error> {
                    let mut global_pass_through_env = HashSet::new();
                    gather_env_vars(env, "globalPassThroughEnv", &mut global_pass_through_env)?;
                    let mut global_pass_through_env: Vec<String> =
                        global_pass_through_env.into_iter().collect();
                    global_pass_through_env.sort();
                    Ok(global_pass_through_env)
                })
                .transpose()?,
            global_deps: {
                let mut global_deps: Vec<_> = global_file_dependencies.into_iter().collect();
                global_deps.sort();

                global_deps
            },
            tasks,
            // copy these over, we don't need any changes here.
            extends: raw_turbo
                .extends
                .unwrap_or_default()
                .map(|s| s.into_iter().map(|s| s.into()).collect()),
            boundaries: raw_turbo.boundaries,
            future_flags: raw_turbo
                .future_flags
                .map(|f| f.into_inner())
                .unwrap_or_default(),
            // Remote Cache config is handled through layered config
        })
    }
}

impl TurboJson {
    fn has_task(&self, task_name: &TaskName) -> bool {
        for key in self.tasks.keys() {
            if key == task_name || (key.task() == task_name.task() && !task_name.is_package_task())
            {
                return true;
            }
        }

        false
    }

    /// Reads a `RawTurboJson` from the given path
    /// and then converts it into `TurboJson`
    ///
    /// Should never be called directly outside of this module.
    /// `TurboJsonReader` should be used instead.
    fn read(
        repo_root: &AbsoluteSystemPath,
        path: &AbsoluteSystemPath,
        future_flags: FutureFlags,
    ) -> Result<Option<TurboJson>, Error> {
        let Some(raw_turbo_json) = RawTurboJson::read(repo_root, path)? else {
            return Ok(None);
        };

        let mut turbo_json = TurboJson::try_from(raw_turbo_json)?;
        // Override with root's future flags (only root turbo.json can define them)
        turbo_json.future_flags = future_flags;
        Ok(Some(turbo_json))
    }

    pub fn task(
        &self,
        task_id: &TaskId,
        task_name: &TaskName,
    ) -> Result<Option<ProcessedTaskDefinition>, Error> {
        match self.tasks.get(&task_id.as_task_name()) {
            Some(entry) => {
                ProcessedTaskDefinition::from_raw(entry.value.clone(), &self.future_flags).map(Some)
            }
            None => self
                .tasks
                .get(task_name)
                .map(|entry| {
                    ProcessedTaskDefinition::from_raw(entry.value.clone(), &self.future_flags)
                })
                .transpose(),
        }
    }

    pub fn validate(&self, validations: &[TurboJSONValidation]) -> Vec<Error> {
        validations
            .iter()
            .flat_map(|validation| validation(self))
            .collect()
    }

    pub fn has_root_tasks(&self) -> bool {
        self.tasks
            .iter()
            .any(|(task_name, _)| task_name.package() == Some(ROOT_PKG_NAME))
    }

    /// Adds a local proxy task to a workspace TurboJson
    pub fn with_proxy(&mut self, mfe_package_name: Option<&str>) {
        if self.extends.is_empty() {
            self.extends = Spanned::new(vec!["//".into()]);
        }

        self.tasks.insert(
            TaskName::from("proxy"),
            Spanned::new(RawTaskDefinition {
                cache: Some(Spanned::new(false)),
                depends_on: mfe_package_name.map(|mfe_package_name| {
                    Spanned::new(vec![Spanned::new(UnescapedString::from(format!(
                        "{mfe_package_name}#build"
                    )))])
                }),
                persistent: Some(Spanned::new(true)),
                env_mode: Some(Spanned::new(EnvMode::Loose)),
                ..Default::default()
            }),
        );
    }

    /// Adds a "with" relationship from `task` to `with`
    pub fn with_task(&mut self, task: TaskName<'static>, with: &TaskName) {
        if self.extends.is_empty() {
            self.extends = Spanned::new(vec!["//".into()]);
        }

        let task_definition = self.tasks.entry(task).or_default();

        let with_tasks = task_definition.as_inner_mut().with.get_or_insert_default();

        with_tasks.push(Spanned::new(UnescapedString::from(with.to_string())))
    }
}

type TurboJSONValidation = fn(&TurboJson) -> Vec<Error>;

pub fn validate_no_package_task_syntax(turbo_json: &TurboJson) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .filter(|(task_name, _)| task_name.is_package_task())
        .map(|(task_name, entry)| {
            let (span, text) = entry.span_and_text("turbo.json");
            Error::UnnecessaryPackageTaskSyntax(Box::new(UnnecessaryPackageTaskSyntaxError {
                actual: task_name.to_string(),
                wanted: task_name.task().to_string(),
                span,
                text,
            }))
        })
        .collect()
}

pub fn validate_extends(turbo_json: &TurboJson) -> Vec<Error> {
    match turbo_json.extends.first() {
        Some(package_name) if package_name != ROOT_PKG_NAME || turbo_json.extends.len() > 1 => {
            let (span, text) = turbo_json.extends.span_and_text("turbo.json");
            vec![Error::ExtendFromNonRoot { span, text }]
        }
        None => {
            let path = turbo_json
                .path
                .as_ref()
                .map_or("turbo.json", |p| p.as_ref());

            let (span, text) = match turbo_json.text {
                Some(ref text) => {
                    let len = text.len();
                    let span: SourceSpan = (0, len - 1).into();
                    (Some(span), text.to_string())
                }
                None => (None, String::new()),
            };

            vec![Error::NoExtends {
                span,
                text: NamedSource::new(path, text),
            }]
        }
        _ => vec![],
    }
}

pub fn validate_with_has_no_topo(turbo_json: &TurboJson) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .flat_map(|(_, definition)| {
            definition.with.iter().flatten().filter_map(|with_task| {
                if with_task.starts_with(TOPOLOGICAL_PIPELINE_DELIMITER) {
                    let (span, text) = with_task.span_and_text("turbo.json");
                    Some(Error::InvalidTaskWith { span, text })
                } else {
                    None
                }
            })
        })
        .collect()
}

fn gather_env_vars(
    vars: Vec<Spanned<impl Into<String>>>,
    key: &str,
    into: &mut HashSet<String>,
) -> Result<(), Error> {
    for value in vars {
        let value: Spanned<String> = value.map(|v| v.into());
        if value.starts_with(ENV_PIPELINE_DELIMITER) {
            let (span, text) = value.span_and_text("turbo.json");
            // Hard error to help people specify this correctly during migration.
            // TODO: Remove this error after we have run summary.
            return Err(Error::InvalidEnvPrefix(Box::new(InvalidEnvPrefixError {
                key: key.to_string(),
                value: value.into_inner(),
                span,
                text,
                env_pipeline_delimiter: ENV_PIPELINE_DELIMITER,
            })));
        }

        into.insert(value.into_inner());
    }

    Ok(())
}

// Takes an input/output glob that might start with TURBO_ROOT_PREFIX
#[cfg(test)]
mod tests {
    use anyhow::Result;
    use biome_deserialize::json::deserialize_from_json_str;
    use biome_json_parser::JsonParserOptions;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use test_case::test_case;
    use turbopath::RelativeUnixPath;
    use turborepo_task_id::TaskName;
    use turborepo_unescape::UnescapedString;

    use super::{processed::*, *};
    use crate::{
        boundaries::BoundariesConfig,
        cli::OutputLogsMode,
        task_graph::{TaskDefinition, TaskOutputs},
        turbo_json::RawTaskDefinition,
    };

    #[test_case("{}", "empty boundaries")]
    #[test_case(r#"{"tags": {} }"#, "empty tags")]
    #[test_case(
        r#"{"tags": { "my-tag": { "dependencies": { "allow": ["my-package"] } } }"#,
        "tags and dependencies"
    )]
    #[test_case(
        r#"{
        "tags": {
            "my-tag": {
                "dependencies": {
                    "allow": ["my-package"],
                    "deny": ["my-other-package"]
                }
            }
        }
    }"#,
        "tags and dependencies 2"
    )]
    #[test_case(
        r#"{
        "tags": {
            "my-tag": {
                "dependents": {
                    "allow": ["my-package"],
                    "deny": ["my-other-package"]
                }
            }
        }
    }"#,
        "tags and dependents"
    )]
    #[test_case(
        r#"{
            "implicitDependencies": ["my-package"],
        }"#,
        "implicit dependencies"
    )]
    #[test_case(
        r#"{
            "implicitDependencies": ["my-package"],
            "tags": {
                "my-tag": {
                    "dependents": {
                        "allow": ["my-package"],
                        "deny": ["my-other-package"]
                    }
                }
            },
        }"#,
        "implicit dependencies and tags"
    )]
    #[test_case(
        r#"{
          "dependencies": {
              "allow": ["my-package"]
          }
      }"#,
        "package rule"
    )]
    fn test_deserialize_boundaries(json: &str, name: &str) {
        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_boundaries_config: BoundariesConfig =
            deserialized_result.into_deserialized().unwrap();
        insta::assert_json_snapshot!(name.replace(' ', "_"), raw_boundaries_config);
    }

    #[test_case(
        "{}",
        RawTaskDefinition::default(),
        TaskDefinition::default()
    ; "empty task definition")]
    #[test_case(
        r#"{ "persistent": false }"#,
        RawTaskDefinition {
            persistent: Some(Spanned::new(false).with_range(16..21)),
            ..RawTaskDefinition::default()
        },
        TaskDefinition::default()
    ; "just persistent"
    )]
    #[test_case(
        r#"{
          "dependsOn": ["cli#build"],
          "env": ["OS"],
          "passThroughEnv": ["AWS_SECRET_KEY"],
          "outputs": ["package/a/dist"],
          "cache": false,
          "inputs": ["package/a/src/**"],
          "outputLogs": "full",
          "persistent": true,
          "interactive": true,
          "interruptible": true
        }"#,
        RawTaskDefinition {
            depends_on: Some(Spanned::new(vec![Spanned::<UnescapedString>::new("cli#build".into()).with_range(26..37)]).with_range(25..38)),
            env: Some(vec![Spanned::<UnescapedString>::new("OS".into()).with_range(58..62)]),
            pass_through_env: Some(vec![Spanned::<UnescapedString>::new("AWS_SECRET_KEY".into()).with_range(94..110)]),
            outputs: Some(vec![Spanned::<UnescapedString>::new("package/a/dist".into()).with_range(135..151)]),
            cache: Some(Spanned::new(false).with_range(173..178)),
            inputs: Some(vec![Spanned::<UnescapedString>::new("package/a/src/**".into()).with_range(201..219)]),
            output_logs: Some(Spanned::new(OutputLogsMode::Full).with_range(246..252)),
            persistent: Some(Spanned::new(true).with_range(278..282)),
            interactive: Some(Spanned::new(true).with_range(309..313)),
            interruptible: Some(Spanned::new(true).with_range(342..346)),
            env_mode: None,
            with: None,
        },
        TaskDefinition {
          env: vec!["OS".to_string()],
          outputs: TaskOutputs {
              inclusions: vec!["package/a/dist".to_string()],
              exclusions: vec![],
          },
          cache: false,
          inputs: TaskInputs::new(vec!["package/a/src/**".to_string()]),
          output_logs: OutputLogsMode::Full,
          pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
          task_dependencies: vec![Spanned::<TaskName<'_>>::new("cli#build".into()).with_range(26..37)],
          topological_dependencies: vec![],
          persistent: true,
          interactive: true,
          interruptible: true,
          env_mode: None,
          with: None,
        }
      ; "full"
    )]
    #[test_case(
        r#"{
              "dependsOn": ["cli#build"],
              "env": ["OS"],
              "passThroughEnv": ["AWS_SECRET_KEY"],
              "outputs": ["package\\a\\dist"],
              "cache": false,
              "inputs": ["package\\a\\src\\**"],
              "outputLogs": "full",
              "persistent": true,
              "interruptible": true
            }"#,
        RawTaskDefinition {
            depends_on: Some(Spanned::new(vec![Spanned::<UnescapedString>::new("cli#build".into()).with_range(30..41)]).with_range(29..42)),
            env: Some(vec![Spanned::<UnescapedString>::new("OS".into()).with_range(66..70)]),
            pass_through_env: Some(vec![Spanned::<UnescapedString>::new("AWS_SECRET_KEY".into()).with_range(106..122)]),
            outputs: Some(vec![Spanned::<UnescapedString>::new("package\\a\\dist".into()).with_range(151..169)]),
            cache: Some(Spanned::new(false).with_range(195..200)),
            inputs: Some(vec![Spanned::<UnescapedString>::new("package\\a\\src\\**".into()).with_range(227..248)]),
            output_logs: Some(Spanned::new(OutputLogsMode::Full).with_range(279..285)),
            persistent: Some(Spanned::new(true).with_range(315..319)),
            interruptible: Some(Spanned::new(true).with_range(352..356)),
            interactive: None,
            env_mode: None,
            with: None,
        },
        TaskDefinition {
            env: vec!["OS".to_string()],
            outputs: TaskOutputs {
                inclusions: vec!["package\\a\\dist".to_string()],
                exclusions: vec![],
            },
            cache: false,
            inputs: TaskInputs::new(vec!["package\\a\\src\\**".to_string()]),
            output_logs: OutputLogsMode::Full,
            pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
            task_dependencies: vec![Spanned::<TaskName<'_>>::new("cli#build".into()).with_range(30..41)],
            topological_dependencies: vec![],
            persistent: true,
            interruptible: true,
            interactive: false,
            env_mode: None,
            with: None,
        }
      ; "full (windows)"
    )]
    #[test_case(
        r#"{
            "inputs": ["$TURBO_ROOT$/config.txt"],
            "outputs": ["$TURBO_ROOT$/coverage/**", "!$TURBO_ROOT$/coverage/index.html"]
        }"#,
        RawTaskDefinition {
            inputs: Some(vec![Spanned::new(UnescapedString::from("$TURBO_ROOT$/config.txt")).with_range(25..50)]),
            outputs: Some(vec![
                Spanned::new(UnescapedString::from("$TURBO_ROOT$/coverage/**")).with_range(77..103),
                Spanned::new(UnescapedString::from("!$TURBO_ROOT$/coverage/index.html")).with_range(105..140),
            ]),
            ..RawTaskDefinition::default()
        },
        TaskDefinition {
            inputs: TaskInputs::new(vec!["../../config.txt".to_owned()]),
            outputs: TaskOutputs {
                inclusions: vec!["../../coverage/**".to_owned()],
                exclusions: vec!["../../coverage/index.html".to_owned()],
            },
            ..TaskDefinition::default()
        }
    ; "turbo root"
    )]
    #[test_case(
        r#"{
            "with": ["proxy"]
        }"#,
        RawTaskDefinition {
            with: Some(vec![
                Spanned::new(UnescapedString::from("proxy")).with_range(23..30),
            ]),
            ..RawTaskDefinition::default()
        },
        TaskDefinition {
            with: Some(vec![Spanned::new(TaskName::from("proxy")).with_range(23..30)]),
            ..TaskDefinition::default()
        }
    ; "with task"
    )]
    fn test_deserialize_task_definition(
        task_definition_content: &str,
        expected_raw_task_definition: RawTaskDefinition,
        expected_task_definition: TaskDefinition,
    ) -> Result<()> {
        let deserialized_result = deserialize_from_json_str(
            task_definition_content,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_task_definition: RawTaskDefinition =
            deserialized_result.into_deserialized().unwrap();
        assert_eq!(raw_task_definition, expected_raw_task_definition);

        let task_definition =
            TaskDefinition::from_raw(raw_task_definition, RelativeUnixPath::new("../..").unwrap())?;
        assert_eq!(task_definition, expected_task_definition);

        Ok(())
    }

    #[test_case("[]", TaskOutputs::default() ; "empty")]
    #[test_case(r#"["target/**"]"#, TaskOutputs { inclusions: vec!["target/**".to_string()], exclusions: vec![] })]
    #[test_case(
        r#"[".next/**", "!.next/cache/**"]"#,
        TaskOutputs {
             inclusions: vec![".next/**".to_string()],
             exclusions: vec![".next/cache/**".to_string()]
        }
        ; "with .next"
    )]
    #[test_case(
        r#"[".next\\**", "!.next\\cache\\**"]"#,
        TaskOutputs {
            inclusions: vec![".next\\**".to_string()],
            exclusions: vec![".next\\cache\\**".to_string()]
        }
        ; "with .next (windows)"
    )]
    fn test_deserialize_task_outputs(
        task_outputs_str: &str,
        expected_task_outputs: TaskOutputs,
    ) -> Result<()> {
        let raw_task_outputs: Vec<UnescapedString> = serde_json::from_str(task_outputs_str)?;
        let turbo_root = RelativeUnixPath::new("../..")?;
        let processed_outputs = ProcessedOutputs::new(
            raw_task_outputs.into_iter().map(Spanned::new).collect(),
            &FutureFlags::default(),
        )?;
        let task_outputs = TaskOutputs::from_processed(processed_outputs, turbo_root)?;
        assert_eq!(task_outputs, expected_task_outputs);

        Ok(())
    }

    #[test]
    fn test_turbo_task_pruning() {
        let json = RawTurboJson::parse_from_serde(json!({
            "tasks": {
                "//#top": {},
                "build": {},
                "a#build": {},
                "b#build": {},
            }
        }))
        .unwrap();
        let pruned_json = json.prune_tasks(&["a"]);
        let expected: RawTurboJson = RawTurboJson::parse_from_serde(json!({
            "tasks": {
                "//#top": {},
                "build": {},
                "a#build": {},
            }
        }))
        .unwrap();
        // We do this comparison manually so we don't compare the `task_name_range`
        // fields, which are expected to be different
        let pruned_pipeline = pruned_json.tasks.unwrap();
        let expected_pipeline = expected.tasks.unwrap();
        for (
            (pruned_task_name, pruned_pipeline_entry),
            (expected_task_name, expected_pipeline_entry),
        ) in pruned_pipeline
            .into_iter()
            .zip(expected_pipeline.into_iter())
        {
            assert_eq!(pruned_task_name, expected_task_name);
            assert_eq!(pruned_pipeline_entry.value, expected_pipeline_entry.value);
        }
    }

    #[test_case("full", Some(OutputLogsMode::Full) ; "full")]
    #[test_case("hash-only", Some(OutputLogsMode::HashOnly) ; "hash-only")]
    #[test_case("new-only", Some(OutputLogsMode::NewOnly) ; "new-only")]
    #[test_case("errors-only", Some(OutputLogsMode::ErrorsOnly) ; "errors-only")]
    #[test_case("none", Some(OutputLogsMode::None) ; "none")]
    #[test_case("junk", None ; "invalid value")]
    fn test_parsing_output_logs_mode(output_logs: &str, expected: Option<OutputLogsMode>) {
        let json: Result<RawTurboJson, _> = RawTurboJson::parse_from_serde(json!({
            "tasks": {
                "build": {
                    "outputLogs": output_logs,
                }
            }
        }));

        let actual = json
            .as_ref()
            .ok()
            .and_then(|j| j.tasks.as_ref())
            .and_then(|pipeline| pipeline.0.get(&TaskName::from("build")))
            .and_then(|build| build.value.output_logs.clone())
            .map(|mode| mode.into_inner());
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{ "tags": [] }"#, "empty tags in package")]
    #[test_case(r#"{ "tags": ["my-tag"] }"#, "one tag")]
    #[test_case(r#"{ "tags": ["my-tag", "my-other-tag"] }"#, "two tags")]
    fn test_tags(json: &str, name: &str) {
        let json = RawTurboJson::parse(json, "").unwrap();
        insta::assert_json_snapshot!(name.replace(' ', "_"), json.tags);
    }

    #[test_case(r#"{ "ui": "tui" }"#, Some(UIMode::Tui) ; "tui")]
    #[test_case(r#"{ "ui": "stream" }"#, Some(UIMode::Stream) ; "stream")]
    #[test_case(r#"{}"#, None ; "missing")]
    fn test_ui(json: &str, expected: Option<UIMode>) {
        let json = RawTurboJson::parse(json, "").unwrap();
        assert_eq!(json.ui.as_ref().map(|ui| *ui.as_inner()), expected);
    }

    #[test_case(r#"{ "experimentalSpaces": { "id": "hello-world" } }"#, Some(SpacesJson { id: Some("hello-world".to_string().into()) }))]
    #[test_case(r#"{ "experimentalSpaces": {} }"#, Some(SpacesJson { id: None }))]
    #[test_case(r#"{}"#, None)]
    fn test_spaces(json: &str, expected: Option<SpacesJson>) {
        let json = RawTurboJson::parse(json, "").unwrap();
        assert_eq!(json.experimental_spaces, expected);
    }

    #[test_case(r#"{ "daemon": true }"#, r#"{"daemon":true}"# ; "daemon_on")]
    #[test_case(r#"{ "daemon": false }"#, r#"{"daemon":false}"# ; "daemon_off")]
    fn test_daemon(json: &str, expected: &str) {
        let parsed = RawTurboJson::parse(json, "").unwrap();
        let actual = serde_json::to_string(&parsed).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{ "ui": "tui" }"#, r#"{"ui":"tui"}"# ; "tui")]
    #[test_case(r#"{ "ui": "stream" }"#, r#"{"ui":"stream"}"# ; "stream")]
    fn test_ui_serialization(input: &str, expected: &str) {
        let parsed = RawTurboJson::parse(input, "").unwrap();
        let actual = serde_json::to_string(&parsed).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{"dangerouslyDisablePackageManagerCheck":true}"#, Some(true) ; "t")]
    #[test_case(r#"{"dangerouslyDisablePackageManagerCheck":false}"#, Some(false) ; "f")]
    #[test_case(r#"{}"#, None ; "missing")]
    fn test_allow_no_package_manager_serde(json_str: &str, expected: Option<bool>) {
        let json = RawTurboJson::parse(json_str, "").unwrap();
        assert_eq!(
            json.allow_no_package_manager
                .as_ref()
                .map(|allow| *allow.as_inner()),
            expected
        );
        let serialized = serde_json::to_string(&json).unwrap();
        assert_eq!(serialized, json_str);
    }

    #[test]
    fn test_with_proxy_empty() {
        let mut json = TurboJson::default();
        json.with_proxy(None);
        assert_eq!(json.extends.as_inner().as_slice(), &["//".to_string()]);
        assert!(json.tasks.contains_key(&TaskName::from("proxy")));
    }

    #[test]
    fn test_with_proxy_existing() {
        let mut json = TurboJson::default();
        json.tasks.insert(
            TaskName::from("build"),
            Spanned::new(RawTaskDefinition::default()),
        );
        json.with_proxy(None);
        assert_eq!(json.extends.as_inner().as_slice(), &["//".to_string()]);
        assert!(json.tasks.contains_key(&TaskName::from("proxy")));
        assert!(json.tasks.contains_key(&TaskName::from("build")));
    }

    #[test]
    fn test_with_proxy_with_proxy_build() {
        let mut json = TurboJson::default();
        json.with_proxy(Some("my-proxy"));
        assert_eq!(json.extends.as_inner().as_slice(), &["//".to_string()]);
        let proxy_task = json.tasks.get(&TaskName::from("proxy"));
        assert!(proxy_task.is_some());
        let proxy_task = proxy_task.unwrap().as_inner();
        assert_eq!(
            proxy_task
                .depends_on
                .as_ref()
                .unwrap()
                .as_inner()
                .as_slice(),
            &[Spanned::new(UnescapedString::from("my-proxy#build"))]
        );
    }

    #[test]
    fn test_with_sibling_empty() {
        let mut json = TurboJson::default();
        json.with_task(TaskName::from("dev"), &TaskName::from("api#server"));
        let dev_task = json.tasks.get(&TaskName::from("dev"));
        assert!(dev_task.is_some());
        let dev_task = dev_task.unwrap().as_inner();
        assert_eq!(
            dev_task.with.as_ref().unwrap().as_slice(),
            &[Spanned::new(UnescapedString::from("api#server"))]
        );
    }

    #[test]
    fn test_with_sibling_existing() {
        let mut json = TurboJson::default();
        json.tasks.insert(
            TaskName::from("dev"),
            Spanned::new(RawTaskDefinition {
                persistent: Some(Spanned::new(true)),
                ..Default::default()
            }),
        );
        json.with_task(TaskName::from("dev"), &TaskName::from("api#server"));
        let dev_task = json.tasks.get(&TaskName::from("dev"));
        assert!(dev_task.is_some());
        let dev_task = dev_task.unwrap().as_inner();
        assert_eq!(dev_task.persistent, Some(Spanned::new(true)));
        assert_eq!(
            dev_task.with.as_ref().unwrap().as_slice(),
            &[Spanned::new(UnescapedString::from("api#server"))]
        );
    }

    #[test]
    fn test_future_flags_not_allowed_in_workspace() {
        let json = r#"{
            "extends": ["//"],
            "tasks": {
                "build": {}
            },
            "futureFlags": {
                "newFeature": true
            }
        }"#;

        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_turbo_json: RawTurboJson = deserialized_result.into_deserialized().unwrap();

        // Try to convert to TurboJson - this should fail
        let turbo_json_result = TurboJson::try_from(raw_turbo_json);
        assert!(turbo_json_result.is_err());

        let error = turbo_json_result.unwrap_err();
        let error_str = error.to_string();
        assert!(
            error_str.contains("The \"futureFlags\" key can only be used in the root turbo.json")
        );
    }

    #[test]
    fn test_deserialize_future_flags() {
        let json = r#"{
            "tasks": {
                "build": {}
            },
            "futureFlags": {
                "turboExtendsKeyword": true
            }
        }"#;

        let deserialized_result = deserialize_from_json_str(
            json,
            JsonParserOptions::default().with_allow_comments(),
            "turbo.json",
        );
        let raw_turbo_json: RawTurboJson = deserialized_result.into_deserialized().unwrap();

        // Verify that futureFlags is parsed correctly
        assert!(raw_turbo_json.future_flags.is_some());
        let future_flags = raw_turbo_json.future_flags.as_ref().unwrap();
        assert_eq!(
            future_flags.as_inner(),
            &FutureFlags {
                turbo_extends_keyword: true
            }
        );

        // Verify that the futureFlags field doesn't cause errors during conversion to
        // TurboJson
        let turbo_json = TurboJson::try_from(raw_turbo_json);
        assert!(turbo_json.is_ok());
    }

    #[test]
    fn test_validate_with_has_no_topo() {
        let turbo_json = TurboJson {
            tasks: Pipeline(
                vec![(
                    TaskName::from("dev"),
                    Spanned::new(RawTaskDefinition {
                        with: Some(vec![Spanned::new(UnescapedString::from("^proxy"))]),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let errs = validate_with_has_no_topo(&turbo_json);
        assert_eq!(errs.len(), 1);
        let error = &errs[0];
        assert_eq!(
            error.to_string(),
            "`with` cannot use dependency relationships."
        );
    }
}
