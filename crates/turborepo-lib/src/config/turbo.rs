use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    ops::{Deref, DerefMut},
    path::Path,
};

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_repository::{package_graph::ROOT_PKG_NAME, package_json::PackageJson};

use crate::{
    cli::OutputLogsMode,
    config::{ConfigurationOptions, Error},
    run::task_id::{TaskId, TaskName},
    task_graph::{Pipeline, TaskDefinition, TaskOutputs},
};

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<String>,
    #[serde(flatten)]
    pub other: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
// The fully processed turbo.json config ready for use by Turborepo.
// Contains fields that are fully resolved from both the root turbo.json
// and the workspace turbo.json files. Also contains synthesized config
// from root package.json in single package mode.
pub struct FinalTurboJson {
    pub(crate) extends: Vec<String>,
    pub(crate) global_deps: Vec<String>,
    pub(crate) global_dot_env: Vec<RelativeUnixPathBuf>,
    pub(crate) global_env: Vec<String>,
    pub(crate) global_pass_through_env: Vec<String>,
    pub(crate) pipeline: Pipeline,
    pub(crate) remote_cache: Option<ConfigurationOptions>,
    pub(crate) space_id: Option<String>,
}

// A turbo.json config that is synthesized but not yet resolved.
// This means that we've done the work to synthesize the config from
// package.json, but we haven't yet resolved the workspace
// turbo.json files into a single definition
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SynthesizedTurboJson {
    pub(crate) extends: Vec<String>,
    pub(crate) global_deps: Vec<String>,
    pub(crate) global_dot_env: Option<Vec<RelativeUnixPathBuf>>,
    pub(crate) global_env: Vec<String>,
    pub(crate) global_pass_through_env: Option<Vec<String>>,
    pub(crate) pipeline: RawPipeline,
    pub(crate) remote_cache: Option<ConfigurationOptions>,
    pub(crate) space_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
// The raw deserialized turbo.json file.
pub struct RawTurboJson {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_spaces: Option<SpacesJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extends: Option<Vec<String>>,
    // Global root filesystem dependencies
    #[serde(skip_serializing_if = "Option::is_none")]
    global_dependencies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    global_env: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    global_pass_through_env: Option<Vec<String>>,
    // .env files to consider, in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    global_dot_env: Option<Vec<String>>,
    // Pipeline is a map of Turbo pipeline entries which define the task graph
    // and cache behavior on a per task or per package-task basis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<RawPipeline>,
    // Configuration options when interfacing with the remote cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) remote_cache: Option<ConfigurationOptions>,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(transparent)]
pub struct RawPipeline(BTreeMap<TaskName<'static>, RawTaskDefinition>);

impl IntoIterator for RawPipeline {
    type Item = (TaskName<'static>, RawTaskDefinition);
    type IntoIter = <BTreeMap<TaskName<'static>, RawTaskDefinition> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Deref for RawPipeline {
    type Target = BTreeMap<TaskName<'static>, RawTaskDefinition>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RawPipeline {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RawTaskDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depends_on: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dot_env: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inputs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pass_through_env: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    persistent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_mode: Option<OutputLogsMode>,
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
    // merges it into RawTaskDefinition. It uses the bookkeeping
    // defined_fields to determine which fields should be overwritten and when
    // 0-values should be respected.
    pub fn merge(&mut self, other: RawTaskDefinition) {
        set_field!(self, other, outputs);
        set_field!(self, other, cache);
        set_field!(self, other, depends_on);
        set_field!(self, other, inputs);
        set_field!(self, other, output_mode);
        set_field!(self, other, persistent);
        set_field!(self, other, env);
        set_field!(self, other, pass_through_env);
        set_field!(self, other, dot_env);
    }
}

const CONFIG_FILE: &str = "turbo.json";
const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

impl From<Vec<String>> for TaskOutputs {
    fn from(outputs: Vec<String>) -> Self {
        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();

        for glob in outputs {
            if let Some(glob) = glob.strip_prefix('!') {
                if Utf8Path::new(glob).is_absolute() {
                    writeln!(
                        std::io::stderr(),
                        "[WARNING] Using an absolute path in \"outputs\" ({}) will not work and \
                         will be an error in a future version",
                        glob
                    )
                    .expect("unable to write to stderr");
                }

                exclusions.push(glob.to_string());
            } else {
                if Utf8Path::new(&glob).is_absolute() {
                    writeln!(
                        std::io::stderr(),
                        "[WARNING] Using an absolute path in \"outputs\" ({}) will not work and \
                         will be an error in a future version",
                        glob
                    )
                    .expect("unable to write to stderr");
                }

                inclusions.push(glob);
            }
        }

        inclusions.sort();
        exclusions.sort();

        TaskOutputs {
            inclusions,
            exclusions,
        }
    }
}

impl TryFrom<RawTaskDefinition> for TaskDefinition {
    type Error = Error;

    fn try_from(raw_task: RawTaskDefinition) -> Result<Self, Error> {
        let outputs = raw_task
            .outputs
            .map(|outputs| outputs.into())
            .unwrap_or_default();

        let cache = raw_task.cache;

        let mut env_var_dependencies = HashSet::new();
        let mut topological_dependencies = Vec::new();
        let mut task_dependencies = Vec::new();
        if let Some(depends_on) = raw_task.depends_on {
            for dependency in depends_on {
                if let Some(dependency) = dependency.strip_prefix(ENV_PIPELINE_DELIMITER) {
                    println!(
                        "[DEPRECATED] Declaring an environment variable in \"dependsOn\" is \
                         deprecated, found {}. Use the \"env\" key or use `npx @turbo/codemod \
                         migrate-env-var-dependencies`.\n",
                        dependency
                    );
                    env_var_dependencies.insert(dependency.to_string());
                } else if let Some(topo_dependency) =
                    dependency.strip_prefix(TOPOLOGICAL_PIPELINE_DELIMITER)
                {
                    topological_dependencies.push(topo_dependency.to_string().into());
                } else {
                    task_dependencies.push(dependency.into());
                }
            }
        }

        task_dependencies.sort();
        topological_dependencies.sort();

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
            .map(|inputs| {
                for input in &inputs {
                    if Path::new(&input).is_absolute() {
                        writeln!(
                            std::io::stderr(),
                            "[WARNING] Using an absolute path in \"inputs\" ({}) will not work \
                             and will be an error in a future version",
                            input
                        )
                        .expect("unable to write to stderr");
                    }
                }

                inputs
            })
            .unwrap_or_default();

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

        let dot_env = raw_task
            .dot_env
            .map(|env| -> Result<Vec<RelativeUnixPathBuf>, Error> {
                // Going to _at least_ be an empty array.
                let mut dot_env = Vec::new();
                for dot_env_path in env {
                    let type_checked_path = RelativeUnixPathBuf::new(dot_env_path)?;
                    // These are _explicitly_ not sorted.
                    dot_env.push(type_checked_path);
                }

                Ok(dot_env)
            })
            .transpose()?;

        Ok(TaskDefinition {
            outputs,
            cache: cache.unwrap_or(true),
            topological_dependencies,
            task_dependencies,
            env,
            inputs,
            pass_through_env,
            dot_env,
            output_mode: raw_task.output_mode.unwrap_or_default(),
            persistent: raw_task.persistent.unwrap_or_default(),
        })
    }
}

impl RawTurboJson {
    pub(crate) fn read(path: &AbsoluteSystemPath) -> Result<RawTurboJson, Error> {
        let contents = path.read()?;
        let raw_turbo_json: RawTurboJson =
            serde_json::from_reader(json_comments::StripComments::new(contents.as_slice()))?;

        Ok(raw_turbo_json)
    }

    /// Produces a new turbo.json without any tasks that reference non-existent
    /// workspaces
    pub fn prune_tasks<S: AsRef<str>>(&self, workspaces: &[S]) -> Self {
        let mut this = self.clone();
        if let Some(pipeline) = &mut this.pipeline {
            pipeline.0.retain(|task_name, _| {
                task_name.in_workspace(ROOT_PKG_NAME)
                    || workspaces
                        .iter()
                        .any(|workspace| task_name.in_workspace(workspace.as_ref()))
            })
        }

        this
    }
}

impl TryFrom<RawTurboJson> for SynthesizedTurboJson {
    type Error = Error;

    fn try_from(raw_turbo: RawTurboJson) -> Result<Self, Error> {
        let mut global_env = HashSet::new();
        let mut global_file_dependencies = HashSet::new();

        if let Some(global_env_from_turbo) = raw_turbo.global_env {
            gather_env_vars(global_env_from_turbo, "globalEnv", &mut global_env)?;
        }

        // TODO: In the rust port, warnings should be refactored to a post-parse
        // validation step
        for value in raw_turbo.global_dependencies.into_iter().flatten() {
            if let Some(env_var) = value.strip_prefix(ENV_PIPELINE_DELIMITER) {
                println!(
                    "[DEPRECATED] Declaring an environment variable in \"dependsOn\" is \
                     deprecated, found {}. Use the \"env\" key or use `npx @turbo/codemod \
                     migrate-env-var-dependencies`.\n",
                    env_var
                );

                global_env.insert(env_var.to_string());
            } else {
                if Path::new(&value).is_absolute() {
                    writeln!(
                        std::io::stderr(),
                        "[WARNING] Using an absolute path in \"globalDependencies\" ({}) will not \
                         work and will be an error in a future version",
                        value
                    )
                    .expect("unable to write to stderr");
                }

                global_file_dependencies.insert(value);
            }
        }

        Ok(SynthesizedTurboJson {
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
            global_dot_env: raw_turbo
                .global_dot_env
                .map(|env| -> Result<Vec<RelativeUnixPathBuf>, Error> {
                    let mut global_dot_env = Vec::new();
                    for dot_env_path in env {
                        let type_checked_path = RelativeUnixPathBuf::new(dot_env_path)?;
                        // These are _explicitly_ not sorted.
                        global_dot_env.push(type_checked_path);
                    }

                    Ok(global_dot_env)
                })
                .transpose()?,
            pipeline: raw_turbo.pipeline.unwrap_or_default(),
            // copy these over, we don't need any changes here.
            remote_cache: raw_turbo.remote_cache,
            extends: raw_turbo.extends.unwrap_or_default(),
            // Directly to space_id, we don't need to keep the struct
            space_id: raw_turbo.experimental_spaces.and_then(|s| s.id),
        })
    }
}

impl SynthesizedTurboJson {
    /// Loads turbo.json by reading the file at `dir` and optionally combining
    /// with synthesized information from the provided package.json
    pub fn load(
        dir: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
        include_synthesized_from_root_package_json: bool,
    ) -> Result<SynthesizedTurboJson, Error> {
        if root_package_json.legacy_turbo_config.is_some() {
            println!(
                "[WARNING] \"turbo\" in package.json is no longer supported. Migrate to {} by \
                 running \"npx @turbo/codemod create-turbo-config\"\n",
                CONFIG_FILE
            );
        }

        let turbo_from_files = Self::read(&dir.join_component(CONFIG_FILE));

        let mut turbo_json = match (include_synthesized_from_root_package_json, turbo_from_files) {
            // If the file didn't exist, throw a custom error here instead of propagating
            (false, Err(Error::Io(_))) => return Err(Error::NoTurboJSON),
            // There was an error, and we don't have any chance of recovering
            // because we aren't synthesizing anything
            (false, Err(e)) => return Err(e),
            // We're not synthesizing anything and there was no error, we're done
            (false, Ok(turbo)) => return Ok(turbo),
            // turbo.json doesn't exist, but we're going try to synthesize something
            (true, Err(Error::Io(_))) => SynthesizedTurboJson::default(),
            // some other happened, we can't recover
            (true, Err(e)) => return Err(e),
            // we're synthesizing, but we have a starting point
            // Note: this will have to change to support task inference in a monorepo
            // for now, we're going to error on any "root" tasks and turn non-root tasks into root
            // tasks
            (true, Ok(mut turbo_from_files)) => {
                let mut pipeline = RawPipeline::default();
                for (task_name, task_definition) in turbo_from_files.pipeline {
                    if task_name.is_package_task() {
                        return Err(Error::PackageTaskInSinglePackageMode {
                            task_id: task_name.to_string(),
                        });
                    }

                    pipeline.insert(task_name.into_root_task(), task_definition);
                }

                turbo_from_files.pipeline = pipeline;

                turbo_from_files
            }
        };

        for script_name in root_package_json.scripts.keys() {
            let task_name = TaskName::from(script_name.as_str());
            if !turbo_json.has_task(&task_name) {
                let task_name = task_name.into_root_task();
                turbo_json.pipeline.insert(
                    task_name,
                    RawTaskDefinition {
                        cache: Some(false),
                        ..RawTaskDefinition::default()
                    },
                );
            }
        }

        Ok(turbo_json)
    }

    fn has_task(&self, task_name: &TaskName) -> bool {
        for key in self.pipeline.keys() {
            if key == task_name || (key.task() == task_name.task() && !task_name.is_package_task())
            {
                return true;
            }
        }

        false
    }

    /// Reads a `RawTurboJson` from the given path
    /// and then converts it into `TurboJson`
    pub(crate) fn read(path: &AbsoluteSystemPath) -> Result<SynthesizedTurboJson, Error> {
        let raw_turbo_json = RawTurboJson::read(path)?;
        raw_turbo_json.try_into()
    }

    pub fn task(&self, task_id: &TaskId, task_name: &TaskName) -> Option<RawTaskDefinition> {
        match self.pipeline.get(&task_id.as_task_name()) {
            Some(task) => Some(task.clone()),
            None => self.pipeline.get(task_name).cloned(),
        }
    }

    pub fn validate(&self, validations: &[TurboJSONValidation]) -> Vec<Error> {
        validations
            .iter()
            .flat_map(|validation| validation(self))
            .collect()
    }
}

type TurboJSONValidation = fn(&SynthesizedTurboJson) -> Vec<Error>;

pub fn validate_no_package_task_syntax(turbo_json: &SynthesizedTurboJson) -> Vec<Error> {
    turbo_json
        .pipeline
        .keys()
        .filter(|task_name| task_name.is_package_task())
        .map(|task_name| Error::UnnecessaryPackageTaskSyntax {
            actual: task_name.to_string(),
            wanted: task_name.task().to_string(),
        })
        .collect()
}

pub fn validate_extends(turbo_json: &SynthesizedTurboJson) -> Vec<Error> {
    match turbo_json.extends.first() {
        Some(package_name) if package_name != ROOT_PKG_NAME || turbo_json.extends.len() > 1 => {
            vec![Error::ExtendFromNonRoot]
        }
        None => vec![Error::NoExtends],
        _ => vec![],
    }
}

fn gather_env_vars(vars: Vec<String>, key: &str, into: &mut HashSet<String>) -> Result<(), Error> {
    for value in vars {
        if value.starts_with(ENV_PIPELINE_DELIMITER) {
            // Hard error to help people specify this correctly during migration.
            // TODO: Remove this error after we have run summary.
            return Err(Error::InvalidEnvPrefix {
                key: key.to_string(),
                value,
                env_pipeline_delimiter: ENV_PIPELINE_DELIMITER,
            });
        }

        into.insert(value);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
    use turborepo_repository::package_json::PackageJson;

    use super::RawTurboJson;
    use crate::{
        cli::OutputLogsMode,
        config::{
            turbo::{RawPipeline, RawTaskDefinition},
            SynthesizedTurboJson,
        },
        run::task_id::TaskName,
        task_graph::{TaskDefinition, TaskOutputs},
    };

    #[test_case(r"{}", SynthesizedTurboJson::default() ; "empty")]
    #[test_case(r#"{ "globalDependencies": ["tsconfig.json", "jest.config.js"] }"#,
        SynthesizedTurboJson {
            global_deps: vec!["jest.config.js".to_string(), "tsconfig.json".to_string()],
            ..SynthesizedTurboJson::default()
        }
    ; "global dependencies (sorted)")]
    #[test_case(r#"{ "globalDotEnv": [".env.local", ".env"] }"#,
        SynthesizedTurboJson {
            global_dot_env: Some(vec![RelativeUnixPathBuf::new(".env.local").unwrap(), RelativeUnixPathBuf::new(".env").unwrap()]),
            ..SynthesizedTurboJson::default()
        }
    ; "global dot env (unsorted)")]
    #[test_case(r#"{ "globalPassThroughEnv": ["GITHUB_TOKEN", "AWS_SECRET_KEY"] }"#,
        SynthesizedTurboJson {
            global_pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string(), "GITHUB_TOKEN".to_string()]),
            ..SynthesizedTurboJson::default()
        }
    )]
    fn test_get_root_turbo_no_synthesizing(
        turbo_json_content: &str,
        expected_turbo_json: SynthesizedTurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let root_package_json = PackageJson::default();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;
        fs::write(repo_root.join_component("turbo.json"), turbo_json_content)?;

        let turbo_json = SynthesizedTurboJson::load(repo_root, &root_package_json, false)?;
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

    #[test_case(
        None,
        PackageJson {
             scripts: [("build".to_string(), "echo build".to_string())].into_iter().collect(),
             ..PackageJson::default()
        },
        SynthesizedTurboJson {
            pipeline: RawPipeline([(
                "//#build".into(),
                RawTaskDefinition {
                  cache: Some(false),
                  ..RawTaskDefinition::default()
                }
            )].into_iter().collect()),
            ..SynthesizedTurboJson::default()
        }
    )]
    #[test_case(
        Some("{}"),
        PackageJson {
            legacy_turbo_config: Some(serde_json::Value::String("build".to_string())),
            ..PackageJson::default()
        },
        SynthesizedTurboJson::default()
    )]
    #[test_case(
        Some(r#"{
            "pipeline": {
                "build": {
                    "cache": true
                }
            }
        }"#),
        PackageJson {
             scripts: [("test".to_string(), "echo test".to_string())].into_iter().collect(),
             ..PackageJson::default()
        },
        SynthesizedTurboJson {
            pipeline: RawPipeline([(
                "//#build".into(),
                RawTaskDefinition {
                    cache: Some(true),
                    ..RawTaskDefinition::default()
                }
            ),
            (
                "//#test".into(),
                RawTaskDefinition {
                  cache: Some(false),
                  ..RawTaskDefinition::default()
                }
            )].into_iter().collect()),
            ..SynthesizedTurboJson::default()
        }
    )]
    fn test_get_root_turbo_with_synthesizing(
        turbo_json_content: Option<&str>,
        root_package_json: PackageJson,
        expected_turbo_json: SynthesizedTurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;

        if let Some(content) = turbo_json_content {
            fs::write(repo_root.join_component("turbo.json"), content)?;
        }

        let turbo_json = SynthesizedTurboJson::load(repo_root, &root_package_json, true)?;
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
            persistent: Some(false),
            ..RawTaskDefinition::default()
        },
        TaskDefinition::default()
    )]
    #[test_case(
        r#"{ "dotEnv": [] }"#,
        RawTaskDefinition {
            dot_env: Some(Vec::new()),
            ..RawTaskDefinition::default()
        },
        TaskDefinition {
            dot_env: Some(Vec::new()),
            ..Default::default()
        }
        ; "empty dotenv"
    )]
    #[test_case(
        r#"{
          "dependsOn": ["cli#build"],
          "dotEnv": ["package/a/.env"],
          "env": ["OS"],
          "passThroughEnv": ["AWS_SECRET_KEY"],
          "outputs": ["package/a/dist"],
          "cache": false,
          "inputs": ["package/a/src/**"],
          "outputMode": "full",
          "persistent": true
        }"#,
        RawTaskDefinition {
            depends_on: Some(vec!["cli#build".to_string()]),
            dot_env: Some(vec!["package/a/.env".to_string()]),
            env: Some(vec!["OS".to_string()]),
            pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
            outputs: Some(vec!["package/a/dist".to_string()]),
            cache: Some(false),
            inputs: Some(vec!["package/a/src/**".to_string()]),
            output_mode: Some(OutputLogsMode::Full),
            persistent: Some(true),
        },
        TaskDefinition {
          dot_env: Some(vec![RelativeUnixPathBuf::new("package/a/.env").unwrap()]),
          env: vec!["OS".to_string()],
          outputs: TaskOutputs {
              inclusions: vec!["package/a/dist".to_string()],
              exclusions: vec![],
          },
          cache: false,
          inputs: vec!["package/a/src/**".to_string()],
          output_mode: OutputLogsMode::Full,
          pass_through_env: Some(vec!["AWS_SECRET_KEY".to_string()]),
          task_dependencies: vec!["cli#build".into()],
          topological_dependencies: vec![],
          persistent: true,
        }
    )]
    fn test_deserialize_task_definition(
        task_definition_content: &str,
        expected_raw_task_definition: RawTaskDefinition,
        expected_task_definition: TaskDefinition,
    ) -> Result<()> {
        let raw_task_definition: RawTaskDefinition = serde_json::from_str(task_definition_content)?;
        assert_eq!(raw_task_definition, expected_raw_task_definition);

        let task_definition: TaskDefinition = raw_task_definition.try_into()?;
        assert_eq!(task_definition, expected_task_definition);

        Ok(())
    }

    #[test_case("[]", TaskOutputs::default())]
    #[test_case(r#"["target/**"]"#, TaskOutputs { inclusions: vec!["target/**".to_string()], exclusions: vec![] })]
    #[test_case(
        r#"[".next/**", "!.next/cache/**"]"#,
        TaskOutputs {
             inclusions: vec![".next/**".to_string()],
             exclusions: vec![".next/cache/**".to_string()]
        }
    )]
    fn test_deserialize_task_outputs(
        task_outputs_str: &str,
        expected_task_outputs: TaskOutputs,
    ) -> Result<()> {
        let raw_task_outputs: Vec<String> = serde_json::from_str(task_outputs_str)?;
        let task_outputs: TaskOutputs = raw_task_outputs.into();
        assert_eq!(task_outputs, expected_task_outputs);

        Ok(())
    }

    #[test]
    fn test_turbo_task_pruning() {
        let json: RawTurboJson = serde_json::from_value(serde_json::json!({
            "pipeline": {
                "//#top": {},
                "build": {},
                "a#build": {},
                "b#build": {},
            }
        }))
        .unwrap();
        let pruned_json = json.prune_tasks(&["a"]);
        let expected: RawTurboJson = serde_json::from_value(serde_json::json!({
            "pipeline": {
                "//#top": {},
                "build": {},
                "a#build": {},
            }
        }))
        .unwrap();

        assert_eq!(pruned_json, expected);
    }

    #[test_case("full", Some(OutputLogsMode::Full) ; "full")]
    #[test_case("hash-only", Some(OutputLogsMode::HashOnly) ; "hash-only")]
    #[test_case("new-only", Some(OutputLogsMode::NewOnly) ; "new-only")]
    #[test_case("errors-only", Some(OutputLogsMode::ErrorsOnly) ; "errors-only")]
    #[test_case("none", Some(OutputLogsMode::None) ; "none")]
    #[test_case("junk", None ; "invalid value")]
    fn test_parsing_output_mode(output_mode: &str, expected: Option<OutputLogsMode>) {
        let json: Result<RawTurboJson, _> = serde_json::from_value(serde_json::json!({
            "pipeline": {
                "build": {
                    "outputMode": output_mode,
                }
            }
        }));

        let actual = json
            .as_ref()
            .ok()
            .and_then(|j| j.pipeline.as_ref())
            .and_then(|pipeline| pipeline.0.get(&TaskName::from("build")))
            .and_then(|build| build.output_mode);
        assert_eq!(actual, expected);
    }
}
