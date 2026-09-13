use std::{
    collections::{HashMap, HashSet},
    io::{ErrorKind, IsTerminal},
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::Local;
use tracing::Instrument;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, RelativeUnixPathBuf,
};
use turborepo_analytics::{start_analytics, AnalyticsHandle};
use turborepo_api_client::{APIAuth, APIClient, CacheClient, SharedHttpClient};
use turborepo_cache::{AsyncCache, CacheScmState, LazyScmState};
use turborepo_env::EnvironmentVariableMap;
use turborepo_errors::Spanned;
use turborepo_process::ProcessManager;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    package_graph::{PackageGraph, PackageName, TaskEntrypointPreference},
    package_json,
    toolchain::{PlanningUncertaintyKind, ToolchainId},
};
use turborepo_run_summary::observability;
use turborepo_scm::SCM;
use turborepo_scope::{filter::ResolutionError, TargetSelector};
use turborepo_shim::TurboState;
use turborepo_signals::SignalHandler;
use turborepo_task_id::{TaskId, TaskName};
use turborepo_telemetry::events::{
    command::CommandEventBuilder,
    generic::{DaemonInitStatus, GenericEventBuilder},
    repo::{RepoEventBuilder, RepoType},
    EventBuilder, TrackedErrors,
};
use turborepo_types::{FilterMode, TaskDefinitionHashInfo, TaskInputs, UIMode};
use turborepo_ui::ColorConfig;
use turborepo_vercel_api::CachingStatusResponse;
use url::Url;

type FilteredPackages = (
    HashMap<PackageName, PackageInclusionReason>,
    FilterMode,
    HashSet<PackageName>,
);

/// Selection-side inputs for [`RunBuilder::refuse_unproven_selection`].
///
/// Everything needed to prove a selection independent of a contributor's
/// unresolved planning facts, with no knowledge of which language any
/// toolchain is: which toolchains preparation will resolve, the resolved
/// package selection, which task names are in play, and whether the
/// selection expanded dependents or dependencies.
struct UnresolvedPlanningContext<'a> {
    /// Toolchains whose observations preparation will replace for this run.
    resolving_toolchains: &'a HashSet<ToolchainId>,
    /// Packages resolved by scope resolution. Together with the engine's
    /// retained task packages this forms the consulted set: the traversal
    /// closure and the catalogue domain.
    filtered_packages: &'a HashMap<PackageName, PackageInclusionReason>,
    /// Task names the run requests for every consulted catalogue: unqualified
    /// task arguments only. Qualified arguments (`js#dev`) name a task for
    /// one package; their scopes are captured by the engine's task ids.
    /// `--filter` does not support `pkg#task` syntax.
    requested_task_names: &'a HashSet<String>,
    /// Whether the selection expanded dependents: `--affected`, `...pkg`
    /// selectors, `pkg...[range]` match-dependencies, or a watch rerun's
    /// affectedness. Dependents completeness depends on edges pointing into
    /// the selected set.
    dependents_direction: bool,
    /// Whether the selection expanded dependencies: `pkg...`,
    /// `pkg^...`, or match-dependencies selectors. The dependency closure
    /// is computed through each member's outgoing edges, and `filtered_pkgs`
    /// is frozen from the planning graph — preparation re-collects task
    /// edges but cannot repair package-level selection.
    dependencies_direction: bool,
}

#[derive(Default)]
struct TaskEntrypointSelection {
    candidates: HashSet<TaskId<'static>>,
    selected: HashSet<TaskId<'static>>,
    excluded: HashSet<TaskId<'static>>,
    orchestration: HashMap<String, HashSet<TaskId<'static>>>,
}

use crate::{
    commands::CommandBase,
    engine::{task_has_command, Engine, EngineBuilder, EngineExt, TaskNode},
    microfrontends::MicrofrontendsConfigs,
    opts::Opts,
    repository_graph::RepositoryGraphFeatures,
    run::{
        scope, task_access::TaskAccess, Error, RemoteCacheStatus, RemoteCacheUnavailableReason,
        Run, RunCache,
    },
    turbo_json::{TurboJson, TurboJsonReader, UnifiedTurboJsonLoader},
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

    #[tracing::instrument(skip_all)]
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
        engine: &Engine,
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

    /// The toolchains that own at least one finally-participating task.
    ///
    /// Selection is drawn only from tasks that will actually execute — a task
    /// with a resolved command, honoring `command` overrides and excluding
    /// commandless transit/opt-out nodes. Package dependency ancestors that
    /// never execute are deliberately not considered, so an unselected
    /// toolchain is never prepared.
    fn participating_toolchains(
        pkg_dep_graph: &PackageGraph,
        engine: &Engine,
    ) -> HashSet<ToolchainId> {
        let mut selection = HashSet::new();
        for node in engine.tasks() {
            let TaskNode::Task(task) = node else {
                continue;
            };
            if !task_has_command(engine, pkg_dep_graph, task) {
                continue;
            }
            if let Some(toolchain) =
                pkg_dep_graph.package_toolchain(&PackageName::from(task.package()))
            {
                selection.insert(toolchain.clone());
            }
        }
        selection
    }

    /// Retain exactly the finalized task set in a rebuilt (prepared) engine,
    /// re-expanding `with` siblings and dependency edges from the rebuilt
    /// graph.
    ///
    /// Refuses if a finalized task no longer exists: static planning topology
    /// that diverges from eager topology would otherwise silently drop a
    /// dependency.
    fn retain_prepared_tasks(
        engine: Engine,
        finalized: &HashSet<TaskId<'static>>,
    ) -> Result<Engine, Error> {
        for task in finalized {
            if engine.task_definition(task).is_none() {
                return Err(Error::StagedTaskTopologyMismatch {
                    task: task.to_string(),
                });
            }
        }
        let expanded = super::task_filter::expand_with_siblings(&engine, finalized.clone());
        let mut retained = expanded.clone();
        retained.extend(engine.collect_task_dependencies(&expanded));
        Ok(engine.retain_task_subset(&retained))
    }

    /// Refuse a prepared engine that would execute a task in a toolchain left
    /// unprepared. Such a task's hash-relevant contracts are only the static
    /// planning observation, so executing it would hash stale inputs.
    fn refuse_unprepared_toolchains(
        pkg_dep_graph: &PackageGraph,
        engine: &Engine,
        unprepared: &HashSet<ToolchainId>,
    ) -> Result<(), Error> {
        if unprepared.is_empty() {
            return Ok(());
        }
        for node in engine.tasks() {
            let TaskNode::Task(task) = node else {
                continue;
            };
            if !task_has_command(engine, pkg_dep_graph, task) {
                continue;
            }
            if let Some(toolchain) =
                pkg_dep_graph.package_toolchain(&PackageName::from(task.package()))
            {
                if unprepared.contains(toolchain) {
                    return Err(Error::StagedUnpreparedToolchain {
                        toolchain: toolchain.to_string(),
                        task: task.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Refuse a selection that cannot be proven exact against a contributor's
    /// unresolved planning facts, before any preparation (and therefore
    /// before any native invocation) is allowed to resolve them.
    ///
    /// Planning uncertainty is deliberately ignored for selections that
    /// provably never consult it: a JavaScript-only run never invokes an
    /// unrelated toolchain. Four precise conditions block a run, each
    /// refusing with a diagnostic naming the scope:
    ///
    /// 1. A retained task in an edge-uncertain scope of a toolchain whose facts
    ///    will remain static: the task's dependency structure is hashed into
    ///    every dependent, and nothing will ever replace the partial facts. A
    ///    selected toolchain that *will* be prepared is exempt: the prepared
    ///    rebuild re-collects its dependency closure.
    /// 2. A catalogue-uncertain scope inside the consulted set — the resolved
    ///    filtered packages plus the engine's retained task packages, phantoms
    ///    included — whose unresolved task names are in play. Without a
    ///    selected owner nothing can ever resolve the catalogue. With a
    ///    selected owner, preparation is allowed only when every relevant
    ///    uncertain task is already retained as a real command: membership is
    ///    then proven and preparation only resolves shape, never inventing
    ///    membership. A relevant task that is absent or only a phantom is
    ///    possibly absent — the frozen selection cannot gain it after
    ///    preparation.
    /// 3. A dependents-direction selection whose closure an edge-uncertain
    ///    scope could extend: a missing dependent cannot be recovered after
    ///    preparation refreezes the finalized task set, so this also blocks
    ///    prepared toolchains.
    /// 4. A dependencies-direction closure that traverses an edge-uncertain
    ///    scope: `filtered_pkgs` is frozen from the planning graph, and
    ///    preparation re-collects task edges but cannot repair package-level
    ///    selection. The closure is provable only when every candidate the
    ///    unknown edges could add is already a closure member.
    fn refuse_unproven_selection(
        pkg_dep_graph: &PackageGraph,
        engine: &Engine,
        context: &UnresolvedPlanningContext<'_>,
    ) -> Result<(), Error> {
        let engine_task_packages: HashSet<PackageName> = engine
            .tasks()
            .filter_map(|node| match node {
                TaskNode::Task(task) => Some(PackageName::from(task.package())),
                TaskNode::Root => None,
            })
            .collect();
        // Real commands the engine retains, per scope: contributed tasks
        // with a resolved command. Membership of these tasks is proven even
        // when the rest of the scope's catalogue is not.
        let mut commanded_tasks: HashMap<&str, HashSet<&str>> = HashMap::new();
        for node in engine.tasks() {
            if let TaskNode::Task(task) = node {
                if task_has_command(engine, pkg_dep_graph, task) {
                    commanded_tasks
                        .entry(task.package())
                        .or_default()
                        .insert(task.task());
                }
            }
        }
        // Packages the selection actually consults: the resolved filtered
        // set plus every package the final engine retains, phantoms included.
        // This is both the traversal closure (unknown edges into/out of these
        // could extend it) and the catalogue domain (these are the scopes
        // whose task catalogues the run's selection can consult) — derived
        // from the actual selection, never from raw filter patterns or mode
        // flags, so narrowed, excluded, and affected-only selections do not
        // consult unrelated scopes while config-wired phantoms stay in scope.
        let mut consulted_packages: HashSet<&PackageName> =
            context.filtered_packages.keys().collect();
        consulted_packages.extend(engine_task_packages.iter());

        for (toolchain, uncertainty) in pkg_dep_graph.planning_uncertainties() {
            let scope = PackageName::from(uncertainty.scope());
            match uncertainty.kind() {
                PlanningUncertaintyKind::InternalEdges => {
                    // (1) A retained task whose hashed dependency structure
                    // will never be replaced with exact facts.
                    if !context.resolving_toolchains.contains(toolchain) {
                        let retained = engine.tasks().find_map(|node| match node {
                            TaskNode::Task(task) => {
                                (task.package() == uncertainty.scope()).then_some(task)
                            }
                            TaskNode::Root => None,
                        });
                        // `turborepo-lib` is edition 2021: no let-chains here.
                        if let Some(task) = retained {
                            return Err(Error::UnresolvedPlanningFact {
                                toolchain: toolchain.to_string(),
                                fact: "internal dependency edges".to_string(),
                                package: uncertainty.scope().to_string(),
                                code: uncertainty.code().to_string(),
                                detail: uncertainty.message().to_string(),
                                reason: format!(
                                    "task `{task}` is retained by this run, so its dependency \
                                     structure — hashed into every dependent — cannot be proven \
                                     without the `{toolchain}` toolchain, whose facts this run \
                                     never resolves"
                                ),
                            });
                        }
                    }
                    // (3) A dependents-direction closure the unknown edges
                    // could extend.
                    if context.dependents_direction {
                        let unbounded_or_touching =
                            uncertainty.possible_targets().is_none_or(|targets| {
                                targets.iter().any(|target| {
                                    consulted_packages.contains(&PackageName::from(target.as_str()))
                                })
                            });
                        if unbounded_or_touching {
                            return Err(Error::UnresolvedPlanningFact {
                                toolchain: toolchain.to_string(),
                                fact: "internal dependency edges".to_string(),
                                package: uncertainty.scope().to_string(),
                                code: uncertainty.code().to_string(),
                                detail: uncertainty.message().to_string(),
                                reason: format!(
                                    "this run expands dependents, and the dependents of the \
                                     selection cannot be proven complete while `{scope}`'s edges \
                                     toward it are unresolved"
                                ),
                            });
                        }
                    }
                    // (4) A dependencies-direction closure computed through
                    // this scope's own unknown outgoing edges. Preparation
                    // cannot repair the frozen package selection, so the
                    // closure is provable only when every candidate the
                    // unknown edges could add is already a closure member.
                    if context.dependencies_direction && consulted_packages.contains(&scope) {
                        let provably_closed =
                            uncertainty.possible_targets().is_some_and(|targets| {
                                targets.iter().all(|target| {
                                    consulted_packages.contains(&PackageName::from(target.as_str()))
                                })
                            });
                        if !provably_closed {
                            return Err(Error::UnresolvedPlanningFact {
                                toolchain: toolchain.to_string(),
                                fact: "internal dependency edges".to_string(),
                                package: uncertainty.scope().to_string(),
                                code: uncertainty.code().to_string(),
                                detail: uncertainty.message().to_string(),
                                reason: format!(
                                    "this run expands dependencies through `{scope}`, whose \
                                     unresolved edges could add packages to the selection that \
                                     preparation cannot recover"
                                ),
                            });
                        }
                    }
                }
                PlanningUncertaintyKind::TaskCatalogue => {
                    // (2) Whether this scope participates in the run's task
                    // set can be proven, or preparation resolves it. The
                    // domain is the actual consulted set: resolved filtered
                    // packages plus engine-retained packages (phantoms
                    // included), never a raw pattern union.
                    if !consulted_packages.contains(&scope) {
                        continue;
                    }
                    // Task names this run's selection consults *for this
                    // scope*: the requested names (which apply to every
                    // in-domain scope) plus every task the engine already
                    // retains for the scope — phantoms included, because a
                    // config-wired phantom's reality matters to the run.
                    let mut in_play: HashSet<&str> = context
                        .requested_task_names
                        .iter()
                        .map(String::as_str)
                        .collect();
                    in_play.extend(engine.tasks().filter_map(|node| match node {
                        TaskNode::Task(task) => {
                            (task.package() == uncertainty.scope()).then_some(task.task())
                        }
                        TaskNode::Root => None,
                    }));
                    // Relevant uncertain names: the whole in-play set when
                    // the record is unnarrowed, otherwise the uncertain names
                    // that are in play. Names not in play are ignored — an
                    // uncertain task this run never selects cannot change
                    // this run's selection.
                    let relevant: Vec<&str> = if uncertainty.uncertain_task_names().is_empty() {
                        in_play.into_iter().collect()
                    } else {
                        uncertainty
                            .uncertain_task_names()
                            .iter()
                            .map(String::as_str)
                            .filter(|&name| in_play.contains(name))
                            .collect()
                    };
                    if relevant.is_empty() {
                        continue;
                    }
                    if !context.resolving_toolchains.contains(toolchain) {
                        // No selected owner: nothing will ever resolve the
                        // catalogue for this run.
                        return Err(Error::UnresolvedPlanningFact {
                            toolchain: toolchain.to_string(),
                            fact: "the task catalogue".to_string(),
                            package: uncertainty.scope().to_string(),
                            code: uncertainty.code().to_string(),
                            detail: uncertainty.message().to_string(),
                            reason: format!(
                                "whether `{scope}` participates in this run's task set cannot be \
                                 proven, and this run never selects the `{toolchain}` toolchain \
                                 to resolve it"
                            ),
                        });
                    }
                    // Selected owner: preparation replaces the observation.
                    // Allow it only when every relevant uncertain task is
                    // already retained as a real command — membership is
                    // proven and preparation only resolves shape, never
                    // inventing membership. A relevant task that is absent
                    // or only a phantom is possibly absent: the frozen
                    // selection cannot gain it after preparation.
                    let unproven: Vec<&str> = relevant
                        .into_iter()
                        .filter(|&name| {
                            !commanded_tasks
                                .get(uncertainty.scope())
                                .is_some_and(|tasks| tasks.contains(name))
                        })
                        .collect();
                    if !unproven.is_empty() {
                        return Err(Error::UnresolvedPlanningFact {
                            toolchain: toolchain.to_string(),
                            fact: "the task catalogue".to_string(),
                            package: uncertainty.scope().to_string(),
                            code: uncertainty.code().to_string(),
                            detail: uncertainty.message().to_string(),
                            reason: format!(
                                "the {} task{} for `{scope}` {} possibly absent: not retained as \
                                 a real command during planning, and preparation cannot add a \
                                 task the frozen selection never included",
                                unproven.join(", "),
                                if unproven.len() == 1 { "" } else { "s" },
                                if unproven.len() == 1 { "is" } else { "are" },
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Parse the run's filter patterns once for the unresolved-planning
    /// check. Patterns that fail to parse here already failed scope
    /// resolution; skipping them cannot bypass a refusal.
    fn parse_filter_selectors(patterns: &[String]) -> Vec<TargetSelector> {
        patterns
            .iter()
            .filter_map(|pattern| pattern.parse::<TargetSelector>().ok())
            .collect()
    }

    /// Whether this run's selection expanded dependents: `--affected`
    /// (package- or task-level), a watch rerun's affectedness, or any
    /// `...pkg` selector — including excludes, whose completeness depends on
    /// the same edges.
    fn selection_expands_dependents(&self, selectors: &[TargetSelector]) -> bool {
        self.opts.scope_opts.affected_range.is_some()
            || self.changed_files_for_watch.is_some()
            || selector_expands_dependents(selectors)
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
        let (mut filtered_pkgs, filter_mode) = scope::resolve_packages(
            &opts.scope_opts,
            repo_root,
            pkg_dep_graph,
            scm,
            root_turbo_json,
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

    /// Whether a `filterUsingTasks` run can resolve its task scope from the
    /// package-level filter instead of constructing a repository-wide task
    /// engine and pruning it after the fact.
    ///
    /// With `--only`, the engine is exactly `{package x requested task}` plus
    /// `with` siblings, so a selector that only names packages selects the
    /// same tasks whether the engine is built for every workspace or only for
    /// the packages the filter resolves to. Every condition below is required
    /// for that equivalence to hold; anything else keeps the general
    /// full-graph path.
    fn task_filter_can_use_package_scope(
        &self,
        pkg_dep_graph: &PackageGraph,
        turbo_json_loader: &impl turborepo_engine::TurboJsonLoader,
    ) -> bool {
        // `--only` prunes the engine to {package x requested task}, which is
        // what makes the package-scoped construction equivalent.
        if !self.opts.run_opts.only {
            return false;
        }
        // Affected selectors, watch reruns, all-tasks graphs, and package
        // inference all require the repository-wide engine.
        if self.opts.scope_opts.affected_range.is_some()
            || self.changed_files_for_watch.is_some()
            || self.add_all_tasks
            || self.opts.scope_opts.pkg_inference_root.is_some()
        {
            return false;
        }
        // Strict entrypoint selection consults command participation across
        // the whole engine, which the scoped engine cannot answer.
        if self.opts.future_flags.strict_task_entrypoint_selection {
            return false;
        }
        // Only plain package-name selectors (optionally excluded) resolve
        // identically at the package and task level. Directory selectors, git
        // ranges, and dependency/dependent expansion have task-level
        // semantics.
        if !self.opts.scope_opts.filter_patterns.iter().all(|pattern| {
            pattern
                .parse::<turborepo_scope::TargetSelector>()
                .map(|selector| selector_selects_only_package_names(&selector))
                .unwrap_or(false)
        }) {
            return false;
        }
        // Mixing `pkg#task` arguments with unqualified task arguments lets the
        // scoped engine pick up tasks the task-level filter would prune.
        let task_names: Vec<TaskName> = self
            .opts
            .run_opts
            .tasks
            .iter()
            .map(|task| TaskName::from(task.as_str()))
            .collect();
        let qualified = task_names
            .iter()
            .filter(|task| task.package().is_some())
            .count();
        if qualified != 0 && qualified != task_names.len() {
            return false;
        }
        // Native task contracts change entrypoint eligibility per package.
        if pkg_dep_graph
            .package_task_contexts()
            .any(|context| context.task_contract().task_entrypoint_domain().is_some())
        {
            return false;
        }
        // `with` siblings can pull tasks of other packages into the engine.
        repo_configs_have_no_with_declarations(pkg_dep_graph, turbo_json_loader)
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
        // it. A shared graph may also be a staged planning graph whose
        // hash-relevant facts were never prepared for this run's task selection;
        // such a graph carries no plan to prepare them. Reuse only graphs that
        // are fully prepared — a generic capability of the graph, with no
        // per-toolchain knowledge here.
        let shared_is_reusable = self
            .shared_pkg_graph
            .as_ref()
            .is_some_and(|graph| graph.is_fully_prepared());
        let shared_pkg_graph = if self.opts.run_opts.parallel {
            None
        } else if self.shared_pkg_graph.is_some() && !shared_is_reusable {
            tracing::debug!(
                "bypassing shared package graph: staged replanning required for the selected tasks"
            );
            None
        } else {
            self.shared_pkg_graph.clone()
        };
        let mut staged_plan = None;
        let mut pkg_dep_graph: Arc<PackageGraph> = match shared_pkg_graph {
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
                    .build_staged()
                    .instrument(tracing::info_span!("pkg_dep_graph_build"))
                    .await;

                match graph {
                    Ok(graph) => {
                        // Take unique ownership of the planning graph so
                        // `--parallel` can mutate it; the plan owns none of it.
                        let (planning, plan) = graph.into_parts();
                        staged_plan = Some(plan);
                        planning
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
                Some(tokio::spawn(
                    async move {
                        client
                            .get_caching_status(&token, team_id.as_deref(), team_slug.as_deref())
                            .await
                    }
                    .instrument(tracing::info_span!("remote_cache_preflight")),
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

        let root_turbo_json_path = self.opts.repo_opts.root_turbo_json_path.clone();
        let future_flags = self.opts.future_flags;
        let root_native_tasks = pkg_dep_graph
            .package_task_context(&PackageName::Root)
            .map(|context| context.native_tasks());
        let task_access_enabled = root_package_json.is_some()
            && root_native_tasks
                .is_some_and(|tasks| TaskAccess::check_enabled(&self.repo_root, tasks));

        let reader = TurboJsonReader::new(self.repo_root.clone()).with_future_flags(future_flags);

        // The loader captures only topology-stable inputs from the planning
        // graph — scope directories and task-catalogue names — never task
        // contracts. Hash-relevant contracts reach the engine from the graph
        // passed to `build_engine`, which is the prepared graph below, so the
        // engine never retains static-planning I/O.
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
        let (mut filtered_pkgs, mut filter_mode, unqualified_entrypoint_packages) = {
            let _span = tracing::info_span!("calculate_filtered_packages").entered();
            Self::calculate_filtered_packages(
                &self.repo_root,
                package_resolution_opts,
                &pkg_dep_graph,
                &scm,
                &root_turbo_json,
            )?
        };
        if use_task_level_affected {
            filter_mode = FilterMode::ExplicitSelection;
        }
        // The root Turbo task namespace exists independently of a root
        // JavaScript package scope. Non-root namespaces, including aggregate
        // scopes, come from authoritative repository knowledge.
        let task_namespace_packages: Vec<_> = std::iter::once(PackageName::Root)
            .chain(
                pkg_dep_graph
                    .package_scope_directories()
                    .map(|(name, _)| name)
                    .filter(|name| name != &PackageName::Root),
            )
            .collect();
        let mut scoped_entrypoint_exclusions = self.task_entrypoint_exclusions(
            &pkg_dep_graph,
            unqualified_entrypoint_packages.iter(),
            task_namespace_packages.iter(),
            &filter_mode,
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

        let task_level_affected_package_scope = if has_task_level_affected_package_scope {
            Some(filtered_pkgs.keys().cloned().collect())
        } else {
            None
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
        let entrypoint_exclusions = if needs_all_packages {
            HashSet::new()
        } else {
            scoped_entrypoint_exclusions
        };

        // Config preloading overlaps engine construction. Repository-wide
        // engines consult every package's config, but scoped engines only
        // consult the filtered packages and the dependency closure their
        // `^task` edges follow, so narrow runs skip preloading unrelated
        // packages and let the engine load anything else lazily.
        crate::rayon_compat::block_in_place(|| {
            let _span = tracing::info_span!("turbo_json_preload").entered();
            if needs_all_packages {
                turbo_json_loader.preload_all();
            } else {
                let packages = Self::scoped_preload_packages(&pkg_dep_graph, filtered_pkgs.keys());
                turbo_json_loader.preload_packages(packages);
            }
        });

        // When task-level filtering or add_all_tasks is active, the engine must
        // contain tasks for ALL packages so that tasks in packages not flagged
        // by package-level scope resolution can still be matched. The
        // task-level filter (below) does the pruning when needed.
        let all_pkgs: Vec<PackageName> = if needs_all_packages {
            task_namespace_packages
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
            &entrypoint_exclusions,
            &turbo_json_loader,
            &env_at_execution_start,
        )?;

        let task_access = {
            let _span = tracing::info_span!("task_access_setup").entered();
            let ta = TaskAccess::new(
                self.repo_root.clone(),
                async_cache.clone(),
                &scm,
                task_access_enabled,
            );
            ta.restore_config().await;
            ta
        };

        // --parallel removes inter-package dependencies from the package graph,
        // requiring a fresh engine build. Affected filtering runs once afterward
        // rather than on both engines to avoid a redundant SCM query.
        if self.opts.run_opts.parallel {
            // A --parallel run never reuses a shared package graph (the
            // sharing path above opts out for parallel), so this Arc is
            // uniquely owned here.
            let Some(graph) = Arc::get_mut(&mut pkg_dep_graph) else {
                unreachable!("--parallel runs never reuse a shared package graph");
            };
            graph.remove_package_dependencies();
            let engine_pkgs: Box<dyn Iterator<Item = &PackageName>> = if needs_all_packages {
                Box::new(all_pkgs.iter())
            } else {
                Box::new(filtered_pkgs.keys())
            };
            engine = self.build_engine(
                &pkg_dep_graph,
                &root_turbo_json,
                engine_pkgs,
                &entrypoint_exclusions,
                &turbo_json_loader,
                &env_at_execution_start,
            )?;
        }

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
                    Some(super::task_filter::resolve_affected_tasks(
                        &engine,
                        affected_range,
                        &pkg_dep_graph,
                        &scm,
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
            engine = super::task_filter::filter_engine_to_tasks_with_inclusions(
                engine,
                &selectors,
                super::task_filter::TaskFilterConstraints {
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
                &scm,
                &self.repo_root,
                &root_turbo_json.global_deps,
            )?;
        }

        // Task-level --affected detection (separate from --filter).
        if use_task_level_affected {
            let (affected_engine, selected_packages) = self.filter_engine_to_affected_tasks(
                engine,
                &pkg_dep_graph,
                &root_turbo_json,
                &scm,
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
            engine = super::task_filter::retain_strict_task_graph(
                engine,
                &pkg_dep_graph,
                task_entrypoints.selected,
                &task_entrypoints.orchestration,
            );
        }

        // Toolchains that own a finally-participating task are fully
        // discovered here, after selection is final and before any hashing.
        // "Participating" means the task will actually execute, honoring
        // command overrides and excluding commandless transit/opt-out nodes;
        // unexecuted package dependencies never trigger a toolchain. Toolchain
        // identity comes from the graph, so no language is special-cased.
        if let Some(staged) = staged_plan.take() {
            let selection = Self::participating_toolchains(&pkg_dep_graph, &engine);
            // Refuse selections that cannot be proven exact against
            // unresolved planning facts — before any preparation, and
            // therefore before any native invocation, is allowed to resolve
            // them. Selections that provably never consult those facts (an
            // unrelated JavaScript-only run) ignore them here.
            {
                let selectors = Self::parse_filter_selectors(&self.opts.scope_opts.filter_patterns);
                let resolving_toolchains = staged.resolving_toolchains(&selection);
                // Task names this run puts in play for every consulted
                // catalogue: unqualified task arguments only. A qualified
                // argument (`js#dev`) names a task for one package; its scope
                // is captured by the engine's retained task ids instead.
                // `--filter` has no `pkg#task` syntax.
                let task_names_in_play: HashSet<String> = self
                    .opts
                    .run_opts
                    .tasks
                    .iter()
                    .map(|task| TaskName::from(task.as_str()))
                    .filter(|task| task.package().is_none())
                    .map(|task| task.task().to_string())
                    .collect();
                Self::refuse_unproven_selection(
                    &pkg_dep_graph,
                    &engine,
                    &UnresolvedPlanningContext {
                        resolving_toolchains: &resolving_toolchains,
                        filtered_packages: &filtered_pkgs,
                        requested_task_names: &task_names_in_play,
                        dependents_direction: self.selection_expands_dependents(&selectors),
                        dependencies_direction: selector_expands_dependencies(&selectors),
                    },
                )?;
            }
            if staged.requires_preparation(&selection) {
                // Toolchains that will remain static even after this
                // preparation. A retained task in one of these must not
                // execute with stale contracts.
                let unprepared = staged.unprepared_toolchains(&selection);
                // The exact final task set, retained so the prepared rebuild
                // cannot undo task-level/strict/affected pruning.
                let finalized_tasks: HashSet<TaskId<'static>> = engine
                    .tasks()
                    .filter_map(|node| match node {
                        TaskNode::Task(task) => Some(task.clone()),
                        TaskNode::Root => None,
                    })
                    .collect();
                pkg_dep_graph = staged.prepare(&selection).await?;
                // --parallel removed inter-package dependencies from the
                // planning graph; reapply after preparation.
                if self.opts.run_opts.parallel {
                    let Some(graph) = Arc::get_mut(&mut pkg_dep_graph) else {
                        unreachable!("--parallel runs never reuse a shared package graph");
                    };
                    graph.remove_package_dependencies();
                }
                let engine_pkgs: Box<dyn Iterator<Item = &PackageName>> = if needs_all_packages {
                    Box::new(all_pkgs.iter())
                } else {
                    Box::new(filtered_pkgs.keys())
                };
                engine = self.build_engine(
                    &pkg_dep_graph,
                    &root_turbo_json,
                    engine_pkgs,
                    &entrypoint_exclusions,
                    &turbo_json_loader,
                    &env_at_execution_start,
                )?;
                engine = Self::retain_prepared_tasks(engine, &finalized_tasks)?;
                // The rebuilt graph may reach a dependency in a toolchain that
                // was not prepared here (eager topology differs from planning).
                // Refuse rather than execute it with stale contracts.
                Self::refuse_unprepared_toolchains(&pkg_dep_graph, &engine, &unprepared)?;
            }
        }

        // The engine is final: every task the run will hash is known. Send
        // the untracked scan its scope. Provably package-local runs walk
        // only the participating packages' directories (plus the root
        // package's internal dependencies); everything else keeps today's
        // whole-repo scan.
        if let Some(scope_tx) = untracked_scan_scope_tx {
            let scoped_prefixes = Self::untracked_scan_prefixes(
                &self.repo_root,
                scm.git_root(),
                &engine,
                &pkg_dep_graph,
                &root_turbo_json,
                &filter_mode,
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
                    &pkg_dep_graph,
                    self.opts.run_opts.concurrency,
                    self.opts.run_opts.ui_mode,
                    self.will_execute_tasks(),
                )
                .map_err(Error::EngineValidation)?;
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
        // The untracked-file scan keeps running in the background;
        // `execute_visitor` awaits it right before file hashing. Deferring
        // the barrier lets everything between `Run` construction and
        // hashing overlap the scan.
        let repo_index = crate::run::PendingRepoIndex::new(repo_index_task);

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
                api_client,
                env_at_execution_start,
                filtered_pkgs: filtered_pkgs.keys().cloned().collect(),
                pkg_dep_graph,
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
                shutdown_started_emitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        package_scope: Option<&HashSet<PackageName>>,
    ) -> Result<(Engine, Option<HashSet<PackageName>>), Error> {
        let (from_ref, to_ref) = self
            .opts
            .scope_opts
            .affected_range
            .as_ref()
            .ok_or(Error::MissingAffectedRange)?;
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
                    super::task_filter::expand_with_siblings(&engine, affected_entrypoints);
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
                let scoped_tasks = super::task_filter::expand_with_siblings(&engine, scoped_tasks);
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
        engine: &Engine,
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
                        && crate::engine::task_participates(engine, pkg_dep_graph, task_id)
                });

            for package in candidate_packages {
                let task_id = TaskId::new(package.as_ref(), task.task()).into_owned();
                if engine.task_definition(&task_id).is_none() {
                    continue;
                }

                selection.candidates.insert(task_id.clone());
                if !has_participant
                    || crate::engine::task_participates(engine, pkg_dep_graph, &task_id)
                {
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
        engine: Engine,
        pkg_dep_graph: &PackageGraph,
        filter_mode: &FilterMode,
    ) -> Engine {
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
        engine.remove_tasks(&exclusions)
    }

    #[tracing::instrument(skip_all)]
    fn build_engine<'a>(
        &self,
        pkg_dep_graph: &PackageGraph,
        root_turbo_json: &TurboJson,
        filtered_pkgs: impl Iterator<Item = &'a PackageName>,
        entrypoint_exclusions: &HashSet<TaskId<'static>>,
        turbo_json_loader: &impl turborepo_engine::TurboJsonLoader,
        environment: &EnvironmentVariableMap,
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

        let mut engine = builder.build()?;

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
                let filter = crate::task_change_detector::resolve_watch_task_filter(
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
        if !watch_task_filtered {
            if let Some(entrypoint_packages) = &self.entrypoint_packages {
                engine = engine.create_engine_for_subgraph(entrypoint_packages);
            }
        }

        Ok(engine)
    }
}

/// Whether a selector's semantics are purely package-level: it names
/// packages (by name pattern, possibly for exclusion) and does not use
/// directory selectors, git ranges, or dependency/dependent expansion.
fn selector_selects_only_package_names(selector: &turborepo_scope::TargetSelector) -> bool {
    !selector.name_pattern.is_empty()
        && selector.parent_dir.is_none()
        && selector.git_range.is_none()
        && !selector.include_dependencies
        && !selector.include_dependents
        && !selector.exclude_self
        && !selector.match_dependencies
        && !selector.follow_prod_deps_only
}

/// Whether any selector traverses edges *into* the selected set:
/// `...pkg` dependents expansion, or `pkg...[range]` match-dependencies
/// (which selects dependents of changed packages — a reverse traversal,
/// despite the name).
fn selector_expands_dependents(selectors: &[TargetSelector]) -> bool {
    selectors
        .iter()
        .any(|selector| selector.include_dependents || selector.match_dependencies)
}

/// Whether any selector traverses the selected set's *outgoing* edges:
/// `pkg...` dependency expansion. Match-dependencies is excluded: it
/// reverse-traverses from changed packages and never follows a closure
/// member's own dependencies.
fn selector_expands_dependencies(selectors: &[TargetSelector]) -> bool {
    selectors
        .iter()
        .any(|selector| selector.include_dependencies)
}

/// Whether every turbo.json in the repository loads successfully and declares
/// no `with` siblings. Configs are preloaded by this point, so this is an
/// in-memory scan, and it only runs for otherwise eligible runs.
fn repo_configs_have_no_with_declarations(
    pkg_dep_graph: &PackageGraph,
    turbo_json_loader: &impl turborepo_engine::TurboJsonLoader,
) -> bool {
    let packages = std::iter::once(PackageName::Root).chain(
        pkg_dep_graph
            .package_scope_directories()
            .map(|(name, _)| name),
    );
    for package in packages {
        match turbo_json_loader.load(&package) {
            // Workspaces without a turbo.json fall back to the root chain.
            Err(err) if err.is_no_turbo_json() => continue,
            // A config that fails to load surfaces as an error while building
            // the repository-wide engine; keep that behavior.
            Err(_) => return false,
            Ok(turbo_json) => {
                if turbo_json.tasks.values().any(|def| def.with.is_some()) {
                    return false;
                }
            }
        }
    }
    true
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
    use turborepo_repository::{
        discovery::PackageDiscovery, package_graph::PackageGraph, package_json::PackageJson,
        package_manager::PackageManager,
    };

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

/// Generic staged-orchestration tests. The fake contributor uses only the open
/// `ToolchainId` and the trait defaults; no language is named.
#[cfg(test)]
mod staged_selection_tests {
    use std::collections::{HashMap, HashSet};

    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_repository::{
        change_mapper::PackageInclusionReason,
        discovery::{DiscoveryResponse, Error as DiscoveryError, PackageDiscovery},
        package_json::PackageJson,
        package_manager::PackageManager,
        toolchain::{
            DiscoverPackagesFuture, DiscoveredPackage, DiscoveredPackages, RepositoryContributor,
            ToolchainId, WorkspaceRoot,
        },
    };
    use turborepo_task_id::TaskId;
    use turborepo_types::{TaskCommandOverride, TaskDefinition};

    use super::*;
    use crate::engine::{Building, Engine};

    struct EmptyDiscovery;

    impl PackageDiscovery for EmptyDiscovery {
        async fn discover_packages(&self) -> Result<DiscoveryResponse, DiscoveryError> {
            Ok(DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(&self) -> Result<DiscoveryResponse, DiscoveryError> {
            self.discover_packages().await
        }
    }

    struct FakeContributor {
        id: ToolchainId,
        root: AbsoluteSystemPathBuf,
    }

    impl RepositoryContributor for FakeContributor {
        fn id(&self) -> ToolchainId {
            self.id.clone()
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            let package = DiscoveredPackage::package(
                Some("native".to_string()),
                PackageJson::default(),
                self.root.join_components(&["native", "manifest"]),
            )
            .with_native_relationships(Vec::new());
            let root = self.root.clone();
            Box::pin(async move {
                Ok(DiscoveredPackages::new(
                    vec![package],
                    vec![WorkspaceRoot::new("fake", root)],
                ))
            })
        }

        fn discover_packages_statically(&self) -> DiscoverPackagesFuture<'_> {
            self.discover_packages()
        }
    }

    async fn graph_with_fake_toolchain(root: &AbsoluteSystemPathBuf) -> PackageGraph {
        PackageGraph::builder_optional(root, None)
            .with_package_discovery(EmptyDiscovery)
            .with_contributor(Arc::new(FakeContributor {
                id: ToolchainId::new("fake-native"),
                root: root.clone(),
            }))
            .build()
            .await
            .unwrap()
    }

    fn add_task(
        builder: &mut Engine<Building>,
        package: &str,
        task: &str,
        definition: TaskDefinition,
    ) -> TaskId<'static> {
        let task_id = TaskId::new(package, task).into_owned();
        builder.get_index(&task_id);
        builder.add_definition(task_id.clone(), definition);
        task_id
    }

    #[tokio::test]
    async fn participating_toolchains_skip_commandless_and_opt_out_tasks() {
        let tmp = tempfile::TempDir::with_prefix("participating_toolchains").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = graph_with_fake_toolchain(&root).await;

        let mut builder: Engine<Building> = Engine::new();
        // A resolved argv executes and selects the owning toolchain.
        add_task(
            &mut builder,
            "native",
            "run",
            TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["echo".to_string()])),
                ..Default::default()
            },
        );
        // An explicit opt-out never executes.
        add_task(
            &mut builder,
            "native",
            "skip",
            TaskDefinition {
                command: Some(TaskCommandOverride::OptOut),
                ..Default::default()
            },
        );
        // A commandless transit task has no command in the catalogue.
        add_task(&mut builder, "native", "transit", TaskDefinition::default());
        let engine: Engine = builder.seal();

        let selection = RunBuilder::participating_toolchains(&graph, &engine);
        assert_eq!(
            selection,
            HashSet::from([ToolchainId::new("fake-native")]),
            "only the toolchain owning an executing task is selected"
        );
    }

    #[tokio::test]
    async fn retain_prepared_tasks_keeps_prepared_dependency_closure() {
        let mut builder: Engine<Building> = Engine::new();
        let run = add_task(&mut builder, "native", "run", TaskDefinition::default());
        let dep = add_task(&mut builder, "native", "dep", TaskDefinition::default());
        let extra = add_task(&mut builder, "native", "extra", TaskDefinition::default());
        let run_idx = builder.get_index(&run);
        let dep_idx = builder.get_index(&dep);
        let extra_idx = builder.get_index(&extra);
        builder.task_graph_mut().add_edge(run_idx, dep_idx, ());
        // `extra` is a dependency that only exists after preparation.
        builder.task_graph_mut().add_edge(run_idx, extra_idx, ());
        builder.connect_to_root(&run);
        let engine = builder.seal();

        let finalized: HashSet<TaskId<'static>> = [run.clone(), dep.clone()].into_iter().collect();
        let retained = RunBuilder::retain_prepared_tasks(engine, &finalized).unwrap();
        let ids: HashSet<String> = retained.task_ids().map(ToString::to_string).collect();

        assert!(ids.contains("native#run"));
        assert!(ids.contains("native#dep"));
        assert!(
            ids.contains("native#extra"),
            "a dependency introduced by preparation must not be dropped"
        );
    }

    #[tokio::test]
    async fn retain_prepared_tasks_refuses_a_missing_finalized_task() {
        let mut builder: Engine<Building> = Engine::new();
        let run = add_task(&mut builder, "native", "run", TaskDefinition::default());
        let engine = builder.seal();

        let finalized: HashSet<TaskId<'static>> =
            [run, TaskId::new("native", "gone")].into_iter().collect();
        assert!(
            RunBuilder::retain_prepared_tasks(engine, &finalized).is_err(),
            "a finalized task missing after preparation must be refused, not dropped"
        );
    }

    #[tokio::test]
    async fn unprepared_toolchain_execution_is_refused() {
        let tmp = tempfile::TempDir::with_prefix("unprepared_toolchain").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = graph_with_fake_toolchain(&root).await;

        let mut builder: Engine<Building> = Engine::new();
        add_task(
            &mut builder,
            "native",
            "run",
            TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["echo".to_string()])),
                ..Default::default()
            },
        );
        let engine: Engine = builder.seal();

        let unprepared: HashSet<ToolchainId> =
            [ToolchainId::new("fake-native")].into_iter().collect();
        assert!(
            RunBuilder::refuse_unprepared_toolchains(&graph, &engine, &unprepared).is_err(),
            "executing a task in an unprepared toolchain must be refused"
        );

        let none: HashSet<ToolchainId> = HashSet::new();
        assert!(RunBuilder::refuse_unprepared_toolchains(&graph, &engine, &none).is_ok());
    }

    /// Generic fake whose *static* observation contributes the same inventory
    /// as full discovery but reports planning uncertainty for its own scope.
    /// Full discovery resolves everything. Only the open `ToolchainId` and
    /// trait defaults are used; no language is named.
    struct UncertainContributor {
        root: AbsoluteSystemPathBuf,
        /// Every scope this observation contributes; the first is the one the
        /// uncertainties are scoped to.
        scopes: Vec<String>,
        uncertainties: Vec<turborepo_repository::toolchain::PlanningUncertainty>,
    }

    impl RepositoryContributor for UncertainContributor {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("uncertain-native")
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            self.observation(Vec::new())
        }

        fn discover_packages_statically(&self) -> DiscoverPackagesFuture<'_> {
            self.observation(self.uncertainties.clone())
        }
    }

    impl UncertainContributor {
        fn observation(
            &self,
            uncertainties: Vec<turborepo_repository::toolchain::PlanningUncertainty>,
        ) -> DiscoverPackagesFuture<'_> {
            let packages = self
                .scopes
                .iter()
                .map(|scope| {
                    DiscoveredPackage::package(
                        Some(scope.clone()),
                        PackageJson::default(),
                        self.root.join_components(&[scope.as_str(), "manifest"]),
                    )
                    .with_native_relationships(Vec::new())
                })
                .collect();
            let root = self.root.clone();
            Box::pin(async move {
                let mut discovered =
                    DiscoveredPackages::new(packages, vec![WorkspaceRoot::new("uncertain", root)]);
                for uncertainty in uncertainties {
                    discovered = discovered.with_planning_uncertainty(uncertainty);
                }
                Ok(discovered)
            })
        }
    }

    type PlanningGraph = Arc<PackageGraph>;

    async fn planning_graph_with_scopes(
        root: &AbsoluteSystemPathBuf,
        uncertainties: Vec<turborepo_repository::toolchain::PlanningUncertainty>,
        scopes: &[&str],
    ) -> PlanningGraph {
        let staged = PackageGraph::builder_optional(root, None)
            .with_package_discovery(EmptyDiscovery)
            .with_contributor(Arc::new(UncertainContributor {
                root: root.clone(),
                scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
                uncertainties,
            }))
            .build_staged()
            .await
            .unwrap();
        let (planning, _plan) = staged.into_parts();
        planning
    }

    async fn planning_graph_with_uncertainty(
        root: &AbsoluteSystemPathBuf,
        uncertainties: Vec<turborepo_repository::toolchain::PlanningUncertainty>,
    ) -> PlanningGraph {
        planning_graph_with_scopes(root, uncertainties, &["native"]).await
    }

    fn root_only_context<'a>(
        resolving: &'a HashSet<ToolchainId>,
        filtered_packages: &'a HashMap<PackageName, PackageInclusionReason>,
        requested_task_names: &'a HashSet<String>,
    ) -> UnresolvedPlanningContext<'a> {
        UnresolvedPlanningContext {
            resolving_toolchains: resolving,
            filtered_packages,
            requested_task_names,
            dependents_direction: false,
            dependencies_direction: false,
        }
    }

    #[tokio::test]
    async fn unrelated_selection_ignores_unresolved_planning_facts() {
        let tmp = tempfile::TempDir::with_prefix("unrelated_selection").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = planning_graph_with_uncertainty(
            &root,
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                )
                .with_possible_targets(["native"]),
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ],
        )
        .await;

        // An unrelated JavaScript-only run: a root task, the uncertain scope
        // outside the selection domain, no dependents expansion, and the
        // uncertain task name not in play. The unresolved facts are ignored,
        // not failed — and the toolchain is never invoked.
        let mut builder: Engine<Building> = Engine::new();
        let build = add_task(&mut builder, "//", "build", TaskDefinition::default());
        builder.connect_to_root(&build);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> = HashSet::new();
        let filtered_packages = HashMap::new();
        let requested: HashSet<String> = ["build".to_string()].into_iter().collect();
        let context = root_only_context(&resolving, &filtered_packages, &requested);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "a selection that provably never consults the unresolved facts must ignore them"
        );
    }

    #[tokio::test]
    async fn commandless_task_in_uncertain_scope_is_refused() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_transit").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = planning_graph_with_uncertainty(
            &root,
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                ),
            ],
        )
        .await;

        // A commandless transit task retained by the run: its dependency
        // structure is hashed into every dependent and nothing will replace
        // the partial facts, so the run is refused with a targeted
        // diagnostic naming the scope.
        let mut builder: Engine<Building> = Engine::new();
        let transit = add_task(&mut builder, "native", "transit", TaskDefinition::default());
        builder.connect_to_root(&transit);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> = HashSet::new();
        let filtered_packages = HashMap::new();
        let requested: HashSet<String> = ["build".to_string()].into_iter().collect();
        let context = root_only_context(&resolving, &filtered_packages, &requested);
        let error = RunBuilder::refuse_unproven_selection(&graph, &engine, &context)
            .expect_err("a retained transit task in an edge-uncertain scope must be refused");
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic naming the uncertain scope, got {error}"
        );
    }

    #[tokio::test]
    async fn preparing_toolchain_exempts_retained_uncertain_tasks() {
        let tmp = tempfile::TempDir::with_prefix("preparing_toolchain").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = planning_graph_with_uncertainty(
            &root,
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                ),
            ],
        )
        .await;

        // The same retained transit task, but the toolchain is selected: the
        // prepared rebuild re-collects the dependency closure, so the
        // uncertainty must not block the run before preparation.
        let mut builder: Engine<Building> = Engine::new();
        let transit = add_task(&mut builder, "native", "transit", TaskDefinition::default());
        builder.connect_to_root(&transit);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> =
            [ToolchainId::new("uncertain-native")].into_iter().collect();
        let filtered_packages = HashMap::new();
        let requested: HashSet<String> = HashSet::new();
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &filtered_packages,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: false,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "a toolchain that will be prepared replaces its uncertainty; the run must proceed to \
             preparation"
        );
    }

    #[tokio::test]
    async fn catalogue_uncertainty_blocks_only_task_names_in_play() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_catalogue").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let graph = planning_graph_with_uncertainty(
            &root,
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ],
        )
        .await;

        let mut builder: Engine<Building> = Engine::new();
        let build = add_task(&mut builder, "//", "build", TaskDefinition::default());
        builder.connect_to_root(&build);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> = HashSet::new();
        // An unfiltered run resolves every package, uncertain scopes included.
        let unfiltered = |packages: &[&str]| -> HashMap<PackageName, PackageInclusionReason> {
            packages
                .iter()
                .map(|name| {
                    (
                        PackageName::from(*name),
                        PackageInclusionReason::IncludedByFilter {
                            filters: Vec::new(),
                        },
                    )
                })
                .collect()
        };
        let all_packages = unfiltered(&["native", "js-app"]);
        // `build` is requested and in the engine; the uncertain `dev` task is
        // not in play, so the catalogue question never arises.
        let build_only: HashSet<String> = ["build".to_string()].into_iter().collect();
        assert!(
            RunBuilder::refuse_unproven_selection(
                &graph,
                &engine,
                &root_only_context(&resolving, &all_packages, &build_only)
            )
            .is_ok(),
            "an uncertain task name this run never requests is ignored"
        );

        // Requesting the uncertain task name makes the catalogue question
        // unprovable: whether the scope participates cannot be answered.
        let dev_requested: HashSet<String> = ["dev".to_string()].into_iter().collect();
        let error = RunBuilder::refuse_unproven_selection(
            &graph,
            &engine,
            &root_only_context(&resolving, &all_packages, &dev_requested),
        )
        .expect_err("requesting an uncertain task name must be refused");
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic, got {error}"
        );

        // A resolved selection that excludes the uncertain scope ignores the
        // catalogue question entirely.
        let js_only = unfiltered(&["js-app"]);
        assert!(
            RunBuilder::refuse_unproven_selection(
                &graph,
                &engine,
                &root_only_context(&resolving, &js_only, &dev_requested)
            )
            .is_ok(),
            "a resolved selection that excludes the scope ignores the fact"
        );
    }

    #[tokio::test]
    async fn dependents_expansion_is_refused_when_uncertain_edges_touch_the_closure() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_dependents").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let bounded = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                )
                .with_possible_targets(["native"]),
            ]
        };
        let unbounded = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                ),
            ]
        };

        let builder: Engine<Building> = Engine::new();
        let engine: Engine = builder.seal();
        let resolving: HashSet<ToolchainId> = HashSet::new();
        let requested: HashSet<String> = HashSet::new();
        let empty_selection = HashMap::new();

        // Bounded unknown edges that touch nothing in the closure are
        // provably independent: the dependents set cannot be extended.
        let graph = planning_graph_with_uncertainty(&root, bounded()).await;
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &empty_selection,
            requested_task_names: &requested,
            dependents_direction: true,
            dependencies_direction: false,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "bounded unknown edges outside the closure are provably independent"
        );

        // The same bounded uncertainty with the candidate inside the closure:
        // a missing dependent cannot be recovered after preparation refreezes
        // the finalized task set.
        let mut native_selected = HashMap::new();
        native_selected.insert(
            PackageName::from("native"),
            PackageInclusionReason::IncludedByFilter {
                filters: vec!["native".to_string()],
            },
        );
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &native_selected,
            requested_task_names: &requested,
            dependents_direction: true,
            dependencies_direction: false,
        };
        let error = RunBuilder::refuse_unproven_selection(&graph, &engine, &context)
            .expect_err("dependents touching an uncertain edge must be refused");
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic, got {error}"
        );

        // Unbounded unknown edges make any dependents expansion unprovable.
        let graph = planning_graph_with_uncertainty(&root, unbounded()).await;
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &empty_selection,
            requested_task_names: &requested,
            dependents_direction: true,
            dependencies_direction: false,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_err(),
            "unbounded unknown edges make dependents unprovable"
        );

        // Without dependents expansion, unbounded edges are irrelevant.
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &empty_selection,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: false,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "dependents-direction is what makes unknown edges selection-relevant"
        );
    }

    #[tokio::test]
    async fn dependencies_expansion_through_uncertain_scope_is_refused_unless_closed() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_dependencies").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        // Two contributed scopes; the uncertain one could connect to either.
        let uncertain = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                )
                .with_possible_targets(["native", "native-two"]),
            ]
        };
        let unbounded = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::internal_edges(
                    "native",
                    "ambiguous-remote-replacement",
                    "an active remote requirement could upgrade a local replacement",
                ),
            ]
        };
        let builder: Engine<Building> = Engine::new();
        let engine: Engine = builder.seal();
        let resolving: HashSet<ToolchainId> = HashSet::new();
        let requested: HashSet<String> = HashSet::new();

        let selected = |names: &[&str]| -> HashMap<PackageName, PackageInclusionReason> {
            names
                .iter()
                .map(|name| {
                    (
                        PackageName::from(*name),
                        PackageInclusionReason::IncludedByFilter {
                            filters: vec![name.to_string()],
                        },
                    )
                })
                .collect()
        };

        // Every candidate is already a closure member: the dependency closure
        // is provably identical under every resolution, so a dependencies
        // expansion through the uncertain scope is allowed.
        let graph = planning_graph_with_scopes(&root, uncertain(), &["native", "native-two"]).await;
        let both = selected(&["native", "native-two"]);
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &both,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: true,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "a closure containing every candidate is provably complete"
        );

        // A candidate outside the closure could be added by the unknown edge,
        // and the frozen `filtered_pkgs` cannot be repaired by preparation.
        let native_only = selected(&["native"]);
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &native_only,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: true,
        };
        let error = RunBuilder::refuse_unproven_selection(&graph, &engine, &context).expect_err(
            "a dependencies expansion through an uncertain scope with outside candidates must be \
             refused",
        );
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic, got {error}"
        );

        // Unbounded unknown edges inside the closure are never provable.
        let graph = planning_graph_with_scopes(&root, unbounded(), &["native", "native-two"]).await;
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &both,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: true,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_err(),
            "unbounded unknown edges make the dependency closure unprovable"
        );

        // Without dependency expansion, an uncertain scope inside the
        // selection is fine: a plain package filter consults no edges, and
        // preparation re-collects task edges for retained tasks.
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &native_only,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: false,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "plain package filters never consult the uncertain edges"
        );

        // An uncertain scope outside the closure is irrelevant.
        let js_only = selected(&["js-app"]);
        let context = UnresolvedPlanningContext {
            resolving_toolchains: &resolving,
            filtered_packages: &js_only,
            requested_task_names: &requested,
            dependents_direction: false,
            dependencies_direction: true,
        };
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "an uncertain scope outside the closure cannot extend it"
        );
    }

    #[tokio::test]
    async fn selected_owner_allows_preparation_when_uncertain_tasks_are_retained_real() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_catalogue_allowed").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        // `dev`'s runnable target is unproven, but `build` is known: the
        // contributor retains it as a real command and the owner is selected.
        let graph = planning_graph_with_uncertainty(
            &root,
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ],
        )
        .await;

        let mut builder: Engine<Building> = Engine::new();
        let build = add_task(
            &mut builder,
            "native",
            "build",
            TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["native-build".to_string()])),
                ..Default::default()
            },
        );
        builder.connect_to_root(&build);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> =
            [ToolchainId::new("uncertain-native")].into_iter().collect();
        let filtered_packages = HashMap::new();
        let requested: HashSet<String> = ["build".to_string()].into_iter().collect();
        // The refusal runs before preparation; the repository staged tests
        // prove that an allowed selection then invokes full discovery. The
        // engine's retained `native#build` puts the scope in the consulted
        // set even with an empty filtered map.
        let context = root_only_context(&resolving, &filtered_packages, &requested);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "a retained-real task with an out-of-play uncertain task must proceed to preparation"
        );
    }

    #[tokio::test]
    async fn possibly_absent_uncertain_task_refuses_even_for_selected_owner() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_catalogue_refused").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let dev_uncertain = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ]
        };
        let whole_catalogue = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the whole task catalogue cannot be proven",
                ),
            ]
        };

        let mut builder: Engine<Building> = Engine::new();
        let build = add_task(
            &mut builder,
            "native",
            "build",
            TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["native-build".to_string()])),
                ..Default::default()
            },
        );
        builder.connect_to_root(&build);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> =
            [ToolchainId::new("uncertain-native")].into_iter().collect();
        let filtered_packages = HashMap::new();

        // A requested uncertain task that is not retained as a real command
        // is possibly absent: preparation cannot add it to the frozen
        // selection, even though the owner is selected.
        let graph = planning_graph_with_uncertainty(&root, dev_uncertain()).await;
        let dev_requested: HashSet<String> = ["dev".to_string()].into_iter().collect();
        let error = RunBuilder::refuse_unproven_selection(
            &graph,
            &engine,
            &root_only_context(&resolving, &filtered_packages, &dev_requested),
        )
        .expect_err("a possibly-absent requested task must be refused");
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic, got {error}"
        );

        // Whole-catalogue uncertainty: `build` retained real is proven, so
        // requesting it proceeds; requesting the unproven `dev` refuses.
        let graph = planning_graph_with_uncertainty(&root, whole_catalogue()).await;
        let build_requested: HashSet<String> = ["build".to_string()].into_iter().collect();
        assert!(
            RunBuilder::refuse_unproven_selection(
                &graph,
                &engine,
                &root_only_context(&resolving, &filtered_packages, &build_requested)
            )
            .is_ok(),
            "a retained-real task is proven even under whole-catalogue uncertainty"
        );
        let error = RunBuilder::refuse_unproven_selection(
            &graph,
            &engine,
            &root_only_context(&resolving, &filtered_packages, &dev_requested),
        )
        .expect_err("a possibly-absent requested task must be refused");
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic, got {error}"
        );
    }

    #[tokio::test]
    async fn shape_uncertain_task_retained_real_allows_preparation() {
        let tmp = tempfile::TempDir::with_prefix("uncertain_shape").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        // `dev` is retained as a real command — membership proven, only its
        // shape unproven — so a selected owner may prepare and resolve the
        // shape without inventing membership.
        let graph = planning_graph_with_uncertainty(
            &root,
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ],
        )
        .await;

        let mut builder: Engine<Building> = Engine::new();
        let dev = add_task(
            &mut builder,
            "native",
            "dev",
            TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["native-dev".to_string()])),
                ..Default::default()
            },
        );
        builder.connect_to_root(&dev);
        let engine: Engine = builder.seal();

        let filtered_packages = HashMap::new();
        let requested: HashSet<String> = ["dev".to_string()].into_iter().collect();

        let resolving: HashSet<ToolchainId> =
            [ToolchainId::new("uncertain-native")].into_iter().collect();
        let context = root_only_context(&resolving, &filtered_packages, &requested);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "a retained-real uncertain task only needs its shape resolved, which preparation does"
        );

        // Without a selected owner, nothing can ever resolve even the shape.
        let resolving: HashSet<ToolchainId> = HashSet::new();
        let context = root_only_context(&resolving, &filtered_packages, &requested);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_err(),
            "an unselected owner can never resolve the catalogue"
        );
    }

    #[tokio::test]
    async fn qualified_and_excluded_selectors_do_not_consult_uncertain_catalogues() {
        let tmp = tempfile::TempDir::with_prefix("scope_guard_selectors").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let dev_uncertain = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ]
        };
        let graph =
            planning_graph_with_scopes(&root, dev_uncertain(), &["native", "native-two", "js-app"])
                .await;

        let selected = |names: &[&str]| -> HashMap<PackageName, PackageInclusionReason> {
            names
                .iter()
                .map(|name| {
                    (
                        PackageName::from(*name),
                        PackageInclusionReason::IncludedByFilter {
                            filters: vec![name.to_string()],
                        },
                    )
                })
                .collect()
        };

        // Qualified task arguments (`js#dev`, `native#build`) put no task
        // name in play for foreign catalogues: their scopes are captured by
        // the engine's retained task ids, and only unqualified arguments
        // contribute requested names.
        let mut builder: Engine<Building> = Engine::new();
        let js_dev = add_task(&mut builder, "js-app", "dev", TaskDefinition::default());
        let native_build = add_task(
            &mut builder,
            "native",
            "build",
            TaskDefinition {
                command: Some(TaskCommandOverride::Argv(vec!["native-build".to_string()])),
                ..Default::default()
            },
        );
        builder.connect_to_root(&js_dev);
        builder.connect_to_root(&native_build);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> = HashSet::new();
        let qualified_only: HashSet<String> = HashSet::new();
        let both_selected = selected(&["js-app", "native"]);
        let context = root_only_context(&resolving, &both_selected, &qualified_only);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &engine, &context).is_ok(),
            "qualified arguments must not put their task names in foreign catalogues"
        );

        // An excluded scope is never consulted: the resolved selection
        // excludes it and the engine retains nothing for it.
        let js_builder: Engine<Building> = {
            let mut builder: Engine<Building> = Engine::new();
            let build = add_task(&mut builder, "js-app", "build", TaskDefinition::default());
            builder.connect_to_root(&build);
            builder
        };
        let js_engine: Engine = js_builder.seal();
        let js_only = selected(&["js-app"]);
        let dev_requested: HashSet<String> = ["dev".to_string()].into_iter().collect();
        let context = root_only_context(&resolving, &js_only, &dev_requested);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &js_engine, &context).is_ok(),
            "an excluded scope's catalogue is never consulted"
        );

        // The same unqualified request against a selection that includes the
        // uncertain scope is refused — proving the pass above came from the
        // exclusion, not from the task name being out of play.
        let with_native = selected(&["js-app", "native"]);
        let context = root_only_context(&resolving, &with_native, &dev_requested);
        assert!(
            RunBuilder::refuse_unproven_selection(&graph, &js_engine, &context).is_err(),
            "an in-scope uncertain catalogue with an in-play task name must be refused"
        );
    }

    #[tokio::test]
    async fn config_wired_phantom_bring_scope_into_catalogue_consultation() {
        let tmp = tempfile::TempDir::with_prefix("phantom_consulted").unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let dev_uncertain = || {
            vec![
                turborepo_repository::toolchain::PlanningUncertainty::task_catalogue(
                    "native",
                    "constrained-runnable-target",
                    "the dev task's runnable target cannot be proven",
                )
                .with_uncertain_task_names(["dev"]),
            ]
        };
        let graph = planning_graph_with_uncertainty(&root, dev_uncertain()).await;

        // A JS-only selection whose engine retains a config-wired phantom
        // `native#dev` (created by a `dependsOn` reference): the phantom
        // brings the scope into the consulted set and its task name into
        // play, so the required task's reality cannot be silently ignored.
        let mut builder: Engine<Building> = Engine::new();
        let js_build = add_task(&mut builder, "js-app", "build", TaskDefinition::default());
        let native_dev = add_task(&mut builder, "native", "dev", TaskDefinition::default());
        builder.connect_to_root(&js_build);
        builder.connect_to_root(&native_dev);
        let engine: Engine = builder.seal();

        let resolving: HashSet<ToolchainId> = HashSet::new();
        let filtered_packages: HashMap<PackageName, PackageInclusionReason> = [(
            PackageName::from("js-app"),
            PackageInclusionReason::IncludedByFilter {
                filters: vec!["js-app".to_string()],
            },
        )]
        .into_iter()
        .collect();
        let build_requested: HashSet<String> = ["build".to_string()].into_iter().collect();
        let context = root_only_context(&resolving, &filtered_packages, &build_requested);
        let error = RunBuilder::refuse_unproven_selection(&graph, &engine, &context)
            .expect_err("a config-wired phantom's reality cannot be silently ignored");
        assert!(
            matches!(
                error,
                Error::UnresolvedPlanningFact { ref package, .. } if package == "native"
            ),
            "expected a targeted diagnostic, got {error}"
        );
    }

    #[test]
    fn match_dependencies_selectors_expand_dependents() {
        let parse = |pattern: &str| pattern.parse::<TargetSelector>().unwrap();
        // `...foo` selects dependents of foo: reverse edges.
        let dependents = parse("...foo");
        assert!(dependents.include_dependents);
        assert!(selector_expands_dependents(&[dependents.clone()]));
        assert!(!selector_expands_dependencies(&[dependents]));

        // `foo...[main]` selects dependents of changed packages — also a
        // reverse traversal, despite the name — so it must count as
        // dependents-direction, never dependencies-direction.
        let match_dependencies = parse("foo...[main]");
        assert!(match_dependencies.match_dependencies);
        assert!(selector_expands_dependents(&[match_dependencies.clone()]));
        assert!(!selector_expands_dependencies(&[match_dependencies]));

        // `foo...` follows foo's own outgoing edges: forward traversal.
        let dependencies = parse("foo...");
        assert!(dependencies.include_dependencies);
        assert!(!selector_expands_dependents(&[dependencies.clone()]));
        assert!(selector_expands_dependencies(&[dependencies]));

        // Plain name selectors traverse nothing.
        let plain = parse("foo");
        assert!(!selector_expands_dependents(&[plain.clone()]));
        assert!(!selector_expands_dependencies(&[plain]));
    }
}
