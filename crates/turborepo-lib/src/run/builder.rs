use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{ErrorKind, IsTerminal},
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::Local;
use globwalk::{ValidatedGlob, WalkType};
use toml::{Table, Value};
use tracing::Instrument;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPathBuf};
use turborepo_analytics::{start_analytics, AnalyticsHandle};
use turborepo_api_client::{APIAuth, APIClient, CacheClient, SharedHttpClient};
use turborepo_cache::{AsyncCache, CacheScmState, LazyScmState};
use turborepo_env::EnvironmentVariableMap;
use turborepo_errors::Spanned;
use turborepo_process::ProcessManager;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName},
    package_json,
    package_json::PackageJson,
    package_manager::PackageManager,
    workspace_provider::{
        CargoWorkspaceProvider, UvWorkspaceProvider, WorkspaceDependencyProvider,
        WorkspaceProviderId, WorkspaceProviderRegistry,
    },
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
use turborepo_types::UIMode;
use turborepo_ui::ColorConfig;
use turborepo_vercel_api::CachingStatusResponse;

use crate::{
    commands::CommandBase,
    engine::{Engine, EngineBuilder, EngineExt},
    microfrontends::MicrofrontendsConfigs,
    opts::Opts,
    run::{
        scope, task_access::TaskAccess, Error, RemoteCacheStatus, RemoteCacheUnavailableReason,
        Run, RunCache,
    },
    shim::TurboState,
    turbo_json::{RawTurboJson, TurboJson, TurboJsonReader, UnifiedTurboJsonLoader},
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
    // In watch mode with `watchUsingTaskInputs`, the file watcher provides
    // the set of changed files that triggered the rebuild. Used to filter
    // the engine down to only tasks whose declared inputs match.
    changed_files_for_watch: Option<HashSet<turbopath::AnchoredSystemPathBuf>>,
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

            should_validate_engine: true,
            add_all_tasks: false,
            output_watcher: None,
            query_server: None,
            changed_files_for_watch: None,
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

    pub fn with_changed_files(mut self, files: HashSet<turbopath::AnchoredSystemPathBuf>) -> Self {
        self.changed_files_for_watch = Some(files);
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

    async fn resolve_remote_cache_status(
        &self,
        preflight_handle: Option<
            tokio::task::JoinHandle<turborepo_api_client::Result<CachingStatusResponse>>,
        >,
    ) -> RemoteCacheStatus {
        use turborepo_vercel_api::CachingStatus;

        if let Some(reason) = self.opts.remote_cache_disabled_reason {
            return RemoteCacheStatus::Disabled(reason);
        }

        let Some(handle) = preflight_handle else {
            return RemoteCacheStatus::Enabled;
        };

        // Wait at most 250ms for the preflight check. This runs concurrently
        // with graph building so in practice it's almost always done by now.
        // If it's not, fall back to "enabled" — the connection warmup still
        // benefits later cache operations.
        let result = tokio::time::timeout(Duration::from_millis(250), handle).await;
        match result {
            Ok(Ok(Ok(response))) => match response.status {
                CachingStatus::Enabled => RemoteCacheStatus::Enabled,
                CachingStatus::Disabled => {
                    RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::DisabledForTeam)
                }
                CachingStatus::OverLimit => {
                    RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::UsageLimitExceeded)
                }
                CachingStatus::Paused => {
                    RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::SpendingPaused)
                }
            },
            Ok(Ok(Err(api_err))) => Self::map_api_error_to_status(api_err),
            Ok(Err(_join_err)) => {
                tracing::debug!("Remote cache preflight task panicked; assuming enabled");
                RemoteCacheStatus::Enabled
            }
            Err(_timeout) => {
                tracing::debug!("Remote cache preflight timed out after 250ms; assuming enabled");
                RemoteCacheStatus::Enabled
            }
        }
    }

    fn map_api_error_to_status(err: turborepo_api_client::Error) -> RemoteCacheStatus {
        match &err {
            turborepo_api_client::Error::ReqwestError(e) if e.is_connect() || e.is_timeout() => {
                RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::CouldNotConnect)
            }
            turborepo_api_client::Error::ReqwestError(e) => {
                if let Some(status) = e.status() {
                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        return RemoteCacheStatus::Unavailable(
                            RemoteCacheUnavailableReason::AuthenticationFailed,
                        );
                    }
                    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        return RemoteCacheStatus::Unavailable(
                            RemoteCacheUnavailableReason::UsageLimitExceeded,
                        );
                    }
                    if status.is_server_error() {
                        return RemoteCacheStatus::Unavailable(
                            RemoteCacheUnavailableReason::UnexpectedServerError,
                        );
                    }
                }
                RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::CouldNotConnect)
            }
            turborepo_api_client::Error::InvalidToken { .. } => {
                RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::AuthenticationFailed)
            }
            turborepo_api_client::Error::ForbiddenToken { .. } => {
                RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::AuthenticationFailed)
            }
            turborepo_api_client::Error::CacheDisabled { status, .. } => {
                use turborepo_vercel_api::CachingStatus;
                match status {
                    CachingStatus::Disabled => RemoteCacheStatus::Unavailable(
                        RemoteCacheUnavailableReason::DisabledForTeam,
                    ),
                    CachingStatus::OverLimit => RemoteCacheStatus::Unavailable(
                        RemoteCacheUnavailableReason::UsageLimitExceeded,
                    ),
                    CachingStatus::Paused => {
                        RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::SpendingPaused)
                    }
                    CachingStatus::Enabled => RemoteCacheStatus::Enabled,
                }
            }
            turborepo_api_client::Error::InvalidJson { .. }
            | turborepo_api_client::Error::UnknownCachingStatus(..)
            | turborepo_api_client::Error::UnknownStatus { .. } => {
                RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::UnexpectedServerError)
            }
            _ => RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::CouldNotConnect),
        }
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

    fn read_workspace_providers_from_root_config(
        repo_root: &AbsoluteSystemPath,
        root_turbo_json_path: &AbsoluteSystemPath,
    ) -> Result<Option<Vec<String>>, Error> {
        let raw_turbo_json = RawTurboJson::read(repo_root, root_turbo_json_path, true)
            .map_err(crate::config::Error::from)?;
        Ok(raw_turbo_json.and_then(|raw| {
            raw.workspace_providers.map(|providers| {
                providers
                    .into_iter()
                    .map(|provider| provider.into_inner().into())
                    .collect()
            })
        }))
    }

    fn load_root_package_json(
        repo_root: &AbsoluteSystemPath,
        workspace_providers: &[WorkspaceProviderId],
    ) -> Result<PackageJson, Error> {
        let package_json_path = repo_root.join_component("package.json");
        if package_json_path.exists() {
            return Ok(PackageJson::load(&package_json_path)?);
        }

        let has_only_non_node_providers = !workspace_providers.is_empty()
            && workspace_providers
                .iter()
                .all(|provider| *provider != WorkspaceProviderId::Node);
        if has_only_non_node_providers {
            return Ok(PackageJson::default());
        }

        PackageJson::load(&package_json_path).map_err(Error::from)
    }

    fn package_manager_for_provider_graph(
        repo_root: &AbsoluteSystemPath,
        root_package_json: &PackageJson,
        workspace_providers: &[WorkspaceProviderId],
    ) -> PackageManager {
        if workspace_providers.contains(&WorkspaceProviderId::Node) {
            PackageManager::read_or_detect_package_manager(root_package_json, repo_root)
                .unwrap_or(PackageManager::Npm)
        } else {
            PackageManager::Npm
        }
    }

    fn include_manifest_glob(member: &str, manifest_name: &str) -> Option<ValidatedGlob> {
        let member = member.trim().trim_start_matches("./");
        if member.is_empty() {
            return None;
        }

        let member = member.trim_end_matches('/');
        let pattern = if member.ends_with(manifest_name) {
            member.to_string()
        } else {
            format!("{member}/{manifest_name}")
        };
        ValidatedGlob::from_str(&pattern).ok()
    }

    fn nested_table<'a>(table: &'a Table, path: &[&str]) -> Option<&'a Table> {
        let mut cursor = table;
        for segment in path {
            cursor = cursor.get(*segment)?.as_table()?;
        }
        Some(cursor)
    }

    fn nested_array_strings(table: &Table, path: &[&str]) -> Vec<String> {
        let Some(array) = Self::nested_table(table, &path[..path.len().saturating_sub(1)])
            .and_then(|parent| parent.get(path[path.len() - 1]))
            .and_then(Value::as_array)
        else {
            return Vec::new();
        };

        array
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect()
    }

    fn manifest_paths_from_members(
        repo_root: &AbsoluteSystemPath,
        include_members: &[String],
        exclude_members: &[String],
        manifest_name: &str,
    ) -> Vec<AbsoluteSystemPathBuf> {
        let include = include_members
            .iter()
            .filter_map(|member| Self::include_manifest_glob(member, manifest_name))
            .collect::<Vec<_>>();
        if include.is_empty() {
            return Vec::new();
        }
        let exclude = exclude_members
            .iter()
            .filter_map(|member| Self::include_manifest_glob(member, manifest_name))
            .collect::<Vec<_>>();

        match globwalk::globwalk(repo_root, &include, &exclude, WalkType::Files) {
            Ok(paths) => paths.into_iter().collect(),
            Err(err) => {
                tracing::warn!("failed to evaluate workspace members for {manifest_name}: {err}");
                Vec::new()
            }
        }
    }

    fn synthesize_package_jsons_from_provider_manifests(
        repo_root: &AbsoluteSystemPath,
        workspace_providers: &[WorkspaceProviderId],
        root_package_json: &PackageJson,
    ) -> HashMap<AbsoluteSystemPathBuf, PackageJson> {
        let mut package_jsons = HashMap::new();

        if workspace_providers.contains(&WorkspaceProviderId::Node) {
            if let Ok(package_manager) =
                PackageManager::read_or_detect_package_manager(root_package_json, repo_root)
            {
                if let Ok(paths) = package_manager.get_package_jsons(repo_root) {
                    for path in paths {
                        if let Ok(package_json) = PackageJson::load(&path) {
                            package_jsons.insert(path, package_json);
                        }
                    }
                }
            }
        }

        if workspace_providers.contains(&WorkspaceProviderId::Cargo) {
            package_jsons.extend(Self::cargo_workspace_package_jsons(repo_root));
        }

        if workspace_providers.contains(&WorkspaceProviderId::Uv) {
            package_jsons.extend(Self::uv_workspace_package_jsons(repo_root));
        }

        package_jsons
    }

    fn cargo_workspace_package_jsons(
        repo_root: &AbsoluteSystemPath,
    ) -> HashMap<AbsoluteSystemPathBuf, PackageJson> {
        let cargo_manifest = repo_root.join_component("Cargo.toml");
        if !cargo_manifest.exists() {
            return HashMap::new();
        }

        let Ok(cargo_manifest_contents) = cargo_manifest.read_to_string() else {
            return HashMap::new();
        };
        let Ok(cargo_manifest_table) = toml::from_str::<Table>(&cargo_manifest_contents) else {
            return HashMap::new();
        };

        let include_members =
            Self::nested_array_strings(&cargo_manifest_table, &["workspace", "members"]);
        let exclude_members =
            Self::nested_array_strings(&cargo_manifest_table, &["workspace", "exclude"]);

        let mut manifests = Self::manifest_paths_from_members(
            repo_root,
            &include_members,
            &exclude_members,
            "Cargo.toml",
        );
        if cargo_manifest_table
            .get("package")
            .and_then(Value::as_table)
            .and_then(|package| package.get("name"))
            .and_then(Value::as_str)
            .is_some()
        {
            manifests.push(cargo_manifest);
        }

        manifests.sort();
        manifests.dedup();

        manifests
            .into_iter()
            .filter_map(|manifest_path| {
                let manifest_contents = manifest_path.read_to_string().ok()?;
                let package_name =
                    toml::from_str::<Table>(&manifest_contents)
                        .ok()
                        .and_then(|manifest| {
                            manifest
                                .get("package")
                                .and_then(Value::as_table)
                                .and_then(|package| package.get("name"))
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        })?;
                let dependencies = CargoWorkspaceProvider
                    .infer_internal_dependencies(&manifest_contents)
                    .into_iter()
                    .map(|dependency| (dependency, "*".to_string()))
                    .collect::<BTreeMap<_, _>>();

                Some((
                    manifest_path,
                    PackageJson {
                        name: Some(Spanned::new(package_name)),
                        dependencies: (!dependencies.is_empty()).then_some(dependencies),
                        ..Default::default()
                    },
                ))
            })
            .collect()
    }

    fn uv_workspace_package_jsons(
        repo_root: &AbsoluteSystemPath,
    ) -> HashMap<AbsoluteSystemPathBuf, PackageJson> {
        let pyproject_manifest = repo_root.join_component("pyproject.toml");
        if !pyproject_manifest.exists() {
            return HashMap::new();
        }

        let Ok(pyproject_contents) = pyproject_manifest.read_to_string() else {
            return HashMap::new();
        };
        let Ok(pyproject_table) = toml::from_str::<Table>(&pyproject_contents) else {
            return HashMap::new();
        };

        let include_members =
            Self::nested_array_strings(&pyproject_table, &["tool", "uv", "workspace", "members"]);
        let exclude_members =
            Self::nested_array_strings(&pyproject_table, &["tool", "uv", "workspace", "exclude"]);

        let mut manifests = Self::manifest_paths_from_members(
            repo_root,
            &include_members,
            &exclude_members,
            "pyproject.toml",
        );
        if pyproject_table
            .get("project")
            .and_then(Value::as_table)
            .and_then(|project| project.get("name"))
            .and_then(Value::as_str)
            .is_some()
        {
            manifests.push(pyproject_manifest);
        }

        manifests.sort();
        manifests.dedup();

        manifests
            .into_iter()
            .filter_map(|manifest_path| {
                let manifest_contents = manifest_path.read_to_string().ok()?;
                let package_name =
                    toml::from_str::<Table>(&manifest_contents)
                        .ok()
                        .and_then(|manifest| {
                            manifest
                                .get("project")
                                .and_then(Value::as_table)
                                .and_then(|project| project.get("name"))
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        })?;
                let dependencies = UvWorkspaceProvider
                    .infer_internal_dependencies(&manifest_contents)
                    .into_iter()
                    .map(|dependency| (dependency, "*".to_string()))
                    .collect::<BTreeMap<_, _>>();

                Some((
                    manifest_path,
                    PackageJson {
                        name: Some(Spanned::new(package_name)),
                        dependencies: (!dependencies.is_empty()).then_some(dependencies),
                        ..Default::default()
                    },
                ))
            })
            .collect()
    }

    /// Resolve the set of packages that should participate in this run.
    ///
    /// Starts with the result of scope resolution (which handles `--filter`
    /// and `--affected`), then layers on root-task inclusion:
    ///
    /// - **No filter** (`AllPackages`): root tasks defined in `turbo.json` are
    ///   included automatically.
    /// - **Exclude-only** (`ExcludeOnly`): semantically "all packages minus
    ///   excluded ones" — root tasks are still included unless the root package
    ///   itself was explicitly excluded (e.g. `--filter=!//`).
    /// - **Explicit selection** (`ExplicitSelection`): the user opted into
    ///   specific packages — root tasks are not auto-injected.
    ///
    /// When `AllPackages` is active and every requested task uses
    /// `package#task` syntax, the set is narrowed to only the referenced
    /// packages.
    pub fn calculate_filtered_packages(
        repo_root: &AbsoluteSystemPath,
        opts: &Opts,
        pkg_dep_graph: &PackageGraph,
        scm: &SCM,
        root_turbo_json: &TurboJson,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, Error> {
        let (mut filtered_pkgs, filter_mode) = scope::resolve_packages(
            &opts.scope_opts,
            repo_root,
            pkg_dep_graph,
            scm,
            root_turbo_json,
        )?;

        let should_include_root_tasks = match filter_mode {
            scope::FilterMode::AllPackages => true,
            scope::FilterMode::ExcludeOnly { root_excluded } => !root_excluded,
            scope::FilterMode::ExplicitSelection => false,
        };

        if should_include_root_tasks {
            for target in opts.run_opts.tasks.iter() {
                let mut task_name = TaskName::from(target.as_str());
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

        if matches!(filter_mode, scope::FilterMode::AllPackages) {
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
        }

        // Packages referenced by `pkg#task` CLI args are direct task graph
        // entry points regardless of --filter. Add them to filtered_pkgs so
        // the engine builder iterates their workspace.
        for task_str in &opts.run_opts.tasks {
            let task_name = TaskName::from(task_str.as_str());
            if let Some(pkg) = task_name.package() {
                filtered_pkgs.entry(PackageName::from(pkg)).or_insert(
                    PackageInclusionReason::IncludedByFilter {
                        filters: vec![task_str.clone()],
                    },
                );
            }
        }

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
        let root_turbo_json_path = self.opts.repo_opts.root_turbo_json_path.clone();

        let workspace_provider_registry = WorkspaceProviderRegistry::default();
        let configured_workspace_providers = Self::read_workspace_providers_from_root_config(
            &self.repo_root,
            &root_turbo_json_path,
        )?;
        let workspace_providers =
            workspace_provider_registry.resolve(configured_workspace_providers.as_deref())?;
        let root_package_json =
            Self::load_root_package_json(&self.repo_root, &workspace_providers)?;

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
        run_telemetry.track_arg_usage(
            "workspace_providers",
            configured_workspace_providers
                .as_ref()
                .is_some_and(|providers| !providers.is_empty()),
        );
        // we only track the remote cache if we're linked because this defaults to
        // Vercel
        if is_linked {
            run_telemetry.track_remote_cache(&self.opts.api_client_opts.api_url);
        }
        let is_single_package = self.opts.run_opts.single_package;
        repo_telemetry.track_type(if is_single_package {
            RepoType::SinglePackage
        } else {
            RepoType::Monorepo
        });

        run_telemetry.track_ci(turborepo_ci::Vendor::get_name());
        run_telemetry.track_ai_agent(turborepo_ai_agents::get_agent());
        tracing::debug!(
            workspace_providers = ?workspace_providers
                .iter()
                .map(|provider| provider.to_string())
                .collect::<Vec<_>>(),
            "resolved workspace providers for run"
        );

        // The daemon is no longer used for `turbo run`. It provided no measurable
        // performance benefit and added IPC overhead. The daemon is still used by
        // `turbo watch` which connects independently.
        run_telemetry.track_daemon_init(DaemonInitStatus::Disabled);

        if self.should_initialize_http_client() {
            self.http_client.activate();
        }

        let mut pkg_dep_graph = {
            let has_non_node_provider = workspace_providers
                .iter()
                .any(|provider| *provider != WorkspaceProviderId::Node);
            let mut builder = PackageGraph::builder(&self.repo_root, root_package_json.clone())
                .with_single_package_mode(self.opts.run_opts.single_package)
                .with_allow_no_package_manager(self.opts.repo_opts.allow_no_package_manager);
            if has_non_node_provider {
                let provider_workspace_package_jsons =
                    Self::synthesize_package_jsons_from_provider_manifests(
                        &self.repo_root,
                        &workspace_providers,
                        &root_package_json,
                    );
                builder = builder
                    .with_package_manager(Self::package_manager_for_provider_graph(
                        &self.repo_root,
                        &root_package_json,
                        &workspace_providers,
                    ))
                    .with_allow_no_package_manager(true)
                    .with_package_jsons(Some(provider_workspace_package_jsons));
            }

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

        // Build the repo index by reading .git/index directly via gix-index
        // (fast, in-process, no subprocess spawns) then running a scoped
        // parallel walk for untracked file discovery. This replaces the
        // subprocess approach (ls-tree + diff-index + ls-files race) which
        // burned ~500ms of CPU on background threads.
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
                let _span = tracing::info_span!("build_repo_index_gix").entered();
                let mut index = scm.build_tracked_repo_index_eager()?;
                if let Err(e) = scm.populate_repo_index_untracked(&mut index, &all_prefixes) {
                    tracing::debug!("failed to populate untracked files: {e}");
                }
                Some(index)
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

        let preflight_handle = if self.opts.remote_cache_disabled_reason.is_none() {
            if let (Some(client), Some(auth)) = (api_client.clone(), self.api_auth.as_ref()) {
                let token = auth.token.clone();
                let team_id = auth.team_id.clone();
                let team_slug = auth.team_slug.clone();
                Some(tokio::spawn(async move {
                    client
                        .get_caching_status(&token, team_id.as_deref(), team_slug.as_deref())
                        .await
                }))
            } else {
                None
            }
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

        let scm_state = LazyScmState::new();
        {
            let scm_state = scm_state.clone();
            let scm = scm.clone();
            let repo_root = self.repo_root.clone();
            tokio::task::spawn_blocking(move || {
                let _span = tracing::info_span!("capture_scm_state").entered();
                let sha = scm.get_current_sha(&repo_root).ok();
                let dirty_hash = scm.get_dirty_hash();
                let state = if sha.is_some() || dirty_hash.is_some() {
                    Some(CacheScmState { sha, dirty_hash })
                } else {
                    None
                };
                scm_state.resolve(state);
            });
        }

        let async_cache = {
            let _span = tracing::info_span!("async_cache_new").entered();
            AsyncCache::new(
                &self.opts.cache_opts,
                &self.repo_root,
                api_client.clone(),
                self.api_auth.clone(),
                analytics_sender,
                scm_state,
            )?
        };

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
        crate::rayon_compat::block_in_place(|| {
            let _span = tracing::info_span!("turbo_json_preload").entered();
            turbo_json_loader.preload_all();
        });

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

        // When filterUsingTasks is active, --affected is handled by the
        // same task-level filter rather than a separate codepath.
        let use_task_level_filter = self.opts.future_flags.filter_using_tasks
            && (!self.opts.scope_opts.filter_patterns.is_empty()
                || self.opts.scope_opts.affected_range.is_some());

        let use_task_level_affected = !use_task_level_filter
            && self.opts.scope_opts.affected_range.is_some()
            && self.opts.future_flags.affected_using_task_inputs;

        let needs_all_packages = use_task_level_affected || use_task_level_filter;

        // When task-level filtering is active, the engine must contain tasks
        // for ALL packages so that $TURBO_ROOT$ inputs in packages not flagged
        // by the package-level scope resolution are still matched.
        // The task-level filter (below) does the pruning.
        let all_pkgs: Vec<PackageName> = if needs_all_packages {
            pkg_dep_graph
                .packages()
                .map(|(name, _)| name.clone())
                .collect()
        } else {
            Vec::new()
        };
        let engine_pkgs: Box<dyn Iterator<Item = &PackageName>> = if needs_all_packages {
            Box::new(all_pkgs.iter())
        } else {
            Box::new(filtered_pkgs.keys())
        };

        let mut engine = self.build_engine(
            &pkg_dep_graph,
            &root_turbo_json,
            engine_pkgs,
            &turbo_json_loader,
        )?;

        let task_access = {
            let _span = tracing::info_span!("task_access_setup").entered();
            let ta = TaskAccess::new(self.repo_root.clone(), async_cache.clone(), &scm);
            ta.restore_config().await;
            ta
        };

        // --parallel removes inter-package dependencies from the package graph,
        // requiring a fresh engine build. Affected filtering runs once afterward
        // rather than on both engines to avoid a redundant SCM query.
        if self.opts.run_opts.parallel {
            pkg_dep_graph.remove_package_dependencies();
            let engine_pkgs: Box<dyn Iterator<Item = &PackageName>> = if needs_all_packages {
                Box::new(all_pkgs.iter())
            } else {
                Box::new(filtered_pkgs.keys())
            };
            engine = self.build_engine(
                &pkg_dep_graph,
                &root_turbo_json,
                engine_pkgs,
                &turbo_json_loader,
            )?;
        }

        // Task-level filter: resolve --filter and/or --affected against the task graph.
        if use_task_level_filter {
            let mut selectors: Vec<turborepo_scope::TargetSelector> = self
                .opts
                .scope_opts
                .filter_patterns
                .iter()
                .map(|p| p.parse())
                .collect::<Result<_, _>>()
                .map_err(turborepo_scope::ResolutionError::from)?;

            // When --affected is used alongside filterUsingTasks, synthesize a
            // selector equivalent to `...[base...HEAD]` so it flows through
            // the same task-level filter instead of a separate codepath.
            if let Some((from_ref, to_ref)) = &self.opts.scope_opts.affected_range {
                selectors.push(turborepo_scope::TargetSelector {
                    git_range: Some(turborepo_scope::GitRange {
                        from_ref: from_ref.clone(),
                        to_ref: to_ref.clone(),
                        include_uncommitted: true,
                        allow_unknown_objects: true,
                        merge_base: true,
                    }),
                    include_dependents: true,
                    ..Default::default()
                });
            }

            engine = super::task_filter::filter_engine_to_tasks(
                engine,
                &selectors,
                &pkg_dep_graph,
                &scm,
                &self.repo_root,
                &root_turbo_json.global_deps,
            )?;
        }

        // Task-level --affected detection (separate from --filter).
        if use_task_level_affected {
            engine = self.filter_engine_to_affected_tasks(
                engine,
                &pkg_dep_graph,
                &root_turbo_json,
                &scm,
            )?;
        }

        let remote_cache_status = self.resolve_remote_cache_status(preflight_handle).await;

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
                remote_cache_status,
                micro_frontend_configs,
                repo_index,
                observability_handle,
                query_server: self.query_server,
            },
            analytics_handle,
        ))
    }

    /// Returns a new engine containing only tasks whose declared `inputs`
    /// globs match the changed files (plus their transitive dependents).
    /// Called after the engine is built via the normal scope resolution path.
    ///
    /// `scm.changed_files()` returns a `Result<Result<..>>`: the outer error
    /// is an SCM communication failure (propagated via `?`), the inner error
    /// means the change set couldn't be computed (e.g. invalid ref, shallow
    /// clone). In the inner-error case, filtering is skipped and all tasks
    /// run — a fail-open to prevent `--affected` from silently dropping tasks
    /// when git state is ambiguous.
    #[tracing::instrument(skip_all)]
    fn filter_engine_to_affected_tasks(
        &self,
        engine: Engine,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        scm: &SCM,
    ) -> Result<Engine, Error> {
        let (from_ref, to_ref) = self
            .opts
            .scope_opts
            .affected_range
            .as_ref()
            .expect("caller verified affected_range is Some");
        let maybe_changed_files = scm.changed_files(
            &self.repo_root,
            from_ref.as_deref(),
            to_ref.as_deref(),
            true,
            true,
            true,
        )?;

        match maybe_changed_files {
            Ok(changed_files) => {
                let total_tasks = engine.task_ids().count();
                let affected_tasks = crate::task_change_detector::affected_task_ids(
                    &engine,
                    pkg_dep_graph,
                    &changed_files,
                    &root_turbo_json.global_deps,
                );
                tracing::info!(
                    total_tasks,
                    affected_tasks = affected_tasks.len(),
                    changed_files = changed_files.len(),
                    "task-level affected detection complete"
                );
                Ok(engine.retain_affected_tasks(&affected_tasks))
            }
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "SCM returned invalid change set; skipping task-level filtering"
                );
                turborepo_log::warn(
                    turborepo_log::Source::turbo(turborepo_log::Subsystem::Scm),
                    "--affected could not determine changed files. All tasks will run. Check your \
                     git fetch depth.",
                )
                .field("error", format!("{e:?}"))
                .emit();
                Ok(engine)
            }
        }
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
        // Inverse of global_deps_for_hash: when globalConfiguration is on,
        // global deps are embedded in per-task inputs via prepend_global_inputs.
        let global_deps_for_task_inputs = if self.opts.future_flags.global_configuration {
            root_turbo_json.global_deps.clone()
        } else {
            Vec::new()
        };
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
        .with_global_deps(global_deps_for_task_inputs)
        .with_tasks(tasks);

        if self.add_all_tasks {
            builder = builder.add_all_tasks();
        }

        if !self.should_validate_engine {
            builder = builder.do_not_validate_engine();
        }

        let mut engine = builder.build()?;

        // In watch mode with the future flag, filter the engine to only tasks
        // whose declared inputs match the changed files.
        //
        // When active, this REPLACES create_engine_for_subgraph because:
        // 1. retain_affected_tasks already selects the correct tasks + dependents
        // 2. The entrypoint packages (from file watcher events) may not overlap with
        //    the affected tasks (e.g. a $TURBO_ROOT$ input in another package)
        // 3. retain_affected_tasks requires the Root sentinel node, which
        //    create_engine_for_subgraph removes
        let watch_task_filtered = if let Some(ref changed_files) = self.changed_files_for_watch {
            if self.opts.future_flags.watch_using_task_inputs && !changed_files.is_empty() {
                // Only consider files that still exist on disk. Editor temp
                // files (vim 4913, *~ backups, etc.) are created and deleted
                // within the same watcher batch. The hash algorithm only sees
                // files that exist, so input matching should too.
                let existing_files: std::collections::HashSet<_> = changed_files
                    .iter()
                    .filter(|f| self.repo_root.resolve(f).exists())
                    .cloned()
                    .collect();

                let total_tasks = engine.task_ids().count();
                let affected_tasks = crate::task_change_detector::affected_task_ids(
                    &engine,
                    pkg_dep_graph,
                    &existing_files,
                    &root_turbo_json.global_deps,
                );
                tracing::info!(
                    total_tasks,
                    affected_tasks = affected_tasks.len(),
                    changed_files = existing_files.len(),
                    "watch task-level input filtering complete"
                );
                engine = engine.retain_affected_tasks(&affected_tasks);
                true
            } else {
                false
            }
        } else {
            false
        };

        // If we have an initial task, we prune out the engine to only
        // tasks that are reachable from that initial task.
        if !watch_task_filtered {
            if let Some(entrypoint_packages) = &self.entrypoint_packages {
                engine = engine.create_engine_for_subgraph(entrypoint_packages);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_workspace_package_jsons_infer_names_and_internal_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        repo_root
            .join_component("Cargo.toml")
            .create_with_contents(
                r#"[workspace]
members = ["crates/*"]
"#,
            )
            .unwrap();
        let a_manifest = repo_root.join_components(&["crates", "a", "Cargo.toml"]);
        a_manifest.ensure_dir().unwrap();
        a_manifest
            .create_with_contents(
                r#"[package]
name = "a"
version = "0.1.0"

[dependencies]
b = { path = "../b" }
"#,
            )
            .unwrap();
        let b_manifest = repo_root.join_components(&["crates", "b", "Cargo.toml"]);
        b_manifest.ensure_dir().unwrap();
        b_manifest
            .create_with_contents(
                r#"[package]
name = "b"
version = "0.1.0"
"#,
            )
            .unwrap();

        let package_jsons = RunBuilder::cargo_workspace_package_jsons(&repo_root);
        assert_eq!(package_jsons.len(), 2);

        let a_package = package_jsons.get(&a_manifest).unwrap();
        assert_eq!(a_package.name.as_ref().unwrap().as_inner(), "a");
        assert_eq!(
            a_package
                .dependencies
                .as_ref()
                .and_then(|deps| deps.get("b"))
                .map(String::as_str),
            Some("*")
        );
    }

    #[test]
    fn uv_workspace_package_jsons_infer_names_and_internal_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        repo_root
            .join_component("pyproject.toml")
            .create_with_contents(
                r#"[tool.uv.workspace]
members = ["packages/*"]
"#,
            )
            .unwrap();
        let api_manifest = repo_root.join_components(&["packages", "api", "pyproject.toml"]);
        api_manifest.ensure_dir().unwrap();
        api_manifest
            .create_with_contents(
                r#"[project]
name = "api"
version = "0.1.0"

[tool.uv.sources]
core = { path = "../core" }
"#,
            )
            .unwrap();
        let core_manifest = repo_root.join_components(&["packages", "core", "pyproject.toml"]);
        core_manifest.ensure_dir().unwrap();
        core_manifest
            .create_with_contents(
                r#"[project]
name = "core"
version = "0.1.0"
"#,
            )
            .unwrap();

        let package_jsons = RunBuilder::uv_workspace_package_jsons(&repo_root);
        assert_eq!(package_jsons.len(), 2);

        let api_package = package_jsons.get(&api_manifest).unwrap();
        assert_eq!(api_package.name.as_ref().unwrap().as_inner(), "api");
        assert_eq!(
            api_package
                .dependencies
                .as_ref()
                .and_then(|deps| deps.get("core"))
                .map(String::as_str),
            Some("*")
        );
    }
}
