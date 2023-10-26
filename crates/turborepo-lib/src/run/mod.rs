#![allow(dead_code)]

mod cache;
pub(crate) mod global_hash;
mod scope;
pub(crate) mod summary;
pub mod task_id;
use std::{
    collections::HashSet,
    io::{BufWriter, IsTerminal, Write},
    sync::Arc,
    time::SystemTime,
};

use anyhow::{anyhow, Context as ErrorContext, Result};
pub use cache::{RunCache, TaskCache};
use chrono::Local;
use itertools::Itertools;
use rayon::iter::ParallelBridge;
use tracing::{debug, info};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_cache::{AsyncCache, RemoteCacheOpts};
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_json::PackageJson;
use turborepo_scm::SCM;
use turborepo_ui::{cprint, cprintln, ColorSelector, BOLD_GREY, GREY};

use self::task_id::TaskName;
use crate::{
    cli::EnvMode,
    commands::CommandBase,
    config::TurboJson,
    daemon::DaemonConnector,
    engine::EngineBuilder,
    opts::{GraphOpts, Opts},
    package_graph::{PackageGraph, WorkspaceName},
    process::ProcessManager,
    run::{global_hash::get_global_hash_inputs, summary::RunTracker},
    shim::TurboState,
    task_graph::Visitor,
    task_hash::{PackageInputsHashes, TaskHashTrackerState},
};

#[derive(Debug)]
pub struct Run<'a> {
    base: &'a CommandBase,
    processes: ProcessManager,
}

impl<'a> Run<'a> {
    pub fn new(base: &'a CommandBase) -> Self {
        let processes = ProcessManager::new();
        Self { base, processes }
    }

    fn targets(&self) -> &[String] {
        self.base.args().get_tasks()
    }

    fn opts(&self) -> Result<Opts> {
        self.base.args().try_into()
    }

    fn print_run_prelude(&self, opts: &Opts<'_>, filtered_pkgs: &HashSet<WorkspaceName>) {
        let targets_list = opts.run_opts.tasks.join(", ");
        if opts.run_opts.single_package {
            cprint!(self.base.ui, GREY, "{}", "• Running");
            cprint!(self.base.ui, BOLD_GREY, " {}\n", targets_list);
        } else {
            let mut packages = filtered_pkgs
                .iter()
                .map(|workspace_name| workspace_name.to_string())
                .collect::<Vec<String>>();
            packages.sort();
            cprintln!(
                self.base.ui,
                GREY,
                "• Packages in scope: {}",
                packages.join(", ")
            );
            cprint!(self.base.ui, GREY, "{} ", "• Running");
            cprint!(self.base.ui, BOLD_GREY, "{}", targets_list);
            cprint!(self.base.ui, GREY, " in {} packages\n", filtered_pkgs.len());
        }

        let use_http_cache = !opts.cache_opts.skip_remote;
        if use_http_cache {
            cprintln!(self.base.ui, GREY, "• Remote caching enabled");
        } else {
            cprintln!(self.base.ui, GREY, "• Remote caching disabled");
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn run(&mut self) -> Result<i32> {
        tracing::trace!(
            platform = %TurboState::platform_name(),
            start_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("system time after epoch").as_micros(),
            turbo_version = %TurboState::version(),
            "performing run on {:?}",
            TurboState::platform_name(),
        );
        let start_at = Local::now();
        let package_json_path = self.base.repo_root.join_component("package.json");
        let root_package_json =
            PackageJson::load(&package_json_path).context("failed to read package.json")?;
        let mut opts = self.opts()?;

        let api_auth = self.base.api_auth()?;
        let api_client = self.base.api_client()?;
        let config = self.base.config()?;

        // Pulled from initAnalyticsClient in run.go
        let is_linked = api_auth.is_some();
        if !is_linked {
            opts.cache_opts.skip_remote = true;
        } else if let Some(enabled) = config.enabled {
            // We're linked, but if the user has explicitly enabled or disabled, use that
            // value
            opts.cache_opts.skip_remote = !enabled;
        }

        let _is_structured_output = opts.run_opts.graph.is_some() || opts.run_opts.dry_run_json;

        let is_single_package = opts.run_opts.single_package;

        let pkg_dep_graph = PackageGraph::builder(&self.base.repo_root, root_package_json.clone())
            .with_single_package_mode(opts.run_opts.single_package)
            .build()?;

        let root_turbo_json =
            TurboJson::load(&self.base.repo_root, &root_package_json, is_single_package)?;

        let team_id = root_turbo_json
            .remote_cache
            .as_ref()
            .and_then(|configuration_options| configuration_options.team_id.clone())
            .unwrap_or_default();

        let signature = root_turbo_json
            .remote_cache
            .as_ref()
            .and_then(|configuration_options| configuration_options.signature)
            .unwrap_or_default();

        opts.cache_opts.remote_cache_opts = Some(RemoteCacheOpts::new(team_id, signature));

        if opts.run_opts.experimental_space_id.is_none() {
            opts.run_opts.experimental_space_id = root_turbo_json.space_id.clone();
        }

        // There's some warning handling code in Go that I'm ignoring
        let is_ci_or_not_tty = turborepo_ci::is_ci() || !std::io::stdout().is_terminal();

        let mut daemon = None;
        if is_ci_or_not_tty && !opts.run_opts.no_daemon {
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
                    let mut task_name = TaskName::from(target.as_str());
                    // If it's not a package task, we convert to a root task
                    if !task_name.is_package_task() {
                        task_name = task_name.into_root_task()
                    }

                    if root_turbo_json.pipeline.contains_key(&task_name) {
                        filtered_pkgs.insert(WorkspaceName::Root);
                        break;
                    }
                }
            };

            filtered_pkgs
        };

        let env_at_execution_start = EnvironmentVariableMap::infer();

        let async_cache = AsyncCache::new(
            &opts.cache_opts,
            &self.base.repo_root,
            api_client.clone(),
            api_auth.clone(),
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
        .with_workspaces(filtered_pkgs.iter().cloned().collect())
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
            return Ok(0);
        }

        if !opts.run_opts.dry_run {
            self.print_run_prelude(&opts, &filtered_pkgs);
        }

        let root_workspace = pkg_dep_graph
            .workspace_info(&WorkspaceName::Root)
            .expect("must have root workspace");

        let is_monorepo = !opts.run_opts.single_package;

        let root_external_dependencies_hash =
            is_monorepo.then(|| root_workspace.get_external_deps_hash());

        let mut global_hash_inputs = get_global_hash_inputs(
            root_external_dependencies_hash.as_deref(),
            &self.base.repo_root,
            pkg_dep_graph.package_manager(),
            pkg_dep_graph.lockfile(),
            &root_turbo_json.global_deps,
            &env_at_execution_start,
            &root_turbo_json.global_env,
            root_turbo_json.global_pass_through_env.as_deref(),
            opts.run_opts.env_mode,
            opts.run_opts.framework_inference,
            root_turbo_json.global_dot_env.as_deref(),
        )?;

        let global_hash = global_hash_inputs.calculate_global_hash_from_inputs();

        debug!("global hash: {}", global_hash);

        let color_selector = ColorSelector::default();

        let runcache = Arc::new(RunCache::new(
            async_cache,
            &self.base.repo_root,
            &opts.runcache_opts,
            color_selector,
            daemon,
            self.base.ui,
        ));

        info!("created cache");

        let mut global_env_mode = opts.run_opts.env_mode;
        if matches!(global_env_mode, EnvMode::Infer)
            && root_turbo_json.global_pass_through_env.is_some()
        {
            global_env_mode = EnvMode::Strict;
        }

        let workspaces = pkg_dep_graph.workspaces().collect();
        let package_inputs_hashes = PackageInputsHashes::calculate_file_hashes(
            &scm,
            engine.tasks().par_bridge(),
            workspaces,
            engine.task_definitions(),
            &self.base.repo_root,
        )?;

        // remove dead code warnings
        let _proc_manager = ProcessManager::new();

        let pkg_dep_graph = Arc::new(pkg_dep_graph);
        let engine = Arc::new(engine);

        let global_env = {
            let mut env = env_at_execution_start
                .from_wildcards(global_hash_inputs.pass_through_env.unwrap_or_default())?;
            if let Some(resolved_global) = &global_hash_inputs.resolved_env_vars {
                env.union(&resolved_global.all);
            }
            env
        };

        let run_tracker = RunTracker::new(
            start_at,
            "todo",
            opts.scope_opts.pkg_inference_root.as_deref(),
            &env_at_execution_start,
            &self.base.repo_root,
            self.base.version(),
            opts.run_opts.experimental_space_id.clone(),
            api_client,
            api_auth,
            Vendor::get_user(),
        );

        let mut visitor = Visitor::new(
            pkg_dep_graph.clone(),
            runcache,
            run_tracker,
            &opts,
            package_inputs_hashes,
            &env_at_execution_start,
            &global_hash,
            global_env_mode,
            self.base.ui,
            false,
            self.processes.clone(),
            &self.base.repo_root,
            global_env,
        );

        if opts.run_opts.dry_run {
            visitor.dry_run();
        }

        // we look for this log line to mark the start of the run
        // in benchmarks, so please don't remove it
        debug!("running visitor");

        let errors = visitor.visit(engine.clone()).await?;

        let exit_code = errors
            .iter()
            .filter_map(|err| err.exit_code())
            .max()
            // We hit some error, it shouldn't be exit code 0
            .unwrap_or(if errors.is_empty() { 0 } else { 1 });

        for err in &errors {
            writeln!(std::io::stderr(), "{err}").ok();
        }

        visitor
            .finish(exit_code, filtered_pkgs, global_hash_inputs)
            .await?;

        Ok(exit_code)
    }

    #[tokio::main]
    #[tracing::instrument(skip(self))]
    pub async fn get_hashes(&self) -> Result<(String, TaskHashTrackerState)> {
        let started_at = Local::now();
        let env_at_execution_start = EnvironmentVariableMap::infer();

        let package_json_path = self.base.repo_root.join_component("package.json");
        let root_package_json =
            PackageJson::load(&package_json_path).context("failed to read package.json")?;

        let opts = self.opts()?;

        let is_single_package = opts.run_opts.single_package;

        let pkg_dep_graph = PackageGraph::builder(&self.base.repo_root, root_package_json.clone())
            .with_single_package_mode(opts.run_opts.single_package)
            .build()?;

        let root_turbo_json =
            TurboJson::load(&self.base.repo_root, &root_package_json, is_single_package)?;

        let root_workspace = pkg_dep_graph
            .workspace_info(&WorkspaceName::Root)
            .expect("must have root workspace");

        let is_monorepo = !opts.run_opts.single_package;
        let root_external_dependencies_hash =
            is_monorepo.then(|| root_workspace.get_external_deps_hash());

        let mut global_hash_inputs = get_global_hash_inputs(
            root_external_dependencies_hash.as_deref(),
            &self.base.repo_root,
            pkg_dep_graph.package_manager(),
            pkg_dep_graph.lockfile(),
            &root_turbo_json.global_deps,
            &env_at_execution_start,
            &root_turbo_json.global_env,
            root_turbo_json.global_pass_through_env.as_deref(),
            opts.run_opts.env_mode,
            opts.run_opts.framework_inference,
            root_turbo_json.global_dot_env.as_deref(),
        )?;

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
                    let task_name = TaskName::from(target.as_str()).into_root_task();

                    if root_turbo_json.pipeline.contains_key(&task_name) {
                        filtered_pkgs.insert(WorkspaceName::Root);
                        break;
                    }
                }
            }

            filtered_pkgs
        };

        let global_hash = global_hash_inputs.calculate_global_hash_from_inputs();
        let api_auth = self.base.api_auth()?;

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
        .with_workspaces(filtered_pkgs.clone().into_iter().collect())
        .with_tasks(
            opts.run_opts
                .tasks
                .iter()
                .map(|task| TaskName::from(task.as_str()).into_owned()),
        )
        .build()?;

        let mut global_env_mode = opts.run_opts.env_mode;
        if matches!(global_env_mode, EnvMode::Infer)
            && root_turbo_json.global_pass_through_env.is_some()
        {
            global_env_mode = EnvMode::Strict;
        }

        let package_inputs_hashes = PackageInputsHashes::calculate_file_hashes(
            &scm,
            engine.tasks().par_bridge(),
            pkg_dep_graph.workspaces().collect(),
            engine.task_definitions(),
            &self.base.repo_root,
        )?;

        let pkg_dep_graph = Arc::new(pkg_dep_graph);
        let engine = Arc::new(engine);
        let api_client = self.base.api_client()?;

        let async_cache = AsyncCache::new(
            &opts.cache_opts,
            &self.base.repo_root,
            api_client.clone(),
            api_auth.clone(),
        )?;

        let color_selector = ColorSelector::default();

        let runcache = Arc::new(RunCache::new(
            async_cache,
            &self.base.repo_root,
            &opts.runcache_opts,
            color_selector,
            None,
            self.base.ui,
        ));

        let run_tracker = RunTracker::new(
            started_at,
            "todo",
            opts.scope_opts.pkg_inference_root.as_deref(),
            &env_at_execution_start,
            &self.base.repo_root,
            self.base.version(),
            opts.run_opts.experimental_space_id.clone(),
            api_client,
            api_auth,
            Vendor::get_user(),
        );

        let mut visitor = Visitor::new(
            pkg_dep_graph.clone(),
            runcache,
            run_tracker,
            &opts,
            package_inputs_hashes,
            &env_at_execution_start,
            &global_hash,
            global_env_mode,
            self.base.ui,
            true,
            self.processes.clone(),
            &self.base.repo_root,
            // TODO: this is only needed for full execution, figure out better way to model this
            // not affecting a dry run
            EnvironmentVariableMap::default(),
        );

        visitor.dry_run();

        visitor.visit(engine.clone()).await?;
        let task_hash_tracker = visitor.into_task_hash_tracker();

        Ok((global_hash, task_hash_tracker))
    }
}
