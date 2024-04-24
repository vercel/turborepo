use std::collections::HashSet;

use futures::StreamExt;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{
    select,
    sync::watch,
    task::{yield_now, JoinHandle},
};
use turborepo_repository::package_graph::PackageName;
use turborepo_telemetry::events::command::CommandEventBuilder;

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

pub enum ChangedPackages {
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
    persistent_tasks_handle: Option<JoinHandle<Result<i32, run::Error>>>,
    connector: DaemonConnector,
    base: CommandBase,
    telemetry: CommandEventBuilder,
    handler: SignalHandler,
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
    #[error("changed packages channel closed, cannot receive new changes")]
    ChangedPackagesRecv(#[from] watch::error::RecvError),
    #[error("changed packages channel closed, cannot send new changes")]
    ChangedPackagesSend(#[from] watch::error::SendError<ChangedPackages>),
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

        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: DaemonPaths::from_repo_root(&base.repo_root),
        };

        Ok(Self {
            base,
            run,
            connector,
            handler,
            telemetry,
            persistent_tasks_handle: None,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let connector = self.connector.clone();
        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;

        self.run.print_run_prelude();

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        let (changed_pkgs_tx, mut changed_pkgs_rx) = watch::channel(ChangedPackages::default());

        let event_fut = async {
            while let Some(event) = events.next().await {
                let event = event?;
                Self::handle_change_event(&changed_pkgs_tx, event.event.unwrap()).await?;
            }

            Err(Error::ConnectionClosed)
        };

        let run_fut = async {
            loop {
                changed_pkgs_rx.changed().await?;
                let changed_pkgs = changed_pkgs_rx.borrow_and_update();

                self.execute_run(&changed_pkgs).await?;

                yield_now().await;
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

    async fn handle_change_event(
        changed_packages_tx: &watch::Sender<ChangedPackages>,
        event: proto::package_change_event::Event,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
            }) => {
                let package_name = PackageName::from(package_name);

                changed_packages_tx.send_if_modified(|changed_pkgs| match changed_pkgs {
                    ChangedPackages::All => false,
                    ChangedPackages::Some(ref mut pkgs) => {
                        pkgs.insert(package_name);

                        true
                    }
                });
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
                changed_packages_tx.send(ChangedPackages::All)?;
            }
            proto::package_change_event::Event::Error(proto::PackageChangeError { message }) => {
                return Err(DaemonError::Unavailable(message).into());
            }
        }

        Ok(())
    }

    async fn execute_run(&mut self, changed_packages: &ChangedPackages) -> Result<i32, Error> {
        // Should we recover here?
        match changed_packages {
            ChangedPackages::Some(packages) => {
                let packages = packages
                    .iter()
                    .filter(|pkg| {
                        // If not in the filtered pkgs, ignore
                        self.run.filtered_pkgs.contains(pkg)
                    })
                    .cloned()
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
                    self.base.ui,
                );

                let signal_handler = self.handler.clone();
                let telemetry = self.telemetry.clone();

                let mut run = RunBuilder::new(new_base)?
                    .with_entrypoint_packages(packages)
                    .hide_prelude()
                    .build(&signal_handler, telemetry)
                    .await?;

                Ok(run.run().await?)
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
                    self.base.ui,
                );

                // rebuild run struct
                self.run = RunBuilder::new(base.clone())?
                    .hide_prelude()
                    .build(&self.handler, self.telemetry.clone())
                    .await?;

                if self.run.has_persistent_tasks() {
                    // Abort old run
                    if let Some(run) = self.persistent_tasks_handle.take() {
                        run.abort();
                    }

                    let mut persistent_run = self.run.create_run_for_persistent_tasks();
                    // If we have persistent tasks, we run them on a separate thread
                    // since persistent tasks don't finish
                    self.persistent_tasks_handle =
                        Some(tokio::spawn(async move { persistent_run.run().await }));

                    // But we still run the regular tasks blocking
                    let mut non_persistent_run = self.run.create_run_without_persistent_tasks();
                    Ok(non_persistent_run.run().await?)
                } else {
                    Ok(self.run.run().await?)
                }
            }
        }
    }
}
