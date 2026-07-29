//! Command provider infrastructure for task execution.
//!
//! This module provides the trait and factory for creating commands to execute
//! tasks.

use std::collections::{HashMap, HashSet};

use tracing::debug;
use turbopath::{PathError, RelativeUnixPath};
use turborepo_env::EnvironmentVariableMap;
use turborepo_process::Command;
use turborepo_repository::{
    native_tasks::NativeCommandTemplate,
    package_graph::{PackageGraph, PackageName, PackageTaskContext},
    toolchain::CompileCacheEndpoint,
};
use turborepo_task_id::TaskId;
use turborepo_types::{TaskArgs, TaskCommandOverride};

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
    #[error("Missing compatibility payload for package {package_name}.")]
    MissingPackagePayload { package_name: PackageName },
    #[error("Package {package_name} is not a JavaScript package execution scope.")]
    InvalidMfePackageContext { package_name: PackageName },
    #[error("Package directory {directory} is outside repository root {repository_root}.")]
    PackageDirectoryOutsideRepository {
        repository_root: String,
        directory: String,
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
    #[error("Unsafe microfrontends config path {path} for package {package_name}.")]
    UnsafeMfeConfigPath {
        package_name: PackageName,
        path: String,
    },
    #[error("Microfrontends config path {path} is outside repository root {repository_root}.")]
    MfeConfigOutsideRepository {
        repository_root: String,
        path: String,
    },
    #[error("Microfrontends require a package manager")]
    MissingPackageManager,
    #[error("Unable to find package manager binary: {0}")]
    Which(#[from] which::Error),
    #[error(transparent)]
    Toolchain(#[from] turborepo_repository::toolchain::Error),
}

/// Command provider that resolves commands through the native-task catalog.
///
/// The catalog owns native command templates (JavaScript: package-manager
/// `run <script>`; Cargo: `cargo <verb>` scoped to crate/workspace). This
/// provider adapts the resolved [`TaskCommand`] data into a process command
/// and applies concerns that are not native-task-specific: the task
/// environment, stdin policy, and microfrontends proxy decorations.
#[derive(Debug)]
pub struct ToolchainCommandProvider<'a, M = crate::NoMfeConfig> {
    package_graph: &'a PackageGraph,
    task_args: TaskArgs<'a>,
    mfe_configs: Option<&'a M>,
    /// A Turborepo-served compile cache endpoint (see
    /// [`turborepo_repository::toolchain::Toolchain::compile_cache_env`]), when
    /// one is running for this run.
    compile_cache: Option<&'a CompileCacheEndpoint>,
    /// Resolved `command` overrides by task, from the engine's task
    /// definitions. An argv replaces the native catalog resolution; an
    /// opt-out makes the task an explicit no-op.
    command_overrides: HashMap<TaskId<'static>, TaskCommandOverride>,
    /// Lazily resolved package-manager binary path for JS framing.
    package_manager_binary: std::sync::OnceLock<Result<std::path::PathBuf, which::Error>>,
    /// Lazily resolved cargo binary path for Cargo framing.
    cargo_binary: std::sync::OnceLock<Result<std::path::PathBuf, which::Error>>,
}

impl<'a, M: MfeConfigProvider> ToolchainCommandProvider<'a, M> {
    pub fn new(
        package_graph: &'a PackageGraph,
        task_args: TaskArgs<'a>,
        mfe_configs: Option<&'a M>,
        compile_cache: Option<&'a CompileCacheEndpoint>,
        command_overrides: HashMap<TaskId<'static>, TaskCommandOverride>,
    ) -> Self {
        Self {
            package_graph,
            task_args,
            mfe_configs,
            compile_cache,
            command_overrides,
            package_manager_binary: std::sync::OnceLock::new(),
            cargo_binary: std::sync::OnceLock::new(),
        }
    }

    fn package_manager_binary(&self) -> Result<Option<&std::path::Path>, CommandProviderError> {
        let Some(package_manager) = self.package_graph.package_manager() else {
            return Ok(None);
        };
        match self
            .package_manager_binary
            .get_or_init(|| which::which(package_manager.command()))
        {
            Ok(path) => Ok(Some(path.as_path())),
            Err(error) => Err(CommandProviderError::Which(*error)),
        }
    }

    fn cargo_binary(&self) -> Result<Option<&std::path::Path>, CommandProviderError> {
        match self.cargo_binary.get_or_init(|| which::which("cargo")) {
            Ok(path) => Ok(Some(path.as_path())),
            Err(_) => Ok(None),
        }
    }

    fn package_context(
        &self,
        task_id: &TaskId,
    ) -> Result<PackageTaskContext<'_>, CommandProviderError> {
        self.package_graph
            .package_task_context(&PackageName::from(task_id.package()))
            .ok_or_else(|| CommandProviderError::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })
    }
}

fn should_inject_toolchain_compile_cache(command_override: Option<&TaskCommandOverride>) -> bool {
    command_override.is_none()
}

impl<'a, M: MfeConfigProvider, E: From<CommandProviderError>> CommandProvider<E>
    for ToolchainCommandProvider<'a, M>
{
    fn command(
        &self,
        task_id: &TaskId,
        environment: &EnvironmentVariableMap,
    ) -> Result<Option<Command>, E> {
        let package_context = self.package_context(task_id)?;

        // A resolved `command` override is authoritative in both
        // directions: an opt-out is an explicit no-op (same outcome as a
        // missing script), and an argv replaces the toolchain's own
        // resolution while the toolchain keeps framing it.
        let command_override = self.command_overrides.get(task_id);
        let override_command = match command_override {
            Some(TaskCommandOverride::OptOut) => return Ok(None),
            Some(TaskCommandOverride::Argv(argv)) => Some(argv.as_slice()),
            None => None,
        };

        // A pure-native repository still has Turbo's root task namespace, but
        // no root language toolchain. Explicit root command overrides are
        // generic argv and execute directly at the knowledge-backed repo root.
        let Some(toolchain_id) = package_context.toolchain() else {
            let Some(override_command) = override_command else {
                return Ok(None);
            };
            let Some(spec) = turborepo_repository::toolchain::override_task_command(
                &package_context,
                override_command,
                self.task_args.args_for_task(task_id),
                None,
            ) else {
                return Ok(None);
            };
            let mut cmd = Command::new(spec.program);
            cmd.args(spec.args).current_dir(spec.cwd);
            apply_environment(&mut cmd, environment);
            cmd.open_stdin();
            return Ok(Some(cmd));
        };

        let spec = if let Some(native_task) = package_context.native_tasks().get(task_id.task()) {
            let (package_manager_binary, cargo_binary) = if override_command.is_some() {
                (None, None)
            } else {
                match native_task.command() {
                    Some(NativeCommandTemplate::JavaScriptPackageManagerRun { .. }) => {
                        (self.package_manager_binary()?, None)
                    }
                    Some(NativeCommandTemplate::Cargo { .. }) => (None, self.cargo_binary()?),
                    None => (None, None),
                }
            };
            turborepo_repository::native_tasks::resolve_task_command(
                &package_context,
                native_task,
                self.package_graph.package_manager(),
                package_manager_binary,
                cargo_binary,
                self.task_args.args_for_task(task_id),
                override_command,
            )
            .map_err(|error| {
                CommandProviderError::Toolchain(turborepo_repository::toolchain::Error::Failed(
                    Box::new(error),
                ))
            })?
        } else if let Some(override_command) = override_command {
            let serial_group = (toolchain_id
                == &turborepo_repository::toolchain::ToolchainId::RUST
                && override_command.first().map(String::as_str) == Some("cargo"))
            .then(|| "cargo".to_string());
            turborepo_repository::toolchain::override_task_command(
                &package_context,
                override_command,
                self.task_args.args_for_task(task_id),
                serial_group,
            )
        } else {
            None
        };
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

        // Compile-cache injection remains toolchain-owned until environment
        // contracts move; look up the toolchain only for that decoration.
        if should_inject_toolchain_compile_cache(command_override)
            && let Some(endpoint) = self.compile_cache
            && let Some(toolchain) = self.package_graph.toolchains().get(toolchain_id)
        {
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
pub struct MicroFrontendProxyProvider<'a, M> {
    package_graph: &'a PackageGraph,
    tasks_in_graph: HashSet<TaskId<'a>>,
    mfe_configs: &'a M,
}

impl<'a, M: MfeConfigProvider> MicroFrontendProxyProvider<'a, M> {
    /// Creates a new `MicroFrontendProxyProvider`.
    ///
    /// # Arguments
    /// * `package_graph` - The package graph provider
    /// * `tasks_in_graph` - Iterator of tasks that are part of the current
    ///   execution graph
    /// * `micro_frontends_configs` - The microfrontends configuration
    pub fn new<'b>(
        package_graph: &'a PackageGraph,
        tasks_in_graph: impl Iterator<Item = &'b TaskId<'static>>,
        micro_frontends_configs: &'a M,
    ) -> Self {
        Self {
            package_graph,
            tasks_in_graph: tasks_in_graph.cloned().collect(),
            mfe_configs: micro_frontends_configs,
        }
    }

    fn dev_tasks(&self, task_id: &TaskId) -> Option<Vec<(TaskId<'static>, String)>> {
        (task_id.task() == "proxy").then(|| self.mfe_configs.dev_tasks(task_id.package()))?
    }

    fn package_context(
        &self,
        task_id: &TaskId,
    ) -> Result<PackageTaskContext<'_>, CommandProviderError> {
        self.package_graph
            .package_task_context(&PackageName::from(task_id.package()))
            .ok_or_else(|| CommandProviderError::MissingPackage {
                package_name: task_id.package().into(),
                task_id: task_id.clone().into_owned(),
            })
    }

    fn validated_package_context(
        &self,
        task_id: &TaskId,
    ) -> Result<PackageTaskContext<'_>, CommandProviderError> {
        let context = self.package_context(task_id)?;
        Self::validate_package_context(context)
    }

    fn validate_package_context(
        context: PackageTaskContext<'_>,
    ) -> Result<PackageTaskContext<'_>, CommandProviderError> {
        let package_directory = context.repository_root().resolve(context.directory());
        if !context.repository_root().contains(&package_directory) {
            return Err(CommandProviderError::PackageDirectoryOutsideRepository {
                repository_root: context.repository_root().to_string(),
                directory: package_directory.to_string(),
            });
        }
        if context.toolchain() != Some(&turborepo_repository::toolchain::ToolchainId::JAVASCRIPT)
            || context.kind()
                == turborepo_repository::package_graph::PackageTaskContextKind::Aggregate
        {
            return Err(CommandProviderError::InvalidMfePackageContext {
                package_name: context.package().clone(),
            });
        }
        if context.package_info().is_none() {
            return Err(CommandProviderError::MissingPackagePayload {
                package_name: context.package().clone(),
            });
        }
        Ok(context)
    }
}

impl<'a, M: MfeConfigProvider, E: From<CommandProviderError>> CommandProvider<E>
    for MicroFrontendProxyProvider<'a, M>
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

        let package_context = self.validated_package_context(task_id)?;
        let package_info = package_context.package_info().ok_or_else(|| {
            CommandProviderError::MissingPackagePayload {
                package_name: package_context.package().clone(),
            }
        })?;
        let has_custom_proxy = package_info.package_json.scripts.contains_key("proxy");

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
        let package_dir = package_context
            .repository_root()
            .resolve(package_context.directory());
        let mfe_config_filename = self
            .mfe_configs
            .config_filename(package_context.package().as_ref())
            .ok_or_else(|| CommandProviderError::MissingMfeConfigPath {
                package_name: package_context.package().clone(),
            })?;
        let unsafe_path = mfe_config_filename.starts_with('/')
            || mfe_config_filename.contains('\\')
            || mfe_config_filename
                .split('/')
                .any(|component| component == "..")
            || mfe_config_filename
                .split('/')
                .next()
                .is_some_and(|component| component.contains(':'));
        if unsafe_path {
            return Err(CommandProviderError::UnsafeMfeConfigPath {
                package_name: package_context.package().clone(),
                path: mfe_config_filename,
            }
            .into());
        }
        let mfe_config_path = RelativeUnixPath::new(&mfe_config_filename).map_err(|source| {
            CommandProviderError::InvalidMfeConfigPath {
                package_name: package_context.package().clone(),
                path: mfe_config_filename.clone(),
                source,
            }
        })?;
        let mfe_path = package_context
            .repository_root()
            .join_unix_path(mfe_config_path);
        if !package_context.repository_root().contains(&mfe_path) {
            return Err(CommandProviderError::MfeConfigOutsideRepository {
                repository_root: package_context.repository_root().to_string(),
                path: mfe_path.to_string(),
            }
            .into());
        }

        let cmd = if has_custom_proxy {
            debug!("MicroFrontendProxyProvider::command - using custom proxy script");
            let package_manager = self
                .package_graph
                .package_manager()
                .ok_or(CommandProviderError::MissingPackageManager)?;
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
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_errors::Spanned;
    use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};
    use turborepo_task_id::TaskName;

    use super::*;

    struct MockMfeConfig(&'static str);

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
            matches!(package_name, "web" | "//")
                .then(|| vec![(TaskId::new("docs", "dev"), "docs-app".to_owned())])
        }

        fn config_filename(&self, package_name: &str) -> Option<String> {
            matches!(package_name, "web" | "//").then(|| self.0.to_owned())
        }
    }

    #[test]
    fn command_override_suppresses_toolchain_compile_cache() {
        assert!(should_inject_toolchain_compile_cache(None));
        assert!(!should_inject_toolchain_compile_cache(Some(
            &TaskCommandOverride::Argv(vec!["node".to_string()])
        )));
        assert!(!should_inject_toolchain_compile_cache(Some(
            &TaskCommandOverride::OptOut
        )));
    }

    #[tokio::test]
    async fn command_override_does_not_resolve_package_manager_binary() {
        let (_tempdir, repo_root, package_dir) = create_test_repo();
        let package_graph = package_graph(
            &repo_root,
            &package_dir,
            PackageJson {
                scripts: BTreeMap::from([(
                    "build".to_owned(),
                    Spanned::new("next build".to_owned()),
                )]),
                ..Default::default()
            },
        )
        .await;
        let task_id = TaskId::new("web", "build").into_owned();
        let provider = ToolchainCommandProvider::<crate::NoMfeConfig>::new(
            &package_graph,
            TaskArgs::new(&[], &[]),
            None,
            None,
            HashMap::from([(
                task_id.clone(),
                TaskCommandOverride::Argv(vec!["custom-build".to_string()]),
            )]),
        );
        provider
            .package_manager_binary
            .set(Err(which::Error::CannotFindBinaryPath))
            .unwrap();

        let command = CommandProvider::<CommandProviderError>::command(
            &provider,
            &task_id,
            &EnvironmentVariableMap::default(),
        )
        .unwrap();
        assert!(command.is_some(), "override should bypass binary lookup");
    }

    fn create_test_repo() -> (TempDir, AbsoluteSystemPathBuf, PathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let repo_root =
            AbsoluteSystemPathBuf::new(tempdir.path().to_string_lossy().to_string()).unwrap();
        let package_dir = tempdir.path().join("apps").join("site").join("web");
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

    async fn package_graph(
        repo_root: &AbsoluteSystemPathBuf,
        package_dir: &Path,
        package_json: PackageJson,
    ) -> PackageGraph {
        let mut package_json = package_json;
        package_json.name = Some(Spanned::new("web".to_owned()));
        let package_path = AbsoluteSystemPathBuf::try_from(package_dir)
            .unwrap()
            .join_component("package.json");
        PackageGraph::builder(repo_root, PackageJson::default())
            .with_package_manager(PackageManager::Npm)
            .with_package_jsons(Some(HashMap::from([(package_path, package_json)])))
            .with_allow_no_package_manager(true)
            .build()
            .await
            .unwrap()
    }

    async fn root_package_graph(
        repo_root: &AbsoluteSystemPathBuf,
        root_json: PackageJson,
    ) -> PackageGraph {
        PackageGraph::builder(repo_root, root_json)
            .with_package_manager(PackageManager::Npm)
            .with_package_jsons(Some(HashMap::new()))
            .with_allow_no_package_manager(true)
            .build()
            .await
            .unwrap()
    }

    fn proxy_command(
        package_graph: &PackageGraph,
        environment: &EnvironmentVariableMap,
    ) -> Command {
        let mfe_config = MockMfeConfig("configs/microfrontends.json");
        proxy_command_result(package_graph, environment, &mfe_config, "web")
            .unwrap()
            .unwrap()
    }

    fn proxy_command_result(
        package_graph: &PackageGraph,
        environment: &EnvironmentVariableMap,
        mfe_config: &MockMfeConfig,
        package: &str,
    ) -> Result<Option<Command>, CommandProviderError> {
        let tasks = [TaskId::new("docs", "dev"), TaskId::new("web", "proxy")];
        let provider = MicroFrontendProxyProvider::new(package_graph, tasks.iter(), mfe_config);

        let task_id = if package == "//" {
            TaskId::from_graph(&PackageName::Root, &TaskName::from("proxy"))
        } else {
            TaskId::new(package, "proxy").into_owned()
        };
        assert_eq!(task_id.package(), package);
        CommandProvider::<CommandProviderError>::command(&provider, &task_id, environment)
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
        let package_graph = package_graph(
            &repo_root,
            &package_dir,
            PackageJson {
                scripts: BTreeMap::from([(
                    "proxy".to_owned(),
                    Spanned::new("node print-env.js".to_owned()),
                )]),
                ..Default::default()
            },
        )
        .await;
        let cmd = proxy_command(&package_graph, &filtered_environment());
        assert!(
            cmd.label()
                .starts_with(&format!("({})", package_dir.display()))
                && (cmd.program().to_string_lossy().contains("npm")
                    || cmd.program().to_string_lossy().contains("node"))
        );
        let config_path = repo_root.join_components(&["configs", "microfrontends.json"]);
        assert!(cmd.label().ends_with(&format!(
            " run proxy -- {} --names docs-app",
            config_path.as_str()
        )));
        let stdout = command_stdout(cmd).await;

        assert_filtered_environment(&stdout, inherited_env);
    }

    #[tokio::test]
    async fn test_microfrontends_binary_proxy_command_applies_filtered_environment() {
        let (_tempdir, repo_root, package_dir) = create_test_repo();
        let inherited_env = inherited_env_name();
        write_microfrontends_binary(&package_dir, inherited_env);
        let package_graph = package_graph(
            &repo_root,
            &package_dir,
            PackageJson {
                dependencies: Some(BTreeMap::from([(
                    "@vercel/microfrontends".to_owned(),
                    "1.0.0".to_owned(),
                )])),
                ..Default::default()
            },
        )
        .await;
        let cmd = proxy_command(&package_graph, &filtered_environment());
        assert_eq!(
            cmd.program(),
            package_dir
                .join("node_modules")
                .join(".bin")
                .join(if cfg!(windows) {
                    "microfrontends.cmd"
                } else {
                    "microfrontends"
                })
                .as_os_str()
        );
        assert!(
            cmd.label()
                .starts_with(&format!("({})", package_dir.display()))
        );
        let config_path = repo_root.join_components(&["configs", "microfrontends.json"]);
        assert!(
            cmd.label()
                .ends_with(&format!(" proxy {} --names docs-app", config_path.as_str()))
        );
        let stdout = command_stdout(cmd).await;

        assert_filtered_environment(&stdout, inherited_env);
    }

    #[tokio::test]
    async fn test_microfrontend_proxy_requires_compatibility_payload() {
        let (_tempdir, repo_root, package_dir) = create_test_repo();
        let mut graph = package_graph(&repo_root, &package_dir, PackageJson::default()).await;
        graph.remove_package_info_for_test(&PackageName::from("web"));
        let mfe_config = MockMfeConfig("configs/microfrontends.json");
        let tasks = [TaskId::new("docs", "dev"), TaskId::new("web", "proxy")];
        let command_provider = MicroFrontendProxyProvider::new(&graph, tasks.iter(), &mfe_config);

        let error = CommandProvider::<CommandProviderError>::command(
            &command_provider,
            &TaskId::new("web", "proxy"),
            &EnvironmentVariableMap::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            CommandProviderError::MissingPackagePayload { .. }
        ));
    }

    #[tokio::test]
    async fn test_microfrontend_proxy_rejects_invalid_contexts_and_config_paths() {
        let (_tempdir, repo_root, package_dir) = create_test_repo();
        let environment = EnvironmentVariableMap::default();
        let graph = package_graph(&repo_root, &package_dir, PackageJson::default()).await;
        for path in [
            "../config.json",
            "C:/config.json",
            r"configs\microfrontends.json",
        ] {
            let config = MockMfeConfig(path);
            assert!(matches!(
                proxy_command_result(&graph, &environment, &config, "web"),
                Err(CommandProviderError::UnsafeMfeConfigPath { .. })
            ));
        }
    }

    #[tokio::test]
    async fn test_root_javascript_proxy_uses_repository_root() {
        let (_tempdir, repo_root, _package_dir) = create_test_repo();
        let environment = EnvironmentVariableMap::default();
        let config = MockMfeConfig("configs/microfrontends.json");
        let pure_root = PackageGraph::builder_optional(&repo_root, None)
            .build()
            .await
            .unwrap();
        let error = MicroFrontendProxyProvider::<MockMfeConfig>::validate_package_context(
            pure_root.package_task_context(&PackageName::Root).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CommandProviderError::InvalidMfePackageContext { .. }
        ));
        let custom_graph = root_package_graph(
            &repo_root,
            PackageJson {
                name: Some(Spanned::new("root-name".to_owned())),
                scripts: BTreeMap::from([("proxy".to_owned(), Spanned::new("proxy".to_owned()))]),
                ..Default::default()
            },
        )
        .await;
        assert!(
            custom_graph
                .package_task_context(&PackageName::Root)
                .unwrap()
                .package_info()
                .unwrap()
                .package_json
                .scripts
                .contains_key("proxy")
        );
        let custom = proxy_command_result(&custom_graph, &environment, &config, "//")
            .unwrap()
            .unwrap();
        assert!(
            custom
                .label()
                .starts_with(&format!("({}", custom_graph.repo_root().as_str())),
            "unexpected root command: {}",
            custom.label()
        );
        assert!(custom.label().contains(" run proxy "));

        let binary_graph = root_package_graph(
            &repo_root,
            PackageJson {
                dependencies: Some(BTreeMap::from([(
                    "@vercel/microfrontends".to_owned(),
                    "1.0.0".to_owned(),
                )])),
                ..Default::default()
            },
        )
        .await;
        let binary = proxy_command_result(&binary_graph, &environment, &config, "//")
            .unwrap()
            .unwrap();
        assert_eq!(
            binary.program(),
            binary_graph
                .repo_root()
                .join_components(&[
                    "node_modules",
                    ".bin",
                    if cfg!(windows) {
                        "microfrontends.cmd"
                    } else {
                        "microfrontends"
                    },
                ])
                .as_std_path()
                .as_os_str()
        );
    }
}
