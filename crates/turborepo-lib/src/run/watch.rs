use std::{
    collections::HashSet,
    env,
    ops::DerefMut as _,
    sync::{Arc, Mutex},
    time::Duration,
};

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{
    select,
    sync::{broadcast, oneshot, Notify},
    task::JoinHandle,
};
use tracing::{instrument, trace};
use turbopath::AnchoredSystemPathBuf;
use turborepo_daemon::{PackageChangeEvent, PackageChangesWatcher as PackageChangesWatcherTrait};
use turborepo_filewatch::{
    cookies::CookieWriter, globwatcher::GlobWatcher, hash_watcher::HashWatcher,
    package_watcher::PackageWatcher, FileSystemWatcher,
};
use turborepo_log::{sinks::collector::CollectorSink, Logger};
use turborepo_repository::package_graph::PackageName;
use turborepo_run_cache::{OutputWatcher, OutputWatcherError};
use turborepo_scm::SCM;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{sender::UISender, StdoutSink, TerminalSink, TuiSink};

use crate::{
    commands::CommandBase,
    config::resolve_turbo_config_path,
    engine::{EngineExt, TaskNode},
    get_version, opts,
    package_changes_watcher::PackageChangesWatcher,
    run::{self, builder::RunBuilder, scope::target_selector::InvalidSelectorError, Run},
};

#[derive(Debug)]
enum ChangedPackages {
    All,
    Some {
        packages: HashSet<PackageName>,
        changed_files: HashSet<AnchoredSystemPathBuf>,
    },
}

impl Default for ChangedPackages {
    fn default() -> Self {
        ChangedPackages::Some {
            packages: HashSet::new(),
            changed_files: HashSet::new(),
        }
    }
}

impl ChangedPackages {
    pub fn is_empty(&self) -> bool {
        match self {
            ChangedPackages::All => false,
            ChangedPackages::Some { packages, .. } => packages.is_empty(),
        }
    }

    /// Filter a `Some` set down to only packages in the watched set.
    /// `All` is left unchanged because it triggers a full rebuild that
    /// recomputes the watched set from scratch.
    fn filter_to_watched(&mut self, watched_packages: &HashSet<PackageName>) {
        if let ChangedPackages::Some { packages, .. } = self {
            packages.retain(|pkg| watched_packages.contains(pkg));
        }
    }
}

/// In-process file watching infrastructure that replaces the daemon.
/// All components are standalone structs from `turborepo-filewatch`
/// and `turborepo-lib` — no gRPC or IPC involved.
struct FileWatching {
    // Kept alive so the OS-level watcher keeps running.
    _watcher: Arc<FileSystemWatcher>,
    glob_watcher: Arc<GlobWatcher>,
    // Kept alive so its background tasks continue providing package
    // discovery data to the HashWatcher.
    _package_watcher: Arc<PackageWatcher>,
    // Kept alive to maintain the watcher background task.
    _package_changes_watcher: PackageChangesWatcher,
}

/// Adapts `GlobWatcher` to the `OutputWatcher` trait so it can be passed
/// to `RunCache`/`TaskCache` for output change tracking.
struct InProcessOutputWatcher {
    glob_watcher: Arc<GlobWatcher>,
}

impl OutputWatcher for InProcessOutputWatcher {
    fn get_changed_outputs(
        &self,
        hash: String,
        output_globs: Vec<String>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HashSet<String>, OutputWatcherError>> + Send>,
    > {
        let glob_watcher = self.glob_watcher.clone();
        let candidates: HashSet<String> = output_globs.into_iter().collect();
        Box::pin(async move {
            glob_watcher
                .get_changed_globs(hash, candidates, Duration::from_millis(100))
                .await
                .map_err(|e| OutputWatcherError(Box::new(e)))
        })
    }

    fn notify_outputs_written(
        &self,
        hash: String,
        output_globs: Vec<String>,
        output_exclusion_globs: Vec<String>,
        _time_saved: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OutputWatcherError>> + Send>>
    {
        let glob_watcher = self.glob_watcher.clone();
        Box::pin(async move {
            let glob_set = turborepo_filewatch::globwatcher::GlobSet::from_raw(
                output_globs,
                output_exclusion_globs,
            )
            .map_err(|e| OutputWatcherError(Box::new(e)))?;
            glob_watcher
                .watch_globs(hash, glob_set, Duration::from_millis(100))
                .await
                .map_err(|e| OutputWatcherError(Box::new(e)))
        })
    }
}

pub struct WatchClient {
    run: Arc<Run>,
    watched_packages: HashSet<PackageName>,
    persistent_tasks_handle: Option<RunHandle>,
    active_runs: Vec<RunHandle>,
    _watching: FileWatching,
    output_watcher: Arc<dyn OutputWatcher>,
    // Subscribed eagerly (before building the Run) so we don't miss the
    // initial Rediscover event from the PackageChangesWatcher.
    package_change_events: broadcast::Receiver<PackageChangeEvent>,
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
    #[error("File watcher error: {0}")]
    FileWatcher(#[from] turborepo_filewatch::WatchError),
    #[error("Package watcher error: {0}")]
    PackageWatcher(String),
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
    #[error(
        "Timed out waiting for the file watcher to become ready. Try running `turbo daemon clean` \
         and retrying."
    )]
    FileWatchingTimeout,
    #[error("Failed to subscribe to signal handler. Shutting down.")]
    NoSignalHandler,
    #[error("Watch interrupted due to signal.")]
    SignalInterrupt,
    #[error("Package change channel closed.")]
    PackageChangeClosed,
    #[error("Package change channel lagged.")]
    PackageChangeLagged,
    #[error(transparent)]
    UI(#[from] turborepo_ui::Error),
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
        subscriber: &crate::tracing::TurboSubscriber,
        verbosity: u8,
    ) -> Result<Self, Error> {
        let signal = get_signal()?;
        let handler = SignalHandler::new(signal);

        let standard_config_path = resolve_turbo_config_path(&base.repo_root)?;

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

        // Build the in-process file watching stack (replaces the daemon).
        let watcher = Arc::new(FileSystemWatcher::new_with_default_cookie_dir(
            &base.repo_root,
        )?);
        let recv = watcher.watch();
        let cookie_writer = CookieWriter::new(
            watcher.cookie_dir(),
            Duration::from_millis(100),
            recv.clone(),
        );
        let glob_watcher = Arc::new(GlobWatcher::new(
            base.repo_root.clone(),
            cookie_writer.clone(),
            recv.clone(),
        ));
        let package_watcher = Arc::new(
            PackageWatcher::new(base.repo_root.clone(), recv.clone(), cookie_writer)
                .map_err(|e| Error::PackageWatcher(format!("{e:?}")))?,
        );
        let scm = SCM::new(&base.repo_root);
        let hash_watcher = Arc::new(HashWatcher::new(
            base.repo_root.clone(),
            package_watcher.watch_discovery(),
            recv.clone(),
            scm,
        ));
        let package_changes_watcher = PackageChangesWatcher::new(
            base.repo_root.clone(),
            recv,
            hash_watcher,
            custom_turbo_json_path,
        );

        // Subscribe before building the Run so we don't miss the initial
        // Rediscover event that PackageChangesWatcher emits on startup.
        let package_change_events = package_changes_watcher.package_changes().await;

        let watching = FileWatching {
            _watcher: watcher,
            glob_watcher: glob_watcher.clone(),
            _package_watcher: package_watcher,
            _package_changes_watcher: package_changes_watcher,
        };

        let output_watcher: Arc<dyn OutputWatcher> =
            Arc::new(InProcessOutputWatcher { glob_watcher });

        let new_base = base.clone();
        let mut run_builder =
            RunBuilder::new(new_base, None)?.with_output_watcher(output_watcher.clone());
        if let Some(ref qs) = query_server {
            run_builder = run_builder.with_query_server(qs.clone());
        }
        let collector = Arc::new(CollectorSink::new());
        let terminal = Arc::new(TerminalSink::new(base.color_config));
        let stdout_sink = Arc::new(StdoutSink::new(base.color_config));
        let tui_sink = Arc::new(TuiSink::new());
        let _ = turborepo_log::init(Logger::new(vec![
            Box::new(collector),
            Box::new(terminal.clone()),
            Box::new(stdout_sink.clone()),
            Box::new(tui_sink.clone()),
        ]));

        if let Ok(message) = env::var(turborepo_shim::GLOBAL_WARNING_ENV_VAR) {
            turborepo_log::warn(turborepo_log::Source::turbo("shim"), message).emit();
            unsafe { env::remove_var(turborepo_shim::GLOBAL_WARNING_ENV_VAR) };
        }

        if verbosity > 0 {
            if let Ok(path) = subscriber.redirect_stderr_to_file(base.repo_root.as_std_path()) {
                tracing::debug!("Verbose tracing redirected to {path}");
            }
        }

        let (run, _analytics) = run_builder.build(&handler, telemetry.clone()).await?;
        let run = Arc::new(run);

        let watched_packages = run.get_relevant_packages();

        terminal.disable();
        stdout_sink.disable();

        let (ui_sender, ui_handle) = run.start_ui()?.unzip();

        if let Some(UISender::Tui(ref tui_sender)) = ui_sender {
            tui_sink.connect(tui_sender.clone());
            if let Some(path) = subscriber.stderr_redirect_path() {
                turborepo_log::info(
                    turborepo_log::Source::turbo("tracing"),
                    format!("Verbose logs redirected to {path}"),
                )
                .emit();
            } else {
                subscriber.suppress_stderr();
            }
        } else {
            terminal.enable();
            stdout_sink.enable();
            if subscriber.stderr_redirect_path().is_some() {
                subscriber.restore_stderr();
            }
        }

        run.emit_run_prelude_logs();

        Ok(Self {
            base,
            run,
            watched_packages,
            _watching: watching,
            output_watcher,
            package_change_events,
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
        let mut events = std::mem::replace(
            &mut self.package_change_events,
            // Replace with a dummy receiver. The real one is consumed above.
            broadcast::channel(1).1,
        );

        // Wait for the initial Rediscover event, which signals that the file
        // watcher is ready. The PackageChangesWatcher emits this on startup.
        let initial_event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .map_err(|_| Error::FileWatchingTimeout)?;
        let initial_event = match initial_event {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Closed) => return Err(Error::PackageChangeClosed),
            Err(broadcast::error::RecvError::Lagged(_)) => return Err(Error::PackageChangeLagged),
        };

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        let pending_changes = Mutex::new(ChangedPackages::default());
        let notify_run = Arc::new(Notify::new());
        let notify_event = notify_run.clone();

        Self::handle_change_event(&pending_changes, initial_event);
        notify_event.notify_one();

        let event_fut = async {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        Self::handle_change_event(&pending_changes, event);
                        notify_event.notify_one();
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(Error::PackageChangeClosed);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        Self::handle_change_event(&pending_changes, PackageChangeEvent::Rediscover);
                        notify_event.notify_one();
                    }
                }
            }
        };

        let run_fut = async {
            loop {
                notify_run.notified().await;
                let some_changed_packages = {
                    let mut guard = pending_changes.lock().expect("poisoned lock");
                    (!guard.is_empty()).then(|| std::mem::take(guard.deref_mut()))
                };

                if let Some(mut changed_packages) = some_changed_packages {
                    // Stop impacted tasks and wait for prior runs to finish
                    // before starting new ones. This prevents:
                    // - Concurrent builds of the same package (Next.js lock conflicts, duplicated
                    //   output)
                    // - A cache-hit run incorrectly signaling persistent task readiness when the
                    //   real build failed
                    match changed_packages {
                        ChangedPackages::Some { ref packages, .. } => {
                            let impacted = self.stop_impacted_tasks(packages).await;
                            if let ChangedPackages::Some {
                                ref mut packages, ..
                            } = changed_packages
                            {
                                *packages = impacted;
                            }
                        }
                        ChangedPackages::All => {
                            for handle in self.active_runs.drain(..) {
                                handle.stopper.stop().await;
                                let _ = handle.run_task.await;
                            }
                        }
                    }

                    changed_packages.filter_to_watched(&self.watched_packages);

                    let new_run = self.execute_run(changed_packages).await?;
                    self.active_runs.push(new_run);

                    // Wait for runs to complete before processing more events.
                    // Combined with the hash baseline pre-population in
                    // PackageChangesWatcher, this ensures build output writes
                    // don't trigger spurious rebuilds.
                    for handle in &mut self.active_runs {
                        let _ = (&mut handle.run_task).await;
                    }
                    self.active_runs.retain(|h| !h.run_task.is_finished());
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
    fn handle_change_event(changed_packages: &Mutex<ChangedPackages>, event: PackageChangeEvent) {
        match event {
            PackageChangeEvent::Package {
                name,
                changed_files: files,
            } => match changed_packages.lock().expect("poisoned lock").deref_mut() {
                ChangedPackages::All => {
                    // Already rediscovering everything, ignore
                }
                ChangedPackages::Some {
                    ref mut packages,
                    ref mut changed_files,
                } => {
                    packages.insert(name);
                    changed_files.extend(files.iter().cloned());
                }
            },
            PackageChangeEvent::Rediscover => {
                *changed_packages.lock().expect("poisoned lock") = ChangedPackages::All;
            }
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(sender) = &self.ui_sender {
            sender.stop().await;
        }
        for handle in self.active_runs.drain(..) {
            handle.stopper.stop().await;
            let _ = handle.run_task.await;
        }
        if let Some(RunHandle { stopper, run_task }) = self.persistent_tasks_handle.take() {
            stopper.stop().await;
            let _ = run_task.await;
        }
    }

    async fn stop_impacted_tasks(&self, pkgs: &HashSet<PackageName>) -> HashSet<PackageName> {
        let engine = self.run.engine();

        let impacted_nodes = engine.tasks_impacted_by_packages(pkgs);

        let task_ids: Vec<_> = impacted_nodes
            .iter()
            .filter_map(|node| match node {
                TaskNode::Task(task_id) => Some(task_id.clone()),
                TaskNode::Root => None,
            })
            .collect();

        let impacted_packages: HashSet<PackageName> = task_ids
            .iter()
            .map(|task_id| PackageName::from(task_id.package()))
            .collect();

        for handle in &self.active_runs {
            handle.stopper.stop_tasks(&task_ids).await;
        }

        impacted_packages
    }

    /// Start executing tasks.
    ///
    /// If `changed_packages` is `Some(set)`, only tasks in those packages run.
    /// If `All`, we rebuild the entire Run struct and re-run everything.
    ///
    /// Persistent (non-interruptible) tasks are split into a separate handle:
    /// 1. First we run non-persistent + interruptible tasks
    /// 2. The non-persistent tasks and the persistent tasks that are allowed to
    ///    be interrupted
    ///
    /// Returns a handle to the task running (2)
    async fn execute_run(&mut self, changed_packages: ChangedPackages) -> Result<RunHandle, Error> {
        trace!("handling run with changed packages: {changed_packages:?}");
        match changed_packages {
            ChangedPackages::Some {
                packages,
                changed_files,
            } => {
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
                    .with_output_watcher(self.output_watcher.clone())
                    .with_entrypoint_packages(packages)
                    .with_changed_files(changed_files);
                if let Some(ref qs) = self.query_server {
                    run_builder = run_builder.with_query_server(qs.clone());
                }
                let (run, _analytics) = run_builder.build(&signal_handler, telemetry).await?;

                let task_names = run.engine.tasks_with_command(&run.pkg_dep_graph);
                if task_names.is_empty() {
                    tracing::debug!("no executable tasks after filtering, skipping run");
                    return Ok(RunHandle {
                        stopper: run.stopper(),
                        run_task: tokio::spawn(async { Ok(0) }),
                    });
                }

                if let Some(sender) = &self.ui_sender {
                    if let Err(err) = sender.restart_tasks(task_names) {
                        tracing::warn!("failed to notify UI of restarted tasks: {err}");
                    }
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

                let mut run_builder = RunBuilder::new(base.clone(), None)?
                    .with_output_watcher(self.output_watcher.clone());
                if let Some(ref qs) = self.query_server {
                    run_builder = run_builder.with_query_server(qs.clone());
                }
                let (run, _analytics) = run_builder
                    .build(&self.handler, self.telemetry.clone())
                    .await?;
                self.run = run.into();

                self.watched_packages = self.run.get_relevant_packages();

                if let Some(RunHandle { stopper, run_task }) = self.persistent_tasks_handle.take() {
                    stopper.stop().await;
                    let _ = run_task.await;
                }
                if let Some(sender) = &self.ui_sender {
                    let task_names = self.run.engine.tasks_with_command(&self.run.pkg_dep_graph);
                    if let Err(err) = sender.update_tasks(task_names) {
                        tracing::warn!("failed to notify UI of updated tasks: {err}");
                    }
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

                    let (ready_tx, ready_rx) = oneshot::channel::<()>();

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
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use tokio::sync::oneshot;
    use turbopath::AnchoredSystemPathBuf;
    use turborepo_daemon::PackageChangeEvent;
    use turborepo_repository::package_graph::PackageName;

    use super::{ChangedPackages, WatchClient};

    fn make_package_changed(name: &str) -> PackageChangeEvent {
        PackageChangeEvent::Package {
            name: PackageName::from(name),
            changed_files: Arc::new(HashSet::new()),
        }
    }

    fn make_package_changed_with_files(name: &str, files: &[&str]) -> PackageChangeEvent {
        PackageChangeEvent::Package {
            name: PackageName::from(name),
            changed_files: Arc::new(
                files
                    .iter()
                    .map(|f| AnchoredSystemPathBuf::from_raw(f).unwrap())
                    .collect(),
            ),
        }
    }

    fn make_rediscover() -> PackageChangeEvent {
        PackageChangeEvent::Rediscover
    }

    #[test]
    fn changed_packages_default_is_empty() {
        let cp = ChangedPackages::default();
        assert!(cp.is_empty());
        assert!(matches!(cp, ChangedPackages::Some { ref packages, .. } if packages.is_empty()));
    }

    #[test]
    fn changed_packages_all_is_never_empty() {
        assert!(!ChangedPackages::All.is_empty());
    }

    #[test]
    fn changed_packages_some_with_items_is_not_empty() {
        let mut packages = HashSet::new();
        packages.insert(PackageName::from("a"));
        assert!(!ChangedPackages::Some {
            packages,
            changed_files: HashSet::new(),
        }
        .is_empty());
    }

    #[test]
    fn handle_change_event_package_changed_inserts() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web"));

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some { packages, .. } => {
                assert_eq!(packages.len(), 1);
                assert!(packages.contains(&PackageName::from("web")));
            }
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn handle_change_event_multiple_packages_accumulate() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web"));
        WatchClient::handle_change_event(&changed, make_package_changed("ui"));
        WatchClient::handle_change_event(&changed, make_package_changed("utils"));

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some { packages, .. } => {
                assert_eq!(packages.len(), 3);
                assert!(packages.contains(&PackageName::from("web")));
                assert!(packages.contains(&PackageName::from("ui")));
                assert!(packages.contains(&PackageName::from("utils")));
            }
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn handle_change_event_duplicate_package_deduplicates() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web"));
        WatchClient::handle_change_event(&changed, make_package_changed("web"));

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some { packages, .. } => assert_eq!(packages.len(), 1),
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn handle_change_event_rediscover_sets_all() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web"));
        WatchClient::handle_change_event(&changed, make_rediscover());

        let guard = changed.lock().unwrap();
        assert!(matches!(*guard, ChangedPackages::All));
    }

    #[test]
    fn handle_change_event_package_changed_after_all_is_noop() {
        let changed = Mutex::new(ChangedPackages::All);
        WatchClient::handle_change_event(&changed, make_package_changed("web"));

        let guard = changed.lock().unwrap();
        assert!(matches!(*guard, ChangedPackages::All));
    }

    #[test]
    fn handle_change_event_rediscover_then_rediscover_stays_all() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_rediscover());
        WatchClient::handle_change_event(&changed, make_rediscover());

        let guard = changed.lock().unwrap();
        assert!(matches!(*guard, ChangedPackages::All));
    }

    #[test]
    fn handle_change_event_accumulates_changed_files() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(
            &changed,
            make_package_changed_with_files("web", &["packages/web/src/index.ts"]),
        );
        WatchClient::handle_change_event(
            &changed,
            make_package_changed_with_files("ui", &["packages/ui/src/button.tsx"]),
        );

        let guard = changed.lock().unwrap();
        match &*guard {
            ChangedPackages::Some {
                packages,
                changed_files,
            } => {
                assert_eq!(packages.len(), 2);
                assert_eq!(changed_files.len(), 2);
            }
            ChangedPackages::All => panic!("expected Some, got All"),
        }
    }

    #[test]
    fn filter_to_watched_removes_unwatched_packages() {
        let watched: HashSet<_> = ["web", "ui"]
            .iter()
            .map(|s| PackageName::from(*s))
            .collect();
        let mut changed = ChangedPackages::Some {
            packages: ["web", "api", "ui", "utils"]
                .iter()
                .map(|s| PackageName::from(*s))
                .collect(),
            changed_files: HashSet::new(),
        };

        changed.filter_to_watched(&watched);

        match changed {
            ChangedPackages::Some { packages, .. } => {
                assert_eq!(packages.len(), 2);
                assert!(packages.contains(&PackageName::from("web")));
                assert!(packages.contains(&PackageName::from("ui")));
                assert!(!packages.contains(&PackageName::from("api")));
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
        let mut changed = ChangedPackages::Some {
            packages: ["web", "ui"]
                .iter()
                .map(|s| PackageName::from(*s))
                .collect(),
            changed_files: HashSet::new(),
        };

        changed.filter_to_watched(&watched);

        match changed {
            ChangedPackages::Some { packages, .. } => assert!(packages.is_empty()),
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    #[test]
    fn filter_to_watched_no_overlap() {
        let watched: HashSet<_> = ["web"].iter().map(|s| PackageName::from(*s)).collect();
        let mut changed = ChangedPackages::Some {
            packages: ["api", "utils"]
                .iter()
                .map(|s| PackageName::from(*s))
                .collect(),
            changed_files: HashSet::new(),
        };

        changed.filter_to_watched(&watched);

        match changed {
            ChangedPackages::Some { packages, .. } => assert!(packages.is_empty()),
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    #[test]
    fn changed_packages_take_resets_to_default() {
        let changed = Mutex::new(ChangedPackages::default());
        WatchClient::handle_change_event(&changed, make_package_changed("web"));

        let taken = {
            let mut guard = changed.lock().unwrap();
            assert!(!guard.is_empty());
            std::mem::take(&mut *guard)
        };

        let guard = changed.lock().unwrap();
        assert!(guard.is_empty());

        match taken {
            ChangedPackages::Some { packages, .. } => {
                assert!(packages.contains(&PackageName::from("web")));
            }
            ChangedPackages::All => panic!("expected Some"),
        }
    }

    // -----------------------------------------------------------------------
    // Oneshot coordination pattern tests
    // -----------------------------------------------------------------------

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

        drop(ready_tx);

        let outcome = persistent.await.unwrap();
        assert_eq!(outcome, "skipped");
    }
}
