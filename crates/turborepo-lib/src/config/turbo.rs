use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    path::Path,
};

use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, RelativeUnixPathBuf};

use crate::{
    config::Error,
    opts::RemoteCacheOpts,
    package_json::PackageJson,
    run::{
        pipeline::{
            BookkeepingTaskDefinition, Pipeline, TaskDefinition, TaskDefinitionHashable,
            TaskOutputMode, TaskOutputs,
        },
        task_id::{get_package_task_from_id, is_package_task, root_task_id},
    },
    task_graph::{BookkeepingTaskDefinition, TaskOutputMode},
};

#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<String>,
    #[serde(flatten)]
    pub other: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
// The raw deserialized turbo.json file.
pub struct RawTurboJson {
    #[serde(default)]
    // Global root filesystem dependencies
    global_deps: Vec<String>,
    #[serde(default)]
    global_env: Vec<String>,
    #[serde(default)]
    global_pass_through_env: Vec<String>,
    #[serde(default)]
    // .env files to consider, in order.
    global_dot_env: Vec<RelativeUnixPathBuf>,
    // Pipeline is a map of Turbo pipeline entries which define the task graph
    // and cache behavior on a per task or per package-task basis.
    pipeline: RawPipeline,
    // Configuration options when interfacing with the remote cache
    pub(crate) remote_cache_options: Option<RemoteCacheOpts>,

    #[serde(default)]
    extends: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_spaces: Option<SpacesJson>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(transparent)]
struct RawPipeline(BTreeMap<String, RawTask>);

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RawTaskDefinition {
    outputs: Option<Vec<String>>,
    cache: Option<bool>,
    depends_on: Option<Vec<String>>,
    inputs: Option<Vec<String>>,
    output_mode: Option<TaskOutputMode>,
    persistent: Option<bool>,
    env: Option<Vec<String>>,
    pass_through_env: Option<Vec<String>>,
    dot_env: Option<Vec<String>>,
}

const CONFIG_FILE: &str = "turbo.json";
const ENV_PIPELINE_DELIMITER: &str = "$";
const TOPOLOGICAL_PIPELINE_DELIMITER: &str = "^";

impl From<RawTaskDefinition> for BookkeepingTaskDefinition {
    fn from(raw_task: RawTaskDefinition) -> Self {
        let mut defined_fields: HashSet<&'static str> = HashSet::new();
        let mut experimental_fields = HashSet::new();

        let outputs = raw_task
            .outputs
            .map(|outputs| {
                let mut inclusions = Vec::new();
                let mut exclusions = Vec::new();
                // Assign a bookkeeping field so we know that there really were
                // outputs configured in the underlying config file.
                defined_fields.insert("outputs");

                for glob in outputs {
                    if let Some(glob) = glob.strip_prefix('!') {
                        if Path::new(glob).is_absolute() {
                            println!(
                                "[WARNING] Using an absolute path in \"outputs\" ({}) will not \
                                 work and will be an error in a future version",
                                glob
                            )
                        }

                        exclusions.push(glob.to_string());
                    } else {
                        if Path::new(&glob).is_absolute() {
                            println!(
                                "[WARNING] Using an absolute path in \"outputs\" ({}) will not \
                                 work and will be an error in a future version",
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
            })
            .unwrap_or_default();

        let cache = raw_task.cache.map_or(true, |cache| {
            defined_fields.insert("cache");

            cache
        });

        let mut env_var_dependencies = Vec::new();

        if let Some(depends_on) = raw_task.depends_on.is_some() {
            defined_fields.insert("dependsOn");

            for dependency in depends_on {}
        }

        BookkeepingTaskDefinition {
            defined_fields,
            experimental_fields: Default::default(),
            experimental: Default::default(),
            task_definition: TaskDefinitionHashable {
                outputs,
                cache,
                env_var_dependencies: vec![],
                topological_dependencies: vec![],
                task_dependencies: vec![],
                inputs: vec![],
                output_mode: Default::default(),
                persistent: false,
            },
        }
    }
}

impl RawTurboJson {
    pub fn load(
        dir: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
        include_synthesized_from_root_package_json: bool,
    ) -> Result<RawTurboJson, Error> {
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
            (false, Err(Error::Io(_))) => return Err(Error::NoTurboJson),
            // There was an error, and we don't have any chance of recovering
            // because we aren't synthesizing anything
            (false, Err(e)) => return Err(e),
            // We're not synthesizing anything and there was no error, we're done
            (false, Ok(turbo)) => return Ok(turbo),
            // turbo.json doesn't exist, but we're going try to synthesize something
            (true, Err(Error::Io(_))) => RawTurboJson::default(),
            // some other happened, we can't recover
            (true, Err(e)) => return Err(e),
            // we're synthesizing, but we have a starting point
            // Note: this will have to change to support task inference in a monorepo
            // for now, we're going to error on any "root" tasks and turn non-root tasks into root
            // tasks
            (true, Ok(mut turbo_from_files)) => {
                let mut pipeline = Pipeline::default();
                for (task_id, task_definition) in turbo_from_files.pipeline {
                    if is_package_task(&task_id) {
                        return Err(Error::PackageTaskInSinglePackageMode { task_id });
                    }
                    pipeline.insert(root_task_id(&task_id), task_definition);
                }

                turbo_from_files.pipeline = pipeline;

                turbo_from_files
            }
        };

        for (script_name, _) in &root_package_json.scripts {
            if !turbo_json.has_task(script_name) {
                let task_name = root_task_id(&script_name);
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
                            should_cache: false,
                            ..TaskDefinitionHashable::default()
                        },
                        ..BookkeepingTaskDefinition::default()
                    },
                );
            }
        }

        Ok(turbo_json)
    }

    fn has_task(&self, task: &str) -> bool {
        for key in self.pipeline.keys() {
            if key == task {
                return true;
            }
            if is_package_task(key) {
                let (_, task_name) = get_package_task_from_id(key);
                if task_name == task {
                    return true;
                }
            }
        }

        false
    }

    fn read(path: &AbsoluteSystemPath) -> Result<RawTurboJson, Error> {
        let file = File::open(path)?;
        let turbo_json: RawTurboJson = serde_json::from_reader(&file)?;
        Ok(turbo_json)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use serde_json::{Map, Value};
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::AbsoluteSystemPath;

    use crate::config::RawTurboJson;

    #[test_case(r"{}", TurboJson::default() ; "empty")]
    fn test_get_root_turbo_no_synthesizing(
        turbo_json_content: &str,
        expected_turbo_json: RawTurboJson,
    ) -> Result<()> {
        let root_dir = tempdir()?;
        let root_package_json = crate::package_json::PackageJson::default();
        let repo_root = AbsoluteSystemPath::new(root_dir.path())?;
        fs::write(repo_root.join_component("turbo.json"), turbo_json_content)?;

        let turbo_json = RawTurboJson::load(repo_root, &root_package_json, false)?;
        assert_eq!(turbo_json, expected_turbo_json);

        Ok(())
    }
}
