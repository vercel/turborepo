use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use biome_deserialize_macros::Deserializable;
use camino::Utf8Path;
use clap::ValueEnum;
use miette::{NamedSource, SourceSpan};
use serde::{Deserialize, Serialize};
use struct_iterable::Iterable;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath};
use turborepo_errors::Spanned;
use turborepo_repository::{package_graph::ROOT_PKG_NAME, package_json::PackageJson};
use turborepo_unescape::UnescapedString;

use crate::{
    cli::{EnvMode, OutputLogsMode},
    config::{ConfigurationOptions, Error, InvalidEnvPrefixError},
    run::{
        task_access::{TaskAccessTraceFile, TASK_ACCESS_CONFIG_PATH},
        task_id::{TaskId, TaskName},
    },
    task_graph::{TaskDefinition, TaskOutputs},
};

pub mod parser;

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
    pub(crate) extends: Spanned<Vec<String>>,
    pub(crate) global_deps: Vec<String>,
    pub(crate) global_env: Vec<String>,
    pub(crate) global_pass_through_env: Option<Vec<String>>,
    pub(crate) tasks: Pipeline,
}

// Iterable is required to enumerate allowed keys
#[derive(Clone, Debug, Default, Iterable, Serialize, Deserializable)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RawRemoteCacheOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preflight: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
}

impl From<&RawRemoteCacheOptions> for ConfigurationOptions {
    fn from(remote_cache_opts: &RawRemoteCacheOptions) -> Self {
        Self {
            api_url: remote_cache_opts.api_url.clone(),
            login_url: remote_cache_opts.login_url.clone(),
            team_slug: remote_cache_opts.team_slug.clone(),
            team_id: remote_cache_opts.team_id.clone(),
            signature: remote_cache_opts.signature,
            preflight: remote_cache_opts.preflight,
            timeout: remote_cache_opts.timeout,
            enabled: remote_cache_opts.enabled,
            ..Self::default()
        }
    }
}

#[derive(Serialize, Default, Debug, Clone, Iterable, Deserializable)]
#[serde(rename_all = "camelCase")]
// The raw deserialized turbo.json file.
pub struct RawTurboJson {
    #[serde(skip)]
    span: Spanned<()>,

    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<UnescapedString>,

    #[serde(skip_serializing_if = "Option::is_none")]
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
    pub(crate) remote_cache: Option<RawRemoteCacheOptions>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "ui")]
    pub ui: Option<UIMode>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "dangerouslyDisablePackageManagerCheck"
    )]
    pub allow_no_package_manager: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon: Option<Spanned<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_mode: Option<EnvMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<Spanned<UnescapedString>>,

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
}

impl Default for UIMode {
    fn default() -> Self {
        Self::Tui
    }
}

impl UIMode {
    pub fn use_tui(&self) -> bool {
        matches!(self, Self::Tui)
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
    outputs: Option<Vec<Spanned<UnescapedString>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_logs: Option<Spanned<OutputLogsMode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interactive: Option<Spanned<bool>>,
}

macro_rules! set_field {
    ($this:ident, $other:ident, $field:ident) => {{
        if let Some(field) = $other.$field {
            $this.$field = field.into();
        }
    }};
}

impl RawTaskDefinition {
    // merge accepts a RawTaskDefinition and
    // merges it into RawTaskDefinition.
    pub fn merge(&mut self, other: RawTaskDefinition) {
        set_field!(self, other, outputs);

        let other_has_range = other.cache.as_ref().map_or(false, |c| c.range.is_some());
        let self_does_not_have_range = self.cache.as_ref().map_or(false, |c| c.range.is_none());

        if other.cache.is_some()
            // If other has range info and we're missing it, carry it over
            || (other_has_range && self_does_not_have_range)
        {
            self.cache = other.cache;
        }
        set_field!(self, other, depends_on);
        set_field!(self, other, inputs);
        set_field!(self, other, output_logs);
        set_field!(self, other, persistent);
        set_field!(self, other, env);
        set_field!(self, other, pass_through_env);
        set_field!(self, other, interactive);
    }
}

const CONFIG_FILE: &str = "turbo.json";
const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

impl TryFrom<Vec<Spanned<UnescapedString>>> for TaskOutputs {
    type Error = Error;
    fn try_from(outputs: Vec<Spanned<UnescapedString>>) -> Result<Self, Self::Error> {
        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();

        for glob in outputs {
            if let Some(stripped_glob) = glob.value.strip_prefix('!') {
                if Utf8Path::new(stripped_glob).is_absolute() {
                    let (span, text) = glob.span_and_text("turbo.json");
                    return Err(Error::AbsolutePathInConfig {
                        field: "outputs",
                        span,
                        text,
                    });
                }

                exclusions.push(stripped_glob.to_string());
            } else {
                if Utf8Path::new(&glob.value).is_absolute() {
                    let (span, text) = glob.span_and_text("turbo.json");
                    return Err(Error::AbsolutePathInConfig {
                        field: "outputs",
                        span,
                        text,
                    });
                }

                inclusions.push(glob.into_inner().into());
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

impl TryFrom<RawTaskDefinition> for TaskDefinition {
    type Error = Error;

    fn try_from(raw_task: RawTaskDefinition) -> Result<Self, Error> {
        let outputs = raw_task.outputs.unwrap_or_default().try_into()?;

        let cache = raw_task.cache.map_or(true, |c| c.into_inner());
        let interactive = raw_task
            .interactive
            .as_ref()
            .map(|value| value.value)
            .unwrap_or_default();

        if let Some(interactive) = raw_task.interactive {
            let (span, text) = interactive.span_and_text("turbo.json");
            if cache && interactive.value {
                return Err(Error::InteractiveNoCacheable { span, text });
            }
        }

        let mut env_var_dependencies = HashSet::new();
        let mut topological_dependencies: Vec<Spanned<TaskName>> = Vec::new();
        let mut task_dependencies: Vec<Spanned<TaskName>> = Vec::new();
        if let Some(depends_on) = raw_task.depends_on {
            for dependency in depends_on.into_inner() {
                let (span, text) = dependency.span_and_text("turbo.json");
                let (dependency, depspan) = dependency.split();
                let dependency: String = dependency.into();
                if dependency.strip_prefix(ENV_PIPELINE_DELIMITER).is_some() {
                    return Err(Error::InvalidDependsOnValue {
                        field: "dependsOn",
                        span,
                        text,
                    });
                } else if let Some(topo_dependency) =
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

        let env = raw_task
            .env
            .map(|env| -> Result<Vec<String>, Error> {
                gather_env_vars(env, "env", &mut env_var_dependencies)?;
                let mut env_var_dependencies: Vec<String> =
                    env_var_dependencies.into_iter().collect();
                env_var_dependencies.sort();
                Ok(env_var_dependencies)
            })
            .transpose()?
            .unwrap_or_default();

        let inputs = raw_task
            .inputs
            .unwrap_or_default()
            .into_iter()
            .map(|input| {
                if Utf8Path::new(&input.value).is_absolute() {
                    let (span, text) = input.span_and_text("turbo.json");
                    Err(Error::AbsolutePathInConfig {
                        field: "inputs",
                        span,
                        text,
                    })
                } else {
                    Ok(input.to_string())
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let pass_through_env = raw_task
            .pass_through_env
            .map(|env| -> Result<Vec<String>, Error> {
                let mut pass_through_env = HashSet::new();
                gather_env_vars(env, "passThroughEnv", &mut pass_through_env)?;
                let mut pass_through_env: Vec<String> = pass_through_env.into_iter().collect();
                pass_through_env.sort();
                Ok(pass_through_env)
            })
            .transpose()?;

        Ok(TaskDefinition {
            outputs,
            cache,
            topological_dependencies,
            task_dependencies,
            env,
            inputs,
            pass_through_env,
            output_logs: *raw_task.output_logs.unwrap_or_default(),
            persistent: *raw_task.persistent.unwrap_or_default(),
            interactive,
        })
    }
}

impl RawTurboJson {
    pub(crate) fn read(
        repo_root: &AbsoluteSystemPath,
        path: &AnchoredSystemPath,
    ) -> Result<RawTurboJson, Error> {
        let absolute_path = repo_root.resolve(path);
        let contents = absolute_path.read_to_string()?;
        let raw_turbo_json = RawTurboJson::parse(&contents, path)?;

        Ok(raw_turbo_json)
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

        Ok(TurboJson {
            text: raw_turbo.span.text,
            path: raw_turbo.span.path,
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
            tasks: raw_turbo.tasks.unwrap_or_default(),
            // copy these over, we don't need any changes here.
            extends: raw_turbo
                .extends
                .unwrap_or_default()
                .map(|s| s.into_iter().map(|s| s.into()).collect()),
            // Spaces and Remote Cache config is handled through layered config
        })
    }
}

impl TurboJson {
    /// Loads turbo.json by reading the file at `dir` and optionally combining
    /// with synthesized information from the provided package.json
    pub fn load(
        repo_root: &AbsoluteSystemPath,
        dir: &AnchoredSystemPath,
        root_package_json: &PackageJson,
        include_synthesized_from_root_package_json: bool,
    ) -> Result<TurboJson, Error> {
        let turbo_from_files = Self::read(repo_root, &dir.join_component(CONFIG_FILE));
        let turbo_from_trace =
            Self::read(repo_root, &dir.join_components(&TASK_ACCESS_CONFIG_PATH));

        // check the zero config case (turbo trace file, but no turbo.json file)
        if let Ok(turbo_from_trace) = turbo_from_trace {
            if turbo_from_files.is_err() {
                debug!("Using turbo.json synthesized from trace file");
                return Ok(turbo_from_trace);
            }
        }

        let mut turbo_json = match (include_synthesized_from_root_package_json, turbo_from_files) {
            // If the file didn't exist, throw a custom error here instead of propagating
            (false, Err(Error::Io(_))) => return Err(Error::NoTurboJSON),
            // There was an error, and we don't have any chance of recovering
            // because we aren't synthesizing anything
            (false, Err(e)) => return Err(e),
            // We're not synthesizing anything and there was no error, we're done
            (false, Ok(turbo)) => return Ok(turbo),
            // turbo.json doesn't exist, but we're going try to synthesize something
            (true, Err(Error::Io(_))) => TurboJson::default(),
            // some other happened, we can't recover
            (true, Err(e)) => return Err(e),
            // we're synthesizing, but we have a starting point
            // Note: this will have to change to support task inference in a monorepo
            // for now, we're going to error on any "root" tasks and turn non-root tasks into root
            // tasks
            (true, Ok(mut turbo_from_files)) => {
                let mut pipeline = Pipeline::default();
                for (task_name, task_definition) in turbo_from_files.tasks {
                    if task_name.is_package_task() {
                        let (span, text) = task_definition.span_and_text("turbo.json");

                        return Err(Error::PackageTaskInSinglePackageMode {
                            task_id: task_name.to_string(),
                            span,
                            text,
                        });
                    }

                    pipeline.insert(task_name.into_root_task(), task_definition);
                }

                turbo_from_files.tasks = pipeline;

                turbo_from_files
            }
        };

        // TODO: Add location info from package.json
        for script_name in root_package_json.scripts.keys() {
            let task_name = TaskName::from(script_name.as_str());
            if !turbo_json.has_task(&task_name) {
                let task_name = task_name.into_root_task();
                // Explicitly set cache to Some(false) in this definition
                // so we can pretend it was set on purpose. That way it
                // won't get clobbered by the merge function.
                turbo_json.tasks.insert(
                    task_name,
                    Spanned::new(RawTaskDefinition {
                        cache: Some(Spanned::new(false)),
                        ..RawTaskDefinition::default()
                    }),
                );
            }
        }

        Ok(turbo_json)
    }

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
    pub(crate) fn read(
        repo_root: &AbsoluteSystemPath,
        path: &AnchoredSystemPath,
    ) -> Result<TurboJson, Error> {
        let raw_turbo_json = RawTurboJson::read(repo_root, path)?;
        raw_turbo_json.try_into()
    }

    pub fn task(&self, task_id: &TaskId, task_name: &TaskName) -> Option<RawTaskDefinition> {
        match self.tasks.get(&task_id.as_task_name()) {
            Some(entry) => Some(entry.value.clone()),
            None => self.tasks.get(task_name).map(|entry| entry.value.clone()),
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
}

type TurboJSONValidation = fn(&TurboJson) -> Vec<Error>;

pub fn validate_no_package_task_syntax(turbo_json: &TurboJson) -> Vec<Error> {
    turbo_json
        .tasks
        .iter()
        .filter(|(task_name, _)| task_name.is_package_task())
        .map(|(task_name, entry)| {
            let (span, text) = entry.span_and_text("turbo.json");
            Error::UnnecessaryPackageTaskSyntax {
                actual: task_name.to_string(),
                wanted: task_name.task().to_string(),
                span,
                text,
            }
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
            let path = value
                .path
                .as_ref()
                .map_or_else(|| "turbo.json".to_string(), |p| p.to_string());
            return Err(Error::InvalidEnvPrefix(Box::new(InvalidEnvPrefixError {
                key: key.to_string(),
                value: value.into_inner(),
                span,
                text: NamedSource::new(path, text),
                env_pipeline_delimiter: ENV_PIPELINE_DELIMITER,
            })));
        }

        into.insert(value.into_inner());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use biome_deserialize::json::deserialize_from_json_str;
    use biome_json_parser::JsonParserOptions;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPath};
    use turborepo_repository::package_json::PackageJson;
    use turborepo_unescape::UnescapedString;

    use super::{Pipeline, RawTurboJson, Spanned, UIMode};
    use crate::{
        cli::OutputLogsMode,
        run::task_id::TaskName,
        task_graph::{TaskDefinition, TaskOutputs},
        turbo_json::{RawTaskDefinition, TurboJson},
    };

    #[test_case(r"{}", TurboJson::default() ; "empty")]
    #[test_case(r#"{ "globalDependencies": ["tsconfig.json", "jest.config.js"] }"#,
        TurboJson {
            global_deps: vec!["jest.config.js".to_string(), "tsconfig.json".to_string()],
            ..TurboJson::default()
        }
    ; "global dependencies (sorted)")]
    #[test_case(r#"{ "globalPassThroughEnv": ["GITHUB_TOKEN", "AWS_SECRET_KEY"] }"#,
        TurboJson {
            global_pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string(), "GITHUB_TOKEN".to_string()]),
            ..TurboJson::default()
        }
    )]
    #[test_case(r#"{ "//": "A comment"}"#, TurboJson::default() ; "faux comment")]
    #[test_case(r#"{ "//": "A comment", "//": "Another comment" }"#, TurboJson::default() ; "two faux comments")]
    fn test_get_root_turbo_no_synthesizing(
        turbo_json_content: &str,
        expected_turbo_json: TurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let root_package_json = PackageJson::default();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;
        fs::write(repo_root.join_component("turbo.json"), turbo_json_content)?;

        let mut turbo_json = TurboJson::load(
            repo_root,
            AnchoredSystemPath::empty(),
            &root_package_json,
            false,
        )?;

        turbo_json.text = None;
        turbo_json.path = None;
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

    #[test_case(
        None,
        PackageJson {
             scripts: [("build".to_string(), Spanned::new("echo build".to_string()))].into_iter().collect(),
             ..PackageJson::default()
        },
        TurboJson {
            tasks: Pipeline([(
                "//#build".into(),
                Spanned::new(RawTaskDefinition {
                    cache: Some(Spanned::new(false)),
                    ..RawTaskDefinition::default()
                })
              )].into_iter().collect()
            ),
            ..TurboJson::default()
        }
    )]
    #[test_case(
        Some(r#"{
            "tasks": {
                "build": {
                    "cache": true
                }
            }
        }"#),
        PackageJson {
             scripts: [("test".to_string(), Spanned::new("echo test".to_string()))].into_iter().collect(),
             ..PackageJson::default()
        },
        TurboJson {
            tasks: Pipeline([(
                "//#build".into(),
                Spanned::new(RawTaskDefinition {
                    cache: Some(Spanned::new(true).with_range(81..85)),
                    ..RawTaskDefinition::default()
                }).with_range(50..103)
            ),
            (
                "//#test".into(),
                Spanned::new(RawTaskDefinition {
                     cache: Some(Spanned::new(false)),
                    ..RawTaskDefinition::default()
                })
            )].into_iter().collect()),
            ..TurboJson::default()
        }
    )]
    fn test_get_root_turbo_with_synthesizing(
        turbo_json_content: Option<&str>,
        root_package_json: PackageJson,
        expected_turbo_json: TurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;

        if let Some(content) = turbo_json_content {
            fs::write(repo_root.join_component("turbo.json"), content)?;
        }

        let mut turbo_json = TurboJson::load(
            repo_root,
            AnchoredSystemPath::empty(),
            &root_package_json,
            true,
        )?;
        turbo_json.text = None;
        turbo_json.path = None;
        for (_, task_definition) in turbo_json.tasks.iter_mut() {
            task_definition.path = None;
            task_definition.text = None;
        }
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

    #[test_case(
        "{}",
        RawTaskDefinition::default(),
        TaskDefinition::default()
    ; "empty")]
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
          "interactive": true
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
        },
        TaskDefinition {
          env: vec!["OS".to_string()],
          outputs: TaskOutputs {
              inclusions: vec!["package/a/dist".to_string()],
              exclusions: vec![],
          },
          cache: false,
          inputs: vec!["package/a/src/**".to_string()],
          output_logs: OutputLogsMode::Full,
          pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
          task_dependencies: vec![Spanned::<TaskName<'_>>::new("cli#build".into()).with_range(26..37)],
          topological_dependencies: vec![],
          persistent: true,
          interactive: true,
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
              "persistent": true
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
            interactive: None,
        },
        TaskDefinition {
            env: vec!["OS".to_string()],
            outputs: TaskOutputs {
                inclusions: vec!["package\\a\\dist".to_string()],
                exclusions: vec![],
            },
            cache: false,
            inputs: vec!["package\\a\\src\\**".to_string()],
            output_logs: OutputLogsMode::Full,
            pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
            task_dependencies: vec![Spanned::<TaskName<'_>>::new("cli#build".into()).with_range(30..41)],
            topological_dependencies: vec![],
            persistent: true,
            interactive: false,
        }
      ; "full (windows)"
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

        let task_definition: TaskDefinition = raw_task_definition.try_into()?;
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
        let raw_task_outputs = raw_task_outputs
            .into_iter()
            .map(Spanned::new)
            .collect::<Vec<_>>();
        let task_outputs: TaskOutputs = raw_task_outputs.try_into()?;
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

    #[test_case(r#"{ "ui": "tui" }"#, Some(UIMode::Tui) ; "tui")]
    #[test_case(r#"{ "ui": "stream" }"#, Some(UIMode::Stream) ; "stream")]
    #[test_case(r#"{}"#, None ; "missing")]
    fn test_ui(json: &str, expected: Option<UIMode>) {
        let json = RawTurboJson::parse(json, AnchoredSystemPath::new("").unwrap()).unwrap();
        assert_eq!(json.ui, expected);
    }

    #[test_case(r#"{ "daemon": true }"#, r#"{"daemon":true}"# ; "daemon_on")]
    #[test_case(r#"{ "daemon": false }"#, r#"{"daemon":false}"# ; "daemon_off")]
    fn test_daemon(json: &str, expected: &str) {
        let parsed = RawTurboJson::parse(json, AnchoredSystemPath::new("").unwrap()).unwrap();
        let actual = serde_json::to_string(&parsed).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{ "ui": "tui" }"#, r#"{"ui":"tui"}"# ; "tui")]
    #[test_case(r#"{ "ui": "stream" }"#, r#"{"ui":"stream"}"# ; "stream")]
    fn test_ui_serialization(input: &str, expected: &str) {
        let parsed = RawTurboJson::parse(input, AnchoredSystemPath::new("").unwrap()).unwrap();
        let actual = serde_json::to_string(&parsed).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(r#"{"dangerouslyDisablePackageManagerCheck":true}"#, Some(true) ; "t")]
    #[test_case(r#"{"dangerouslyDisablePackageManagerCheck":false}"#, Some(false) ; "f")]
    #[test_case(r#"{}"#, None ; "missing")]
    fn test_allow_no_package_manager_serde(json_str: &str, expected: Option<bool>) {
        let json = RawTurboJson::parse(json_str, AnchoredSystemPath::new("").unwrap()).unwrap();
        assert_eq!(json.allow_no_package_manager, expected);
        let serialized = serde_json::to_string(&json).unwrap();
        assert_eq!(serialized, json_str);
    }
}
