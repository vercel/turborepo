use std::{
    collections::HashSet,
    ops::DerefMut as _,
    sync::{Arc, Mutex},
};

use futures::{future::join_all, StreamExt};
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{
    select,
    sync::{oneshot, Notify},
    task::JoinHandle,
};
use tracing::{instrument, trace};
use turborepo_daemon::{
    proto, DaemonClient, DaemonConnector, DaemonConnectorError, DaemonError, Paths,
};
use turborepo_repository::package_graph::PackageName;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::sender::UISender;

use crate::{
    commands::CommandBase,
    config::resolve_turbo_config_path,
    engine::{EngineExt, TaskNode},
    get_version, opts,
    run::{self, builder::RunBuilder, scope::target_selector::InvalidSelectorError, Run},
};

#[derive(Debug)]
enum ChangedPackages {
    All,
    Some(HashSet<PackageName>),
}

impl Default for ChangedPackages {
    fn default() -> Self {
        ChangedPackages::Some(HashSet::new())
    }
}

impl ChangedPackages {
    pub fn is_empty(&self) -> bool {
        match self {
            ChangedPackages::All => false,
            ChangedPackages::Some(pkgs) => pkgs.is_empty(),
        }
    }

    /// Filter a `Some` set down to only packages in the watched set.
    /// `All` is left unchanged because it triggers a full rebuild that
    /// recomputes the watched set from scratch.
    fn filter_to_watched(&mut self, watched_packages: &HashSet<PackageName>) {
        if let ChangedPackages::Some(pkgs) = self {
            pkgs.retain(|pkg| watched_packages.contains(pkg));
        }
    }
}

pub struct WatchClient {
    run: Arc<Run>,
    watched_packages: HashSet<PackageName>,
    persistent_tasks_handle: Option<RunHandle>,
    active_runs: Vec<RunHandle>,
    connector: DaemonConnector,
    // A daemon client used by the run cache to register output globs and check
    // whether outputs have changed. This prevents cache restores from writing
    // files that trigger the file watcher and cause infinite rebuild loops.
    daemon_client: DaemonClient<DaemonConnector>,
    base: CommandBase,
    telemetry: CommandEventBuilder,
    handler: SignalHandler,
    ui_sender: Option<UISender>,
    ui_handle: Option<JoinHandle<Result<(), turborepo_ui::Error>>>,
    experimental_write_cache: bool,
    query_server: Option<Arc<dyn turborepo_query_api::QueryServer>>,
}

struct RunHandle {
    stopper: run::RunStopper,
    run_task: JoinHandle<Result<i32, run::Error>>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Failed to connect to daemon.")]
    #[diagnostic(transparent)]
    Daemon(#[from] DaemonError),
    #[error("Failed to connect to daemon.")]
    DaemonConnector(#[from] DaemonConnectorError),
    #[error("Failed to decode message from daemon.")]
    Decode(#[from] prost::DecodeError),
    #[error("Could not get current executable.")]
    CurrentExe(std::io::Error),
    #[error("Could not start `turbo`.")]
    Start(std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] run::Error),
    #[error("`--since` is not supported in Watch Mode.")]
    SinceNotSupported,
    #[error(transparent)]
    Opts(#[from] opts::Error),
    #[error("Invalid filter pattern")]
    InvalidSelector(#[from] InvalidSelectorError),
    #[error("Filter cannot contain a git range in Watch Mode.")]
    GitRangeInFilter {
        #[source_code]
        filter: String,
        #[label]
        span: SourceSpan,
    },
    #[error("Daemon connection closed.")]
    ConnectionClosed,
    #[error(
        "Timed out waiting for the daemon's file watcher to become ready. The daemon may be \
         having trouble watching your repository. Try running `turbo daemon clean` and retrying."
    )]
    DaemonFileWatchingTimeout,
    #[error("Failed to subscribe to signal handler. Shutting down.")]
    NoSignalHandler,
    #[error("Watch interrupted due to signal.")]
    SignalInterrupt,
    #[error("Package change error.")]
    PackageChange(#[from] tonic::Status),
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
    #[error("Could not connect to UI thread: {0}")]
    UISend(String),
    #[error("Invalid config: {0}")]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
}

impl WatchClient {
    pub async fn new(
        base: CommandBase,
        experimental_write_cache: bool,
        telemetry: CommandEventBuilder,
        query_server: Option<Arc<dyn turborepo_query_api::QueryServer>>,
    ) -> Result<Self, Error> {
        let signal = get_signal()?;
        let handler = SignalHandler::new(signal);

        let standard_config_path = resolve_turbo_config_path(&base.repo_root)?;

        // Determine if we're using a custom turbo.json path
        let custom_turbo_json_path =
            if base.opts.repo_opts.root_turbo_json_path != standard_config_path {
                tracing::info!(
                    "Using custom turbo.json path: {} (standard: {})",
                    base.opts.repo_opts.root_turbo_json_path,
                    standard_config_path
                );
                Some(base.opts.repo_opts.root_turbo_json_path.clone())
            } else {
                None
            };

        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: Paths::from_repo_root(&base.repo_root),
            custom_turbo_json_path,
        };

        // Connect a daemon client for the run cache. This allows the cache to
        // register output globs with the daemon's GlobWatcher and skip restoring
        // outputs that are already on disk, preventing the file watcher from
        // seeing restored files as changes and causing an infinite rebuild loop.
        let daemon_client = connector.clone().connect().await?;

        let new_base = base.clone();
        let mut run_builder =
            RunBuilder::new(new_base, None)?.with_daemon_client(daemon_client.clone());
        if let Some(ref qs) = query_server {
            run_builder = run_builder.with_query_server(qs.clone());
        }
        let (run, _analytics) = run_builder.build(&handler, telemetry.clone()).await?;
        let run = Arc::new(run);

        let watched_packages = run.get_relevant_packages();

        let (ui_sender, ui_handle) = run.start_ui()?.unzip();

        Ok(Self {
            base,
            run,
            watched_packages,
            connector,
            daemon_client,
            handler,
            telemetry,
            experimental_write_cache,
            persistent_tasks_handle: None,
            active_runs: Vec::new(),
            ui_sender,
            ui_handle,
            query_server,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let connector = self.connector.clone();
        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;

        // Wait for the initial event from the daemon with a timeout.
        // The daemon sends a Rediscover event immediately when the stream opens,
        // but the stream won't produce anything until the daemon's file watcher
        // is ready. If it never becomes ready, we'd hang here forever.
        let initial_event = tokio::time::timeout(std::time::Duration::from_secs(10), events.next())
            .await
            .map_err(|_| Error::DaemonFileWatchingTimeout)?
            .ok_or(Error::ConnectionClosed)?;
        let initial_event = initial_event?;

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        // We explicitly use a tokio::sync::Mutex here to avoid deadlocks.
        // If we used a std::sync::Mutex, we could deadlock by spinning the lock
        // and not yielding back to the tokio runtime.
        let changed_packages = Mutex::new(ChangedPackages::default());
        let notify_run = Arc::new(Notify::new());
        let notify_event = notify_run.clone();

        // Process the initial event
        Self::handle_change_event(&changed_packages, initial_event.event.unwrap())?;
        notify_event.notify_one();

        let event_fut = async {
            while let Some(event) = events.next().await {
                let event = event?;
                Self::handle_change_event(&changed_packages, event.event.unwrap())?;
                notify_event.notify_one();
            }

            Err(Error::ConnectionClosed)
        };

        let run_fut = async {
            loop {
                notify_run.notified().await;
                let some_changed_packages = {
                    let mut changed_packages_guard =
                        changed_packages.lock().expect("poisoned lock");
                    (!changed_packages_guard.is_empty())
                        .then(|| std::mem::take(changed_packages_guard.deref_mut()))
                };

                if let Some(mut changed_packages) = some_changed_packages {
                    // Clean up currently running tasks
                    self.active_runs.retain(|h| !h.run_task.is_finished());

                    // Safe to filter early: the engine only contains tasks from
                    // watched_packages, so unwatched packages can't impact any
                    // running tasks.
                    changed_packages.filter_to_watched(&self.watched_packages);

                    match changed_packages {
                        ChangedPackages::Some(ref pkgs) => {
                            let impacted = self.stop_impacted_tasks(pkgs).await;
                            changed_packages = ChangedPackages::Some(impacted);
                        }
                        ChangedPackages::All => {
                            for handle in self.active_runs.drain(..) {
                                handle.stopper.stop().await;
                                let _ = handle.run_task.await;
                            }
                        }
                    }
                    let new_run = self.execute_run(changed_packages).await?;
                    self.active_runs.push(new_run);
                }
            }
        };

        select! {
            biased;
            _ = signal_subscriber.listen() => {
                tracing::info!("shutting down");
                Err(Error::SignalInterrupt)
            }
            result = event_fut => {
                result
            }
            run_result = run_fut => {
                run_result
            }
        }
    }

    #[instrument(skip(changed_packages))]
    fn handle_change_event(
        changed_packages: &Mutex<ChangedPackages>,
        event: proto::package_change_event::Event,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
            }) => {
                let package_name = PackageName::from(package_name);

                match changed_packages.lock().expect("poisoned lock").deref_mut() {
                    ChangedPackages::All => {
                        // If we've already changed all packages, ignore
                    }
                    ChangedPackages::Some(ref mut pkgs) => {
                        pkgs.insert(package_name);
                    }
                }
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
                *changed_packages.lock().expect("poisoned lock") = ChangedPackages::All;
            }
            proto::package_change_event::Event::Error(proto::PackageChangeError { message }) => {
                return Err(DaemonError::Unavailable(message).into());
            }
        }

        Ok(())
    }

    async fn stop_impacted_tasks(&self, pkgs: &HashSet<PackageName>) -> HashSet<PackageName> {
        let engine = self.run.engine();

        let impacted_nodes = engine.tasks_impacted_by_packages(pkgs);

        // Extract task IDs from task nodes (filtering out Root nodes)
        let task_ids: Vec<_> = impacted_nodes
            .iter()
            .filter_map(|node| match node {
                TaskNode::Task(task_id) => Some(task_id.clone()),
                TaskNode::Root => None,
            })
            .collect();

        // Collect unique impacted packages
        let impacted_packages: HashSet<_> = task_ids
            .iter()
            .map(|task_id| PackageName::from(task_id.package()))
            .collect();

        join_all(
            self.active_runs
                .iter()
                .map(|handle| handle.stopper.stop_tasks(&task_ids)),
        )
        .await;

        impacted_packages
    }

    /// Shut down any resources that run as part of watch.
    pub async fn shutdown(&mut self) {
        if let Some(sender) = &self.ui_sender {
            sender.stop().await;
        }
        for handle in self.active_runs.drain(..) {
            handle.stopper.stop().await;
            let _ = handle.run_task.await;
        }
        if let Some(RunHandle { stopper, run_task }) = self.persistent_tasks_handle.take() {
            // Shut down the tasks for the run
            stopper.stop().await;
            // Run should exit shortly after we stop all child tasks, wait for it to finish
            // to ensure all messages are flushed.
            let _ = run_task.await;
        }
    }

    /// Executes a run with the given changed packages. Splits the run into two
    /// parts:
    /// 1. The persistent tasks that are not allowed to be interrupted
    /// 2. The non-persistent tasks and the persistent tasks that are allowed to
    ///    be interrupted
    ///
    /// Returns a handle to the task running (2)
    async fn execute_run(&mut self, changed_packages: ChangedPackages) -> Result<RunHandle, Error> {
        // Should we recover here?
        trace!("handling run with changed packages: {changed_packages:?}");
        match changed_packages {
            ChangedPackages::Some(packages) => {
                let mut opts = self.base.opts().clone();
                if !self.experimental_write_cache {
                    opts.cache_opts.cache.remote.write = false;
                    opts.cache_opts.cache.remote.read = false;
                }

                let new_base = CommandBase::from_opts(
                    opts,
                    self.base.repo_root.clone(),
                    get_version(),
                    self.base.color_config,
                );

                let signal_handler = self.handler.clone();
                let telemetry = self.telemetry.clone();

                let mut run_builder = RunBuilder::new(new_base, None)?
                    .with_daemon_client(self.daemon_client.clone())
                    .with_entrypoint_packages(packages)
                    .hide_prelude();
                if let Some(ref qs) = self.query_server {
                    run_builder = run_builder.with_query_server(qs.clone());
                }
                let (run, _analytics) = run_builder.build(&signal_handler, telemetry).await?;

                if let Some(sender) = &self.ui_sender {
                    let task_names = run.engine.tasks_with_command(&run.pkg_dep_graph);
                    sender
                        .restart_tasks(task_names)
                        .map_err(|err| Error::UISend(format!("some packages changed: {err}")))?;
                }

                let ui_sender = self.ui_sender.clone();
                Ok(RunHandle {
                    stopper: run.stopper(),
                    run_task: tokio::spawn(async move { run.run(ui_sender, true).await }),
                })
            }
            ChangedPackages::All => {
                let mut opts = self.base.opts().clone();
                if !self.experimental_write_cache {
                    opts.cache_opts.cache.remote.write = false;
                    opts.cache_opts.cache.remote.read = false;
                }

                let base = CommandBase::from_opts(
                    opts,
                    self.base.repo_root.clone(),
                    get_version(),
                    self.base.color_config,
                );

                // rebuild run struct
                let mut run_builder = RunBuilder::new(base.clone(), None)?
                    .with_daemon_client(self.daemon_client.clone())
                    .hide_prelude();
                if let Some(ref qs) = self.query_server {
                    run_builder = run_builder.with_query_server(qs.clone());
                }
                let (run, _analytics) = run_builder
                    .build(&self.handler, self.telemetry.clone())
                    .await?;
                self.run = run.into();

                self.watched_packages = self.run.get_relevant_packages();

                // Clean up currently running persistent tasks
                if let Some(RunHandle { stopper, run_task }) = self.persistent_tasks_handle.take() {
                    // Shut down the tasks for the run
                    stopper.stop().await;
                    // Run should exit shortly after we stop all child tasks, wait for it to finish
                    // to ensure all messages are flushed.
                    let _ = run_task.await;
                }
                if let Some(sender) = &self.ui_sender {
                    let task_names = self.run.engine.tasks_with_command(&self.run.pkg_dep_graph);
                    sender
                        .update_tasks(task_names)
                        .map_err(|err| Error::UISend(format!("all packages changed {err}")))?;
                }

                if self.run.has_non_interruptible_tasks() {
                    debug_assert!(
                        self.persistent_tasks_handle.is_none(),
                        "persistent handle should be empty before creating a new one"
                    );
                    let persistent_run = self.run.create_run_for_non_interruptible_tasks();
                    let non_persistent_run = self.run.create_run_for_interruptible_tasks();

                    let persistent_stopper = persistent_run.stopper();
                    let non_persistent_stopper = non_persistent_run.stopper();

                    let non_persistent_ui_sender = self.ui_sender.clone();
                    let persistent_ui_sender = self.ui_sender.clone();

                    // Signal from non-persistent run to persistent run: non-persistent
                    // tasks finished successfully, so it's safe to start persistent ones.
                    let (ready_tx, ready_rx) = oneshot::channel::<()>();

                    // If we have persistent tasks, we run them on a separate thread
                    // since persistent tasks don't finish
                    let persistent_task = tokio::spawn(async move {
                        match ready_rx.await {
                            Ok(()) => persistent_run.run(persistent_ui_sender, true).await,
                            Err(_) => Ok(0),
                        }
                    });

                    self.persistent_tasks_handle = Some(RunHandle {
                        stopper: persistent_stopper,
                        run_task: persistent_task,
                    });

                    let non_persistent_task = tokio::spawn(async move {
                        let result = non_persistent_run.run(non_persistent_ui_sender, true).await;
                        if matches!(result, Ok(0)) {
                            let _ = ready_tx.send(());
                        }
                        result
                    });

                    Ok(RunHandle {
                        stopper: non_persistent_stopper,
                        run_task: non_persistent_task,
                    })
                } else {
                    let ui_sender = self.ui_sender.clone();
                    let run = self.run.clone();
                    Ok(RunHandle {
                        stopper: run.stopper(),
                        run_task: tokio::spawn(async move { run.run(ui_sender, true).await }),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashSet, sync::Mutex};

    use tokio::sync::oneshot;
    use turborepo_daemon::proto;
    use turborepo_repository::package_graph::PackageName;

    use super::{ChangedPackages, WatchClient};

    fn make_package_changed(name: &str) -> proto::package_change_event::Event {
        proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
            package_name: name.to_string(),
        })
    }

    fn make_rediscover() -> proto::package_change_event::Event {
        proto::package_change_event::Event::RediscoverPackages(proto::RediscoverPackages {})
    }

    fn make_error(msg: &str) -> proto::package_change_event::Event {
        proto::package_change_event::Event::Error(proto::PackageChangeError {
            message: msg.to_string(),
        })
    }

    #[test]
    fn changed_packages_default_is_empty() {
        let cp = ChangedPackages::default();
        assert!(cp.is_empty());
        assert!(matches!(cp, ChangedPackages::Some(ref s) if s.is_empty()));
    }

    #[test]
    fn changed_packages_all_is_never_empty() {
        assert!(!ChangedPackages::All.is_empty());
    }

    #[test]
    fn changed_packages_some_with_items_is_not_empty() {
        let mut set = HashSet::new();
        set.insert(PackageName::from("a"));
        assert!(!ChangedPackages::Some(set).is_empty());
    }

    #[test]
    fn handle_change_event_package_changed_inserts() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some(pkgs) => {
                assert_eq!(pkgs.len(), 1);
                assert!(pkgs.contains(&PackageName::from("web")));
            }
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn handle_change_event_multiple_packages_accumulate() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();
        WatchClient::handle_change_event(&changed, make_package_changed("ui")).unwrap();
        WatchClient::handle_change_event(&changed, make_package_changed("utils")).unwrap();

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some(pkgs) => {
                assert_eq!(pkgs.len(), 3);
                assert!(pkgs.contains(&PackageName::from("web")));
                assert!(pkgs.contains(&PackageName::from("ui")));
                assert!(pkgs.contains(&PackageName::from("utils")));
            }
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn handle_change_event_duplicate_package_deduplicates() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some(pkgs) => assert_eq!(pkgs.len(), 1),
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn handle_change_event_rediscover_sets_all() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();
        WatchClient::handle_change_event(&changed, make_rediscover()).unwrap();

        let guard = changed.lock().unwrap();
        assert!(matches!(*guard, ChangedPackages::All));
    }

    #[test]
    fn handle_change_event_package_changed_after_all_is_noop() {
        let changed = Mutex::new(ChangedPackages::All);
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();

        let guard = changed.lock().unwrap();
        assert!(matches!(*guard, ChangedPackages::All));
    }

    #[test]
    fn handle_change_event_error_returns_err() {
        let changed = Mutex::new(ChangedPackages::default());
        let result =
            WatchClient::handle_change_event(&changed, make_error("daemon is unavailable"));
        assert!(result.is_err());
    }

    #[test]
    fn handle_change_event_rediscover_then_rediscover_stays_all() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_rediscover()).unwrap();
        WatchClient::handle_change_event(&changed, make_rediscover()).unwrap();

        let guard = changed.lock().unwrap();
        assert!(matches!(*guard, ChangedPackages::All));
    }

    #[test]
    fn filter_to_watched_removes_unwatched_packages() {
        let watched: HashSet<_> = ["web", "ui"]
            .iter()
            .map(|s| PackageName::from(*s))
            .collect();
        let mut changed = ChangedPackages::Some(
            ["web", "api", "ui", "utils"]
                .iter()
                .map(|s| PackageName::from(*s))
                .collect(),
        );

        changed.filter_to_watched(&watched);

        match changed {
            ChangedPackages::Some(pkgs) => {
                assert_eq!(pkgs.len(), 2);
                assert!(pkgs.contains(&PackageName::from("web")));
                assert!(pkgs.contains(&PackageName::from("ui")));
                assert!(!pkgs.contains(&PackageName::from("api")));
            }
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    #[test]
    fn filter_to_watched_leaves_all_unchanged() {
        let watched: HashSet<_> = ["web"].iter().map(|s| PackageName::from(*s)).collect();
        let mut changed = ChangedPackages::All;

        changed.filter_to_watched(&watched);
        assert!(matches!(changed, ChangedPackages::All));
    }

    #[test]
    fn filter_to_watched_empty_watched_set_clears_all() {
        let watched: HashSet<PackageName> = HashSet::new();
        let mut changed = ChangedPackages::Some(
            ["web", "ui"]
                .iter()
                .map(|s| PackageName::from(*s))
                .collect(),
        );

        changed.filter_to_watched(&watched);

        match changed {
            ChangedPackages::Some(pkgs) => assert!(pkgs.is_empty()),
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    #[test]
    fn filter_to_watched_no_overlap() {
        let watched: HashSet<_> = ["web"].iter().map(|s| PackageName::from(*s)).collect();
        let mut changed = ChangedPackages::Some(
            ["api", "utils"]
                .iter()
                .map(|s| PackageName::from(*s))
                .collect(),
        );

        changed.filter_to_watched(&watched);

        match changed {
            ChangedPackages::Some(pkgs) => assert!(pkgs.is_empty()),
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    #[test]
    fn changed_packages_take_resets_to_default() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web")).unwrap();

        let taken = {
            let mut guard = changed.lock().unwrap();
            assert!(!guard.is_empty());
            std::mem::take(&mut *guard)
        };

        // After take, the mutex should hold an empty Some
        let guard = changed.lock().unwrap();
        assert!(guard.is_empty());

        // The taken value should have the package
        match taken {
            ChangedPackages::Some(pkgs) => {
                assert!(pkgs.contains(&PackageName::from("web")));
            }
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    // -----------------------------------------------------------------------
    // Oneshot coordination pattern tests
    //
    // These test the contract used in execute_run to gate persistent tasks
    // behind non-persistent task completion. The pattern:
    //   - Non-persistent run sends on ready_tx only when result is Ok(0)
    //   - Persistent run waits on ready_rx before starting
    //   - If ready_tx is dropped (failure/cancellation), persistent run exits
    // -----------------------------------------------------------------------

    /// Simulates the non-persistent side of the coordination: only sends the
    /// ready signal when the run result is Ok(0).
    fn simulate_non_persistent(
        ready_tx: oneshot::Sender<()>,
        result: Result<i32, &str>,
    ) -> Result<i32, &str> {
        if matches!(result, Ok(0)) {
            let _ = ready_tx.send(());
        }
        result
    }

    #[tokio::test]
    async fn persistent_starts_after_successful_build() {
        let (ready_tx, ready_rx) = oneshot::channel::<()>();

        let persistent = tokio::spawn(async move {
            match ready_rx.await {
                Ok(()) => "started",
                Err(_) => "skipped",
            }
        });

        let _ = simulate_non_persistent(ready_tx, Ok(0));

        let outcome = persistent.await.unwrap();
        assert_eq!(outcome, "started");
    }

    #[tokio::test]
    async fn persistent_skipped_on_nonzero_exit() {
        let (ready_tx, ready_rx) = oneshot::channel::<()>();

        let persistent = tokio::spawn(async move {
            match ready_rx.await {
                Ok(()) => "started",
                Err(_) => "skipped",
            }
        });

        let result = simulate_non_persistent(ready_tx, Ok(1));
        assert!(matches!(result, Ok(1)));

        let outcome = persistent.await.unwrap();
        assert_eq!(outcome, "skipped");
    }

    #[tokio::test]
    async fn persistent_skipped_on_error() {
        let (ready_tx, ready_rx) = oneshot::channel::<()>();

        let persistent = tokio::spawn(async move {
            match ready_rx.await {
                Ok(()) => "started",
                Err(_) => "skipped",
            }
        });

        let result = simulate_non_persistent(ready_tx, Err("build failed"));
        assert!(result.is_err());

        let outcome = persistent.await.unwrap();
        assert_eq!(outcome, "skipped");
    }

    #[tokio::test]
    async fn persistent_skipped_when_sender_dropped_without_sending() {
        let (ready_tx, ready_rx) = oneshot::channel::<()>();

        let persistent = tokio::spawn(async move {
            match ready_rx.await {
                Ok(()) => "started",
                Err(_) => "skipped",
            }
        });

        // Simulate non-persistent task being cancelled: sender dropped
        drop(ready_tx);

        let outcome = persistent.await.unwrap();
        assert_eq!(outcome, "skipped");
    }
}
