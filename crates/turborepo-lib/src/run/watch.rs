use std::{cell::RefCell, collections::HashSet, sync::Arc};

use futures::StreamExt;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{
    select,
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tracing::{instrument, trace};
use turborepo_repository::package_graph::PackageName;
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{tui, tui::AppSender};

use crate::{
    cli::{Command, RunArgs},
    commands,
    commands::CommandBase,
    daemon::{proto, DaemonConnectorError, DaemonError},
    get_version, opts, run,
    run::{builder::RunBuilder, scope::target_selector::InvalidSelectorError, Run},
    signal::SignalHandler,
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
    run: Run,
    watched_packages: HashSet<PackageName>,
    persistent_tasks_handle: Option<PersistentRunHandle>,
    connector: DaemonConnector,
    base: CommandBase,
    telemetry: CommandEventBuilder,
    handler: SignalHandler,
    ui_sender: Option<AppSender>,
    ui_handle: Option<JoinHandle<Result<(), tui::Error>>>,
}

struct PersistentRunHandle {
    stopper: run::RunStopper,
    run_task: JoinHandle<Result<i32, run::Error>>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("failed to connect to daemon")]
    #[diagnostic(transparent)]
    Daemon(#[from] DaemonError),
    #[error("failed to connect to daemon")]
    DaemonConnector(#[from] DaemonConnectorError),
    #[error("failed to decode message from daemon")]
    Decode(#[from] prost::DecodeError),
    #[error("could not get current executable")]
    CurrentExe(std::io::Error),
    #[error("could not start turbo")]
    Start(std::io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] run::Error),
    #[error("`--since` is not supported in watch mode")]
    SinceNotSupported,
    #[error(transparent)]
    Opts(#[from] opts::Error),
    #[error("invalid filter pattern")]
    InvalidSelector(#[from] InvalidSelectorError),
    #[error("filter cannot contain a git range in watch mode")]
    GitRangeInFilter {
        #[source_code]
        filter: String,
        #[label]
        span: SourceSpan,
    },
    #[error("daemon connection closed")]
    ConnectionClosed,
    #[error("failed to subscribe to signal handler, shutting down")]
    NoSignalHandler,
    #[error("watch interrupted due to signal")]
    SignalInterrupt,
    #[error("package change error")]
    PackageChange(#[from] tonic::Status),
    #[error("could not connect to UI thread")]
    UISend(String),
}

impl WatchClient {
    pub async fn new(base: CommandBase, telemetry: CommandEventBuilder) -> Result<Self, Error> {
        let signal = commands::run::get_signal()?;
        let handler = SignalHandler::new(signal);

        let Some(Command::Watch(execution_args)) = &base.args().command else {
            unreachable!()
        };

        let mut new_base = base.clone();
        new_base.args_mut().command = Some(Command::Run {
            run_args: Box::default(),
            execution_args: execution_args.clone(),
        });

        let run = RunBuilder::new(new_base)?
            .build(&handler, telemetry.clone())
            .await?;

        let watched_packages = run.get_relevant_packages();

        let (sender, handle) = run.start_experimental_ui()?.unzip();

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
            persistent_tasks_handle: None,
            ui_sender: sender,
            ui_handle: handle,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let connector = self.connector.clone();
        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;

        if !self.run.has_tui() {
            self.run.print_run_prelude();
        }

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        // We explicitly use a tokio::sync::Mutex here to avoid deadlocks.
        // If we used a std::sync::Mutex, we could deadlock by spinning the lock
        // and not yielding back to the tokio runtime.
        let changed_packages = Mutex::new(RefCell::new(ChangedPackages::default()));
        let notify_run = Arc::new(Notify::new());
        let notify_event = notify_run.clone();

        let event_fut = async {
            while let Some(event) = events.next().await {
                let event = event?;
                Self::handle_change_event(&changed_packages, event.event.unwrap()).await?;
                notify_event.notify_one();
            }

            Err(Error::ConnectionClosed)
        };

        let run_fut = async {
            loop {
                notify_run.notified().await;
                let changed_packages_guard = changed_packages.lock().await;
                if !changed_packages_guard.borrow().is_empty() {
                    let changed_packages = changed_packages_guard.take();
                    self.execute_run(changed_packages).await?;
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
    async fn handle_change_event(
        changed_packages: &Mutex<RefCell<ChangedPackages>>,
        event: proto::package_change_event::Event,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
            }) => {
                let package_name = PackageName::from(package_name);

                match changed_packages.lock().await.get_mut() {
                    ChangedPackages::All => {
                        // If we've already changed all packages, ignore
                    }
                    ChangedPackages::Some(ref mut pkgs) => {
                        pkgs.insert(package_name);
                    }
                }
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
                *changed_packages.lock().await.get_mut() = ChangedPackages::All;
            }
            proto::package_change_event::Event::Error(proto::PackageChangeError { message }) => {
                return Err(DaemonError::Unavailable(message).into());
            }
        }

        Ok(())
    }

    async fn execute_run(&mut self, changed_packages: ChangedPackages) -> Result<i32, Error> {
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

                let mut args = self.base.args().clone();
                args.command = args.command.map(|c| {
                    if let Command::Watch(execution_args) = c {
                        Command::Run {
                            execution_args,
                            run_args: Box::new(RunArgs {
                                no_cache: true,
                                daemon: true,
                                ..Default::default()
                            }),
                        }
                    } else {
                        unreachable!()
                    }
                });

                let new_base = CommandBase::new(
                    args,
                    self.base.repo_root.clone(),
                    get_version(),
                    self.base.color_config,
                );

                let signal_handler = self.handler.clone();
                let telemetry = self.telemetry.clone();

                let mut run = RunBuilder::new(new_base)?
                    .with_entrypoint_packages(packages)
                    .hide_prelude()
                    .build(&signal_handler, telemetry)
                    .await?;

                if let Some(sender) = &self.ui_sender {
                    let task_names = run.engine.tasks_with_command(&run.pkg_dep_graph);
                    sender
                        .restart_tasks(task_names)
                        .map_err(|err| Error::UISend(err.to_string()))?;
                }

                Ok(run.run(self.ui_sender.clone(), true).await?)
            }
            ChangedPackages::All => {
                let mut args = self.base.args().clone();
                args.command = args.command.map(|c| {
                    if let Command::Watch(execution_args) = c {
                        Command::Run {
                            run_args: Box::new(RunArgs {
                                no_cache: true,
                                daemon: true,
                                ..Default::default()
                            }),
                            execution_args,
                        }
                    } else {
                        unreachable!()
                    }
                });

                let base = CommandBase::new(
                    args,
                    self.base.repo_root.clone(),
                    get_version(),
                    self.base.color_config,
                );

                // rebuild run struct
                self.run = RunBuilder::new(base.clone())?
                    .hide_prelude()
                    .build(&self.handler, self.telemetry.clone())
                    .await?;

                self.watched_packages = self.run.get_relevant_packages();

                // Clean up currently running persistent tasks
                if let Some(PersistentRunHandle { stopper, run_task }) =
                    self.persistent_tasks_handle.take()
                {
                    // Shut down the tasks for the run
                    stopper.stop().await;
                    run_task.abort();
                }
                if let Some(sender) = &self.ui_sender {
                    let task_names = self.run.engine.tasks_with_command(&self.run.pkg_dep_graph);
                    sender
                        .update_tasks(task_names)
                        .map_err(|err| Error::UISend(err.to_string()))?;
                }

                if self.run.has_persistent_tasks() {
                    debug_assert!(
                        self.persistent_tasks_handle.is_none(),
                        "persistent handle should be empty before creating a new one"
                    );
                    let mut persistent_run = self.run.create_run_for_persistent_tasks();
                    let ui_sender = self.ui_sender.clone();
                    // If we have persistent tasks, we run them on a separate thread
                    // since persistent tasks don't finish
                    self.persistent_tasks_handle = Some(PersistentRunHandle {
                        stopper: persistent_run.stopper(),
                        run_task: tokio::spawn(
                            async move { persistent_run.run(ui_sender, true).await },
                        ),
                    });

                    // But we still run the regular tasks blocking
                    let mut non_persistent_run = self.run.create_run_without_persistent_tasks();
                    Ok(non_persistent_run.run(self.ui_sender.clone(), true).await?)
                } else {
                    Ok(self.run.run(self.ui_sender.clone(), true).await?)
                }
            }
        }
    }
}
