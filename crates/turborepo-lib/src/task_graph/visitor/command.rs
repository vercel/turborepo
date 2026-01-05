use std::collections::{HashMap, HashSet};

use turbopath::AbsoluteSystemPath;
use turborepo_env::EnvironmentVariableMap;
use turborepo_microfrontends::MICROFRONTENDS_PACKAGE;
use turborepo_process::Command;
use turborepo_repository::package_graph::{PackageInfo, PackageName};
use turborepo_task_executor::CommandProvider;
use turborepo_task_id::TaskId;

use super::Error;
use crate::microfrontends::MicrofrontendsConfigs;

// Re-export CommandFactory from turborepo-task-executor with our Error type
pub type CommandFactory<'a> = turborepo_task_executor::CommandFactory<'a, Error>;

// Re-export PackageGraphCommandProvider from turborepo-task-executor with our
// MicrofrontendsConfigs type
pub type PackageGraphCommandProvider<'a> =
    turborepo_task_executor::PackageGraphCommandProvider<'a, MicrofrontendsConfigs>;

// Re-export PackageInfoProvider trait from turborepo-task-executor
pub use turborepo_task_executor::PackageInfoProvider;

#[derive(Debug)]
pub struct MicroFrontendProxyProvider<'a, T> {
    repo_root: &'a AbsoluteSystemPath,
    package_graph: &'a T,
    tasks_in_graph: HashSet<TaskId<'a>>,
    mfe_configs: &'a MicrofrontendsConfigs,
}

impl<'a, T: PackageInfoProvider> MicroFrontendProxyProvider<'a, T> {
    pub fn new<'b>(
        repo_root: &'a AbsoluteSystemPath,
        package_graph: &'a T,
        tasks_in_graph: impl Iterator<Item = &'b TaskId<'static>>,
        micro_frontends_configs: &'a MicrofrontendsConfigs,
    ) -> Self {
        Self {
            repo_root,
            package_graph,
            tasks_in_graph: tasks_in_graph.cloned().collect(),
            mfe_configs: micro_frontends_configs,
        }
    }

    fn dev_tasks(&self, task_id: &TaskId) -> Option<&HashMap<TaskId<'static>, String>> {
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

impl<'a, T: PackageInfoProvider> CommandProvider<Error> for MicroFrontendProxyProvider<'a, T> {
    fn command(
        &self,
        task_id: &TaskId,
        _environment: EnvironmentVariableMap,
    ) -> Result<Option<Command>, Error> {
        tracing::debug!(
            "MicroFrontendProxyProvider::command - called for task: {}",
            task_id
        );

        let Some(dev_tasks) = self.dev_tasks(task_id) else {
            tracing::debug!(
                "MicroFrontendProxyProvider::command - no dev tasks found for {}",
                task_id
            );
            return Ok(None);
        };

        tracing::debug!(
            "MicroFrontendProxyProvider::command - found {} dev tasks for {}",
            dev_tasks.len(),
            task_id
        );

        let has_custom_proxy = self.has_custom_proxy(task_id)?;
        let package_info = self.package_info(task_id)?;
        let has_mfe_dependency = package_info
            .package_json
            .all_dependencies()
            .any(|(package, _version)| package.as_str() == MICROFRONTENDS_PACKAGE);

        tracing::debug!(
            "MicroFrontendProxyProvider::command - has_custom_proxy: {}, has_mfe_dependency: {}",
            has_custom_proxy,
            has_mfe_dependency
        );

        let local_apps = dev_tasks.iter().filter_map(|(task, app_name)| {
            self.tasks_in_graph
                .contains(task)
                .then_some(app_name.as_str())
        });
        let package_dir = self.repo_root.resolve(package_info.package_path());
        let mfe_config_filename = self
            .mfe_configs
            .config_filename(task_id.package())
            .expect("every microfrontends default application should have configuration path");
        let mfe_path = self.repo_root.join_unix_path(mfe_config_filename);

        let cmd = if has_custom_proxy {
            tracing::debug!("MicroFrontendProxyProvider::command - using custom proxy script");
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
            Some(cmd)
        } else if has_mfe_dependency {
            tracing::debug!(
                "MicroFrontendProxyProvider::command - using @vercel/microfrontends proxy"
            );
            let mut args = vec!["proxy", mfe_path.as_str(), "--names"];
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
            tracing::debug!("MicroFrontendProxyProvider::command - using Turborepo built-in proxy");
            // No custom proxy and no @vercel/microfrontends dependency.
            // The Turborepo proxy will be started separately.
            None
        };

        tracing::debug!(
            "MicroFrontendProxyProvider::command - returning command: {}",
            if cmd.is_some() { "Some" } else { "None" }
        );

        Ok(cmd)
    }
}

#[cfg(test)]
mod test {
    use std::ffi::OsStr;

    use insta::assert_snapshot;
    use turbopath::AnchoredSystemPath;
    use turborepo_microfrontends::TurborepoMfeConfig as Config;
    use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

    use super::*;

    struct EchoCmdFactory;

    impl CommandProvider<Error> for EchoCmdFactory {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Ok(Some(Command::new("echo")))
        }
    }

    struct ErrProvider;

    impl CommandProvider<Error> for ErrProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Err(Error::InternalErrors("oops!".into()))
        }
    }

    struct NoneProvider;

    impl CommandProvider<Error> for NoneProvider {
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
        assert_snapshot!(cmd.to_string(), @"Internal errors encountered: oops!");
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

    #[test]
    fn test_mfe_application_passed() {
        let repo_root = AbsoluteSystemPath::new(if cfg!(windows) {
            "C:\\repo-root"
        } else {
            "/tmp/repo-root"
        })
        .unwrap();
        struct MockPackageInfo(PackageInfo);
        impl PackageInfoProvider for MockPackageInfo {
            fn package_manager(&self) -> &PackageManager {
                &PackageManager::Npm
            }

            fn package_info(&self, name: &PackageName) -> Option<&PackageInfo> {
                match name {
                    PackageName::Root => unimplemented!(),
                    PackageName::Other(name) => match name.as_str() {
                        "web" | "docs" => Some(&self.0),
                        _ => None,
                    },
                }
            }
        }
        let mut config = Config::from_str(
            r#"
        {
            "applications": {
                "web-app": {
                    "packageName": "web"
                },
                "docs-app": {
                    "packageName": "docs",
                    "routing": [{"paths": ["/docs"]}]
                }
            }
        }"#,
            "microfrontends.json",
        )
        .unwrap();
        // Set the path to simulate loading from a directory
        config.set_path(AnchoredSystemPath::new("web").unwrap());
        let microfrontends_configs = MicrofrontendsConfigs::from_configs(
            ["web", "docs"].iter().copied().collect(),
            std::iter::once(("web", Ok(Some(config)))),
            std::collections::HashMap::new(),
        )
        .unwrap()
        .unwrap();

        let mock_package_info = MockPackageInfo(PackageInfo {
            package_json: PackageJson {
                dependencies: Some(
                    vec![(MICROFRONTENDS_PACKAGE.to_owned(), "1.0.0".to_owned())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
            package_json_path: AnchoredSystemPath::new("package.json").unwrap().to_owned(),
            unresolved_external_dependencies: None,
            transitive_dependencies: None,
        });
        let mut factory = CommandFactory::new();
        factory.add_provider(MicroFrontendProxyProvider::new(
            repo_root,
            &mock_package_info,
            [TaskId::new("docs", "dev"), TaskId::new("web", "proxy")].iter(),
            &microfrontends_configs,
        ));
        let cmd = factory
            .command(
                &TaskId::new("web", "proxy"),
                EnvironmentVariableMap::default(),
            )
            .unwrap()
            .unwrap();
        assert!(
            cmd.label().ends_with("--names docs-app"),
            "Expected command to use application name instead of package name: {}",
            cmd.label(),
        );
    }
}
