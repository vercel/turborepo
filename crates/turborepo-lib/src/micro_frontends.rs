use std::collections::{HashMap, HashSet};

use itertools::Itertools;
use tracing::warn;
use turbopath::AbsoluteSystemPath;
use turborepo_micro_frontend::{Config as MFEConfig, Error, DEFAULT_MICRO_FRONTENDS_CONFIG};
use turborepo_repository::package_graph::{PackageGraph, PackageName};

use crate::run::task_id::TaskId;

#[derive(Debug, Clone)]
pub struct MicroFrontendsConfigs {
    configs: HashMap<String, HashSet<TaskId<'static>>>,
}

impl MicroFrontendsConfigs {
    pub fn new(
        repo_root: &AbsoluteSystemPath,
        package_graph: &PackageGraph,
    ) -> Result<Option<Self>, Error> {
        let mut configs = HashMap::new();
        for (package_name, package_info) in package_graph.packages() {
            let config_path = repo_root
                .resolve(package_info.package_path())
                .join_component(DEFAULT_MICRO_FRONTENDS_CONFIG);
            let Some(config) = MFEConfig::load(&config_path).or_else(|err| {
                if matches!(err, turborepo_micro_frontend::Error::UnsupportedVersion(_)) {
                    warn!("Ignoring {config_path}: {err}");
                    Ok(None)
                } else {
                    Err(err)
                }
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

        Ok((!configs.is_empty()).then_some(Self { configs }))
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

    /// Returns the proxy task for a given package in a MFE
    pub fn package_mfe_proxy(&self, package_name: &PackageName) -> Option<DevTaskAndProxy> {
        self.configs
            .iter()
            // Find the MFE this package belongs to
            .find_map(|(pkg_with_proxy, dev_tasks)| {
                dev_tasks.iter().find_map(|dev_task| {
                    (dev_task.package() == package_name.as_str())
                        .then_some((pkg_with_proxy, dev_task))
                })
            })
            // Return package's dev task with it's proxy task
            .map(|(pkg_with_proxy, dev_task)| DevTaskAndProxy {
                dev: dev_task.as_borrowed(),
                proxy: TaskId::new(pkg_with_proxy, "proxy"),
            })
    }
}

pub struct DevTaskAndProxy<'a> {
    pub dev: TaskId<'a>,
    pub proxy: TaskId<'a>,
}
