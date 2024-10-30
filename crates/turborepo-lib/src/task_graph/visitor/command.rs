use std::path::PathBuf;

use turbopath::AbsoluteSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageName};

use super::Error;
use crate::{opts::TaskArgs, process::Command, run::task_id::TaskId};

#[derive(Debug)]
pub struct CommandFactory<'a> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    package_manager_binary: Result<PathBuf, which::Error>,
    task_args: TaskArgs<'a>,
}

impl<'a> CommandFactory<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        task_args: TaskArgs<'a>,
    ) -> Self {
        let package_manager_binary = which::which(package_graph.package_manager().command());
        Self {
            repo_root,
            package_graph,
            package_manager_binary,
            task_args,
        }
    }

    pub fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, Error> {
        let workspace_info = self
            .package_graph
            .package_info(&PackageName::from(task_id.package()))
            .ok_or_else(|| Error::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })?;

        // bail if the script doesn't exist or is empty
        if workspace_info
            .package_json
            .scripts
            .get(task_id.task())
            .map_or(true, |script| script.is_empty())
        {
            return Ok(None);
        }
        let package_manager_binary = self.package_manager_binary.as_deref().map_err(|e| *e)?;
        let mut cmd = Command::new(package_manager_binary);
        let mut args = vec!["run".to_string(), task_id.task().to_string()];
        if let Some(pass_through_args) = self.task_args.args_for_task(task_id) {
            args.extend(
                self.package_graph
                    .package_manager()
                    .arg_separator(pass_through_args)
                    .map(|s| s.to_string()),
            );
            args.extend(pass_through_args.iter().cloned());
        }
        cmd.args(args);

        let package_dir = self.repo_root.resolve(workspace_info.package_path());
        cmd.current_dir(package_dir);

        // We clear the env before populating it with variables we expect
        cmd.env_clear();
        cmd.envs(environment.iter());

        // We always open stdin and the visitor will close it depending on task
        // configuration
        cmd.open_stdin();

        Ok(Some(cmd))
    }
}
