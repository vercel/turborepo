use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use tracing::warn;
use turbopath::AbsoluteSystemPath;
use turborepo_micro_frontend::{Config as MFEConfig, Error, MICRO_FRONTENDS_PACKAGES};
use turborepo_repository::package_graph::{PackageGraph, PackageName};

use crate::{
    config,
    run::task_id::{TaskId, TaskName},
    turbo_json::TurboJson,
};

#[derive(Debug, Clone)]
pub struct MicroFrontendsConfigs {
    configs: HashMap<String, HashSet<TaskId<'static>>>,
    mfe_package: Option<&'static str>,
}

impl MicroFrontendsConfigs {
    pub fn new(
        repo_root: &AbsoluteSystemPath,
        package_graph: &PackageGraph,
    ) -> Result<Option<Self>, Error> {
        let mut configs = HashMap::new();
        let mut referenced_default_apps = HashSet::new();
        for (package_name, package_info) in package_graph.packages() {
            let package_dir = repo_root.resolve(package_info.package_path());
            let Some(config) = MFEConfig::load_from_dir(&package_dir).or_else(|err| match err {
                turborepo_micro_frontend::Error::UnsupportedVersion(_) => {
                    warn!("Ignoring {package_dir}: {err}");
                    Ok(None)
                }
                turborepo_micro_frontend::Error::ChildConfig { reference } => {
                    referenced_default_apps.insert(reference);
                    Ok(None)
                }
                err => Err(err),
            })?
            else {
                continue;
            };
            let tasks = config
                .applications
                .iter()
                .map(|(application, options)| {
                    let dev_task = options.development.task.as_deref().unwrap_or("dev");
                    TaskId::new(application, dev_task).into_owned()
                })
                .collect();
            configs.insert(package_name.to_string(), tasks);
        }
        let default_apps_found = configs.keys().cloned().collect();
        let mut missing_default_apps = referenced_default_apps
            .difference(&default_apps_found)
            .map(|s| s.as_str())
            .collect::<Vec<_>>();
        if !missing_default_apps.is_empty() {
            missing_default_apps.sort();
            warn!(
                "Missing default applications: {}",
                missing_default_apps.join(", ")
            );
        }
        let mfe_package = package_graph
            .packages()
            .map(|(pkg, _)| pkg.as_str())
            .sorted()
            // We use `find_map` here instead of a simple `find` so we get the &'static str
            // instead of the &str tied to the lifetime of the package graph.
            .find_map(|pkg| {
                MICRO_FRONTENDS_PACKAGES
                    .iter()
                    .find(|static_pkg| pkg == **static_pkg)
            })
            .copied();

        Ok((!configs.is_empty()).then_some(Self {
            configs,
            mfe_package,
        }))
    }

    pub fn contains_package(&self, package_name: &str) -> bool {
        self.configs.contains_key(package_name)
    }

    pub fn configs(&self) -> impl Iterator<Item = (&String, &HashSet<TaskId<'static>>)> {
        self.configs.iter()
    }

    pub fn get(&self, package_name: &str) -> Option<&HashSet<TaskId<'static>>> {
        self.configs.get(package_name)
    }

    pub fn task_has_mfe_proxy(&self, task_id: &TaskId) -> bool {
        self.configs
            .values()
            .any(|dev_tasks| dev_tasks.contains(task_id))
    }

    pub fn update_turbo_json(
        &self,
        package_name: &PackageName,
        turbo_json: Result<TurboJson, config::Error>,
    ) -> Result<TurboJson, config::Error> {
        // If package either
        // - contains the proxy task
        // - a member of one of the microfrontends
        // then we need to modify its task definitions
        if let Some(FindResult { dev, proxy }) = self.package_turbo_json_update(package_name) {
            // We need to modify turbo.json, use default one if there isn't one present
            let mut turbo_json = turbo_json.or_else(|err| match err {
                config::Error::NoTurboJSON => Ok(TurboJson::default()),
                err => Err(err),
            })?;

            // If the current package contains the proxy task, then add that definition
            if proxy.package() == package_name.as_str() {
                turbo_json.with_proxy(self.mfe_package);
            }

            if let Some(dev) = dev {
                // If this package has a dev task that's part of the MFE, then we make sure the
                // proxy gets included in the task graph.
                turbo_json.with_sibling(
                    TaskName::from(dev.task()).into_owned(),
                    &proxy.as_task_name(),
                );
            }

            Ok(turbo_json)
        } else {
            turbo_json
        }
    }

    fn package_turbo_json_update<'a>(
        &'a self,
        package_name: &PackageName,
    ) -> Option<FindResult<'a>> {
        self.configs().find_map(|(config, tasks)| {
            let dev_task = tasks.iter().find_map(|task| {
                (task.package() == package_name.as_str()).then(|| FindResult {
                    dev: Some(task.as_borrowed()),
                    proxy: TaskId::new(config, "proxy"),
                })
            });
            let proxy_owner = (config.as_str() == package_name.as_str()).then(|| FindResult {
                dev: None,
                proxy: TaskId::new(config, "proxy"),
            });
            dev_task.or(proxy_owner)
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FindResult<'a> {
    dev: Option<TaskId<'a>>,
    proxy: TaskId<'a>,
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    macro_rules! mfe_configs {
        {$($config_owner:expr => $dev_tasks:expr),+} => {
            {
                let mut _map = std::collections::HashMap::new();
                $(
                    let mut _dev_tasks = std::collections::HashSet::new();
                    for _dev_task in $dev_tasks.as_slice() {
                        _dev_tasks.insert(crate::run::task_id::TaskName::from(*_dev_task).task_id().unwrap().into_owned());
                    }
                    _map.insert($config_owner.to_string(), _dev_tasks);
                )+
                _map
            }
        };
    }

    struct PackageUpdateTest {
        package_name: &'static str,
        result: Option<TestFindResult>,
    }

    struct TestFindResult {
        dev: Option<&'static str>,
        proxy: &'static str,
    }

    impl PackageUpdateTest {
        pub const fn new(package_name: &'static str) -> Self {
            Self {
                package_name,
                result: None,
            }
        }

        pub const fn dev(mut self, dev: &'static str, proxy: &'static str) -> Self {
            self.result = Some(TestFindResult {
                dev: Some(dev),
                proxy,
            });
            self
        }

        pub const fn proxy_only(mut self, proxy: &'static str) -> Self {
            self.result = Some(TestFindResult { dev: None, proxy });
            self
        }

        pub fn package_name(&self) -> PackageName {
            PackageName::from(self.package_name)
        }

        pub fn expected(&self) -> Option<FindResult> {
            match self.result {
                Some(TestFindResult {
                    dev: Some(dev),
                    proxy,
                }) => Some(FindResult {
                    dev: Some(Self::str_to_task(dev)),
                    proxy: Self::str_to_task(proxy),
                }),
                Some(TestFindResult { dev: None, proxy }) => Some(FindResult {
                    dev: None,
                    proxy: Self::str_to_task(proxy),
                }),
                None => None,
            }
        }

        fn str_to_task(s: &str) -> TaskId<'static> {
            crate::run::task_id::TaskName::from(s)
                .task_id()
                .unwrap()
                .into_owned()
        }
    }

    const NON_MFE_PKG: PackageUpdateTest = PackageUpdateTest::new("other-pkg");
    const MFE_CONFIG_PKG: PackageUpdateTest =
        PackageUpdateTest::new("mfe-config-pkg").proxy_only("mfe-config-pkg#proxy");
    const MFE_CONFIG_PKG_DEV_TASK: PackageUpdateTest =
        PackageUpdateTest::new("web").dev("web#dev", "mfe-config-pkg#proxy");
    const DEFAULT_APP_PROXY: PackageUpdateTest =
        PackageUpdateTest::new("mfe-docs").dev("mfe-docs#serve", "mfe-web#proxy");
    const DEFAULT_APP_PROXY_AND_DEV: PackageUpdateTest =
        PackageUpdateTest::new("mfe-web").dev("mfe-web#dev", "mfe-web#proxy");

    #[test_case(NON_MFE_PKG)]
    #[test_case(MFE_CONFIG_PKG)]
    #[test_case(MFE_CONFIG_PKG_DEV_TASK)]
    #[test_case(DEFAULT_APP_PROXY)]
    #[test_case(DEFAULT_APP_PROXY_AND_DEV)]
    fn test_package_turbo_json_update(test: PackageUpdateTest) {
        let configs = mfe_configs!(
            "mfe-config-pkg" => ["web#dev", "docs#dev"],
            "mfe-web" => ["mfe-web#dev", "mfe-docs#serve"]
        );
        let mfe = MicroFrontendsConfigs {
            configs,
            mfe_package: None,
        };
        assert_eq!(
            mfe.package_turbo_json_update(&test.package_name()),
            test.expected()
        );
    }
}
