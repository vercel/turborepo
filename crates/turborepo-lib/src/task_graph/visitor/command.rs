use std::path::PathBuf;

use turbopath::AbsoluteSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName};

use super::Error;
use crate::{opts::TaskArgs, process::Command, run::task_id::TaskId};

pub trait CommandProvider {
    fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, Error>;
}

/// A collection of command providers.
///
/// Will attempt to find a command from any of the providers it contains.
/// Ordering of the providers matters as the first present command will be
/// returned. Any errors returned by the providers will be immediately returned.
pub struct CommandFactory<'a> {
    providers: Vec<Box<dyn CommandProvider + 'a + Send>>,
}

impl<'a> CommandFactory<'a> {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add_provider(mut self, provider: impl CommandProvider + 'a + Send) -> Self {
        self.providers.push(Box::new(provider));
        self
    }

    pub fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, Error> {
        for provider in self.providers.iter() {
            let cmd = provider.command(task_id, environment.clone())?;
            if cmd.is_some() {
                return Ok(cmd);
            }
        }
        Ok(None)
    }
}

#[derive(Debug)]
pub struct PackageGraphCommandProvider<'a> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    package_manager_binary: Result<PathBuf, which::Error>,
    task_args: TaskArgs<'a>,
}

impl<'a> PackageGraphCommandProvider<'a> {
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

    fn package_info(&self, task_id: &TaskId) -> Result<&PackageInfo, Error> {
        self.package_graph
            .package_info(&PackageName::from(task_id.package()))
            .ok_or_else(|| Error::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })
    }
}

impl<'a> CommandProvider for PackageGraphCommandProvider<'a> {
    fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, Error> {
        let workspace_info = self.package_info(task_id)?;

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

#[cfg(test)]
mod test {
    use std::ffi::OsStr;

    use insta::assert_snapshot;

    use super::*;

    struct EchoCmdFactory;

    impl CommandProvider for EchoCmdFactory {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Ok(Some(Command::new("echo")))
        }
    }

    struct ErrProvider;

    impl CommandProvider for ErrProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Err(Error::InternalErrors("oops!".into()))
        }
    }

    struct NoneProvider;

    impl CommandProvider for NoneProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Ok(None)
        }
    }

    #[test]
    fn test_first_present_cmd_returned() {
        let factory = CommandFactory::new()
            .add_provider(EchoCmdFactory)
            .add_provider(ErrProvider);
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap()
            .unwrap();
        assert_eq!(cmd.program(), OsStr::new("echo"));
    }

    #[test]
    fn test_error_short_circuits_factory() {
        let factory = CommandFactory::new()
            .add_provider(ErrProvider)
            .add_provider(EchoCmdFactory);
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap_err();
        assert_snapshot!(cmd.to_string(), @"internal errors encountered: oops!");
    }

    #[test]
    fn test_none_values_filtered() {
        let factory = CommandFactory::new()
            .add_provider(EchoCmdFactory)
            .add_provider(NoneProvider);
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap()
            .unwrap();
        assert_eq!(cmd.program(), OsStr::new("echo"));
    }

    #[test]
    fn test_none_returned_if_no_commands_found() {
        let factory = CommandFactory::new();
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap();
        assert!(cmd.is_none(), "expected no cmd, got {cmd:?}");
    }
}
