use std::{env, sync::Arc};

use tracing::error;
use turborepo_api_client::SharedHttpClient;
use turborepo_log::StructuredLogSink;
use turborepo_query_api::QueryServer;
use turborepo_signals::{listeners::get_signal, SignalHandler};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_types::DryRunMode;
use turborepo_ui::{sender::UISender, LogSinks};

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
