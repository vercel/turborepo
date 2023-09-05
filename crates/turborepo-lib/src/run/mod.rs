#![allow(dead_code)]

mod cache;
mod global_hash;
mod scope;
mod summary;
pub mod task_id;
use std::{
    io::{BufWriter, IsTerminal},
    sync::Arc,
};

use anyhow::{anyhow, Context as ErrorContext, Result};
pub use cache::{RunCache, TaskCache};
use itertools::Itertools;
use tracing::{debug, info};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_cache::{http::APIAuth, AsyncCache};
use turborepo_env::EnvironmentVariableMap;
use turborepo_scm::SCM;
use turborepo_ui::ColorSelector;

use self::task_id::TaskName;
use crate::{
    cli::EnvMode,
    commands::CommandBase,
    config::TurboJson,
    daemon::DaemonConnector,
    engine::EngineBuilder,
    manager::Manager,
    opts::{GraphOpts, Opts},
    package_graph::{PackageGraph, WorkspaceName},
    package_json::PackageJson,
    run::global_hash::get_global_hash_inputs,
    task_graph::Visitor,
    task_hash::PackageInputsHashes,
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

        let _is_structured_output = opts.run_opts.graph.is_some() || opts.run_opts.dry_run_json;

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

        let mut daemon = None;
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
            daemon = Some(client);
        }

        pkg_dep_graph
            .validate()
            .context("Invalid package dependency graph")?;

        let scm = SCM::new(&self.base.repo_root);

        let filtered_pkgs = {
            let mut filtered_pkgs = scope::resolve_packages(
                &opts.scope_opts,
                &self.base.repo_root,
                &pkg_dep_graph,
                &scm,
            )?;

            if filtered_pkgs.len() != pkg_dep_graph.len() {
                for target in self.targets() {
                    let task_name = TaskName::from(target.as_str());
                    if root_turbo_json.pipeline.contains_key(&task_name) {
                        filtered_pkgs.insert(WorkspaceName::Root);
                        break;
                    }
                }
            };

            filtered_pkgs
        };

        let env_at_execution_start = EnvironmentVariableMap::infer();

        let repo_config = self.base.repo_config()?;
        let team_id = repo_config.team_id();
        let team_slug = repo_config.team_slug();

        let token = self.base.user_config()?.token();

        let api_auth = team_id.zip(token).map(|(team_id, token)| APIAuth {
            team_id: team_id.to_string(),
            token: token.to_string(),
            team_slug: team_slug.map(|s| s.to_string()),
        });

        let async_cache = AsyncCache::new(
            &opts.cache_opts,
            &self.base.repo_root,
            self.base.api_client()?,
            api_auth,
        )?;

        info!("created cache");
        let engine = EngineBuilder::new(
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
        .with_workspaces(filtered_pkgs.into_iter().collect())
        .with_tasks(
            opts.run_opts
                .tasks
                .iter()
                .map(|task| TaskName::from(task.as_str()).into_owned()),
        )
        .build()?;

        engine
            .validate(&pkg_dep_graph, opts.run_opts.concurrency)
            .map_err(|errors| {
                anyhow!(
                    "error preparing engine: Invalid persistent task configuration:\n{}",
                    errors
                        .into_iter()
                        .map(|e| e.to_string())
                        .sorted()
                        .join("\n")
                )
            })?;

        if let Some(graph_opts) = opts.run_opts.graph {
            match graph_opts {
                GraphOpts::File(graph_file) => {
                    let graph_file =
                        AbsoluteSystemPathBuf::from_unknown(self.base.cwd(), graph_file);
                    let file = graph_file.open()?;
                    let _writer = BufWriter::new(file);
                    todo!("Need to implement different format support");
                }
                GraphOpts::Stdout => {
                    engine.dot_graph(std::io::stdout(), opts.run_opts.single_package)?
                }
            }
            return Ok(());
        }

        let root_workspace = pkg_dep_graph
            .workspace_info(&WorkspaceName::Root)
            .expect("must have root workspace");

        let global_hash_inputs = get_global_hash_inputs(
            root_workspace,
            &self.base.repo_root,
            pkg_dep_graph.package_manager(),
            pkg_dep_graph.lockfile(),
            &root_turbo_json.global_deps,
            &env_at_execution_start,
            &root_turbo_json.global_env,
            &root_turbo_json.global_pass_through_env,
            opts.run_opts.env_mode,
            opts.run_opts.framework_inference,
            &root_turbo_json.global_dot_env,
        )?;

        let global_hash = global_hash_inputs.calculate_global_hash_from_inputs();

        debug!("global hash: {}", global_hash);

        let color_selector = ColorSelector::default();

        let _runcache = RunCache::new(
            async_cache,
            &self.base.repo_root,
            &opts.runcache_opts,
            color_selector,
            daemon,
            self.base.ui,
        );

        let mut global_env_mode = opts.run_opts.env_mode;
        if matches!(global_env_mode, EnvMode::Infer)
            && !root_turbo_json.global_pass_through_env.is_empty()
        {
            global_env_mode = EnvMode::Strict;
        }

        let workspaces = pkg_dep_graph.workspaces().collect();
        let package_inputs_hashes = PackageInputsHashes::calculate_file_hashes(
            scm,
            engine.tasks(),
            workspaces,
            engine.task_definitions(),
            &self.base.repo_root,
        )?;

        debug!("package inputs hashes: {:?}", package_inputs_hashes);

        let pkg_dep_graph = Arc::new(pkg_dep_graph);
        let engine = Arc::new(engine);
        let visitor = Visitor::new(
            pkg_dep_graph.clone(),
            &opts,
            package_inputs_hashes,
            &env_at_execution_start,
            &global_hash,
            global_env_mode,
        );

        visitor.visit(engine.clone()).await?;

        Ok(())
    }
}
