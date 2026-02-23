use std::sync::Arc;

use tracing::error;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::sender::UISender;

use crate::{commands::CommandBase, run, run::builder::RunBuilder};

#[tracing::instrument(skip_all)]
pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    http_client_cell: Arc<tokio::sync::OnceCell<reqwest::Client>>,
) -> Result<i32, run::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let run_builder = {
        let _span = tracing::info_span!("run_builder_new").entered();
        RunBuilder::new(base, Some(http_client_cell))?
    };

    let run_fut = async {
        let (run, analytics_handle) = {
            let (run, analytics_handle) = run_builder.build(&handler, telemetry).await?;
            (Arc::new(run), analytics_handle)
        };

        let (sender, handle) = {
            let _span = tracing::info_span!("start_ui").entered();
            run.start_ui()?.unzip()
        };

        let result = run.run(sender.clone(), false).await;

        if let Some(analytics_handle) = analytics_handle {
            analytics_handle.close_with_timeout().await;
        }

        // We only stop if it's the TUI, for the web UI we don't need to stop
        if let Some(UISender::Tui(sender)) = sender {
            sender.stop().await;
        }

        if let Some(handle) = handle {
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
