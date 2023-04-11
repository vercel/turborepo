use std::collections::HashSet;

use anyhow::{anyhow, Result};
use log::warn;
use serde::Deserialize;
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use crate::{
    commands::CommandBase,
    package_json::PackageJson,
    pipeline::{BookkeepingTaskDefinition, HashableTaskDefinition, Pipeline},
    task_utils::{is_package_task, root_task_id},
};

const CONFIG_FILE: &str = "turbo.json";

#[derive(Debug, Clone, Deserialize)]
struct TurboJson {
    pipeline: Pipeline,
}

fn load_turbo_config_for_single_package(
    base: &mut CommandBase,
    root_package_json: &mut PackageJson,
) -> Result<TurboJson> {
    if root_package_json.legacy_turbo_config.is_some() {
        warn!(
            "[WARNING] \"turbo\" in package.json is no longer supported. Migrate to {} by running \
             \"npx @turbo/codemod create-turbo-config\"\n",
            CONFIG_FILE
        );

        root_package_json.legacy_turbo_config = None;
    }

    let mut turbo_json = read_turbo_config(
        &base
            .repo_root
            .join_relative(RelativeSystemPathBuf::new("turbo.json")?),
    )?;

    let mut pipeline = Pipeline::new();
    for (task_id, task_definition) in turbo_from_files.pipeline {
        if is_package_task(&task_id) {
            return Err(anyhow!(
                "Package tasks (<package>#<task>) are not allowed in single-package repositories: \
                 found {}",
                task_id
            ));
        }
        pipeline.insert(root_task_id(&task_id).to_string(), task_definition);
    }

    turbo_json.pipeline = pipeline;

    for (script_name, _) in &root_package_json.scripts {
        if !turbo_from_files.pipeline.contains_key(&script_name) {
            let task_name = root_task_id(script_name);
            let mut defined_fields = HashSet::new();
            defined_fields.insert("ShouldCache".to_string());
            let task_definition = BookkeepingTaskDefinition {
                defined_fields,
                task_definition: HashableTaskDefinition {
                    should_cache: false,
                },
                ..Default::default()
            };
            turbo_from_files.insert(task_name, task_definition);
        }
    }

    Ok(turbo_from_files)
}

fn read_turbo_config(turbo_json_path: &AbsoluteSystemPathBuf) -> Result<TurboJson> {
    if turbo_json_path.exists() {
        let contents = std::fs::read_to_string(turbo_json_path)?;
        let turbo_json: TurboJson = serde_json::from_str(&contents)?;
        Ok(turbo_json)
    } else {
        Err!("No {} found", CONFIG_FILE)
    }
}
