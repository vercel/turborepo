use tracing::Instrument;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{commands::CommandBase, run, run::builder::RunBuilder, signal::SignalHandler};

#[tracing::instrument(skip_all)]
pub async fn run(base: CommandBase, telemetry: CommandEventBuilder) -> Result<i32, run::Error> {
    let (run_builder, handler, api_client) = async {
        #[cfg(windows)]
        let signal = {
            let mut ctrl_c = tokio::signal::windows::ctrl_c().map_err(run::Error::SignalHandler)?;
            async move { ctrl_c.recv().await }
        };
        #[cfg(not(windows))]
        let signal = {
            use tokio::signal::unix;
            let mut sigint =
                unix::signal(unix::SignalKind::interrupt()).map_err(run::Error::SignalHandler)?;
            let mut sigterm =
                unix::signal(unix::SignalKind::terminate()).map_err(run::Error::SignalHandler)?;
            async move {
                tokio::select! {
                    res = sigint.recv() => {
                        res
                    }
                    res = sigterm.recv() => {
                        res
                    }
                }
            }
        };

        let handler = SignalHandler::new(signal);

        let api_auth = base.api_auth()?;
        let api_client = base.api_client()?;
        let run_builder = RunBuilder::new(base, api_auth)?;

        Ok::<_, run::Error>((run_builder, handler, api_client))
    }
    .instrument(tracing::info_span!("setup"))
    .await?;

    let run_fut = async {
        let run = run_builder.build(&handler, telemetry, api_client).await?;
        run.run().await
    };

    let handler_fut = handler.done();
    tokio::select! {
        biased;
        // If we get a handler exit at the same time as a run finishes we choose that
        // future to display that we're respecting user input
        _ = handler_fut => {
            // We caught a signal, which already notified the subscribers
            Ok(1)
        }
        result = run_fut => {
            // Run finished so close the signal handler
            handler.close().await;
            result
        },
    }
}
