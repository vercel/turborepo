#![allow(dead_code)]

pub mod builder;
mod cache;
mod error;
pub(crate) mod global_hash;
mod graph_visualizer;
pub(crate) mod package_discovery;
mod scope;
pub(crate) mod summary;
pub mod task_access;
pub mod task_id;
pub mod watch;

use std::{collections::HashSet, io::Write, sync::Arc};

pub use cache::{ConfigCache, RunCache, TaskCache};
use chrono::{DateTime, Local};
use rayon::iter::ParallelBridge;
use tracing::debug;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageName};
use turborepo_scm::SCM;
use turborepo_telemetry::events::generic::GenericEventBuilder;
use turborepo_ui::{cprint, cprintln, BOLD_GREY, GREY, UI};

pub use crate::run::error::Error;
use crate::{
    cli::EnvMode,
    engine::Engine,
    opts::Opts,
    process::ProcessManager,
    run::{global_hash::get_global_hash_inputs, summary::RunTracker, task_access::TaskAccess},
    signal::SignalHandler,
    task_graph::Visitor,
    task_hash::{get_external_deps_hash, PackageInputsHashes},
    turbo_json::TurboJson,
    DaemonClient, DaemonConnector,
};

pub struct Run {
    version: &'static str,
    ui: UI,
    experimental_ui: bool,
    start_at: DateTime<Local>,
    processes: ProcessManager,
    run_telemetry: GenericEventBuilder,
    repo_root: AbsoluteSystemPathBuf,
    opts: Opts,
    api_client: APIClient,
    api_auth: Option<APIAuth>,
    env_at_execution_start: EnvironmentVariableMap,
    filtered_pkgs: HashSet<PackageName>,
    pkg_dep_graph: Arc<PackageGraph>,
    root_turbo_json: TurboJson,
    scm: SCM,
    run_cache: Arc<RunCache>,
    signal_handler: SignalHandler,
    engine: Arc<Engine>,
    task_access: TaskAccess,
    daemon: Option<DaemonClient<DaemonConnector>>,
    should_print_prelude: bool,
}

impl Run {
    fn has_persistent_tasks(&self) -> bool {
        self.engine.has_persistent_tasks
    }
    fn print_run_prelude(&self) {
        let targets_list = self.opts.run_opts.tasks.join(", ");
        if self.opts.run_opts.single_package {
            cprint!(self.ui, GREY, "{}", "• Running");
            cprint!(self.ui, BOLD_GREY, " {}\n", targets_list);
        } else {
            let mut packages = self
                .filtered_pkgs
                .iter()
                .map(|workspace_name| workspace_name.to_string())
                .collect::<Vec<String>>();
            packages.sort();
            cprintln!(
                self.ui,
                GREY,
                "• Packages in scope: {}",
                packages.join(", ")
            );
            cprint!(self.ui, GREY, "{} ", "• Running");
            cprint!(self.ui, BOLD_GREY, "{}", targets_list);
            cprint!(self.ui, GREY, " in {} packages\n", self.filtered_pkgs.len());
        }

        let use_http_cache = !self.opts.cache_opts.skip_remote;
        if use_http_cache {
            cprintln!(self.ui, GREY, "• Remote caching enabled");
        } else {
            cprintln!(self.ui, GREY, "• Remote caching disabled");
        }
    }

    pub async fn run(&mut self) -> Result<i32, Error> {
        if self.should_print_prelude {
            self.print_run_prelude();
        }
        if let Some(subscriber) = self.signal_handler.subscribe() {
            let run_cache = self.run_cache.clone();
            tokio::spawn(async move {
                let _guard = subscriber.listen().await;
                let spinner = turborepo_ui::start_spinner("...Finishing writing to cache...");
                run_cache.shutdown_cache().await;
                spinner.finish_and_clear();
            });
        }

        if let Some(graph_opts) = &self.opts.run_opts.graph {
            graph_visualizer::write_graph(
                self.ui,
                graph_opts,
                &self.engine,
                self.opts.run_opts.single_package,
                // Note that cwd used to be pulled from CommandBase, which had it set
                // as the repo root.
                &self.repo_root,
            )?;
            return Ok(0);
        }

        let workspaces = self.pkg_dep_graph.packages().collect();
        let package_inputs_hashes = PackageInputsHashes::calculate_file_hashes(
            &self.scm,
            self.engine.tasks().par_bridge(),
            workspaces,
            self.engine.task_definitions(),
            &self.repo_root,
            &self.run_telemetry,
            &mut self.daemon,
        )?;

        let root_workspace = self
            .pkg_dep_graph
            .package_info(&PackageName::Root)
            .expect("must have root workspace");

        let is_monorepo = !self.opts.run_opts.single_package;

        let root_external_dependencies_hash =
            is_monorepo.then(|| get_external_deps_hash(&root_workspace.transitive_dependencies));

        let global_hash_inputs = {
            let (env_mode, pass_through_env) = match self.opts.run_opts.env_mode {
                // In infer mode, if there is any pass_through config (even if it is an empty array)
                // we'll hash the whole object, so we can detect changes to that config
                // Further, resolve the envMode to the concrete value.
                EnvMode::Infer if self.root_turbo_json.global_pass_through_env.is_some() => (
                    EnvMode::Strict,
                    self.root_turbo_json.global_pass_through_env.as_deref(),
                ),
                EnvMode::Loose => {
                    // Remove the passthroughs from hash consideration if we're explicitly loose.
                    (EnvMode::Loose, None)
                }
                env_mode => (
                    env_mode,
                    self.root_turbo_json.global_pass_through_env.as_deref(),
                ),
            };

            get_global_hash_inputs(
                root_external_dependencies_hash.as_deref(),
                &self.repo_root,
                self.pkg_dep_graph.package_manager(),
                self.pkg_dep_graph.lockfile(),
                &self.root_turbo_json.global_deps,
                &self.env_at_execution_start,
                &self.root_turbo_json.global_env,
                pass_through_env,
                env_mode,
                self.opts.run_opts.framework_inference,
                self.root_turbo_json.global_dot_env.as_deref(),
                &self.scm,
            )?
        };
        let global_hash = global_hash_inputs.calculate_global_hash();

        let global_env = {
            let mut env = self
                .env_at_execution_start
                .from_wildcards(global_hash_inputs.pass_through_env.unwrap_or_default())
                .map_err(Error::Env)?;
            if let Some(resolved_global) = &global_hash_inputs.resolved_env_vars {
                env.union(&resolved_global.all);
            }
            env
        };

        let run_tracker = RunTracker::new(
            self.start_at,
            self.opts.synthesize_command(),
            self.opts.scope_opts.pkg_inference_root.as_deref(),
            &self.env_at_execution_start,
            &self.repo_root,
            self.version,
            self.opts.run_opts.experimental_space_id.clone(),
            self.api_client.clone(),
            self.api_auth.clone(),
            Vendor::get_user(),
            &self.scm,
        );

        let mut visitor = Visitor::new(
            self.pkg_dep_graph.clone(),
            self.run_cache.clone(),
            run_tracker,
            &self.task_access,
            &self.opts.run_opts,
            package_inputs_hashes,
            &self.env_at_execution_start,
            &global_hash,
            self.opts.run_opts.env_mode,
            self.ui,
            self.processes.clone(),
            &self.repo_root,
            global_env,
            self.experimental_ui,
        );

        if self.opts.run_opts.dry_run.is_some() {
            visitor.dry_run();
        }

        // we look for this log line to mark the start of the run
        // in benchmarks, so please don't remove it
        debug!("running visitor");

        let errors = visitor
            .visit(self.engine.clone(), &self.run_telemetry)
            .await?;

        let exit_code = errors
            .iter()
            .filter_map(|err| err.exit_code())
            .max()
            // We hit some error, it shouldn't be exit code 0
            .unwrap_or(if errors.is_empty() { 0 } else { 1 });

        let error_prefix = if self.opts.run_opts.is_github_actions {
            "::error::"
        } else {
            ""
        };
        for err in &errors {
            writeln!(std::io::stderr(), "{error_prefix}{err}").ok();
        }

        visitor
            .finish(
                exit_code,
                &self.filtered_pkgs,
                global_hash_inputs,
                &self.engine,
                &self.env_at_execution_start,
                self.opts.scope_opts.pkg_inference_root.as_deref(),
            )
            .await?;

        Ok(exit_code)
    }
}
