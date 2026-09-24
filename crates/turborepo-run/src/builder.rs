use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io::{ErrorKind, IsTerminal},
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::Local;
use tracing::Instrument;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
    RelativeUnixPathBuf,
};
use turborepo_analytics::{AnalyticsHandle, start_analytics};
use turborepo_api_client::{APIAuth, APIClient, CacheClient, SharedHttpClient};
use turborepo_cache::{AsyncCache, CacheScmState, LazyScmState};
use turborepo_engine::{Built, Engine, EngineBuilder, task_has_command, task_participates};
use turborepo_env::EnvironmentVariableMap;
use turborepo_errors::Spanned;
use turborepo_process::ProcessManager;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    discovery::{CachingPackageDiscovery, LocalPackageDiscovery},
    package_graph::{LazyPlan, PackageGraph, PackageName, TaskEntrypointPreference},
    package_json,
    toolchain::ToolchainId,
};
use turborepo_run_context::RepoContext;
use turborepo_run_opts::{Opts, RemoteCacheDisabledReason};
use turborepo_run_summary::observability;
use turborepo_scm::SCM;
use turborepo_scope::{
    ChangedFilesDetector, GitChangeDetector, TargetSelector, filter::ResolutionError,
};
use turborepo_shim::TurboState;
use turborepo_signals::SignalHandler;
use turborepo_task_id::{TaskId, TaskName};
use turborepo_telemetry::events::{
    EventBuilder, TrackedErrors,
    command::CommandEventBuilder,
    generic::{DaemonInitStatus, GenericEventBuilder},
    repo::{RepoEventBuilder, RepoType},
};
use turborepo_types::{
    FilterMode, SecretString, TaskDefinition, TaskDefinitionHashInfo, TaskInputs, UIMode,
};
use turborepo_ui::ColorConfig;
use turborepo_vercel_api::CachingStatusResponse;
use url::Url;

type RunEngine = Engine<Built, TaskDefinition>;
type FilteredPackages = (
    HashMap<PackageName, PackageInclusionReason>,
    FilterMode,
    HashSet<PackageName>,
);

#[derive(Default)]
struct TaskEntrypointSelection {
    candidates: HashSet<TaskId<'static>>,
    selected: HashSet<TaskId<'static>>,
    excluded: HashSet<TaskId<'static>>,
    orchestration: HashMap<String, HashSet<TaskId<'static>>>,
}

struct RepoDiscovery {
    run_telemetry: GenericEventBuilder,
    is_single_package: bool,
    root_package_json: Option<package_json::PackageJson>,
    pkg_dep_graph: Arc<PackageGraph>,
    lazy_plan: Option<LazyPlan<CachingPackageDiscovery<LocalPackageDiscovery>>>,
    scm: SCM,
    micro_frontend_configs: Option<MicrofrontendsConfigs>,
    repo_index: PendingRepoIndex,
    untracked_scan_scope_tx: Option<tokio::sync::oneshot::Sender<Option<Vec<RelativeUnixPathBuf>>>>,
}

/// Inputs that join package-graph discovery, SCM detection, and turbo.json
/// loading into the repository context consumed by the rest of a run. Keeping
/// the graph as a `PackageGraph` lets callers provide JavaScript and native
/// contributor scopes without coupling this phase to a particular discovery
/// implementation.
struct RepoContextInput {
    repo_root: AbsoluteSystemPathBuf,
    color_config: ColorConfig,
    version: &'static str,
    scm: SCM,
    pkg_dep_graph: Arc<PackageGraph>,
    turbo_json_loader: UnifiedTurboJsonLoader,
    root_turbo_json: TurboJson,
}

fn build_repo_context(input: RepoContextInput) -> Arc<RepoContext> {
    Arc::new(RepoContext {
        repo_root: input.repo_root,
        color_config: input.color_config,
        version: input.version,
        scm: input.scm,
        pkg_dep_graph: input.pkg_dep_graph,
        turbo_json_loader: input.turbo_json_loader,
        root_turbo_json: input.root_turbo_json,
    })
}

struct ExecutionContextInput<'a> {
    root_package_json: Option<package_json::PackageJson>,
    is_single_package: bool,
    pkg_dep_graph: Arc<PackageGraph>,
    lazy_plan: Option<LazyPlan<CachingPackageDiscovery<LocalPackageDiscovery>>>,
    scm: &'a SCM,
    micro_frontend_configs: Option<MicrofrontendsConfigs>,
    untracked_scan_scope_tx: Option<tokio::sync::oneshot::Sender<Option<Vec<RelativeUnixPathBuf>>>>,
    async_cache: AsyncCache,
}

struct BuiltExecutionContext {
    pkg_dep_graph: Arc<PackageGraph>,
    turbo_json_loader: UnifiedTurboJsonLoader,
    root_turbo_json: TurboJson,
    task_access: TaskAccess,
    env_at_execution_start: EnvironmentVariableMap,
    filtered_pkgs: HashSet<PackageName>,
    engine: Arc<RunEngine>,
    micro_frontend_configs: Option<MicrofrontendsConfigs>,
}

struct EngineSettlementInput<'a> {
    pkg_dep_graph: Arc<PackageGraph>,
    lazy_plan: Option<LazyPlan<CachingPackageDiscovery<LocalPackageDiscovery>>>,
    turbo_json_loader: &'a UnifiedTurboJsonLoader,
    root_turbo_json: &'a TurboJson,
    env_at_execution_start: &'a EnvironmentVariableMap,
    package_resolution_opts: &'a Opts,
    scm: &'a SCM,
    use_task_level_affected: bool,
    has_task_level_affected_package_scope: bool,
    needs_all_packages: bool,
}

struct SettledEngine {
    pkg_dep_graph: Arc<PackageGraph>,
    engine: RunEngine,
    filtered_pkgs: HashMap<PackageName, PackageInclusionReason>,
    filter_mode: FilterMode,
    unqualified_entrypoint_packages: HashSet<PackageName>,
    all_pkgs: Vec<PackageName>,
    task_level_affected_package_scope: Option<HashSet<PackageName>>,
}

struct PassEngineContext<'a> {
    root_turbo_json: &'a TurboJson,
    engine_loader: &'a EngineTurboJsonLoader<'a>,
    env_at_execution_start: &'a EnvironmentVariableMap,
    needs_all_packages: bool,
}

struct SelectionModes {
    use_task_level_filter: bool,
    use_task_level_affected: bool,
    needs_all_packages: bool,
}

type RemoteCachePreflight =
    tokio::task::JoinHandle<turborepo_api_client::Result<CachingStatusResponse>>;

pub(crate) fn changed_files_for_affected_range<D: ChangedFilesDetector>(
    detector: &D,
    repo_root: &AbsoluteSystemPath,
    from_ref: Option<&str>,
    to_ref: Option<&str>,
) -> Result<
    Result<HashSet<AnchoredSystemPathBuf>, turborepo_scm::git::InvalidRange>,
    turborepo_scm::Error,
> {
    detector.changed_files(repo_root, from_ref, to_ref, true, true, true)
}

trait CacheStatusProbe: Send + Sync + 'static {
    fn check_caching_status(
        &self,
        token: &SecretString,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = turborepo_api_client::Result<CachingStatusResponse>> + Send;
}

impl CacheStatusProbe for APIClient {
    fn check_caching_status(
        &self,
        token: &SecretString,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = turborepo_api_client::Result<CachingStatusResponse>> + Send {
        CacheClient::get_caching_status(self, token, team_id, team_slug)
    }
}

fn start_remote_cache_preflight(
    client: impl CacheStatusProbe,
    token: SecretString,
    team_id: Option<String>,
    team_slug: Option<String>,
) -> RemoteCachePreflight {
    tokio::spawn(
        async move {
            client
                .check_caching_status(&token, team_id.as_deref(), team_slug.as_deref())
                .await
        }
        .instrument(tracing::info_span!("remote_cache_preflight")),
    )
}

#[tracing::instrument(skip_all)]
async fn resolve_remote_cache_status(
    remote_cache_disabled_reason: Option<RemoteCacheDisabledReason>,
    preflight_handle: Option<RemoteCachePreflight>,
) -> RemoteCacheStatus {
    use turborepo_vercel_api::CachingStatus;

    if let Some(reason) = remote_cache_disabled_reason {
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
        Ok(Ok(Err(api_err))) => map_api_error_to_status(api_err),
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
                CachingStatus::Disabled => {
                    RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::DisabledForTeam)
                }
                CachingStatus::OverLimit => {
                    RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::UsageLimitExceeded)
                }
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

struct RunServicesInput<'a> {
    preflight_handle: Option<RemoteCachePreflight>,
    async_cache: AsyncCache,
    scm_state: LazyScmState,
    scm_state_task: Option<tokio::task::JoinHandle<Option<String>>>,
    scm: &'a SCM,
    repo_index: &'a PendingRepoIndex,
    analytics_handle: Option<AnalyticsHandle>,
}

struct BuiltRunServices {
    run_cache: Arc<RunCache>,
    remote_cache_status: RemoteCacheStatus,
    observability_handle: Option<observability::Handle>,
    analytics_handle: Option<AnalyticsHandle>,
}

use turborepo_microfrontends_config::{MicrofrontendsConfigs, UnifiedTurboJsonLoader};
use turborepo_package_watcher::repository_graph::RepositoryGraphFeatures;
use turborepo_task_access::TaskAccess;
use turborepo_turbo_json::{TurboJson, TurboJsonReader};

use crate::{
    Error, ExecutionContext, PendingRepoIndex, RemoteCacheStatus, RemoteCacheUnavailableReason,
    Run, RunBuilderInput, RunCache, RunServices, engine_loader::EngineTurboJsonLoader, scope,
};

fn project_task_io_environment(
    patterns: std::collections::BTreeMap<
        turborepo_repository::task_contracts::TaskEnvironmentDomain,
        Vec<&'static str>,
    >,
    environment: &EnvironmentVariableMap,
) -> Result<
    HashMap<
        turborepo_repository::task_contracts::TaskEnvironmentDomain,
        turborepo_repository::toolchain::TaskIOEnvironment,
    >,
    turborepo_env::Error,
> {
    patterns
        .into_iter()
        .map(|(domain, patterns)| {
            let selected = environment.from_wildcards(&patterns)?;
            Ok((
                domain,
                turborepo_repository::toolchain::TaskIOEnvironment::new(selected.into_inner()),
            ))
        })
        .collect()
}

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
    // Package listing needs SCM queries for affected filters, but never hashes files or uses
    // cache provenance. Skip the repository-wide work that only serves those consumers.
    skip_repo_index_and_scm_state: bool,
    skip_external_dependencies: bool,
    // In watch mode, partial reruns may reuse the package graph built by the
    // previous full run instead of rediscovering the workspace, re-reading
    // every manifest, and re-parsing the lockfile. Only sound when the caller
    // has proven that no graph-defining file (workspace manifests, lockfile,
    // workspace configuration) changed since the graph was built; the watch
    // client checks the changed-file set before sharing it.
    shared_pkg_graph: Option<Arc<PackageGraph>>,
}

impl RunBuilder {
    #[tracing::instrument(skip_all)]
    pub fn new(
        input: RunBuilderInput,
        http_client: Option<SharedHttpClient>,
    ) -> Result<Self, Error> {
        let http_client = http_client.unwrap_or_default();
        let processes = ProcessManager::new(
            // We currently only use a pty if the following are met:
            // - we're attached to a tty
            std::io::stdout().is_terminal() &&
            // - if we're on windows, we're using the UI
            (!cfg!(windows) || matches!(input.opts.run_opts.ui_mode, UIMode::Tui)),
        );

        let RunBuilderInput {
            repo_root,
            color_config: ui,
            opts,
            version,
            api_auth,
        } = input;

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
            skip_repo_index_and_scm_state: false,
            skip_external_dependencies: false,
            shared_pkg_graph: None,
        })
    }

    pub fn skip_repo_index_and_scm_state(mut self) -> Self {
        self.skip_repo_index_and_scm_state = true;
        self
    }

    pub fn skip_external_dependencies(mut self) -> Self {
        self.skip_external_dependencies = true;
        self
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

    /// Reuse a package graph from an earlier run instead of building a fresh
    /// one from disk. Only sound when workspace manifests, the lockfile, and
    /// workspace configuration are unchanged; used by watch-mode partial
    /// reruns where the watcher proves that from the changed-file set.
    /// Ignored for `--parallel`, which mutates the graph after construction.
    pub fn with_shared_package_graph(mut self, graph: Arc<PackageGraph>) -> Self {
        self.shared_pkg_graph = Some(graph);
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

    /// The scan prefix for untracked-file discovery: the repo root anchored
    /// to the git root. When the repo is nested inside a larger git
    /// repository this restricts the scan to the repo's subtree; when the
    /// two coincide it is empty and the scan walks the whole repository.
    /// Every package lives under the repo root (enforced at discovery), so
    /// this single prefix covers exactly what per-package prefixes used to.
    fn repo_prefix_for_repo_index(
        repo_root: &AbsoluteSystemPath,
        index_root: &AbsoluteSystemPath,
    ) -> Result<RelativeUnixPathBuf, turbopath::PathError> {
        Ok(index_root.anchor(repo_root)?.to_unix())
    }

    /// Whether this run might scope its untracked-file discovery to the
    /// selected packages' directories.
    ///
    /// Decided from options alone, before any graph or engine work, so runs
    /// that can never scope keep today's eager whole-repo scan without
    /// waiting on a decision. Scoping requires a narrow explicit package
    /// selection: include `--filter` patterns without `--affected`, no
    /// package inference, no watch-mode changed files, not single-package
    /// mode, no task-level filtering (which builds the engine across every
    /// package before pruning), and not `--add-all-tasks`. Exclude-only
    /// filters select (nearly) every package and cannot scope either.
    fn untracked_scoping_candidate(&self) -> bool {
        !self.opts.run_opts.single_package
            && self
                .opts
                .scope_opts
                .filter_patterns
                .iter()
                .any(|pattern| !pattern.starts_with('!'))
            && self.opts.scope_opts.affected_range.is_none()
            && self.opts.scope_opts.pkg_inference_root.is_none()
            && self
                .changed_files_for_watch
                .as_ref()
                .is_none_or(|files| files.is_empty())
            && !self.add_all_tasks
            && !self.opts.future_flags.filter_using_tasks
    }

    /// Directory prefixes, relative to the git root, that cover every file
    /// input this run will hash; `None` when the run is not provably
    /// package-scoped and untracked discovery must walk the whole repo.
    ///
    /// The run hashes files through the repo index for each participating
    /// task's package directory (tasks without `inputs`, and
    /// `$TURBO_DEFAULT$`, hash everything under the package) and for the
    /// root package's internal dependencies, which fold into the global
    /// hash that every task hash includes.
    ///
    /// Scoping is refused whenever a hashed input can reach outside those
    /// directories: root tasks hash relative to the repo root,
    /// `globalDependencies` reach the whole repo, and `$TURBO_ROOT$` or
    /// `..`-relative input globs (the engine stores `$TURBO_ROOT$`
    /// references in rewritten `../` form) escape their package.
    fn untracked_scan_prefixes(
        repo_root: &AbsoluteSystemPath,
        git_root: Option<&AbsoluteSystemPath>,
        engine: &RunEngine,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        filter_mode: &FilterMode,
    ) -> Option<Vec<RelativeUnixPathBuf>> {
        // Only an explicit include selection is narrow. Unfiltered and
        // exclude-only runs select (nearly) every package and would gain
        // nothing from scoping.
        if filter_mode != &FilterMode::ExplicitSelection {
            return None;
        }
        if !root_turbo_json.global_deps_for_hash().is_empty() {
            return None;
        }
        // A manual SCM has no git root to anchor prefixes against (and its
        // untracked population is a no-op); keep the whole-repo decision.
        let git_root = git_root?;

        let mut package_dirs: Vec<&AnchoredSystemPath> = Vec::new();
        let mut seen_packages: HashSet<PackageName> = HashSet::new();
        for task_id in engine.task_ids() {
            let package = PackageName::from(task_id.package());
            // Root tasks hash files relative to the repo root, which is not
            // a package subtree.
            if package == PackageName::Root {
                return None;
            }
            if !seen_packages.insert(package.clone()) {
                continue;
            }
            let definition = engine.task_definitions().get(task_id)?;
            if !task_inputs_are_package_local(definition.inputs()) {
                return None;
            }
            let context = pkg_dep_graph.package_task_context(&package)?;
            package_dirs.push(context.directory());
        }
        // The root package's internal dependencies are hashed into the
        // global hash for every monorepo run; cover their directories even
        // when none of their tasks participate.
        package_dirs.extend(pkg_dep_graph.root_internal_package_dependencies_paths());

        let mut prefixes = Vec::with_capacity(package_dirs.len());
        for dir in package_dirs {
            let prefix = git_root.anchor(&repo_root.resolve(dir)).ok()?.to_unix();
            // An empty prefix is the git root itself, not a package subtree.
            if prefix.as_str().is_empty() {
                return None;
            }
            prefixes.push(prefix);
        }
        prefixes.sort_unstable();
        prefixes.dedup();
        // `UntrackedScope` reads an empty prefix list as a full walk; an
        // empty selection has nothing to hash, but keep the whole-repo scan
        // so degenerate runs match non-scoped behavior exactly.
        if prefixes.is_empty() {
            return None;
        }
        Some(prefixes)
    }

    /// Parse the run's filter patterns for the lazy-loading decision.
    ///
    /// `None` means a pattern did not parse: the run then conservatively
    /// requires a complete graph (scope resolution surfaces the actual parse
    /// error either way, so nothing is bypassed).
    fn parse_filter_selectors(patterns: &[String]) -> Option<Vec<TargetSelector>> {
        patterns
            .iter()
            .map(|pattern| pattern.parse::<TargetSelector>())
            .collect::<Result<_, _>>()
            .ok()
    }

    /// Whether this run must narrow against a complete graph snapshot rather
    /// than an inventory graph.
    ///
    /// Graph-dependent queries — dependency (`pkg...`) or dependents
    /// (`...pkg`) expansion, match-dependencies, git ranges — and
    /// affectedness, watch reruns, and whole-graph engines (`add_all_tasks`)
    /// have answers that depend on edges and task catalogues only
    /// authoritative discovery provides, so every unloaded owner is loaded up
    /// front: the accepted conservative tradeoff of lazy native discovery.
    ///
    /// The no-turbo-json loader path is also complete-graph: it infers the
    /// repository's task set from every scope's native catalogue, which is a
    /// task-catalogue query over all scopes.
    ///
    /// Strict task entrypoint selection is likewise a whole-repo catalogue
    /// query: it asks whether any scope's catalogue participates in each
    /// requested task, and an inventory-only catalogue would silently answer
    /// no. Loading up front preserves the eager baseline exactly — a generic
    /// flag semantic, with no per-toolchain knowledge here.
    fn requires_complete_graph_snapshot(
        &self,
        selectors: Option<&[TargetSelector]>,
        micro_frontend_configs: Option<&MicrofrontendsConfigs>,
    ) -> bool {
        let Some(selectors) = selectors else {
            return true;
        };
        self.opts.scope_opts.affected_range.is_some()
            || self.changed_files_for_watch.is_some()
            || self.add_all_tasks
            || self.opts.future_flags.strict_task_entrypoint_selection
            || selectors_require_complete_graph(selectors)
            || (!self.opts.repo_opts.root_turbo_json_path.exists()
                && (self.opts.repo_opts.allow_no_turbo_json || micro_frontend_configs.is_some()))
    }

    /// Resolve the set of packages that should participate in this run.
    ///
    /// Starts with the result of scope resolution (which handles `--filter`
    /// and `--affected`), then layers on root-task inclusion:
    ///
    /// - **No filter** (`AllPackages`): root tasks defined in `turbo.json` are
    ///   included automatically.
    /// - **Exclude-only** (`ExcludeOnly`): semantically "all packages minus
    ///   excluded ones" — root tasks are still included unless the root Turbo
    ///   namespace itself was explicitly excluded (e.g. `--filter=!//` or
    ///   `--filter=!{.}`). Root-directory syntax addresses this namespace even
    ///   when no root JavaScript package exists.
    /// - **Explicit selection** (`ExplicitSelection`): the user opted into
    ///   specific packages — root tasks are not auto-injected.
    ///
    /// When `AllPackages` is active and every requested task uses
    /// `package#task` syntax, the set is narrowed to only the referenced
    /// packages.
    pub(crate) fn calculate_filtered_packages(
        repo_root: &AbsoluteSystemPath,
        opts: &Opts,
        pkg_dep_graph: &PackageGraph,
        scm: &SCM,
        root_turbo_json: &TurboJson,
    ) -> Result<FilteredPackages, Error> {
        let change_detector = scope::change_detector(
            &opts.scope_opts,
            repo_root,
            pkg_dep_graph,
            scm,
            root_turbo_json,
        )?;
        Self::calculate_filtered_packages_with_change_detector(
            repo_root,
            opts,
            pkg_dep_graph,
            change_detector,
            root_turbo_json,
        )
    }

    pub(crate) fn calculate_filtered_packages_with_change_detector<D: GitChangeDetector>(
        repo_root: &AbsoluteSystemPath,
        opts: &Opts,
        pkg_dep_graph: &PackageGraph,
        change_detector: D,
        root_turbo_json: &TurboJson,
    ) -> Result<FilteredPackages, Error> {
        let (mut filtered_pkgs, filter_mode) = scope::resolve_packages_with_change_detector(
            &opts.scope_opts,
            repo_root,
            pkg_dep_graph,
            change_detector,
        )
        .map_err(|err| match err {
            // A filter that names a Rust crate or Python package is a likely
            // mistake when the repository has a native workspace but the
            // toolchain's package support is not enabled; point at the
            // opt-in.
            ResolutionError::NoPackagesMatchedWithName(name)
                if !cargo_enabled(&opts.future_flags)
                    && repo_root
                        .join_component(turborepo_repository::cargo::CARGO_TOML)
                        .exists() =>
            {
                Error::PackageMayBeCargoCrate { name }
            }
            ResolutionError::NoPackagesMatchedWithName(name)
                if !python_enabled(&opts.future_flags)
                    && repo_root
                        .join_component(turborepo_repository::uv::PYPROJECT_TOML)
                        .exists() =>
            {
                Error::PackageMayBePythonPackage { name }
            }
            ResolutionError::NoPackagesMatchedWithName(name)
                if !go_enabled(&opts.future_flags)
                    && repo_root
                        .join_component(turborepo_repository::go::GO_WORK)
                        .exists() =>
            {
                Error::PackageMayBeGoModule { name }
            }
            err => Error::Scope(err),
        })?;

        let should_include_root_tasks = match filter_mode {
            FilterMode::AllPackages => true,
            FilterMode::ExcludeOnly { root_excluded } => !root_excluded,
            FilterMode::ExplicitSelection => false,
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

        if matches!(filter_mode, FilterMode::AllPackages) {
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

        let unqualified_entrypoint_packages = filtered_pkgs.keys().cloned().collect();

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

        Ok((filtered_pkgs, filter_mode, unqualified_entrypoint_packages))
    }

    /// Packages whose turbo.json a scoped engine may consult: the filtered
    /// packages plus the transitive package dependencies their topological
    /// `^task` edges follow. Anything else loads lazily if engine traversal
    /// ever reaches it, so this is only a pre-warm set, not a correctness
    /// boundary.
    fn scoped_preload_packages<'a>(
        pkg_dep_graph: &PackageGraph,
        filtered_pkgs: impl Iterator<Item = &'a PackageName>,
    ) -> Vec<PackageName> {
        let ordering = pkg_dep_graph.ordering_relationships();
        let mut seen: HashSet<PackageName> = filtered_pkgs.cloned().collect();
        let mut queue: Vec<PackageName> = seen.iter().cloned().collect();
        while let Some(package) = queue.pop() {
            // Unknown packages surface through engine construction, which
            // resolves the same relationships.
            let Ok(dependencies) = ordering.direct_dependencies(&package) else {
                continue;
            };
            for dependency in dependencies {
                if seen.insert(dependency.clone()) {
                    queue.push(dependency.clone());
                }
            }
        }
        seen.into_iter().collect()
    }

    async fn discover_repo(&self, telemetry: &CommandEventBuilder) -> Result<RepoDiscovery, Error> {
        // SCM detection, the tracked repo index, and untracked-file discovery
        // all run on one background task, overlapping package graph and
        // engine construction. The SCM handle is sent back as soon as it
        // exists so the main flow never waits behind the scans.
        //
        // Untracked discovery historically waited for the package graph to
        // compute package prefixes, but the scan scope was always exactly
        // the repo-root subtree: every package lives under the repo root
        // (enforced at discovery), the root package's prefix is always in
        // the set, and `UntrackedScope` deduplicates nested prefixes.
        // Scanning the repo-root prefix directly is equivalent and needs
        // nothing from the graph.
        //
        // Narrow filtered runs can do better: when every file input the run
        // will hash is provably package-local, the walk only needs the
        // selected packages' subtrees. That is only provable after the
        // engine is built, so candidate runs hold the untracked population
        // until the main flow sends its scope decision (below). Runs that
        // cannot scope based on their options alone never wait, keeping
        // today's eager whole-repo scan.
        let untracked_scoping_candidate = self.untracked_scoping_candidate();
        if !untracked_scoping_candidate {
            tracing::debug!("untracked-file scan scope: whole repo (not a scoping candidate)");
        }
        let (untracked_scan_scope_tx, untracked_scan_scope_rx) = if untracked_scoping_candidate {
            let (tx, rx) = tokio::sync::oneshot::channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let (scm_tx, scm_rx) = tokio::sync::oneshot::channel();
        let repo_index_task = {
            let repo_root = self.repo_root.clone();
            let git_root = self.opts.git_root.clone();
            let github_actions_remote_base_ref_fallback = self
                .opts
                .future_flags
                .github_actions_remote_base_ref_fallback;
            let skip_repo_index = self.skip_repo_index_and_scm_state;
            tokio::task::spawn_blocking(move || {
                let scm = match git_root {
                    Some(root) => SCM::new_with_git_root(&repo_root, root),
                    None => SCM::new(&repo_root),
                }
                .with_github_actions_remote_base_ref_fallback(
                    github_actions_remote_base_ref_fallback,
                );
                if skip_repo_index {
                    let _ = scm_tx.send(scm);
                    return None;
                }
                // The tracked half of the repo index only needs `.git/index`.
                let tracked_index = {
                    let _span = tracing::info_span!("build_tracked_repo_index_gix").entered();
                    scm.build_tracked_repo_index_eager()
                };
                let _ = scm_tx.send(scm.clone());
                let mut index = tracked_index?;
                // Candidate runs wait here for the scope decision:
                // `Some(prefixes)` walks only those subtrees (relative to
                // the git root), while `None` is today's whole-repo scan.
                // A dropped sender means the run failed before deciding;
                // the whole-repo fallback matches non-scoped runs.
                let scoped_prefixes = match untracked_scan_scope_rx {
                    Some(rx) => rx.blocking_recv().ok().flatten(),
                    None => None,
                };
                let prefixes = scoped_prefixes.unwrap_or_else(|| {
                    let index_root = scm.git_root().unwrap_or(&repo_root);
                    match Self::repo_prefix_for_repo_index(&repo_root, index_root) {
                        Ok(repo_prefix) => vec![repo_prefix],
                        Err(e) => {
                            tracing::debug!(
                                "failed to compute repo prefix for untracked files: {e}"
                            );
                            // Leave untracked entries unpopulated, as before.
                            Vec::new()
                        }
                    }
                });
                // The span covers the walk itself; how long the run had to
                // wait for the population is visible in the
                // `repo_index_untracked_await` barrier span.
                let _span = tracing::info_span!("populate_repo_index_untracked").entered();
                if let Err(e) = scm.populate_repo_index_untracked(&mut index, &prefixes) {
                    tracing::debug!("failed to populate untracked files: {e}");
                }
                Some(index)
            })
        };
        // A pure native workspace (experimentalCargoWorkspaces,
        // experimentalPythonWorkspaces, or experimentalGoWorkspaces, no root
        // package.json) has no JavaScript root manifest. A *missing* file is only
        // tolerated in those modes; a malformed one always fails, and a missing
        // one without native support keeps the original hard error.
        let graph_features = RepositoryGraphFeatures::new(&self.opts.future_flags);
        let root_package_json = graph_features.load_root_package_json(&self.repo_root)?;
        let run_telemetry = GenericEventBuilder::new().with_parent(telemetry);
        let repo_telemetry =
            RepoEventBuilder::new(&self.repo_root.to_string()).with_parent(telemetry);

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

        // --parallel removes inter-package dependencies from the graph after
        // construction, so a graph shared with other runs cannot be reused for
        // it. A shared graph that still carries inventory-only scopes has no
        // construction plan attached (snapshots never retain live toolchains),
        // so this run re-inventories instead of reusing it; only complete
        // snapshots are reusable — a generic property of the graph, with no
        // per-toolchain knowledge here.
        let shared_is_reusable = self
            .shared_pkg_graph
            .as_ref()
            .is_some_and(|graph| !graph.has_unloaded_scopes());
        let shared_pkg_graph = if self.opts.run_opts.parallel {
            None
        } else if self.shared_pkg_graph.is_some() && !shared_is_reusable {
            tracing::debug!(
                "bypassing shared package graph: inventory-only scopes require re-inventorying"
            );
            None
        } else {
            self.shared_pkg_graph.clone()
        };
        let mut lazy_plan = None;
        let pkg_dep_graph: Arc<PackageGraph> = match shared_pkg_graph {
            Some(graph) => {
                tracing::debug!("reusing package graph from previous run");
                graph
            }
            None => {
                let builder =
                    PackageGraph::builder_optional(&self.repo_root, root_package_json.clone())
                        .with_single_package_mode(self.opts.run_opts.single_package)
                        .with_allow_no_package_manager(
                            self.opts.repo_opts.allow_no_package_manager,
                        );
                let builder = if self.skip_external_dependencies {
                    builder.without_external_dependencies()
                } else {
                    builder
                };
                let builder = graph_features.configure(builder);

                let graph = builder
                    .build_lazy()
                    .instrument(tracing::info_span!("pkg_dep_graph_build"))
                    .await;

                match graph {
                    Ok(graph) => {
                        // Take unique ownership of the graph so `--parallel`
                        // can mutate it; the plan owns none of it.
                        let (graph, plan) = graph.into_parts();
                        lazy_plan = Some(plan);
                        graph
                    }
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
            }
        };

        if let Some(package_manager) = pkg_dep_graph.package_manager() {
            repo_telemetry.track_package_manager(package_manager.name().to_string());
        }
        repo_telemetry.track_size(pkg_dep_graph.len());
        run_telemetry.track_run_type(self.opts.run_opts.dry_run.is_some());

        // The SCM handle arrives as soon as detection and the tracked index
        // build finish; the untracked scan continues in the background and
        // is joined just before the first repo-index consumer.
        let scm = {
            let _span = tracing::info_span!("scm_task_await").entered();
            match scm_rx.await {
                Ok(scm) => scm,
                Err(_) => {
                    // The sender only drops without sending if the SCM task
                    // panicked; that panic surfaces when the repo-index task
                    // is joined below. Detect inline so the run reaches that
                    // point.
                    SCM::new(&self.repo_root)
                }
            }
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

        let repo_index = PendingRepoIndex::new(repo_index_task);

        Ok(RepoDiscovery {
            run_telemetry,
            is_single_package,
            root_package_json,
            pkg_dep_graph,
            lazy_plan,
            scm,
            micro_frontend_configs,
            repo_index,
            untracked_scan_scope_tx,
        })
    }

    async fn build_execution_context(
        &self,
        input: ExecutionContextInput<'_>,
    ) -> Result<BuiltExecutionContext, Error> {
        let ExecutionContextInput {
            root_package_json,
            is_single_package,
            mut pkg_dep_graph,
            mut lazy_plan,
            scm,
            micro_frontend_configs,
            untracked_scan_scope_tx,
            async_cache,
        } = input;
        // Graph-dependent queries (dependency/dependent expansion, git
        // ranges), affectedness, watch reruns, whole-graph engines, and the
        // no-turbo-json inference loader cannot narrow against an inventory:
        // load every unloaded owner up front -- the accepted conservative
        // tradeoff of lazy native discovery. This must happen before the
        // turbo.json loader is constructed: the no-turbo-json loader infers
        // the repository's task set from every scope's native catalogue, and
        // an inventory scope would capture an empty (guessed) catalogue.
        let selectors = Self::parse_filter_selectors(&self.opts.scope_opts.filter_patterns);
        if let Some(plan) = lazy_plan.as_mut()
            && self.requires_complete_graph_snapshot(
                selectors.as_deref(),
                micro_frontend_configs.as_ref(),
            )
        {
            let owners: HashSet<ToolchainId> =
                pkg_dep_graph.unloaded_owners().into_iter().collect();
            if !owners.is_empty() {
                pkg_dep_graph = Arc::new(plan.load(&owners).await?);
            }
        }

        let (turbo_json_loader, task_access_enabled) = self.build_turbo_json_loader(
            &pkg_dep_graph,
            root_package_json.is_some(),
            is_single_package,
            &micro_frontend_configs,
        )?;

        let root_turbo_json = {
            let _span = tracing::info_span!("root_turbo_json_load").entered();
            turbo_json_loader
                .load(&PackageName::Root)
                .map_err(turborepo_config::Error::from)?
                .clone()
        };

        let env_at_execution_start = {
            let _span = tracing::info_span!("env_infer").entered();
            EnvironmentVariableMap::infer()
        };

        // When filterUsingTasks is active, --affected is handled by the
        // same task-level filter rather than a separate codepath.
        let use_task_level_filter = self.opts.future_flags.filter_using_tasks
            && (!self.opts.scope_opts.filter_patterns.is_empty()
                || self.opts.scope_opts.affected_range.is_some());

        let use_task_level_affected = !use_task_level_filter
            && self.opts.scope_opts.affected_range.is_some()
            && self.opts.future_flags.affected_using_task_inputs;

        let has_task_level_affected_package_scope = use_task_level_affected
            && (!self.opts.scope_opts.filter_patterns.is_empty()
                || self.opts.scope_opts.pkg_inference_root.is_some());

        // Task-level affectedness replaces package-level affectedness. Resolve
        // package constraints independently so SCM is queried only by the task
        // detector and the final package list can come from selected tasks.
        let package_scope_opts = use_task_level_affected.then(|| {
            let mut opts = self.opts.clone();
            opts.scope_opts.affected_range = None;
            opts
        });
        let package_resolution_opts = package_scope_opts.as_ref().unwrap_or(&self.opts);

        // Resolution knowledge is complete at package-graph construction, so
        // scope filtering can read lockfile-affected packages without joining
        // deferred closure work.

        let task_access = {
            let _span = tracing::info_span!("task_access_setup").entered();
            let ta = TaskAccess::new(
                self.repo_root.clone(),
                async_cache.clone(),
                scm,
                task_access_enabled,
            );
            ta.restore_config().await;
            ta
        };

        let use_watch_task_level_filter = self
            .changed_files_for_watch
            .as_ref()
            .is_some_and(|changed_files| !changed_files.is_empty())
            && self.opts.future_flags.watch_using_task_inputs;

        let needs_all_packages = use_task_level_affected
            || use_task_level_filter
            || use_watch_task_level_filter
            || self.add_all_tasks;

        let settled = self
            .settle_engine(EngineSettlementInput {
                pkg_dep_graph,
                lazy_plan,
                turbo_json_loader: &turbo_json_loader,
                root_turbo_json: &root_turbo_json,
                env_at_execution_start: &env_at_execution_start,
                package_resolution_opts,
                scm,
                use_task_level_affected,
                has_task_level_affected_package_scope,
                needs_all_packages,
            })
            .await?;
        let settled = self.select_tasks(
            settled,
            SelectionModes {
                use_task_level_filter,
                use_task_level_affected,
                needs_all_packages,
            },
            &root_turbo_json,
            scm,
        )?;
        self.finalize_engine(&settled, &root_turbo_json, scm, untracked_scan_scope_tx)?;
        let SettledEngine {
            pkg_dep_graph,
            engine,
            filtered_pkgs,
            ..
        } = settled;

        Ok(BuiltExecutionContext {
            pkg_dep_graph,
            turbo_json_loader,
            root_turbo_json,
            task_access,
            env_at_execution_start,
            filtered_pkgs: filtered_pkgs.keys().cloned().collect(),
            engine: Arc::new(engine),
            micro_frontend_configs,
        })
    }

    /// Chooses the turbo.json loader for this repository shape and reports
    /// whether task access tracing is enabled.
    fn build_turbo_json_loader(
        &self,
        pkg_dep_graph: &PackageGraph,
        has_root_package_json: bool,
        is_single_package: bool,
        micro_frontend_configs: &Option<MicrofrontendsConfigs>,
    ) -> Result<(UnifiedTurboJsonLoader, bool), Error> {
        let root_turbo_json_path = self.opts.repo_opts.root_turbo_json_path.clone();
        let future_flags = self.opts.future_flags;
        let root_native_tasks = pkg_dep_graph
            .package_task_context(&PackageName::Root)
            .map(|context| context.native_tasks());
        let task_access_enabled = has_root_package_json
            && root_native_tasks
                .is_some_and(|tasks| TaskAccess::check_enabled(&self.repo_root, tasks));

        let reader = TurboJsonReader::new(self.repo_root.clone()).with_future_flags(future_flags);

        // The loader captures topology-stable inputs from the graph — scope
        // directories and root scripts — and reads package configs lazily per
        // package. It never captures hash-relevant contracts: those reach the
        // engine from the graph passed to `build_engine`, which by then is
        // authoritative for every scope the run consults.
        let turbo_json_loader = {
            let _span = tracing::info_span!("turbo_json_loader_setup").entered();
            if task_access_enabled {
                let root_scripts = root_native_tasks
                    .map(|tasks| tasks.script_names())
                    .unwrap_or_default();
                UnifiedTurboJsonLoader::task_access(
                    reader,
                    root_turbo_json_path.clone(),
                    root_scripts,
                )
            } else if is_single_package {
                let root_scripts = pkg_dep_graph
                    .package_task_context(&PackageName::Root)
                    .map(|context| {
                        context
                            .native_tasks()
                            .tasks()
                            .iter()
                            .filter(|task| task.participates() || task.authored())
                            .map(|task| task.name().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                UnifiedTurboJsonLoader::single_package(
                    reader,
                    root_turbo_json_path.clone(),
                    root_scripts,
                )
            } else if !root_turbo_json_path.exists() &&
            // Infer a turbo.json if allowing no turbo.json is explicitly allowed or if MFE configs are discovered
            (self.opts.repo_opts.allow_no_turbo_json || micro_frontend_configs.is_some())
            {
                let package_scripts = pkg_dep_graph
                    .package_task_contexts()
                    .map(|context| {
                        let package = context.package().clone();
                        let scripts = context
                            .native_tasks()
                            .tasks()
                            .iter()
                            .filter(|task| task.participates() || task.authored())
                            .map(|task| task.name().to_string())
                            .collect();
                        Ok((package, scripts))
                    })
                    .collect::<Result<_, Error>>()?;
                UnifiedTurboJsonLoader::workspace_no_turbo_json(
                    reader,
                    pkg_dep_graph.package_scope_directories(),
                    package_scripts,
                    micro_frontend_configs.clone(),
                )
            } else if let Some(micro_frontends) = &micro_frontend_configs {
                UnifiedTurboJsonLoader::workspace_with_microfrontends(
                    reader,
                    root_turbo_json_path.clone(),
                    pkg_dep_graph.package_scope_directories(),
                    micro_frontends.clone(),
                )
            } else {
                UnifiedTurboJsonLoader::workspace(
                    reader,
                    root_turbo_json_path.clone(),
                    pkg_dep_graph.package_scope_directories(),
                )
            }
        };

        Ok((turbo_json_loader, task_access_enabled))
    }

    /// Settles package selection, the package graph, and the engine against
    /// the run's metadata demands, then validates the settled graph.
    async fn settle_engine(
        &self,
        input: EngineSettlementInput<'_>,
    ) -> Result<SettledEngine, Error> {
        let EngineSettlementInput {
            mut pkg_dep_graph,
            mut lazy_plan,
            turbo_json_loader,
            root_turbo_json,
            env_at_execution_start,
            package_resolution_opts,
            scm,
            use_task_level_affected,
            has_task_level_affected_package_scope,
            needs_all_packages,
        } = input;

        // The task graph is finalized only after the run's metadata demands
        // settle. Each pass recomputes package selection and the engine over
        // the current graph; engine construction records the owner of any
        // inventory-only scope it had to consult, that owner is loaded, and
        // the pass repeats with authoritative facts. Loading is monotone, so
        // the loop terminates; the final selection below runs exactly once,
        // on the settled graph and engine — no frozen task set, no post-hoc
        // reconciliation.
        let engine_loader = EngineTurboJsonLoader::new(turbo_json_loader);
        let pass = PassEngineContext {
            root_turbo_json,
            engine_loader: &engine_loader,
            env_at_execution_start,
            needs_all_packages,
        };
        let mut engine;
        let mut filtered_pkgs;
        let mut filter_mode;
        let mut unqualified_entrypoint_packages;
        let mut entrypoint_exclusions;
        let mut all_pkgs;
        let mut task_level_affected_package_scope;
        loop {
            let (resolution, mode, entrypoint_packages) = {
                let _span = tracing::info_span!("calculate_filtered_packages").entered();
                Self::calculate_filtered_packages(
                    &self.repo_root,
                    package_resolution_opts,
                    &pkg_dep_graph,
                    scm,
                    root_turbo_json,
                )?
            };
            filtered_pkgs = resolution;
            filter_mode = mode;
            unqualified_entrypoint_packages = entrypoint_packages;
            if use_task_level_affected {
                filter_mode = FilterMode::ExplicitSelection;
            }

            // Narrowing: the existing package filter resolver matched this
            // selection against the inventory's exact identities (names,
            // directories, globs, negation — none of which need edges). Load
            // exactly the owners of the inventory-only scopes the selection
            // named, then recompute: identities do not change on load.
            //
            // The root's internal dependencies are hashed into every task —
            // their package directories feed the global hash regardless of
            // any `^task` edges — so their closure must be settled before
            // the run finalizes. An inventory-only scope inside the closure
            // has unknown outgoing edges: loading its owner can grow the
            // closure (and the hashed directory set) further, so each pass
            // re-checks until the closure reaches no unloaded scope.
            if let Some(plan) = lazy_plan.as_mut() {
                let mut owners_to_load: HashSet<ToolchainId> = filtered_pkgs
                    .keys()
                    .filter_map(|package| pkg_dep_graph.unloaded_scope_owner(package))
                    .cloned()
                    .collect();
                owners_to_load.extend(
                    pkg_dep_graph
                        .root_internal_package_dependencies()
                        .into_iter()
                        .filter_map(|package| pkg_dep_graph.unloaded_scope_owner(&package.name))
                        .cloned(),
                );
                if !owners_to_load.is_empty() {
                    pkg_dep_graph = Arc::new(plan.load(&owners_to_load).await?);
                    continue;
                }
            }

            // The root Turbo task namespace exists independently of a root
            // JavaScript package scope. Non-root namespaces, including
            // aggregate scopes, come from authoritative repository knowledge.
            let task_namespace_packages: Vec<_> = std::iter::once(PackageName::Root)
                .chain(
                    pkg_dep_graph
                        .package_scope_directories()
                        .map(|(name, _)| name)
                        .filter(|name| name != &PackageName::Root),
                )
                .collect();
            task_level_affected_package_scope = if has_task_level_affected_package_scope {
                Some(filtered_pkgs.keys().cloned().collect())
            } else {
                None
            };

            entrypoint_exclusions = self.pass_entrypoint_exclusions(
                &pkg_dep_graph,
                &unqualified_entrypoint_packages,
                &task_namespace_packages,
                &filter_mode,
                needs_all_packages,
            );

            // Config preloading overlaps engine construction. Repository-wide
            // engines consult every package's config, but scoped engines only
            // consult the filtered packages and the dependency closure their
            // `^task` edges follow, so narrow runs skip preloading unrelated
            // packages and let the engine load anything else lazily.
            turborepo_rayon_compat::block_in_place(|| {
                let _span = tracing::info_span!("turbo_json_preload").entered();
                if needs_all_packages {
                    turbo_json_loader.preload_all();
                } else {
                    let packages =
                        Self::scoped_preload_packages(&pkg_dep_graph, filtered_pkgs.keys());
                    turbo_json_loader.preload_packages(packages);
                }
            });

            // When task-level filtering or add_all_tasks is active, the engine
            // must contain tasks for ALL packages so that tasks in packages
            // not flagged by package-level scope resolution can still be
            // matched. The task-level filter (below) does the pruning when
            // needed.
            //
            // Inventory-only scopes are excluded from the repository-wide
            // workspace set: package-level resolution already loaded every
            // scope the selectors named (inventory identities are exact), so
            // an unloaded scope's tasks can only enter this run through a
            // dependency edge — and engine construction demands the owner for
            // exactly those. Whole-workspace enumeration therefore never
            // demands unrelated native owners.
            all_pkgs = if needs_all_packages {
                task_namespace_packages
                    .into_iter()
                    .filter(|package| pkg_dep_graph.unloaded_scope_owner(package).is_none())
                    .collect()
            } else {
                Vec::new()
            };
            let (built_engine, demands) = self.build_pass_engine(
                &mut pkg_dep_graph,
                &pass,
                &all_pkgs,
                &filtered_pkgs,
                &entrypoint_exclusions,
            )?;
            engine = built_engine;

            // Settle: load the owners construction had to consult and repeat
            // the pass. A reused complete snapshot has no inventory-only
            // scopes, so it records no demands.
            let newly_demanded = Self::newly_demanded_owners(lazy_plan.as_ref(), &demands);
            if newly_demanded.is_empty() {
                break;
            }
            let Some(plan) = lazy_plan.as_mut() else {
                unreachable!("metadata demands require a construction plan");
            };
            pkg_dep_graph = Arc::new(plan.load(&newly_demanded).await?);
        }

        // Validate the settled graph: the inventory graph and every
        // intermediate recomputation are provisional by construction, so
        // the invariant check belongs on the graph the run finalizes.
        {
            let _span = tracing::info_span!("pkg_dep_graph_validate").entered();
            pkg_dep_graph.validate()?;
        }

        Ok(SettledEngine {
            pkg_dep_graph,
            engine,
            filtered_pkgs,
            filter_mode,
            unqualified_entrypoint_packages,
            all_pkgs,
            task_level_affected_package_scope,
        })
    }

    /// Computes one settlement pass's task entrypoint exclusions. Explicitly
    /// requested package tasks are never excluded, and repository-wide
    /// engines exclude nothing.
    fn pass_entrypoint_exclusions(
        &self,
        pkg_dep_graph: &PackageGraph,
        unqualified_entrypoint_packages: &HashSet<PackageName>,
        task_namespace_packages: &[PackageName],
        filter_mode: &FilterMode,
        needs_all_packages: bool,
    ) -> HashSet<TaskId<'static>> {
        let mut scoped_entrypoint_exclusions = self.task_entrypoint_exclusions(
            pkg_dep_graph,
            unqualified_entrypoint_packages.iter(),
            task_namespace_packages.iter(),
            filter_mode,
        );
        let explicitly_requested_tasks: HashSet<_> = self
            .opts
            .run_opts
            .tasks
            .iter()
            .filter_map(|task| {
                TaskName::from(task.as_str())
                    .task_id()
                    .map(TaskId::into_owned)
            })
            .collect();
        scoped_entrypoint_exclusions
            .retain(|task_id| !explicitly_requested_tasks.contains(task_id));

        if needs_all_packages {
            HashSet::new()
        } else {
            scoped_entrypoint_exclusions
        }
    }

    /// Builds one settlement pass's engine, rebuilding it without package
    /// dependencies for `--parallel`, and returns the owners it demanded.
    fn build_pass_engine(
        &self,
        pkg_dep_graph: &mut Arc<PackageGraph>,
        pass: &PassEngineContext<'_>,
        all_pkgs: &[PackageName],
        filtered_pkgs: &HashMap<PackageName, PackageInclusionReason>,
        entrypoint_exclusions: &HashSet<TaskId<'static>>,
    ) -> Result<(RunEngine, HashSet<ToolchainId>), Error> {
        let needs_all_packages = pass.needs_all_packages;
        let engine_pkgs: Box<dyn Iterator<Item = &PackageName>> = if needs_all_packages {
            Box::new(all_pkgs.iter())
        } else {
            Box::new(filtered_pkgs.keys())
        };

        let (mut engine, mut demands) = self.build_engine(
            pkg_dep_graph,
            pass.root_turbo_json,
            engine_pkgs,
            entrypoint_exclusions,
            pass.engine_loader,
            pass.env_at_execution_start,
        )?;

        // --parallel removes inter-package dependencies from the package
        // graph, requiring a fresh engine build. Affected filtering runs
        // once afterward rather than on both engines to avoid a
        // redundant SCM query.
        if self.opts.run_opts.parallel {
            // A --parallel run never reuses a shared package graph (watch
            // graph sharing opts out for parallel), so this Arc is uniquely
            // owned here.
            let Some(graph) = Arc::get_mut(pkg_dep_graph) else {
                unreachable!("--parallel runs never reuse a shared package graph");
            };
            graph.remove_package_dependencies();
            let engine_pkgs: Box<dyn Iterator<Item = &PackageName>> = if needs_all_packages {
                Box::new(all_pkgs.iter())
            } else {
                Box::new(filtered_pkgs.keys())
            };
            let (rebuilt_engine, rebuilt_demands) = self.build_engine(
                pkg_dep_graph,
                pass.root_turbo_json,
                engine_pkgs,
                entrypoint_exclusions,
                pass.engine_loader,
                pass.env_at_execution_start,
            )?;
            engine = rebuilt_engine;
            demands.extend(rebuilt_demands);
        }

        Ok((engine, demands))
    }

    /// Owners demanded by engine construction that the plan has not loaded.
    fn newly_demanded_owners(
        lazy_plan: Option<&LazyPlan<CachingPackageDiscovery<LocalPackageDiscovery>>>,
        demands: &HashSet<ToolchainId>,
    ) -> HashSet<ToolchainId> {
        match lazy_plan {
            Some(plan) => {
                let loaded = plan.loaded_owners();
                demands
                    .iter()
                    .filter(|owner| !loaded.contains(*owner))
                    .cloned()
                    .collect()
            }
            None => {
                debug_assert!(
                    demands.is_empty(),
                    "a complete graph snapshot cannot produce metadata demands"
                );
                HashSet::new()
            }
        }
    }

    /// Applies task-level filtering, affectedness, and entrypoint selection to
    /// the settled engine.
    fn select_tasks(
        &self,
        settled: SettledEngine,
        modes: SelectionModes,
        root_turbo_json: &TurboJson,
        scm: &SCM,
    ) -> Result<SettledEngine, Error> {
        let SettledEngine {
            pkg_dep_graph,
            mut engine,
            mut filtered_pkgs,
            filter_mode,
            unqualified_entrypoint_packages,
            all_pkgs,
            task_level_affected_package_scope,
        } = settled;
        let SelectionModes {
            use_task_level_filter,
            use_task_level_affected,
            needs_all_packages,
        } = modes;

        // Task-level filter: resolve --filter and/or --affected against the task graph.
        if use_task_level_filter {
            let task_entrypoints = self
                .opts
                .future_flags
                .strict_task_entrypoint_selection
                .then(|| {
                    self.command_task_entrypoints(
                        &engine,
                        &pkg_dep_graph,
                        &all_pkgs.iter().cloned().collect(),
                    )
                });
            let excluded_entrypoints = HashSet::new();
            let selectors: Vec<turborepo_scope::TargetSelector> = self
                .opts
                .scope_opts
                .filter_patterns
                .iter()
                .map(|p| p.parse())
                .collect::<Result<_, _>>()
                .map_err(ResolutionError::from)?;

            let affected_constraint =
                if let Some(affected_range) = &self.opts.scope_opts.affected_range {
                    Some(turborepo_task_filter::resolve_affected_tasks(
                        &engine,
                        affected_range,
                        &pkg_dep_graph,
                        scm,
                        &self.repo_root,
                        &root_turbo_json.global_deps,
                    )?)
                } else {
                    None
                };

            let package_tasks: HashSet<_> = self
                .opts
                .run_opts
                .tasks
                .iter()
                .filter_map(|task| {
                    TaskName::from(task.as_str())
                        .task_id()
                        .map(TaskId::into_owned)
                })
                .collect();
            engine = turborepo_task_filter::filter_engine_to_tasks_with_inclusions(
                engine,
                &selectors,
                turborepo_task_filter::TaskFilterConstraints {
                    affected: affected_constraint.as_ref(),
                    always_include: &package_tasks,
                    entrypoints: task_entrypoints
                        .as_ref()
                        .map(|selection| &selection.candidates),
                    excluded_entrypoints: task_entrypoints
                        .as_ref()
                        .map_or(&excluded_entrypoints, |selection| &selection.excluded),
                    orchestration_entrypoints: task_entrypoints
                        .as_ref()
                        .map(|selection| &selection.orchestration),
                },
                &pkg_dep_graph,
                scm,
                &self.repo_root,
                &root_turbo_json.global_deps,
            )?;
        }

        // Task-level --affected detection (separate from --filter).
        if use_task_level_affected {
            let (affected_engine, selected_packages) = self.filter_engine_to_affected_tasks(
                engine,
                &pkg_dep_graph,
                root_turbo_json,
                scm,
                task_level_affected_package_scope.as_ref(),
            )?;
            engine = affected_engine;
            if let Some(selected_packages) = selected_packages {
                filtered_pkgs.retain(|package, _| selected_packages.contains(package));
            }
        }

        if needs_all_packages {
            engine = self.select_engine_task_entrypoints(engine, &pkg_dep_graph, &filter_mode);
        }

        if self.opts.future_flags.strict_task_entrypoint_selection
            && !use_task_level_filter
            && !self.add_all_tasks
        {
            let task_entrypoints = self.command_task_entrypoints(
                &engine,
                &pkg_dep_graph,
                &unqualified_entrypoint_packages,
            );
            engine = turborepo_task_filter::retain_strict_task_graph(
                engine,
                &pkg_dep_graph,
                task_entrypoints.selected,
                &task_entrypoints.orchestration,
            );
        }

        Ok(SettledEngine {
            pkg_dep_graph,
            engine,
            filtered_pkgs,
            filter_mode,
            unqualified_entrypoint_packages,
            all_pkgs,
            task_level_affected_package_scope,
        })
    }

    /// Scopes the untracked-file scan to the final engine and validates it.
    fn finalize_engine(
        &self,
        settled: &SettledEngine,
        root_turbo_json: &TurboJson,
        scm: &SCM,
        untracked_scan_scope_tx: Option<
            tokio::sync::oneshot::Sender<Option<Vec<RelativeUnixPathBuf>>>,
        >,
    ) -> Result<(), Error> {
        let SettledEngine {
            pkg_dep_graph,
            engine,
            filter_mode,
            ..
        } = settled;
        // The engine is final: every task the run will hash is known. Send
        // the untracked scan its scope. Provably package-local runs walk
        // only the participating packages' directories (plus the root
        // package's internal dependencies); everything else keeps today's
        // whole-repo scan.
        if let Some(scope_tx) = untracked_scan_scope_tx {
            let scoped_prefixes = Self::untracked_scan_prefixes(
                &self.repo_root,
                scm.git_root(),
                engine,
                pkg_dep_graph,
                root_turbo_json,
                filter_mode,
            );
            match &scoped_prefixes {
                Some(prefixes) => tracing::debug!(
                    prefixes = prefixes.len(),
                    "untracked-file scan scope: package directory prefixes"
                ),
                None => {
                    tracing::debug!("untracked-file scan scope: whole repo (not provably scoped)")
                }
            }
            // A send failure means the scan task is already gone; there is
            // nothing left to decide.
            let _ = scope_tx.send(scoped_prefixes);
        }

        // Validate after all filtering so the persistent task count reflects
        // the actual tasks that will execute, not the full pre-filter engine.
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

        Ok(())
    }

    async fn build_run_services(
        &self,
        input: RunServicesInput<'_>,
    ) -> Result<BuiltRunServices, Error> {
        let RunServicesInput {
            preflight_handle,
            async_cache,
            scm_state,
            scm_state_task,
            scm,
            repo_index,
            analytics_handle,
        } = input;
        let remote_cache_status =
            resolve_remote_cache_status(self.opts.remote_cache_disabled_reason, preflight_handle)
                .await;

        let run_cache = Arc::new(RunCache::new(
            async_cache,
            &self.repo_root,
            self.opts.runcache_opts,
            &self.opts.cache_opts,
            self.output_watcher.clone(),
            self.color_config,
            self.opts.run_opts.dry_run.is_some(),
        ));

        // futureFlags are hard gates: reject observability config when disabled.
        if let Some(obs_opts) = &self.opts.experimental_observability
            && obs_opts.otel.is_some()
            && !self.opts.future_flags.experimental_observability
        {
            return Err(turborepo_config::Error::InvalidExperimentalOtelConfig {
                message: "experimentalObservability.otel is configured but \
                          futureFlags.experimentalObservability is not enabled in turbo.json."
                    .to_string(),
            }
            .into());
        }

        let observability_handle = self
            .opts
            .experimental_observability
            .as_ref()
            .and_then(|opts| {
                let token = opts.otel.as_ref().and_then(|otel| {
                    if !otel.use_remote_cache_token.unwrap_or(false) {
                        return None;
                    }
                    let endpoint = otel.endpoint.as_deref().unwrap_or("");
                    let api_url = &self.opts.api_client_opts.api_url;
                    if !origins_match(endpoint, api_url) {
                        tracing::warn!(
                            "use_remote_cache_token is enabled but the OTEL endpoint ({endpoint}) \
                             does not match the API URL ({api_url}). Skipping cache token \
                             injection to prevent sending credentials to an unrelated endpoint."
                        );
                        return None;
                    }
                    self.api_auth.as_ref().map(|auth| auth.token.expose())
                });
                observability::Handle::try_init(opts, token)
            });
        if let Some(scm_state_task) = scm_state_task {
            let scm_state = scm_state.clone();
            let scm = scm.clone();
            let repo_index = repo_index.clone();
            tokio::spawn(
                async move {
                    let sha = scm_state_task.await.ok().flatten();
                    let repo_index = repo_index.get().await;
                    let dirty_hash =
                        tokio::task::spawn_blocking(move || match repo_index.as_ref() {
                            Some(repo_index) => scm.get_dirty_hash_from_repo_index(repo_index),
                            None => scm.get_dirty_hash(),
                        })
                        .await
                        .ok()
                        .flatten();
                    let state = if sha.is_some() || dirty_hash.is_some() {
                        Some(CacheScmState { sha, dirty_hash })
                    } else {
                        None
                    };
                    scm_state.resolve(state);
                }
                .instrument(tracing::info_span!("capture_scm_state")),
            );
        } else {
            scm_state.resolve(None);
        }

        Ok(BuiltRunServices {
            run_cache,
            remote_cache_status,
            observability_handle,
            analytics_handle,
        })
    }

    #[tracing::instrument(skip(self, signal_handler))]
    pub async fn build(
        self,
        signal_handler: &SignalHandler,
        telemetry: CommandEventBuilder,
    ) -> Result<(Run, Option<AnalyticsHandle>), Error> {
        tracing::trace!(
            platform = %TurboState::platform_name(),
            start_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |duration| duration.as_micros()),
            turbo_version = %TurboState::version(),
            numcpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
            "performing run on {:?}",
            TurboState::platform_name(),
        );
        let start_at = Local::now();

        let RepoDiscovery {
            run_telemetry,
            is_single_package,
            root_package_json,
            pkg_dep_graph,
            lazy_plan,
            scm,
            micro_frontend_configs,
            repo_index,
            untracked_scan_scope_tx,
        } = self.discover_repo(&telemetry).await?;

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
                Some(start_remote_cache_preflight(
                    client,
                    auth.token.clone(),
                    auth.team_id.clone(),
                    auth.team_slug.clone(),
                ))
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
            .and_then(|auth| {
                api_client
                    .clone()
                    .map(|api_client| start_analytics(auth.clone(), api_client))
            })
            .unzip();

        let scm_state = LazyScmState::new();
        let scm_state_task = (!self.skip_repo_index_and_scm_state).then(|| {
            let scm = scm.clone();
            let repo_root = self.repo_root.clone();
            tokio::task::spawn_blocking(move || {
                let _span = tracing::info_span!("capture_scm_sha").entered();
                scm.get_current_sha(&repo_root).ok()
            })
        });

        let async_cache = {
            let _span = tracing::info_span!("async_cache_new").entered();
            AsyncCache::new(
                &self.opts.cache_opts,
                &self.repo_root,
                api_client.clone(),
                self.api_auth.clone(),
                analytics_sender,
                scm_state.clone(),
            )?
        };

        let BuiltExecutionContext {
            pkg_dep_graph,
            turbo_json_loader,
            root_turbo_json,
            task_access,
            env_at_execution_start,
            filtered_pkgs,
            engine,
            micro_frontend_configs,
        } = self
            .build_execution_context(ExecutionContextInput {
                root_package_json,
                is_single_package,
                pkg_dep_graph,
                lazy_plan,
                scm: &scm,
                micro_frontend_configs,
                untracked_scan_scope_tx,
                async_cache: async_cache.clone(),
            })
            .await?;

        let BuiltRunServices {
            run_cache,
            remote_cache_status,
            observability_handle,
            analytics_handle,
        } = self
            .build_run_services(RunServicesInput {
                preflight_handle,
                async_cache,
                scm_state,
                scm_state_task,
                scm: &scm,
                repo_index: &repo_index,
                analytics_handle,
            })
            .await?;

        let repo = build_repo_context(RepoContextInput {
            repo_root: self.repo_root,
            color_config: self.color_config,
            version: self.version,
            scm,
            pkg_dep_graph,
            turbo_json_loader,
            root_turbo_json,
        });

        Ok((
            Run {
                repo,
                execution: ExecutionContext {
                    start_at,
                    opts: Arc::new(self.opts),
                    env_at_execution_start,
                    filtered_pkgs,
                    remote_cache_status,
                    engine,
                    task_access,
                    micro_frontend_configs,
                },
                services: RunServices {
                    processes: self.processes,
                    run_telemetry,
                    api_auth: self.api_auth,
                    run_cache,
                    signal_handler: signal_handler.clone(),
                    repo_index,
                    observability_handle,
                    query_server: self.query_server,
                    shutdown_started_emitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                },
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
        engine: RunEngine,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        change_detector: &impl ChangedFilesDetector,
        package_scope: Option<&HashSet<PackageName>>,
    ) -> Result<(RunEngine, Option<HashSet<PackageName>>), Error> {
        let (from_ref, to_ref) = self
            .opts
            .scope_opts
            .affected_range
            .as_ref()
            .ok_or(Error::MissingAffectedRange)?;
        let maybe_changed_files = changed_files_for_affected_range(
            change_detector,
            &self.repo_root,
            from_ref.as_deref(),
            to_ref.as_deref(),
        )?;

        match maybe_changed_files {
            Ok(changed_files) => {
                let total_tasks = engine.task_ids().count();
                let affected_tasks = turborepo_task_filter::affected_task_ids(
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
                // Scope affected entrypoints before retaining their execution
                // dependencies. Scoping the fully expanded execution graph
                // would incorrectly promote unaffected upstream dependencies
                // to selected tasks.
                let mut affected_entrypoints = engine.collect_task_dependents(&affected_tasks);
                affected_entrypoints.extend(affected_tasks);
                if let Some(package_scope) = package_scope {
                    let scoped_tasks = engine.task_ids_for_packages(package_scope);
                    affected_entrypoints.retain(|task| scoped_tasks.contains(task));
                }
                let selected_packages = affected_entrypoints
                    .iter()
                    .map(|task| PackageName::from(task.package()))
                    .collect();
                let affected_entrypoints =
                    turborepo_task_filter::expand_with_siblings(&engine, affected_entrypoints);
                Ok((
                    engine.retain_filtered_tasks(&affected_entrypoints),
                    Some(selected_packages),
                ))
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
                let Some(package_scope) = package_scope else {
                    return Ok((engine, None));
                };
                let scoped_tasks = engine.task_ids_for_packages(package_scope);
                let scoped_tasks =
                    turborepo_task_filter::expand_with_siblings(&engine, scoped_tasks);
                Ok((engine.retain_filtered_tasks(&scoped_tasks), None))
            }
        }
    }

    fn task_entrypoint_exclusions<'a>(
        &self,
        pkg_dep_graph: &PackageGraph,
        candidates: impl Iterator<Item = &'a PackageName>,
        exclusion_candidates: impl Iterator<Item = &'a PackageName>,
        filter_mode: &FilterMode,
    ) -> HashSet<TaskId<'static>> {
        let candidates: Vec<_> = candidates.cloned().collect();
        let exclusion_candidates: Vec<_> = exclusion_candidates.cloned().collect();
        self.opts
            .run_opts
            .tasks
            .iter()
            .map(|requested| TaskName::from(requested.as_str()))
            .filter(|task| task.package().is_none())
            .flat_map(|task| {
                self.task_entrypoint_exclusions_for_task(
                    pkg_dep_graph,
                    &task,
                    &candidates,
                    &exclusion_candidates,
                    filter_mode,
                )
            })
            .collect()
    }

    fn command_task_entrypoints(
        &self,
        engine: &RunEngine,
        pkg_dep_graph: &PackageGraph,
        candidate_packages: &HashSet<PackageName>,
    ) -> TaskEntrypointSelection {
        let mut selection = TaskEntrypointSelection::default();

        for requested in &self.opts.run_opts.tasks {
            let task = TaskName::from(requested.as_str());
            if let Some(task_id) = task.task_id().map(TaskId::into_owned) {
                if engine.task_definition(&task_id).is_some() {
                    selection.candidates.insert(task_id.clone());
                    selection.selected.insert(task_id.clone());
                    if !task_has_command(engine, pkg_dep_graph, &task_id) {
                        selection
                            .orchestration
                            .entry(task.to_string())
                            .or_default()
                            .insert(task_id);
                    }
                }
                continue;
            }

            let has_participant = pkg_dep_graph
                .package_task_contexts()
                .any(|context| context.native_tasks().participates(task.task()))
                || engine.task_ids().any(|task_id| {
                    task_id.task() == task.task()
                        && task_participates(engine, pkg_dep_graph, task_id)
                });

            for package in candidate_packages {
                let task_id = TaskId::new(package.as_ref(), task.task()).into_owned();
                if engine.task_definition(&task_id).is_none() {
                    continue;
                }

                selection.candidates.insert(task_id.clone());
                if !has_participant || task_participates(engine, pkg_dep_graph, &task_id) {
                    selection.selected.insert(task_id.clone());
                    if !has_participant {
                        selection
                            .orchestration
                            .entry(task.task().to_string())
                            .or_default()
                            .insert(task_id);
                    }
                } else {
                    selection.excluded.insert(task_id);
                }
            }
        }

        selection
    }

    fn task_entrypoint_exclusions_for_task(
        &self,
        pkg_dep_graph: &PackageGraph,
        task: &TaskName,
        candidates: &[PackageName],
        exclusion_candidates: &[PackageName],
        filter_mode: &FilterMode,
    ) -> HashSet<TaskId<'static>> {
        let preference = match filter_mode {
            FilterMode::AllPackages => TaskEntrypointPreference::Always,
            FilterMode::ExcludeOnly { .. } => TaskEntrypointPreference::Never,
            FilterMode::ExplicitSelection => TaskEntrypointPreference::WhenSingleCandidate,
        };
        pkg_dep_graph
            .task_entrypoint_exclusions(task.task(), candidates, exclusion_candidates, preference)
            .into_iter()
            .map(|name| TaskId::new(name.as_str(), task.task()).into_owned())
            .collect()
    }

    fn select_engine_task_entrypoints(
        &self,
        engine: RunEngine,
        pkg_dep_graph: &PackageGraph,
        filter_mode: &FilterMode,
    ) -> RunEngine {
        let package_tasks: HashSet<_> = self
            .opts
            .run_opts
            .tasks
            .iter()
            .filter_map(|task| {
                TaskName::from(task.as_str())
                    .task_id()
                    .map(TaskId::into_owned)
            })
            .collect();
        let mut exclusions = HashSet::new();
        for requested in &self.opts.run_opts.tasks {
            let task = TaskName::from(requested.as_str());
            if task.package().is_some() {
                continue;
            }
            let candidates: Vec<_> = engine
                .task_ids()
                .filter(|task_id| {
                    task_id.task() == task.task() && !package_tasks.contains(*task_id)
                })
                .map(|task_id| PackageName::from(task_id.package()))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            exclusions.extend(self.task_entrypoint_exclusions_for_task(
                pkg_dep_graph,
                &task,
                &candidates,
                &candidates,
                filter_mode,
            ));
        }
        if exclusions.is_empty() {
            return engine;
        }
        // Native-task preferences select entrypoints, not the dependency closure.
        // Keep excluded entrypoints when a retained task depends on them or schedules
        // them with `with`. Dependencies may themselves have `with` siblings, so
        // alternate both closures until no new tasks are added.
        let mut retained_tasks: HashSet<TaskId<'static>> = engine
            .task_ids()
            .filter(|task_id| !exclusions.contains(*task_id))
            .cloned()
            .collect();
        loop {
            let previous_len = retained_tasks.len();
            retained_tasks = turborepo_task_filter::expand_with_siblings(&engine, retained_tasks);
            let dependencies = engine.collect_task_dependencies(&retained_tasks);
            retained_tasks.extend(dependencies);
            if retained_tasks.len() == previous_len {
                break;
            }
        }
        engine.retain_filtered_tasks(&retained_tasks)
    }

    #[tracing::instrument(skip_all)]
    /// Build the engine for the current graph, returning it together with
    /// the owners of inventory-only scopes whose task metadata construction
    /// consulted. The demands are internal orchestration state: the caller
    /// loads the owners and rebuilds until no new demands arise, so the
    /// finalized engine never contains a task resolved against
    /// inventory-only metadata.
    fn build_engine<'a>(
        &self,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        filtered_pkgs: impl Iterator<Item = &'a PackageName>,
        entrypoint_exclusions: &HashSet<TaskId<'static>>,
        turbo_json_loader: &impl turborepo_engine::TurboJsonLoader,
        environment: &EnvironmentVariableMap,
    ) -> Result<(RunEngine, HashSet<ToolchainId>), Error> {
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
        let task_io_environment =
            project_task_io_environment(pkg_dep_graph.task_io_env_vars_by_domain(), environment)
                .map_err(Error::Env)?;
        let mut builder = EngineBuilder::new(
            &self.repo_root,
            pkg_dep_graph,
            turbo_json_loader,
            self.opts.run_opts.single_package,
        )
        .with_root_tasks(root_turbo_json.tasks.keys().cloned())
        .with_tasks_only(self.opts.run_opts.only)
        .with_entrypoint_exclusions(entrypoint_exclusions.clone())
        .with_workspaces(filtered_pkgs.cloned().collect())
        .with_future_flags(self.opts.future_flags)
        .with_global_deps(global_deps_for_task_inputs)
        .with_global_env(root_turbo_json.global_env.clone())
        .with_task_io_context(
            self.opts.run_opts.pass_through_args.clone(),
            self.opts.run_opts.tasks.clone(),
            task_io_environment,
        )
        .with_tasks(tasks);

        if self.add_all_tasks {
            builder = builder.add_all_tasks();
        }

        if !self.should_validate_engine {
            builder = builder.do_not_validate_engine();
        }

        let (mut engine, unloaded_demands) = builder.build_with_unloaded_demands()?;

        // In watch mode with the future flag, filter the engine to only tasks
        // whose declared inputs match the changed files.
        //
        // When active, this REPLACES create_engine_for_subgraph because
        // the entrypoint packages (from file watcher events) may not overlap
        // with the affected tasks (e.g. a $TURBO_ROOT$ input in another
        // package changes but the watcher only reports the package containing
        // the file).
        let watch_task_filtered = if let Some(ref changed_files) = self.changed_files_for_watch {
            if self.opts.future_flags.watch_using_task_inputs && !changed_files.is_empty() {
                let filter = turborepo_task_filter::resolve_watch_task_filter(
                    &engine,
                    pkg_dep_graph,
                    &self.repo_root,
                    changed_files,
                    &root_turbo_json.global_deps,
                );
                tracing::info!(
                    total_tasks = engine.task_ids().count(),
                    affected_tasks = filter.directly_affected.len(),
                    changed_files = filter.existing_files.len(),
                    "watch task-level input filtering complete"
                );
                engine = engine.retain_watch_affected_tasks(&filter.directly_affected);
                true
            } else {
                false
            }
        } else {
            false
        };

        // If we have an initial task, we prune out the engine to only
        // tasks that are reachable from that initial task.
        if !watch_task_filtered && let Some(entrypoint_packages) = &self.entrypoint_packages {
            engine = engine.create_engine_for_subgraph(entrypoint_packages);
        }

        Ok((engine, unloaded_demands))
    }
}

/// Whether any selector's answer depends on the package graph's edges or on
/// git history: dependency (`pkg...`) or dependents (`...pkg`) expansion,
/// match-dependencies (`pkg...[range]` — a reverse traversal, despite the
/// name), or git ranges. Such queries cannot be narrowed against an
/// inventory graph, whose edges are only the always-loaded JavaScript ones;
/// the run conservatively loads every native owner for them instead.
fn selectors_require_complete_graph(selectors: &[TargetSelector]) -> bool {
    selectors.iter().any(|selector| {
        selector.include_dependencies
            || selector.include_dependents
            || selector.match_dependencies
            || selector.git_range.is_some()
    })
}

/// Whether every file this task hashes stays inside its package directory.
///
/// `$TURBO_ROOT$` references are rewritten to `..`-relative globs before
/// reaching the engine, so any `..` path segment (or a surviving
/// `$TURBO_ROOT$` token) marks an input that escapes the package. Exclusion
/// globs only remove files, and dependency-output globs hash freshly
/// produced outputs without the repo index, so neither is checked. JIT
/// globs are hashed through the repo index at visitation time and must obey
/// the same rule as eager globs.
fn task_inputs_are_package_local(inputs: &TaskInputs) -> bool {
    inputs
        .globs
        .iter()
        .chain(inputs.jit_globs.iter())
        .filter(|glob| !glob.starts_with('!'))
        .all(|glob| !glob_escapes_package(glob))
}

/// Whether an input glob, interpreted relative to the task's package
/// directory, can match files outside that directory.
fn glob_escapes_package(glob: &str) -> bool {
    glob.contains("$TURBO_ROOT$") || glob.split('/').any(|segment| segment == "..")
}

/// Whether experimental Cargo package support is enabled, via
/// `futureFlags.experimentalCargoWorkspaces` in the root turbo.json. The
/// future flag is the only switch: it is repo-level configuration, so every
/// invoker sees the same package graph.
pub(crate) fn cargo_enabled(future_flags: &turborepo_turbo_json::FutureFlags) -> bool {
    RepositoryGraphFeatures::new(future_flags).cargo_enabled()
}

/// Whether experimental Python (uv) package support is enabled, via
/// `futureFlags.experimentalPythonWorkspaces` in the root turbo.json. The
/// future flag is the only switch: it is repo-level configuration, so every
/// invoker sees the same package graph.
pub(crate) fn python_enabled(future_flags: &turborepo_turbo_json::FutureFlags) -> bool {
    RepositoryGraphFeatures::new(future_flags).python_enabled()
}

/// Whether experimental Go workspace package support is enabled, via
/// `futureFlags.experimentalGoWorkspaces` in the root turbo.json.
pub(crate) fn go_enabled(future_flags: &turborepo_turbo_json::FutureFlags) -> bool {
    RepositoryGraphFeatures::new(future_flags).go_enabled()
}

fn origins_match(url1: &str, url2: &str) -> bool {
    let (Ok(url1), Ok(url2)) = (Url::parse(url1), Url::parse(url2)) else {
        return false;
    };

    if has_userinfo(&url1) || has_userinfo(&url2) {
        return false;
    }

    url1.origin() == url2.origin()
}

fn has_userinfo(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

#[cfg(test)]
mod task_io_context_tests {
    use std::collections::HashMap;

    use turborepo_env::EnvironmentVariableMap;
    use turborepo_hash::TaskHashable;
    use turborepo_repository::task_contracts::TaskEnvironmentDomain;
    use turborepo_types::EnvMode;

    use super::project_task_io_environment;

    #[test]
    fn empty_contract_patterns_are_excluded_from_task_io_projection() {
        let patterns = std::collections::BTreeMap::from([(
            TaskEnvironmentDomain::new("other"),
            vec!["OTHER_*"],
        )]);
        let environment = EnvironmentVariableMap::from(HashMap::from([
            ("NODE_ENV".to_string(), "production".to_string()),
            ("OTHER_KEY".to_string(), "value".to_string()),
        ]));

        let projected = project_task_io_environment(patterns, &environment).unwrap();
        assert!(!projected.contains_key(&TaskEnvironmentDomain::new("javascript")));
        assert_eq!(
            projected
                .get(&TaskEnvironmentDomain::new("other"))
                .and_then(|environment| environment.get("OTHER_KEY")),
            Some("value")
        );
    }

    #[test]
    fn projection_is_isolated_per_toolchain() {
        let alpha = TaskEnvironmentDomain::new("alpha");
        let beta = TaskEnvironmentDomain::new("beta");
        let patterns = std::collections::BTreeMap::from([
            (alpha.clone(), vec!["ALPHA_*"]),
            (beta.clone(), vec!["BETA_KEY"]),
        ]);
        let environment = EnvironmentVariableMap::from(HashMap::from([
            ("ALPHA_TARGET".to_string(), "alpha".to_string()),
            ("BETA_KEY".to_string(), "beta".to_string()),
            ("UNDECLARED_SECRET".to_string(), "secret".to_string()),
        ]));

        let projected = project_task_io_environment(patterns, &environment).unwrap();
        assert_eq!(projected.len(), 2);
        let alpha_environment = projected.get(&alpha).unwrap();
        assert_eq!(alpha_environment.get("ALPHA_TARGET"), Some("alpha"));
        assert_eq!(alpha_environment.get("BETA_KEY"), None);
        assert_eq!(alpha_environment.get("UNDECLARED_SECRET"), None);
        let beta_environment = projected.get(&beta).unwrap();
        assert_eq!(beta_environment.get("BETA_KEY"), Some("beta"));
        assert_eq!(beta_environment.get("ALPHA_TARGET"), None);
        assert_eq!(beta_environment.get("UNDECLARED_SECRET"), None);
    }

    #[test]
    fn cargo_projection_keeps_only_rustup_selection_environment() {
        let patterns = std::collections::BTreeMap::from([(
            TaskEnvironmentDomain::new("cargo-task-io"),
            vec!["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"],
        )]);
        let environment = EnvironmentVariableMap::from(HashMap::from([
            ("RUSTUP_HOME".to_string(), "/rustup".to_string()),
            ("RUSTUP_TOOLCHAIN".to_string(), "stable-host".to_string()),
            (
                "RUSTUP_DIST_SERVER".to_string(),
                "https://example.invalid".to_string(),
            ),
        ]));

        let projected = project_task_io_environment(patterns, &environment).unwrap();
        let cargo = projected
            .get(&TaskEnvironmentDomain::new("cargo-task-io"))
            .unwrap();
        assert_eq!(cargo.get("RUSTUP_HOME"), Some("/rustup"));
        assert_eq!(cargo.get("RUSTUP_TOOLCHAIN"), Some("stable-host"));
        assert_eq!(cargo.get("RUSTUP_DIST_SERVER"), None);
    }

    fn projected_task_hash(layout: &str, secret: &str) -> String {
        let alpha = TaskEnvironmentDomain::new("alpha");
        let patterns = std::collections::BTreeMap::from([(alpha.clone(), vec!["ALPHA_*"])]);
        let environment = EnvironmentVariableMap::from(HashMap::from([
            ("ALPHA_TARGET".to_string(), layout.to_string()),
            ("UNDECLARED_SECRET".to_string(), secret.to_string()),
        ]));
        let projected = project_task_io_environment(patterns, &environment).unwrap();
        let layout = projected
            .get(&alpha)
            .and_then(|environment| environment.get("ALPHA_TARGET"))
            .unwrap();
        let declared = vec!["ALPHA_*".to_string()];

        TaskHashable {
            global_hash: "global",
            task_dependency_hashes: Vec::new(),
            hash_of_files: "files",
            external_deps_hash: None,
            package_dir: None,
            task: "build",
            outputs: Default::default(),
            pass_through_args: &[],
            env: &declared,
            resolved_env_vars: vec![format!("ALPHA_TARGET={layout}")],
            pass_through_env: &[],
            env_mode: EnvMode::Strict,
            command_override: &[],
            command_opt_out: false,
            experimental_ci: None,
        }
        .calculate_task_hash()
        .unwrap()
    }

    #[test]
    fn projected_declared_environment_changes_task_hash_only() {
        let baseline = projected_task_hash("one", "secret-one");
        assert_ne!(baseline, projected_task_hash("two", "secret-one"));
        assert_eq!(baseline, projected_task_hash("one", "secret-two"));
    }
}

#[cfg(test)]
mod package_prefix_tests {
    use super::*;

    #[test]
    fn repo_index_prefix_is_git_root_relative_for_nested_turbo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let git_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let repo_root = git_root.join_component("downloaded-app");

        assert_eq!(
            RunBuilder::repo_prefix_for_repo_index(&repo_root, &git_root).unwrap(),
            RelativeUnixPathBuf::new("downloaded-app").unwrap()
        );
    }

    #[test]
    fn repo_index_prefix_is_empty_when_git_root_matches_turbo_root() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

        assert_eq!(
            RunBuilder::repo_prefix_for_repo_index(&repo_root, &repo_root).unwrap(),
            RelativeUnixPathBuf::new("").unwrap()
        );
    }
}

#[cfg(test)]
mod untracked_scoping_tests {
    use super::*;

    fn inputs(globs: &[&str]) -> TaskInputs {
        TaskInputs {
            globs: globs.iter().map(|g| g.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn default_and_package_relative_inputs_are_package_local() {
        // A task with no `inputs` hashes everything under the package.
        assert!(task_inputs_are_package_local(&TaskInputs::default()));
        assert!(task_inputs_are_package_local(&inputs(&[
            "src/**",
            "README.md"
        ])));
        // Exclusions only remove files.
        assert!(task_inputs_are_package_local(&inputs(&[
            "src/**", "!dist/**"
        ])));
        assert!(task_inputs_are_package_local(&inputs(&["!../../dist/**"])));
        // Literal names containing dots are not parent references.
        assert!(task_inputs_are_package_local(&inputs(&[
            "a..b.txt",
            "v1.0.0.txt"
        ])));
    }

    #[test]
    fn escaping_inputs_are_not_package_local() {
        // `$TURBO_ROOT$` is rewritten to `..`-relative paths in the engine.
        assert!(!task_inputs_are_package_local(&inputs(&[
            "../../config.json"
        ])));
        // Defensive: a surviving `$TURBO_ROOT$` token also escapes.
        assert!(!task_inputs_are_package_local(&inputs(&[
            "$TURBO_ROOT$/config.json"
        ])));
        assert!(!task_inputs_are_package_local(&inputs(&[".."])));
        assert!(!task_inputs_are_package_local(&inputs(&[
            "src/../../shared/**"
        ])));
        assert!(!task_inputs_are_package_local(&inputs(&["src/.."])));
    }

    #[test]
    fn jit_globs_obey_the_same_rule() {
        let mut jit = inputs(&[]);
        jit.jit_globs = vec!["../generated/**".to_string()];
        assert!(!task_inputs_are_package_local(&jit));

        let mut jit_ok = inputs(&[]);
        jit_ok.jit_globs = vec!["src/generated/**".to_string()];
        assert!(task_inputs_are_package_local(&jit_ok));
    }

    #[test]
    fn glob_escape_segments() {
        assert!(glob_escapes_package("../x"));
        assert!(glob_escapes_package("a/../x"));
        assert!(glob_escapes_package("a/.."));
        assert!(glob_escapes_package(".."));
        assert!(!glob_escapes_package("a..b"));
        assert!(!glob_escapes_package("src/**"));
        assert!(!glob_escapes_package(""));
    }
}

#[cfg(test)]
mod origins_match_tests {
    use std::sync::{Arc, Mutex};

    use turbopath::AnchoredSystemPathBuf;
    use turborepo_engine::Building;
    use turborepo_repository::{
        discovery::PackageDiscovery, package_graph::PackageGraph, package_json::PackageJson,
        package_manager::PackageManager,
    };
    use turborepo_run_opts::{ExecutionSelector, RunSelector};

    use super::*;

    struct MockDiscovery;

    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            Ok(turborepo_repository::discovery::DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<
            turborepo_repository::discovery::DiscoveryResponse,
            turborepo_repository::discovery::Error,
        > {
            self.discover_packages().await
        }
    }

    fn package_graph_with_dependencies(
        root: &AbsoluteSystemPath,
        dependencies: &[(&str, &str)],
    ) -> PackageGraph {
        let package_names: std::collections::BTreeSet<&str> =
            dependencies.iter().flat_map(|(a, b)| [*a, *b]).collect();
        let mut package_jsons = std::collections::HashMap::new();
        for package in package_names {
            let deps: Vec<(String, String)> = dependencies
                .iter()
                .filter(|(a, _)| *a == package)
                .map(|(_, b)| (b.to_string(), "*".to_string()))
                .collect();
            package_jsons.insert(
                root.join_components(&["packages", package, "package.json"]),
                PackageJson {
                    name: Some(turborepo_errors::Spanned::new(package.to_string())),
                    dependencies: (!deps.is_empty()).then(|| {
                        deps.into_iter()
                            .collect::<std::collections::BTreeMap<_, _>>()
                    }),
                    ..Default::default()
                },
            );
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(
            PackageGraph::builder(root, Default::default())
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(package_jsons))
                .build(),
        )
        .unwrap()
    }

    fn injected_repo_context(
        repo_root: &AbsoluteSystemPath,
        pkg_dep_graph: Arc<PackageGraph>,
    ) -> Arc<RepoContext> {
        build_repo_context(RepoContextInput {
            repo_root: repo_root.to_owned(),
            color_config: ColorConfig::new(true),
            version: "test",
            scm: SCM::Manual,
            pkg_dep_graph,
            turbo_json_loader: UnifiedTurboJsonLoader::noop(HashMap::from([(
                PackageName::Root,
                TurboJson::default(),
            )])),
            root_turbo_json: TurboJson::default(),
        })
    }

    #[test]
    fn repo_context_accepts_injected_javascript_package_graph_and_services() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let graph = Arc::new(package_graph_with_dependencies(
            &repo_root,
            &[("web", "shared")],
        ));

        let context = injected_repo_context(&repo_root, graph);

        assert_eq!(context.repo_root(), repo_root.as_ref());
        assert_eq!(context.version(), "test");
        assert!(context.scm.is_manual());
        assert!(
            context
                .pkg_dep_graph()
                .package_task_context(&PackageName::from("web"))
                .is_some()
        );
        assert!(
            context
                .pkg_dep_graph()
                .package_task_context(&PackageName::from("shared"))
                .is_some()
        );
        assert!(context.turbo_json_loader.load(&PackageName::Root).is_ok());
    }

    #[tokio::test]
    async fn repo_context_accepts_injected_mixed_toolchain_package_graph() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let write = |relative: &[&str], contents: &str| {
            let path = repo_root.join_components(relative);
            std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
            std::fs::write(path.as_std_path(), contents).unwrap();
        };
        write(
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"rust/app\"]\nresolver = \
             \"2\"\n\n[workspace.metadata]\nname = \"rust-workspace\"\n",
        );
        write(
            &["rust", "app", "Cargo.toml"],
            "[package]\nname = \"rust-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            &["Cargo.lock"],
            "version = 3\n\n[[package]]\nname = \"rust-app\"\nversion = \"0.1.0\"\n",
        );
        write(&["go.work"], "go 1.23.0\n\nuse ./go/app\n");
        write(
            &["go", "app", "go.mod"],
            "module example.com/go-app\n\ngo 1.23.0\n",
        );
        write(
            &["pyproject.toml"],
            "[tool.turbo]\nname = \"python-workspace\"\n\n[tool.uv.workspace]\nmembers = \
             [\"python/*\"]\n",
        );
        write(
            &["python", "app", "pyproject.toml"],
            "[project]\nname = \"python-app\"\nversion = \"0.1.0\"\n",
        );

        let js_package_path = repo_root.join_components(&["packages", "web", "package.json"]);
        let package_graph =
            PackageGraph::builder_optional(&repo_root, Some(PackageJson::default()))
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(HashMap::from([(
                    js_package_path,
                    PackageJson {
                        name: Some(turborepo_errors::Spanned::new("web".to_string())),
                        ..Default::default()
                    },
                )])))
                .with_cargo()
                .with_go()
                .with_uv()
                .build_lazy()
                .await
                .unwrap()
                .into_parts()
                .0;
        let context = injected_repo_context(&repo_root, package_graph);

        for package in ["web", "rust-app", "go-app", "python-app"] {
            assert!(
                context
                    .pkg_dep_graph()
                    .package_task_context(&PackageName::from(package))
                    .is_some(),
                "expected injected package graph to retain {package}"
            );
        }
        assert!(context.scm.is_manual());
        assert!(context.turbo_json_loader.load(&PackageName::Root).is_ok());
    }

    type ChangeCall = (Option<String>, Option<String>, bool, bool, bool);

    #[derive(Clone)]
    struct FixedPackageChanges {
        calls: Arc<Mutex<Vec<ChangeCall>>>,
    }

    impl GitChangeDetector for FixedPackageChanges {
        fn changed_packages(
            &self,
            from_ref: Option<&str>,
            to_ref: Option<&str>,
            include_uncommitted: bool,
            allow_unknown_objects: bool,
            merge_base: bool,
        ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
            self.calls.lock().unwrap().push((
                from_ref.map(str::to_string),
                to_ref.map(str::to_string),
                include_uncommitted,
                allow_unknown_objects,
                merge_base,
            ));
            Ok(HashMap::from([(
                PackageName::from("lib"),
                PackageInclusionReason::FileChanged {
                    file: AnchoredSystemPathBuf::from_raw("packages/lib/src/index.ts").unwrap(),
                },
            )]))
        }
    }

    fn affected_opts(repo_root: &AbsoluteSystemPathBuf, filters: &[&str]) -> Opts {
        let run_opts = RunSelector::default();
        let execution_opts = ExecutionSelector {
            affected: true,
            ..Default::default()
        };
        let mut opts = Opts::new(
            repo_root,
            &run_opts,
            &execution_opts,
            turborepo_config::ConfigurationOptions::default(),
        )
        .unwrap();
        opts.scope_opts.filter_patterns = filters.iter().map(|filter| filter.to_string()).collect();
        opts
    }

    fn run_builder(repo_root: &AbsoluteSystemPathBuf, filters: &[&str]) -> RunBuilder {
        RunBuilder::new(
            crate::RunBuilderInput {
                repo_root: repo_root.clone(),
                color_config: ColorConfig::new(true),
                opts: affected_opts(repo_root, filters),
                version: "test",
                api_auth: None,
            },
            None,
        )
        .unwrap()
    }

    fn task_engine(
        definitions: &[(TaskId<'static>, TaskDefinition)],
        edges: &[(TaskId<'static>, TaskId<'static>)],
    ) -> RunEngine {
        let mut engine: Engine<Building, TaskDefinition> = Engine::new();
        for (task_id, definition) in definitions {
            engine.get_index(task_id);
            engine.add_definition(task_id.clone(), definition.clone());
        }
        for (from, to) in edges {
            let from_index = engine.get_index(from);
            let to_index = engine.get_index(to);
            engine.task_graph_mut().add_edge(from_index, to_index, ());
        }
        engine.seal()
    }

    fn task_with_inputs(globs: &[&str]) -> TaskDefinition {
        TaskDefinition {
            inputs: TaskInputs {
                globs: globs.iter().map(|glob| glob.to_string()).collect(),
                default: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn filtered_packages_can_use_an_injected_change_detector() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let graph = package_graph_with_dependencies(&repo_root, &[("app", "lib")]);
        let run_opts = RunSelector::default();
        let execution_opts = ExecutionSelector {
            affected: true,
            ..Default::default()
        };
        let opts = Opts::new(
            &repo_root,
            &run_opts,
            &execution_opts,
            turborepo_config::ConfigurationOptions::default(),
        )
        .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));

        let (packages, mode, _) = RunBuilder::calculate_filtered_packages_with_change_detector(
            &repo_root,
            &opts,
            &graph,
            FixedPackageChanges {
                calls: calls.clone(),
            },
            &TurboJson::default(),
        )
        .unwrap();

        assert_eq!(mode, FilterMode::ExplicitSelection);
        assert!(packages.contains_key(&PackageName::from("lib")));
        assert!(packages.contains_key(&PackageName::from("app")));
        assert_eq!(*calls.lock().unwrap(), [(None, None, true, true, true)]);
    }

    #[test]
    fn affected_package_filter_intersects_fixed_package_changes() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let graph = package_graph_with_dependencies(&repo_root, &[("app", "lib")]);
        let opts = affected_opts(&repo_root, &["app"]);
        let calls = Arc::new(Mutex::new(Vec::new()));

        let (packages, mode, _) = RunBuilder::calculate_filtered_packages_with_change_detector(
            &repo_root,
            &opts,
            &graph,
            FixedPackageChanges {
                calls: calls.clone(),
            },
            &TurboJson::default(),
        )
        .unwrap();

        assert_eq!(mode, FilterMode::ExplicitSelection);
        assert_eq!(names(packages.into_keys().collect()), ["app"]);
        assert_eq!(*calls.lock().unwrap(), [(None, None, true, true, true)]);
    }

    #[test]
    fn missing_python_filter_points_to_disabled_workspace_support() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        repo_root
            .join_component(turborepo_repository::uv::PYPROJECT_TOML)
            .create_with_contents("[tool.uv.workspace]\nmembers = ['packages/*']\n")
            .unwrap();
        let graph = package_graph_with_dependencies(&repo_root, &[("app", "lib")]);
        let run_opts = RunSelector::default();
        let execution_opts = ExecutionSelector::default();
        let mut opts = Opts::new(
            &repo_root,
            &run_opts,
            &execution_opts,
            turborepo_config::ConfigurationOptions::default(),
        )
        .unwrap();
        opts.scope_opts.filter_patterns = vec!["py-app".to_string()];

        let error = RunBuilder::calculate_filtered_packages_with_change_detector(
            &repo_root,
            &opts,
            &graph,
            FixedPackageChanges {
                calls: Arc::new(Mutex::new(Vec::new())),
            },
            &TurboJson::default(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Error::PackageMayBePythonPackage { ref name } if name == "py-app"
        ));
        let hint = miette::Diagnostic::help(&error)
            .expect("disabled Python support has an opt-in hint")
            .to_string();
        assert!(hint.contains("experimentalPythonWorkspaces"), "{hint}");
    }

    #[derive(Clone)]
    struct FixedChangedFiles {
        files: HashSet<AnchoredSystemPathBuf>,
        calls: Arc<Mutex<Vec<ChangeCall>>>,
    }

    impl ChangedFilesDetector for FixedChangedFiles {
        fn changed_files(
            &self,
            _turbo_root: &AbsoluteSystemPath,
            from_ref: Option<&str>,
            to_ref: Option<&str>,
            include_uncommitted: bool,
            allow_unknown_objects: bool,
            merge_base: bool,
        ) -> Result<
            Result<HashSet<AnchoredSystemPathBuf>, turborepo_scm::git::InvalidRange>,
            turborepo_scm::Error,
        > {
            self.calls.lock().unwrap().push((
                from_ref.map(str::to_string),
                to_ref.map(str::to_string),
                include_uncommitted,
                allow_unknown_objects,
                merge_base,
            ));
            Ok(Ok(self.files.clone()))
        }
    }

    #[test]
    fn affected_file_observation_passes_merge_base_policy_to_the_injected_detector() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let files =
            HashSet::from([AnchoredSystemPathBuf::from_raw("apps/web/src/index.ts").unwrap()]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let detected = changed_files_for_affected_range(
            &FixedChangedFiles {
                files: files.clone(),
                calls: calls.clone(),
            },
            &repo_root,
            Some("main"),
            Some("HEAD"),
        )
        .unwrap()
        .unwrap();

        assert_eq!(detected, files);
        assert_eq!(
            *calls.lock().unwrap(),
            [(
                Some("main".to_string()),
                Some("HEAD".to_string()),
                true,
                true,
                true
            )]
        );
    }

    #[test]
    fn task_level_affected_filter_uses_an_injected_changed_file_set() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let graph = package_graph_with_dependencies(&repo_root, &[("app", "lib")]);
        let run_opts = RunSelector::default();
        let execution_opts = ExecutionSelector {
            affected: true,
            ..Default::default()
        };
        let opts = Opts::new(
            &repo_root,
            &run_opts,
            &execution_opts,
            turborepo_config::ConfigurationOptions::default(),
        )
        .unwrap();
        let builder = RunBuilder::new(
            crate::RunBuilderInput {
                repo_root: repo_root.clone(),
                color_config: ColorConfig::new(true),
                opts,
                version: "test",
                api_auth: None,
            },
            None,
        )
        .unwrap();

        let app_build = TaskId::new("app", "build").into_owned();
        let lib_build = TaskId::new("lib", "build").into_owned();
        let mut engine: Engine<Building, TaskDefinition> = Engine::new();
        let app_index = engine.get_index(&app_build);
        let lib_index = engine.get_index(&lib_build);
        engine.add_definition(
            app_build.clone(),
            TaskDefinition {
                command: Some(turborepo_types::TaskCommandOverride::Argv(vec![
                    "build".into(),
                ])),
                ..Default::default()
            },
        );
        engine.add_definition(
            lib_build.clone(),
            TaskDefinition {
                command: Some(turborepo_types::TaskCommandOverride::Argv(vec![
                    "build".into(),
                ])),
                inputs: TaskInputs {
                    globs: vec!["src/**".to_string()],
                    default: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        engine.task_graph_mut().add_edge(app_index, lib_index, ());

        let calls = Arc::new(Mutex::new(Vec::new()));
        let (filtered, selected_packages) = builder
            .filter_engine_to_affected_tasks(
                engine.seal(),
                &graph,
                &TurboJson::default(),
                &FixedChangedFiles {
                    files: HashSet::from([AnchoredSystemPathBuf::from_raw(
                        "packages/lib/src/index.ts",
                    )
                    .unwrap()]),
                    calls: calls.clone(),
                },
                None,
            )
            .unwrap();

        assert!(filtered.task_definition(&lib_build).is_some());
        assert!(filtered.task_definition(&app_build).is_some());
        assert_eq!(
            selected_packages.unwrap(),
            HashSet::from([PackageName::from("app"), PackageName::from("lib")])
        );
        assert_eq!(*calls.lock().unwrap(), [(None, None, true, true, true)]);
    }

    #[test]
    fn task_level_affected_filter_scopes_entrypoints_before_dependencies() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let graph = package_graph_with_dependencies(
            &repo_root,
            &[("app", "lib"), ("lib-no-test", "placeholder")],
        );
        let builder = run_builder(&repo_root, &[]);
        let app_build = TaskId::new("app", "build").into_owned();
        let lib_build = TaskId::new("lib", "build").into_owned();
        let no_test_build = TaskId::new("lib-no-test", "build").into_owned();
        let definitions = [
            (app_build.clone(), task_with_inputs(&["src/**"])),
            (lib_build.clone(), task_with_inputs(&["src/**"])),
            (no_test_build.clone(), task_with_inputs(&["src/**"])),
        ];
        let edges = [(app_build.clone(), lib_build.clone())];
        let files = HashSet::from([
            AnchoredSystemPathBuf::from_raw("packages/app/src/index.ts").unwrap(),
            AnchoredSystemPathBuf::from_raw("packages/lib-no-test/src/index.ts").unwrap(),
        ]);

        let library_scope = HashSet::from([PackageName::from("lib")]);
        let (filtered, selected) = builder
            .filter_engine_to_affected_tasks(
                task_engine(&definitions, &edges),
                &graph,
                &TurboJson::default(),
                &FixedChangedFiles {
                    files: files.clone(),
                    calls: Arc::new(Mutex::new(Vec::new())),
                },
                Some(&library_scope),
            )
            .unwrap();
        assert!(filtered.task_ids().next().is_none());
        assert!(selected.unwrap().is_empty());

        let app_scope = HashSet::from([PackageName::from("app")]);
        let (filtered, selected) = builder
            .filter_engine_to_affected_tasks(
                task_engine(&definitions, &edges),
                &graph,
                &TurboJson::default(),
                &FixedChangedFiles {
                    files: files.clone(),
                    calls: Arc::new(Mutex::new(Vec::new())),
                },
                Some(&app_scope),
            )
            .unwrap();
        assert!(filtered.task_definition(&app_build).is_some());
        assert!(filtered.task_definition(&lib_build).is_some());
        assert_eq!(selected.unwrap(), app_scope);

        let no_test_scope = HashSet::from([PackageName::from("lib-no-test")]);
        let (filtered, selected) = builder
            .filter_engine_to_affected_tasks(
                task_engine(&definitions, &edges),
                &graph,
                &TurboJson::default(),
                &FixedChangedFiles {
                    files,
                    calls: Arc::new(Mutex::new(Vec::new())),
                },
                Some(&no_test_scope),
            )
            .unwrap();
        assert!(filtered.task_definition(&no_test_build).is_some());
        assert!(filtered.task_definition(&app_build).is_none());
        assert!(filtered.task_definition(&lib_build).is_none());
        assert_eq!(selected.unwrap(), no_test_scope);
    }

    #[test]
    fn task_level_affected_filter_matches_root_inputs_without_globalizing_package_json() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let graph = package_graph_with_dependencies(&repo_root, &[("lib-a", "placeholder")]);
        let builder = run_builder(&repo_root, &[]);
        let test_task = TaskId::new("lib-a", "test").into_owned();
        let definitions = [(test_task.clone(), task_with_inputs(&["../../shared.txt"]))];

        for (file, expected) in [("shared.txt", true), ("package.json", false)] {
            let (filtered, selected) = builder
                .filter_engine_to_affected_tasks(
                    task_engine(&definitions, &[]),
                    &graph,
                    &TurboJson::default(),
                    &FixedChangedFiles {
                        files: HashSet::from([AnchoredSystemPathBuf::from_raw(file).unwrap()]),
                        calls: Arc::new(Mutex::new(Vec::new())),
                    },
                    None,
                )
                .unwrap();

            assert_eq!(
                filtered.task_definition(&test_task).is_some(),
                expected,
                "unexpected selection for changed file {file}"
            );
            assert_eq!(
                selected.unwrap(),
                if expected {
                    HashSet::from([PackageName::from("lib-a")])
                } else {
                    HashSet::new()
                }
            );
        }
    }

    fn names(packages: Vec<PackageName>) -> Vec<String> {
        let mut names: Vec<String> = packages.into_iter().map(|name| name.to_string()).collect();
        names.sort();
        names
    }

    #[test]
    fn scoped_preload_packages_follows_topological_dependency_closure() {
        let temp_folder = tempfile::TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(temp_folder.path()).unwrap();
        // a -> b -> c and d -> b
        let graph = package_graph_with_dependencies(&root, &[("a", "b"), ("b", "c"), ("d", "b")]);

        let closure = |seeds: &[&str]| {
            let owned: Vec<PackageName> =
                seeds.iter().map(|name| PackageName::from(*name)).collect();
            names(super::RunBuilder::scoped_preload_packages(
                &graph,
                owned.iter(),
            ))
        };

        assert_eq!(closure(&["a"]), ["a", "b", "c"]);
        assert_eq!(closure(&["d"]), ["b", "c", "d"]);
        // A dependency-free package preloads only itself.
        assert_eq!(closure(&["c"]), ["c"]);
        // Multiple seeds are unioned and deduplicated.
        assert_eq!(closure(&["a", "d"]), ["a", "b", "c", "d"]);
    }

    #[test]
    fn same_host_different_paths() {
        assert!(origins_match(
            "https://vercel.com/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn different_hosts() {
        assert!(!origins_match(
            "https://third-party.com/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn case_insensitive() {
        assert!(origins_match(
            "https://Vercel.COM/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn with_port() {
        assert!(origins_match(
            "https://vercel.com:443/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn different_ports_do_not_match() {
        assert!(!origins_match(
            "https://vercel.com:4317/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn different_schemes_do_not_match() {
        assert!(!origins_match(
            "http://vercel.com/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn different_subdomains_do_not_match() {
        assert!(!origins_match(
            "https://otel.vercel.com/v1",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn userinfo_host_bypass_does_not_match() {
        assert!(!origins_match(
            "https://api.vercel.com:443@attacker.example/otel",
            "https://api.vercel.com/api"
        ));
    }

    #[test]
    fn same_origin_with_userinfo_does_not_match() {
        assert!(!origins_match(
            "https://token@vercel.com/otel",
            "https://vercel.com/api"
        ));
    }

    #[test]
    fn api_url_with_userinfo_does_not_match() {
        assert!(!origins_match(
            "https://vercel.com/otel",
            "https://token@vercel.com/api"
        ));
    }

    #[test]
    fn missing_scheme_returns_false() {
        assert!(!origins_match("vercel.com/otel", "https://vercel.com/api"));
    }

    #[test]
    fn empty_url_returns_false() {
        assert!(!origins_match("", "https://vercel.com/api"));
    }
}

/// Generic lazy-loading decision tests. Selector classification is purely
/// structural — no language is named, no toolchain is special-cased.
#[cfg(test)]
mod lazy_selector_tests {
    use turborepo_scope::TargetSelector;

    use super::selectors_require_complete_graph;

    fn selectors(patterns: &[&str]) -> Vec<TargetSelector> {
        patterns
            .iter()
            .map(|pattern| pattern.parse::<TargetSelector>())
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn package_level_selectors_do_not_require_a_complete_graph() {
        // Exact names, name globs, directories, and negation resolve against
        // inventory identities alone; the existing scope resolver handles
        // them without edges.
        for patterns in [
            vec!["web"],
            vec!["@repo/*"],
            vec!["./apps/*"],
            vec!["!web"],
            vec!["web", "!docs"],
            vec!["!{**/docs/**}"],
        ] {
            assert!(
                !selectors_require_complete_graph(&selectors(&patterns)),
                "{patterns:?} must narrow against an inventory graph"
            );
        }
    }

    #[test]
    fn graph_dependent_selectors_require_a_complete_graph() {
        for patterns in [
            vec!["web..."],
            vec!["...web"],
            vec!["web^..."],
            vec!["web...[main]"],
            vec!["web", "...docs"],
        ] {
            assert!(
                selectors_require_complete_graph(&selectors(&patterns)),
                "{patterns:?} depends on edges or git history and cannot narrow"
            );
        }
    }
}

#[cfg(test)]
mod remote_cache_status_tests {
    use std::{future::Future, sync::Mutex};

    use turborepo_run_opts::RemoteCacheDisabledReason;
    use turborepo_types::SecretString;
    use turborepo_vercel_api::{CachingStatus, CachingStatusResponse};

    use super::{
        CacheStatusProbe, RemoteCacheStatus, RemoteCacheUnavailableReason,
        resolve_remote_cache_status, start_remote_cache_preflight,
    };

    struct StubCacheStatusProbe(Mutex<Option<turborepo_api_client::Result<CachingStatusResponse>>>);

    impl CacheStatusProbe for StubCacheStatusProbe {
        fn check_caching_status(
            &self,
            _token: &SecretString,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> impl Future<Output = turborepo_api_client::Result<CachingStatusResponse>> + Send
        {
            let result = self.0.lock().unwrap().take().unwrap();
            async move { result }
        }
    }

    async fn resolve_status(
        result: turborepo_api_client::Result<CachingStatusResponse>,
    ) -> RemoteCacheStatus {
        let probe = StubCacheStatusProbe(Mutex::new(Some(result)));
        let handle = start_remote_cache_preflight(
            probe,
            SecretString::new("test-token".to_string()),
            None,
            None,
        );
        resolve_remote_cache_status(None, Some(handle)).await
    }

    fn response(status: CachingStatus) -> CachingStatusResponse {
        CachingStatusResponse { status }
    }

    #[tokio::test]
    async fn preserves_local_disabled_reason_without_a_preflight() {
        let status =
            resolve_remote_cache_status(Some(RemoteCacheDisabledReason::ByFlags), None).await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Disabled(RemoteCacheDisabledReason::ByFlags)
        ));
    }

    #[tokio::test]
    async fn reports_could_not_connect() {
        let status = resolve_status(Err(turborepo_api_client::Error::HttpClientCancelled)).await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::CouldNotConnect)
        ));
    }

    #[tokio::test]
    async fn reports_usage_limit_exceeded() {
        let status = resolve_status(Ok(response(CachingStatus::OverLimit))).await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::UsageLimitExceeded)
        ));
    }

    #[tokio::test]
    async fn reports_spending_paused() {
        let status = resolve_status(Ok(response(CachingStatus::Paused))).await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::SpendingPaused)
        ));
    }

    #[tokio::test]
    async fn reports_disabled_for_team() {
        let status = resolve_status(Ok(response(CachingStatus::Disabled))).await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::DisabledForTeam)
        ));
    }

    #[tokio::test]
    async fn reports_authentication_failed() {
        let status = resolve_status(Err(turborepo_api_client::Error::InvalidToken {
            status: 401,
            url: "https://example.com/v8/artifacts/status".to_string(),
            message: "unauthorized".to_string(),
        }))
        .await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::AuthenticationFailed)
        ));
    }

    #[tokio::test]
    async fn reports_unexpected_server_error() {
        let status = resolve_status(Err(turborepo_api_client::Error::UnknownCachingStatus(
            "unknown".to_string(),
            std::backtrace::Backtrace::capture(),
        )))
        .await;
        assert!(matches!(
            status,
            RemoteCacheStatus::Unavailable(RemoteCacheUnavailableReason::UnexpectedServerError)
        ));
    }
}
