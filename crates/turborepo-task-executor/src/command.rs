//! Command provider infrastructure for task execution.
//!
//! This module provides the trait and factory for creating commands to execute
//! tasks.

use std::collections::HashSet;

use tracing::debug;
use turbopath::{AbsoluteSystemPath, PathError, RelativeUnixPath};
use turborepo_env::EnvironmentVariableMap;
use turborepo_process::Command;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageInfo, PackageName},
    package_manager::PackageManager,
    toolchain::CompileCacheEndpoint,
};
use turborepo_task_id::TaskId;
use turborepo_types::TaskArgs;

use crate::MfeConfigProvider;

fn apply_environment(cmd: &mut Command, environment: &EnvironmentVariableMap) {
    cmd.env_clear();
    cmd.envs(environment.iter());
}

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
/// - `ToolchainCommandProvider` (resolves commands through the package's
///   toolchain: package.json scripts for JavaScript, cargo verbs for Cargo)
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
        environment: &EnvironmentVariableMap,
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
        environment: &EnvironmentVariableMap,
    ) -> Result<Option<Command>, E> {
        for provider in self.providers.iter() {
            let cmd = provider.command(task_id, environment)?;
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
    #[error("Missing microfrontends config path for package {package_name}.")]
    MissingMfeConfigPath { package_name: PackageName },
    #[error("Invalid microfrontends config path {path} for package {package_name}.")]
    InvalidMfeConfigPath {
        package_name: PackageName,
        path: String,
        #[source]
        source: PathError,
    },
    #[error("Unable to find package manager binary: {0}")]
    Which(#[from] which::Error),
    #[error("No toolchain '{toolchain}' registered for package {package_name}.")]
    MissingToolchain {
        toolchain: turborepo_repository::toolchain::ToolchainId,
        package_name: PackageName,
    },
    #[error(transparent)]
    Toolchain(#[from] turborepo_repository::toolchain::Error),
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

/// Command provider that resolves commands through the package's toolchain.
///
/// The toolchain owns command construction (JavaScript: run the package.json
/// script via the package manager; Cargo: `cargo <verb>` scoped to the
/// crate or workspace). This provider adapts the resolved [`TaskCommand`]
/// data into a process command and applies concerns that are not
/// toolchain-specific: the task environment, stdin policy, and
/// microfrontends proxy decorations.
#[derive(Debug)]
pub struct ToolchainCommandProvider<'a, M = crate::NoMfeConfig> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a PackageGraph,
    task_args: TaskArgs<'a>,
    mfe_configs: Option<&'a M>,
    /// A Turborepo-served compile cache endpoint (see
    /// [`turborepo_repository::toolchain::Toolchain::compile_cache_env`]), when
    /// one is running for this run.
    compile_cache: Option<&'a CompileCacheEndpoint>,
}

impl<'a, M: MfeConfigProvider> ToolchainCommandProvider<'a, M> {
    pub fn new(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a PackageGraph,
        task_args: TaskArgs<'a>,
        mfe_configs: Option<&'a M>,
        compile_cache: Option<&'a CompileCacheEndpoint>,
    ) -> Self {
        Self {
            repo_root,
            package_graph,
            task_args,
            mfe_configs,
            compile_cache,
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
    for ToolchainCommandProvider<'a, M>
{
    fn command(
        &self,
        task_id: &TaskId,
        environment: &EnvironmentVariableMap,
    ) -> Result<Option<Command>, E> {
        let workspace_info = self.package_info(task_id)?;
        let toolchain = self
            .package_graph
            .toolchains()
            .get(&workspace_info.toolchain)
            .ok_or_else(|| CommandProviderError::MissingToolchain {
                toolchain: workspace_info.toolchain.clone(),
                package_name: task_id.package().into(),
            })?;

        let spec = toolchain
            .task_command(
                self.repo_root,
                workspace_info,
                task_id.task(),
                self.task_args.args_for_task(task_id),
            )
            .map_err(CommandProviderError::from)?;
        let Some(spec) = spec else {
            return Ok(None);
        };

        let mut cmd = Command::new(spec.program);
        cmd.args(spec.args);
        cmd.current_dir(spec.cwd);
        if let Some(group) = spec.serial_group {
            cmd.serial_group(group);
        }

        apply_environment(&mut cmd, environment);

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

        // The toolchain decides how the injection composes with the task's
        // environment (competing configuration suppresses it, ambient
        // settings are tolerated) and returns exactly what to inject; see
        // `Toolchain::compile_cache_env`.
        if let Some(endpoint) = self.compile_cache {
            let vars = toolchain.compile_cache_env(endpoint, environment);
            if vars.is_empty() {
                debug!("no compile cache env to inject for {task_id}");
            }
            for (key, value) in vars {
                cmd.env(key, value);
            }
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
        environment: &EnvironmentVariableMap,
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
            .ok_or_else(|| CommandProviderError::MissingMfeConfigPath {
                package_name: task_id.to_workspace_name(),
            })?;
        let mfe_config_path = RelativeUnixPath::new(&mfe_config_filename).map_err(|source| {
            CommandProviderError::InvalidMfeConfigPath {
                package_name: task_id.to_workspace_name(),
                path: mfe_config_filename.clone(),
                source,
            }
        })?;
        let mfe_path = self.repo_root.join_unix_path(mfe_config_path);

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
            apply_environment(&mut cmd, environment);
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
            apply_environment(&mut cmd, environment);
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
    use std::{
        collections::{BTreeMap, HashMap},
        ffi::OsStr,
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
    use turborepo_errors::Spanned;
    use turborepo_repository::package_json::PackageJson;

    use super::*;

    struct MockPackageInfoProvider {
        package_info: PackageInfo,
        package_manager: PackageManager,
    }

    impl PackageInfoProvider for MockPackageInfoProvider {
        fn package_manager(&self) -> &PackageManager {
            &self.package_manager
        }

        fn package_info(&self, name: &PackageName) -> Option<&PackageInfo> {
            matches!(name, PackageName::Other(package) if package == "web")
                .then_some(&self.package_info)
        }
    }

    struct MockMfeConfig;

    impl MfeConfigProvider for MockMfeConfig {
        fn task_has_mfe_proxy(&self, _task_id: &TaskId) -> bool {
            false
        }

        fn dev_task_port(&self, _task_id: &TaskId) -> Option<u16> {
            None
        }

        fn task_uses_turborepo_proxy(&self, _task_id: &TaskId) -> bool {
            false
        }

        fn has_dev_task<'a>(&self, _task_ids: impl Iterator<Item = &'a TaskId<'static>>) -> bool {
            false
        }

        fn should_use_turborepo_proxy(&self) -> bool {
            false
        }

        fn dev_tasks(&self, package_name: &str) -> Option<Vec<(TaskId<'static>, String)>> {
            (package_name == "web")
                .then(|| vec![(TaskId::new("docs", "dev"), "docs-app".to_owned())])
        }

        fn config_filename(&self, package_name: &str) -> Option<String> {
            (package_name == "web").then(|| "web/microfrontends.json".to_owned())
        }
    }

    fn create_test_repo() -> (TempDir, AbsoluteSystemPathBuf, PathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tempdir.path().to_string_lossy().to_string()).unwrap();
        let package_dir = tempdir.path().join("web");
        fs::create_dir_all(&package_dir).unwrap();

        (tempdir, repo_root, package_dir)
    }

    fn inherited_env_name() -> &'static str {
        let name = if cfg!(windows) { "USERNAME" } else { "USER" };
        assert!(
            std::env::var_os(name).is_some(),
            "{name} must be set for this environment filtering regression test"
        );
        name
    }

    fn filtered_environment() -> EnvironmentVariableMap {
        EnvironmentVariableMap::from(HashMap::from([
            ("ALLOWED_VAR".to_owned(), "allowed".to_owned()),
            ("PATH".to_owned(), std::env::var("PATH").unwrap()),
        ]))
    }

    fn package_info(package_json: PackageJson) -> PackageInfo {
        PackageInfo {
            package_json,
            package_json_path: AnchoredSystemPathBuf::from_raw("web/package.json").unwrap(),
            unresolved_external_dependencies: None,
            transitive_dependencies: None,
            ..Default::default()
        }
    }

    fn proxy_command(
        repo_root: &AbsoluteSystemPathBuf,
        package_info_provider: &MockPackageInfoProvider,
        environment: &EnvironmentVariableMap,
    ) -> Command {
        let mfe_config = MockMfeConfig;
        let tasks = [TaskId::new("docs", "dev"), TaskId::new("web", "proxy")];
        let provider = MicroFrontendProxyProvider::new(
            repo_root,
            package_info_provider,
            tasks.iter(),
            &mfe_config,
        );

        CommandProvider::<CommandProviderError>::command(
            &provider,
            &TaskId::new("web", "proxy"),
            environment,
        )
        .unwrap()
        .unwrap()
    }

    async fn command_stdout(cmd: Command) -> String {
        let output = tokio::process::Command::from(cmd).output().await.unwrap();
        assert!(
            output.status.success(),
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).unwrap()
    }

    fn assert_filtered_environment(stdout: &str, inherited_env: &str) {
        assert!(
            stdout.lines().any(|line| line == "ALLOWED_VAR=allowed"),
            "allowed env var missing from stdout: {stdout}"
        );
        assert!(
            stdout
                .lines()
                .any(|line| line == format!("{inherited_env}=")),
            "inherited env var leaked into stdout: {stdout}"
        );
    }

    fn write_custom_proxy_package(package_dir: &Path, inherited_env: &str) {
        fs::write(
            package_dir.join("package.json"),
            r#"{"scripts":{"proxy":"node print-env.js"}}"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("print-env.js"),
            format!(
                "const inherited = {inherited_env:?};\nconsole.log('ALLOWED_VAR=' + \
                 (process.env.ALLOWED_VAR ?? ''));\nconsole.log(inherited + '=' + \
                 (process.env[inherited] ?? ''));\n"
            ),
        )
        .unwrap();
    }

    fn write_microfrontends_binary(package_dir: &Path, inherited_env: &str) {
        let bin_dir = package_dir.join("node_modules").join(".bin");
        fs::create_dir_all(&bin_dir).unwrap();

        let binary_path = bin_dir.join(if cfg!(windows) {
            "microfrontends.cmd"
        } else {
            "microfrontends"
        });
        let script = if cfg!(windows) {
            format!(
                "@echo off\r\necho ALLOWED_VAR=%ALLOWED_VAR%\r\necho \
                 {inherited_env}=%{inherited_env}%\r\n"
            )
        } else {
            format!(
                "#!/bin/sh\nprintf 'ALLOWED_VAR=%s\\n' \"${{ALLOWED_VAR-}}\"\nprintf \
                 '{inherited_env}=%s\\n' \"${{{inherited_env}-}}\"\n"
            )
        };
        fs::write(&binary_path, script).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(&binary_path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&binary_path, permissions).unwrap();
        }
    }

    struct EchoProvider;

    impl CommandProvider<String> for EchoProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: &EnvironmentVariableMap,
        ) -> Result<Option<Command>, String> {
            Ok(Some(Command::new("echo")))
        }
    }

    struct NoneProvider;

    impl CommandProvider<String> for NoneProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: &EnvironmentVariableMap,
        ) -> Result<Option<Command>, String> {
            Ok(None)
        }
    }

    struct ErrProvider;

    impl CommandProvider<String> for ErrProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: &EnvironmentVariableMap,
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
            .command(&task_id, &EnvironmentVariableMap::default())
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
            .command(&task_id, &EnvironmentVariableMap::default())
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
            .command(&task_id, &EnvironmentVariableMap::default())
            .unwrap()
            .unwrap();
        assert_eq!(cmd.program(), OsStr::new("echo"));
    }

    #[test]
    fn test_none_returned_if_no_commands_found() {
        let factory: CommandFactory<String> = CommandFactory::new();
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, &EnvironmentVariableMap::default())
            .unwrap();
        assert!(cmd.is_none(), "expected no cmd, got {cmd:?}");
    }

    #[tokio::test]
    async fn test_custom_microfrontend_proxy_command_applies_filtered_environment() {
        let (_tempdir, repo_root, package_dir) = create_test_repo();
        let inherited_env = inherited_env_name();
        write_custom_proxy_package(&package_dir, inherited_env);
        let package_info_provider = MockPackageInfoProvider {
            package_info: package_info(PackageJson {
                scripts: BTreeMap::from([(
                    "proxy".to_owned(),
                    Spanned::new("node print-env.js".to_owned()),
                )]),
                ..Default::default()
            }),
            package_manager: PackageManager::Npm,
        };
        let cmd = proxy_command(&repo_root, &package_info_provider, &filtered_environment());
        let stdout = command_stdout(cmd).await;

        assert_filtered_environment(&stdout, inherited_env);
    }

    #[tokio::test]
    async fn test_microfrontends_binary_proxy_command_applies_filtered_environment() {
        let (_tempdir, repo_root, package_dir) = create_test_repo();
        let inherited_env = inherited_env_name();
        write_microfrontends_binary(&package_dir, inherited_env);
        let package_info_provider = MockPackageInfoProvider {
            package_info: package_info(PackageJson {
                dependencies: Some(BTreeMap::from([(
                    "@vercel/microfrontends".to_owned(),
                    "1.0.0".to_owned(),
                )])),
                ..Default::default()
            }),
            package_manager: PackageManager::Npm,
        };
        let cmd = proxy_command(&repo_root, &package_info_provider, &filtered_environment());
        let stdout = command_stdout(cmd).await;

        assert_filtered_environment(&stdout, inherited_env);
    }
}
