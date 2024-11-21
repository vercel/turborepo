use std::{
    collections::{HashMap, HashSet},
    io::{ErrorKind, IsTerminal},
    sync::Arc,
    time::SystemTime,
};

use chrono::Local;
use itertools::Itertools;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_analytics::{start_analytics, AnalyticsHandle, AnalyticsSender};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_cache::AsyncCache;
use turborepo_env::EnvironmentVariableMap;
use turborepo_errors::Spanned;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
    package_json,
    package_json::PackageJson,
};
use turborepo_scm::SCM;
use turborepo_telemetry::events::{
    command::CommandEventBuilder,
    generic::{DaemonInitStatus, GenericEventBuilder},
    repo::{RepoEventBuilder, RepoType},
    EventBuilder, TrackedErrors,
};
use turborepo_ui::{ColorConfig, ColorSelector};
#[cfg(feature = "daemon-package-discovery")]
use {
    crate::run::package_discovery::DaemonPackageDiscovery,
    std::time::Duration,
    turborepo_repository::discovery::{
        Error as DiscoveryError, FallbackPackageDiscovery, LocalPackageDiscoveryBuilder,
        PackageDiscoveryBuilder,
    },
};

use super::task_id::TaskId;
use crate::{
    cli::DryRunMode,
    commands::CommandBase,
    engine::{Engine, EngineBuilder},
    micro_frontends::MicroFrontendsConfigs,
    opts::Opts,
    process::ProcessManager,
    run::{scope, task_access::TaskAccess, task_id::TaskName, Error, Run, RunCache},
    shim::TurboState,
    signal::{SignalHandler, SignalSubscriber},
    turbo_json::{TurboJson, TurboJsonLoader, UIMode},
    DaemonConnector,
};

pub struct RunBuilder {
    processes: ProcessManager,
    opts: Opts,
    api_auth: Option<APIAuth>,
    repo_root: AbsoluteSystemPathBuf,
    root_turbo_json_path: AbsoluteSystemPathBuf,
    color_config: ColorConfig,
    version: &'static str,
    api_client: APIClient,
    analytics_sender: Option<AnalyticsSender>,
    // In watch mode, we can have a changed package that we want to serve as an entrypoint.
    // We will then prune away any tasks that do not depend on tasks inside
    // this package.
    entrypoint_packages: Option<HashSet<PackageName>>,
    should_print_prelude_override: Option<bool>,
    allow_missing_package_manager: bool,
    allow_no_turbo_json: bool,
    // In query, we don't want to validate the engine. Defaults to `true`
    should_validate_engine: bool,
    // If true, we will add all tasks to the graph, even if they are not specified
    add_all_tasks: bool,
}

impl RunBuilder {
    pub fn new(base: CommandBase) -> Result<Self, Error> {
        let api_client = base.api_client()?;

        let opts = Opts::new(&base)?;
        let api_auth = base.api_auth()?;
        let config = base.config()?;
        let allow_missing_package_manager = config.allow_no_package_manager();

        let version = base.version();
        let processes = ProcessManager::new(
            // We currently only use a pty if the following are met:
            // - we're attached to a tty
            atty::is(atty::Stream::Stdout) &&
            // - if we're on windows, we're using the UI
            (!cfg!(windows) || matches!(opts.run_opts.ui_mode, UIMode::Tui)),
        );
        let root_turbo_json_path = config.root_turbo_json_path(&base.repo_root);
        let allow_no_turbo_json = config.allow_no_turbo_json();

        let CommandBase {
            repo_root,
            color_config: ui,
            ..
        } = base;

        Ok(Self {
            processes,
            opts,
            api_client,
            repo_root,
            color_config: ui,
            version,
            api_auth,
            analytics_sender: None,
            entrypoint_packages: None,
            should_print_prelude_override: None,
            allow_missing_package_manager,
            root_turbo_json_path,
            allow_no_turbo_json,
            should_validate_engine: true,
            add_all_tasks: false,
        })
    }

    pub fn with_entrypoint_packages(mut self, entrypoint_packages: HashSet<PackageName>) -> Self {
        self.entrypoint_packages = Some(entrypoint_packages);
        self
    }

    pub fn hide_prelude(mut self) -> Self {
        self.should_print_prelude_override = Some(false);
        self
    }

    pub fn add_all_tasks(mut self) -> Self {
        self.add_all_tasks = true;
        self
    }

    pub fn do_not_validate_engine(mut self) -> Self {
        self.should_validate_engine = false;
        self
    }

    fn connect_process_manager(&self, signal_subscriber: SignalSubscriber) {
        let manager = self.processes.clone();
        tokio::spawn(async move {
            let _guard = signal_subscriber.listen().await;
            manager.stop().await;
        });
    }

    pub fn with_analytics_sender(mut self, analytics_sender: Option<AnalyticsSender>) -> Self {
        self.analytics_sender = analytics_sender;
        self
    }

    pub fn calculate_filtered_packages(
        repo_root: &AbsoluteSystemPath,
        opts: &Opts,
        pkg_dep_graph: &PackageGraph,
        scm: &SCM,
        root_turbo_json: &TurboJson,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, Error> {
        let (mut filtered_pkgs, is_all_packages) = scope::resolve_packages(
            &opts.scope_opts,
            repo_root,
            pkg_dep_graph,
            scm,
            root_turbo_json,
        )?;

        if is_all_packages {
            for target in opts.run_opts.tasks.iter() {
                let mut task_name = TaskName::from(target.as_str());
                // If it's not a package task, we convert to a root task
                if !task_name.is_package_task() {
                    task_name = task_name.into_root_task()
                }

                if root_turbo_json.tasks.contains_key(&task_name) {
                    filtered_pkgs.insert(
                        PackageName::Root,
                        PackageInclusionReason::RootTask {
                            task: task_name.to_string(),
                        },
                    );
                    break;
                }
            }
        };

        Ok(filtered_pkgs)
    }

    // Starts analytics and returns handle. This is not included in the main `build`
    // function because we don't want the handle stored in the `Run` struct.
    pub fn start_analytics(&self) -> (Option<AnalyticsSender>, Option<AnalyticsHandle>) {
        // If there's no API auth, we don't want to record analytics
        let Some(api_auth) = self.api_auth.clone() else {
            return (None, None);
        };
        api_auth
            .is_linked()
            .then(|| start_analytics(api_auth, self.api_client.clone()))
            .unzip()
    }

    #[tracing::instrument(skip(self, signal_handler))]
    pub async fn build(
        mut self,
        signal_handler: &SignalHandler,
        telemetry: CommandEventBuilder,
    ) -> Result<Run, Error> {
        tracing::trace!(
            platform = %TurboState::platform_name(),
            start_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("system time after epoch").as_micros(),
            turbo_version = %TurboState::version(),
            numcpus = num_cpus::get(),
            "performing run on {:?}",
            TurboState::platform_name(),
        );
        let start_at = Local::now();
        if let Some(subscriber) = signal_handler.subscribe() {
            self.connect_process_manager(subscriber);
        }

        let scm = {
            let repo_root = self.repo_root.clone();
            tokio::task::spawn_blocking(move || SCM::new(&repo_root))
        };
        let package_json_path = self.repo_root.join_component("package.json");
        let root_package_json = PackageJson::load(&package_json_path)?;
        let run_telemetry = GenericEventBuilder::new().with_parent(&telemetry);
        let repo_telemetry =
            RepoEventBuilder::new(&self.repo_root.to_string()).with_parent(&telemetry);

        // Pulled from initAnalyticsClient in run.go
        let is_linked = turborepo_api_client::is_linked(&self.api_auth);
        run_telemetry.track_is_linked(is_linked);
        run_telemetry.track_arg_usage(
            "dangerously_allow_missing_package_manager",
            self.allow_missing_package_manager,
        );
        // we only track the remote cache if we're linked because this defaults to
        // Vercel
        if is_linked {
            run_telemetry.track_remote_cache(self.api_client.base_url());
        }
        let _is_structured_output = self.opts.run_opts.graph.is_some()
            || matches!(self.opts.run_opts.dry_run, Some(DryRunMode::Json));

        let is_single_package = self.opts.run_opts.single_package;
        repo_telemetry.track_type(if is_single_package {
            RepoType::SinglePackage
        } else {
            RepoType::Monorepo
        });

        let is_ci_or_not_tty = turborepo_ci::is_ci() || !std::io::stdout().is_terminal();
        run_telemetry.track_ci(turborepo_ci::Vendor::get_name());

        // Remove allow when daemon is flagged back on
        let daemon = match (is_ci_or_not_tty, self.opts.run_opts.daemon) {
            (true, None) => {
                run_telemetry.track_daemon_init(DaemonInitStatus::Skipped);
                debug!("skipping turbod since we appear to be in a non-interactive context");
                None
            }
            (_, Some(true)) | (false, None) => {
                let can_start_server = true;
                let can_kill_server = true;
                let connector =
                    DaemonConnector::new(can_start_server, can_kill_server, &self.repo_root);
                match (connector.connect().await, self.opts.run_opts.daemon) {
                    (Ok(client), _) => {
                        run_telemetry.track_daemon_init(DaemonInitStatus::Started);
                        debug!("running in daemon mode");
                        Some(client)
                    }
                    (Err(e), Some(true)) => {
                        run_telemetry.track_daemon_init(DaemonInitStatus::Failed);
                        debug!("failed to connect to daemon when forced {e}, exiting");
                        return Err(e.into());
                    }
                    (Err(e), None) => {
                        run_telemetry.track_daemon_init(DaemonInitStatus::Failed);
                        debug!("failed to connect to daemon {e}");
                        None
                    }
                    (_, Some(false)) => unreachable!(),
                }
            }
            (_, Some(false)) => {
                run_telemetry.track_daemon_init(DaemonInitStatus::Disabled);
                debug!("skipping turbod since --no-daemon was passed");
                None
            }
        };

        let mut pkg_dep_graph = {
            let builder = PackageGraph::builder(&self.repo_root, root_package_json.clone())
                .with_single_package_mode(self.opts.run_opts.single_package)
                .with_allow_no_package_manager(self.allow_missing_package_manager);

            // Daemon package discovery depends on packageManager existing in package.json
            let graph = if cfg!(feature = "daemon-package-discovery")
                && !self.allow_missing_package_manager
            {
                match (&daemon, self.opts.run_opts.daemon) {
                    (None, Some(true)) => {
                        // We've asked for the daemon, but it's not available. This is an error
                        return Err(turborepo_repository::package_graph::Error::Discovery(
                            DiscoveryError::Unavailable,
                        )
                        .into());
                    }
                    (Some(daemon), Some(true)) => {
                        // We have the daemon, and have explicitly asked to only use that
                        let daemon_discovery = DaemonPackageDiscovery::new(daemon.clone());
                        builder
                            .with_package_discovery(daemon_discovery)
                            .build()
                            .await
                    }
                    (_, Some(false)) | (None, _) => {
                        // We have explicitly requested to not use the daemon, or we don't have it
                        // No change to default.
                        builder.build().await
                    }
                    (Some(daemon), None) => {
                        // We have the daemon, and it's not flagged off. Use the fallback strategy
                        let daemon_discovery = DaemonPackageDiscovery::new(daemon.clone());
                        let local_discovery = LocalPackageDiscoveryBuilder::new(
                            self.repo_root.clone(),
                            None,
                            Some(root_package_json.clone()),
                        )
                        .build()?;
                        let fallback_discover = FallbackPackageDiscovery::new(
                            daemon_discovery,
                            local_discovery,
                            Duration::from_millis(10),
                        );
                        builder
                            .with_package_discovery(fallback_discover)
                            .build()
                            .await
                    }
                }
            } else {
                builder.build().await
            };

            match graph {
                Ok(graph) => graph,
                // if we can't find the package.json, it is a bug, and we should report it.
                // likely cause is that package discovery watching is not up to date.
                // note: there _is_ a false positive from a race condition that can occur
                //       from toctou if the package.json is deleted, but we'd like to know
                Err(turborepo_repository::package_graph::Error::PackageJson(
                    package_json::Error::Io(io),
                )) if io.kind() == ErrorKind::NotFound => {
                    run_telemetry.track_error(TrackedErrors::InvalidPackageDiscovery);
                    return Err(turborepo_repository::package_graph::Error::PackageJson(
                        package_json::Error::Io(io),
                    )
                    .into());
                }
                Err(e) => return Err(e.into()),
            }
        };

        repo_telemetry.track_package_manager(pkg_dep_graph.package_manager().to_string());
        repo_telemetry.track_size(pkg_dep_graph.len());
        run_telemetry.track_run_type(self.opts.run_opts.dry_run.is_some());
        let micro_frontend_configs = MicroFrontendsConfigs::new(&self.repo_root, &pkg_dep_graph)?;

        let scm = scm.await.expect("detecting scm panicked");
        let async_cache = AsyncCache::new(
            &self.opts.cache_opts,
            &self.repo_root,
            self.api_client.clone(),
            self.api_auth.clone(),
            self.analytics_sender.take(),
        )?;

        // restore config from task access trace if it's enabled
        let task_access = TaskAccess::new(self.repo_root.clone(), async_cache.clone(), &scm);
        task_access.restore_config().await;

        let mut turbo_json_loader = if task_access.is_enabled() {
            TurboJsonLoader::task_access(
                self.repo_root.clone(),
                self.root_turbo_json_path.clone(),
                root_package_json.clone(),
            )
        } else if is_single_package {
            TurboJsonLoader::single_package(
                self.repo_root.clone(),
                self.root_turbo_json_path.clone(),
                root_package_json.clone(),
            )
        } else if self.allow_no_turbo_json && !self.root_turbo_json_path.exists() {
            TurboJsonLoader::workspace_no_turbo_json(
                self.repo_root.clone(),
                pkg_dep_graph.packages(),
            )
        } else if let Some(micro_frontends) = &micro_frontend_configs {
            TurboJsonLoader::workspace_with_microfrontends(
                self.repo_root.clone(),
                self.root_turbo_json_path.clone(),
                pkg_dep_graph.packages(),
                micro_frontends.clone(),
            )
        } else {
            TurboJsonLoader::workspace(
                self.repo_root.clone(),
                self.root_turbo_json_path.clone(),
                pkg_dep_graph.packages(),
            )
        };

        let root_turbo_json = turbo_json_loader.load(&PackageName::Root)?.clone();

        pkg_dep_graph.validate()?;

        let filtered_pkgs = Self::calculate_filtered_packages(
            &self.repo_root,
            &self.opts,
            &pkg_dep_graph,
            &scm,
            &root_turbo_json,
        )?;

        let env_at_execution_start = EnvironmentVariableMap::infer();
        let mut engine = self.build_engine(
            &pkg_dep_graph,
            &root_turbo_json,
            filtered_pkgs.keys(),
            turbo_json_loader.clone(),
            micro_frontend_configs.as_ref(),
        )?;

        if self.opts.run_opts.parallel {
            pkg_dep_graph.remove_package_dependencies();
            engine = self.build_engine(
                &pkg_dep_graph,
                &root_turbo_json,
                filtered_pkgs.keys(),
                turbo_json_loader,
                micro_frontend_configs.as_ref(),
            )?;
        }

        let color_selector = ColorSelector::default();

        let run_cache = Arc::new(RunCache::new(
            async_cache,
            &self.repo_root,
            self.opts.runcache_opts,
            &self.opts.cache_opts,
            color_selector,
            daemon.clone(),
            self.color_config,
            self.opts.run_opts.dry_run.is_some(),
        ));

        let should_print_prelude = self.should_print_prelude_override.unwrap_or_else(|| {
            self.opts.run_opts.dry_run.is_none() && self.opts.run_opts.graph.is_none()
        });

        Ok(Run {
            version: self.version,
            color_config: self.color_config,
            start_at,
            processes: self.processes,
            run_telemetry,
            task_access,
            repo_root: self.repo_root,
            opts: Arc::new(self.opts),
            api_client: self.api_client,
            api_auth: self.api_auth,
            env_at_execution_start,
            filtered_pkgs: filtered_pkgs.keys().cloned().collect(),
            pkg_dep_graph: Arc::new(pkg_dep_graph),
            root_turbo_json,
            scm,
            engine: Arc::new(engine),
            run_cache,
            signal_handler: signal_handler.clone(),
            daemon,
            should_print_prelude,
            micro_frontend_configs,
        })
    }

    fn build_engine<'a>(
        &self,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        filtered_pkgs: impl Iterator<Item = &'a PackageName>,
        turbo_json_loader: TurboJsonLoader,
        micro_frontends_configs: Option<&MicroFrontendsConfigs>,
    ) -> Result<Engine, Error> {
        let mut tasks = self
            .opts
            .run_opts
            .tasks
            .iter()
            .map(|task| {
                // TODO: Pull span info from command
                Spanned::new(TaskName::from(task.as_str()).into_owned())
            })
            .collect::<Vec<_>>();
        let mut workspace_packages = filtered_pkgs.cloned().collect::<Vec<_>>();
        if let Some(micro_frontends_configs) = micro_frontends_configs {
            // TODO: this logic is very similar to what happens inside engine builder and
            // could probably be exposed
            let tasks_from_filter = workspace_packages
                .iter()
                .map(|package| package.as_str())
                .cartesian_product(tasks.iter())
                .map(|(package, task)| {
                    task.task_id().map_or_else(
                        || TaskId::new(package, task.task()).into_owned(),
                        |task_id| task_id.into_owned(),
                    )
                })
                .collect::<HashSet<_>>();
            // we need to add the MFE config packages into the scope here to make sure the
            // proxy gets run
            // TODO(olszewski): We are relying on the fact that persistent tasks must be
            // entry points to the task graph so we can get away with only
            // checking the entrypoint tasks.
            for (mfe_config_package, dev_tasks) in micro_frontends_configs.configs() {
                for dev_task in dev_tasks {
                    if tasks_from_filter.contains(dev_task) {
                        workspace_packages.push(PackageName::from(mfe_config_package.as_str()));
                        tasks.push(Spanned::new(
                            TaskId::new(mfe_config_package, "proxy")
                                .as_task_name()
                                .into_owned(),
                        ));
                        break;
                    }
                }
            }
        }
        let mut builder = EngineBuilder::new(
            &self.repo_root,
            pkg_dep_graph,
            turbo_json_loader,
            self.opts.run_opts.single_package,
        )
        .with_root_tasks(root_turbo_json.tasks.keys().cloned())
        .with_tasks_only(self.opts.run_opts.only)
        .with_workspaces(workspace_packages)
        .with_tasks(tasks);

        if self.add_all_tasks {
            builder = builder.add_all_tasks();
        }

        if !self.should_validate_engine {
            builder = builder.do_not_validate_engine();
        }

        let mut engine = builder.build()?;

        // If we have an initial task, we prune out the engine to only
        // tasks that are reachable from that initial task.
        if let Some(entrypoint_packages) = &self.entrypoint_packages {
            engine = engine.create_engine_for_subgraph(entrypoint_packages);
        }

        if !self.opts.run_opts.parallel && self.should_validate_engine {
            engine
                .validate(
                    pkg_dep_graph,
                    self.opts.run_opts.concurrency,
                    self.opts.run_opts.ui_mode,
                )
                .map_err(Error::EngineValidation)?;
        }

        Ok(engine)
    }
}
