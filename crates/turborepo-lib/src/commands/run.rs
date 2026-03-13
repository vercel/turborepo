use std::sync::Arc;

use tracing::error;
use turborepo_api_client::SharedHttpClient;
use turborepo_log::{sinks::collector::CollectorSink, Logger};
use turborepo_query_api::QueryServer;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{sender::UISender, TerminalSink, TuiSink};

use crate::{commands::CommandBase, run, run::builder::RunBuilder};

#[tracing::instrument(skip_all)]
pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    http_client: SharedHttpClient,
    query_server: Option<Arc<dyn QueryServer>>,
) -> Result<i32, run::Error> {
    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);

    let collector = Arc::new(CollectorSink::new());
    let terminal = Arc::new(TerminalSink::new(base.color_config));
    let tui_sink = Arc::new(TuiSink::new());
    let _ = turborepo_log::init(Logger::new(vec![
        Box::new(collector),
        Box::new(terminal.clone()),
        Box::new(tui_sink.clone()),
    ]));

    let mut run_builder = {
        let _span = tracing::info_span!("run_builder_new").entered();
        RunBuilder::new(base, Some(http_client))?
    };
    if let Some(qs) = query_server {
        run_builder = run_builder.with_query_server(qs);
    }

    let run_fut = async {
        let (run, analytics_handle) = {
            let (run, analytics_handle) = run_builder.build(&handler, telemetry).await?;
            (Arc::new(run), analytics_handle)
        };

        let (sender, handle) = {
            let _span = tracing::info_span!("start_ui").entered();
            run.start_ui()?.unzip()
        };

        if let Some(UISender::Tui(ref tui_sender)) = sender {
            tui_sink.connect(tui_sender.clone());
            terminal.disable();
        }

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
    let result = tokio::select! {
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
    };

    turborepo_log::flush();
    result
}
