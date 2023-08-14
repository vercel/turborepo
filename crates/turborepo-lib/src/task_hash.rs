use std::{collections::HashMap, rc::Rc};

use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_env::ResolvedEnvMode;
use turborepo_scm::SCM;

use crate::{
    engine::TaskNode,
    hash::{FileHashes, TaskHashable, TurboHash},
    package_graph::{WorkspaceInfo, WorkspaceName},
    run::task_id::{TaskId, ROOT_PKG_NAME},
    task_graph::TaskDefinition,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing pipeline entry {0}")]
    MissingPipelineEntry(TaskId<'static>),
    #[error("missing package.json for {0}")]
    MissingPackageJson(String),
    #[error(transparent)]
    SCM(#[from] turborepo_scm::Error),
}

#[derive(Debug)]
struct PackageFileHashInputs<'a> {
    task_id: TaskId<'static>,
    task_definition: &'a TaskDefinition,
    workspace_name: WorkspaceName,
}

impl TaskHashable {
    fn calculate_task_hash(mut self) -> String {
        if matches!(self.env_mode, ResolvedEnvMode::Loose) {
            self.pass_through_env = Vec::new();
        }

        self.hash()
    }
}

#[derive(Debug)]
pub struct PackageFileHashes {
    package_input_hashes: HashMap<TaskId<'static>, String>,
    package_inputs_expanded_hashes: HashMap<TaskId<'static>, FileHashes>,
}

impl PackageFileHashes {
    pub fn calculate_file_hashes<'a>(
        scm: SCM,
        all_tasks: impl Iterator<Item = &'a TaskNode>,
        workspaces: HashMap<&WorkspaceName, &WorkspaceInfo>,
        task_definitions: &HashMap<TaskId<'static>, TaskDefinition>,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<PackageFileHashes, Error> {
        let mut hash_tasks = Vec::new();

        for task in all_tasks {
            let TaskNode::Task(task_id) = task else {
                continue;
            };

            if task_id.package() == ROOT_PKG_NAME {
                continue;
            }

            let task_definition = task_definitions
                .get(&task_id)
                .ok_or_else(|| Error::MissingPipelineEntry(task_id.clone()))?;

            // TODO: Look into making WorkspaceName take a Cow
            let workspace_name = WorkspaceName::Other(task_id.package().to_string());

            let package_file_hash_inputs = PackageFileHashInputs {
                task_id: task_id.clone(),
                task_definition,
                workspace_name,
            };

            hash_tasks.push(package_file_hash_inputs);
        }

        let mut hashes = HashMap::with_capacity(hash_tasks.len());
        let mut hash_objects = HashMap::with_capacity(hash_tasks.len());

        for package_file_hash_inputs in hash_tasks {
            let pkg = workspaces
                .get(&package_file_hash_inputs.workspace_name)
                .ok_or_else(|| {
                    Error::MissingPackageJson(package_file_hash_inputs.workspace_name.to_string())
                })?;

            let package_path = pkg
                .package_json_path
                .parent()
                .unwrap_or_else(|| AnchoredSystemPath::new("").unwrap());

            let mut hash_object = scm.get_package_file_hashes(
                &repo_root,
                package_path,
                &package_file_hash_inputs.task_definition.inputs,
            )?;

            if !package_file_hash_inputs.task_definition.dot_env.is_empty() {
                let package_path = pkg
                    .package_json_path
                    .parent()
                    .unwrap_or_else(|| AnchoredSystemPath::new("").unwrap());
                let absolute_package_path = repo_root.resolve(package_path);
                let dot_env_object = scm.hash_existing_of(
                    &absolute_package_path,
                    package_file_hash_inputs
                        .task_definition
                        .dot_env
                        .iter()
                        .map(|p| p.to_anchored_system_path_buf()),
                )?;

                for (key, value) in dot_env_object {
                    hash_object.insert(key, value);
                }
            }

            let file_hashes = FileHashes(hash_object);
            let hash = file_hashes.clone().hash();

            hashes.insert(package_file_hash_inputs.task_id.clone(), hash);
            hash_objects.insert(package_file_hash_inputs.task_id.clone(), file_hashes);
        }

        Ok(PackageFileHashes {
            package_input_hashes: hashes,
            package_inputs_expanded_hashes: hash_objects,
        })
    }
}
