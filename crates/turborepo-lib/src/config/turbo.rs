use std::{collections::HashSet, fs::File};

use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;

use crate::{
    config::Error,
    opts::RemoteCacheOpts,
    package_json::PackageJson,
    run::task_id::{get_package_task_from_id, is_package_task, root_task_id},
    task_graph::{BookkeepingTaskDefinition, Pipeline, TaskDefinitionHashable},
};

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SpacesJson {
    pub id: Option<String>,
    #[serde(flatten)]
    pub other: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TurboJson {
    #[serde(flatten)]
    other: serde_json::Value,
    pub(crate) remote_cache_opts: Option<RemoteCacheOpts>,
    pub space_id: Option<String>,
    #[allow(dead_code)]
    pub pipeline: Pipeline,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental_spaces: Option<SpacesJson>,
}

const CONFIG_FILE: &str = "turbo.json";

impl TurboJson {
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
            (false, Err(Error::Io(_))) => return Err(Error::NoTurboJson),
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

    fn read(path: &AbsoluteSystemPath) -> Result<TurboJson, Error> {
        let file = File::open(path)?;
        let turbo_json: TurboJson = serde_json::from_reader(&file)?;
        Ok(turbo_json)
    }
}

fn get_root_turbo_json(
    repo_root: &AbsoluteSystemPath,
    is_single_package: bool,
) -> Result<TurboJson, Error> {
    let package_json = PackageJson::load(&repo_root.join_component("package.json"))?;

    TurboJson::load(repo_root, &package_json, is_single_package)
}
