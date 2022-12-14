use std::collections::HashMap;

use anyhow::{anyhow, Result};

use crate::shim::{PackageJson, RepoMode, RepoState, TaskDefinition, TurboConfig};

fn run(repo_state: &RepoState) -> Result<()> {
    let package_json_path = repo_state.root.join("package.json").canonicalize()?;

    let mut root_package_json = serde_json::from_reader(std::fs::File::open(&package_json_path)?)?;
    let turbo_json = load_turbo_config(repo_state, &mut root_package_json)?;

    Ok(())
}

const TASK_DELIMITER: &str = "#";
const ROOT_PKG_NAME: &str = "//";

fn is_package_task(task: &str) -> bool {
    task.contains(TASK_DELIMITER)
}

fn get_task_id(package_name: &str, target: &str) -> String {
    if is_package_task(target) {
        target.to_string()
    } else {
        format!("{}{}{}", package_name, TASK_DELIMITER, target)
    }
}

fn root_task_id(target: &str) -> String {
    get_task_id(ROOT_PKG_NAME, target)
}

fn load_turbo_config(
    repo_state: &RepoState,
    root_package_json: &mut PackageJson,
) -> Result<TurboConfig> {
    let turbo_config_from_files = read_turbo_config(repo_state, root_package_json);

    let is_multi_package = matches!(repo_state.mode, RepoMode::MultiPackage);

    // If we are in a multi-package repo, we try to get the config
    // from turbo.json/package.json and if it does not exist, we error
    if is_multi_package {
        return turbo_config_from_files.and_then(|turbo_config| {
            turbo_config.ok_or_else(|| {
                anyhow!(
                    "Could not find turbo.json. Follow directions at \
                     https://turbo.build/repo/docs to create one"
                )
            })
        });
    }

    // Otherwise we attempt to synthesize tasks from the root package.json
    let mut turbo_config = if let Some(turbo_config_from_files) = turbo_config_from_files? {
        // we're synthesizing, but we have a starting point
        // Note: this will have to change to support task inference in a monorepo
        // for now, we're going to error on any "root" tasks and turn non-root tasks
        // into root tasks
        let mut pipeline = HashMap::new();
        for (task_id, task_definition) in turbo_config_from_files.pipeline {
            if is_package_task(&task_id) {
                return Err(anyhow!(
                    "Package tasks (<package>#<task>) are not allowed in single-package \
                     repositories: found {}",
                    task_id
                ));
            }
            pipeline.insert(task_id, task_definition);
        }

        TurboConfig { pipeline }
    } else {
        // turbo.json doesn't exist, but we're going try to synthesize something
        TurboConfig {
            pipeline: HashMap::new(),
        }
    };

    for script_name in root_package_json.scripts.iter() {
        if !turbo_config.pipeline.contains_key(script_name) {
            let task_name = root_task_id(&script_name);
            turbo_config.pipeline.insert(task_name, TaskDefinition {});
        }
    }

    Ok(turbo_config)
}

fn read_turbo_config(
    repo_state: &RepoState,
    root_package_json: &mut PackageJson,
) -> Result<Option<TurboConfig>> {
    let turbo_json_path = repo_state.root.join("turbo.json");

    let has_legacy_config = root_package_json.legacy_turbo_config.is_some();

    if turbo_json_path.exists() {
        let turbo_json = serde_json::from_reader(std::fs::File::open(&turbo_json_path)?)?;
        if has_legacy_config {
            println!("[WARNING] Ignoring \"turbo\" key in package.json, using turbo.json instead.",);
            root_package_json.legacy_turbo_config = None;
        }

        return Ok(Some(turbo_json));
    }

    if let Some(legacy_turbo_config) = &root_package_json.legacy_turbo_config {
        println!(
            "[DEPRECATED] \"turbo\" in package.json is deprecated. Migrate to turbo.json by \
             running \"npx @turbo/codemod create-turbo-config\""
        );
        Ok(Some(legacy_turbo_config.clone()))
    } else {
        Ok(None)
    }
}
