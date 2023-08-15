#![allow(dead_code)]

mod global_hash;
mod scope;
pub mod task_id;

use std::io::IsTerminal;

use anyhow::{Context as ErrorContext, Result};
use tracing::{debug, info};
use turborepo_cache::{http::APIAuth, AsyncCache};
use turborepo_env::EnvironmentVariableMap;
use turborepo_scm::SCM;
use turborepo_ui::UI;

use self::task_id::TaskName;
use crate::{
    commands::CommandBase,
    config::TurboJson,
    daemon::DaemonConnector,
    engine::EngineBuilder,
    manager::Manager,
    opts::Opts,
    package_graph::{PackageGraph, WorkspaceName},
    package_json::PackageJson,
    run::global_hash::get_global_hash_inputs,
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
        self.base.args().try_into()
    }

    pub async fn run(&mut self) -> Result<()> {
        let _start_at = std::time::Instant::now();
        let package_json_path = self.base.repo_root.join_component("package.json");
        let root_package_json =
            PackageJson::load(&package_json_path).context("failed to read package.json")?;
        let mut opts = self.opts()?;

        let _is_structured_output = opts.run_opts.graph_dot || opts.run_opts.dry_run_json;

        let is_single_package = opts.run_opts.single_package;

        let pkg_dep_graph = PackageGraph::builder(&self.base.repo_root, root_package_json.clone())
            .with_single_package_mode(opts.run_opts.single_package)
            .build()?;

        let root_turbo_json =
            TurboJson::load(&self.base.repo_root, &root_package_json, is_single_package)?;

        opts.cache_opts.remote_cache_opts = root_turbo_json.remote_cache_options.clone();

        if opts.run_opts.experimental_space_id.is_none() {
            opts.run_opts.experimental_space_id = root_turbo_json.space_id.clone();
        }

        // There's some warning handling code in Go that I'm ignoring
        let is_ci_and_not_tty = turborepo_ci::is_ci() && !std::io::stdout().is_terminal();
        if is_ci_and_not_tty && !opts.run_opts.no_daemon {
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

        let scm = SCM::new(&self.base.repo_root);

        let filtered_pkgs =
            scope::resolve_packages(&opts.scope_opts, &self.base, &pkg_dep_graph, &scm)?;

        // TODO: Add this back once scope/filter is implemented.
        //       Currently this code has lifetime issues
        // if filtered_pkgs.len() != pkg_dep_graph.len() {
        //     for target in targets {
        //         let key = task_id::root_task_id(target);
        //         if pipeline.contains_key(&key) {
        //             filtered_pkgs.insert(task_id::ROOT_PKG_NAME.to_string());
        //             break;
        //         }
        //     }
        // }
        let env_at_execution_start = EnvironmentVariableMap::infer();

        let _global_hash_inputs = get_global_hash_inputs(
            &self.base.ui,
            &self.base.repo_root,
            pkg_dep_graph.root_package_json(),
            pkg_dep_graph.package_manager(),
            pkg_dep_graph.lockfile(),
            // TODO: Fill in these vec![] once turbo.json is ported
            vec![],
            &env_at_execution_start,
            vec![],
            vec![],
            opts.run_opts.env_mode,
            opts.run_opts.framework_inference,
            vec![],
        )?;

        let team_id = self.base.repo_config()?.team_id();

        let token = self.base.user_config()?.token();

        let api_auth = team_id.zip(token).map(|(team_id, token)| APIAuth {
            team_id: team_id.to_string(),
            token: token.to_string(),
        });

        let _turbo_cache = AsyncCache::new(
            &opts.cache_opts,
            &self.base.repo_root,
            self.base.api_client()?,
            api_auth,
        )?;

        info!("created cache");
        let _engine = EngineBuilder::new(
            &self.base.repo_root,
            &pkg_dep_graph,
            opts.run_opts.single_package,
        )
        .with_root_tasks(root_turbo_json.pipeline.keys().cloned())
        .with_turbo_jsons(Some(
            Some((WorkspaceName::Root, root_turbo_json.clone()))
                .into_iter()
                .collect(),
        ))
        .with_tasks_only(opts.run_opts.only)
        .with_workspaces(
            filtered_pkgs
                .iter()
                .map(|workspace| WorkspaceName::from(workspace.as_str()))
                .collect(),
        )
        .with_tasks(
            opts.run_opts
                .tasks
                .iter()
                .map(|task| TaskName::from(task.as_str()).into_owned()),
        )
        .build()?;

        Ok(())
    }
}

#[cfg(test)]
mod test {

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_ui::UI;

    use crate::{
        cli::{Command, RunArgs},
        commands::CommandBase,
        get_version,
        run::Run,
        Args,
    };

    #[tokio::test]
    async fn test_run() -> Result<()> {
        let dir = tempdir()?;
        let repo_root = AbsoluteSystemPathBuf::try_from(dir.path())?;
        let mut args = Args::default();
        // Daemon does not work with run stub yet
        let run_args = RunArgs {
            no_daemon: true,
            pkg_inference_root: Some(["apps", "my-app"].join(std::path::MAIN_SEPARATOR_STR)),
            ..Default::default()
        };
        args.command = Some(Command::Run(Box::new(run_args)));

        let ui = UI::infer();

        // Add package.json
        repo_root
            .join_component("package.json")
            .create_with_contents("{\"workspaces\": [\"apps/*\"]}")?;
        repo_root
            .join_component("package-lock.json")
            .create_with_contents("")?;
        repo_root
            .join_component("turbo.json")
            .create_with_contents("{}")?;

        let base = CommandBase::new(args, repo_root, get_version(), ui)?;
        let mut run = Run::new(base);
        run.run().await
    }
}
