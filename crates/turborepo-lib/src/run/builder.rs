use std::{
    collections::{HashMap, HashSet},
    io::{ErrorKind, IsTerminal},
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::Local;
use tracing::Instrument;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPathBuf};
use turborepo_analytics::{start_analytics, AnalyticsHandle};
use turborepo_api_client::{APIAuth, APIClient, SharedHttpClient};
use turborepo_cache::AsyncCache;
use turborepo_env::EnvironmentVariableMap;
use turborepo_errors::Spanned;
use turborepo_process::ProcessManager;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
    package_json,
    package_json::PackageJson,
};
use turborepo_run_summary::observability;
use turborepo_scm::SCM;
use turborepo_signals::SignalHandler;
use turborepo_task_id::TaskName;
use turborepo_telemetry::events::{
    command::CommandEventBuilder,
    generic::{DaemonInitStatus, GenericEventBuilder},
    repo::{RepoEventBuilder, RepoType},
    EventBuilder, TrackedErrors,
};
use turborepo_types::{DryRunMode, UIMode};
use turborepo_ui::ColorConfig;

use crate::{
    commands::CommandBase,
    engine::{Engine, EngineBuilder, EngineExt},
    microfrontends::MicrofrontendsConfigs,
    opts::Opts,
    run::{scope, task_access::TaskAccess, Error, Run, RunCache},
    shim::TurboState,
    turbo_json::{TurboJson, TurboJsonReader, UnifiedTurboJsonLoader},
};

pub struct RunBuilder {
    processes: ProcessManager,
    opts: Opts,
    api_auth: Option<APIAuth>,
    repo_root: AbsoluteSystemPathBuf,
    color_config: ColorConfig,
    version: &'static str,
    http_client: SharedHttpClient,
    // In watch mode, we can have a changed package that we want to serve as an entrypoint.
    // We will then prune away any tasks that do not depend on tasks inside
    // this package.
    entrypoint_packages: Option<HashSet<PackageName>>,
    should_print_prelude_override: Option<bool>,
    // In query, we don't want to validate the engine. Defaults to `true`
    should_validate_engine: bool,
    // If true, we will add all tasks to the graph, even if they are not specified
    add_all_tasks: bool,
    // When running under `turbo watch`, an output watcher is needed so that
    // the run cache can register output globs and skip restoring outputs
    // that are already on disk. Without this, cache restores write files
    // that trigger the file watcher, causing an infinite rebuild loop.
    output_watcher: Option<Arc<dyn turborepo_run_cache::OutputWatcher>>,
    query_server: Option<Arc<dyn turborepo_query_api::QueryServer>>,
}

impl RunBuilder {
    #[tracing::instrument(skip_all)]
    pub fn new(base: CommandBase, http_client: Option<SharedHttpClient>) -> Result<Self, Error> {
        let http_client = http_client.unwrap_or_default();
        let opts = base.opts();
        let api_auth = base.api_auth()?;

        let version = base.version();
        let processes = ProcessManager::new(
            // We currently only use a pty if the following are met:
            // - we're attached to a tty
            std::io::stdout().is_terminal() &&
            // - if we're on windows, we're using the UI
            (!cfg!(windows) || matches!(opts.run_opts.ui_mode, UIMode::Tui)),
        );

        let CommandBase {
            repo_root,
            color_config: ui,
            opts,
            ..
        } = base;

        Ok(Self {
            processes,
            opts,
            http_client,
            repo_root,
            color_config: ui,
            version,
            api_auth,
            entrypoint_packages: None,
            should_print_prelude_override: None,
            should_validate_engine: true,
            add_all_tasks: false,
            output_watcher: None,
            query_server: None,
        })
    }

    pub fn with_entrypoint_packages(mut self, entrypoint_packages: HashSet<PackageName>) -> Self {
        self.entrypoint_packages = Some(entrypoint_packages);
        self
    }

    pub fn with_output_watcher(
        mut self,
        watcher: Arc<dyn turborepo_run_cache::OutputWatcher>,
    ) -> Self {
        self.output_watcher = Some(watcher);
        self
    }

    pub fn with_query_server(mut self, server: Arc<dyn turborepo_query_api::QueryServer>) -> Self {
        self.query_server = Some(server);
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

    fn will_execute_tasks(&self) -> bool {
        self.opts.run_opts.dry_run.is_none() && self.opts.run_opts.graph.is_none()
    }

    fn should_initialize_http_client(&self) -> bool {
        self.api_auth.as_ref().is_some_and(APIAuth::is_linked)
            || (self.opts.cache_opts.cache.remote.should_use() && self.api_auth.is_some())
    }

    fn api_client_from_http(&self, http_client: reqwest::Client) -> APIClient {
        let timeout = self.opts.api_client_opts.timeout;
        let upload_timeout = self.opts.api_client_opts.upload_timeout;

        APIClient::new_with_client(
            http_client,
            &self.opts.api_client_opts.api_url,
            if timeout > 0 {
                Some(Duration::from_secs(timeout))
            } else {
                None
            },
            if upload_timeout > 0 {
                Some(Duration::from_secs(upload_timeout))
            } else {
                None
            },
            self.version,
            self.opts.api_client_opts.preflight,
        )
    }

    fn all_package_prefixes(pkg_dep_graph: &PackageGraph) -> Vec<RelativeUnixPathBuf> {
        let mut prefixes = pkg_dep_graph
            .packages()
            .filter_map(|(name, _)| pkg_dep_graph.package_dir(name))
            .map(|package_dir| package_dir.to_unix())
            .collect::<Vec<_>>();

        prefixes.extend(
            pkg_dep_graph
                .root_internal_package_dependencies_paths()
                .into_iter()
                .map(|package_dir| package_dir.to_unix()),
        );

        prefixes
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

        // When using exclude-only filters (e.g. --filter=!docs), the semantic is
        // "all packages minus the excluded ones". Root tasks should still be
        // included in this case, just as they are when no filter is specified.
        let filters = opts.scope_opts.get_filters();
        let has_exclude_only_filters = !filters.is_empty()
            && filters.iter().all(|f| f.starts_with('!'));

        // Include root tasks when either no filter is specified (is_all_packages)
        // or when only exclude filters are used and the root is not explicitly
        // excluded by the filter.
        let should_include_root_tasks = is_all_packages
            || (has_exclude_only_filters
                && !filtered_pkgs.contains_key(&PackageName::Root)
                && !filters.iter().any(|f| {
                    // Check if any exclude filter explicitly targets the root
                    // package (e.g. --filter=!//). The root package name is "//".
                    let pattern = f.strip_prefix('!').unwrap_or(f);
                    pattern == PackageName::Root.as_ref()
                }));

        if should_include_root_tasks {
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
        }

        if is_all_packages {
            // When all tasks use package#task syntax, we can narrow the package
            // set to only the referenced packages rather than the entire monorepo.
            let task_names: Vec<TaskName> = opts
                .run_opts
                .tasks
                .iter()
                .map(|t| TaskName::from(t.as_str()))
                .collect();
            let all_package_qualified =
                !task_names.is_empty() && task_names.iter().all(|t| t.is_package_task());
            if all_package_qualified {
                let target_packages: HashSet<PackageName> = task_names
                    .iter()
                    .filter_map(|t| t.package().map(PackageName::from))
                    .collect();
                filtered_pkgs.retain(|pkg, _| target_packages.contains(pkg));
            }
        };

        Ok(filtered_pkgs)
    }

    #[tracing::instrument(skip(self, signal_handler))]
    pub async fn build(
        self,
        signal_handler: &SignalHandler,
        telemetry: CommandEventBuilder,
    ) -> Result<(Run, Option<AnalyticsHandle>), Error> {
        tracing::trace!(
            platform = %TurboState::platform_name(),
            start_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).expect("system time after epoch").as_micros(),
            turbo_version = %TurboState::version(),
            numcpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
            "performing run on {:?}",
            TurboState::platform_name(),
        );
        let start_at = Local::now();

        let scm_task = {
            let repo_root = self.repo_root.clone();
            let git_root = self.opts.git_root.clone();
            tokio::task::spawn_blocking(move || match git_root {
                Some(root) => SCM::new_with_git_root(&repo_root, root),
                None => SCM::new(&repo_root),
            })
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
            self.opts.repo_opts.allow_no_package_manager,
        );
        // we only track the remote cache if we're linked because this defaults to
        // Vercel
        if is_linked {
            run_telemetry.track_remote_cache(&self.opts.api_client_opts.api_url);
        }
        let _is_structured_output = self.opts.run_opts.graph.is_some()
            || matches!(self.opts.run_opts.dry_run, Some(DryRunMode::Json));

        let is_single_package = self.opts.run_opts.single_package;
        repo_telemetry.track_type(if is_single_package {
            RepoType::SinglePackage
        } else {
            RepoType::Monorepo
        });

        run_telemetry.track_ci(turborepo_ci::Vendor::get_name());
        run_telemetry.track_ai_agent(turborepo_ai_agents::get_agent());

        // The daemon is no longer used for `turbo run`. It provided no measurable
        // performance benefit and added IPC overhead. The daemon is still used by
        // `turbo watch` which connects independently.
        run_telemetry.track_daemon_init(DaemonInitStatus::Disabled);

        if self.should_initialize_http_client() {
            self.http_client.activate();
        }

        let mut pkg_dep_graph = {
            let builder = PackageGraph::builder(&self.repo_root, root_package_json.clone())
                .with_single_package_mode(self.opts.run_opts.single_package)
                .with_allow_no_package_manager(self.opts.repo_opts.allow_no_package_manager);

            let graph = builder
                .build()
                .instrument(tracing::info_span!("pkg_dep_graph_build"))
                .await;

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

        repo_telemetry.track_package_manager(pkg_dep_graph.package_manager().name().to_string());
        repo_telemetry.track_size(pkg_dep_graph.len());
        run_telemetry.track_run_type(self.opts.run_opts.dry_run.is_some());

        // Build the repo index using parallel git subprocesses for the tracked
        // index (ls-tree + diff-index) and a race between walk_candidate_files
        // and git ls-files for untracked discovery. The race ensures optimal
        // performance: the walk wins on macOS, ls-files wins on Linux.
        let all_prefixes = Self::all_package_prefixes(&pkg_dep_graph);
        let scm = scm_task
            .instrument(tracing::info_span!("scm_task_await"))
            .await
            .expect("detecting scm panicked");
        let repo_index_task = if all_prefixes.is_empty() {
            None
        } else {
            let scm = scm.clone();
            Some(tokio::task::spawn_blocking(move || {
                let _span = tracing::info_span!("build_repo_index_subprocesses").entered();
                scm.build_repo_index_from_subprocesses(&all_prefixes)
            }))
        };
        let micro_frontend_configs = {
            let _span = tracing::info_span!("micro_frontends_from_disk").entered();
            match MicrofrontendsConfigs::from_disk(&self.repo_root, &pkg_dep_graph) {
                Ok(configs) => configs,
                Err(err) => {
                    return Err(Error::MicroFrontends(err));
                }
            }
        };

        // SCM-independent work runs while the background scm_task continues.
        // The await is deferred until just before the first SCM consumer,
        // letting API client resolution, cache init, turbo.json loading,
        // validation, env inference, and turbo.json preloading overlap with
        // tracked git-index construction.

        let api_client = if self.should_initialize_http_client() {
            let _span = tracing::info_span!("resolve_api_client").entered();
            let http_client = self.http_client.get_or_init().await?;
            Some(self.api_client_from_http(http_client))
        } else {
            None
        };

        let (analytics_sender, analytics_handle) = self
            .api_auth
            .as_ref()
            .filter(|auth| auth.is_linked())
            .map(|auth| {
                let api_client = api_client
                    .clone()
                    .expect("linked analytics require a resolved API client");
                start_analytics(auth.clone(), api_client)
            })
            .unzip();

        let async_cache = {
            let _span = tracing::info_span!("async_cache_new").entered();
            AsyncCache::new(
                &self.opts.cache_opts,
                &self.repo_root,
                api_client.clone(),
                self.api_auth.clone(),
                analytics_sender,
            )?
        };

        let root_turbo_json_path = self.opts.repo_opts.root_turbo_json_path.clone();
        let future_flags = self.opts.future_flags;

        let reader = TurboJsonReader::new(self.repo_root.clone()).with_future_flags(future_flags);

        let turbo_json_loader = {
            let _span = tracing::info_span!("turbo_json_loader_setup").entered();
            if TaskAccess::check_enabled(&self.repo_root) {
                UnifiedTurboJsonLoader::task_access(
                    reader,
                    root_turbo_json_path.clone(),
                    root_package_json.clone(),
                )
            } else if is_single_package {
                UnifiedTurboJsonLoader::single_package(
                    reader,
                    root_turbo_json_path.clone(),
                    root_package_json.clone(),
                )
            } else if !root_turbo_json_path.exists() &&
            // Infer a turbo.json if allowing no turbo.json is explicitly allowed or if MFE configs are discovered
            (self.opts.repo_opts.allow_no_turbo_json || micro_frontend_configs.is_some())
            {
                UnifiedTurboJsonLoader::workspace_no_turbo_json(
                    reader,
                    pkg_dep_graph.packages(),
                    micro_frontend_configs.clone(),
                )
            } else if let Some(micro_frontends) = &micro_frontend_configs {
                UnifiedTurboJsonLoader::workspace_with_microfrontends(
                    reader,
                    root_turbo_json_path.clone(),
                    pkg_dep_graph.packages(),
                    micro_frontends.clone(),
                )
            } else {
                UnifiedTurboJsonLoader::workspace(
                    reader,
                    root_turbo_json_path.clone(),
                    pkg_dep_graph.packages(),
                )
            }
        };

        let root_turbo_json = {
            let _span = tracing::info_span!("root_turbo_json_load").entered();
            turbo_json_loader.load(&PackageName::Root)?.clone()
        };

        {
            let _span = tracing::info_span!("pkg_dep_graph_validate").entered();
            pkg_dep_graph.validate()?;
        }

        let env_at_execution_start = {
            let _span = tracing::info_span!("env_infer").entered();
            EnvironmentVariableMap::infer()
        };
        {
            let _span = tracing::info_span!("turbo_json_preload").entered();
            turbo_json_loader.preload_all();
        }

        let filtered_pkgs = {
            let _span = tracing::info_span!("calculate_filtered_packages").entered();
            Self::calculate_filtered_packages(
                &self.repo_root,
                &self.opts,
                &pkg_dep_graph,
                &scm,
                &root_turbo_json,
            )?
        };

        let task_access = {
            let _span = tracing::info_span!("task_access_setup").entered();
            let ta = TaskAccess::new(self.repo_root.clone(), async_cache.clone(), &scm);
            ta.restore_config().await;
            ta
        };

        let mut engine = self.build_engine(
            &pkg_dep_graph,
            &root_turbo_json,
            filtered_pkgs.keys(),
            &turbo_json_loader,
        )?;

        if self.opts.run_opts.parallel {
            pkg_dep_graph.remove_package_dependencies();
            engine = self.build_engine(
                &pkg_dep_graph,
                &root_turbo_json,
                filtered_pkgs.keys(),
                &turbo_json_loader,
            )?;
        }

        let should_print_prelude = self
            .should_print_prelude_override
            .unwrap_or_else(|| self.will_execute_tasks());

        let run_cache = Arc::new(RunCache::new(
            async_cache,
            &self.repo_root,
            self.opts.runcache_opts,
            &self.opts.cache_opts,
            self.output_watcher,
            self.color_config,
            self.opts.run_opts.dry_run.is_some(),
        ));

        // futureFlags are hard gates: reject observability config when disabled.
        if let Some(obs_opts) = &self.opts.experimental_observability {
            if obs_opts.otel.is_some() && !self.opts.future_flags.experimental_observability {
                return Err(turborepo_config::Error::InvalidExperimentalOtelConfig {
                    message: "experimentalObservability.otel is configured but \
                              futureFlags.experimentalObservability is not enabled in turbo.json."
                        .to_string(),
                }
                .into());
            }
        }

        let observability_handle = self
            .opts
            .experimental_observability
            .as_ref()
            .and_then(|opts| {
                let token = opts
                    .otel
                    .as_ref()
                    .and_then(|otel| otel.use_remote_cache_token)
                    .unwrap_or(false)
                    .then(|| self.api_auth.as_ref().map(|auth| auth.token.expose()))
                    .flatten();
                observability::Handle::try_init(opts, token)
            });
        let repo_index = Arc::new(match repo_index_task {
            Some(repo_index_task) => repo_index_task
                .instrument(tracing::info_span!("repo_index_untracked_await"))
                .await
                .expect("scoping repo index panicked"),
            None => None,
        });
        Ok((
            Run {
                version: self.version,
                color_config: self.color_config,
                start_at,
                processes: self.processes,
                run_telemetry,
                task_access,
                repo_root: self.repo_root,
                opts: Arc::new(self.opts),
                api_auth: self.api_auth,
                env_at_execution_start,
                filtered_pkgs: filtered_pkgs.keys().cloned().collect(),
                pkg_dep_graph: Arc::new(pkg_dep_graph),
                turbo_json_loader,
                root_turbo_json,
                scm,
                engine: Arc::new(engine),
                run_cache,
                signal_handler: signal_handler.clone(),
                should_print_prelude,
                micro_frontend_configs,
                repo_index,
                observability_handle,
                query_server: self.query_server,
            },
            analytics_handle,
        ))
    }

    #[tracing::instrument(skip_all)]
    fn build_engine<'a>(
        &self,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        filtered_pkgs: impl Iterator<Item = &'a PackageName>,
        turbo_json_loader: &impl turborepo_engine::TurboJsonLoader,
    ) -> Result<Engine, Error> {
        let tasks = self.opts.run_opts.tasks.iter().map(|task| {
            // TODO: Pull span info from command
            Spanned::new(TaskName::from(task.as_str()).into_owned())
        });
        let mut builder = EngineBuilder::new(
            &self.repo_root,
            pkg_dep_graph,
            turbo_json_loader,
            self.opts.run_opts.single_package,
        )
        .with_root_tasks(root_turbo_json.tasks.keys().cloned())
        .with_tasks_only(self.opts.run_opts.only)
        .with_workspaces(filtered_pkgs.cloned().collect())
        .with_future_flags(self.opts.future_flags)
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
                    self.will_execute_tasks(),
                )
                .map_err(Error::EngineValidation)?;
        }

        Ok(engine)
    }
}
