use std::collections::HashMap;

use futures::StreamExt;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{select, task::JoinHandle};
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

pub struct WatchClient {
    run: Run,
    persistent_tasks_handle: Option<JoinHandle<Result<i32, run::Error>>>,
    current_runs: HashMap<PackageName, JoinHandle<Result<i32, run::Error>>>,
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
            current_runs: HashMap::new(),
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        let connector = self.connector.clone();
        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;

        self.run.print_run_prelude();

        let signal_subscriber = self.handler.subscribe().ok_or(Error::NoSignalHandler)?;

        let event_fut = async {
            while let Some(event) = events.next().await {
                let event = event?;
                self.handle_change_event(event.event.unwrap()).await?;
            }

            Err(Error::ConnectionClosed)
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
        }
    }

    async fn handle_change_event(
        &mut self,
        event: proto::package_change_event::Event,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
            }) => {
                let package_name = PackageName::from(package_name);
                // If not in the filtered pkgs, ignore
                if !self.run.filtered_pkgs.contains(&package_name) {
                    return Ok(());
                }

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

                // TODO: Add logic on when to abort vs wait
                if let Some(run) = self.current_runs.remove(&package_name) {
                    run.abort();
                }

                let signal_handler = self.handler.clone();
                let telemetry = self.telemetry.clone();

                self.current_runs.insert(
                    package_name.clone(),
                    tokio::spawn(async move {
                        let mut run = RunBuilder::new(new_base)?
                            .with_entrypoint_package(package_name)
                            .hide_prelude()
                            .build(&signal_handler, telemetry)
                            .await?;

                        run.run().await
                    }),
                );
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
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

                // When we rediscover, stop all current runs
                for (_, run) in self.current_runs.drain() {
                    run.abort();
                }

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
                    non_persistent_run.run().await?;
                } else {
                    self.run.run().await?;
                }
            }
            proto::package_change_event::Event::Error(proto::PackageChangeError { message }) => {
                return Err(DaemonError::Unavailable(message).into());
            }
        }

        Ok(())
    }
}
