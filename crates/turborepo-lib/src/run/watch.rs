use std::collections::HashMap;

use futures::StreamExt;
use thiserror::Error;
use tokio::{select, task::JoinHandle};
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli::{Command, OutputLogsMode, RunArgs},
    commands,
    commands::CommandBase,
    daemon::{proto, DaemonConnectorError, DaemonError},
    get_version,
    opts::Opts,
    run,
    signal::SignalHandler,
    Args, DaemonConnector, DaemonPaths,
};

pub struct WatchClient {}

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to connect to daemon")]
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
    Run(#[from] run::Error),
}

impl WatchClient {
    pub async fn start(base: CommandBase, telemetry: CommandEventBuilder) -> Result<(), Error> {
        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: DaemonPaths::from_repo_root(&base.repo_root),
        };

        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;
        let mut current_runs: HashMap<String, JoinHandle<Result<i32, run::Error>>> = HashMap::new();

        loop {
            let signal = commands::run::get_signal()?;
            let handler = SignalHandler::new(signal);

            let Some(subscriber) = handler.subscribe() else {
                tracing::warn!("failed to subscribe to signal handler, shutting down");
                break;
            };
            select! {
                Some(event) = events.next() => {
                    let event = event.unwrap();
                    Self::handle_change_event(
                        event.event.unwrap(),
                        &mut current_runs,
                        &base,
                        &telemetry,
                        &handler,
                    ).await?;
                }
                _ = subscriber.listen() => {
                    println!("got shutdown signal");
                    break;
                }
                proto::package_change_event::Event::Error(proto::PackageChangeError {
                    message,
                }) => {
                    return Err(DaemonError::Unavailable(message));
                }
            }
        }

        Ok(())
    }

    async fn handle_change_event(
        event: proto::package_change_event::Event,
        current_runs: &mut HashMap<String, JoinHandle<Result<i32, run::Error>>>,
        base: &CommandBase,
        telemetry: &CommandEventBuilder,
        handler: &SignalHandler,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
                package_path,
            }) => {
                println!(
                    "Spawning {} on package {}",
                    base.args().get_tasks().join(", "),
                    package_name
                );
                let args = Args {
                    command: Some(Command::Run(Box::new(RunArgs {
                        tasks: base.args().get_tasks().to_owned(),
                        filter: vec![format!("...{}", package_name)],
                        output_logs: Some(OutputLogsMode::None),
                        ..Default::default()
                    }))),
                    ..Args::default()
                };

                let new_base =
                    CommandBase::new(args, base.repo_root.clone(), get_version(), base.ui.clone());

                // TODO: Add logic on when to abort vs wait
                if let Some(run) = current_runs.remove(&package_name) {
                    run.abort();
                }

                current_runs.insert(
                    package_name,
                    tokio::spawn(commands::run::run_with_signal_handler(
                        new_base,
                        telemetry.clone(),
                        handler.clone(),
                    )),
                );
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
                let args = Args {
                    command: Some(Command::Run(Box::new(RunArgs {
                        tasks: base.args().get_tasks().to_owned(),
                        output_logs: Some(OutputLogsMode::None),
                        ..Default::default()
                    }))),
                    ..Args::default()
                };
                let new_base =
                    CommandBase::new(args, base.repo_root.clone(), get_version(), base.ui.clone());

                // When we rediscover, stop all current runs
                for (_, run) in current_runs.drain() {
                    run.abort();
                }

                // and then run everything
                commands::run::run_with_signal_handler(
                    new_base,
                    telemetry.clone(),
                    handler.clone(),
                )
                .await?;
            }
        }

        Ok(())
    }
}
