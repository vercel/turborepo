use turbopath::AbsoluteSystemPathBuf;

use crate::{
    cli::EnvMode,
    package_json::PackageJson,
    task_graph::{TaskDefinition, TaskOutputs},
};

pub struct PackageTask {
    pub task_id: String,
    task: String,
    package_name: String,
    pkg: PackageJson,
    env_mode: EnvMode,
    pub(crate) task_definition: TaskDefinition,
    pub dir: AbsoluteSystemPathBuf,
    command: String,
    outputs: Vec<String>,
    excluded_outputs: Vec<String>,
    pub(crate) log_file: String,
    hash: String,
}

impl PackageTask {
    pub fn hashable_outputs(&self) -> TaskOutputs {
        let mut inclusion_outputs = vec![format!(".turbo/turbo-{}.log", self.task)];
        inclusion_outputs.extend_from_slice(&self.task_definition.outputs.inclusions[..]);

        let mut hashable = TaskOutputs {
            inclusions: inclusion_outputs,
            exclusions: self.task_definition.outputs.exclusions.clone(),
        };

        hashable.inclusions.sort();
        hashable.exclusions.sort();

        hashable
    }
}
