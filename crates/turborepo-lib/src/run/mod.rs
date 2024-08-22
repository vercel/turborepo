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

use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    sync::Arc,
    time::Duration,
};

pub use cache::{CacheOutput, ConfigCache, Error as CacheError, RunCache, TaskCache};
use chrono::{DateTime, Local};
use rayon::iter::ParallelBridge;
use tokio::{select, task::JoinHandle};
use tracing::{debug, instrument};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageNode};
use turborepo_scm::SCM;
use turborepo_telemetry::events::generic::GenericEventBuilder;
use turborepo_ui::{cprint, cprintln, tui, tui::AppSender, ColorConfig, BOLD_GREY, GREY};

pub use crate::run::error::Error;
use crate::{
    cli::EnvMode,
    engine::Engine,
    opts::Opts,
    process::ProcessManager,
    run::{global_hash::get_global_hash_inputs, summary::RunTracker, task_access::TaskAccess},
    signal::SignalHandler,
    task_graph::Visitor,
    task_hash::{get_external_deps_hash, get_internal_deps_hash, PackageInputsHashes},
    turbo_json::{TurboJson, UIMode},
    DaemonClient, DaemonConnector,
};

#[derive(Clone)]
pub struct Run {
    version: &'static str,
    color_config: ColorConfig,
    start_at: DateTime<Local>,
    processes: ProcessManager,
    run_telemetry: GenericEventBuilder,
    repo_root: AbsoluteSystemPathBuf,
    opts: Arc<Opts>,
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
    ui_mode: UIMode,
}

impl Run {
    fn has_persistent_tasks(&self) -> bool {
        self.engine.has_persistent_tasks
    }
    fn print_run_prelude(&self) {
        let targets_list = self.opts.run_opts.tasks.join(", ");
        if self.opts.run_opts.single_package {
            cprint!(self.color_config, GREY, "{}", "• Running");
            cprint!(self.color_config, BOLD_GREY, " {}\n", targets_list);
        } else {
            let mut packages = self
                .filtered_pkgs
                .iter()
                .map(|workspace_name| workspace_name.to_string())
                .collect::<Vec<String>>();
            packages.sort();
            cprintln!(
                self.color_config,
                GREY,
                "• Packages in scope: {}",
                packages.join(", ")
            );
            cprint!(self.color_config, GREY, "{} ", "• Running");
            cprint!(self.color_config, BOLD_GREY, "{}", targets_list);
            cprint!(
                self.color_config,
                GREY,
                " in {} packages\n",
                self.filtered_pkgs.len()
            );
        }

        let use_http_cache = !self.opts.cache_opts.skip_remote;
        if use_http_cache {
            cprintln!(self.color_config, GREY, "• Remote caching enabled");
        } else {
            cprintln!(self.color_config, GREY, "• Remote caching disabled");
        }
    }

    pub fn opts(&self) -> &Opts {
        &self.opts
    }

    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        &self.repo_root
    }

    pub fn scm(&self) -> &SCM {
        &self.scm
    }

    pub fn root_turbo_json(&self) -> &TurboJson {
        &self.root_turbo_json
    }

    pub fn create_run_for_persistent_tasks(&self) -> Self {
        let mut new_run = self.clone();
        let new_engine = new_run.engine.create_engine_for_persistent_tasks();
        new_run.engine = Arc::new(new_engine);

        new_run
    }

    pub fn create_run_without_persistent_tasks(&self) -> Self {
        let mut new_run = self.clone();
        let new_engine = new_run.engine.create_engine_without_persistent_tasks();
        new_run.engine = Arc::new(new_engine);

        new_run
    }

    // Produces the transitive closure of the filtered packages,
    // i.e. the packages relevant for this run.
    #[instrument(skip(self), ret)]
    pub fn get_relevant_packages(&self) -> HashSet<PackageName> {
        let packages: Vec<_> = self
            .filtered_pkgs
            .iter()
            .map(|pkg| PackageNode::Workspace(pkg.clone()))
            .collect();
        self.pkg_dep_graph
            .transitive_closure(&packages)
            .into_iter()
            .filter_map(|node| match node {
                PackageNode::Root => None,
                PackageNode::Workspace(pkg) => Some(pkg.clone()),
            })
            .collect()
    }

    // Produces a map of tasks to the packages where they're defined.
    // Used to print a list of potential tasks to run. Obeys the `--filter` flag
    pub fn get_potential_tasks(&self) -> Result<BTreeMap<String, Vec<String>>, Error> {
        let mut tasks = BTreeMap::new();
        for (name, info) in self.pkg_dep_graph.packages() {
            if !self.filtered_pkgs.contains(name) {
                continue;
            }
            for task_name in info.package_json.scripts.keys() {
                tasks
                    .entry(task_name.clone())
                    .or_insert_with(Vec::new)
                    .push(name.to_string())
            }
        }

        Ok(tasks)
    }

    pub fn pkg_dep_graph(&self) -> &PackageGraph {
        &self.pkg_dep_graph
    }

    pub fn filtered_pkgs(&self) -> &HashSet<PackageName> {
        &self.filtered_pkgs
    }

    pub fn color_config(&self) -> ColorConfig {
        self.color_config
    }

    pub fn has_tui(&self) -> bool {
        self.ui_mode.use_tui()
    }

    pub fn should_start_ui(&self) -> Result<bool, Error> {
        Ok(self.ui_mode.use_tui()
            && self.opts.run_opts.dry_run.is_none()
            && tui::terminal_big_enough()?)
    }

    #[allow(clippy::type_complexity)]
    pub fn start_experimental_ui(
        &self,
    ) -> Result<Option<(AppSender, JoinHandle<Result<(), tui::Error>>)>, Error> {
        // Print prelude here as this needs to happen before the UI is started
        if self.should_print_prelude {
            self.print_run_prelude();
        }

        if !self.should_start_ui()? {
            return Ok(None);
        }

        let task_names = self.engine.tasks_with_command(&self.pkg_dep_graph);
        // If there aren't any tasks to run, then shouldn't start the UI
        if task_names.is_empty() {
            return Ok(None);
        }

        let (sender, receiver) = AppSender::new();
        let handle = tokio::task::spawn_blocking(move || tui::run_app(task_names, receiver));

        Ok(Some((sender, handle)))
    }

    /// Returns a handle that can be used to stop a run
    pub fn stopper(&self) -> RunStopper {
        RunStopper {
            manager: self.processes.clone(),
        }
    }

    pub async fn run(
        &mut self,
        experimental_ui_sender: Option<AppSender>,
        is_watch: bool,
    ) -> Result<i32, Error> {
        let skip_cache_writes = self.opts.runcache_opts.skip_writes;
        if let Some(subscriber) = self.signal_handler.subscribe() {
            let run_cache = self.run_cache.clone();
            tokio::spawn(async move {
                // Cache writes are disabled, can skip setting up cache write listener
                if skip_cache_writes {
                    return;
                }
                let _guard = subscriber.listen().await;
                let spinner = turborepo_ui::start_spinner("...Finishing writing to cache...");
                if let Ok((status, closed)) = run_cache.shutdown_cache().await {
                    let fut = async {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            // loop through hashmap, extract items that are still running,
                            // sum up bit per second
                            let (bytes_per_second, bytes_uploaded, bytes_total) = {
                                let status = status.lock().unwrap();
                                let total_bps: f64 = status
                                    .iter()
                                    .filter_map(|(_hash, task)| task.average_bps())
                                    .sum();
                                let bytes_uploaded: usize =
                                    status.iter().filter_map(|(_hash, task)| task.bytes()).sum();
                                let bytes_total: usize = status
                                    .iter()
                                    .filter(|(_hash, task)| !task.done())
                                    .filter_map(|(_hash, task)| task.size())
                                    .sum();
                                (total_bps, bytes_uploaded, bytes_total)
                            };

                            if bytes_total == 0 {
                                continue;
                            }

                            // convert to human readable
                            let mut formatter = human_format::Formatter::new();
                            let formatter = formatter.with_decimals(2).with_separator("");
                            let bytes_per_second =
                                formatter.with_units("B/s").format(bytes_per_second);
                            let bytes_remaining = formatter
                                .with_units("B")
                                .format(bytes_total.saturating_sub(bytes_uploaded) as f64);

                            spinner.set_message(format!(
                                "...Finishing writing to cache... ({} remaining, {})",
                                bytes_remaining, bytes_per_second
                            ));
                        }
                    };

                    let interrupt = async {
                        if let Ok(fut) = crate::commands::run::get_signal() {
                            fut.await;
                        } else {
                            tracing::warn!("could not register ctrl-c handler");
                            // wait forever
                            tokio::time::sleep(Duration::MAX).await;
                        }
                    };

                    select! {
                        _ = closed => {}
                        _ = fut => {}
                        _ = interrupt => {tracing::debug!("received interrupt, exiting");}
                    }
                } else {
                    tracing::warn!("could not start shutdown, exiting");
                }
                spinner.finish_and_clear();
            });
        }

        if let Some(graph_opts) = &self.opts.run_opts.graph {
            graph_visualizer::write_graph(
                self.color_config,
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

        let root_internal_dependencies_hash = is_monorepo
            .then(|| {
                get_internal_deps_hash(
                    &self.scm,
                    &self.repo_root,
                    self.pkg_dep_graph
                        .root_internal_package_dependencies_paths(),
                )
            })
            .transpose()?;

        let global_hash_inputs = {
            let env_mode = self.opts.run_opts.env_mode;
            let pass_through_env = match env_mode {
                EnvMode::Loose => {
                    // Remove the passthroughs from hash consideration if we're explicitly loose.
                    None
                }
                EnvMode::Strict => self.root_turbo_json.global_pass_through_env.as_deref(),
            };

            get_global_hash_inputs(
                root_external_dependencies_hash.as_deref(),
                root_internal_dependencies_hash.as_deref(),
                root_workspace,
                &self.repo_root,
                self.pkg_dep_graph.package_manager(),
                self.pkg_dep_graph.lockfile(),
                &self.root_turbo_json.global_deps,
                &self.env_at_execution_start,
                &self.root_turbo_json.global_env,
                pass_through_env,
                env_mode,
                self.opts.run_opts.framework_inference,
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
            self.color_config,
            self.processes.clone(),
            &self.repo_root,
            global_env,
            experimental_ui_sender,
            is_watch,
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

#[derive(Debug, Clone)]
pub struct RunStopper {
    manager: ProcessManager,
}

impl RunStopper {
    pub async fn stop(&self) {
        self.manager.stop().await;
    }
}
