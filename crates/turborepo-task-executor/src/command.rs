//! Command provider infrastructure for task execution.
//!
//! This module provides the trait and factory for creating commands to execute
//! tasks.

use std::{collections::HashSet, path::PathBuf};

use tracing::debug;
use turbopath::{AbsoluteSystemPath, RelativeUnixPath};
use turborepo_env::EnvironmentVariableMap;
use turborepo_process::Command;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageInfo, PackageName},
    package_manager::PackageManager,
};
use turborepo_task_id::TaskId;
use turborepo_types::TaskArgs;

use crate::MfeConfigProvider;

/// Trait for providing commands to execute tasks.
///
/// Implementors of this trait are responsible for determining how to execute
/// a given task, including:
/// - Finding the appropriate script/binary to run
/// - Setting up the working directory
/// - Configuring environment variables
///
/// # Type Parameters
/// - `E`: The error type returned when command creation fails
///
/// # Implementors
/// - `PackageGraphCommandProvider` in turborepo-lib (executes package.json
///   scripts)
/// - `MicroFrontendProxyProvider` in turborepo-lib (starts MFE proxy)
pub trait CommandProvider<E> {
    /// Create a command for the given task.
    ///
    /// Returns `Ok(Some(command))` if the provider can handle this task,
    /// `Ok(None)` if this provider doesn't handle this task (allows
    /// fallthrough), or `Err(e)` if an error occurred.
    fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, E>;
}

/// A collection of command providers.
///
/// Will attempt to find a command from any of the providers it contains.
/// Ordering of the providers matters as the first present command will be
/// returned. Any errors returned by the providers will be immediately returned.
///
/// # Type Parameters
/// - `'a`: Lifetime of the providers
/// - `E`: The error type returned by providers
pub struct CommandFactory<'a, E> {
    providers: Vec<Box<dyn CommandProvider<E> + 'a + Send>>,
}

impl<'a, E> CommandFactory<'a, E> {
    /// Create a new empty command factory.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Add a command provider to this factory.
    ///
    /// Providers are checked in the order they are added.
    pub fn add_provider(&mut self, provider: impl CommandProvider<E> + 'a + Send) -> &mut Self {
        self.providers.push(Box::new(provider));
        self
    }

    /// Get a command for the given task.
    ///
    /// Iterates through providers in order until one returns a command.
    /// Returns `Ok(None)` if no provider can handle the task.
    pub fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, E> {
        for provider in self.providers.iter() {
            let cmd = provider.command(task_id, environment.clone())?;
            if cmd.is_some() {
                return Ok(cmd);
            }
        }
        Ok(None)
    }
}

impl<'a, E> Default for CommandFactory<'a, E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Error type for package graph command provider operations.
#[derive(Debug, thiserror::Error)]
pub enum CommandProviderError {
    #[error("Cannot find package {package_name} for task {task_id}.")]
    MissingPackage {
        package_name: PackageName,
        task_id: TaskId<'static>,
    },
    #[error("Unable to find package manager binary: {0}")]
    Which(#[from] which::Error),
}

/// A trait for fetching package information required to execute commands
pub trait PackageInfoProvider {
    fn package_manager(&self) -> &PackageManager;

    fn package_info(&self, name: &PackageName) -> Option<&PackageInfo>;
}

impl PackageInfoProvider for PackageGraph {
    fn package_manager(&self) -> &PackageManager {
        PackageGraph::package_manager(self)
    }

    fn package_info(&self, name: &PackageName) -> Option<&PackageInfo> {
        PackageGraph::package_info(self, name)
    }
}

/// Command provider that creates commands from package.json scripts.
///
/// This provider looks up the task's script in the package's package.json
/// and creates a command to execute it via the package manager.
#[derive(Debug)]
pub struct PackageGraphCommandProvider<'a, M = crate::NoMfeConfig> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    package_manager_binary: Result<PathBuf, which::Error>,
    task_args: TaskArgs<'a>,
    mfe_configs: Option<&'a M>,
}

impl<'a, M: MfeConfigProvider> PackageGraphCommandProvider<'a, M> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        task_args: TaskArgs<'a>,
        mfe_configs: Option<&'a M>,
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

    fn package_info(&self, task_id: &TaskId) -> Result<&PackageInfo, CommandProviderError> {
        self.package_graph
            .package_info(&PackageName::from(task_id.package()))
            .ok_or_else(|| CommandProviderError::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })
    }
}

impl<'a, M: MfeConfigProvider, E: From<CommandProviderError>> CommandProvider<E>
    for PackageGraphCommandProvider<'a, M>
{
    fn command(
        &self,
        task_id: &TaskId,
        environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, E> {
        let workspace_info = self.package_info(task_id)?;

        // bail if the script doesn't exist or is empty
        if workspace_info
            .package_json
            .scripts
            .get(task_id.task())
            .is_none_or(|script| script.is_empty())
        {
            return Ok(None);
        }
        let package_manager_binary = self
            .package_manager_binary
            .as_deref()
            .map_err(|e| CommandProviderError::from(*e))?;
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
            .is_some_and(|mfe_configs| mfe_configs.task_has_mfe_proxy(task_id))
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

        // If this task is using the Turborepo proxy (not @vercel/microfrontends),
        // set the local port value in an env var
        if let Some(mfe_configs) = self.mfe_configs
            && mfe_configs.task_uses_turborepo_proxy(task_id)
            && let Some(port) = mfe_configs.dev_task_port(task_id)
        {
            cmd.env("TURBO_MFE_PORT", port.to_string());
        }

        // We always open stdin and the visitor will close it depending on task
        // configuration
        cmd.open_stdin();

        Ok(Some(cmd))
    }
}

/// Command provider for microfrontends proxy tasks.
///
/// This provider handles the `proxy` task for microfrontends configurations,
/// creating commands to start the proxy server.
#[derive(Debug)]
pub struct MicroFrontendProxyProvider<'a, T, M> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a T,
    tasks_in_graph: HashSet<TaskId<'a>>,
    mfe_configs: &'a M,
}

impl<'a, T: PackageInfoProvider, M: MfeConfigProvider> MicroFrontendProxyProvider<'a, T, M> {
    /// Creates a new `MicroFrontendProxyProvider`.
    ///
    /// # Arguments
    /// * `repo_root` - The root of the repository
    /// * `package_graph` - The package graph provider
    /// * `tasks_in_graph` - Iterator of tasks that are part of the current
    ///   execution graph
    /// * `micro_frontends_configs` - The microfrontends configuration
    pub fn new<'b>(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a T,
        tasks_in_graph: impl Iterator<Item = &'b TaskId<'static>>,
        micro_frontends_configs: &'a M,
    ) -> Self {
        Self {
            repo_root,
            package_graph,
            tasks_in_graph: tasks_in_graph.cloned().collect(),
            mfe_configs: micro_frontends_configs,
        }
    }

    fn dev_tasks(&self, task_id: &TaskId) -> Option<Vec<(TaskId<'static>, String)>> {
        (task_id.task() == "proxy").then(|| self.mfe_configs.dev_tasks(task_id.package()))?
    }

    fn package_info(&self, task_id: &TaskId) -> Result<&PackageInfo, CommandProviderError> {
        self.package_graph
            .package_info(&PackageName::from(task_id.package()))
            .ok_or_else(|| CommandProviderError::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })
    }

    fn has_custom_proxy(&self, task_id: &TaskId) -> Result<bool, CommandProviderError> {
        let package_info = self.package_info(task_id)?;
        Ok(package_info.package_json.scripts.contains_key("proxy"))
    }
}

impl<'a, T: PackageInfoProvider + Send + Sync, M: MfeConfigProvider, E: From<CommandProviderError>>
    CommandProvider<E> for MicroFrontendProxyProvider<'a, T, M>
{
    fn command(
        &self,
        task_id: &TaskId,
        _environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, E> {
        debug!(
            "MicroFrontendProxyProvider::command - called for task: {}",
            task_id
        );

        let Some(dev_tasks) = self.dev_tasks(task_id) else {
            debug!(
                "MicroFrontendProxyProvider::command - no dev tasks found for {}",
                task_id
            );
            return Ok(None);
        };

        debug!(
            "MicroFrontendProxyProvider::command - found {} dev tasks for {}",
            dev_tasks.len(),
            task_id
        );

        let has_custom_proxy = self.has_custom_proxy(task_id)?;
        let package_info = self.package_info(task_id)?;

        // Check if package depends on @vercel/microfrontends
        const MICROFRONTENDS_PACKAGE: &str = "@vercel/microfrontends";
        let has_mfe_dependency = package_info
            .package_json
            .all_dependencies()
            .any(|(package, _version)| package.as_str() == MICROFRONTENDS_PACKAGE);

        debug!(
            "MicroFrontendProxyProvider::command - has_custom_proxy: {}, has_mfe_dependency: {}",
            has_custom_proxy, has_mfe_dependency
        );

        let local_apps: Vec<&str> = dev_tasks
            .iter()
            .filter_map(|(task, app_name)| {
                self.tasks_in_graph
                    .contains(task)
                    .then_some(app_name.as_str())
            })
            .collect();
        let package_dir = self.repo_root.resolve(package_info.package_path());
        let mfe_config_filename = self
            .mfe_configs
            .config_filename(task_id.package())
            .expect("every microfrontends default application should have configuration path");
        let mfe_path = self.repo_root.join_unix_path(
            RelativeUnixPath::new(&mfe_config_filename)
                .expect("config_filename should return a valid relative unix path"),
        );

        let cmd = if has_custom_proxy {
            debug!("MicroFrontendProxyProvider::command - using custom proxy script");
            let package_manager = self.package_graph.package_manager();
            let mut proxy_args: Vec<&str> = vec![mfe_path.as_str(), "--names"];
            proxy_args.extend(local_apps);
            let mut args = vec!["run", "proxy"];
            if let Some(sep) = package_manager.arg_separator(&proxy_args) {
                args.push(sep);
            }
            args.extend(proxy_args);

            let program =
                which::which(package_manager.command()).map_err(CommandProviderError::from)?;
            let mut cmd = Command::new(&program);
            cmd.current_dir(package_dir).args(args).open_stdin();
            Some(cmd)
        } else if has_mfe_dependency {
            debug!("MicroFrontendProxyProvider::command - using @vercel/microfrontends proxy");
            let mut args: Vec<&str> = vec!["proxy", mfe_path.as_str(), "--names"];
            args.extend(local_apps);

            // On Windows, a package manager will rework the binary to be a .cmd extension
            // since that's what Windows needs
            let bin_name = if cfg!(windows) {
                "microfrontends.cmd"
            } else {
                "microfrontends"
            };

            // TODO: leverage package manager to find the local proxy
            let program = package_dir.join_components(&["node_modules", ".bin", bin_name]);
            let mut cmd = Command::new(program.as_std_path());
            cmd.current_dir(package_dir).args(args).open_stdin();
            Some(cmd)
        } else {
            debug!("MicroFrontendProxyProvider::command - using Turborepo built-in proxy");
            // No custom proxy and no @vercel/microfrontends dependency.
            // The Turborepo proxy will be started separately.
            None
        };

        debug!(
            "MicroFrontendProxyProvider::command - returning command: {}",
            if cmd.is_some() { "Some" } else { "None" }
        );

        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    struct EchoProvider;

    impl CommandProvider<String> for EchoProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, String> {
            Ok(Some(Command::new("echo")))
        }
    }

    struct NoneProvider;

    impl CommandProvider<String> for NoneProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, String> {
            Ok(None)
        }
    }

    struct ErrProvider;

    impl CommandProvider<String> for ErrProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, String> {
            Err("error".to_string())
        }
    }

    #[test]
    fn test_first_present_cmd_returned() {
        let mut factory = CommandFactory::new();
        factory.add_provider(EchoProvider).add_provider(ErrProvider);
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
        factory.add_provider(ErrProvider).add_provider(EchoProvider);
        let task_id = TaskId::new("foo", "build");
        let err = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap_err();
        assert_eq!(err, "error");
    }

    #[test]
    fn test_none_values_filtered() {
        let mut factory = CommandFactory::new();
        factory
            .add_provider(NoneProvider)
            .add_provider(EchoProvider);
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap()
            .unwrap();
        assert_eq!(cmd.program(), OsStr::new("echo"));
    }

    #[test]
    fn test_none_returned_if_no_commands_found() {
        let factory: CommandFactory<String> = CommandFactory::new();
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, EnvironmentVariableMap::default())
            .unwrap();
        assert!(cmd.is_none(), "expected no cmd, got {cmd:?}");
    }
}
