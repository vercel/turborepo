use std::collections::{HashMap, HashSet};

use turbopath::AbsoluteSystemPath;
use turborepo_micro_frontend::{Config as MFEConfig, Error, MICRO_FRONTENDS_CONFIG};
use turborepo_repository::package_graph::PackageGraph;

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
                .join_component(MICRO_FRONTENDS_CONFIG);
            let Some(config) = MFEConfig::load(&config_path)? else {
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
}
