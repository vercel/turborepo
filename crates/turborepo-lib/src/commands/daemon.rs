use std::time::Duration;

use futures::FutureExt;
use serde_json::json;
use tokio::signal::ctrl_c;
use turborepo_daemon::{
    clean_daemon, follow_daemon_logs, serve, CloseReason, DaemonError, DaemonLifecycleCommand,
    DaemonLifecycleOutput, Paths,
};
use turborepo_ui::{color, BOLD_GREEN, BOLD_RED, GREY};

use super::CommandBase;
use crate::{
    cli::DaemonCommand, package_changes_watcher::PackageChangesWatcher, tracing::TurboSubscriber,
};

const DAEMON_NOT_RUNNING_MESSAGE: &str =
    "daemon is not running, run `turbo daemon start` to start it";

/// Converts an optional turbo.json path to an absolute system path.
fn convert_turbo_json_path(
    path: Option<&camino::Utf8Path>,
) -> Result<Option<turbopath::AbsoluteSystemPathBuf>, DaemonError> {
    match path {
        Some(p) => match turbopath::AbsoluteSystemPathBuf::from_cwd(p) {
            Ok(path) => Ok(Some(path)),
            Err(e) => {
                tracing::error!("Failed to convert custom turbo.json path: {}", e);
                Err(DaemonError::Unavailable(format!(
                    "Invalid turbo.json path: {}",
                    e
                )))
            }
        },
        None => Ok(None),
    }
}

/// Runs the daemon command.
pub async fn daemon_client(
    command: &DaemonCommand,
    base: &CommandBase,
    custom_turbo_json_path: Option<camino::Utf8PathBuf>,
) -> Result<(), DaemonError> {
    let custom_turbo_json_path = convert_turbo_json_path(custom_turbo_json_path.as_deref())?;

    match command {
        DaemonCommand::Restart => {
            let _ = turborepo_daemon::run_lifecycle_command(
                DaemonLifecycleCommand::Restart,
                &base.repo_root,
                custom_turbo_json_path.clone(),
            )
            .await?;

            println!(
                "{} restarted daemon",
                color!(base.color_config, BOLD_GREEN, "✓")
            );
        }
        DaemonCommand::Start => {
            let _ = turborepo_daemon::run_lifecycle_command(
                DaemonLifecycleCommand::Start,
                &base.repo_root,
                custom_turbo_json_path.clone(),
            )
            .await?;
            println!(
                "{} daemon is running",
                color!(base.color_config, BOLD_GREEN, "✓")
            );
        }
        DaemonCommand::Stop => {
            let _ = turborepo_daemon::run_lifecycle_command(
                DaemonLifecycleCommand::Stop,
                &base.repo_root,
                custom_turbo_json_path.clone(),
            )
            .await?;
            println!(
                "{} stopped daemon",
                color!(base.color_config, BOLD_GREEN, "✓")
            );
        }
        DaemonCommand::Status { json } => {
            let output = turborepo_daemon::run_lifecycle_command(
                DaemonLifecycleCommand::Status,
                &base.repo_root,
                custom_turbo_json_path.clone(),
            )
            .await?;
            let DaemonLifecycleOutput::Status(status) = output else {
                unreachable!("status command must return status output");
            };
            let status = match status {
                Some(status) => status,
                None if *json => {
                    println!("{}", json!({ "error": DAEMON_NOT_RUNNING_MESSAGE }));
                    return Ok(());
                }
                None => {
                    println!(
                        "{} {}",
                        color!(base.color_config, BOLD_RED, "x"),
                        DAEMON_NOT_RUNNING_MESSAGE
                    );
                    return Ok(());
                }
            };

            if *json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "{} daemon is running",
                    color!(base.color_config, BOLD_GREEN, "✓")
                );
                println!(
                    "log file: {}",
                    color!(base.color_config, GREY, "{}", status.log_file)
                );
                println!(
                    "uptime: {}",
                    color!(
                        base.color_config,
                        GREY,
                        "{}s",
                        humantime::format_duration(Duration::from_millis(status.uptime_ms))
                    )
                );
                println!(
                    "pid file: {}",
                    color!(base.color_config, GREY, "{}", status.pid_file)
                );
                println!(
                    "socket file: {}",
                    color!(base.color_config, GREY, "{}", status.sock_file)
                );
            }
        }
        DaemonCommand::Logs => {
            follow_daemon_logs(&base.repo_root, custom_turbo_json_path.clone()).await?;
        }
        DaemonCommand::Clean {
            clean_logs: should_clean_logs,
        } => {
            clean_daemon(
                &base.repo_root,
                custom_turbo_json_path.clone(),
                *should_clean_logs,
            )
            .await?;
            println!("Done");
        }
    };

    Ok(())
}

#[tracing::instrument(skip(base, logging), fields(repo_root = %base.repo_root))]
pub async fn daemon_server(
    base: &CommandBase,
    idle_time: &String,
    turbo_json_path: Option<camino::Utf8PathBuf>,
    logging: &TurboSubscriber,
) -> Result<(), DaemonError> {
    let paths = Paths::from_repo_root(&base.repo_root)?;

    tracing::trace!("logging to file: {:?}", paths.log_file);
    if let Err(e) = logging.set_daemon_logger(tracing_appender::rolling::daily(
        &paths.log_folder,
        &paths.log_file,
    )) {
        // error here is not fatal, just log it
        tracing::error!("failed to set file logger: {}", e);
    }

    let timeout = go_parse_duration::parse_duration(idle_time)
        .map_err(|_| DaemonError::InvalidTimeout(idle_time.to_owned()))
        .map(|d| Duration::from_nanos(d as u64))?;

    let exit_signal = ctrl_c().map(|result| {
        if let Err(e) = result {
            tracing::error!("Error with signal handling: {}", e);
        }
        CloseReason::Interrupt
    });
    let custom_turbo_json_path = convert_turbo_json_path(turbo_json_path.as_deref())?;
    let allow_no_package_manager = base.opts().repo_opts.allow_no_package_manager;
    serve(
        base.repo_root.clone(),
        timeout,
        exit_signal,
        custom_turbo_json_path,
        allow_no_package_manager,
        {
            let graph_features =
                crate::repository_graph::RepositoryGraphFeatures::new(&base.opts().future_flags);
            move |args| {
                PackageChangesWatcher::new(
                    args.repo_root,
                    args.file_events,
                    args.hash_watcher,
                    args.custom_turbo_json_path,
                    false,
                    args.allow_no_package_manager,
                    graph_features,
                )
            }
        },
    )
    .await
}
