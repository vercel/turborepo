#![allow(dead_code)]

pub mod builder;
mod cache;
mod error;
pub(crate) mod global_hash;
mod graph_visualizer;
pub(crate) mod package_discovery;
pub(crate) mod scope;
pub(crate) mod summary;
pub mod task_access;
mod ui;
pub mod watch;

use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    sync::Arc,
    time::Duration,
};

pub use cache::{CacheOutput, ConfigCache, Error as CacheError, RunCache, TaskCache};
use chrono::{DateTime, Local};
use futures::StreamExt;
use itertools::Itertools;
use rayon::iter::ParallelBridge;
use tokio::{pin, select, task::JoinHandle};
use tracing::{debug, error, info, instrument, warn};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_ci::Vendor;
use turborepo_env::EnvironmentVariableMap;
use turborepo_microfrontends_proxy::ProxyServer;
use turborepo_process::ProcessManager;
use turborepo_repository::package_graph::{PackageGraph, PackageName, PackageNode};
use turborepo_scm::SCM;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::generic::GenericEventBuilder;
use turborepo_ui::{
    cprint, cprintln, sender::UISender, tui, tui::TuiSender, wui::sender::WebUISender, ColorConfig,
    BOLD_GREY, GREY,
};

pub use crate::run::error::Error;
use crate::{
    cli::EnvMode,
    engine::Engine,
    microfrontends::MicrofrontendsConfigs,
    opts::Opts,
    run::{global_hash::get_global_hash_inputs, summary::RunTracker, task_access::TaskAccess},
    task_graph::Visitor,
    task_hash::{get_external_deps_hash, get_internal_deps_hash, PackageInputsHashes},
    turbo_json::{TurboJson, TurboJsonLoader, UIMode},
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
    turbo_json_loader: TurboJsonLoader,
    root_turbo_json: TurboJson,
    scm: SCM,
    run_cache: Arc<RunCache>,
    signal_handler: SignalHandler,
    engine: Arc<Engine>,
    task_access: TaskAccess,
    daemon: Option<DaemonClient<DaemonConnector>>,
    should_print_prelude: bool,
    micro_frontend_configs: Option<MicrofrontendsConfigs>,
}

type UIResult<T> = Result<Option<(T, JoinHandle<Result<(), turborepo_ui::Error>>)>, Error>;

type WuiResult = UIResult<WebUISender>;
type TuiResult = UIResult<TuiSender>;

impl Run {
    fn has_non_interruptible_tasks(&self) -> bool {
        self.engine.has_non_interruptible_tasks
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

        let use_http_cache = self.opts.cache_opts.cache.remote.should_use();
        if use_http_cache {
            cprintln!(self.color_config, GREY, "• Remote caching enabled");
        } else {
            cprintln!(self.color_config, GREY, "• Remote caching disabled");
        }
    }

    pub fn turbo_json_loader(&self) -> &TurboJsonLoader {
        &self.turbo_json_loader
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

    pub fn create_run_for_non_interruptible_tasks(&self) -> Self {
        let mut new_run = Self {
            // ProcessManager is shared via an `Arc`,
            // so we want to explicitly recreate it instead of cloning
            processes: ProcessManager::new(self.processes.use_pty()),
            ..self.clone()
        };

        let new_engine = new_run.engine.create_engine_for_non_interruptible_tasks();
        new_run.engine = Arc::new(new_engine);

        new_run
    }

    pub fn create_run_for_interruptible_tasks(&self) -> Self {
        let mut new_run = self.clone();
        let new_engine = new_run.engine.create_engine_for_interruptible_tasks();
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

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn filtered_pkgs(&self) -> &HashSet<PackageName> {
        &self.filtered_pkgs
    }

    pub fn color_config(&self) -> ColorConfig {
        self.color_config
    }

    pub fn has_tui(&self) -> bool {
        self.opts.run_opts.ui_mode.use_tui()
    }

    pub fn should_start_ui(&self) -> Result<bool, Error> {
        Ok(self.opts.run_opts.ui_mode.use_tui()
            && self.opts.run_opts.dry_run.is_none()
            && tui::terminal_big_enough()?)
    }

    pub fn start_ui(self: &Arc<Self>) -> UIResult<UISender> {
        // Print prelude here as this needs to happen before the UI is started
        if self.should_print_prelude {
            self.print_run_prelude();
        }

        match self.opts.run_opts.ui_mode {
            UIMode::Tui => self
                .start_terminal_ui()
                .map(|res| res.map(|(sender, handle)| (UISender::Tui(sender), handle))),
            UIMode::Stream => Ok(None),
            UIMode::Web => self
                .start_web_ui()
                .map(|res| res.map(|(sender, handle)| (UISender::Wui(sender), handle))),
        }
    }
    fn start_web_ui(self: &Arc<Self>) -> WuiResult {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let handle = tokio::spawn(ui::start_web_ui_server(rx, self.clone()));

        Ok(Some((WebUISender { tx }, handle)))
    }

    #[allow(clippy::type_complexity)]
    fn start_terminal_ui(&self) -> TuiResult {
        if !self.should_start_ui()? {
            return Ok(None);
        }

        let task_names = self.engine.tasks_with_command(&self.pkg_dep_graph);
        // If there aren't any tasks to run, then shouldn't start the UI
        if task_names.is_empty() {
            return Ok(None);
        }

        let (sender, receiver) = TuiSender::new();
        let color_config = self.color_config;
        let scrollback_len = self.opts.tui_opts.scrollback_length;
        let repo_root = self.repo_root.clone();
        let handle = tokio::task::spawn(async move {
            Ok(tui::run_app(
                task_names,
                receiver,
                color_config,
                &repo_root,
                scrollback_len,
            )
            .await?)
        });

        Ok(Some((sender, handle)))
    }

    /// Returns a handle that can be used to stop a run
    pub fn stopper(&self) -> RunStopper {
        RunStopper {
            manager: self.processes.clone(),
        }
    }

    async fn start_proxy_if_needed(
        &self,
    ) -> Result<
        Option<(
            tokio::sync::broadcast::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
        Error,
    > {
        let Some(mfe_configs) = &self.micro_frontend_configs else {
            return Ok(None);
        };

        if !mfe_configs.should_use_turborepo_proxy()
            || !mfe_configs.has_dev_task(self.engine.task_ids())
        {
            return Ok(None);
        }

        info!("Starting Turborepo microfrontends proxy");

        let config = self.load_proxy_config(mfe_configs).await?;
        let (mut server, shutdown_handle) = self.start_proxy_server(config).await?;

        let signal_handler_complete_rx =
            self.setup_shutdown_handlers(&mut server, shutdown_handle.clone());

        tokio::spawn(async move {
            if let Err(e) = server.run().await {
                error!("Turborepo proxy error: {}", e);
            }
        });

        info!("Turborepo proxy started successfully");
        Ok(Some((shutdown_handle, signal_handler_complete_rx)))
    }

    async fn load_proxy_config(
        &self,
        mfe_configs: &MicrofrontendsConfigs,
    ) -> Result<turborepo_microfrontends::Config, Error> {
        let config_path = mfe_configs
            .configs()
            .sorted_by(|(a, _), (b, _)| a.cmp(b))
            .find_map(|(pkg, _tasks)| mfe_configs.config_filename(pkg));

        let Some(config_path) = config_path else {
            return Err(Error::Proxy(
                "No microfrontends config file found".to_string(),
            ));
        };

        let full_path = self.repo_root.join_unix_path(config_path);
        let contents = std::fs::read_to_string(&full_path).map_err(|e| {
            Error::Proxy(format!("Failed to read microfrontends config file: {}", e))
        })?;

        let config = turborepo_microfrontends::TurborepoMfeConfig::from_str(
            &contents,
            full_path.as_str(),
        )
        .map_err(|e| Error::Proxy(format!("Failed to parse microfrontends config: {}", e)))?;

        Ok(config.into_config())
    }

    async fn start_proxy_server(
        &self,
        config: turborepo_microfrontends::Config,
    ) -> Result<(ProxyServer, tokio::sync::broadcast::Sender<()>), Error> {
        let server = ProxyServer::new(config)
            .map_err(|e| Error::Proxy(format!("Failed to create Turborepo proxy: {}", e)))?;

        if !server.check_port_available().await {
            return Err(Error::Proxy("Port is not available.".to_string()));
        }

        let shutdown_handle = server.shutdown_handle();
        Ok((server, shutdown_handle))
    }

    fn setup_shutdown_handlers(
        &self,
        server: &mut ProxyServer,
        shutdown_handle: tokio::sync::broadcast::Sender<()>,
    ) -> tokio::sync::oneshot::Receiver<()> {
        let (proxy_shutdown_complete_tx, proxy_shutdown_complete_rx) =
            tokio::sync::oneshot::channel();
        let (cleanup_complete_tx, cleanup_complete_rx) = tokio::sync::oneshot::channel();
        let (signal_handler_complete_tx, signal_handler_complete_rx) =
            tokio::sync::oneshot::channel();

        server.set_shutdown_complete_tx(proxy_shutdown_complete_tx);

        tokio::spawn(async move {
            if proxy_shutdown_complete_rx.await.is_ok() {
                let _ = cleanup_complete_tx.send(());
                let _ = signal_handler_complete_tx.send(());
            }
        });

        self.register_proxy_signal_handler(shutdown_handle, signal_handler_complete_rx);

        cleanup_complete_rx
    }

    fn register_proxy_signal_handler(
        &self,
        shutdown_handle: tokio::sync::broadcast::Sender<()>,
        shutdown_complete_rx: tokio::sync::oneshot::Receiver<()>,
    ) {
        if let Some(subscriber) = self.signal_handler.subscribe() {
            let process_manager = self.processes.clone();
            tokio::spawn(async move {
                info!("Proxy signal handler registered and waiting");
                let _guard = subscriber.listen().await;
                info!("Signal received! Shutting down proxy BEFORE process manager stops");
                let _ = shutdown_handle.send(());
                debug!("Proxy shutdown signal sent, waiting for shutdown completion notification");

                match tokio::time::timeout(
                    tokio::time::Duration::from_millis(1000),
                    shutdown_complete_rx,
                )
                .await
                {
                    Ok(Ok(())) => {
                        info!("Proxy websocket close complete, now stopping child processes");
                    }
                    Ok(Err(_)) => {
                        warn!("Proxy shutdown notification channel closed unexpectedly");
                    }
                    Err(_) => {
                        info!("Proxy shutdown notification timed out after 500 milliseconds");
                    }
                }

                process_manager.stop().await;
                debug!("Child processes stopped");
            });
        } else {
            warn!("Could not subscribe to signal handler for proxy shutdown");
        }
    }

    fn setup_cache_shutdown_handler(&self) {
        let skip_cache_writes = self.opts.cache_opts.cache.skip_writes();
        if skip_cache_writes {
            return;
        }

        let Some(subscriber) = self.signal_handler.subscribe() else {
            return;
        };

        let run_cache = self.run_cache.clone();
        tokio::spawn(async move {
            let _guard = subscriber.listen().await;
            let spinner = turborepo_ui::start_spinner("...Finishing writing to cache...");

            if let Ok((status, closed)) = run_cache.shutdown_cache().await {
                let fut = async {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

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

                        let mut formatter = human_format::Formatter::new();
                        let formatter = formatter.with_decimals(2).with_separator("");
                        let bytes_per_second = formatter.with_units("B/s").format(bytes_per_second);
                        let bytes_remaining = formatter
                            .with_units("B")
                            .format(bytes_total.saturating_sub(bytes_uploaded) as f64);

                        spinner.set_message(format!(
                            "...Finishing writing to cache... ({bytes_remaining} remaining, \
                             {bytes_per_second})"
                        ));
                    }
                };

                let interrupt = async {
                    if let Ok(fut) = get_signal() {
                        pin!(fut);
                        fut.next().await;
                    } else {
                        tracing::warn!("could not register ctrl-c handler");
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

    fn setup_process_manager_shutdown_handler(&self) {
        let Some(subscriber) = self.signal_handler.subscribe() else {
            return;
        };

        let process_manager = self.processes.clone();
        tokio::spawn(async move {
            let _guard = subscriber.listen().await;
            debug!("Signal received, stopping child processes");
            process_manager.stop().await;
            debug!("Child processes stopped");
        });
    }

    async fn cleanup_proxy(
        &self,
        proxy_shutdown: Option<(
            tokio::sync::broadcast::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    ) {
        let Some((shutdown_tx, shutdown_complete_rx)) = proxy_shutdown else {
            return;
        };

        info!("Shutting down Turborepo proxy gracefully BEFORE stopping child processes");
        let _ = shutdown_tx.send(());
        debug!("Sent shutdown signal to proxy, waiting for completion signal");

        match tokio::time::timeout(tokio::time::Duration::from_secs(2), shutdown_complete_rx).await
        {
            Ok(Ok(())) => {
                info!("Proxy shutdown completed successfully");
            }
            Ok(Err(_)) => {
                warn!("Proxy shutdown channel closed unexpectedly");
            }
            Err(_) => {
                warn!("Proxy shutdown timed out after 2 seconds");
            }
        }

        info!("Proxy shutdown complete, proceeding with visitor cleanup");
    }

    async fn execute_visitor(
        &self,
        ui_sender: Option<UISender>,
        is_watch: bool,
        proxy_shutdown: Option<(
            tokio::sync::broadcast::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    ) -> Result<i32, Error> {
        let workspaces = self.pkg_dep_graph.packages().collect();
        let package_inputs_hashes = PackageInputsHashes::calculate_file_hashes(
            &self.scm,
            self.engine.tasks().par_bridge(),
            workspaces,
            self.engine.task_definitions(),
            &self.repo_root,
            &self.run_telemetry,
            &self.daemon,
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
            &self.env_at_execution_start,
            &self.repo_root,
            self.version,
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
            self.color_config,
            self.processes.clone(),
            &self.repo_root,
            global_env,
            ui_sender,
            is_watch,
            self.micro_frontend_configs.as_ref(),
        )
        .await;

        if self.opts.run_opts.dry_run.is_some() {
            visitor.dry_run();
        }

        debug!("running visitor");

        let errors = visitor
            .visit(self.engine.clone(), &self.run_telemetry)
            .await?;

        debug!("visitor completed, calculating exit code");

        let exit_code = errors
            .iter()
            .filter_map(|err| err.exit_code())
            .max()
            .unwrap_or(if errors.is_empty() { 0 } else { 1 });

        let error_prefix = if self.opts.run_opts.is_github_actions {
            "::error::"
        } else {
            ""
        };
        for err in &errors {
            writeln!(std::io::stderr(), "{error_prefix}{err}").ok();
        }

        self.cleanup_proxy(proxy_shutdown).await;

        // When a proxy is present, the signal handler only stops processes on OS
        // signal. For normal completion without user interruption, we need an
        // explicit stop here.
        self.processes.stop().await;

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

        debug!("visitor.finish() completed, run cleanup done");

        Ok(exit_code)
    }

    pub async fn run(&self, ui_sender: Option<UISender>, is_watch: bool) -> Result<i32, Error> {
        let proxy_shutdown = self.start_proxy_if_needed().await?;
        self.setup_cache_shutdown_handler();

        // If there's no proxy, we need a fallback signal handler for the process
        // manager When a proxy is present, register_proxy_signal_handler
        // handles process manager shutdown
        if proxy_shutdown.is_none() {
            self.setup_process_manager_shutdown_handler();
        }

        if let Some(graph_opts) = &self.opts.run_opts.graph {
            graph_visualizer::write_graph(
                self.color_config,
                graph_opts,
                &self.engine,
                self.opts.run_opts.single_package,
                &self.repo_root,
            )?;
            return Ok(0);
        }

        self.execute_visitor(ui_sender, is_watch, proxy_shutdown)
            .await
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
