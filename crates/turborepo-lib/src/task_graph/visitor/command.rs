use std::{collections::HashSet, path::PathBuf};

use tracing::debug;
use turbopath::AbsoluteSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_microfrontends::MICROFRONTENDS_PACKAGES;
use turborepo_repository::package_graph::{PackageGraph, PackageInfo, PackageName};

use super::Error;
use crate::{
    engine::Engine, microfrontends::MicrofrontendsConfigs, opts::TaskArgs, process::Command,
    run::task_id::TaskId,
};

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

    pub fn add_provider(&mut self, provider: impl CommandProvider + 'a + Send) -> &mut Self {
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
    mfe_configs: Option<&'a MicrofrontendsConfigs>,
}

impl<'a> PackageGraphCommandProvider<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        task_args: TaskArgs<'a>,
        mfe_configs: Option<&'a MicrofrontendsConfigs>,
    ) -> Self {
        let package_manager_binary = which::which(package_graph.package_manager().command());
        Self {
            repo_root,
            package_graph,
            package_manager_binary,
            task_args,
            mfe_configs,
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

        // If the task has an associated proxy, then we indicate this to the underlying
        // task via an env var
        if self
            .mfe_configs
            .map_or(false, |mfe_configs| mfe_configs.task_has_mfe_proxy(task_id))
        {
            cmd.env("TURBO_TASK_HAS_MFE_PROXY", "true");
        }
        if let Some(port) = self
            .mfe_configs
            .and_then(|mfe_configs| mfe_configs.dev_task_port(task_id))
        {
            debug!("Found port {port} for {task_id}");
            cmd.env("TURBO_PORT", port.to_string());
        }

        // We always open stdin and the visitor will close it depending on task
        // configuration
        cmd.open_stdin();

        Ok(Some(cmd))
    }
}

#[derive(Debug)]
pub struct MicroFrontendProxyProvider<'a> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    tasks_in_graph: HashSet<TaskId<'a>>,
    mfe_configs: &'a MicrofrontendsConfigs,
}

impl<'a> MicroFrontendProxyProvider<'a> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        engine: &Engine,
        micro_frontends_configs: &'a MicrofrontendsConfigs,
    ) -> Self {
        let tasks_in_graph = engine
            .tasks()
            .filter_map(|task| match task {
                crate::engine::TaskNode::Task(task_id) => Some(task_id),
                crate::engine::TaskNode::Root => None,
            })
            .cloned()
            .collect();
        Self {
            repo_root,
            package_graph,
            tasks_in_graph,
            mfe_configs: micro_frontends_configs,
        }
    }

    fn dev_tasks(&self, task_id: &TaskId) -> Option<&HashSet<TaskId<'static>>> {
        (task_id.task() == "proxy")
            .then(|| self.mfe_configs.get(task_id.package()))
            .flatten()
    }

    fn package_info(&self, task_id: &TaskId) -> Result<&PackageInfo, Error> {
        self.package_graph
            .package_info(&PackageName::from(task_id.package()))
            .ok_or_else(|| Error::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })
    }

    fn has_custom_proxy(&self, task_id: &TaskId) -> Result<bool, Error> {
        let package_info = self.package_info(task_id)?;
        Ok(package_info.package_json.scripts.contains_key("proxy"))
    }
}

impl<'a> CommandProvider for MicroFrontendProxyProvider<'a> {
    fn command(
        &self,
        task_id: &TaskId,
        _environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, Error> {
        let Some(dev_tasks) = self.dev_tasks(task_id) else {
            return Ok(None);
        };
        let has_custom_proxy = self.has_custom_proxy(task_id)?;
        let package_info = self.package_info(task_id)?;
        let has_mfe_dependency = package_info
            .package_json
            .all_dependencies()
            .any(|(package, _version)| MICROFRONTENDS_PACKAGES.contains(&package.as_str()));
        if !has_mfe_dependency && !has_custom_proxy {
            let mfe_config_filename = self.mfe_configs.config_filename(task_id.package());
            return Err(Error::MissingMFEDependency {
                package: task_id.package().into(),
                mfe_config_filename: mfe_config_filename
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            });
        }
        let local_apps = dev_tasks
            .iter()
            .filter(|task| self.tasks_in_graph.contains(task))
            .map(|task| task.package());
        let package_dir = self.repo_root.resolve(package_info.package_path());
        let mfe_config_filename = self
            .mfe_configs
            .config_filename(task_id.package())
            .expect("every microfrontends default application should have configuration path");
        let mfe_path = self.repo_root.join_unix_path(mfe_config_filename);
        let cmd = if has_custom_proxy {
            let package_manager = self.package_graph.package_manager();
            let mut proxy_args = vec![mfe_path.as_str(), "--names"];
            proxy_args.extend(local_apps);
            let mut args = vec!["run", "proxy"];
            if let Some(sep) = package_manager.arg_separator(&proxy_args) {
                args.push(sep);
            }
            args.extend(proxy_args);

            let program = which::which(package_manager.command())?;
            let mut cmd = Command::new(&program);
            cmd.current_dir(package_dir).args(args).open_stdin();
            cmd
        } else {
            let mut args = vec!["proxy", mfe_path.as_str(), "--names"];
            args.extend(local_apps);

            // TODO: leverage package manager to find the local proxy
            let program = package_dir.join_components(&["node_modules", ".bin", "microfrontends"]);
            let mut cmd = Command::new(program.as_std_path());
            cmd.current_dir(package_dir).args(args).open_stdin();
            cmd
        };

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
        let mut factory = CommandFactory::new();
        factory
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
        let mut factory = CommandFactory::new();
        factory
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
        let mut factory = CommandFactory::new();
        factory
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
