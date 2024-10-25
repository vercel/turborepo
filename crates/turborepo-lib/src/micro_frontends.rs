use std::collections::{HashMap, HashSet};

use tracing::warn;
use turbopath::AbsoluteSystemPath;
use turborepo_micro_frontend::{Config as MFEConfig, Error, MICRO_FRONTENDS_CONFIG};
use turborepo_repository::package_graph::PackageGraph;

use crate::run::task_id::TaskId;

pub fn find_micro_frontend_configs(
    repo_root: &AbsoluteSystemPath,
    package_graph: &PackageGraph,
) -> Result<HashMap<String, HashSet<TaskId<'static>>>, Error> {
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
            .filter_map(|(application, options)| {
                let Some(dev_task) = options.development.task.as_deref() else {
                    warn!(
                        "{application} does not have a dev task configured. Turborepo will be \
                         unable to detect if traffic should be routed to local server"
                    );
                    return None;
                };
                Some(TaskId::new(application, dev_task).into_owned())
            })
            .collect();
        configs.insert(package_name.to_string(), tasks);
    }

    Ok(configs)
}
