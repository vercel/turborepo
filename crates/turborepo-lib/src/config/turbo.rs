use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};
use turborepo_cache::RemoteCacheOpts;

use crate::{
    cli::OutputLogsMode,
    config::Error,
    package_json::PackageJson,
    run::task_id::{TaskId, TaskName, ROOT_PKG_NAME},
    task_graph::{BookkeepingTaskDefinition, Pipeline, TaskDefinitionHashable, TaskOutputs},
};

#[derive(Serialize, Deserialize, Debug, Default, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<String>,
    #[serde(flatten)]
    pub other: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
// The processed TurboJSON ready for use by Turborepo.
pub struct TurboJson {
    extends: Vec<String>,
    pub(crate) global_deps: Vec<String>,
    pub(crate) global_dot_env: Vec<RelativeUnixPathBuf>,
    pub(crate) global_env: Vec<String>,
    pub(crate) global_pass_through_env: Vec<String>,
    pub(crate) pipeline: Pipeline,
    pub(crate) remote_cache_options: Option<RemoteCacheOpts>,
    pub(crate) space_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
// The raw deserialized turbo.json file.
pub struct RawTurboJSON {
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
    pipeline: Option<RawPipeline>,
    // Configuration options when interfacing with the remote cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) remote_cache_options: Option<RemoteCacheOpts>,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(transparent)]
struct RawPipeline(BTreeMap<TaskName<'static>, RawTaskDefinition>);

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
struct RawTaskDefinition {
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
                    println!(
                        "[WARNING] Using an absolute path in \"outputs\" ({}) will not work and \
                         will be an error in a future version",
                        glob
                    )
                }

                exclusions.push(glob.to_string());
            } else {
                if Utf8Path::new(&glob).is_absolute() {
                    println!(
                        "[WARNING] Using an absolute path in \"outputs\" ({}) will not work and \
                         will be an error in a future version",
                        glob
                    )
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

impl TryFrom<RawTaskDefinition> for BookkeepingTaskDefinition {
    type Error = Error;

    fn try_from(raw_task: RawTaskDefinition) -> Result<Self, Error> {
        let mut defined_fields = HashSet::new();

        let outputs = raw_task
            .outputs
            .map(|outputs| {
                // Assign a bookkeeping field so we know that there really were
                // outputs configured in the underlying config file.
                defined_fields.insert("Outputs".to_string());

                outputs.into()
            })
            .unwrap_or_default();

        let cache = raw_task.cache.map_or(true, |cache| {
            defined_fields.insert("Cache".to_string());

            cache
        });

        let mut topological_dependencies = Vec::new();
        let mut task_dependencies = Vec::new();

        let mut env_var_dependencies = HashSet::new();
        if let Some(depends_on) = raw_task.depends_on {
            // If there was a dependsOn field, add the bookkeeping
            // we don't care what's in the field, just that it was there
            // We'll use this marker to overwrite while merging TaskDefinitions.
            defined_fields.insert("DependsOn".to_string());

            for dependency in depends_on {
                if let Some(dependency) = dependency.strip_prefix(ENV_PIPELINE_DELIMITER) {
                    println!(
                        "[DEPRECATED] Declaring an environment variable in \"dependsOn\" is \
                         deprecated, found {}. Use the \"env\" key or use `npx @turbo/codemod \
                         migrate-env-var-dependencies`.\n",
                        dependency
                    );
                    defined_fields.insert("Env".to_string());
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
                defined_fields.insert("Env".to_string());
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
                defined_fields.insert("Inputs".to_string());
                for input in &inputs {
                    if Path::new(&input).is_absolute() {
                        println!(
                            "[WARNING] Using an absolute path in \"inputs\" ({}) will not work \
                             and will be an error in a future version",
                            input
                        )
                    }
                }

                inputs
            })
            .unwrap_or_default();

        let pass_through_env = raw_task
            .pass_through_env
            .map(|env| -> Result<Vec<String>, Error> {
                defined_fields.insert("PassThroughEnv".to_string());
                let mut pass_through_env = HashSet::new();
                gather_env_vars(env, "passThroughEnv", &mut pass_through_env)?;
                let mut pass_through_env: Vec<String> = pass_through_env.into_iter().collect();
                pass_through_env.sort();
                Ok(pass_through_env)
            })
            .transpose()?
            .unwrap_or_default();

        let dot_env = raw_task
            .dot_env
            .map(|env| -> Result<Vec<RelativeUnixPathBuf>, Error> {
                defined_fields.insert("DotEnv".to_string());
                // Going to _at least_ be an empty array.
                let mut dot_env = Vec::new();
                for dot_env_path in env {
                    let type_checked_path = RelativeUnixPathBuf::new(dot_env_path)?;
                    // These are _explicitly_ not sorted.
                    dot_env.push(type_checked_path);
                }

                Ok(dot_env)
            })
            .transpose()?
            .unwrap_or_default();

        if raw_task.output_mode.is_some() {
            defined_fields.insert("OutputMode".to_string());
        }
        if raw_task.persistent.is_some() {
            defined_fields.insert("Persistent".to_string());
        }

        Ok(BookkeepingTaskDefinition {
            defined_fields,
            experimental_fields: Default::default(),
            experimental: Default::default(),
            task_definition: TaskDefinitionHashable {
                outputs,
                cache,
                topological_dependencies,
                task_dependencies,
                env,
                inputs,
                pass_through_env,
                dot_env,
                output_mode: raw_task.output_mode.unwrap_or_default(),
                persistent: raw_task.persistent.unwrap_or_default(),
            },
        })
    }
}

impl RawTurboJSON {
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

impl TryFrom<RawTurboJSON> for TurboJson {
    type Error = Error;

    fn try_from(raw_turbo: RawTurboJSON) -> Result<Self, Error> {
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
                    println!(
                        "[WARNING] Using an absolute path in \"globalDependencies\" ({}) will not \
                         work and will be an error in a future version",
                        value
                    )
                }

                global_file_dependencies.insert(value);
            }
        }

        Ok(TurboJson {
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
                .transpose()?
                .unwrap_or_default(),
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
                .transpose()?
                .unwrap_or_default(),
            pipeline: raw_turbo
                .pipeline
                .into_iter()
                .flat_map(|p| p.0)
                .map(|(task_name, task_definition)| Ok((task_name, task_definition.try_into()?)))
                .collect::<Result<HashMap<_, _>, Error>>()?,
            // copy these over, we don't need any changes here.
            remote_cache_options: raw_turbo.remote_cache_options,
            extends: raw_turbo.extends.unwrap_or_default(),
            // Directly to space_id, we don't need to keep the struct
            space_id: raw_turbo.experimental_spaces.and_then(|s| s.id),
        })
    }
}

impl TurboJson {
    /// Loads turbo.json by reading the file at `dir` and optionally combining
    /// with synthesized information from the provided package.json
    pub fn load(
        dir: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
        include_synthesized_from_root_package_json: bool,
    ) -> Result<TurboJson, Error> {
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
            (true, Err(Error::Io(_))) => TurboJson::default(),
            // some other happened, we can't recover
            (true, Err(e)) => return Err(e),
            // we're synthesizing, but we have a starting point
            // Note: this will have to change to support task inference in a monorepo
            // for now, we're going to error on any "root" tasks and turn non-root tasks into root
            // tasks
            (true, Ok(mut turbo_from_files)) => {
                let mut pipeline = Pipeline::default();
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
                // Explicitly set Cache to false in this definition and add the bookkeeping
                // fields so downstream we can pretend that it was set on
                // purpose (as if read from a config file) rather than
                // defaulting to the 0-value of a boolean field.
                turbo_json.pipeline.insert(
                    task_name,
                    BookkeepingTaskDefinition {
                        defined_fields: {
                            let mut set = HashSet::new();
                            set.insert("Cache".to_string());
                            set
                        },
                        task_definition: TaskDefinitionHashable {
                            cache: false,
                            ..TaskDefinitionHashable::default()
                        },
                        ..BookkeepingTaskDefinition::default()
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
    fn read(path: &AbsoluteSystemPath) -> Result<TurboJson, Error> {
        let contents = path.read()?;
        let turbo_json: RawTurboJSON =
            serde_json::from_reader(json_comments::StripComments::new(contents.as_slice()))?;

        turbo_json.try_into()
    }

    pub fn task(
        &self,
        task_id: &TaskId,
        task_name: &TaskName,
    ) -> Option<BookkeepingTaskDefinition> {
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

type TurboJSONValidation = fn(&TurboJson) -> Vec<Error>;

pub fn validate_no_package_task_syntax(turbo_json: &TurboJson) -> Vec<Error> {
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

pub fn validate_extends(turbo_json: &TurboJson) -> Vec<Error> {
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
    use std::{collections::HashSet, fs};

    use anyhow::Result;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};

    use super::RawTurboJSON;
    use crate::{
        cli::OutputLogsMode,
        config::{turbo::RawTaskDefinition, TurboJson},
        package_json::PackageJson,
        run::task_id::TaskName,
        task_graph::{
            BookkeepingTaskDefinition, TaskDefinitionExperiments, TaskDefinitionHashable,
            TaskOutputs,
        },
    };

    #[test_case(r"{}", TurboJson::default() ; "empty")]
    #[test_case(r#"{ "globalDependencies": ["tsconfig.json", "jest.config.js"] }"#,
        TurboJson {
            global_deps: vec!["jest.config.js".to_string(), "tsconfig.json".to_string()],
            ..TurboJson::default()
        }
    ; "global dependencies (sorted)")]
    #[test_case(r#"{ "globalDotEnv": [".env.local", ".env"] }"#,
        TurboJson {
            global_dot_env: vec![RelativeUnixPathBuf::new(".env.local").unwrap(), RelativeUnixPathBuf::new(".env").unwrap()],
            ..TurboJson::default()
        }
    ; "global dot env (unsorted)")]
    #[test_case(r#"{ "globalPassThroughEnv": ["GITHUB_TOKEN", "AWS_SECRET_KEY"] }"#,
        TurboJson {
            global_pass_through_env: vec!["AWS_SECRET_KEY".to_string(), "GITHUB_TOKEN".to_string()],
            ..TurboJson::default()
        }
    )]
    fn test_get_root_turbo_no_synthesizing(
        turbo_json_content: &str,
        expected_turbo_json: TurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let root_package_json = PackageJson::default();
        let repo_root = AbsoluteSystemPath::from_std_path(root_dir.path())?;
        fs::write(repo_root.join_component("turbo.json"), turbo_json_content)?;

        let turbo_json = TurboJson::load(repo_root, &root_package_json, false)?;
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

    #[test_case(
        None,
        PackageJson {
             scripts: [("build".to_string(), "echo build".to_string())].into_iter().collect(),
             ..PackageJson::default()
        },
        TurboJson {
            pipeline: [(
                "//#build".into(),
                BookkeepingTaskDefinition {
                    defined_fields: ["Cache".to_string()].into_iter().collect(),
                    task_definition: TaskDefinitionHashable {
                        cache: false,
                        ..TaskDefinitionHashable::default()
                    },
                    ..BookkeepingTaskDefinition::default()
                }
            )].into_iter().collect(),
            ..TurboJson::default()
        }
    )]
    #[test_case(
        Some("{}"),
        PackageJson {
            legacy_turbo_config: Some(serde_json::Value::String("build".to_string())),
            ..PackageJson::default()
        },
        TurboJson::default()
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
        TurboJson {
            pipeline: [(
                "//#build".into(),
                BookkeepingTaskDefinition {
                    defined_fields: ["Cache".to_string()].into_iter().collect(),
                    task_definition: TaskDefinitionHashable {
                        cache: true,
                        ..TaskDefinitionHashable::default()
                    },
                    ..BookkeepingTaskDefinition::default()
                }
            ),
            (
                "//#test".into(),
                BookkeepingTaskDefinition {
                    defined_fields: ["Cache".to_string()].into_iter().collect(),
                    task_definition: TaskDefinitionHashable {
                        cache: false,
                        ..TaskDefinitionHashable::default()
                    },
                    ..BookkeepingTaskDefinition::default()
                }
            )].into_iter().collect(),
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

        let turbo_json = TurboJson::load(repo_root, &root_package_json, true)?;
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }

    #[test_case(
        "{}",
        RawTaskDefinition::default(),
        BookkeepingTaskDefinition::default()
    ; "empty")]
    #[test_case(
        r#"{ "persistent": false }"#,
        RawTaskDefinition {
            persistent: Some(false),
            ..RawTaskDefinition::default()
        },
        BookkeepingTaskDefinition {
            defined_fields: ["Persistent".to_string()].into_iter().collect(),
            experimental_fields: HashSet::new(),
            experimental: TaskDefinitionExperiments::default(),
            task_definition: TaskDefinitionHashable::default()
        }
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
        BookkeepingTaskDefinition {
            defined_fields: [
                "Outputs".to_string(),
                "Env".to_string(),
                "DotEnv".to_string(),
                "OutputMode".to_string(),
                "PassThroughEnv".to_string(),
                "Cache".to_string(),
                "Persistent".to_string(),
                "Inputs".to_string(),
                "DependsOn".to_string()
            ].into_iter().collect(),
            experimental_fields: HashSet::new(),
            experimental: TaskDefinitionExperiments {},
            task_definition: TaskDefinitionHashable {
                dot_env: vec![RelativeUnixPathBuf::new("package/a/.env").unwrap()],
                env: vec!["OS".to_string()],
                outputs: TaskOutputs {
                    inclusions: vec!["package/a/dist".to_string()],
                    exclusions: vec![],
                },
                cache: false,
                inputs: vec!["package/a/src/**".to_string()],
                output_mode: OutputLogsMode::Full,
                pass_through_env: vec!["AWS_SECRET_KEY".to_string()],
                task_dependencies: vec!["cli#build".into()],
                topological_dependencies: vec![],
                persistent: true,
            }
        }
    )]
    fn test_deserialize_task_definition(
        task_definition_content: &str,
        expected_raw_task_definition: RawTaskDefinition,
        expected_task_definition: BookkeepingTaskDefinition,
    ) -> Result<()> {
        let raw_task_definition: RawTaskDefinition = serde_json::from_str(task_definition_content)?;
        assert_eq!(raw_task_definition, expected_raw_task_definition);

        let task_definition: BookkeepingTaskDefinition = raw_task_definition.try_into()?;
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
        let json: RawTurboJSON = serde_json::from_value(serde_json::json!({
            "pipeline": {
                "//#top": {},
                "build": {},
                "a#build": {},
                "b#build": {},
            }
        }))
        .unwrap();
        let pruned_json = json.prune_tasks(&["a"]);
        let expected: RawTurboJSON = serde_json::from_value(serde_json::json!({
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
        let json: Result<RawTurboJSON, _> = serde_json::from_value(serde_json::json!({
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
