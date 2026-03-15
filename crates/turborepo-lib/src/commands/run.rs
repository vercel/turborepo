use std::{env, sync::Arc};

use tracing::error;
use turborepo_api_client::SharedHttpClient;
use turborepo_log::{sinks::collector::CollectorSink, Logger};
use turborepo_query_api::QueryServer;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_ui::{sender::UISender, TerminalSink, TuiSink};

use crate::{commands::CommandBase, run, run::builder::RunBuilder, tracing::TurboSubscriber};

#[tracing::instrument(skip_all)]
pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    http_client: SharedHttpClient,
    query_server: Option<Arc<dyn QueryServer>>,
    subscriber: &TurboSubscriber,
    verbosity: u8,
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

    if let Ok(message) = env::var(turborepo_shim::GLOBAL_WARNING_ENV_VAR) {
        turborepo_log::warn(turborepo_log::Source::turbo("shim"), message).emit();
        unsafe { env::remove_var(turborepo_shim::GLOBAL_WARNING_ENV_VAR) };
    }

    // When verbosity is active, redirect tracing to a file before build()
    // so SCM, hashing, and config tracing is captured. Only done here
    // (not in the shim) because non-TUI commands should keep stderr output.
    let repo_root = base.repo_root.clone();
    if let Some(path) = subscriber.stderr_redirect_path() {
        // Already redirected (shouldn't happen, but be safe)
        tracing::debug!("stderr already redirected to {path}");
    } else if verbosity > 0 {
        if let Ok(path) = subscriber.redirect_stderr_to_file(repo_root.as_std_path()) {
            tracing::debug!("Verbose tracing redirected to {path}");
        }
    }

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

        terminal.disable();

        let (sender, handle) = {
            let _span = tracing::info_span!("start_ui").entered();
            run.start_ui()?.unzip()
        };

        if let Some(UISender::Tui(ref tui_sender)) = sender {
            tui_sink.connect(tui_sender.clone());
            if let Some(path) = subscriber.stderr_redirect_path() {
                turborepo_log::info(
                    turborepo_log::Source::turbo("tracing"),
                    format!("Verbose logs redirected to {path}"),
                )
                .emit();
            } else {
                subscriber.suppress_stderr();
            }
        } else {
            terminal.enable();
            if subscriber.stderr_redirect_path().is_some() {
                subscriber.restore_stderr();
            }
        }

        run.emit_run_prelude_logs();

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

        if let Some(path) = subscriber.stderr_redirect_path() {
            subscriber.restore_stderr();
            println!("Verbose logs written to {path}");
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
