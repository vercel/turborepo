use std::{
    collections::HashSet,
    ops::DerefMut as _,
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{select, sync::Notify, task::JoinHandle};
use tracing::{instrument, trace, warn};
use turborepo_repository::package_graph::PackageName;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::sender::UISender;

use crate::{
    commands::CommandBase,
    daemon::{proto, DaemonConnectorError, DaemonError},
    get_version, opts,
    run::{self, builder::RunBuilder, scope::target_selector::InvalidSelectorError, Run},
    turbo_json::CONFIG_FILE,
    DaemonConnector, DaemonPaths,
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
}

pub struct WatchClient {
    run: Arc<Run>,
    watched_packages: HashSet<PackageName>,
    persistent_tasks_handle: Option<RunHandle>,
    connector: DaemonConnector,
    base: CommandBase,
    telemetry: CommandEventBuilder,
    handler: SignalHandler,
    ui_sender: Option<UISender>,
    ui_handle: Option<JoinHandle<Result<(), turborepo_ui::Error>>>,
    experimental_write_cache: bool,
}

struct RunHandle {
    stopper: run::RunStopper,
    run_task: JoinHandle<Result<i32, run::Error>>,
    persistent_exit: Option<tokio::sync::oneshot::Receiver<()>>,
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
    #[error("Cannot use root turbo.json at {0} with Watch Mode.")]
    NonStandardTurboJsonPath(String),
    #[error("Invalid config: {0}")]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
    #[error("persistent tasks exited unexpectedly")]
    PersistentExit,
}

impl WatchClient {
    pub async fn new(
        base: CommandBase,
        experimental_write_cache: bool,
        telemetry: CommandEventBuilder,
    ) -> Result<Self, Error> {
        let signal = get_signal()?;
        let handler = SignalHandler::new(signal);

        // Check if the turbo.json path is the standard one
        let standard_path = base.repo_root.join_component(CONFIG_FILE);
        if base.opts.repo_opts.root_turbo_json_path != standard_path {
            return Err(Error::NonStandardTurboJsonPath(
                base.opts.repo_opts.root_turbo_json_path.to_string(),
            ));
        }

        if matches!(base.opts.run_opts.daemon, Some(false)) {
            warn!("daemon is required for watch, ignoring request to disable daemon");
        }

        let new_base = base.clone();
        let run = Arc::new(
            RunBuilder::new(new_base)?
                .build(&handler, telemetry.clone())
                .await?,
        );

        let watched_packages = run.get_relevant_packages();

        let (ui_sender, ui_handle) = run.start_ui()?.unzip();

        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: DaemonPaths::from_repo_root(&base.repo_root),
        };

        Ok(Self {
            base,
            run,
            watched_packages,
            connector,
            handler,
            telemetry,
            experimental_write_cache,
            persistent_tasks_handle: None,
            ui_sender,
            ui_handle,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let connector = self.connector.clone();
        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        // We explicitly use a tokio::sync::Mutex here to avoid deadlocks.
        // If we used a std::sync::Mutex, we could deadlock by spinning the lock
        // and not yielding back to the tokio runtime.
        let changed_packages = Mutex::new(ChangedPackages::default());
        let notify_run = Arc::new(Notify::new());
        let notify_event = notify_run.clone();

        let event_fut = async {
            let mut first_rediscover = true;
            while let Some(event) = events.next().await {
                let event = event?;

                // Skip the first RediscoverPackages event which is sent immediately by the
                // daemon when we connect. The file watcher will send the real
                // one.
                if first_rediscover {
                    if matches!(
                        event.event,
                        Some(proto::package_change_event::Event::RediscoverPackages(_))
                    ) {
                        first_rediscover = false;
                        continue;
                    }
                    first_rediscover = false;
                }

                Self::handle_change_event(&changed_packages, event.event.unwrap())?;
                notify_event.notify_one();
            }

            Err(Error::ConnectionClosed)
        };

        let run_fut = async {
            let mut run_handle: Option<RunHandle> = None;
            let mut persistent_exit = None;
            loop {
                if let Some(persistent) = &mut persistent_exit {
                    // here we watch both notify *and* persistent task
                    // if notify exits, then continue per usual
                    // if persist exits, then we break out of loop with a
                    select! {
                        biased;
                        _ = persistent => {
                            break;
                        }
                        _ = notify_run.notified() => {},
                    }
                } else {
                    notify_run.notified().await;
                }

                let some_changed_packages = {
                    let mut changed_packages_guard =
                        changed_packages.lock().expect("poisoned lock");
                    (!changed_packages_guard.is_empty())
                        .then(|| std::mem::take(changed_packages_guard.deref_mut()))
                };

                if let Some(changed_packages) = some_changed_packages {
                    // Clean up currently running tasks
                    if let Some(RunHandle {
                        stopper,
                        run_task,
                        persistent_exit,
                    }) = run_handle.take()
                    {
                        // Shut down the tasks for the run
                        stopper.stop().await;
                        // Run should exit shortly after we stop all child tasks, wait for it to
                        // finish to ensure all messages are flushed.
                        let _ = run_task.await;
                        if let Some(persistent_exit) = persistent_exit {
                            let _ = persistent_exit.await;
                        }
                    }
                    let mut raw_run_handle = self.execute_run(changed_packages).await?;
                    persistent_exit = raw_run_handle.persistent_exit.take();
                    run_handle = Some(raw_run_handle);
                }
            }
            Err(Error::PersistentExit)
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

    /// Shut down any resources that run as part of watch.
    pub async fn shutdown(&mut self) {
        if let Some(sender) = &self.ui_sender {
            sender.stop().await;
        }
        if let Some(RunHandle {
            stopper, run_task, ..
        }) = self.persistent_tasks_handle.take()
        {
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
                let packages = packages
                    .into_iter()
                    .filter(|pkg| {
                        // If not in the watched packages set, ignore
                        self.watched_packages.contains(pkg)
                    })
                    .collect();

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

                let run = RunBuilder::new(new_base)?
                    .with_entrypoint_packages(packages)
                    .hide_prelude()
                    .build(&signal_handler, telemetry)
                    .await?;

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
                    persistent_exit: None,
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
                self.run = RunBuilder::new(base.clone())?
                    .hide_prelude()
                    .build(&self.handler, self.telemetry.clone())
                    .await?
                    .into();

                self.watched_packages = self.run.get_relevant_packages();

                // Clean up currently running persistent tasks
                if let Some(RunHandle {
                    stopper,
                    run_task,
                    persistent_exit,
                }) = self.persistent_tasks_handle.take()
                {
                    // Shut down the tasks for the run
                    stopper.stop().await;
                    // Run should exit shortly after we stop all child tasks, wait for it to finish
                    // to ensure all messages are flushed.
                    let _ = run_task.await;
                    if let Some(persistent_exit) = persistent_exit {
                        let _ = persistent_exit.await;
                    }
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
                    let ui_sender = self.ui_sender.clone();
                    // If we have persistent tasks, we run them on a separate thread
                    // since persistent tasks don't finish
                    let (persist_guard, persist_exit) = tokio::sync::oneshot::channel::<()>();
                    self.persistent_tasks_handle = Some(RunHandle {
                        stopper: persistent_run.stopper(),
                        run_task: tokio::spawn(async move {
                            // We move the guard in here so we can determine if the persist tasks
                            // exit as it'll go out of scope and drop.
                            let _guard = persist_guard;
                            persistent_run.run(ui_sender, true).await
                        }),
                        persistent_exit: None,
                    });

                    let non_persistent_run = self.run.create_run_for_interruptible_tasks();
                    let ui_sender = self.ui_sender.clone();
                    Ok(RunHandle {
                        stopper: non_persistent_run.stopper(),
                        run_task: tokio::spawn(async move {
                            non_persistent_run.run(ui_sender, true).await
                        }),
                        persistent_exit: Some(persist_exit),
                    })
                } else {
                    let ui_sender = self.ui_sender.clone();
                    let run = self.run.clone();
                    Ok(RunHandle {
                        stopper: run.stopper(),
                        run_task: tokio::spawn(async move { run.run(ui_sender, true).await }),
                        persistent_exit: None,
                    })
                }
            }
        }
    }
}
