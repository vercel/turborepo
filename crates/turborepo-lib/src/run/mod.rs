#![allow(dead_code)]

mod graph;
mod package_graph;
pub mod pipeline;
mod scope;
mod task_id;
mod workspace_catalog;

use anyhow::{Context as ErrorContext, Result};
use graph::CompleteGraph;
use tracing::{debug, info};

use crate::{
    commands::CommandBase,
    daemon::DaemonConnector,
    manager::Manager,
    opts::Opts,
    package_json::PackageJson,
    run::{package_graph::PackageGraph, task_id::ROOT_PKG_NAME},
};

#[derive(Debug)]
pub struct Run {
    base: CommandBase,
    processes: Manager,
}

impl Run {
    pub fn new(base: CommandBase) -> Self {
        let processes = Manager::new();
        Self { base, processes }
    }

    fn targets(&self) -> &[String] {
        self.base.args().get_tasks()
    }

    fn opts(&self) -> Result<Opts> {
        Ok(self.base.args().try_into()?)
    }

    pub async fn run(&mut self) -> Result<()> {
        let _start_at = std::time::Instant::now();
        let package_json_path = self.base.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(package_json_path.as_absolute_path())
            .context("failed to read package.json")?;
        let targets = self.targets();
        let mut opts = self.opts()?;

        let _is_structured_output = opts.run_opts.graph_dot || opts.run_opts.dry_run_json;

        let pkg_dep_graph = if opts.run_opts.single_package {
            PackageGraph::build_single_package_graph(root_package_json)?
        } else {
            PackageGraph::build_multi_package_graph(&self.base.repo_root, &root_package_json)?
        };
        // There's some warning handling code in Go that I'm ignoring

        if self.base.ui.is_ci() && !opts.run_opts.no_daemon {
            info!("skipping turbod since we appear to be in a non-interactive context");
        } else if !opts.run_opts.no_daemon {
            let connector = DaemonConnector {
                can_start_server: true,
                can_kill_server: true,
                pid_file: self.base.daemon_file_root().join_component("turbod.pid"),
                sock_file: self.base.daemon_file_root().join_component("turbod.sock"),
            };

            let client = connector.connect().await?;
            debug!("running in daemon mode");
            opts.runcache_opts.output_watcher = Some(client);
        }

        pkg_dep_graph
            .validate()
            .context("Invalid package dependency graph")?;

        let g = CompleteGraph::new(
            pkg_dep_graph.workspace_graph.clone(),
            pkg_dep_graph.workspace_infos.clone(),
            self.base.repo_root.as_absolute_path(),
        );

        let is_single_package = opts.run_opts.single_package;
        let turbo_json = g.get_turbo_config_from_workspace(ROOT_PKG_NAME, is_single_package)?;

        opts.cache_opts.remote_cache_opts = turbo_json.remote_cache_opts.clone();

        if opts.run_opts.experimental_space_id.is_none() {
            opts.run_opts.experimental_space_id = turbo_json.space_id.clone();
        }

        let pipeline = &turbo_json.pipeline;

        let mut filtered_pkgs =
            scope::resolve_packages(&opts.scope_opts, &self.base, &pkg_dep_graph)?;

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

#[cfg(test)]
mod test {
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::{
        cli::{Command, RunArgs},
        commands::CommandBase,
        get_version,
        run::Run,
        ui::UI,
        Args,
    };

    #[tokio::test]
    async fn test_run() -> Result<()> {
        let dir = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::new(dir.path())?;
        let mut args = Args::default();
        let mut run_args = RunArgs::default();
        // Daemon does not work with run stub yet
        run_args.no_daemon = true;
        args.command = Some(Command::Run(Box::new(run_args)));

        let ui = UI::infer();

        // Add package.json
        fs::write(repo_root.join_component("package.json"), "{}")?;

        let base = CommandBase::new(args, repo_root, get_version(), ui)?;
        let mut run = Run::new(base);
        run.run().await
    }
}
