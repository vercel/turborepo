mod context;
mod graph;
pub mod pipeline;
mod scope;
mod task_id;

use anyhow::{Context as ErrorContext, Result};
use graph::CompleteGraph;
use tracing::{debug, info};

use crate::{
    commands::CommandBase,
    daemon::DaemonConnector,
    manager::Manager,
    opts::Opts,
    package_json::PackageJson,
    run::{context::Context, task_id::ROOT_PKG_NAME},
};

#[derive(Debug)]
pub struct Run<'a> {
    base: &'a CommandBase,
    opts: Opts<'a>,
    processes: Manager,
}

impl<'a> Run<'a> {
    pub fn new(base: &'a CommandBase, opts: Opts<'a>) -> Self {
        let processes = Manager::new();
        Self {
            base,
            opts,
            processes,
        }
    }

    pub async fn run(&mut self, targets: &[String]) -> Result<()> {
        let _start_at = std::time::Instant::now();
        let package_json_path = self.base.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(package_json_path.as_absolute_path())?;

        let is_structured_output = self.opts.run_opts.graph_dot || self.opts.run_opts.dry_run_json;

        let pkg_dep_graph = if self.opts.run_opts.single_package {
            Context::build_single_package_graph(root_package_json)?
        } else {
            Context::build_multi_package_graph(&self.base.repo_root, &root_package_json)?
        };
        // There's some warning handling code in Go that I'm ignoring

        if self.base.ui.is_ci() && !self.opts.run_opts.no_daemon {
            info!("skipping turbod since we appear to be in a non-interactive context");
        } else if !self.opts.run_opts.no_daemon {
            let connector = DaemonConnector {
                can_start_server: true,
                can_kill_server: true,
                pid_file: self.base.daemon_file_root().join_component("turbod.pid"),
                sock_file: self.base.daemon_file_root().join_component("turbod.sock"),
            };

            let client = connector.connect().await?;
            debug!("running in daemon mode");
            self.opts.runcache_opts.output_watcher = Some(client);
        }

        pkg_dep_graph
            .validate()
            .context("Invalid package dependency graph")?;

        let g = CompleteGraph::new(
            pkg_dep_graph.workspace_graph.clone(),
            pkg_dep_graph.workspace_infos.clone(),
            self.base.repo_root.as_absolute_path(),
        );

        let is_single_package = self.opts.run_opts.single_package;
        let turbo_json = g.get_turbo_config_from_workspace(ROOT_PKG_NAME, is_single_package)?;

        self.opts.cache_opts.remote_cache_opts = turbo_json.remote_cache_opts.clone();

        if self.opts.run_opts.experimental_space_id.is_none() {
            self.opts.run_opts.experimental_space_id = turbo_json.space_id.clone();
        }

        let pipeline = &turbo_json.pipeline;

        let mut filtered_pkgs =
            scope::resolve_packages(&self.opts.scope_opts, &self.base, &pkg_dep_graph)?;

        if filtered_pkgs.len() == pkg_dep_graph.len() {
            for target in targets {
                let key = task_id::root_task_id(target);
                if pipeline.contains_key(&key) {
                    filtered_pkgs.insert(task_id::ROOT_PKG_NAME.to_string());
                    break;
                }
            }
        }

        Ok(())
    }
}
