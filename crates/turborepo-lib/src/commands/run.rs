use std::{env, future::Future, sync::Arc};

use tracing::error;
use turborepo_api_client::SharedHttpClient;
use turborepo_log::StructuredLogSink;
use turborepo_query_api::QueryServer;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_types::DryRunMode;
use turborepo_ui::{sender::UISender, LogSinks};

use crate::{commands::CommandBase, run, run::builder::RunBuilder, tracing::TurboSubscriber};

#[derive(Debug, PartialEq, Eq)]
enum RunOutcome<T> {
    Completed(T),
    Interrupted(T),
}

async fn wait_for_run_cleanup_on_signal<F, T>(handler: &SignalHandler, run_fut: F) -> RunOutcome<T>
where
    F: Future<Output = T>,
{
    tokio::pin!(run_fut);

    tokio::select! {
        biased;
        _ = handler.signal_started() => {
            // Keep the run future alive so the TUI can continue rendering task
            // output until shutdown drains and the UI closes cleanly.
            let result = (&mut run_fut).await;
            handler.done().await;
            RunOutcome::Interrupted(result)
        }
        result = &mut run_fut => {
            handler.close().await;
            RunOutcome::Completed(result)
        }
    }
}

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

    let mut sinks = LogSinks::new(base.color_config);

    // Set up structured logging before initializing the global logger so
    // turbo messages during build() are also captured.
    let structured_sink = create_structured_sink(base.opts.log_file_path.as_ref(), base.opts.json);
    if let Some(ref sink) = structured_sink {
        sinks.with_structured_sink(sink.clone());
    }

    // In --json mode, disable the TerminalSink before init_logger so ALL
    // messages (including the shim warning below) go exclusively through
    // the StructuredLogSink.
    if base.opts.json {
        sinks.terminal.disable();
    }

    sinks.init_logger();

    if let Ok(message) = env::var(turborepo_shim::GLOBAL_WARNING_ENV_VAR) {
        turborepo_log::warn(
            turborepo_log::Source::turbo(turborepo_log::Subsystem::Shim),
            message,
        )
        .emit();
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

        let json_mode = run.opts().json;

        // JSON and other machine-readable modes own stdout.
        if run.opts().run_opts.graph.is_some()
            || matches!(run.opts().run_opts.dry_run, Some(DryRunMode::Json))
            || json_mode
        {
            sinks.suppress_stdout();
        }

        // In --json mode, disable the TerminalSink entirely before the
        // prelude so nothing leaks to the terminal. The StructuredLogSink
        // captures everything via the Logger.
        if json_mode {
            sinks.terminal.disable();
        }

        // Emit the prelude while TerminalSink is still active so it
        // lands in the main terminal buffer (survives TUI alternate-
        // screen). TuiSink buffers these events and flushes on connect().
        run.emit_run_prelude_logs();

        if !json_mode {
            sinks.disable_for_tui();
        }

        let (sender, handle) = {
            let _span = tracing::info_span!("start_ui").entered();
            run.start_ui()?.unzip()
        };

        if let Some(UISender::Tui(ref tui_sender)) = sender {
            sinks.tui.connect(tui_sender.clone());
            if let Some(path) = subscriber.stderr_redirect_path() {
                turborepo_log::info(
                    turborepo_log::Source::turbo(turborepo_log::Subsystem::Tracing),
                    format!("Verbose logs redirected to {path}"),
                )
                .emit();
            } else {
                subscriber.suppress_stderr();
            }
        } else if !json_mode {
            // Only re-enable the TerminalSink when NOT in --json mode.
            // In --json mode it stays disabled — all output goes through
            // the StructuredLogSink.
            sinks.enable_for_stream();
            if subscriber.stderr_redirect_path().is_some() {
                subscriber.restore_stderr();
            }
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

        if let Some(path) = subscriber.stderr_redirect_path() {
            subscriber.restore_stderr();
            println!("Verbose logs written to {path}");
        }
        result
    };

    let result = match wait_for_run_cleanup_on_signal(&handler, run_fut).await {
        RunOutcome::Completed(result) => result,
        // The run future has already drained shutdown and closed the UI.
        RunOutcome::Interrupted(_result) => Ok(1),
    };

    turborepo_log::flush();
    result
}

fn create_structured_sink(
    file_path: Option<&turbopath::AbsoluteSystemPathBuf>,
    terminal: bool,
) -> Option<Arc<StructuredLogSink>> {
    if file_path.is_none() && !terminal {
        return None;
    }

    let mut builder = StructuredLogSink::builder();

    if let Some(path) = file_path {
        builder = builder.file_path(path.as_std_path());
        // Warn that structured logs capture ALL output including potential secrets.
        // This fires before the logger is initialized so we use eprintln.
        eprintln!(
            "turbo: structured logs will capture all output (including potential secrets) to {}",
            path
        );
    }

    if terminal {
        builder = builder.terminal(true);
    }

    match builder.build() {
        Ok(sink) => Some(Arc::new(sink)),
        Err(e) => {
            // Structured logging is best-effort. This fires before the
            // global logger is initialized, so use eprintln directly.
            eprintln!("turbo: failed to set up structured logging: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::stream;
    use tokio::sync::oneshot;
    use turborepo_signals::{signals::Signal, SignalHandler};

    use super::{wait_for_run_cleanup_on_signal, RunOutcome};

    #[cfg(windows)]
    const DEFAULT_SIGNAL: Signal = Signal::CtrlC;
    #[cfg(not(windows))]
    const DEFAULT_SIGNAL: Signal = Signal::Interrupt;

    #[tokio::test]
    async fn signal_wait_keeps_run_future_alive_until_cleanup_finishes() {
        let (signal_tx, signal_rx) = oneshot::channel();
        let handler = SignalHandler::new(stream::once(async move {
            signal_rx.await.ok();
            Some(DEFAULT_SIGNAL)
        }));

        let (cleanup_started_tx, cleanup_started_rx) = oneshot::channel();
        let (cleanup_finish_tx, cleanup_finish_rx) = oneshot::channel();

        let outcome = tokio::spawn({
            let handler = handler.clone();
            async move {
                wait_for_run_cleanup_on_signal(&handler, async move {
                    cleanup_started_tx.send(()).ok();
                    cleanup_finish_rx.await.ok();
                    "cleaned-up"
                })
                .await
            }
        });

        signal_tx.send(DEFAULT_SIGNAL).unwrap();
        cleanup_started_rx.await.unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            !outcome.is_finished(),
            "signal handling should keep the run future alive until cleanup completes"
        );

        cleanup_finish_tx.send(()).unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(1), outcome)
            .await
            .expect("cleanup should finish promptly")
            .unwrap();

        assert_eq!(outcome, RunOutcome::Interrupted("cleaned-up"));
    }
}
