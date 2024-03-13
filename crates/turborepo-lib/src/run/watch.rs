use std::{collections::HashMap, str::FromStr};

use futures::StreamExt;
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;
use tokio::{select, task::JoinHandle};
use turborepo_repository::package_graph::PackageName;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{
    cli::{Command, ExecutionArgs, RunArgs},
    commands,
    commands::CommandBase,
    daemon::{proto, DaemonConnectorError, DaemonError},
    get_version, opts,
    opts::Opts,
    run,
    run::{
        builder::RunBuilder,
        scope::target_selector::{InvalidSelectorError, TargetSelector},
        Run,
    },
    signal::SignalHandler,
    DaemonConnector, DaemonPaths,
};

pub struct WatchClient {}

#[derive(Debug, Error, Diagnostic)]
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
}

impl WatchClient {
    fn validate_filter(opts: &Opts, filters: &[String]) -> Result<(), Error> {
        if opts.scope_opts.legacy_filter.since.is_some() {
            return Err(Error::SinceNotSupported);
        }

        let selectors = filters
            .iter()
            .map(|f| TargetSelector::from_str(f))
            .collect::<Result<Vec<_>, _>>()?;

        for selector in selectors {
            if !selector.from_ref.is_empty() || !selector.to_ref_override.is_empty() {
                let filter = format!("--filter={}", selector.raw);
                let span: SourceSpan = (0, filter.len()).into();

                return Err(Error::GitRangeInFilter { filter, span });
            }
        }

        Ok(())
    }
    pub async fn start(base: CommandBase, telemetry: CommandEventBuilder) -> Result<(), Error> {
        let signal = commands::run::get_signal()?;
        let handler = SignalHandler::new(signal);
        let Some(subscriber) = handler.subscribe() else {
            tracing::warn!("failed to subscribe to signal handler, shutting down");
            return Ok(());
        };

        let Some(Command::Watch(args)) = &base.args().command else {
            unreachable!();
        };

        let opts = Opts::try_from(base.args())?;

        Self::validate_filter(&opts, &args.filter)?;
        // We currently don't actually need the whole Run struct, just the filtered
        // packages. But in the future we'll likely need it to more efficiently
        // spawn tasks.
        let mut run = RunBuilder::new(base.clone())?
            .build(&handler, telemetry.clone())
            .await?;

        let connector = DaemonConnector {
            can_start_server: true,
            can_kill_server: true,
            paths: DaemonPaths::from_repo_root(&base.repo_root),
        };

        let mut client = connector.connect().await?;

        let mut events = client.package_changes().await?;
        let mut current_runs: HashMap<PackageName, JoinHandle<Result<i32, run::Error>>> =
            HashMap::new();
        let event_fut = async {
            while let Some(event) = events.next().await {
                let event = event.unwrap();
                Self::handle_change_event(
                    &mut run,
                    event.event.unwrap(),
                    &mut current_runs,
                    &base,
                    &telemetry,
                    &handler,
                )
                .await?;
            }

            Ok::<(), Error>(())
        };

        select! {
            _ = event_fut => {}
            _ = subscriber.listen() => {
                tracing::info!("shutting down");
            }
        }
        Ok(())
    }

    async fn handle_change_event(
        run: &mut Run,
        event: proto::package_change_event::Event,
        current_runs: &mut HashMap<PackageName, JoinHandle<Result<i32, run::Error>>>,
        base: &CommandBase,
        telemetry: &CommandEventBuilder,
        handler: &SignalHandler,
    ) -> Result<(), Error> {
        // Should we recover here?
        match event {
            proto::package_change_event::Event::PackageChanged(proto::PackageChanged {
                package_name,
            }) => {
                let package_name = PackageName::from(package_name);
                // If not in the filtered pkgs, ignore
                if !run.filtered_pkgs.contains(&package_name) {
                    return Ok(());
                }

                let mut args = base.args().clone();
                args.command = args.command.map(|c| {
                    if let Command::Watch(execution_args) = c {
                        Command::Run(Box::new(RunArgs {
                            execution_args: ExecutionArgs {
                                filter: vec![format!("...{}", package_name)],
                                ..*execution_args
                            },
                            no_cache: true,
                            daemon: true,
                            ..Default::default()
                        }))
                    } else {
                        unreachable!()
                    }
                });

                let new_base =
                    CommandBase::new(args, base.repo_root.clone(), get_version(), base.ui.clone());

                // TODO: Add logic on when to abort vs wait
                if let Some(run) = current_runs.remove(&package_name) {
                    run.abort();
                }

                let telemetry = telemetry.clone();
                let handler = handler.clone();
                current_runs.insert(
                    package_name,
                    tokio::spawn(async move {
                        let mut run = RunBuilder::new(new_base)?
                            .build(&handler, telemetry)
                            .await?;
                        run.run().await
                    }),
                );
            }
            proto::package_change_event::Event::RediscoverPackages(_) => {
                let mut args = base.args().clone();
                args.command = args.command.map(|c| {
                    if let Command::Watch(execution_args) = c {
                        Command::Run(Box::new(RunArgs {
                            execution_args: *execution_args,
                            no_cache: true,
                            daemon: true,
                            ..Default::default()
                        }))
                    } else {
                        unreachable!()
                    }
                });

                let base =
                    CommandBase::new(args, base.repo_root.clone(), get_version(), base.ui.clone());

                // When we rediscover, stop all current runs
                for (_, run) in current_runs.drain() {
                    run.abort();
                }

                // rebuild run struct
                *run = RunBuilder::new(base.clone())?
                    .build(&handler, telemetry.clone())
                    .await?;

                // Execute run
                run.run().await?;
            }
            proto::package_change_event::Event::Error(proto::PackageChangeError { message }) => {
                return Err(DaemonError::Unavailable(message).into());
            }
        }

        Ok(())
    }
}
