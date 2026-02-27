use super::Error;
use crate::microfrontends::MicrofrontendsConfigs;

// Re-export CommandFactory from turborepo-task-executor with our Error type
pub type CommandFactory<'a> = turborepo_task_executor::CommandFactory<'a, Error>;

// Re-export PackageGraphCommandProvider from turborepo-task-executor with our
// MicrofrontendsConfigs type
pub type PackageGraphCommandProvider<'a> =
    turborepo_task_executor::PackageGraphCommandProvider<'a, MicrofrontendsConfigs>;

// Re-export MicroFrontendProxyProvider from turborepo-task-executor with our
// MicrofrontendsConfigs type
pub type MicroFrontendProxyProvider<'a, T> =
    turborepo_task_executor::MicroFrontendProxyProvider<'a, T, MicrofrontendsConfigs>;

#[cfg(test)]
mod test {
    use std::ffi::OsStr;

    use insta::assert_snapshot;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPath};
    use turborepo_env::EnvironmentVariableMap;
    use turborepo_microfrontends::{TurborepoMfeConfig as Config, MICROFRONTENDS_PACKAGE};
    use turborepo_process::Command;
    use turborepo_repository::{
        package_graph::{PackageInfo, PackageName},
        package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_task_executor::{CommandProvider, PackageInfoProvider};
    use turborepo_task_id::TaskId;

    use super::*;

    struct EchoCmdFactory;

    impl CommandProvider<Error> for EchoCmdFactory {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: &EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Ok(Some(Command::new("echo")))
        }
    }

    struct ErrProvider;

    impl CommandProvider<Error> for ErrProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: &EnvironmentVariableMap,
        ) -> Result<Option<Command>, Error> {
            Err(Error::InternalErrors("oops!".into()))
        }
    }

    struct NoneProvider;

    impl CommandProvider<Error> for NoneProvider {
        fn command(
            &self,
            _task_id: &TaskId,
            _environment: &EnvironmentVariableMap,
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
            .command(&task_id, &EnvironmentVariableMap::default())
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
            .command(&task_id, &EnvironmentVariableMap::default())
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
            .command(&task_id, &EnvironmentVariableMap::default())
            .unwrap()
            .unwrap();
        assert_eq!(cmd.program(), OsStr::new("echo"));
    }

    #[test]
    fn test_none_returned_if_no_commands_found() {
        let factory = CommandFactory::new();
        let task_id = TaskId::new("foo", "build");
        let cmd = factory
            .command(&task_id, &EnvironmentVariableMap::default())
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
                &EnvironmentVariableMap::default(),
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
