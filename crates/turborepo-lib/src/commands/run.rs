use std::future::Future;

use tracing::error;
use turborepo_telemetry::events::command::CommandEventBuilder;

use crate::{commands::CommandBase, run, run::builder::RunBuilder, signal::SignalHandler};

#[cfg(windows)]
pub fn get_signal() -> Result<impl Future<Output = Option<()>>, run::Error> {
    let mut ctrl_c = tokio::signal::windows::ctrl_c().map_err(run::Error::SignalHandler)?;
    Ok(async move { ctrl_c.recv().await })
}

#[cfg(not(windows))]
pub fn get_signal() -> Result<impl Future<Output = Option<()>>, run::Error> {
    use tokio::signal::unix;
    let mut sigint =
        unix::signal(unix::SignalKind::interrupt()).map_err(run::Error::SignalHandler)?;
    let mut sigterm =
        unix::signal(unix::SignalKind::terminate()).map_err(run::Error::SignalHandler)?;

    Ok(async move {
        tokio::select! {
            res = sigint.recv() => {
                res
            }
            res = sigterm.recv() => {
                res
            }
        }
    })
}

pub async fn run(base: CommandBase, telemetry: CommandEventBuilder) -> Result<i32, run::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = RunBuilder::new(base)?;

    let run_fut = async {
        let (analytics_sender, analytics_handle) = run_builder.start_analytics();
        let mut run = run_builder
            .with_analytics_sender(analytics_sender)
            .build(&handler, telemetry)
            .await?;

        let (sender, handle) = run.start_experimental_ui()?.unzip();
        let result = run.run(sender.clone(), false).await;

        if let Some(analytics_handle) = analytics_handle {
            analytics_handle.close_with_timeout().await;
        }

        if let (Some(handle), Some(sender)) = (handle, sender) {
            sender.stop();
            if let Err(e) = handle.await.expect("render thread panicked") {
                error!("error encountered rendering tui: {e}");
            }
        }

        result
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
